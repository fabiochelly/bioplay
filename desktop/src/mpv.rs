//! Liaison minimale avec libmpv (client + rendu OpenGL), sans crate intermédiaire.

use std::collections::HashMap;
use std::ffi::{c_char, c_double, c_int, c_void, CStr, CString};
use std::ptr;

#[allow(non_camel_case_types)]
type mpv_handle = c_void;
#[allow(non_camel_case_types)]
type mpv_render_context = c_void;

#[repr(C)]
struct Event {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct EventProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct RenderParam {
    kind: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct GlInitParams {
    get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    ctx: *mut c_void,
}

#[repr(C)]
struct GlFbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}

#[link(name = "mpv")]
extern "C" {
    fn mpv_create() -> *mut mpv_handle;
    fn mpv_initialize(h: *mut mpv_handle) -> c_int;
    fn mpv_terminate_destroy(h: *mut mpv_handle);
    fn mpv_set_option_string(h: *mut mpv_handle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_set_property_string(h: *mut mpv_handle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_set_property(h: *mut mpv_handle, name: *const c_char, format: c_int, data: *mut c_void) -> c_int;
    fn mpv_get_property_string(h: *mut mpv_handle, name: *const c_char) -> *mut c_char;
    fn mpv_command_async(h: *mut mpv_handle, reply: u64, args: *mut *const c_char) -> c_int;
    fn mpv_command(h: *mut mpv_handle, args: *mut *const c_char) -> c_int;
    fn mpv_observe_property(h: *mut mpv_handle, reply: u64, name: *const c_char, format: c_int) -> c_int;
    fn mpv_wait_event(h: *mut mpv_handle, timeout: c_double) -> *mut Event;
    fn mpv_set_wakeup_callback(h: *mut mpv_handle, cb: extern "C" fn(*mut c_void), d: *mut c_void);
    fn mpv_free(data: *mut c_void);

    fn mpv_render_context_create(res: *mut *mut mpv_render_context, h: *mut mpv_handle, params: *mut RenderParam) -> c_int;
    fn mpv_render_context_set_update_callback(ctx: *mut mpv_render_context, cb: extern "C" fn(*mut c_void), d: *mut c_void);
    fn mpv_render_context_update(ctx: *mut mpv_render_context) -> u64;
    fn mpv_render_context_render(ctx: *mut mpv_render_context, params: *mut RenderParam) -> c_int;
    fn mpv_render_context_report_swap(ctx: *mut mpv_render_context);
    fn mpv_render_context_free(ctx: *mut mpv_render_context);
}

const FORMAT_STRING: c_int = 1;
const FORMAT_FLAG: c_int = 3;
const FORMAT_INT64: c_int = 4;
const FORMAT_DOUBLE: c_int = 5;

const EVENT_SHUTDOWN: c_int = 1;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const EVENT_PROPERTY_CHANGE: c_int = 22;

#[derive(Clone, Debug, Default)]
pub struct Track {
    pub id: i64,
    pub kind: String,
    pub src_id: i64,
    pub title: String,
    pub lang: String,
    pub codec: String,
    pub channels: i64,
    pub external: Option<String>,
    pub forced: bool,
}

/// L'état de lecture tel que mpv le publie, tenu à jour par [`Mpv::poll`].
#[derive(Default)]
pub struct State {
    pub position: f64,
    pub duration: f64,
    pub cached: f64,
    pub paused: bool,
    pub speed: f64,
    pub volume: f64,
    pub sub_delay: f64,
    pub audio_delay: f64,
    pub tracks: Vec<Track>,
    pub aid: Option<i64>,
    pub sid: Option<i64>,
    pub fullscreen: bool,
    pub title: String,
    pub path: String,
    pub hwdec: String,
    pub idle: bool,
    pub zoom: f64,
}

pub enum Change {
    Loaded,
    Ended,
    Shutdown,
    /// Propriété modifiée : nom, pour les messages à l'écran.
    Property(&'static str),
}

const OBSERVED: &[(&str, c_int)] = &[
    ("time-pos", FORMAT_DOUBLE),
    ("duration", FORMAT_DOUBLE),
    ("demuxer-cache-time", FORMAT_DOUBLE),
    ("pause", FORMAT_FLAG),
    ("speed", FORMAT_DOUBLE),
    ("volume", FORMAT_DOUBLE),
    ("sub-delay", FORMAT_DOUBLE),
    ("audio-delay", FORMAT_DOUBLE),
    ("track-list", FORMAT_STRING),
    ("aid", FORMAT_STRING),
    ("sid", FORMAT_STRING),
    ("fullscreen", FORMAT_FLAG),
    ("media-title", FORMAT_STRING),
    ("path", FORMAT_STRING),
    ("hwdec-current", FORMAT_STRING),
    ("idle-active", FORMAT_FLAG),
    ("video-zoom", FORMAT_DOUBLE),
];

pub struct Mpv {
    h: *mut mpv_handle,
    render: *mut mpv_render_context,
    pub state: State,
}

fn cs(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap()
}

impl Mpv {
    /// Crée mpv avec la configuration de l'utilisateur (`~/.config/mpv/mpv.conf` et
    /// `input.conf`) : mêmes réglages, mêmes raccourcis que son mpv habituel.
    pub fn new(wakeup: extern "C" fn(*mut c_void), data: *mut c_void) -> Result<Self, String> {
        let h = unsafe { mpv_create() };
        if h.is_null() {
            return Err("mpv_create a échoué".into());
        }
        let mut m = Self { h, render: ptr::null_mut(), state: State { speed: 1.0, ..Default::default() } };
        for (k, v) in [
            ("config", "yes"),
            ("vo", "libmpv"),
            ("input-default-bindings", "yes"),
            ("idle", "yes"),
            ("keep-open", "yes"),
            ("terminal", "no"),
            ("msg-level", "all=warn"),
        ] {
            m.option(k, v);
        }
        if unsafe { mpv_initialize(h) } < 0 {
            return Err("mpv_initialize a échoué".into());
        }
        // Après la configuration : nos choix qui priment sur mpv.conf.
        m.set("vo", "libmpv"); // rendu dans notre fenêtre, sous l'interface
        m.set("osd-level", "0"); // les messages sont les nôtres, en verre
        m.set("save-position-on-quit", "yes"); // reprise toujours, quoi qu'il arrive
        m.set("resume-playback", "yes");
        m.set("input-cursor", "no");
        // Sous-titres sobres : taille raisonnable, liseré noir fin, aucune ombre, y compris pour
        // les ASS dont le style imposait des tailles démesurées. L'épaisseur du liseré reste celle
        // de mpv.conf.
        for (k, v) in [
            ("sub-font-size", "34"),
            ("sub-color", "#FFFFFF"),
            ("sub-outline-color", "#000000"),
            ("sub-shadow-offset", "0"),
            ("sub-blur", "0"),
            ("sub-back-color", "#00000000"),
            ("sub-ass-override", "force"),
        ] {
            m.set(k, v);
        }
        for (i, (name, f)) in OBSERVED.iter().enumerate() {
            unsafe { mpv_observe_property(h, i as u64, cs(name).as_ptr(), *f) };
        }
        unsafe { mpv_set_wakeup_callback(h, wakeup, data) };
        Ok(m)
    }

    fn option(&self, k: &str, v: &str) {
        unsafe { mpv_set_option_string(self.h, cs(k).as_ptr(), cs(v).as_ptr()) };
    }

    pub fn set(&self, k: &str, v: &str) {
        unsafe { mpv_set_property_string(self.h, cs(k).as_ptr(), cs(v).as_ptr()) };
    }

    pub fn set_f64(&self, k: &str, mut v: f64) {
        unsafe { mpv_set_property(self.h, cs(k).as_ptr(), FORMAT_DOUBLE, &mut v as *mut f64 as *mut c_void) };
    }

    pub fn set_flag(&self, k: &str, v: bool) {
        let mut i: c_int = v as c_int;
        unsafe { mpv_set_property(self.h, cs(k).as_ptr(), FORMAT_FLAG, &mut i as *mut c_int as *mut c_void) };
    }

    pub fn get(&self, k: &str) -> Option<String> {
        unsafe {
            let p = mpv_get_property_string(self.h, cs(k).as_ptr());
            if p.is_null() {
                return None;
            }
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            mpv_free(p as *mut c_void);
            Some(s)
        }
    }

    /// Commande asynchrone : ne bloque jamais l'interface.
    pub fn command(&self, args: &[&str]) {
        let owned: Vec<CString> = args.iter().map(|a| cs(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(ptr::null());
        unsafe { mpv_command_async(self.h, 0, ptrs.as_mut_ptr()) };
    }

    /// Commande synchrone (pour `quit` et l'écriture de la position).
    pub fn command_sync(&self, args: &[&str]) {
        let owned: Vec<CString> = args.iter().map(|a| cs(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(ptr::null());
        unsafe { mpv_command(self.h, ptrs.as_mut_ptr()) };
    }

    pub fn seek(&self, t: f64, exact: bool) {
        let max = if self.state.duration > 0.0 { self.state.duration } else { f64::MAX };
        let t = t.clamp(0.0, max);
        self.command(&["seek", &format!("{t:.3}"), if exact { "absolute+exact" } else { "absolute+keyframes" }]);
    }

    // --- Rendu -----------------------------------------------------------------------------

    pub fn init_render(
        &mut self,
        get_proc: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
        ctx: *mut c_void,
        wl_display: *mut c_void,
        update: extern "C" fn(*mut c_void),
        data: *mut c_void,
    ) -> Result<(), String> {
        let api = cs("opengl");
        let mut init = GlInitParams { get_proc_address: get_proc, ctx };
        let mut advanced: c_int = 1;
        let mut params = vec![
            RenderParam { kind: 1, data: api.as_ptr() as *mut c_void },
            RenderParam { kind: 2, data: &mut init as *mut _ as *mut c_void },
            RenderParam { kind: 10, data: &mut advanced as *mut _ as *mut c_void },
        ];
        // Wayland : nécessaire au décodage matériel VA-API sans copie.
        if !wl_display.is_null() {
            params.push(RenderParam { kind: 9, data: wl_display });
        }
        params.push(RenderParam { kind: 0, data: ptr::null_mut() });
        let r = unsafe { mpv_render_context_create(&mut self.render, self.h, params.as_mut_ptr()) };
        if r < 0 {
            return Err(format!("mpv_render_context_create : {r}"));
        }
        unsafe { mpv_render_context_set_update_callback(self.render, update, data) };
        Ok(())
    }

    /// Vrai si une nouvelle image vidéo est à dessiner.
    pub fn frame_ready(&self) -> bool {
        !self.render.is_null() && unsafe { mpv_render_context_update(self.render) } & 1 != 0
    }

    pub fn render(&self, fbo: u32, w: i32, h: i32, flip: bool) {
        if self.render.is_null() {
            return;
        }
        let mut target = GlFbo { fbo: fbo as c_int, w, h, internal_format: 0 };
        let mut flip_y: c_int = flip as c_int;
        let mut params = [
            RenderParam { kind: 3, data: &mut target as *mut _ as *mut c_void },
            RenderParam { kind: 4, data: &mut flip_y as *mut _ as *mut c_void },
            RenderParam { kind: 0, data: ptr::null_mut() },
        ];
        unsafe { mpv_render_context_render(self.render, params.as_mut_ptr()) };
    }

    pub fn report_swap(&self) {
        if !self.render.is_null() {
            unsafe { mpv_render_context_report_swap(self.render) };
        }
    }

    // --- Événements ------------------------------------------------------------------------

    /// Vide la file d'événements de mpv et met l'état à jour.
    pub fn poll(&mut self) -> Vec<Change> {
        let mut out = Vec::new();
        loop {
            let e = unsafe { &*mpv_wait_event(self.h, 0.0) };
            match e.event_id {
                0 => break,
                EVENT_SHUTDOWN => {
                    out.push(Change::Shutdown);
                    break;
                }
                EVENT_FILE_LOADED => out.push(Change::Loaded),
                EVENT_END_FILE => out.push(Change::Ended),
                EVENT_PROPERTY_CHANGE => {
                    let p = unsafe { &*(e.data as *const EventProperty) };
                    let Some(&(name, _)) = OBSERVED.get(e.reply_userdata as usize) else { continue };
                    self.apply(name, p);
                    out.push(Change::Property(name));
                }
                _ => {}
            }
        }
        out
    }

    fn apply(&mut self, name: &str, p: &EventProperty) {
        let s = &mut self.state;
        let double = || (p.format == FORMAT_DOUBLE && !p.data.is_null()).then(|| unsafe { *(p.data as *const f64) });
        let flag = || (p.format == FORMAT_FLAG && !p.data.is_null()).then(|| unsafe { *(p.data as *const c_int) } != 0);
        let string = || {
            (p.format == FORMAT_STRING && !p.data.is_null())
                .then(|| unsafe { CStr::from_ptr(*(p.data as *const *const c_char)).to_string_lossy().into_owned() })
        };
        match name {
            "time-pos" => s.position = double().unwrap_or(0.0),
            "duration" => s.duration = double().unwrap_or(0.0),
            "demuxer-cache-time" => s.cached = double().unwrap_or(0.0),
            "speed" => s.speed = double().unwrap_or(1.0),
            "volume" => s.volume = double().unwrap_or(100.0),
            "sub-delay" => s.sub_delay = double().unwrap_or(0.0),
            "audio-delay" => s.audio_delay = double().unwrap_or(0.0),
            "video-zoom" => s.zoom = double().unwrap_or(0.0),
            "pause" => s.paused = flag().unwrap_or(false),
            "fullscreen" => s.fullscreen = flag().unwrap_or(false),
            "idle-active" => s.idle = flag().unwrap_or(false),
            "track-list" => s.tracks = string().map(|j| parse_tracks(&j)).unwrap_or_default(),
            "aid" => s.aid = string().and_then(|v| v.parse().ok()),
            "sid" => s.sid = string().and_then(|v| v.parse().ok()),
            "media-title" => s.title = string().unwrap_or_default(),
            "path" => s.path = string().unwrap_or_default(),
            "hwdec-current" => s.hwdec = string().unwrap_or_default(),
            _ => {}
        }
        let _ = FORMAT_INT64;
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        unsafe {
            if !self.render.is_null() {
                mpv_render_context_free(self.render);
            }
            mpv_terminate_destroy(self.h);
        }
    }
}

/// `track-list` en JSON (format chaîne de mpv) : lecture des seuls champs utiles.
fn parse_tracks(json: &str) -> Vec<Track> {
    let Ok(serde_json::Value::Array(list)) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    list.iter()
        .map(|o| {
            let str_of = |k: &str| o.get(k).and_then(|v| v.as_str()).unwrap_or("").to_owned();
            Track {
                id: o.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
                kind: str_of("type"),
                src_id: o.get("src-id").and_then(|v| v.as_i64()).unwrap_or(-1),
                title: str_of("title"),
                lang: str_of("lang"),
                codec: str_of("codec"),
                channels: o.get("demux-channel-count").and_then(|v| v.as_i64()).unwrap_or(0),
                external: o.get("external").and_then(|v| v.as_bool()).filter(|&e| e).map(|_| str_of("external-filename")),
                forced: o.get("forced").and_then(|v| v.as_bool()).unwrap_or(false),
            }
        })
        .collect()
}

/// Libellé lisible d'une piste : son titre, sinon la langue, sinon « Piste n ».
pub fn track_label(t: &Track) -> String {
    if !t.title.is_empty() {
        return t.title.clone();
    }
    language(&t.lang).unwrap_or_else(|| format!("Piste {}", t.id))
}

pub fn track_detail(t: &Track) -> String {
    let mut parts = Vec::new();
    if !t.title.is_empty() {
        if let Some(l) = language(&t.lang) {
            parts.push(l);
        }
    }
    parts.push(match t.codec.as_str() {
        "subrip" => "SRT".into(),
        "ass" | "ssa" => "ASS".into(),
        "hdmv_pgs_subtitle" => "PGS".into(),
        "dvd_subtitle" => "VobSub".into(),
        "truehd" => "TrueHD".into(),
        "eac3" => "E-AC3".into(),
        c => c.to_uppercase(),
    });
    match t.channels {
        0 => {}
        1 => parts.push("Mono".into()),
        2 => parts.push("Stéréo".into()),
        6 => parts.push("5.1".into()),
        8 => parts.push("7.1".into()),
        n => parts.push(format!("{n} canaux")),
    }
    if t.forced {
        parts.push("Forcés".into());
    }
    if t.external.is_some() {
        parts.push("Externe".into());
    }
    parts.join(" · ")
}

pub fn language(code: &str) -> Option<String> {
    let names: HashMap<&str, &str> = [
        ("fr", "Français"), ("fre", "Français"), ("fra", "Français"),
        ("en", "Anglais"), ("eng", "Anglais"),
        ("de", "Allemand"), ("ger", "Allemand"), ("deu", "Allemand"),
        ("es", "Espagnol"), ("spa", "Espagnol"),
        ("it", "Italien"), ("ita", "Italien"),
        ("pt", "Portugais"), ("por", "Portugais"),
        ("ja", "Japonais"), ("jpn", "Japonais"),
        ("ko", "Coréen"), ("kor", "Coréen"),
        ("zh", "Chinois"), ("chi", "Chinois"), ("zho", "Chinois"),
        ("ru", "Russe"), ("rus", "Russe"),
        ("nl", "Néerlandais"), ("dut", "Néerlandais"), ("nld", "Néerlandais"),
        ("ar", "Arabe"), ("ara", "Arabe"),
        ("pl", "Polonais"), ("pol", "Polonais"),
        ("sv", "Suédois"), ("swe", "Suédois"),
        ("tr", "Turc"), ("tur", "Turc"),
        ("he", "Hébreu"), ("heb", "Hébreu"),
    ]
    .into_iter()
    .collect();
    if code.is_empty() || code == "und" {
        return None;
    }
    Some(names.get(code.to_ascii_lowercase().as_str()).map(|s| s.to_string()).unwrap_or_else(|| code.to_uppercase()))
}
