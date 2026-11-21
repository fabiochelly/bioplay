//! Un lecteur ouvert : mpv, l'index des sous-titres, les panneaux et messages, la mémoire.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

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
    /// Fichiers de sous-titres ajoutés, par nom donné à mpv.
    pub external: HashMap<String, SubIndex>,
    pub indexing: bool,
    pub message: Option<(String, Instant)>,
    pub panel: Option<Panel>,
    pub scrub: Option<Scrub>,
    pub activity: Instant,
    pub recents: Vec<Recent>,
    pub over_ui: bool,
    /// Dernières valeurs annoncées : un message seulement quand l'utilisateur change quelque chose.
    pub shown: HashMap<&'static str, f64>,
    pub loaded_at: Option<Instant>,
}

impl Player {
    pub fn new(mpv: Mpv) -> Self {
        Self {
            mpv,
            embedded: HashMap::new(),
            external: HashMap::new(),
            indexing: false,
            message: None,
            panel: None,
            scrub: None,
            activity: Instant::now(),
            recents: load_recents(),
            over_ui: false,
            shown: HashMap::new(),
            loaded_at: None,
        }
    }

    pub fn say(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), Instant::now()));
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

    pub fn cue_at(&self, seconds: f64) -> Option<Cue> {
        let idx = self.sub_index()?;
        let ms = self.sub_clock(seconds);
        let c = &idx.cues[idx.at(ms)?];
        (ms < c.end).then(|| clone_cue(c))
    }

    /// Saut de ligne en ligne : exact grâce à l'index, sinon `sub-seek` de mpv.
    pub fn step_sub(&mut self, steps: i32) {
        if steps == 0 {
            return;
        }
        match self.cue_after(steps, self.mpv.state.position) {
            Some(c) => self.mpv.seek(self.cue_time(&c), true),
            None => self.mpv.command(&["sub-seek", &steps.to_string()]),
        }
    }

    /// Flèches : saut à la ligne suivante (ou précédente), limité à 10 s. Sans index de
    /// sous-titres (MP4, AVI…), simple saut de 10 s.
    pub fn step_line_or_ten(&mut self, dir: i32) {
        let pos = self.mpv.state.position;
        let ten = pos + 10.0 * dir as f64;
        let target = match self.cue_after(dir, pos) {
            Some(c) => {
                let t = self.cue_time(&c);
                let ahead = (t - pos) * dir as f64;
                if ahead > 0.05 && ahead <= 10.0 { t } else { ten }
            }
            None => ten,
        };
        self.mpv.seek(target, true);
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
        let s = &self.mpv.state;
        let path = s.path.clone();
        let name = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| s.title.clone());
        let (pos, dur) = (s.position, s.duration);
        self.recents.retain(|r| r.path != path);
        self.recents.insert(0, Recent { path, name, position: pos, duration: dur });
        self.recents.truncate(8);
        save_recents(&self.recents);
    }

    pub fn forget(&mut self, path: &str) {
        self.recents.retain(|r| r.path != path);
        save_recents(&self.recents);
    }
}

fn clone_cue(c: &Cue) -> Cue {
    Cue { start: c.start, end: c.end, text: c.text.clone() }
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
        .filter(|r| Path::new(&r.path).exists())
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
