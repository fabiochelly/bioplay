//! Un lecteur ouvert : mpv, l'index des sous-titres, les panneaux et messages, la mémoire.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use bioplay_core::{Cue, Track as SubIndex};

use crate::mpv::{Mpv, Track};

#[derive(Clone, Copy, PartialEq)]
pub enum Panel {
    Tracks,
    Speed,
    Zoom,
    List,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Zone {
    Minute,
    Ten,
    Line,
}

pub struct Scrub {
    pub zone: Zone,
    pub steps: i32,
    pub target: f64,
    pub cue: Option<Cue>,
    pub from: f64,
}

pub struct Recent {
    pub path: String,
    pub name: String,
    pub position: f64,
    pub duration: f64,
}

pub struct Player {
    pub mpv: Mpv,
    /// Pistes intégrées, par numéro de piste du conteneur (`src-id`).
    pub embedded: HashMap<i64, SubIndex>,
    /// Images clés de la vidéo (secondes, MKV) : sauts rapides quand l'une tombe près de la cible.
    pub keyframes: Vec<f64>,
    /// Fichiers de sous-titres ajoutés, par nom donné à mpv.
    pub external: HashMap<String, SubIndex>,
    pub indexing: bool,
    pub message: Option<(String, Instant)>,
    /// Jauge du message en cours (valeur, maximum de l'échelle) : le volume.
    pub gauge: Option<(f64, f64)>,
    /// Message d'un saut : sens (−1, +1), dessiné en icône devant l'heure atteinte, et le détail
    /// (« +10 s », « Sous-titre suivant ») en petit dessous.
    pub seek_dir: Option<i32>,
    pub seek_detail: String,
    pub panel: Option<Panel>,
    pub scrub: Option<Scrub>,
    pub activity: Instant,
    pub recents: Vec<Recent>,
    /// Position par chemin (index de `recents`) : lue pour chaque miniature à chaque image.
    pub positions: HashMap<String, f64>,
    pub library: crate::library::Library,
    /// Miniatures de l'accueil à poser cette image (incrustations mpv sous l'interface).
    pub thumbs: Vec<crate::overlay::Thumb>,
    /// Dossier affiché dans la bibliothèque.
    pub place: crate::library::Place,
    /// Fenêtre des dossiers surveillés ouverte (accueil).
    pub folders_open: bool,
    pub over_ui: bool,
    /// Mesure : position de défilement imposée à la grille (`BIOPLAY_SCROLLTEST`).
    pub test_scroll: Option<f32>,
    /// Dernières valeurs annoncées : un message seulement quand l'utilisateur change quelque chose.
    pub shown: HashMap<&'static str, f64>,
    pub loaded_at: Option<Instant>,
    /// Taille des sous-titres (facteur mpv `sub-scale`) et hauteur ajoutée à leur marge basse
    /// (unités mpv, image de 720 lignes). Communes à toutes les vidéos, mémorisées.
    pub sub_scale: f64,
    pub sub_lift: i32,
    /// Dernier réglage de taille ou de hauteur : un aperçu reste à l'écran un instant.
    pub sub_preview: Option<Instant>,
}

/// Durée d'un message, de son fondu d'entrée et de son fondu de sortie, en secondes.
pub const MESSAGE_LIFE: f32 = 1.05;

/// Avance tolérée (s) sur le début d'une réplique pour sauter sur une image clé.
const LINE_EARLY: f64 = 0.5;
pub const MESSAGE_IN: f32 = 0.08;
pub const MESSAGE_OUT: f32 = 0.25;

/// Opacité d'un message affiché depuis [age] secondes (0 une fois sa durée passée).
pub fn message_alpha(age: f32) -> f32 {
    ((age / MESSAGE_IN).min(1.0) * ((MESSAGE_LIFE - age) / MESSAGE_OUT).min(1.0)).max(0.0)
}

impl Player {
    pub fn new(mpv: Mpv) -> Self {
        let library = crate::library::Library::new(mpv.waker());
        let recents = load_recents();
        let positions = recents.iter().rev().map(|r| (r.path.clone(), r.position)).collect();
        Self {
            library,
            folders_open: false,
            place: Default::default(),
            thumbs: Vec::new(),
            mpv,
            embedded: HashMap::new(),
            keyframes: Vec::new(),
            external: HashMap::new(),
            indexing: false,
            message: None,
            gauge: None,
            seek_dir: None,
            seek_detail: String::new(),
            panel: None,
            scrub: None,
            activity: Instant::now(),
            recents,
            positions,
            over_ui: false,
            test_scroll: None,
            shown: HashMap::new(),
            loaded_at: None,
            sub_scale: 1.0,
            sub_lift: 0,
            sub_preview: None,
        }
        .with_subs()
    }

    fn with_subs(mut self) -> Self {
        let saved = std::fs::read_to_string(subs_file()).unwrap_or_default();
        let mut it = saved.split_whitespace();
        self.sub_scale = it.next().and_then(|v| v.parse().ok()).filter(|v: &f64| (0.5..=2.0).contains(v)).unwrap_or(1.0);
        self.sub_lift = it.next().and_then(|v| v.parse().ok()).filter(|v: &i32| (-40..=400).contains(v)).unwrap_or(0);
        self.apply_subs();
        self
    }

    /// Réapplique la taille (un fichier repris peut avoir mémorisé la sienne dans mpv).
    pub fn apply_subs(&self) {
        self.mpv.set_f64("sub-scale", self.sub_scale);
    }

    /// Taille des sous-titres, par crans de 5 %.
    pub fn sub_size(&mut self, steps: i32) {
        self.sub_scale = ((self.sub_scale * 20.0).round() / 20.0 + steps as f64 * 0.05).clamp(0.5, 2.0);
        self.apply_subs();
        self.save_subs();
        self.sub_preview = Some(Instant::now());
        self.say(format!("{} {:.0} %", crate::i18n::tr("Taille des sous-titres", "Subtitle size"), self.sub_scale * 100.0));
    }

    /// Hauteur des sous-titres, par crans de 6 unités (≈ 1 % de l'image).
    pub fn sub_height(&mut self, steps: i32) {
        self.sub_lift = (self.sub_lift + steps * 6).clamp(-36, 396);
        self.save_subs();
        self.sub_preview = Some(Instant::now());
        self.say(format!("{} {}", crate::i18n::tr("Hauteur des sous-titres", "Subtitle height"), lift_label(self.sub_lift)));
    }

    fn save_subs(&self) {
        let f = subs_file();
        if let Some(d) = f.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(f, format!("{} {}\n", self.sub_scale, self.sub_lift));
    }

    pub fn say(&mut self, text: impl Into<String>) {
        self.gauge = None;
        self.seek_dir = None;
        self.post(text.into());
    }

    /// Saut : l'heure atteinte dans le film, avec une icône du sens.
    pub fn say_seek(&mut self, dir: f64, at: f64, detail: impl Into<String>) {
        self.gauge = None;
        self.seek_dir = Some(if dir < 0.0 { -1 } else { 1 });
        self.seek_detail = detail.into();
        self.post(clock(at.max(0.0)));
    }

    /// Message accompagné d'une jauge : [value] sur une échelle qui va jusqu'à [max] (au moins 100).
    pub fn say_gauge(&mut self, text: impl Into<String>, value: f64, max: f64) {
        self.gauge = Some((value, max));
        self.seek_dir = None;
        self.post(text.into());
    }

    /// Un message qui en remplace un autre encore à l'écran garde son opacité : pas de nouveau
    /// fondu d'entrée, donc pas de clignotement quand les valeurs défilent (molette).
    fn post(&mut self, text: String) {
        let now = Instant::now();
        let at = match &self.message {
            Some((_, old)) => {
                let a = message_alpha(old.elapsed().as_secs_f32());
                now.checked_sub(Duration::from_secs_f32(MESSAGE_IN * a)).unwrap_or(now)
            }
            None => now,
        };
        self.message = Some((text, at));
    }

    pub fn has_file(&self) -> bool {
        !self.mpv.state.idle && !self.mpv.state.path.is_empty()
    }

    pub fn sub_track(&self) -> Option<&Track> {
        let sid = self.mpv.state.sid?;
        self.mpv.state.tracks.iter().find(|t| t.kind == "sub" && t.id == sid)
    }

    pub fn sub_index(&self) -> Option<&SubIndex> {
        let t = self.sub_track()?;
        match &t.external {
            Some(f) => self.external.get(f),
            None => self.embedded.get(&t.src_id),
        }
    }

    /// Temps des sous-titres (décalage compris), en ms.
    pub fn sub_clock(&self, seconds: f64) -> i64 {
        ((seconds - self.mpv.state.sub_delay) * 1000.0) as i64
    }

    pub fn cue_time(&self, c: &Cue) -> f64 {
        c.start as f64 / 1000.0 + self.mpv.state.sub_delay
    }

    pub fn cue_after(&self, steps: i32, from: f64) -> Option<Cue> {
        let idx = self.sub_index()?;
        let i = idx.step(self.sub_clock(from), steps)?;
        Some(clone_cue(&idx.cues[i]))
    }

    /// Texte de l'aperçu : la réplique la plus proche du film, sinon une phrase d'exemple.
    pub fn preview_text(&self) -> String {
        let pos = self.mpv.position();
        self.cue_at(pos)
            .or_else(|| self.cue_after(-1, pos))
            .or_else(|| self.cue_after(1, pos))
            .map(|c| c.text)
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| crate::i18n::tr("Aperçu des sous-titres\nTaille et hauteur", "Subtitle preview\nSize and height").into())
    }

    pub fn cue_at(&self, seconds: f64) -> Option<Cue> {
        let idx = self.sub_index()?;
        let ms = self.sub_clock(seconds);
        let c = &idx.cues[idx.at(ms)?];
        (ms < c.end).then(|| clone_cue(c))
    }

    /// Début d'une réplique : précis (la réplique doit être entendue en entier). Image clé
    /// seulement si elle tombe dans les 500 ms qui précèdent (saut immédiat, rien de perdu).
    pub fn seek_line(&self, t: f64) {
        let k = &self.keyframes;
        let i = k.partition_point(|&x| x <= t);
        match i.checked_sub(1).map(|j| k[j]).filter(|&x| x >= t - LINE_EARLY) {
            Some(key) => self.mpv.seek_keyframe(key, t),
            None => self.mpv.seek(t, true),
        }
    }

    /// Saut d'une durée (10 s, 1 min) depuis `from` : toujours sur une image clé (immédiat ; la
    /// précision compte peu ici), la plus proche de la cible dans le bon sens.
    pub fn seek_span(&self, from: f64, t: f64) {
        let max = if self.mpv.state.duration > 0.0 { self.mpv.state.duration } else { f64::MAX };
        let t = t.clamp(0.0, max);
        let dir = if t >= from { 1.0 } else { -1.0 };
        let k = &self.keyframes;
        let i = k.partition_point(|&x| x < t);
        // Voisines de la cible, puis plus loin dans le bon sens si elles sont du mauvais côté.
        let around = k[i.saturating_sub(2)..(i + 2).min(k.len())].iter().copied();
        let beyond = if dir > 0.0 { k[i.min(k.len())..].iter().copied().find(|&x| x > from + 1.0) } else { k[..i].iter().rev().copied().find(|&x| x < from - 1.0) };
        let key = around
            .chain(beyond)
            .filter(|&x| (x - from) * dir > 1.0)
            .min_by(|a, b| (*a - t).abs().total_cmp(&(*b - t).abs()));
        match key {
            Some(key) => self.mpv.seek_keyframe(key, key),
            // Sans index (MP4…) : saut relatif de mpv à l'image clé, qui va dans le bon sens.
            None if k.is_empty() => self.mpv.seek_relative_keyframe(t - from, t),
            None => self.mpv.seek(t, false),
        }
    }

    /// Saut de ligne en ligne : exact grâce à l'index, sinon `sub-seek` de mpv.
    pub fn step_sub(&mut self, steps: i32) {
        if steps == 0 {
            return;
        }
        match self.cue_after(steps, self.mpv.pos()) {
            Some(c) => self.seek_line(self.cue_time(&c)),
            None => {
                self.mpv.own_seeks.set(self.mpv.own_seeks.get() + 1);
                self.mpv.command(&["sub-seek", &steps.to_string()]);
            }
        }
    }

    /// Flèches : saut à la ligne suivante (ou précédente), limité à 10 s. Sans index de
    /// sous-titres (MP4, AVI…), simple saut de 10 s.
    pub fn step_line_or_ten(&mut self, dir: i32) {
        let pos = self.mpv.pos();
        let ten = pos + 10.0 * dir as f64;
        let line = self.cue_after(dir, pos).map(|c| self.cue_time(&c)).filter(|t| {
            let ahead = (t - pos) * dir as f64;
            ahead > 0.05 && ahead <= 10.0
        });
        // Réplique (à moins de 10 s) : précise. Sinon 10 s : toujours une image clé, et la bulle
        // montre le saut réel (+8 s, +13 s…).
        let target = match line {
            Some(t) => {
                self.seek_line(t);
                t
            }
            None => {
                self.seek_span(pos, ten);
                self.mpv.target.get().map_or(ten, |t| t.at)
            }
        };
        let tr = crate::i18n::tr;
        let detail = match (line.is_some(), dir > 0) {
            (true, true) => tr("Sous-titre suivant", "Next subtitle").to_string(),
            (true, false) => tr("Sous-titre précédent", "Previous subtitle").to_string(),
            (false, _) => delta(target - pos),
        };
        self.say_seek(dir as f64, target, detail);
    }

    pub fn preview(&self, zone: Zone, steps: i32, from: f64) -> Scrub {
        let max = if self.mpv.state.duration > 0.0 { self.mpv.state.duration } else { f64::MAX };
        match zone {
            Zone::Line => {
                let cue = self.cue_after(steps, from);
                let target = cue.as_ref().map(|c| self.cue_time(c)).unwrap_or(from);
                Scrub { zone, steps, target, cue, from }
            }
            Zone::Ten | Zone::Minute => {
                let unit = if zone == Zone::Ten { 10.0 } else { 60.0 };
                let target = (from + steps as f64 * unit).clamp(0.0, max);
                Scrub { zone, steps, target, cue: self.cue_at(target), from }
            }
        }
    }

    // --- Mémoire -------------------------------------------------------------------------------

    /// La position est écrite par mpv lui-même (`watch_later`, partagée avec le mpv habituel) :
    /// on reprend au même endroit dans les deux lecteurs.
    pub fn save(&mut self) {
        if !self.has_file() {
            return;
        }
        self.mpv.command_sync(&["write-watch-later-config"]);
        if !self.mpv.is_seeking() {
            self.mpv.refresh_position();
        }
        let s = &self.mpv.state;
        let path = s.path.clone();
        let name = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| s.title.clone());
        let (pos, dur) = (self.mpv.position(), s.duration);
        self.recents.retain(|r| r.path != path);
        self.positions.insert(path.clone(), pos);
        self.recents.insert(0, Recent { path, name, position: pos, duration: dur });
        // Gardée longtemps : c'est elle qui donne l'avancée de chaque miniature de l'accueil.
        self.recents.truncate(5000);
        save_recents(&self.recents);
    }
}

fn clone_cue(c: &Cue) -> Cue {
    Cue { start: c.start, end: c.end, text: c.text.clone() }
}

/// « 0 », « +12 », « −6 » : hauteur ajoutée, en crans affichés comme des points.
pub fn lift_label(lift: i32) -> String {
    let n = lift / 6;
    if n == 0 { "0".into() } else { format!("{n:+}").replace('-', "−") }
}

fn subs_file() -> PathBuf {
    state_file().with_file_name("subs")
}

fn state_file() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/state"));
    base.join("bioplay/recents.tsv")
}

fn load_recents() -> Vec<Recent> {
    std::fs::read_to_string(state_file())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() == 4).then(|| Recent {
                path: f[0].into(),
                name: f[1].into(),
                position: f[2].parse().unwrap_or(0.0),
                duration: f[3].parse().unwrap_or(0.0),
            })
        })
        .collect()
}

fn save_recents(list: &[Recent]) {
    let f = state_file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let body: String = list
        .iter()
        .map(|r| format!("{}\t{}\t{:.1}\t{:.1}\n", r.path.replace('\t', " "), r.name.replace('\t', " "), r.position, r.duration))
        .collect();
    let _ = std::fs::write(f, body);
}

/// « +10 s », « −1 min 30 s » : un écart de temps lisible.
pub fn delta(d: f64) -> String {
    let sign = if d < 0.0 { "−" } else { "+" };
    let s = d.abs().round() as i64;
    match (s / 60, s % 60) {
        (0, sec) => format!("{sign}{sec} s"),
        (m, 0) => format!("{sign}{m} min"),
        (m, sec) => format!("{sign}{m} min {sec} s"),
    }
}

pub fn clock(seconds: f64) -> String {
    let s = seconds.abs() as i64;
    let sign = if seconds < 0.0 { "−" } else { "" };
    if s >= 3600 {
        format!("{sign}{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{sign}{}:{:02}", s / 60, s % 60)
    }
}

pub fn speed_label(s: f64) -> String {
    if (s - 1.0).abs() < 0.001 {
        return "1×".into();
    }
    let t = format!("{s:.2}");
    format!("{}×", t.trim_end_matches('0').trim_end_matches('.'))
}
