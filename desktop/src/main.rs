//! bioplay pour le bureau. mpv ouvre et dessine sa fenêtre en gpu-next, avec la configuration
//! et les raccourcis de l'utilisateur ; bioplay y pose son interface (superposition egui) et le
//! verre (shader mpv qui floute la vidéo sous les plaques). Interface masquée : aucun coût.

mod flags;
mod i18n;
mod library;
mod mpv;
mod overlay;
mod player;
mod ui;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use egui::{pos2, vec2, Event, Order, PointerButton, Rect};

use i18n::tr;
use player::{Panel, Player};

const GLASS_SHADER: &str = include_str!("../shaders/glass.glsl");
use library::VIDEO_EXT;
const SUBTITLE_EXT: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "sup"];

/// Raccourcis propres à bioplay. Tout le reste (input.conf, raccourcis par défaut) reste à mpv.
const BINDINGS: &str = "RIGHT script-binding main/right
LEFT script-binding main/left
Ctrl+RIGHT script-binding main/ctrl-right
Ctrl+LEFT script-binding main/ctrl-left
TAB script-binding main/tab
Shift+KP_ADD script-binding main/sub-bigger
Shift+KP_SUBTRACT script-binding main/sub-smaller
Shift+UP script-binding main/sub-up
Shift+DOWN script-binding main/sub-down
MBTN_LEFT script-binding main/mouse-left";
/// Actifs quand la souris est sur l'interface : la molette fait défiler, pas de plein écran.
const HOVER_BINDINGS: &str = "MBTN_LEFT_DBL ignore
MBTN_RIGHT ignore
WHEEL_UP script-binding main/wheel-up
WHEEL_DOWN script-binding main/wheel-down";
/// Actifs quand un tiroir est ouvert : Échap le ferme au lieu de quitter.
const PANEL_BINDINGS: &str = "ESC script-binding main/esc";
/// Accueil, dans un sous-dossier ou fenêtre ouverte : Échap, Retour arrière et le bouton
/// « précédent » de la souris remontent au lieu de quitter.
const BACK_BINDINGS: &str = "ESC script-binding main/esc
BS script-binding main/esc
MBTN_BACK script-binding main/esc";
/// Durée de l'aperçu des sous-titres après un réglage de taille ou de hauteur.
const PREVIEW_LIFE: Duration = Duration::from_millis(1600);
/// Numéro de l'incrustation mpv qui porte l'aperçu.
const PREVIEW_OVERLAY: &str = "7";

/// Durée du mouvement après un cran de molette (défilement animé d'egui compris).
const WHEEL_MOTION: Duration = Duration::from_millis(350);

/// Marge basse des sous-titres (unités mpv : pixels rapportés à une image de 720 lignes).
const SUB_MARGIN: i32 = 46;

enum Message {
    Indexed { path: String, external: Option<String>, tracks: Vec<bioplay_core::Track> },
    Picked(String),
    Folder(PathBuf),
}

struct App {
    player: Player,
    overlay: overlay::Overlay,
    ctx: egui::Context,
    start: Instant,
    events: Vec<Event>,
    pointer: Option<egui::Pos2>,
    tx: Sender<Message>,
    rx: Receiver<Message>,
    waker: mpv::Waker,
    /// Prochaine image de l'interface (animation, horloge), None si rien n'est prévu.
    next_frame: Option<Instant>,
    dirty: bool,
    last_frame: Instant,
    /// Aperçu des sous-titres affiché (incrustation mpv).
    preview_on: bool,
    /// La lecture tournait à l'ouverture du tiroir : elle reprend à sa fermeture.
    resume_after_panel: bool,
    /// Dernier cran de molette : l'interface est dessinée en résolution réduite pendant le mouvement.
    last_wheel: Option<Instant>,
    /// Mesure : durées (ms) de chaque image — interface, triangles, rendu + envoi à mpv.
    profile: Option<Vec<[f64; 3]>>,
    visible: bool,
    shader_opts: String,
    hover_section: bool,
    panel_section: bool,
    back_section: bool,
    last_save: Instant,
    seek_from: Option<f64>,
    indexing: Vec<String>,
    sub_margin: i32,
}

impl App {
    fn open(&mut self, path: &str) {
        let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if SUBTITLE_EXT.contains(&ext.as_str()) {
            let title = Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            self.player.mpv.command(&["sub-add", path, "select", &title]);
            self.player.say(format!("{} : {title}", tr("Sous-titres", "Subtitles")));
        } else {
            self.player.save();
            self.player.embedded.clear();
            self.player.external.clear();
            self.player.panel = None;
            self.player.folders_open = false;
            // Miniatures libérées : la mémoire va au film.
            self.player.library.forget_images();
            self.player.mpv.command(&["loadfile", path, "replace"]);
        }
        self.dirty = true;
    }

    /// Index des sous-titres lu dans un fil à part : la lecture démarre sans l'attendre.
    fn index(&mut self, path: String, external: Option<String>) {
        if external.is_none() {
            self.player.indexing = true;
        }
        let (tx, waker) = (self.tx.clone(), self.waker);
        std::thread::spawn(move || {
            let tracks = std::fs::File::open(&path).ok().and_then(|f| bioplay_core::index(f).ok()).unwrap_or_default();
            let _ = tx.send(Message::Indexed { path, external, tracks });
            waker.wake();
        });
    }

    fn pick(&self) {
        let (tx, waker) = (self.tx.clone(), self.waker);
        std::thread::spawn(move || {
            let f = rfd::FileDialog::new()
                .set_title(tr("Ouvrir une vidéo", "Open a video"))
                .add_filter(tr("Vidéos", "Videos"), VIDEO_EXT)
                .add_filter(tr("Tous les fichiers", "All files"), &["*"])
                .pick_file();
            if let Some(f) = f {
                let _ = tx.send(Message::Picked(f.to_string_lossy().into_owned()));
                waker.wake();
            }
        });
    }

    /// Retour à la bibliothèque : position gardée, lecture arrêtée, mpv revient à l'accueil.
    fn home(&mut self) {
        self.player.save();
        self.player.panel = None;
        self.player.mpv.command(&["stop"]);
        self.dirty = true;
    }

    fn pick_folder(&self) {
        let (tx, waker) = (self.tx.clone(), self.waker);
        std::thread::spawn(move || {
            if let Some(f) = rfd::FileDialog::new().set_title(tr("Ajouter un dossier", "Add a folder")).pick_folder() {
                let _ = tx.send(Message::Folder(f));
                waker.wake();
            }
        });
    }

    fn section(&self, name: &str, on: bool) {
        // allow-hide-cursor : nos sections captent la souris, sans empêcher mpv de masquer le
        // pointeur quand il ne bouge plus.
        if on {
            self.player.mpv.command(&["enable-section", name, "allow-hide-cursor"]);
        } else {
            self.player.mpv.command(&["disable-section", name]);
        }
    }

    fn poke(&mut self) {
        self.player.activity = Instant::now();
        self.dirty = true;
    }

    // --- Raccourcis et souris -------------------------------------------------------------------

    fn binding(&mut self, name: &str, state: &str) {
        if std::env::var_os("BIOPLAY_DEBUG").is_some() {
            eprintln!("raccourci {name} {state} souris {:?}", self.pointer);
        }
        let down = state.starts_with('d') || state.starts_with('r') || state.starts_with('p');
        let up = state.starts_with('u');
        if matches!(name, "mouse-left" | "wheel-up" | "wheel-down") {
            let pos = self.pointer.unwrap_or(egui::Pos2::ZERO);
            let button = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            match name {
                // « p » : appui et relâchement en un seul message (clic bref) → les deux.
                "mouse-left" if state.starts_with('p') => {
                    self.events.push(button(true));
                    self.events.push(button(false));
                }
                "mouse-left" if down || up => self.events.push(button(down)),
                "wheel-up" | "wheel-down" if down => {
                    self.last_wheel = Some(Instant::now());
                    self.events.push(Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: vec2(0.0, if name == "wheel-up" { 1.0 } else { -1.0 }),
                    modifiers: Default::default(),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                    })
                }
                _ => {}
            }
            if name == "mouse-left" && down {
                self.player.activity = Instant::now();
            }
            self.dirty = true;
            return;
        }
        if !down {
            return;
        }
        let p = &mut self.player;
        match name {
            // Flèches : ligne suivante / précédente (au plus 10 s) ; Ctrl : 10 s tout court.
            "right" => p.step_line_or_ten(1),
            "left" => p.step_line_or_ten(-1),
            "ctrl-right" | "ctrl-left" => {
                let dir = if name == "ctrl-right" { 1.0 } else { -1.0 };
                p.mpv.seek(p.mpv.state.position + 10.0 * dir, true);
                p.say(if dir > 0.0 { "+10 s" } else { "−10 s" });
            }
            "sub-bigger" => p.sub_size(1),
            "sub-smaller" => p.sub_size(-1),
            "sub-up" => p.sub_height(1),
            "sub-down" => p.sub_height(-1),
            "tab" => p.panel = if p.panel == Some(Panel::List) { None } else { Some(Panel::List) },
            "esc" => {
                if p.panel.is_some() {
                    p.panel = None;
                } else if p.folders_open {
                    p.folders_open = false;
                } else if let Some(up) = p.library.up(&p.place) {
                    p.place = up;
                }
            }
            _ => {}
        }
        // Le clavier n'affiche pas le contrôleur : seule la bulle confirme l'action.
        self.dirty = true;
    }

    fn mouse_moved(&mut self) {
        let s = &self.player.mpv.state;
        let scale = s.scale as f32;
        match s.mouse {
            Some((x, y)) => {
                let pos = pos2(x as f32 / scale, y as f32 / scale);
                if self.pointer != Some(pos) {
                    self.events.push(Event::PointerMoved(pos));
                    self.pointer = Some(pos);
                    self.poke();
                }
            }
            None => {
                if self.pointer.take().is_some() {
                    self.events.push(Event::PointerGone);
                    self.dirty = true;
                }
            }
        }
    }

    // --- Événements de mpv ----------------------------------------------------------------------

    fn on_change(&mut self, change: mpv::Change) {
        match change {
            mpv::Change::Shutdown | mpv::Change::Ended => {}
            mpv::Change::Binding(name, state) => self.binding(&name, &state),
            mpv::Change::Loaded => {
                self.player.loaded_at = Some(Instant::now());
                self.player.apply_subs();
                let path = self.player.mpv.state.path.clone();
                if Path::new(&path).is_file() {
                    self.index(path, None);
                }
                self.player.save();
                self.dirty = true;
            }
            mpv::Change::Seeking => {
                let own = self.player.mpv.own_seeks.replace(0) > 0;
                if !own && self.seek_from.is_none() {
                    self.seek_from = Some(self.player.mpv.state.position);
                }
            }
            // Saut venu d'un raccourci mpv (flèches haut/bas…) : annoncé dans une bulle.
            mpv::Change::Restarted => {
                if let Some(before) = self.seek_from.take() {
                    let d = self.player.mpv.state.position - before;
                    if d.abs() >= 0.9 && self.player.loaded_at.is_some_and(|t| t.elapsed() > Duration::from_secs(1)) {
                        self.player.say(player::delta(d));
                        self.dirty = true;
                    }
                }
            }
            mpv::Change::Property(name) => self.on_property(name),
        }
    }

    fn on_property(&mut self, name: &'static str) {
        match name {
            "mouse-pos" => return self.mouse_moved(),
            // L'horloge et la barre ne bougent que si l'interface est visible.
            "time-pos" | "demuxer-cache-time" => {
                if self.visible && self.last_frame.elapsed() > Duration::from_millis(33) {
                    self.dirty = true;
                }
                return;
            }
            "track-list" => self.index_external(),
            _ => {}
        }
        self.dirty = true;
        let p = &mut self.player;
        let s = &p.mpv.state;
        let value = match name {
            "aid" => s.aid.map(|v| v as f64).unwrap_or(-1.0),
            "sid" => s.sid.map(|v| v as f64).unwrap_or(-1.0),
            "speed" => s.speed,
            "volume" => s.volume,
            "sub-delay" => s.sub_delay,
            "audio-delay" => s.audio_delay,
            _ => return,
        };
        let settled = p.loaded_at.is_some_and(|t| t.elapsed() > Duration::from_millis(800));
        let previous = p.shown.insert(name, value);
        // Décalage des sous-titres : changer de signe s'arrête d'abord sur 0.
        if name == "sub-delay" && settled {
            if let Some(old) = previous {
                if (old < -1e-6 && value > 1e-6) || (old > 1e-6 && value < -1e-6) {
                    p.mpv.set_f64("sub-delay", 0.0);
                    return;
                }
            }
        }
        if !(settled && previous.is_some_and(|old| (old - value).abs() > 1e-9)) {
            return;
        }
        let msg = match name {
            "speed" => Some(format!("{} {}", tr("Vitesse", "Speed"), player::speed_label(s.speed))),
            "volume" => {
                // Le volume a sa jauge, qui continue au-delà de 100 % jusqu'au maximum de mpv.
                let (v, max) = (s.volume, p.mpv.get("volume-max").and_then(|m| m.parse().ok()).unwrap_or(130.0));
                p.say_gauge(format!("Volume {v:.0} %"), v, max);
                return;
            }
            "sub-delay" => Some(if s.sub_delay.abs() < 1e-6 {
                tr("Sous-titres : aucun décalage", "Subtitles: no delay").into()
            } else {
                format!("{} {:+.0} ms", tr("Sous-titres", "Subtitles"), s.sub_delay * 1000.0)
            }),
            "audio-delay" => Some(format!("Audio {:+.0} ms", s.audio_delay * 1000.0)),
            "aid" => s
                .tracks
                .iter()
                .find(|t| t.kind == "audio" && Some(t.id) == s.aid)
                .map(|t| format!("{} : {}", tr("Audio", "Audio"), mpv::track_label(t))),
            "sid" => Some(match p.sub_track() {
                Some(t) => format!("{} : {}", tr("Sous-titres", "Subtitles"), mpv::track_label(t)),
                None => tr("Sous-titres désactivés", "Subtitles off").into(),
            }),
            _ => None,
        };
        if let Some(m) = msg {
            p.say(m);
        }
    }

    /// Sous-titres ajoutés (glisser-déposer, menu) : indexés dès qu'ils apparaissent.
    fn index_external(&mut self) {
        let files: Vec<String> = self.player.mpv.state.tracks.iter().filter_map(|t| t.external.clone()).collect();
        for f in files {
            if !self.player.external.contains_key(&f) && !self.indexing.contains(&f) && Path::new(&f).is_file() {
                self.indexing.push(f.clone());
                self.index(f.clone(), Some(f));
            }
        }
    }

    fn on_message(&mut self, m: Message) {
        match m {
            Message::Indexed { path, external, tracks } => {
                let p = &mut self.player;
                match external {
                    Some(f) => {
                        self.indexing.retain(|x| x != &f);
                        if let Some(t) = tracks.into_iter().next() {
                            p.external.insert(f, t);
                        }
                    }
                    None if path == p.mpv.state.path => {
                        p.embedded = tracks.into_iter().map(|t| (t.number as i64, t)).collect();
                        p.indexing = false;
                    }
                    None => {}
                }
            }
            Message::Picked(path) => self.open(&path),
            Message::Folder(f) => self.player.library.add_folder(f),
        }
        self.dirty = true;
    }

    // --- Image de l'interface ---------------------------------------------------------------------

    /// Aperçu dessiné comme un sous-titre : même police, taille, couleurs, liseré et place.
    fn show_preview(&mut self, margin: i32, video_h: f64, video_bottom: f64) {
        let s = &self.player.mpv.state;
        let k = video_h / 720.0;
        let size = 36.0 * self.player.sub_scale * k;
        // L'image est centrée dans la fenêtre : le sous-titre aussi.
        let x = s.osd_w / 2.0;
        let y = video_bottom - margin as f64 * k;
        let text: String = self
            .player
            .preview_text()
            .replace('\\', "\\\\")
            .replace('{', "\\{")
            .replace('}', "\\}")
            .replace('\n', "\\N");
        let ass = format!(
            "{{\\an2\\pos({x:.0},{y:.0})\\fnInter Medium\\fs{size:.1}\\bord{:.2}\\shad0\\blur0\\1c&HEBEBEB&\\3c&H0A0A0A&}}{text}",
            1.6 * k
        );
        let (w, h) = (format!("{:.0}", s.osd_w), format!("{:.0}", s.osd_h));
        self.player.mpv.command(&["osd-overlay", PREVIEW_OVERLAY, "ass-events", &ass, &w, &h]);
        self.preview_on = true;
    }

    fn hide_preview(&mut self) {
        if self.preview_on {
            self.player.mpv.command(&["osd-overlay", PREVIEW_OVERLAY, "none", ""]);
            self.preview_on = false;
        }
    }

    fn frame(&mut self) {
        let s = &self.player.mpv.state;
        let (w, h, scale) = (s.osd_w as f32, s.osd_h as f32, s.scale as f32);
        if w < 1.0 || h < 1.0 {
            return;
        }
        let mut raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(w / scale, h / scale))),
            time: Some(self.start.elapsed().as_secs_f64()),
            events: std::mem::take(&mut self.events),
            focused: true,
            ..Default::default()
        };
        raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale);
        let t0 = Instant::now();
        let mut plates = Vec::new();
        let mut actions = Vec::new();
        let player = &mut self.player;
        let mut out = self.ctx.run_ui(raw, |ui| {
            let (p, a) = ui::draw(ui.ctx(), player, scale);
            plates = p;
            actions = a;
        });
        let t1 = Instant::now();
        // Défilement en cours : demi-résolution, puis une image nette juste après l'arrêt.
        let moving = self.last_wheel.is_some_and(|t| t.elapsed() < WHEEL_MOTION);
        let reduce = if moving { 2 } else { 1 };
        if !moving {
            self.last_wheel = None;
        }
        let prims = self.ctx.tessellate(std::mem::take(&mut out.shapes), out.pixels_per_point / reduce as f32);
        let t2 = Instant::now();
        let home = !self.player.has_file();
        self.overlay.present(&self.player.mpv, &prims, &mut out.textures_delta, [w as u32, h as u32], out.pixels_per_point, home, &self.player.thumbs, reduce);
        if let Some(p) = &mut self.profile {
            let ms = |a: Instant, b: Instant| (b - a).as_secs_f64() * 1000.0;
            p.push([ms(t0, t1), ms(t1, t2), ms(t2, Instant::now())]);
        }
        self.visible = !prims.is_empty();
        self.last_frame = Instant::now();

        // Verre : rectangles passés au shader, en pixels de l'image vidéo (hors bandes noires).
        let s = &self.player.mpv.state;
        let mut opts = format!("glass_n={}", plates.len().min(8));
        for (i, g) in plates.iter().take(8).enumerate() {
            opts += &format!(
                ",r{i}x={:.1},r{i}y={:.1},r{i}w={:.1},r{i}h={:.1},r{i}r={:.1},r{i}a={:.3}",
                g.x - s.margin_left as f32,
                g.y - s.margin_top as f32,
                g.w,
                g.h,
                g.radius,
                g.alpha
            );
        }
        if opts != self.shader_opts {
            self.player.mpv.set("glsl-shader-opts", &opts);
            self.shader_opts = opts;
        }

        // La barre ne cache jamais les sous-titres : tant qu'elle est là, ils remontent au-dessus.
        let bottom_ui = plates.iter().filter(|g| g.y + g.h > h - 40.0 * scale && g.alpha > 0.3).map(|g| g.y).fold(f32::MAX, f32::min);
        let video_h = (h as f64 - s.margin_top - s.margin_bottom).max(1.0);
        let video_bottom = h as f64 - s.margin_bottom;
        let base = SUB_MARGIN + self.player.sub_lift;
        let margin = if bottom_ui < f32::MAX {
            let needed = (video_bottom - bottom_ui as f64 + 14.0 * scale as f64) * 720.0 / video_h;
            (needed.ceil() as i32).max(base)
        } else {
            base
        };
        if margin != self.sub_margin {
            self.player.mpv.set("sub-margin-y", &margin.to_string());
            self.sub_margin = margin;
        }

        // Souris sur l'interface : molette et double clic pour elle, pas pour mpv.
        let over = self.pointer.and_then(|p| self.ctx.layer_id_at(p)).is_some_and(|l| l.order != Order::Background);
        self.player.over_ui = over;
        if over != self.hover_section {
            self.section("bioplay-hover", over);
            self.hover_section = over;
        }
        let back = !self.player.has_file() && (self.player.folders_open || self.player.library.up(&self.player.place).is_some());
        if back != self.back_section {
            self.section("bioplay-back", back);
            self.back_section = back;
        }
        let panel = self.player.panel.is_some();
        if panel != self.panel_section {
            self.section("bioplay-panel", panel);
            // Sous un tiroir, les sous-titres (dessinés après le flou) resteraient nets : masqués.
            self.player.mpv.set("sub-visibility", if panel { "no" } else { "yes" });
            // Tiroir ouvert : la lecture s'arrête, toute la machine est à l'interface. Elle
            // reprend à la fermeture si elle tournait avant.
            if panel {
                self.resume_after_panel = !self.player.mpv.state.paused && self.player.has_file();
                if self.resume_after_panel {
                    self.player.mpv.set("pause", "yes");
                }
            } else if std::mem::take(&mut self.resume_after_panel) {
                self.player.mpv.set("pause", "no");
            }
            self.panel_section = panel;
        }

        self.next_frame = out
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.repaint_delay)
            .filter(|d| *d < Duration::from_secs(3600))
            .map(|d| Instant::now() + d.max(Duration::from_millis(8)));
        // Image nette à la fin du mouvement.
        if let Some(t) = self.last_wheel {
            let end = t + WHEEL_MOTION;
            self.next_frame = Some(self.next_frame.map_or(end, |n| n.min(end)));
        }

        // Réglage de taille ou de hauteur : une réplique reste à l'écran le temps de juger,
        // même entre deux sous-titres ou sous un tiroir (là où les vrais sont masqués).
        let preview = self.player.sub_preview.filter(|t| t.elapsed() < PREVIEW_LIFE);
        if let Some(t) = preview {
            let real = !panel && self.player.mpv.state.sid.is_some() && self.player.mpv.get("sub-text").is_some_and(|t| !t.trim().is_empty());
            if real {
                self.hide_preview();
            } else {
                self.show_preview(margin, video_h, video_bottom);
            }
            let end = t + PREVIEW_LIFE;
            self.next_frame = Some(self.next_frame.map_or(end, |n| n.min(end)));
        } else if self.preview_on {
            self.player.sub_preview = None;
            self.hide_preview();
        }
        for a in actions {
            match a {
                ui::Action::Open => self.pick(),
                ui::Action::Home => self.home(),
                ui::Action::AddFolder => self.pick_folder(),
                ui::Action::Play(path) => self.open(&path),
            }
        }
    }

    fn run(&mut self) {
        loop {
            let now = Instant::now();
            // Réveils : prochaine image prévue, masquage de l'interface après inactivité.
            let hide_at = self.player.activity + if self.player.over_ui { ui::HIDE_AFTER_HOVER } else { ui::HIDE_AFTER } + Duration::from_millis(100);
            let mut wake = now + Duration::from_secs(1);
            if let Some(t) = self.next_frame {
                wake = wake.min(t);
            }
            if self.visible && hide_at > now {
                wake = wake.min(hide_at);
            }
            let timeout = if self.dirty { 0.0 } else { wake.saturating_duration_since(now).as_secs_f64() };
            for c in self.player.mpv.wait(timeout) {
                if matches!(c, mpv::Change::Shutdown) {
                    return;
                }
                self.on_change(c);
            }
            while let Ok(m) = self.rx.try_recv() {
                self.on_message(m);
            }
            // Bibliothèque : relecture et miniatures en pause pendant la lecture.
            self.player.library.set_hold(self.player.has_file());
            if self.player.library.poll() && !self.player.has_file() {
                self.dirty = true;
            }
            let now = Instant::now();
            if self.next_frame.is_some_and(|t| t <= now) || (self.visible && hide_at <= now) {
                self.dirty = true;
            }
            if self.dirty {
                self.dirty = false;
                self.frame();
            }
            // Position notée toutes les 15 s pendant la lecture : reprise même après un arrêt brutal.
            if self.player.has_file() && !self.player.mpv.state.paused && self.last_save.elapsed() > Duration::from_secs(15) {
                self.player.save();
                self.last_save = Instant::now();
            }
        }
    }
}

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

fn main() {
    i18n::init();
    let mpv = mpv::Mpv::new().expect("libmpv");
    // Options mpv en ligne de commande, comme mpv : `bioplay --mute=yes --start=20 film.mkv`.
    let mut files = Vec::new();
    let mut panel = None;
    for a in std::env::args().skip(1) {
        if let Some(o) = a.strip_prefix("--") {
            match o.split_once('=') {
                Some(("panel", v)) => panel = Some(v.to_owned()),
                Some((k, v)) => mpv.set(k, v),
                None => mpv.set(o, "yes"),
            }
        } else {
            files.push(a);
        }
    }

    // Le verre : shader ajouté à ceux de l'utilisateur, inactif tant que glass_n vaut 0.
    let shader = runtime_dir().join("bioplay-glass.glsl");
    let _ = std::fs::write(&shader, GLASS_SHADER);
    mpv.command_sync(&["change-list", "glsl-shaders", "append", &shader.to_string_lossy()]);
    mpv.command_sync(&["define-section", "bioplay", BINDINGS, "force"]);
    mpv.command_sync(&["define-section", "bioplay-hover", HOVER_BINDINGS, "force"]);
    mpv.command_sync(&["define-section", "bioplay-panel", PANEL_BINDINGS, "force"]);
    mpv.command_sync(&["define-section", "bioplay-back", BACK_BINDINGS, "force"]);
    mpv.command_sync(&["enable-section", "bioplay", "allow-hide-cursor"]);

    let overlay = overlay::Overlay::new().expect("contexte GPU de l'interface");
    let ctx = egui::Context::default();
    ui::fonts(&ctx);
    let (tx, rx) = channel();
    let waker = mpv.waker();
    let mut app = App {
        player: Player::new(mpv),
        overlay,
        ctx,
        start: Instant::now(),
        events: Vec::new(),
        pointer: None,
        tx,
        rx,
        waker,
        next_frame: None,
        dirty: true,
        last_frame: Instant::now(),
        visible: false,
        shader_opts: String::new(),
        hover_section: false,
        panel_section: false,
        back_section: false,
        last_save: Instant::now(),
        seek_from: None,
        indexing: Vec::new(),
        sub_margin: SUB_MARGIN,
        preview_on: false,
        resume_after_panel: false,
        profile: None,
        last_wheel: None,
    };
    for (i, f) in files.iter().enumerate() {
        if i == 0 {
            app.open(f);
        } else {
            app.player.mpv.command(&["loadfile", f, "append"]);
        }
    }
    if let Ok(mode) = std::env::var("BIOPLAY_BENCH") {
        bench(&mut app, &mode);
        return;
    }
    app.player.panel = match panel.as_deref() {
        Some("list") => Some(Panel::List),
        Some("tracks") => Some(Panel::Tracks),
        Some("speed") => Some(Panel::Speed),
        Some("zoom") => Some(Panel::Zoom),
        _ => None,
    };
    app.run();
    app.player.save();
    app.overlay.hide(&app.player.mpv);
}

/// Mesure du défilement de l'accueil, sans fenêtre : `BIOPLAY_BENCH=grid|list bioplay`.
/// Écran simulé de 2880×1800 (échelle 2), vraie bibliothèque, un cran de molette toutes les
/// 6 images. Affiche les durées par étape (médiane, 90e centile, maximum).
fn bench(app: &mut App, mode: &str) {
    app.player.library.set_hold(true);
    // Écran simulé, réimposé à chaque image (mpv sans fenêtre le remet à zéro).
    let screen = |app: &mut App| {
        let s = &mut app.player.mpv.state;
        s.osd_w = 2880.0;
        s.osd_h = 1800.0;
        s.scale = 2.0;
    };
    app.player.library.grid = mode != "list";
    if let Ok(path) = std::env::var("BIOPLAY_PLACE") {
        app.player.place.path = path;
    }
    app.events.push(Event::PointerMoved(pos2(720.0, 500.0)));
    for _ in 0..20 {
        // Laisse mpv charger un éventuel fichier (vérification des commandes de lecture).
        for c in app.player.mpv.wait(0.05) {
            app.on_change(c);
        }
        screen(app);
        app.frame();
    }
    if let Some(f) = std::env::var_os("BIOPLAY_SNAPSHOT") {
        app.overlay.snapshot(Path::new(&f));
        return;
    }
    app.profile = Some(Vec::new());
    for i in 0..360 {
        if i % 6 == 0 {
            app.last_wheel = Some(Instant::now());
            app.events.push(Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, if i < 180 { -1.0 } else { 1.0 }),
                modifiers: Default::default(),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
            });
        }
        screen(app);
        app.frame();
    }
    let p = app.profile.take().unwrap_or_default();
    println!("{} vidéos, mode {mode}, {} images", app.player.library.videos.len(), p.len());
    for (k, name) in ["interface", "triangles", "rendu+envoi"].iter().enumerate() {
        let mut v: Vec<f64> = p.iter().map(|x| x[k]).collect();
        v.sort_by(|a, b| a.total_cmp(b));
        let at = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
        println!("{name:>12} : médiane {:.1} ms, 90 % {:.1} ms, max {:.1} ms", at(0.5), at(0.9), at(1.0));
    }
}
