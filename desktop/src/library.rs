//! Bibliothèque : les vidéos des dossiers surveillés et leurs miniatures.
//!
//! Les dossiers sont dans `~/.local/state/bioplay/folders` (un chemin par ligne). La liste des
//! vidéos est gardée dans `~/.cache/bioplay/library.tsv` et affichée tout de suite au lancement ;
//! les dossiers sont relus dans un fil à part et la vue n'est mise à jour que si quelque chose a
//! changé. Les miniatures (PNG 320 px, bandes noires ôtées, tirées par ffmpeg) vont dans
//! `~/.cache/bioplay/thumbs`, nommées d'après le fichier : créées une fois, gardées tant que la
//! vidéo existe, effacées quand elle disparaît. Rien ne tourne pendant la lecture.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use crate::mpv::Waker;

pub const VIDEO_EXT: &[&str] = &["mkv", "mk3d", "mp4", "m4v", "avi", "webm", "mov", "ts", "m2ts", "mts", "wmv", "flv", "mpg", "mpeg", "ogv", "3gp"];
const THUMB_W: u32 = 320;

#[derive(Clone, PartialEq)]
pub struct Video {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified: u64,
    pub folder: String,
    pub duration: f64,
    /// Sous-dossier, relatif au dossier surveillé (« Série/Saison 1 », vide à la racine).
    pub dir: String,
}

/// Endroit de la bibliothèque : un dossier surveillé et un sous-dossier ; racine si `root` est vide.
#[derive(Clone, Default, PartialEq, Eq, Hash, Debug)]
pub struct Place {
    pub root: Option<String>,
    pub path: String,
}

/// Un dossier à l'écran (jamais vide) : nom, où il mène, vidéos en dessous, sous-dossiers directs.
pub struct Dir {
    pub name: String,
    pub place: Place,
    pub videos: usize,
    pub dirs: usize,
}

/// Contenu d'un endroit : dossiers puis vidéos, par ordre naturel.
pub struct Shelf {
    pub place: Place,
    pub title: String,
    pub dirs: Vec<Dir>,
    pub videos: Vec<usize>,
}

/// Ordre « naturel » : « Épisode 2 » avant « Épisode 10 ».
pub fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    fn chunks(s: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in s.to_lowercase().chars() {
            let d = c.is_ascii_digit();
            match out.last_mut() {
                Some((k, t)) if *k == d => t.push(c),
                _ => out.push((d, c.to_string())),
            }
        }
        out
    }
    let (x, y) = (chunks(a), chunks(b));
    for (p, q) in x.iter().zip(&y) {
        let c = if p.0 && q.0 {
            let (a, b) = (p.1.trim_start_matches('0'), q.1.trim_start_matches('0'));
            a.len().cmp(&b.len()).then(a.cmp(b)).then(p.1.len().cmp(&q.1.len()))
        } else {
            p.1.cmp(&q.1)
        };
        if c != std::cmp::Ordering::Equal {
            return c;
        }
    }
    x.len().cmp(&y.len())
}

impl Video {
    /// Nom de la miniature : change si le fichier est remplacé.
    pub fn thumb(&self) -> String {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&self.name, self.size, self.modified).hash(&mut h);
        format!("{:016x}", h.finish())
    }

    /// Titre lisible : sans extension, points et soulignés remplacés par des espaces.
    pub fn title(&self) -> String {
        let stem = Path::new(&self.name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| self.name.clone());
        stem.replace(['.', '_'], " ")
    }
}

enum Event {
    Scanned(Vec<Video>),
    Thumb(String),
    Durations(Vec<(String, f64)>),
}

pub struct Library {
    pub folders: Vec<PathBuf>,
    pub videos: Vec<Video>,
    pub scanning: bool,
    pub grid: bool,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    waker: Waker,
    hold: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    again: bool,
    /// Miniatures lues du disque (None : illisible).
    images: HashMap<String, Option<Arc<egui::ColorImage>>>,
}

impl Library {
    pub fn new(waker: Waker) -> Self {
        let (tx, rx) = channel();
        let folders = std::fs::read_to_string(folders_file())
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(PathBuf::from)
            .collect();
        let grid = std::fs::read_to_string(view_file()).map(|s| s.trim() != "list").unwrap_or(true);
        let mut lib = Self {
            folders,
            videos: read_list(),
            scanning: false,
            grid,
            tx,
            rx,
            waker,
            hold: Arc::new(AtomicBool::new(false)),
            busy: Arc::new(AtomicBool::new(false)),
            again: false,
            images: HashMap::new(),
        };
        lib.refresh();
        lib
    }

    /// Pendant la lecture : la relecture et les miniatures attendent.
    pub fn set_hold(&self, on: bool) {
        self.hold.store(on, Ordering::Relaxed);
    }

    pub fn toggle_view(&mut self) {
        self.grid = !self.grid;
        write(&view_file(), if self.grid { "grid" } else { "list" });
    }

    pub fn add_folder(&mut self, f: PathBuf) {
        if !self.folders.contains(&f) {
            self.folders.push(f);
            self.save_folders();
            self.refresh();
        }
    }

    pub fn remove_folder(&mut self, f: &Path) {
        self.folders.retain(|x| x != f);
        let key = f.to_string_lossy();
        self.videos.retain(|v| v.folder != key);
        self.save_folders();
        self.refresh();
    }

    /// Dossiers surveillés qui contiennent au moins une vidéo.
    fn roots(&self) -> Vec<String> {
        self.folders.iter().map(|f| f.to_string_lossy().into_owned()).filter(|k| self.videos.iter().any(|v| &v.folder == k)).collect()
    }

    pub fn shelf(&self, place: &Place) -> Shelf {
        Tree { folders: &self.folders, videos: &self.videos }.shelf(place)
    }

    pub fn up(&self, place: &Place) -> Option<Place> {
        Tree { folders: &self.folders, videos: &self.videos }.up(place)
    }
}

/// Arborescence des vidéos, déduite de leurs sous-dossiers (un dossier sans vidéo n'existe pas).
struct Tree<'a> {
    folders: &'a [PathBuf],
    videos: &'a [Video],
}

impl Tree<'_> {
    fn roots(&self) -> Vec<String> {
        self.folders.iter().map(|f| f.to_string_lossy().into_owned()).filter(|k| self.videos.iter().any(|v| &v.folder == k)).collect()
    }

    /// Ce qu'on voit à cet endroit. Un seul dossier surveillé : on est directement dedans.
    fn shelf(&self, place: &Place) -> Shelf {
        let roots = self.roots();
        let root = place.root.clone().or_else(|| (roots.len() == 1).then(|| roots[0].clone()));
        let Some(root) = root else {
            let mut dirs: Vec<Dir> = roots
                .iter()
                .map(|k| {
                    let mine: Vec<&Video> = self.videos.iter().filter(|v| &v.folder == k).collect();
                    let mut subs: Vec<&str> = mine.iter().map(|v| v.dir.split('/').next().unwrap_or("")).filter(|d| !d.is_empty()).collect();
                    subs.sort_unstable();
                    subs.dedup();
                    let name = Path::new(k).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| k.clone());
                    Dir { name, place: Place { root: Some(k.clone()), path: String::new() }, videos: mine.len(), dirs: subs.len() }
                })
                .collect();
            dirs.sort_by(|a, b| natural(&a.name, &b.name));
            return Shelf { place: Place::default(), title: String::new(), dirs, videos: Vec::new() };
        };
        let path = &place.path;
        let prefix = format!("{path}/");
        let inside = |v: &Video| v.folder == root && (path.is_empty() || &v.dir == path || v.dir.starts_with(&prefix));
        let mut videos: Vec<usize> = (0..self.videos.len()).filter(|&i| inside(&self.videos[i]) && &self.videos[i].dir == path).collect();
        videos.sort_by(|&a, &b| natural(&self.videos[a].name, &self.videos[b].name));
        let mut groups: HashMap<String, (usize, HashSet<String>)> = HashMap::new();
        for v in self.videos.iter().filter(|v| inside(v) && &v.dir != path) {
            let rest = if path.is_empty() { v.dir.as_str() } else { &v.dir[prefix.len()..] };
            let (name, deeper) = rest.split_once('/').map_or((rest, None), |(a, b)| (a, Some(b)));
            let g = groups.entry(name.to_owned()).or_default();
            g.0 += 1;
            if let Some(d) = deeper {
                g.1.insert(d.split('/').next().unwrap_or("").to_owned());
            }
        }
        let mut dirs: Vec<Dir> = groups
            .into_iter()
            .map(|(name, (n, subs))| {
                let sub = if path.is_empty() { name.clone() } else { format!("{path}/{name}") };
                Dir { name, place: Place { root: Some(root.clone()), path: sub }, videos: n, dirs: subs.len() }
            })
            .collect();
        dirs.sort_by(|a, b| natural(&a.name, &b.name));
        let title = if path.is_empty() {
            Path::new(&root).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        } else {
            path.rsplit('/').next().unwrap_or("").to_owned()
        };
        Shelf { place: Place { root: Some(root), path: path.clone() }, title, dirs, videos }
    }

    /// L'endroit au-dessus (None si on est déjà en haut).
    fn up(&self, place: &Place) -> Option<Place> {
        let root = place.root.clone()?;
        if !place.path.is_empty() {
            return Some(Place { root: Some(root), path: place.path.rsplit_once('/').map_or(String::new(), |(a, _)| a.to_owned()) });
        }
        (self.roots().len() > 1).then(Place::default)
    }
}

impl Library {
    pub fn count(&self, f: &Path) -> usize {
        let key = f.to_string_lossy();
        self.videos.iter().filter(|v| v.folder == key).count()
    }

    fn save_folders(&self) {
        let body: String = self.folders.iter().map(|f| format!("{}\n", f.to_string_lossy())).collect();
        write(&folders_file(), &body);
    }

    /// Relit les dossiers dans un fil à part (une seule relecture à la fois).
    pub fn refresh(&mut self) {
        if self.busy.swap(true, Ordering::AcqRel) {
            self.again = true;
            return;
        }
        self.scanning = true;
        let roots = self.folders.clone();
        let old = self.videos.clone();
        let (tx, waker, hold, busy) = (self.tx.clone(), self.waker, self.hold.clone(), self.busy.clone());
        std::thread::spawn(move || {
            wait(&hold);
            let mut found = Vec::new();
            let mut complete = true;
            for root in &roots {
                let key = root.to_string_lossy().into_owned();
                if root.is_dir() {
                    scan(root, &key, "", 0, &mut found);
                } else {
                    // Disque débranché : on garde ce qu'on en savait.
                    complete = false;
                    found.extend(old.iter().filter(|v| v.folder == key).cloned());
                }
            }
            // Durées déjà connues reprises (même fichier).
            let known: HashMap<&str, &Video> = old.iter().map(|v| (v.path.as_str(), v)).collect();
            for v in &mut found {
                if let Some(o) = known.get(v.path.as_str()).filter(|o| o.size == v.size && o.modified == v.modified) {
                    v.duration = o.duration;
                }
            }
            found.sort_by(|a, b| b.modified.cmp(&a.modified));
            found.dedup_by(|a, b| a.path == b.path);
            if found != old {
                write_list(&found);
            }
            let _ = tx.send(Event::Scanned(found.clone()));
            waker.wake();
            if complete {
                prune(&found);
            }
            let mut durations = Vec::new();
            for v in &found {
                let file = thumbs_dir().join(format!("{}.png", v.thumb()));
                if file.exists() && v.duration > 0.0 {
                    continue;
                }
                wait(&hold);
                if !file.exists() {
                    let dur = probe(&v.path);
                    if dur > 0.0 {
                        durations.push((v.path.clone(), dur));
                    }
                    make_thumb(&v.path, dur, &file);
                    let _ = tx.send(Event::Thumb(v.thumb()));
                    waker.wake();
                } else {
                    let dur = probe(&v.path);
                    if dur > 0.0 {
                        durations.push((v.path.clone(), dur));
                    }
                }
            }
            if !durations.is_empty() {
                let map: HashMap<String, f64> = durations.iter().cloned().collect();
                let updated: Vec<Video> = found.into_iter().map(|mut v| {
                    if let Some(d) = map.get(&v.path) {
                        v.duration = *d;
                    }
                    v
                }).collect();
                write_list(&updated);
                let _ = tx.send(Event::Durations(durations));
                waker.wake();
            }
            busy.store(false, Ordering::Release);
        });
    }

    /// Nouvelles du fil de relecture. Vrai si la vue doit être redessinée.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(e) = self.rx.try_recv() {
            changed = true;
            match e {
                Event::Scanned(list) => {
                    self.scanning = false;
                    if list != self.videos {
                        self.videos = list;
                    }
                }
                Event::Thumb(id) => {
                    self.images.remove(&id);
                }
                Event::Durations(d) => {
                    let map: HashMap<String, f64> = d.into_iter().collect();
                    for v in &mut self.videos {
                        if let Some(x) = map.get(&v.path) {
                            v.duration = *x;
                        }
                    }
                }
            }
        }
        if self.again && !self.busy.load(Ordering::Acquire) {
            self.again = false;
            self.refresh();
        }
        changed
    }

    /// Miniature d'une vidéo, lue du cache disque à la première demande.
    pub fn image(&mut self, v: &Video) -> Option<Arc<egui::ColorImage>> {
        let id = v.thumb();
        if let Some(t) = self.images.get(&id) {
            return t.clone();
        }
        let file = thumbs_dir().join(format!("{id}.png"));
        if !file.exists() {
            // Pas encore faite : relue quand le fil l'annoncera.
            return None;
        }
        // Mémoire bornée : les vignettes lues gardent au plus ~25 Mo.
        if self.images.len() > 110 {
            self.images.clear();
        }
        let img = read_png(&file).map(Arc::new);
        self.images.insert(id, img.clone());
        img
    }

    /// Libère les miniatures (lecture en cours : la mémoire va au film).
    pub fn forget_images(&mut self) {
        self.images.clear();
    }
}

fn wait(hold: &AtomicBool) {
    while hold.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn scan(dir: &Path, folder: &str, rel: &str, depth: u32, out: &mut Vec<Video>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = e.path();
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if meta.is_dir() {
            if depth < 8 {
                let sub = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                scan(&path, folder, &sub, depth + 1, out);
            }
        } else if path.extension().is_some_and(|x| VIDEO_EXT.contains(&x.to_string_lossy().to_ascii_lowercase().as_str())) {
            let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
            out.push(Video { path: path.to_string_lossy().into_owned(), name, size: meta.len(), modified, folder: folder.to_owned(), duration: 0.0, dir: rel.to_owned() });
        }
    }
}

fn prune(list: &[Video]) {
    let keep: HashSet<String> = list.iter().map(|v| format!("{}.png", v.thumb())).collect();
    if let Ok(rd) = std::fs::read_dir(thumbs_dir()) {
        for e in rd.flatten() {
            if !keep.contains(&e.file_name().to_string_lossy().into_owned()) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Durée en secondes (ffprobe), 0 si inconnue.
fn probe(path: &str) -> f64 {
    Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "csv=p=0", path])
        .stderr(Stdio::null())
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0.0)
}

/// Une image représentative : vers 15 % du film, plus loin si elle est presque noire.
/// Illisible : un fichier vide, pour ne pas réessayer à chaque lancement.
fn make_thumb(path: &str, dur: f64, out: &Path) {
    let _ = std::fs::create_dir_all(thumbs_dir());
    let tmp = out.with_extension("tmp.png");
    let mut best: Option<(u32, u32, Vec<u8>)> = None;
    for f in [0.15, 0.32, 0.5] {
        let at = format!("{:.2}", dur * f);
        let ok = Command::new("nice")
            .args(["-n", "19", "ffmpeg", "-v", "error", "-y", "-ss", &at, "-i", path, "-frames:v", "1", "-an", "-sn"])
            .args(["-vf", &format!("scale={THUMB_W}:-2"), "-f", "image2", "-c:v", "png"])
            .arg(&tmp)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        let Some((w, h, px)) = ok.then(|| decode_rgb(&tmp)).flatten() else { continue };
        let lit = luma(&px) > 28;
        best = Some((w, h, px));
        if lit || dur <= 0.0 {
            break;
        }
    }
    let _ = std::fs::remove_file(&tmp);
    match best {
        Some((w, h, px)) => {
            let (h2, px) = trim_bars(w, h, px);
            let tmp = out.with_extension("part");
            if encode_png(&tmp, w, h2, &px) {
                let _ = std::fs::rename(&tmp, out);
            }
        }
        None => write(out, ""),
    }
}

fn luma(px: &[u8]) -> u32 {
    let n = px.len() / 3;
    if n == 0 {
        return 0;
    }
    let sum: u64 = px.chunks_exact(3).step_by(7).map(|p| (p[0] as u64 * 54 + p[1] as u64 * 183 + p[2] as u64 * 19) >> 8).sum();
    (sum / (n as u64 / 7).max(1)) as u32
}

/// Ôte les bandes noires en haut et en bas : la vignette montre l'image, pas le cadre.
fn trim_bars(w: u32, h: u32, px: Vec<u8>) -> (u32, Vec<u8>) {
    let row = |y: u32| &px[(y * w * 3) as usize..((y + 1) * w * 3) as usize];
    let dark = |y: u32| row(y).chunks_exact(3).step_by(4).all(|p| (p[0] as u32 * 54 + p[1] as u32 * 183 + p[2] as u32 * 19) >> 8 <= 30);
    let mut top = 0;
    while top < h / 3 && dark(top) {
        top += 1;
    }
    let mut bottom = h;
    while bottom > h * 2 / 3 && dark(bottom - 1) {
        bottom -= 1;
    }
    let out = px[(top * w * 3) as usize..(bottom * w * 3) as usize].to_vec();
    (bottom - top, out)
}

fn decode_rgb(file: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let mut d = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(file).ok()?));
    d.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = d.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()?];
    let info = r.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let px = match info.color_type {
        png::ColorType::Rgb => buf,
        png::ColorType::Rgba => buf.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0]]).collect(),
        _ => return None,
    };
    Some((info.width, info.height, px))
}

fn encode_png(file: &Path, w: u32, h: u32, px: &[u8]) -> bool {
    let Ok(f) = std::fs::File::create(file) else { return false };
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    e.set_color(png::ColorType::Rgb);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().and_then(|mut wr| wr.write_image_data(px)).is_ok()
}

fn read_png(file: &Path) -> Option<egui::ColorImage> {
    let (w, h, px) = decode_rgb(file)?;
    Some(egui::ColorImage::from_rgb([w as usize, h as usize], &px))
}

// --- Fichiers -----------------------------------------------------------------------------------

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".local/state")).join("bioplay")
}

fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".cache")).join("bioplay")
}

fn folders_file() -> PathBuf {
    state_dir().join("folders")
}

fn view_file() -> PathBuf {
    state_dir().join("view")
}

fn thumbs_dir() -> PathBuf {
    cache_dir().join("thumbs")
}

fn list_file() -> PathBuf {
    cache_dir().join("library.tsv")
}

fn write(f: &Path, body: &str) {
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = f.with_extension("tmp");
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(tmp, f);
    }
}

fn read_list() -> Vec<Video> {
    std::fs::read_to_string(list_file())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() >= 6).then(|| Video {
                path: f[0].into(),
                name: f[1].into(),
                size: f[2].parse().unwrap_or(0),
                modified: f[3].parse().unwrap_or(0),
                folder: f[4].into(),
                duration: f[5].parse().unwrap_or(0.0),
                dir: f.get(6).map(|d| d.to_string()).unwrap_or_default(),
            })
        })
        .collect()
}

fn write_list(list: &[Video]) {
    let body: String = list
        .iter()
        .map(|v| format!("{}\t{}\t{}\t{}\t{}\t{:.1}\t{}\n", v.path.replace('\t', " "), v.name.replace('\t', " "), v.size, v.modified, v.folder.replace('\t', " "), v.duration, v.dir.replace('\t', " ")))
        .collect();
    write(&list_file(), &body);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_are_trimmed() {
        let (w, h) = (8u32, 10u32);
        let mut px = vec![0u8; (w * h * 3) as usize];
        for y in 2..8 {
            for x in 0..w {
                let i = ((y * w + x) * 3) as usize;
                px[i..i + 3].copy_from_slice(&[200, 180, 160]);
            }
        }
        let (h2, out) = trim_bars(w, h, px);
        assert_eq!(h2, 6);
        assert_eq!(out.len(), (w * 6 * 3) as usize);
    }

    #[test]
    fn tree() {
        let v = |name: &str, folder: &str, dir: &str| Video { path: format!("{folder}/{dir}/{name}"), name: name.into(), size: 1, modified: 0, folder: folder.into(), duration: 0.0, dir: dir.into() };
        let videos = vec![
            v("Film.mkv", "/a", ""),
            v("E10.mkv", "/a", "Série/S1"),
            v("E2.mkv", "/a", "Série/S1"),
            v("E1.mkv", "/a", "Série/S2"),
            v("Doc.mkv", "/a", "Docs"),
        ];
        let folders = vec![PathBuf::from("/a"), PathBuf::from("/vide")];
        let t = Tree { folders: &folders, videos: &videos };
        // Un seul dossier surveillé avec des vidéos : on est directement dedans.
        let top = t.shelf(&Place::default());
        assert_eq!(top.dirs.iter().map(|d| (d.name.as_str(), d.videos, d.dirs)).collect::<Vec<_>>(), [("Docs", 1, 0), ("Série", 3, 2)]);
        assert_eq!(top.videos.len(), 1);
        assert_eq!(t.up(&top.place), None);
        let serie = t.shelf(&top.dirs[1].place);
        assert_eq!(serie.dirs.len(), 2);
        let s1 = t.shelf(&serie.dirs[0].place);
        assert_eq!(s1.videos.iter().map(|&i| videos[i].name.as_str()).collect::<Vec<_>>(), ["E2.mkv", "E10.mkv"]);
        assert_eq!(t.up(&s1.place), Some(serie.place.clone()));
        assert_eq!(t.up(&serie.place), Some(top.place.clone()));
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["Episode 10", "episode 2", "Episode 1", "Bonus"];
        v.sort_by(|a, b| natural(a, b));
        assert_eq!(v, ["Bonus", "Episode 1", "episode 2", "Episode 10"]);
    }

    /// Miniature tirée d'une vraie vidéo (`samples/test.mkv`, si présente).
    #[test]
    fn thumb_from_sample() {
        let f = concat!(env!("CARGO_MANIFEST_DIR"), "/../samples/test.mkv");
        if !Path::new(f).exists() {
            return;
        }
        let out = std::env::temp_dir().join("bioplay-thumb-test.png");
        let dur = probe(f);
        assert!(dur > 0.0);
        make_thumb(f, dur, &out);
        let (w, h, _) = decode_rgb(&out).expect("png");
        assert_eq!(w, THUMB_W);
        assert!(h > 50 && h <= 240, "{h}");
        let _ = std::fs::remove_file(out);
    }
}
