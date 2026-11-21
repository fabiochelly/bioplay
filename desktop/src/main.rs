//! bioplay pour le bureau : mpv (configuration et raccourcis de l'utilisateur) rendu dans une
//! fenêtre OpenGL, sous une interface de verre liquide plein écran.

mod gfx;
mod i18n;
mod keys;
mod mpv;
mod player;
mod ui;

use std::ffi::{c_char, c_void, CStr};
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui_glow::glow;
use egui_winit::winit;
use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, GlProfile, NotCurrentGlContext, PossiblyCurrentContext, Version};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SwapInterval, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Fullscreen, Window, WindowId};

use i18n::tr;
use player::{Panel, Player};

const SUBTITLE_EXT: &[&str] = &["srt", "ass", "ssa", "vtt", "sub", "sup"];
const VIDEO_EXT: &[&str] = &["mkv", "mp4", "m4v", "avi", "webm", "mov", "ts", "m2ts", "wmv", "flv", "mpg", "mpeg", "ogv", "3gp"];

enum UserEvent {
    MpvEvents,
    MpvFrame,
    Indexed { path: String, external: Option<String>, tracks: Vec<bioplay_core::Track> },
    Picked(String),
}

extern "C" fn on_wakeup(d: *mut c_void) {
    let proxy = unsafe { &*(d as *const EventLoopProxy<UserEvent>) };
    let _ = proxy.send_event(UserEvent::MpvEvents);
}

extern "C" fn on_frame(d: *mut c_void) {
    let proxy = unsafe { &*(d as *const EventLoopProxy<UserEvent>) };
    let _ = proxy.send_event(UserEvent::MpvFrame);
}

unsafe extern "C" fn get_proc(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    let display = &*(ctx as *const glutin::display::Display);
    display.get_proc_address(CStr::from_ptr(name)) as *mut c_void
}

struct Gl {
    window: Arc<Window>,
    surface: Surface<WindowSurface>,
    context: PossiblyCurrentContext,
    display: Box<glutin::display::Display>,
    gl: Arc<glow::Context>,
    painter: egui_glow::Painter,
    egui: egui_winit::State,
    gfx: gfx::Gfx,
}

struct App {
    proxy: EventLoopProxy<UserEvent>,
    proxy_ptr: *mut c_void,
    player: Option<Player>,
    gl: Option<Gl>,
    ctx: egui::Context,
    files: Vec<String>,
    mods: ModifiersState,
    last_save: Instant,
    resized: bool,
    frame_pending: bool,
    /// La texture vidéo n'a plus la dernière image (rendu direct entre-temps).
    texture_stale: bool,
}

impl App {
    fn player(&mut self) -> &mut Player {
        self.player.as_mut().unwrap()
    }

    fn open(&mut self, path: &str) {
        let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if SUBTITLE_EXT.contains(&ext.as_str()) {
            let p = self.player();
            let title = Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            p.mpv.command(&["sub-add", path, "select", &title]);
            p.say(format!("{} : {title}", tr("Sous-titres", "Subtitles")));
            self.index(path.to_owned(), Some(path.to_owned()));
        } else {
            self.player().save();
            let p = self.player();
            p.embedded.clear();
            p.external.clear();
            p.panel = None;
            p.mpv.command(&["loadfile", path, "replace"]);
        }
    }

    /// Lit l'index des sous-titres dans un fil à part : la lecture démarre sans l'attendre.
    fn index(&mut self, path: String, external: Option<String>) {
        if external.is_none() {
            self.player().indexing = true;
        }
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let tracks = std::fs::File::open(&path).ok().and_then(|f| bioplay_core::index(f).ok()).unwrap_or_default();
            let _ = proxy.send_event(UserEvent::Indexed { path, external, tracks });
        });
    }

    fn pick(&self) {
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let f = rfd::FileDialog::new()
                .set_title(tr("Ouvrir une vidéo", "Open a video"))
                .add_filter(tr("Vidéos", "Videos"), VIDEO_EXT)
                .add_filter(tr("Tous les fichiers", "All files"), &["*"])
                .pick_file();
            if let Some(f) = f {
                let _ = proxy.send_event(UserEvent::Picked(f.to_string_lossy().into_owned()));
            }
        });
    }

    fn redraw(&self) {
        if let Some(g) = &self.gl {
            g.window.request_redraw();
        }
    }

    fn poke(&mut self) {
        if let Some(p) = self.player.as_mut() {
            p.activity = Instant::now();
        }
        self.redraw();
    }

    fn sync_window(&mut self) {
        let Some(g) = &self.gl else { return };
        let Some(p) = &self.player else { return };
        let want = p.mpv.state.fullscreen;
        if want != g.window.fullscreen().is_some() {
            g.window.set_fullscreen(want.then_some(Fullscreen::Borderless(None)));
        }
        let title = if p.has_file() { format!("{} — bioplay", p.mpv.state.title) } else { "bioplay".into() };
        g.window.set_title(&title);
    }

    /// Fréquence réelle de l'écran : avec le rendu intégré, mpv ne la connaît pas, et il en a
    /// besoin pour caler les images (statistiques de synchro, `video-sync=display-*`).
    fn tell_refresh_rate(&mut self) {
        let Some(g) = &self.gl else { return };
        let hz = g.window.current_monitor().and_then(|m| m.refresh_rate_millihertz()).map(|m| m as f64 / 1000.0);
        if let (Some(hz), Some(p)) = (hz, self.player.as_ref()) {
            p.mpv.set_f64("override-display-fps", hz);
        }
    }

    fn render(&mut self) {
        let (Some(g), Some(p)) = (self.gl.as_mut(), self.player.as_mut()) else { return };
        let size = g.window.inner_size();
        let (w, h) = (size.width.max(1) as i32, size.height.max(1) as i32);
        let has_video = p.has_file();
        let new_frame = has_video && p.mpv.frame_ready();

        // L'interface d'abord : elle dit s'il y a du verre à l'écran.
        let raw = g.egui.take_egui_input(&g.window);
        let ppp = g.window.scale_factor() as f32;
        let mut glass = Vec::new();
        let mut actions = Vec::new();
        let mut out = self.ctx.run_ui(raw, |ui| {
            let (gl_list, a) = ui::draw(ui.ctx(), p, ppp);
            glass = gl_list;
            actions = a;
        });
        g.egui.handle_platform_output(&g.window, out.platform_output);
        let prims = self.ctx.tessellate(out.shapes, out.pixels_per_point);

        if has_video && glass.is_empty() {
            // Interface masquée : mpv dessine directement à l'écran, comme mpv seul. Aucune
            // copie ni flou : en 4K, le coût est exactement celui de mpv.
            p.mpv.render(0, w, h, true);
            self.texture_stale = true;
        } else {
            if has_video {
                // Verre visible : mpv dessine dans une texture que le verre floute et réfracte.
                let fbo = g.gfx.video_fbo(&g.gl, w, h);
                if new_frame || self.resized || self.frame_pending || self.texture_stale {
                    p.mpv.render(fbo, w, h, true);
                    g.gfx.mark_frame();
                    self.texture_stale = false;
                }
            }
            g.gfx.compose(&g.gl, w, h, has_video, &glass);
        }
        self.resized = false;
        self.frame_pending = false;
        g.painter.paint_and_update_textures([w as u32, h as u32], out.pixels_per_point, &prims, &mut out.textures_delta);
        g.window.pre_present_notify();
        let _ = g.surface.swap_buffers(&g.context);
        p.mpv.report_swap();

        let idle = !p.mpv.state.paused && p.panel.is_none() && !p.over_ui && p.activity.elapsed() > Duration::from_millis(2200);
        g.window.set_cursor_visible(!(has_video && idle));

        if let Some(vo) = out.viewport_output.get(&egui::ViewportId::ROOT) {
            if vo.repaint_delay.is_zero() {
                g.window.request_redraw();
            }
        }
        for a in actions {
            match a {
                ui::Action::Open => self.pick(),
                ui::Action::Play(path) => self.open(&path),
                ui::Action::Forget(path) => self.player().forget(&path),
            }
        }
    }

    /// Touches gérées ici avant mpv ; tout le reste part à mpv via `keypress`.
    fn key(&mut self, key: &Key, location: winit::keyboard::KeyLocation) {
        let p = self.player.as_mut().unwrap();
        let ctrl = self.mods.control_key();
        match key {
            Key::Named(NamedKey::Escape) if p.panel.is_some() => {
                p.panel = None;
                return;
            }
            Key::Named(NamedKey::Tab) if !ctrl => {
                p.panel = if p.panel == Some(Panel::List) { None } else { Some(Panel::List) };
                return;
            }
            // Flèches seules : sous-titre précédent / suivant, ou 10 s s'il est plus loin.
            // Ctrl + flèche : 10 s, sans tenir compte des sous-titres.
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowRight) if !self.mods.shift_key() && !self.mods.alt_key() => {
                let dir = if matches!(key, Key::Named(NamedKey::ArrowRight)) { 1 } else { -1 };
                if ctrl {
                    let t = p.mpv.state.position + 10.0 * dir as f64;
                    p.mpv.seek(t, true);
                    p.say(if dir > 0 { "+10 s" } else { "−10 s" });
                } else {
                    p.step_line_or_ten(dir);
                }
                return;
            }
            _ => {}
        }
        if let Some(name) = keys::name(key, location, self.mods) {
            p.key_seek = Some((Instant::now(), p.mpv.state.position));
            p.mpv.command(&["keypress", &name]);
        }
    }

    fn on_change(&mut self, change: mpv::Change) {
        match change {
            mpv::Change::Shutdown => {}
            mpv::Change::Loaded => {
                self.player().loaded_at = Some(Instant::now());
                let path = self.player().mpv.state.path.clone();
                if Path::new(&path).is_file() {
                    self.index(path, None);
                }
                self.player().save();
            }
            mpv::Change::Ended => {}
            // Saut provoqué par un raccourci mpv (flèches haut/bas…) : annoncé dans une bulle.
            mpv::Change::Restarted => {
                let p = self.player();
                if let Some((at, before)) = p.key_seek.take() {
                    let d = p.mpv.state.position - before;
                    if at.elapsed() < Duration::from_millis(1500) && d.abs() >= 0.9 {
                        p.say(player::delta(d));
                    }
                }
            }
            mpv::Change::Property(name) => {
                let p = self.player.as_mut().unwrap();
                let s = &p.mpv.state;
                let value = match name {
                    "aid" => s.aid.map(|v| v as f64).unwrap_or(-1.0),
                    "sid" => s.sid.map(|v| v as f64).unwrap_or(-1.0),
                    "speed" => s.speed,
                    "volume" => s.volume,
                    "sub-delay" => s.sub_delay,
                    "audio-delay" => s.audio_delay,
                    _ => f64::NAN,
                };
                let settled = p.loaded_at.is_some_and(|t| t.elapsed() > Duration::from_millis(800));
                let previous = if value.is_nan() { None } else { p.shown.insert(name, value) };
                // Décalage des sous-titres : passer du négatif au positif (ou l'inverse) s'arrête
                // d'abord sur 0, sans décalage.
                if name == "sub-delay" && settled {
                    if let Some(old) = previous {
                        if (old < -1e-6 && value > 1e-6) || (old > 1e-6 && value < -1e-6) {
                            p.mpv.set_f64("sub-delay", 0.0);
                            return;
                        }
                    }
                }
                let changed = previous.is_some_and(|old| (old - value).abs() > 1e-9);
                let msg = if !(settled && changed) { None } else { match name {
                    "speed" if p.has_file() => Some(format!("{} {}", tr("Vitesse", "Speed"), player::speed_label(s.speed))),
                    "volume" if p.has_file() => Some(format!("Volume {:.0} %", s.volume)),
                    "sub-delay" if p.has_file() => Some(if s.sub_delay.abs() < 1e-6 { tr("Sous-titres : aucun décalage", "Subtitles: no delay").into() } else { format!("{} {:+.0} ms", tr("Sous-titres", "Subtitles"), s.sub_delay * 1000.0) }),
                    "audio-delay" if p.has_file() => Some(format!("Audio {:+.0} ms", s.audio_delay * 1000.0)),
                    "aid" => s.tracks.iter().find(|t| t.kind == "audio" && Some(t.id) == s.aid)
                        .map(|t| format!("{} : {}", tr("Audio", "Audio"), mpv::track_label(t))),
                    "sid" => Some(match p.sub_track() {
                        Some(t) => format!("{} : {}", tr("Sous-titres", "Subtitles"), mpv::track_label(t)),
                        None => tr("Sous-titres désactivés", "Subtitles off").into(),
                    }),
                    _ => None,
                } };
                if let Some(m) = msg {
                    p.say(m);
                }
                if matches!(name, "fullscreen" | "media-title") {
                    self.sync_window();
                }
            }
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gl.is_some() {
            return;
        }
        // app_id « bioplay » : icône et entrée du lanceur reconnues par le compositeur.
        let attrs = winit::platform::wayland::WindowAttributesExtWayland::with_name(
            Window::default_attributes().with_title("bioplay").with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0)),
            "bioplay",
            "",
        );
        let template = ConfigTemplateBuilder::new().with_alpha_size(8).prefer_hardware_accelerated(Some(true));
        let (window, config) = DisplayBuilder::new()
            .with_window_attributes(Some(attrs))
            .build(el, template, |configs| configs.min_by_key(|c| c.num_samples()).unwrap())
            .expect("fenêtre OpenGL");
        let window = Arc::new(window.unwrap());
        let raw = window.window_handle().ok().map(|h| h.as_raw());
        let display = config.display();
        let core = ContextAttributesBuilder::new()
            .with_profile(GlProfile::Core)
            .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
            .build(raw);
        let context = unsafe { display.create_context(&config, &core) }.expect("contexte OpenGL 3.3");
        let sattrs = window.build_surface_attributes(Default::default()).unwrap();
        let surface = unsafe { display.create_window_surface(&config, &sattrs) }.unwrap();
        let context = context.make_current(&surface).unwrap();
        // Pas d'attente de la synchro dans swap_buffers : sur un bureau virtuel caché, le compositeur ne
        // demande plus d'images et l'appel bloquerait l'app (Hyprland la croirait plantée). Le rythme
        // vient de winit (pre_present_notify + rappels d'image du compositeur).
        let _ = surface.set_swap_interval(&context, SwapInterval::DontWait);
        let gl = Arc::new(unsafe { glow::Context::from_loader_function_cstr(|s| display.get_proc_address(s)) });
        let painter = egui_glow::Painter::new(gl.clone(), "", None, true).expect("egui_glow");
        let egui = egui_winit::State::new(self.ctx.clone(), egui::ViewportId::ROOT, &window, Some(window.scale_factor() as f32), None, Some(painter.max_texture_side()));
        let display = Box::new(display);

        // mpv : configuration de l'utilisateur, rendu dans notre contexte OpenGL.
        let mut m = mpv::Mpv::new(on_wakeup, self.proxy_ptr).expect("libmpv");
        // Options mpv en ligne de commande, comme mpv : `bioplay --mute=yes --sub-delay=0.5 film.mkv`.
        for a in std::env::args().skip(1) {
            if let Some((k, v)) = a.strip_prefix("--").and_then(|o| o.split_once('=')) {
                if k != "panel" {
                    m.set(k, v);
                }
            }
        }
        let wl = match window.display_handle().map(|h| h.as_raw()) {
            Ok(RawDisplayHandle::Wayland(h)) => h.display.as_ptr(),
            _ => std::ptr::null_mut(),
        };
        m.init_render(get_proc, &*display as *const _ as *mut c_void, wl, on_frame, self.proxy_ptr).expect("rendu mpv");
        let gfx = gfx::Gfx::new(&gl);
        self.gl = Some(Gl { window, surface, context, display, gl, painter, egui, gfx });
        self.player = Some(Player::new(m));
        let files = std::mem::take(&mut self.files);
        for (i, f) in files.iter().enumerate() {
            if i == 0 {
                self.open(f);
            } else {
                self.player().mpv.command(&["loadfile", f, "append"]);
            }
        }
        self.sync_window();
        self.tell_refresh_rate();
        // Pour les captures de développement : `--panel=list|tracks|speed|zoom`.
        if let Some(a) = std::env::args().find_map(|a| a.strip_prefix("--panel=").map(str::to_owned)) {
            self.player().panel = match a.as_str() {
                "list" => Some(Panel::List),
                "tracks" => Some(Panel::Tracks),
                "speed" => Some(Panel::Speed),
                "zoom" => Some(Panel::Zoom),
                _ => None,
            };
        }
    }

    fn user_event(&mut self, el: &ActiveEventLoop, e: UserEvent) {
        match e {
            UserEvent::MpvEvents => {
                let changes = self.player.as_mut().map(|p| p.mpv.poll()).unwrap_or_default();
                for c in changes {
                    if matches!(c, mpv::Change::Shutdown) {
                        el.exit();
                        return;
                    }
                    self.on_change(c);
                }
                // L'interface n'a besoin d'être redessinée que si elle est visible.
                let p = self.player.as_ref().unwrap();
                if p.mpv.state.paused || p.activity.elapsed() < Duration::from_secs(3) || !p.has_file() || p.message.is_some() {
                    self.redraw();
                }
            }
            UserEvent::MpvFrame => {
                self.frame_pending = true;
                self.redraw();
            }
            UserEvent::Indexed { path, external, tracks } => {
                let p = self.player();
                match external {
                    Some(f) => {
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
                self.redraw();
            }
            UserEvent::Picked(path) => self.open(&path),
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(g) = self.gl.as_mut() else { return };
        // Le clavier va à mpv (raccourcis de l'utilisateur), pas à egui.
        if !matches!(event, WindowEvent::KeyboardInput { .. }) {
            let r = g.egui.on_window_event(&g.window, &event);
            if r.repaint {
                g.window.request_redraw();
            }
        }
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.tell_refresh_rate();
            }
            WindowEvent::Resized(s) => {
                g.surface.resize(&g.context, NonZeroU32::new(s.width.max(1)).unwrap(), NonZeroU32::new(s.height.max(1)).unwrap());
                self.resized = true;
                self.redraw();
            }
            WindowEvent::ModifiersChanged(m) => self.mods = m.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                // Le clavier n'affiche pas le contrôleur (il masquerait les sous-titres) :
                // seule la souris le fait apparaître.
                self.key(&event.logical_key, event.location);
                self.redraw();
            }
            WindowEvent::CursorMoved { .. } => self.poke(),
            WindowEvent::MouseWheel { delta, .. } => {
                // Panneau ouvert : la molette fait défiler ses listes ; sinon elle va à mpv.
                if self.player().panel.is_none() {
                    let dy = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(p) => p.y as f32,
                    };
                    if dy != 0.0 && !self.ctx.egui_is_using_pointer() {
                        let mut name = String::new();
                        if self.mods.control_key() {
                            name += "Ctrl+";
                        }
                        if self.mods.shift_key() {
                            name += "Shift+";
                        }
                        name += if dy > 0.0 { "WHEEL_UP" } else { "WHEEL_DOWN" };
                        let p = self.player();
                        if p.panel.is_none() {
                            p.mpv.command(&["keypress", &name]);
                        }
                    }
                }
                self.poke();
            }
            WindowEvent::DroppedFile(path) => {
                let s = path.to_string_lossy().into_owned();
                self.open(&s);
            }
            WindowEvent::RedrawRequested => self.render(),
            _ => {}
        }
    }

    /// Tout est libéré tant que la connexion Wayland existe encore : la libérer après faisait
    /// planter l'app à chaque fermeture.
    fn exiting(&mut self, _: &ActiveEventLoop) {
        if let Some(p) = self.player.as_mut() {
            p.save();
        }
        self.player = None;
        self.gl = None;
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        // Position notée toutes les 15 s pendant la lecture : reprise même après un arrêt brutal.
        if let Some(p) = self.player.as_mut() {
            if p.has_file() && !p.mpv.state.paused && self.last_save.elapsed() > Duration::from_secs(15) {
                p.save();
                self.last_save = Instant::now();
            }
            let wake = if p.has_file() && !p.mpv.state.paused { Duration::from_secs(1) } else { Duration::from_secs(5) };
            // Réveil pour masquer l'interface et le curseur après inactivité.
            let hide_at = p.activity + Duration::from_millis(2300);
            let next = if hide_at > Instant::now() { hide_at } else { Instant::now() + wake };
            el.set_control_flow(ControlFlow::WaitUntil(next));
            if hide_at <= Instant::now() && hide_at + Duration::from_millis(500) > Instant::now() {
                self.redraw();
            }
        }
    }
}

fn main() {
    let files: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("--")).collect();
    let el = EventLoop::<UserEvent>::with_user_event().build().expect("boucle d'événements");
    let proxy = el.create_proxy();
    let proxy_ptr = Box::into_raw(Box::new(proxy.clone())) as *mut c_void;
    let mut app = App {
        proxy,
        proxy_ptr,
        player: None,
        gl: None,
        ctx: egui::Context::default(),
        files,
        mods: ModifiersState::empty(),
        last_save: Instant::now(),
        resized: true,
        frame_pending: false,
        texture_stale: true,
    };
    i18n::init();
    ui::fonts(&app.ctx);
    el.run_app(&mut app).expect("exécution");
    // mpv d'abord (son rendu utilise le contexte GL), puis le reste.
    app.player = None;
    app.gl = None;
}
