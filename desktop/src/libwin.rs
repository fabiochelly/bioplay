//! La bibliothèque dans sa propre fenêtre (Slint, rendu Skia) : transparente, le compositeur
//! floute le fond d'écran derrière. Rien ne s'y dessine sans raison : Slint ne redessine que ce
//! qui change, et la grille ne crée que les rangées visibles (miniatures lues à ce moment).
//!
//! La vidéo s'ouvre dans la fenêtre de mpv, préparé dès le lancement (fil de lecture) : cliquer
//! une vidéo l'envoie au lecteur et cache cette fenêtre ; revenir à la bibliothèque (bouton,
//! fermeture de la vidéo) arrête la lecture, ferme la fenêtre de mpv et réaffiche celle-ci.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::Arc;

use slint::{ComponentHandle, Model, ModelNotify, ModelRc, ModelTracker, SharedPixelBuffer, VecModel};

use crate::i18n::{self, tr};
use crate::library::{Library, Place};
use crate::player::{clock, positions};
use crate::Message;

slint::include_modules!();

/// Vers le lecteur : ses messages et de quoi réveiller sa boucle.
pub struct Link {
    pub tx: Sender<Message>,
    pub wake: crate::mpv::Waker,
}

struct State {
    ui: LibraryWindow,
    lib: Library,
    place: Place,
    positions: HashMap<String, f64>,
    images: Rc<RefCell<Images>>,
    cols: usize,
    link: Link,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.borrow_mut().as_mut().map(f))
}

/// Depuis n'importe quel fil : réaffiche la bibliothèque (fin de lecture).
pub fn show() {
    let _ = slint::invoke_from_event_loop(|| {
        with(|s| {
            s.positions = positions();
            s.lib.set_hold(false);
            s.rebuild();
            let _ = s.ui.show();
        });
    });
}

/// Depuis n'importe quel fil : quitte (mpv s'est arrêté).
pub fn quit() {
    let _ = slint::quit_event_loop();
}

/// Construit la fenêtre et tourne jusqu'à la fin (fil principal). `visible` : faux quand une
/// vidéo est donnée au lancement (la bibliothèque n'apparaît qu'au retour).
pub fn run(link: Link, visible: bool) {
    // Skia, comme Biodrive : l'alpha de la vitre est rendu proprement.
    if std::env::var_os("SLINT_BACKEND").is_none() {
        let _ = slint::BackendSelector::new().backend_name("winit".into()).renderer_name("skia".into()).select();
    }
    let _ = slint::set_xdg_app_id("bioplay");
    let ui = setup(link, visible);
    // Fermer la bibliothèque quitte bioplay (aucune vidéo n'est alors en cours).
    ui.window().on_close_requested(|| {
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });
    if visible {
        let _ = ui.show();
    }
    let _ = slint::run_event_loop_until_quit();
}

/// Capture de la bibliothèque sans fenêtre (vérification) : `BIOPLAY_LIBSNAP=f.png
/// [BIOPLAY_LIBPLACE=sous/dossier] [BIOPLAY_LIBVIEW=list] [BIOPLAY_LIBSHEET=1] bioplay`.
/// Rendu logiciel de Slint, 1440×900 points à l'échelle 2, fond transparent.
pub fn snapshot(path: &str) {
    use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
    struct P(Rc<MinimalSoftwareWindow>);
    impl slint::platform::Platform for P {
        fn create_window_adapter(&self) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
    }
    let (w, h) = (2880u32, 1800u32);
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let _ = slint::platform::set_platform(Box::new(P(window.clone())));
    window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    window.set_size(slint::PhysicalSize::new(w, h));
    let (tx, _rx) = std::sync::mpsc::channel();
    let ui = setup(Link { tx, wake: crate::mpv::Waker::none() }, true);
    let _ = ui.show();
    with(|s| {
        if let Ok(p) = std::env::var("BIOPLAY_LIBPLACE") {
            let roots = s.lib.folders.clone();
            if let Some(r) = roots.first() {
                s.place = Place { root: Some(r.to_string_lossy().into_owned()), path: p };
            }
        }
        if std::env::var("BIOPLAY_LIBVIEW").as_deref() == Ok("list") && s.lib.grid {
            s.lib.grid = false;
            s.ui.set_grid(false);
        }
        s.ui.set_sheet(std::env::var_os("BIOPLAY_LIBSHEET").is_some());
        s.cols = columns(1440.0);
        s.rebuild();
    });
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|r| {
            let mut buf = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
            r.render(&mut buf, w as usize);
            let rgba: Vec<u8> = buf.iter().flat_map(|p| [p.red, p.green, p.blue, p.alpha]).collect();
            if let Ok(f) = std::fs::File::create(path) {
                let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
                e.set_color(png::ColorType::Rgba);
                let _ = e.write_header().and_then(|mut wr| wr.write_image_data(&rgba));
            }
        });
    }
}

/// La fenêtre, la bibliothèque et l'état partagé.
fn setup(link: Link, visible: bool) -> LibraryWindow {
    let ui = LibraryWindow::new().expect("fenêtre de la bibliothèque");
    let notify: crate::library::Notify = Arc::new(|| {
        let _ = slint::invoke_from_event_loop(|| {
            with(|s| {
                if s.lib.poll() {
                    s.rebuild();
                }
            });
        });
    });
    let lib = Library::new(notify);
    lib.set_hold(!visible);
    ui.set_en(i18n::en());
    ui.set_grid(lib.grid);
    STATE.with(|st| {
        *st.borrow_mut() = Some(State {
            ui: ui.clone_strong(),
            lib,
            place: Place::default(),
            positions: positions(),
            images: Rc::new(RefCell::new(Images::default())),
            cols: 4,
            link,
        })
    });
    with(|s| s.lib.refresh());
    callbacks(&ui);
    with(|s| s.rebuild());
    ui
}

fn callbacks(ui: &LibraryWindow) {
    ui.on_relayout(|w| {
        with(|s| {
            let cols = columns(w);
            if cols != s.cols {
                s.cols = cols;
                s.rebuild();
            } else {
                s.sizes(w);
            }
        });
    });
    ui.on_toggle_view(|| {
        with(|s| {
            s.lib.toggle_view();
            s.ui.set_grid(s.lib.grid);
            s.rebuild();
        });
    });
    ui.on_set_lang(|en| {
        if en != i18n::en() {
            i18n::toggle();
        }
        with(|s| {
            s.ui.set_en(en);
            s.rebuild();
        });
    });
    ui.on_up(|| {
        with(|s| {
            if let Some(up) = s.lib.up(&s.place) {
                s.place = up;
                s.rebuild();
            }
        });
    });
    ui.on_activate(|kind, id| {
        with(|s| {
            let shelf = s.lib.shelf(&s.place);
            if kind == 0 {
                if let Some(d) = shelf.dirs.get(id as usize) {
                    s.place = d.place.clone();
                    s.rebuild();
                }
            } else if let Some(v) = s.lib.videos.get(id as usize) {
                let path = v.path.clone();
                s.play(path);
            }
        });
    });
    ui.on_open_file(|| {
        // Sélecteur du système dans un fil à part : la fenêtre reste vivante.
        std::thread::spawn(|| {
            let f = rfd::FileDialog::new()
                .set_title(tr("Ouvrir une vidéo", "Open a video"))
                .add_filter(tr("Vidéos", "Videos"), crate::VIDEO_EXT)
                .add_filter(tr("Tous les fichiers", "All files"), &["*"])
                .pick_file();
            if let Some(f) = f {
                let path = f.to_string_lossy().into_owned();
                let _ = slint::invoke_from_event_loop(move || {
                    with(|s| s.play(path));
                });
            }
        });
    });
    ui.on_add_folder(|| {
        std::thread::spawn(|| {
            if let Some(f) = rfd::FileDialog::new().set_title(tr("Dossier de vidéos", "Video folder")).pick_folder() {
                let _ = slint::invoke_from_event_loop(move || {
                    with(|s| {
                        s.lib.add_folder(f);
                        s.rebuild();
                    });
                });
            }
        });
    });
    ui.on_remove_folder(|i| {
        with(|s| {
            if let Some(f) = s.lib.folders.get(i as usize).cloned() {
                s.lib.remove_folder(&f);
                s.place = Place::default();
                s.rebuild();
            }
        });
    });
}

/// Colonnes de la grille pour une largeur de fenêtre (tuiles d'au moins 230 px, écart 16).
fn columns(w: f32) -> usize {
    let inner = w - 2.0 * 28.0;
    ((inner + 16.0) / (230.0 + 16.0)).floor().max(1.0) as usize
}

impl State {
    fn play(&mut self, path: String) {
        let _ = self.link.tx.send(Message::Picked(path));
        self.link.wake.wake();
        // Pendant la lecture : rien en fond, et la fenêtre de mpv prend la place.
        self.lib.set_hold(true);
        self.ui.set_sheet(false);
        let _ = self.ui.hide();
    }

    fn sizes(&self, w: f32) {
        let inner = w - 2.0 * 28.0;
        let tw = (inner - 16.0 * (self.cols as f32 - 1.0)) / self.cols as f32;
        self.ui.set_tile_w(tw);
        self.ui.set_th(tw * 9.0 / 16.0);
    }

    /// Recompose ce qui est affiché à partir de la bibliothèque (dossier en cours).
    fn rebuild(&mut self) {
        let ui = &self.ui;
        self.sizes(ui.get_width_px());
        // Dossiers surveillés (panneau).
        let folders: Vec<FolderItem> = self
            .lib
            .folders
            .iter()
            .enumerate()
            .map(|(i, f)| FolderItem {
                id: i as i32,
                name: f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default().into(),
                path: f.to_string_lossy().into_owned().into(),
                count: videos_label(self.lib.count(f)).into(),
            })
            .collect();
        ui.set_folder_count(folders.len() as i32);
        ui.set_folders(ModelRc::new(VecModel::from(folders)));
        if self.lib.folders.is_empty() {
            ui.set_can_up(false);
            ui.set_empty_add(true);
            ui.set_empty(tr("Choisissez les dossiers où sont vos vidéos.", "Choose the folders holding your videos.").into());
            return;
        }
        if self.lib.videos.is_empty() {
            ui.set_can_up(false);
            ui.set_empty_add(false);
            ui.set_empty(if self.lib.scanning { tr("Recherche des vidéos…", "Looking for videos…") } else { tr("Aucune vidéo dans ces dossiers.", "No video in these folders.") }.into());
            return;
        }
        ui.set_empty("".into());
        let shelf = self.lib.shelf(&self.place);
        self.place = shelf.place.clone();
        ui.set_can_up(self.lib.up(&self.place).is_some());
        ui.set_place_title(shelf.title.clone().into());
        let mut items: Vec<Item> = Vec::with_capacity(shelf.dirs.len() + shelf.videos.len());
        for (i, d) in shelf.dirs.iter().enumerate() {
            items.push(Item {
                kind: 0,
                id: i as i32,
                name: d.name.clone(),
                detail: dir_count(d.videos, d.dirs),
                duration: String::new(),
                progress: 0.0,
                thumb: None,
            });
        }
        for &k in &shelf.videos {
            let v = &self.lib.videos[k];
            let pos = self.positions.get(&v.path).copied().unwrap_or(0.0);
            let progress = if v.duration > 0.0 { (pos / v.duration).clamp(0.0, 1.0) as f32 } else { 0.0 };
            let mut detail = if v.duration > 0.0 { clock(v.duration) } else { String::new() };
            if pos > 5.0 && v.duration > 0.0 {
                detail = format!("{detail}  ·  {} {}", tr("vu jusqu'à", "watched to"), clock(pos));
            }
            items.push(Item {
                kind: 1,
                id: k as i32,
                name: v.title(),
                detail,
                duration: if v.duration > 0.0 { clock(v.duration) } else { String::new() },
                progress,
                thumb: Some(crate::library::thumb_path(v)),
            });
        }
        let cols = if self.lib.grid { self.cols } else { 1 };
        ui.set_rows(ModelRc::new(Rows { items, cols, images: self.images.clone(), notify: ModelNotify::default() }));
    }
}

trait WidthPx {
    fn get_width_px(&self) -> f32;
}

impl WidthPx for LibraryWindow {
    /// Largeur logique de la fenêtre (avant son premier affichage : la largeur préférée).
    fn get_width_px(&self) -> f32 {
        let s = self.window().size();
        let scale = self.window().scale_factor().max(0.5);
        if s.width == 0 { 1280.0 } else { s.width as f32 / scale }
    }
}

fn videos_label(n: usize) -> String {
    format!("{n} {}", if n > 1 { tr("vidéos", "videos") } else { tr("vidéo", "video") })
}

/// « 12 vidéos », « 3 dossiers · 12 vidéos ».
fn dir_count(videos: usize, dirs: usize) -> String {
    let v = videos_label(videos);
    if dirs == 0 {
        v
    } else {
        format!("{dirs} {}  ·  {v}", if dirs > 1 { tr("dossiers", "folders") } else { tr("dossier", "folder") })
    }
}

/// Ce que montre une tuile, avant la lecture de sa miniature.
struct Item {
    kind: i32,
    id: i32,
    name: String,
    detail: String,
    duration: String,
    progress: f32,
    thumb: Option<PathBuf>,
}

/// Miniatures déjà lues (textures), bornées : les plus anciennes partent.
#[derive(Default)]
struct Images {
    map: HashMap<PathBuf, (slint::Image, u64)>,
    tick: u64,
}

impl Images {
    fn get(&mut self, path: &PathBuf) -> Option<slint::Image> {
        self.tick += 1;
        let tick = self.tick;
        if let Some(e) = self.map.get_mut(path) {
            e.1 = tick;
            return Some(e.0.clone());
        }
        let img = read_png(path)?;
        if self.map.len() >= 240 {
            // Les 40 moins récemment vues partent.
            let mut ages: Vec<(u64, PathBuf)> = self.map.iter().map(|(k, v)| (v.1, k.clone())).collect();
            ages.sort_unstable_by_key(|a| a.0);
            for (_, k) in ages.into_iter().take(40) {
                self.map.remove(&k);
            }
        }
        self.map.insert(path.clone(), (img.clone(), tick));
        Some(img)
    }
}

/// PNG RGB(A) 8 bits → image Slint.
fn read_png(path: &PathBuf) -> Option<slint::Image> {
    let f = std::fs::File::open(path).ok()?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(f));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0u8; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let mut px = SharedPixelBuffer::<slint::Rgba8Pixel>::new(w, h);
    let out = px.make_mut_bytes();
    match info.color_type {
        png::ColorType::Rgba => out.copy_from_slice(&buf[..out.len()]),
        png::ColorType::Rgb => {
            for (d, s) in out.chunks_exact_mut(4).zip(buf.chunks_exact(3)) {
                d[..3].copy_from_slice(s);
                d[3] = 255;
            }
        }
        _ => return None,
    }
    Some(slint::Image::from_rgba8(px))
}

/// La grille par rangées : Slint ne demande que les rangées visibles ; leurs miniatures sont
/// lues à ce moment-là (puis gardées dans `Images`).
struct Rows {
    items: Vec<Item>,
    cols: usize,
    images: Rc<RefCell<Images>>,
    notify: ModelNotify,
}

impl Model for Rows {
    type Data = Row;

    fn row_count(&self) -> usize {
        self.items.len().div_ceil(self.cols.max(1))
    }

    fn row_data(&self, row: usize) -> Option<Row> {
        let start = row * self.cols;
        if start >= self.items.len() {
            return None;
        }
        let mut images = self.images.borrow_mut();
        let tiles: Vec<Tile> = self.items[start..(start + self.cols).min(self.items.len())]
            .iter()
            .map(|it| {
                let image = it.thumb.as_ref().and_then(|p| images.get(p));
                Tile {
                    kind: it.kind,
                    id: it.id,
                    name: it.name.clone().into(),
                    detail: it.detail.clone().into(),
                    duration: it.duration.clone().into(),
                    progress: it.progress,
                    has_image: image.is_some(),
                    image: image.unwrap_or_default(),
                }
            })
            .collect();
        Some(Row { tiles: ModelRc::new(VecModel::from(tiles)) })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}
