//! Liaison minimale avec libmpv (client + rendu OpenGL), sans crate intermédiaire.

use std::collections::HashMap;
use std::ffi::{c_char, c_double, c_int, c_void, CStr, CString};
use std::ptr;

#[allow(non_camel_case_types)]
type mpv_handle = c_void;

#[repr(C)]
struct Event {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct ClientMessage {
    num_args: c_int,
    args: *const *const c_char,
}

#[repr(C)]
struct EventProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
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
    fn mpv_free(data: *mut c_void);
    fn mpv_wakeup(h: *mut mpv_handle);

}

const FORMAT_STRING: c_int = 1;
const FORMAT_FLAG: c_int = 3;
const FORMAT_INT64: c_int = 4;
const FORMAT_DOUBLE: c_int = 5;

const EVENT_SHUTDOWN: c_int = 1;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const EVENT_CLIENT_MESSAGE: c_int = 16;
const EVENT_SEEK: c_int = 20;
const EVENT_PLAYBACK_RESTART: c_int = 21;
const EVENT_PROPERTY_CHANGE: c_int = 22;

#[derive(Clone, Copy)]
pub struct Waker(usize);

impl Waker {
    pub fn wake(&self) {
        // mpv_wakeup est sûr depuis n'importe quel fil tant que le lecteur existe.
        unsafe { mpv_wakeup(self.0 as *mut mpv_handle) };
    }
}

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
    /// Fenêtre (pixels) et position de l'image vidéo dedans (marges des bandes noires).
    pub osd_w: f64,
    pub osd_h: f64,
    pub margin_left: f64,
    pub margin_top: f64,
    pub margin_bottom: f64,
    pub scale: f64,
    /// Souris dans la fenêtre (pixels), None si elle en est sortie.
    pub mouse: Option<(f64, f64)>,
}

pub enum Change {
    Loaded,
    Ended,
    Shutdown,
    /// Message d'une section de raccourcis (`script-binding main/<nom>`) : nom, état (d-, u-…).
    Binding(String, String),
    /// Début d'un saut (position encore celle d'avant).
    Seeking,
    /// Lecture reprise après un saut : position définitive.
    Restarted,
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
    ("osd-dimensions", FORMAT_STRING),
    ("mouse-pos", FORMAT_STRING),
    ("display-hidpi-scale", FORMAT_DOUBLE),
];

pub struct Mpv {
    h: *mut mpv_handle,
    pub state: State,
    /// Sauts lancés par bioplay (les autres viennent des raccourcis : annoncés dans une bulle).
    pub own_seeks: std::cell::Cell<u32>,
}

fn cs(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap()
}

impl Mpv {
    /// Crée mpv avec la configuration de l'utilisateur (`~/.config/mpv/mpv.conf` et
    /// `input.conf`) : mêmes réglages, mêmes raccourcis que son mpv habituel.
    pub fn new() -> Result<Self, String> {
        let h = unsafe { mpv_create() };
        if h.is_null() {
            return Err("mpv_create a échoué".into());
        }
        let mut m = Self { h, state: State { speed: 1.0, scale: 1.0, ..Default::default() }, own_seeks: Default::default() };
        // mpv ouvre et dessine sa propre fenêtre en gpu-next : même image, même coût que mpv
        // (Dolby Vision et HDR compris). bioplay y ajoute son interface en superposition.
        for (k, v) in [
            ("config", "yes"),
            ("input-default-bindings", "yes"),
            ("input-vo-keyboard", "yes"),
            ("force-window", "yes"),
            ("idle", "yes"),
            ("keep-open", "yes"),
            ("terminal", "no"),
            ("msg-level", "all=warn"),
            ("wayland-app-id", "bioplay"),
            ("title", "${?media-title:${media-title} — }bioplay"),
            ("osc", "no"),
        ] {
            m.option(k, v);
        }
        if unsafe { mpv_initialize(h) } < 0 {
            return Err("mpv_initialize a échoué".into());
        }
        // Après la configuration : nos choix qui priment sur mpv.conf.
        m.set("osd-level", "0"); // les messages sont les nôtres, en verre
        m.set("save-position-on-quit", "yes"); // reprise toujours, quoi qu'il arrive
        m.set("resume-playback", "yes");
        // Sous-titres dans l'image (pas dans les bandes noires), Inter SemiBold, blanc adouci, liseré
        // noir net et sans ombre ; appliqué aussi aux ASS dont le style imposait des tailles démesurées.
        for (k, v) in [
            ("sub-use-margins", "no"),
            ("sub-ass-force-margins", "no"),
            ("sub-font-size", "40"),
            ("sub-font", "Inter SemiBold"),
            ("sub-bold", "no"),
            ("sub-margin-y", "46"),
            ("sub-color", "#EBEBEB"),
            ("sub-outline-size", "1.6"),
            ("sub-outline-color", "#0A0A0A"),
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
        self.own_seeks.set(self.own_seeks.get() + 1);
        self.command(&["seek", &format!("{t:.3}"), if exact { "absolute+exact" } else { "absolute+keyframes" }]);
    }

    /// Réveille `wait` depuis un autre fil (index des sous-titres prêt, fichier choisi…).
    pub fn waker(&self) -> Waker {
        Waker(self.h as usize)
    }

    /// Attend un événement de mpv (au plus `timeout` secondes), puis vide la file.
    pub fn wait(&mut self, timeout: f64) -> Vec<Change> {
        let mut out = Vec::new();
        let first = unsafe { &*mpv_wait_event(self.h, timeout) };
        self.handle(first, &mut out);
        if !matches!(out.last(), Some(Change::Shutdown)) {
            out.extend(self.poll());
        }
        out
    }

    // --- Événements ------------------------------------------------------------------------

    /// Vide la file d'événements de mpv et met l'état à jour.
    pub fn poll(&mut self) -> Vec<Change> {
        let mut out = Vec::new();
        loop {
            let e = unsafe { &*mpv_wait_event(self.h, 0.0) };
            if e.event_id == 0 {
                break;
            }
            self.handle(e, &mut out);
            if matches!(out.last(), Some(Change::Shutdown)) {
                break;
            }
        }
        out
    }

    fn handle(&mut self, e: &Event, out: &mut Vec<Change>) {
        match e.event_id {
            EVENT_SHUTDOWN => out.push(Change::Shutdown),
            EVENT_FILE_LOADED => out.push(Change::Loaded),
            EVENT_END_FILE => out.push(Change::Ended),
            EVENT_SEEK => out.push(Change::Seeking),
            EVENT_PLAYBACK_RESTART => out.push(Change::Restarted),
            EVENT_CLIENT_MESSAGE => {
                let m = unsafe { &*(e.data as *const ClientMessage) };
                let args: Vec<String> = (0..m.num_args as usize)
                    .map(|i| unsafe { CStr::from_ptr(*m.args.add(i)).to_string_lossy().into_owned() })
                    .collect();
                // script-binding : ["key-binding", nom, état, touche, …]
                if args.len() >= 3 && args[0] == "key-binding" {
                    out.push(Change::Binding(args[1].clone(), args[2].clone()));
                }
            }
            EVENT_PROPERTY_CHANGE => {
                let p = unsafe { &*(e.data as *const EventProperty) };
                if let Some(&(name, _)) = OBSERVED.get(e.reply_userdata as usize) {
                    self.apply(name, p);
                    out.push(Change::Property(name));
                }
            }
            _ => {}
        }
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
            "pause" => {
                s.paused = flag().unwrap_or(false);
            }
            "fullscreen" => s.fullscreen = flag().unwrap_or(false),
            "idle-active" => s.idle = flag().unwrap_or(false),
            "track-list" => s.tracks = string().map(|j| parse_tracks(&j)).unwrap_or_default(),
            "aid" => s.aid = string().and_then(|v| v.parse().ok()),
            "sid" => s.sid = string().and_then(|v| v.parse().ok()),
            "media-title" => s.title = string().unwrap_or_default(),
            "path" => s.path = string().unwrap_or_default(),
            "hwdec-current" => s.hwdec = string().unwrap_or_default(),
            "display-hidpi-scale" => s.scale = double().filter(|v| *v > 0.0).unwrap_or(1.0),
            "osd-dimensions" => {
                if let Some(v) = string().and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok()) {
                    let f = |k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
                    (s.osd_w, s.osd_h, s.margin_left, s.margin_top, s.margin_bottom) = (f("w"), f("h"), f("ml"), f("mt"), f("mb"));
                }
            }
            "mouse-pos" => {
                s.mouse = string().and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok()).and_then(|v| {
                    let hover = v.get("hover").and_then(|h| h.as_bool()).unwrap_or(false);
                    let x = v.get("x").and_then(|x| x.as_f64())?;
                    let y = v.get("y").and_then(|y| y.as_f64())?;
                    hover.then_some((x, y))
                });
            }
            _ => {}
        }
        let _ = FORMAT_INT64;
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        unsafe { mpv_terminate_destroy(self.h) };
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
    language(&t.lang).unwrap_or_else(|| format!("{} {}", crate::i18n::tr("Piste", "Track"), t.id))
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
        2 => parts.push(crate::i18n::tr("Stéréo", "Stereo").into()),
        6 => parts.push("5.1".into()),
        8 => parts.push("7.1".into()),
        n => parts.push(format!("{n} {}", crate::i18n::tr("canaux", "channels"))),
    }
    if t.forced {
        parts.push(crate::i18n::tr("Forcés", "Forced").into());
    }
    if t.external.is_some() {
        parts.push(crate::i18n::tr("Externe", "External").into());
    }
    parts.join(" · ")
}

pub fn language(code: &str) -> Option<String> {
    let names: HashMap<&str, (&str, &str)> = [
        ("fr", ("Français", "French")),
        ("fre", ("Français", "French")),
        ("fra", ("Français", "French")),
        ("en", ("Anglais", "English")),
        ("eng", ("Anglais", "English")),
        ("de", ("Allemand", "German")),
        ("ger", ("Allemand", "German")),
        ("deu", ("Allemand", "German")),
        ("es", ("Espagnol", "Spanish")),
        ("spa", ("Espagnol", "Spanish")),
        ("it", ("Italien", "Italian")),
        ("ita", ("Italien", "Italian")),
        ("pt", ("Portugais", "Portuguese")),
        ("por", ("Portugais", "Portuguese")),
        ("ja", ("Japonais", "Japanese")),
        ("jpn", ("Japonais", "Japanese")),
        ("ko", ("Coréen", "Korean")),
        ("kor", ("Coréen", "Korean")),
        ("zh", ("Chinois", "Chinese")),
        ("chi", ("Chinois", "Chinese")),
        ("zho", ("Chinois", "Chinese")),
        ("ru", ("Russe", "Russian")),
        ("rus", ("Russe", "Russian")),
        ("nl", ("Néerlandais", "Dutch")),
        ("dut", ("Néerlandais", "Dutch")),
        ("nld", ("Néerlandais", "Dutch")),
        ("ar", ("Arabe", "Arabic")),
        ("ara", ("Arabe", "Arabic")),
        ("pl", ("Polonais", "Polish")),
        ("pol", ("Polonais", "Polish")),
        ("sv", ("Suédois", "Swedish")),
        ("swe", ("Suédois", "Swedish")),
        ("tr", ("Turc", "Turkish")),
        ("tur", ("Turc", "Turkish")),
        ("he", ("Hébreu", "Hebrew")),
        ("heb", ("Hébreu", "Hebrew")),
    ]
    .into_iter()
    .collect();
    if code.is_empty() || code == "und" {
        return None;
    }
    Some(names.get(code.to_ascii_lowercase().as_str()).map(|(f, e)| crate::i18n::tr(f, e).to_string()).unwrap_or_else(|| code.to_uppercase()))
}
