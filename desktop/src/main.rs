//! bioplay pour le bureau. mpv ouvre et dessine sa fenêtre en gpu-next, avec la configuration
//! et les raccourcis de l'utilisateur ; bioplay y pose son interface (superposition egui) et le
//! verre (shader mpv qui floute la vidéo sous les plaques). Interface masquée : aucun coût.

mod i18n;
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
const VIDEO_EXT: &[&str] = &["mkv", "mp4", "m4v", "avi", "webm", "mov", "ts", "m2ts", "wmv", "flv", "mpg", "mpeg", "ogv", "3gp"];
const SUBTITLE_EXT: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "sup"];

/// Raccourcis propres à bioplay. Tout le reste (input.conf, raccourcis par défaut) reste à mpv.
const BINDINGS: &str = "RIGHT script-binding main/right
LEFT script-binding main/left
Ctrl+RIGHT script-binding main/ctrl-right
Ctrl+LEFT script-binding main/ctrl-left
TAB script-binding main/tab
MBTN_LEFT script-binding main/mouse-left";
/// Actifs quand la souris est sur l'interface : la molette fait défiler, pas de plein écran.
const HOVER_BINDINGS: &str = "MBTN_LEFT_DBL ignore
MBTN_RIGHT ignore
WHEEL_UP script-binding main/wheel-up
WHEEL_DOWN script-binding main/wheel-down";
/// Actifs quand un tiroir est ouvert : Échap le ferme au lieu de quitter.
const PANEL_BINDINGS: &str = "ESC script-binding main/esc";
/// Marge basse des sous-titres (unités mpv : pixels rapportés à une image de 720 lignes).
const SUB_MARGIN: i32 = 46;

enum Message {
    Indexed { path: String, external: Option<String>, tracks: Vec<bioplay_core::Track> },
    Picked(String),
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
    visible: bool,
    shader_opts: String,
    hover_section: bool,
    panel_section: bool,
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
                "wheel-up" | "wheel-down" if down => self.events.push(Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: vec2(0.0, if name == "wheel-up" { 1.0 } else { -1.0 }),
                    modifiers: Default::default(),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Wheel,
                }),
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
            "tab" => p.panel = if p.panel == Some(Panel::List) { None } else { Some(Panel::List) },
            "esc" => p.panel = None,
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
        }
        self.dirty = true;
    }

    // --- Image de l'interface ---------------------------------------------------------------------

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
        let mut plates = Vec::new();
        let mut actions = Vec::new();
        let player = &mut self.player;
        let mut out = self.ctx.run_ui(raw, |ui| {
            let (p, a) = ui::draw(ui.ctx(), player, scale);
            plates = p;
            actions = a;
        });
        let prims = self.ctx.tessellate(std::mem::take(&mut out.shapes), out.pixels_per_point);
        let home = !self.player.has_file();
        self.overlay.present(&self.player.mpv, &prims, &mut out.textures_delta, [w as u32, h as u32], out.pixels_per_point, home);
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
        let margin = if bottom_ui < f32::MAX {
            let needed = (video_bottom - bottom_ui as f64 + 14.0 * scale as f64) * 720.0 / video_h;
            (needed.ceil() as i32).max(SUB_MARGIN)
        } else {
            SUB_MARGIN
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
        let panel = self.player.panel.is_some();
        if panel != self.panel_section {
            self.section("bioplay-panel", panel);
            // Sous un tiroir, les sous-titres (dessinés après le flou) resteraient nets : masqués.
            self.player.mpv.set("sub-visibility", if panel { "no" } else { "yes" });
            self.panel_section = panel;
        }

        self.next_frame = out
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.repaint_delay)
            .filter(|d| *d < Duration::from_secs(3600))
            .map(|d| Instant::now() + d.max(Duration::from_millis(8)));
        for a in actions {
            match a {
                ui::Action::Open => self.pick(),
                ui::Action::Play(path) => self.open(&path),
                ui::Action::Forget(path) => self.player.forget(&path),
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
        last_save: Instant::now(),
        seek_from: None,
        indexing: Vec::new(),
        sub_margin: SUB_MARGIN,
    };
    for (i, f) in files.iter().enumerate() {
        if i == 0 {
            app.open(f);
        } else {
            app.player.mpv.command(&["loadfile", f, "append"]);
        }
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
