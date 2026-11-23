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

// libmpv est chargée au démarrage (`load`), pas liée à la compilation : le binaire se construit
// sur une distribution ancienne (bibliothèque C compatible partout) et cherche ici
// `libmpv.so.2` ; sans elle, ou trop ancienne, un message clair plutôt qu'un échec du chargeur.
macro_rules! mpv_api {
    ($($name:ident($($arg:ident: $ty:ty),*) $(-> $ret:ty)?;)*) => {
        #[allow(non_snake_case)]
        struct Api {
            _lib: libloading::Library,
            $($name: unsafe extern "C" fn($($ty),*) $(-> $ret)?,)*
        }

        impl Api {
            fn open() -> Result<Api, String> {
                let mut last = String::new();
                for file in ["libmpv.so.2", "libmpv.so"] {
                    // Liaison différée (RTLD_LAZY), comme le chargeur du système : une liaison
                    // immédiate des ~200 bibliothèques de mpv coûtait 110 à 190 ms au lancement.
                    let opened = unsafe { libloading::os::unix::Library::open(Some(file), libloading::os::unix::RTLD_LAZY | libloading::os::unix::RTLD_LOCAL) };
                    match opened.map(libloading::Library::from) {
                        Ok(lib) => unsafe {
                            $(let $name = *lib
                                .get::<unsafe extern "C" fn($($ty),*) $(-> $ret)?>(concat!(stringify!($name), "\0").as_bytes())
                                .map_err(|e| format!("{} : {e} (mpv trop ancien)", stringify!($name)))?;)*
                            return Ok(Api { _lib: lib, $($name,)* });
                        },
                        Err(e) => last = e.to_string(),
                    }
                }
                Err(format!("libmpv introuvable ({last})"))
            }
        }

        $(
            #[allow(clippy::missing_safety_doc)]
            unsafe fn $name($($arg: $ty),*) $(-> $ret)? {
                unsafe { (api().$name)($($arg),*) }
            }
        )*
    };
}

mpv_api! {
    mpv_create() -> *mut mpv_handle;
    mpv_initialize(h: *mut mpv_handle) -> c_int;
    mpv_terminate_destroy(h: *mut mpv_handle);
    mpv_set_option_string(h: *mut mpv_handle, name: *const c_char, data: *const c_char) -> c_int;
    mpv_set_property(h: *mut mpv_handle, name: *const c_char, format: c_int, data: *mut c_void) -> c_int;
    mpv_get_property_string(h: *mut mpv_handle, name: *const c_char) -> *mut c_char;
    mpv_command_async(h: *mut mpv_handle, reply: u64, args: *mut *const c_char) -> c_int;
    mpv_command(h: *mut mpv_handle, args: *mut *const c_char) -> c_int;
    mpv_observe_property(h: *mut mpv_handle, reply: u64, name: *const c_char, format: c_int) -> c_int;
    mpv_set_property_async(h: *mut mpv_handle, reply: u64, name: *const c_char, format: c_int, data: *mut c_void) -> c_int;
    mpv_wait_event(h: *mut mpv_handle, timeout: c_double) -> *mut Event;
    mpv_free(data: *mut c_void);
    mpv_wakeup(h: *mut mpv_handle);
    mpv_client_api_version() -> std::os::raw::c_ulong;
}

static API: std::sync::OnceLock<Result<Api, String>> = std::sync::OnceLock::new();

fn api() -> &'static Api {
    API.get_or_init(Api::open).as_ref().expect("libmpv chargée (voir `load`)")
}

/// Version minimale de l'API client : libmpv 2 (mpv 0.35 et suivants ; 0.38 ou plus récent
/// recommandé, non vérifié ici en détail).

/// Charge libmpv et vérifie sa version ; message lisible sinon.
pub fn load() -> Result<(), String> {
    API.get_or_init(Api::open).as_ref().map_err(Clone::clone)?;
    let v = unsafe { mpv_client_api_version() } as u64;
    let (major, minor) = (v >> 16, v & 0xffff);
    if major < 2 {
        return Err(format!("mpv trop ancien (API {major}.{minor} ; il faut mpv 0.35 ou plus récent, 0.38 recommandé)"));
    }
    Ok(())
}

const FORMAT_STRING: c_int = 1;
const FORMAT_FLAG: c_int = 3;
const FORMAT_INT64: c_int = 4;
const FORMAT_DOUBLE: c_int = 5;

const EVENT_SHUTDOWN: c_int = 1;
const EVENT_COMMAND_REPLY: c_int = 5;
/// Réponses aux `overlay-add` : `OVERLAY_TAG + numéro d'envoi`.
pub const OVERLAY_TAG: u64 = 1 << 62;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const EVENT_CLIENT_MESSAGE: c_int = 16;
const EVENT_SEEK: c_int = 20;
const EVENT_PLAYBACK_RESTART: c_int = 21;
const EVENT_PROPERTY_CHANGE: c_int = 22;

/// Réveille la boucle depuis un autre fil (index, sélecteur de fichiers, bibliothèque). Le
/// lecteur peut avoir été détruit entre-temps (fermeture pendant un index) : l'adresse est remise
/// à zéro sous verrou avant la destruction, et `wake` ne fait alors plus rien.
#[derive(Clone)]
pub struct Waker(std::sync::Arc<std::sync::Mutex<usize>>);

impl Waker {
    pub fn wake(&self) {
        let h = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if *h != 0 {
            unsafe { mpv_wakeup(*h as *mut mpv_handle) };
        }
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
    pub title: String,
    pub path: String,
    pub idle: bool,
    pub zoom: f64,
    /// Fenêtre (pixels) et position de l'image vidéo dedans (marges des bandes noires).
    pub osd_w: f64,
    pub osd_h: f64,
    pub margin_left: f64,
    pub margin_top: f64,
    pub margin_bottom: f64,
    pub scale: f64,
    pub hdr: bool,
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
    ("duration", FORMAT_DOUBLE),
    ("pause", FORMAT_FLAG),
    ("speed", FORMAT_DOUBLE),
    ("volume", FORMAT_DOUBLE),
    ("sub-delay", FORMAT_DOUBLE),
    ("audio-delay", FORMAT_DOUBLE),
    ("track-list", FORMAT_STRING),
    ("aid", FORMAT_STRING),
    ("sid", FORMAT_STRING),
    ("media-title", FORMAT_STRING),
    ("path", FORMAT_STRING),
    ("idle-active", FORMAT_FLAG),
    ("video-zoom", FORMAT_DOUBLE),
    ("osd-dimensions", FORMAT_STRING),
    ("mouse-pos", FORMAT_STRING),
    ("display-hidpi-scale", FORMAT_DOUBLE),
    ("video-params/gamma", FORMAT_STRING),
];

#[derive(Clone, Copy)]
pub struct Target {
    /// Saut précis (long : mpv vide sa file d'images, l'interface attend la reprise).
    pub exact: bool,
    pub at: f64,
    pub landing: f64,
    pub sent: std::time::Instant,
    pub done: bool,
}

pub struct Mpv {
    h: *mut mpv_handle,
    waker: Waker,
    pub state: State,
    /// Sauts lancés par bioplay (les autres viennent des raccourcis : annoncés dans une bulle).
    pub own_seeks: std::cell::Cell<u32>,
    /// Dernier `overlay-add` traité par mpv (il lit l'image à ce moment-là, pas avant).
    pub overlay_done: std::cell::Cell<u64>,
    /// Cible du dernier saut lancé par bioplay, tant qu'il n'est pas terminé.
    /// (cible, envoi, terminé) : mpv fond les sauts rapprochés, si bien qu'une fin de saut
    /// peut précéder un saut encore en file ; la cible reste donc valable une seconde.
    /// Avec un saut à l'image clé, l'image atteinte (`landing`) précède un peu la cible : tant
    /// que la lecture n'a pas quitté cet intervalle, la cible reste la référence (sinon
    /// « sous-titre suivant » viserait de nouveau la même réplique).
    pub target: std::cell::Cell<Option<Target>>,
    /// Dernière position lue chez mpv et son instant (la position n'est pas suivie).
    anchor: (f64, std::time::Instant),
    seeking: bool,
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
        let m = Self { h, waker: Waker(std::sync::Arc::new(std::sync::Mutex::new(h as usize))), state: State { speed: 1.0, scale: 1.0, ..Default::default() }, own_seeks: Default::default(), overlay_done: Default::default(), target: Default::default(), anchor: (0.0, std::time::Instant::now()), seeking: false };
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
        // Mesure (BIOPLAY_BENCH) : aucune fenêtre, aucune configuration de l'utilisateur.
        if std::env::var_os("BIOPLAY_BENCH").is_some() {
            for (k, v) in [("vo", "null"), ("ao", "null"), ("force-window", "no"), ("config", "no")] {
                m.option(k, v);
            }
        }
        if unsafe { mpv_initialize(h) } < 0 {
            return Err("mpv_initialize a échoué".into());
        }
        // Après la configuration : nos choix qui priment sur mpv.conf.
        m.set("osd-level", "0"); // les messages sont les nôtres, en verre
        m.set("save-position-on-quit", "yes"); // reprise toujours, quoi qu'il arrive
        m.set("resume-playback", "yes");
        // Sous-titres dans l'image (pas dans les bandes noires), Inter Medium, blanc adouci, liseré
        // noir net et sans ombre ; appliqué aussi aux ASS dont le style imposait des tailles démesurées.
        for (k, v) in [
            ("sub-use-margins", "no"),
            ("sub-ass-force-margins", "no"),
            ("sub-font-size", "36"),
            ("sub-font", "Inter Medium"),
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

    /// Sans attendre mpv (il copie la valeur tout de suite) : appelé à chaque image pour le verre
    /// et la marge des sous-titres, il ne doit pas bloquer l'interface pendant un saut.
    pub fn set(&self, k: &str, v: &str) {
        crate::trace::count_with(|| format!("mpv set {k}"));
        let v = cs(v);
        let mut ptr = v.as_ptr();
        unsafe { mpv_set_property_async(self.h, 0, cs(k).as_ptr(), FORMAT_STRING, &mut ptr as *mut *const c_char as *mut c_void) };
    }

    // --- Position sans suivi continu ---------------------------------------------------------
    //
    // `time-pos` change à chaque image : le suivre réveillerait bioplay des dizaines de fois par
    // seconde. Il n'est jamais suivi ; la position est déduite de la dernière lue et de l'horloge,
    // relue chez mpv à chaque reprise après un saut, pause, changement de vitesse ou de fichier.

    pub fn is_seeking(&self) -> bool {
        self.seeking
    }



    /// Relit la position chez mpv (appel bref, hors saut en cours).
    pub fn refresh_position(&mut self) {
        if let Some(v) = self.get("time-pos").and_then(|v| v.parse::<f64>().ok()) {
            self.state.position = v;
            self.anchor = (v, std::time::Instant::now());
        }
    }

    /// Position actuelle : suivie, ou déduite de la dernière lue et du temps écoulé.
    pub fn position(&self) -> f64 {
        let s = &self.state;
        let (p, at) = self.anchor;
        if s.paused || s.idle || self.seeking {
            return p;
        }
        let max = if s.duration > 0.0 { s.duration } else { f64::MAX };
        (p + at.elapsed().as_secs_f64() * s.speed).clamp(0.0, max)
    }

    pub fn set_f64(&self, k: &str, mut v: f64) {
        crate::trace::count_with(|| format!("mpv set {k}"));
        unsafe { mpv_set_property(self.h, cs(k).as_ptr(), FORMAT_DOUBLE, &mut v as *mut f64 as *mut c_void) };
    }

    pub fn get(&self, k: &str) -> Option<String> {
        crate::trace::count_with(|| format!("mpv get (synchrone) {k}"));
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
        crate::trace::count_with(|| format!("mpv commande {}", args.first().unwrap_or(&"")));
        let owned: Vec<CString> = args.iter().map(|a| cs(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(ptr::null());
        unsafe { mpv_command_async(self.h, 0, ptrs.as_mut_ptr()) };
    }

    /// Commande asynchrone dont la fin est signalée par `EVENT_COMMAND_REPLY` (`reply`).
    pub fn command_reply(&self, reply: u64, args: &[&str]) {
        crate::trace::count_with(|| format!("mpv commande {}", args.first().unwrap_or(&"")));
        let owned: Vec<CString> = args.iter().map(|a| cs(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(ptr::null());
        unsafe { mpv_command_async(self.h, reply, ptrs.as_mut_ptr()) };
    }

    /// Commande synchrone (pour `quit` et l'écriture de la position).
    pub fn command_sync(&self, args: &[&str]) {
        crate::trace::count_with(|| format!("mpv commande (synchrone) {}", args.first().unwrap_or(&"")));
        let owned: Vec<CString> = args.iter().map(|a| cs(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(ptr::null());
        unsafe { mpv_command(self.h, ptrs.as_mut_ptr()) };
    }

    pub fn seek(&self, t: f64, exact: bool) {
        let max = if self.state.duration > 0.0 { self.state.duration } else { f64::MAX };
        let t = t.clamp(0.0, max);
        self.own_seeks.set(self.own_seeks.get() + 1);
        self.target.set(Some(Target { exact, at: t, landing: t, sent: std::time::Instant::now(), done: false }));
        self.command(&["seek", &format!("{t:.3}"), if exact { "absolute+exact" } else { "absolute+keyframes" }]);
    }

    /// Saut rapide sur l'image clé `key` (≈ 30 ms au lieu de 200 à 700 ms pour un saut précis),
    /// pour viser `t` qui en est tout près. Un saut absolu à l'image clé va à celle qui précède
    /// la demande : on demande donc à peine après `key`.
    pub fn seek_keyframe(&self, key: f64, t: f64) {
        self.own_seeks.set(self.own_seeks.get() + 1);
        self.target.set(Some(Target { exact: false, at: t, landing: key, sent: std::time::Instant::now(), done: false }));
        self.command(&["seek", &format!("{:.3}", key + 0.001), "absolute+keyframes"]);
    }

    /// Saut relatif à l'image clé (fichiers sans index d'images clés) : mpv le fait dans le bon
    /// sens et cumule les appuis rapprochés.
    pub fn seek_relative_keyframe(&self, by: f64, t: f64) {
        self.own_seeks.set(self.own_seeks.get() + 1);
        self.target.set(Some(Target { exact: false, at: t, landing: t, sent: std::time::Instant::now(), done: false }));
        self.command(&["seek", &format!("{by:.3}"), "relative+keyframes"]);
    }

    /// Position d'où partir pour un saut relatif : la cible du saut en cours s'il y en a un
    /// (sinon des appuis rapprochés viseraient tous le même instant), sinon la position.
    pub fn pos(&self) -> f64 {
        let p = self.position();
        match self.target.get() {
            Some(t) if !t.done || t.sent.elapsed() < std::time::Duration::from_secs(1) => t.at,
            Some(t) if p >= t.at.min(t.landing) - 0.1 && p <= t.at.max(t.landing) + 0.1 => t.at,
            _ => p,
        }
    }

    /// Réveille `wait` depuis un autre fil (index des sous-titres prêt, fichier choisi…).
    pub fn waker(&self) -> Waker {
        self.waker.clone()
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
        if crate::trace::on() && e.event_id != 0 {
            let what = match e.event_id {
                EVENT_PROPERTY_CHANGE => format!("mpv événement : propriété {}", OBSERVED.get(e.reply_userdata as usize).map_or("?", |p| p.0)),
                EVENT_COMMAND_REPLY if e.reply_userdata >= OVERLAY_TAG => "mpv événement : réponse overlay-add/remove".into(),
                id => format!("mpv événement : n° {id}"),
            };
            crate::trace::count(&what);
        }
        match e.event_id {
            EVENT_SHUTDOWN => out.push(Change::Shutdown),
            EVENT_COMMAND_REPLY if e.reply_userdata >= OVERLAY_TAG => {
                let n = e.reply_userdata - OVERLAY_TAG;
                self.overlay_done.set(self.overlay_done.get().max(n));
            }
            EVENT_FILE_LOADED => out.push(Change::Loaded),
            EVENT_END_FILE => out.push(Change::Ended),
            EVENT_SEEK => {
                // Position d'avant le saut figée (sert à annoncer l'écart d'un saut venu de mpv).
                if !self.seeking {
                    self.anchor = (self.position(), std::time::Instant::now());
                }
                self.seeking = true;
                out.push(Change::Seeking)
            }
            EVENT_PLAYBACK_RESTART => {
                self.seeking = false;
                self.refresh_position();
                if let Some(t) = self.target.get() {
                    self.target.set(Some(Target { done: true, ..t }));
                }
                out.push(Change::Restarted)
            }
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
                    if matches!(name, "pause" | "speed" | "path" | "duration") {
                        self.refresh_position();
                    }
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
            "duration" => s.duration = double().unwrap_or(0.0),
            "speed" => s.speed = double().unwrap_or(1.0),
            "volume" => s.volume = double().unwrap_or(100.0),
            "sub-delay" => s.sub_delay = double().unwrap_or(0.0),
            "audio-delay" => s.audio_delay = double().unwrap_or(0.0),
            "video-zoom" => s.zoom = double().unwrap_or(0.0),
            "pause" => {
                s.paused = flag().unwrap_or(false);
            }
            "idle-active" => s.idle = flag().unwrap_or(false),
            "track-list" => s.tracks = string().map(|j| parse_tracks(&j)).unwrap_or_default(),
            "aid" => s.aid = string().and_then(|v| v.parse().ok()),
            "sid" => s.sid = string().and_then(|v| v.parse().ok()),
            "media-title" => s.title = string().unwrap_or_default(),
            "path" => s.path = string().unwrap_or_default(),
            "display-hidpi-scale" => s.scale = double().filter(|v| *v > 0.0).unwrap_or(1.0),
            // Vidéo HDR (PQ, HLG) : mpv éclaircit et sature alors les couleurs de l'interface.
            "video-params/gamma" => s.hdr = string().is_some_and(|g| g == "pq" || g == "hlg"),
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
        *self.waker.0.lock().unwrap_or_else(|e| e.into_inner()) = 0;
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
    // Table construite une fois (appelée à chaque image de la barre).
    static NAMES: std::sync::OnceLock<HashMap<&'static str, (&'static str, &'static str)>> = std::sync::OnceLock::new();
    let names = NAMES.get_or_init(|| [
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
    .collect());
    if code.is_empty() || code == "und" {
        return None;
    }
    Some(names.get(code.to_ascii_lowercase().as_str()).map(|(f, e)| crate::i18n::tr(f, e).to_string()).unwrap_or_else(|| code.to_uppercase()))
}
