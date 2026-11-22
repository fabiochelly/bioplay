//! bioplay pour le bureau. mpv ouvre et dessine sa fenêtre en gpu-next, avec la configuration
//! et les raccourcis de l'utilisateur ; bioplay y pose son interface (superposition egui) et le
//! verre (shader mpv qui floute la vidéo sous les plaques). Interface masquée : aucun coût.

mod flags;
mod i18n;
mod library;
mod mpv;
mod overlay;
mod player;
mod trace;
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
    Keyframes { path: String, keyframes: Vec<f64> },
    Picked(String),
    Folder(PathBuf),
}

struct App {
    /// Compteurs (mesures) : tours de boucle et images dessinées.
    wakeups: u64,
    frames: u64,
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
    /// Image en attente : `overlay_done` au moment où mpv n'avait pas encore lu le tampon.
    overlay_wait: Option<u64>,
    /// Prochain changement de l'horloge ou de la barre (réveil prévu).
    change_at: Option<Instant>,
    /// Horloge et barre telles que dessinées en dernier (`display_sig`).
    drawn_sig: [i64; 3],
    /// Raisons des images demandées depuis la dernière (mesure).
    why: Vec<String>,
    /// Instant de masquage des commandes déjà traité (il change avec l'activité ou le survol).
    hidden_for: Option<Instant>,
    /// Mesure : envoi du dernier saut, et durées appui → reprise de l'image.
    seek_clock: Option<Instant>,
    seek_times: Vec<f64>,
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
        self.redraw("ouverture d'un fichier");
    }

    /// Index des sous-titres lu dans un fil à part : la lecture démarre sans l'attendre.
    fn index(&mut self, path: String, external: Option<String>) {
        if external.is_none() {
            self.player.indexing = true;
        }
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
        std::thread::spawn(move || {
            // Images clés d'abord, envoyées seules (index du fichier, ≈ 1 ms) : les sauts rapides
            // marchent tout de suite, même pendant un long index des sous-titres.
            if external.is_none() {
                let keyframes = std::fs::File::open(&path).map(bioplay_core::keyframes).unwrap_or_default();
                let _ = tx.send(Message::Keyframes { path: path.clone(), keyframes });
                waker.wake();
            }
            let tracks = std::fs::File::open(&path).ok().and_then(|f| bioplay_core::index(f).ok()).unwrap_or_default();
            let _ = tx.send(Message::Indexed { path, external, tracks });
            waker.wake();
        });
    }

    fn pick(&self) {
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
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
        self.redraw("retour à l'accueil");
    }

    fn pick_folder(&self) {
        let (tx, waker) = (self.tx.clone(), self.waker.clone());
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

    /// Une image est à refaire, pour la raison `why` (comptée en mesure).
    fn redraw(&mut self, why: &str) {
        self.dirty = true;
        if trace::on() {
            self.why.push(why.to_owned());
        }
    }

    fn redraw_with(&mut self, why: impl FnOnce() -> String) {
        self.dirty = true;
        if trace::on() {
            self.why.push(why());
        }
    }

    fn poke(&mut self) {
        self.player.activity = Instant::now();
        self.redraw("activité (souris, clic)");
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
            self.redraw("souris (clic, molette)");
            return;
        }
        if !down {
            return;
        }
        // Touche : position exacte avant de calculer un saut (réplique suivante, 10 s).
        if !self.player.mpv.is_seeking() && self.player.mpv.target.get().is_none_or(|t| t.done) {
            self.player.mpv.refresh_position();
        }
        let p = &mut self.player;
        match name {
            // Flèches : ligne suivante / précédente (au plus 10 s) ; Ctrl : 10 s tout court.
            "right" => p.step_line_or_ten(1),
            "left" => p.step_line_or_ten(-1),
            "ctrl-right" | "ctrl-left" => {
                let dir = if name == "ctrl-right" { 1.0 } else { -1.0 };
                let from = p.mpv.pos();
                p.seek_span(from, from + 10.0 * dir);
                let target = p.mpv.target.get().map_or(from, |t| t.at);
                p.say_seek(dir, target, player::delta(target - from));
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
        self.redraw("touche");
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
                    self.redraw("souris sortie");
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
                self.player.keyframes.clear();
                self.player.mpv.target.set(None);
                self.player.apply_subs();
                let path = self.player.mpv.state.path.clone();
                if Path::new(&path).is_file() {
                    self.index(path, None);
                }
                self.player.save();
                self.redraw("fichier chargé");
            }
            mpv::Change::Seeking => {
                let own = self.player.mpv.own_seeks.replace(0) > 0;
                if !own && self.seek_from.is_none() {
                    self.seek_from = Some(self.player.mpv.position());
                }
            }
            // Saut venu d'un raccourci mpv (flèches haut/bas…) : annoncé dans une bulle.
            // (Une image retenue pendant un saut précis part d'elle-même à la reprise.)
            mpv::Change::Restarted => {
                if let Some(t) = self.seek_clock.take() {
                    self.seek_times.push(t.elapsed().as_secs_f64() * 1000.0);
                }
                if let Some(before) = self.seek_from.take() {
                    let d = self.player.mpv.position() - before;
                    if d.abs() >= 0.9 && self.player.loaded_at.is_some_and(|t| t.elapsed() > Duration::from_secs(1)) {
                        self.player.say_seek(d, self.player.mpv.position(), player::delta(d));
                        self.redraw("bulle d'un saut venu de mpv");
                    }
                }
            }
            mpv::Change::Property(name) => self.on_property(name),
        }
    }

    fn on_property(&mut self, name: &'static str) {
        match name {
            "mouse-pos" => return self.mouse_moved(),
            "track-list" => self.index_external(),
            _ => {}
        }
        self.redraw_with(|| format!("propriété mpv {name}"));
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
            Message::Keyframes { path, keyframes } => {
                if path == self.player.mpv.state.path {
                    self.player.keyframes = keyframes;
                }
            }
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
        self.redraw("message (index, fichier choisi, dossier)");
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

    /// Prochain instant où l'horloge (seconde écoulée ou restante) ou la barre (un pixel) change,
    /// commandes visibles et lecture en cours. La position n'est plus suivie image par image
    /// (des dizaines de réveils par seconde) : elle est déduite de l'horloge. Tiroir ouvert
    /// (liste des sous-titres qui suit la lecture) : 30 images par seconde.
    fn next_display_change(&self) -> Option<Instant> {
        let s = &self.player.mpv.state;
        if !self.visible || !self.player.has_file() || s.paused || s.speed <= 0.0 {
            return None;
        }
        if self.player.panel.is_some() {
            return Some(self.last_frame + Duration::from_millis(33));
        }
        let pos = self.player.mpv.position();
        let dur = s.duration.max(0.001);
        let px = s.osd_w.max(1.0) / dur;
        // Temps jusqu'au prochain entier (secondes écoulées) ; le restant change, lui, quand sa
        // partie fractionnaire s'épuise. Une valeur pile sur la limite compte pour un tour entier.
        let up = |v: f64| {
            let f = 1.0 - v.fract();
            if f < 1e-4 { 1.0 } else { f }
        };
        let down = |v: f64| {
            let f = v.fract();
            if f < 1e-4 { 1.0 } else { f }
        };
        let pixel = up(pos * px) / px;
        // + 3 ms : la position déduite et celle de mpv diffèrent de quelques millisecondes ; sans
        // marge, le réveil tombe parfois juste avant le changement et ne sert à rien.
        let dt = up(pos).min(down(dur - pos)).min(pixel).max(0.004) / s.speed + 0.003;
        Some(Instant::now() + Duration::from_secs_f64(dt).min(Duration::from_secs(1)))
    }

    /// Un saut précis n'est pas fini : pas d'image d'interface d'ici là. Au plus 1,5 s, au cas
    /// où mpv ne signalerait pas la reprise.
    fn seek_hold(&self) -> bool {
        self.player.has_file() && self.player.mpv.target.get().is_some_and(|t| t.exact && !t.done && t.sent.elapsed() < Duration::from_millis(1500))
    }

    /// Ce que montrent l'horloge et la barre : secondes écoulées et restantes, pixel de la barre.
    fn display_sig(&self) -> [i64; 3] {
        let s = &self.player.mpv.state;
        let pos = self.player.mpv.position();
        let dur = s.duration.max(0.001);
        let px = s.osd_w.max(1.0) / dur;
        [pos.floor() as i64, (dur - pos).floor() as i64, (pos * px).floor() as i64]
    }

    fn frame(&mut self) {
        self.frames += 1;
        if trace::on() {
            let mut why = std::mem::take(&mut self.why);
            why.sort();
            why.dedup();
            if why.is_empty() {
                trace::count("image : (sans cause notée)");
            }
            for w in why {
                trace::count(&format!("image : {w}"));
            }
        }
        // Position déduite (non suivie) mise à jour pour l'affichage ; cache relu, commandes
        // visibles seulement (une lecture brève par image, hors saut).
        // Commandes visibles (≈ 4 images/s) : position et cache relus chez mpv, ce qui recale aussi
        // la position déduite (un blocage de lecture la ferait sinon dériver).
        if self.visible && self.player.has_file() && !self.player.mpv.is_seeking() {
            self.player.mpv.refresh_position();
            if let Some(c) = self.player.mpv.get("demuxer-cache-time").and_then(|v| v.parse::<f64>().ok()) {
                self.player.mpv.state.cached = c;
            }
        }
        let pos = self.player.mpv.position();
        self.player.mpv.state.position = pos;
        self.drawn_sig = self.display_sig();
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
        let shapes = std::mem::take(&mut out.shapes);
        let marks = overlay::shape_marks(&shapes);
        let prims = self.ctx.tessellate(shapes, out.pixels_per_point / reduce as f32);
        let t2 = Instant::now();
        let home = !self.player.has_file();
        let presented = self.overlay.present(&self.player.mpv, &prims, &marks, &mut out.textures_delta, [w as u32, h as u32], out.pixels_per_point, home, &self.player.thumbs, reduce);
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
            .map(|d| Instant::now() + d.max(Duration::from_millis(16)));
        if trace::on() && self.next_frame.is_some() {
            for c in self.ctx.repaint_causes() {
                trace::count(&format!("egui redemande une image : {c}"));
            }
        }
        // Image pas encore posée (mpv n'avait pas lu la précédente) : on la refait dès sa
        // réponse (qui réveille la boucle), sans tourner en attendant.
        if !presented {
            self.overlay_wait = Some(self.player.mpv.overlay_done.get());
        }
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
        while self.step(None) {}
    }

    /// Un tour de boucle : attend le prochain réveil (au plus `limit`), traite, redessine si besoin.
    /// Faux à la fermeture de mpv.
    fn step(&mut self, limit: Option<Duration>) -> bool {
        {
            let now = Instant::now();
            // Réveils : prochaine image prévue, masquage de l'interface après inactivité.
            let hide_at = self.player.activity + if self.player.over_ui { ui::HIDE_AFTER_HOVER } else { ui::HIDE_AFTER } + Duration::from_millis(100);
            // Rien de prévu : on dort jusqu'au prochain événement (mpv, fichier choisi, bibliothèque),
            // sans réveil périodique. En lecture, seulement la note de la position toutes les 15 s.
            let mut wake = now + Duration::from_secs(3600);
            let mut cause = "rien de prévu";
            let mut sooner = |w: &mut Instant, t: Instant, why: &'static str| {
                if t < *w {
                    *w = t;
                    cause = why;
                }
            };
            if self.player.has_file() && !self.player.mpv.state.paused {
                sooner(&mut wake, self.last_save + Duration::from_secs(15), "note de la position (15 s)");
            }
            if let Some(t) = self.next_frame {
                sooner(&mut wake, t, "demande d'egui");
            }
            if self.visible && hide_at > now {
                sooner(&mut wake, hide_at, "masquage des commandes");
            }
            // Horloge et barre : réveil à l'instant exact où l'une change (seconde, pixel).
            self.change_at = self.next_display_change();
            if let Some(t) = self.change_at {
                sooner(&mut wake, t, "horloge ou barre");
            }
            if let Some(l) = limit {
                sooner(&mut wake, now + l, "mesure");
            }
            // Saut en cours (lecture) : l'interface attend la nouvelle image. Redessiner pendant
            // qu'un saut précis vide la file de mpv lui fait afficher du noir (clignotement).
            let hold = self.seek_hold();
            // Retenue : on dort jusqu'à la reprise (événement de mpv), au plus 1,5 s ; les réveils
            // prévus (fondus, horloge) attendront.
            if let Some(t) = self.player.mpv.target.get().filter(|_| hold) {
                wake = t.sent + Duration::from_millis(1500);
                cause = "fin de retenue d'un saut";
                if let Some(l) = limit {
                    wake = wake.min(now + l);
                }
            }
            let timeout = if self.dirty && !hold { 0.0 } else { wake.saturating_duration_since(now).as_secs_f64() };
            self.wakeups += 1;
            let changes = self.player.mpv.wait(timeout);
            if trace::on() {
                let early = Instant::now() + Duration::from_millis(2) < wake;
                let what = if timeout == 0.0 {
                    "réveil : image à faire".to_owned()
                } else if !changes.is_empty() {
                    "réveil : événement de mpv".to_owned()
                } else if early {
                    "réveil : réponse de mpv (envoi d'image, réglage)".to_owned()
                } else {
                    format!("réveil : minuterie ({cause})")
                };
                trace::count(&what);
            }
            for c in changes {
                if matches!(c, mpv::Change::Shutdown) {
                    return false;
                }
                self.on_change(c);
            }
            while let Ok(m) = self.rx.try_recv() {
                trace::count("message d'un autre fil (index, bibliothèque, sélecteur)");
                self.on_message(m);
            }
            // Réveil prévu : redessin seulement si l'affichage change vraiment (une position
            // recalée peut avancer ou retarder l'instant prévu de quelques millisecondes).
            // mpv rend la main un rien avant l'échéance demandée : 2 ms d'avance comptent comme
            // l'échéance (sinon un second réveil, juste pour attendre le reste).
            let due = Instant::now() + Duration::from_millis(2);
            if self.change_at.is_some_and(|t| t <= due) {
                self.change_at = None;
                if self.player.panel.is_some() || self.display_sig() != self.drawn_sig {
                    self.redraw("horloge ou barre (seconde, pixel)");
                }
            }
            self.overlay.release(&self.player.mpv);
            if self.overlay_wait.is_some_and(|d| self.player.mpv.overlay_done.get() > d) {
                self.overlay_wait = None;
                self.redraw("image en attente (mpv occupé)");
            }
            // Bibliothèque : relecture et miniatures en pause pendant la lecture.
            self.player.library.set_hold(self.player.has_file());
            if self.player.library.poll() && !self.player.has_file() {
                self.redraw("bibliothèque (nouvelles données)");
            }
            let now = Instant::now() + Duration::from_millis(2);
            if self.next_frame.is_some_and(|t| t <= now) {
                self.redraw("demande d'egui (animation, bulle)");
            }
            // Une seule image pour lancer le masquage (le fondu se poursuit de lui-même), pas une
            // par tour de boucle tant qu'une bulle garde l'interface visible.
            if self.visible && hide_at <= now && self.hidden_for != Some(hide_at) {
                self.hidden_for = Some(hide_at);
                self.redraw("masquage des commandes");
            }
            if self.dirty && !self.seek_hold() {
                self.dirty = false;
                self.frame();
            }
            // Position notée toutes les 15 s pendant la lecture : reprise même après un arrêt brutal.
            if self.player.has_file() && !self.player.mpv.state.paused && self.last_save.elapsed() > Duration::from_secs(15) {
                self.player.save();
                self.last_save = Instant::now();
            }
        }
        true
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
        wakeups: 0,
        frames: 0,
        dirty: true,
        last_frame: Instant::now(),
        overlay_wait: None,
        change_at: None,
        drawn_sig: [0; 3],
        why: Vec::new(),
        hidden_for: None,
        seek_clock: None,
        seek_times: Vec::new(),
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
    // Bibliothèque relue en fond, mais pas pendant la lecture d'un fichier donné au lancement :
    // en pause dès le départ (sans quoi la relecture concurrencerait l'ouverture du film).
    app.player.library.set_hold(!files.is_empty());
    app.player.library.refresh();
    if let Ok(mode) = std::env::var("BIOPLAY_BENCH") {
        if mode == "play" {
            bench_play(&mut app);
            return;
        }
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

/// Temps processeur (ms) du processus entier et du fil principal, lus dans /proc.
fn cpu_ms() -> (f64, f64) {
    let read = |p: &str| {
        let s = std::fs::read_to_string(p).unwrap_or_default();
        let f: Vec<&str> = s.rsplit_once(')').map_or("", |r| r.1).split_whitespace().collect();
        let t = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        (t(11) + t(12)) * 10.0 // utime + stime, en tops de 10 ms
    };
    (read("/proc/self/stat"), read("/proc/thread-self/stat"))
}

/// Mesure de la lecture, sans fenêtre : `BIOPLAY_BENCH=play bioplay film.mkv`. Vraie boucle,
/// vraies durées ; compte réveils, images dessinées, envois à mpv et temps processeur.
fn bench_play(app: &mut App) {
    let screen = |app: &mut App| {
        let s = &mut app.player.mpv.state;
        s.osd_w = 2880.0;
        s.osd_h = 1800.0;
        s.scale = 2.0;
    };
    trace::enable();
    let t = Instant::now();
    while app.player.mpv.state.duration <= 0.0 && t.elapsed() < Duration::from_secs(if app.player.has_file() { 10 } else { 2 }) {
        screen(app);
        app.step(Some(Duration::from_millis(20)));
    }
    let secs = std::env::var("BIOPLAY_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(10.0);
    let phases: &[&str] = if app.player.mpv.state.duration > 0.0 {
        &["commandes visibles", "commandes masquées", "pause, commandes masquées", "pause, commandes visibles", "sauts → (2 par s)", "→ maintenue (30 par s)"]
    } else {
        &["accueil au repos"]
    };
    for &phase in phases {
        let pause = phase.starts_with("pause");
        if app.player.mpv.state.duration > 0.0 && app.player.mpv.state.paused != pause {
            app.player.mpv.set("pause", if pause { "yes" } else { "no" });
        }
        // Commandes masquées : souris partie, inactivité ancienne.
        app.events.push(Event::PointerGone);
        app.pointer = None;
        app.player.activity = Instant::now() - Duration::from_secs(10);
        for _ in 0..20 {
            screen(app);
            app.step(Some(Duration::from_millis(20)));
        }
        if phase.ends_with("commandes visibles") {
            // Souris bougée au milieu de l'image : les commandes apparaissent.
            app.events.push(Event::PointerMoved(pos2(720.0, 450.0)));
            app.player.activity = Instant::now();
            app.dirty = true;
            for _ in 0..10 {
                screen(app);
                app.step(Some(Duration::from_millis(20)));
            }
        }
        let threads0 = threads_ms();
        let switches0 = switches();
        trace::take();
        app.seek_times.clear();
        app.seek_clock = None;
        let (w0, f0, s0, (c0, m0)) = (app.wakeups, app.frames, app.overlay.sends, cpu_ms());
        let start = Instant::now();
        let mut key = start;
        while start.elapsed().as_secs_f64() < secs {
            screen(app);
            match phase {
                "commandes visibles" | "pause, commandes visibles" => app.player.activity = Instant::now(),
                "commandes masquées" | "pause, commandes masquées" | "accueil au repos" => {}
                _ => {
                    let every = if phase.contains("30") { 33 } else { 500 };
                    if Instant::now() >= key {
                        let state = if phase.contains("30") && key > start { "r" } else { "d" };
                        if app.seek_clock.is_none() {
                            app.seek_clock = Some(Instant::now());
                        }
                        app.binding("right", state);
                        app.dirty = true;
                        key += Duration::from_millis(every);
                    }
                }
            }
            // Sans sauts à envoyer : la boucle dort jusqu'à la fin de la phase (comme en vrai).
            let until = if phase.contains("saut") || phase.contains("maintenue") { key } else { start + Duration::from_secs_f64(secs) };
            app.step(Some(until.saturating_duration_since(Instant::now()).max(Duration::from_millis(1))));
        }
        let el = start.elapsed().as_secs_f64();
        let (c1, m1) = cpu_ms();
        println!(
            "{phase:24} réveils {:6.1}/s  images {:5.1}/s  envois {:5.1}/s  CPU processus {:5.1} %  fil principal {:5.1} %  position {:.0} s",
            (app.wakeups - w0) as f64 / el,
            (app.frames - f0) as f64 / el,
            (app.overlay.sends - s0) as f64 / el,
            (c1 - c0) / el / 10.0,
            (m1 - m0) / el / 10.0,
            app.player.mpv.position(),
        );
        if !app.seek_times.is_empty() {
            let mut v = app.seek_times.clone();
            v.sort_by(|a, b| a.total_cmp(b));
            println!("{:24}   saut appui → image : médiane {:.0} ms, 90 % {:.0} ms ({} sauts)", "", v[v.len() / 2], v[v.len() * 9 / 10], v.len());
        }
        // Causes : chaque réveil, image, appel et événement de mpv, par seconde.
        let mut c: Vec<(String, u64)> = trace::take().into_iter().collect();
        c.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for (what, n) in &c {
            println!("{:28}{:8.2}/s  {what}", "", *n as f64 / el);
        }
        // Réveils de chaque fil, vus par le noyau (changements de contexte volontaires) : tout
        // le processus, mpv compris, même ce que bioplay ne compte pas lui-même.
        let mut by: std::collections::HashMap<String, f64> = Default::default();
        for (tid, (name, n)) in switches() {
            let before = switches0.get(&tid).map_or(0.0, |t| t.1);
            *by.entry(name).or_default() += n - before;
        }
        let mut v: Vec<_> = by.into_iter().filter(|(_, n)| *n > 0.0).collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        let list: Vec<String> = v.iter().map(|(n, k)| format!("{n} {:.1}", k / el)).collect();
        println!("{:28}réveils des fils /s : {}", "", list.join(" · "));
        // Fils les plus coûteux (par nom), en % d'un cœur.
        let mut by: std::collections::HashMap<String, f64> = Default::default();
        for (tid, (name, ms)) in threads_ms() {
            let before = threads0.get(&tid).map_or(0.0, |t| t.1);
            *by.entry(name).or_default() += ms - before;
        }
        let mut v: Vec<_> = by.into_iter().filter(|(_, ms)| *ms > 0.0).collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1));
        let top: Vec<String> = v.iter().take(6).map(|(n, ms)| format!("{n} {:.1} %", ms / el / 10.0)).collect();
        println!("{:24}   {}", "", top.join(" · "));
    }
}

/// Réveils de chaque fil (changements de contexte volontaires, /proc), avec son nom.
fn switches() -> std::collections::HashMap<String, (String, f64)> {
    let mut out = std::collections::HashMap::new();
    for e in std::fs::read_dir("/proc/self/task").into_iter().flatten().flatten() {
        let s = std::fs::read_to_string(e.path().join("status")).unwrap_or_default();
        let name = s.lines().find_map(|l| l.strip_prefix("Name:")).unwrap_or("").trim().to_owned();
        let n = s.lines().find_map(|l| l.strip_prefix("voluntary_ctxt_switches:")).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(0.0);
        out.insert(e.file_name().to_string_lossy().into_owned(), (name, n));
    }
    out
}

/// Temps processeur de chaque fil (ms), avec son nom.
fn threads_ms() -> std::collections::HashMap<String, (String, f64)> {
    let mut out = std::collections::HashMap::new();
    for e in std::fs::read_dir("/proc/self/task").into_iter().flatten().flatten() {
        let s = std::fs::read_to_string(e.path().join("stat")).unwrap_or_default();
        let Some((head, rest)) = s.rsplit_once(')') else { continue };
        let name = head.split_once('(').map_or("", |h| h.1).to_owned();
        let f: Vec<&str> = rest.split_whitespace().collect();
        let t = |i: usize| f.get(i).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        out.insert(e.file_name().to_string_lossy().into_owned(), (name, (t(11) + t(12)) * 10.0));
    }
    out
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
    // Capture d'une bulle de saut (vérification) : BIOPLAY_SEEK=-1 ou 1.
    if let Some(d) = std::env::var("BIOPLAY_SEEK").ok().and_then(|v| v.parse::<f64>().ok()) {
        app.player.say_seek(d, 754.0, if d > 0.0 { "Next subtitle" } else { "−10 s" });
        app.player.activity = Instant::now() - Duration::from_secs(10);
        for _ in 0..12 {
            std::thread::sleep(Duration::from_millis(20));
            let _ = app.player.mpv.wait(0.001);
            screen(app);
            app.frame();
        }
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
        // Réponses de mpv (image précédente lue), comme la vraie boucle.
        let _ = app.player.mpv.wait(0.005);
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
