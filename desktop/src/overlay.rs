//! L'interface, rendue en Rust pur (sans OpenGL ni pilote graphique) puis confiée à mpv comme
//! superposition (`overlay-add`). egui fournit des triangles ; ils sont remplis ici, sur le CPU,
//! dans le tampon même que lit mpv. Seule la zone dessinée est calculée et transmise, et
//! seulement quand l'interface change : interface masquée, coût nul et tampons libérés.

use std::collections::HashMap;

use egui::epaint::{ImageData, Primitive, TextureId, Vertex};
use egui::{Color32, Rect};

use crate::mpv::Mpv;

/// Trois tampons en rotation : mpv lit la superposition au rendu, on n'écrit jamais dans
/// celui qu'il affiche.
const BUFFERS: usize = 3;

struct Texture {
    w: usize,
    h: usize,
    /// RGBA prémultiplié, un octet par canal (4 octets par pixel).
    px: Vec<[u8; 4]>,
}

enum Drawn {
    /// Rien à afficher.
    Empty,
    /// Image identique à la précédente.
    Same,
    /// Nouvelle image dans le dernier tampon rempli, à cette place.
    New { x: i32, y: i32, w: usize, h: usize },
}

/// Image précédente : sa place, son tampon et l'empreinte de chacun de ses maillages.
struct Last {
    area: [i32; 4],
    buffer: usize,
    backdrop: bool,
    prims: Vec<(u64, [i32; 4])>,
}

pub struct Overlay {
    textures: HashMap<TextureId, Texture>,
    buffers: [Vec<u8>; BUFFERS],
    next: usize,
    shown: bool,
    last: Option<Last>,
    /// Fond de l'accueil, calculé une fois par taille d'écran (BGRA).
    backdrop: Option<(usize, usize, Vec<u8>)>,
    /// Accueil affiché : fond en incrustation 0, miniatures au-dessus, interface en 63.
    home: bool,
    backdrop_shown: bool,
    thumbs: Thumbs,
}

/// Une miniature à poser (points de l'interface), découpée par `clip`.
pub struct Thumb {
    pub key: String,
    pub rect: Rect,
    pub clip: Rect,
    pub radius: f32,
    pub image: std::sync::Arc<egui::ColorImage>,
}

/// Miniatures confiées à mpv, chacune dans sa propre incrustation (1 à 62) : la carte
/// graphique les compose, le défilement ne fait que les déplacer. Mises à l'échelle une fois
/// (coins arrondis compris) et gardées tant qu'elles servent.
#[derive(Default)]
struct Thumbs {
    scaled: HashMap<(String, usize, usize, usize), Vec<u8>>,
    shown: HashMap<usize, ((String, usize, usize, usize), [i32; 5])>,
}

const UI_HOME: &str = "63";
const THUMB_IDS: usize = 62;

impl Overlay {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            textures: HashMap::new(),
            buffers: Default::default(),
            next: 0,
            shown: false,
            last: None,
            backdrop: None,
            home: false,
            backdrop_shown: false,
            thumbs: Thumbs::default(),
        })
    }

    fn update_textures(&mut self, delta: &egui::TexturesDelta) {
        for (id, list) in &delta.set {
          for d in list {
            let ImageData::Color(img) = &d.image;
            let [w, h] = img.size;
            let px: Vec<[u8; 4]> = img.pixels.iter().map(|c| c.to_array()).collect();
            match d.pos {
                Some([x0, y0]) => {
                    if let Some(t) = self.textures.get_mut(id) {
                        for y in 0..h {
                            for x in 0..w {
                                if x0 + x < t.w && y0 + y < t.h {
                                    t.px[(y0 + y) * t.w + x0 + x] = px[y * w + x];
                                }
                            }
                        }
                    }
                }
                None => {
                    self.textures.insert(*id, Texture { w, h, px });
                }
            }
          }
        }
    }

    /// Dessine les primitives et met à jour la superposition de mpv (ou la retire si vide).
    pub fn present(
        &mut self,
        mpv: &Mpv,
        prims: &[egui::ClippedPrimitive],
        textures: &mut egui::TexturesDelta,
        size: [u32; 2],
        ppp: f32,
        home: bool,
    ) {
        if home != self.home {
            // Changement d'écran : l'interface change d'incrustation, le fond part ou arrive.
            self.hide(mpv);
            if !home {
                self.leave_home(mpv);
            }
            self.home = home;
        }
        if home {
            self.show_backdrop(mpv, size);
        }
        let id = self.ui_id();
        match self.render(prims, textures, size, ppp, false) {
            Drawn::Empty => self.hide(mpv),
            Drawn::Same => {}
            Drawn::New { x, y, w, h } => {
                let buf = &self.buffers[self.last.as_ref().map_or(0, |l| l.buffer)];
                let addr = format!("&{}", buf.as_ptr() as usize);
                mpv.command_sync(&["overlay-add", id, &x.to_string(), &y.to_string(), &addr, "0", "bgra", &w.to_string(), &h.to_string(), &(w * 4).to_string()]);
                self.shown = true;
            }
        }
    }

    fn ui_id(&self) -> &'static str {
        if self.home { UI_HOME } else { "0" }
    }

    /// Fond de l'accueil, posé une fois par taille d'écran sous tout le reste.
    fn show_backdrop(&mut self, mpv: &Mpv, size: [u32; 2]) {
        let (w, h) = (size[0].max(1) as usize, size[1].max(1) as usize);
        if self.backdrop_shown && self.backdrop.as_ref().is_some_and(|b| (b.0, b.1) == (w, h)) {
            return;
        }
        let addr = format!("&{}", self.backdrop(w, h).as_ptr() as usize);
        mpv.command_sync(&["overlay-add", "0", "0", "0", &addr, "0", "bgra", &w.to_string(), &h.to_string(), &(w * 4).to_string()]);
        self.backdrop_shown = true;
    }

    /// Sortie de l'accueil : fond et miniatures retirés, leur mémoire rendue.
    fn leave_home(&mut self, mpv: &Mpv) {
        self.show_thumbs(mpv, &[], 1.0);
        self.thumbs.scaled.clear();
        if self.backdrop_shown {
            mpv.command_sync(&["overlay-remove", "0"]);
            self.backdrop_shown = false;
        }
        self.backdrop = None;
    }

    /// Pose les miniatures visibles ; une incrustation n'est renvoyée à mpv que si elle a bougé.
    pub fn show_thumbs(&mut self, mpv: &Mpv, list: &[Thumb], ppp: f32) {
        let mut used = std::collections::HashSet::new();
        let mut next_shown = HashMap::new();
        for (i, t) in list.iter().take(THUMB_IDS).enumerate() {
            let id = i + 1;
            let x = (t.rect.min.x * ppp).round() as i32;
            let y = (t.rect.min.y * ppp).round() as i32;
            let w = ((t.rect.max.x * ppp).round() as i32 - x).max(1) as usize;
            let h = ((t.rect.max.y * ppp).round() as i32 - y).max(1) as usize;
            let r = (t.radius * ppp).round() as usize;
            let key = (t.key.clone(), w, h, r);
            let cx0 = x.max((t.clip.min.x * ppp).round() as i32);
            let cy0 = y.max((t.clip.min.y * ppp).round() as i32);
            let cx1 = (x + w as i32).min((t.clip.max.x * ppp).round() as i32);
            let cy1 = (y + h as i32).min((t.clip.max.y * ppp).round() as i32);
            if cx0 >= cx1 || cy0 >= cy1 {
                continue;
            }
            let buf = self.thumbs.scaled.entry(key.clone()).or_insert_with(|| scale_rounded(&t.image, w, h, r as f32));
            let offset = ((cy0 - y) as usize * w + (cx0 - x) as usize) * 4;
            let params = [cx0, cy0, cx1 - cx0, cy1 - cy0, offset as i32];
            used.insert(key.clone());
            if self.thumbs.shown.get(&id) != Some(&(key.clone(), params)) {
                let addr = format!("&{}", buf.as_ptr() as usize + offset);
                mpv.command_sync(&[
                    "overlay-add", &id.to_string(), &cx0.to_string(), &cy0.to_string(), &addr, "0", "bgra",
                    &(cx1 - cx0).to_string(), &(cy1 - cy0).to_string(), &(w * 4).to_string(),
                ]);
            }
            next_shown.insert(id, (key, params));
        }
        for id in self.thumbs.shown.keys() {
            if !next_shown.contains_key(id) {
                mpv.command_sync(&["overlay-remove", &id.to_string()]);
            }
        }
        self.thumbs.shown = next_shown;
        // Images mises à l'échelle gardées pour un aller-retour de défilement, pas au-delà.
        if self.thumbs.scaled.len() > 160 {
            self.thumbs.scaled.retain(|k, _| used.contains(k));
        }
    }

    /// Remplit le prochain tampon ; ne touche pas à mpv.
    fn render(&mut self, prims: &[egui::ClippedPrimitive], textures: &mut egui::TexturesDelta, size: [u32; 2], ppp: f32, backdrop: bool) -> Drawn {
        self.update_textures(textures);
        let (w, h) = (size[0].max(1) as i32, size[1].max(1) as i32);
        // Zone dessinée : union des maillages, bornée par leurs rectangles de découpe.
        let mut bb = Rect::NOTHING;
        for p in prims {
            if let Primitive::Mesh(m) = &p.primitive {
                let mut r = Rect::NOTHING;
                for v in &m.vertices {
                    r.extend_with(v.pos);
                }
                bb = bb.union(r.intersect(p.clip_rect));
            }
        }
        if backdrop {
            bb = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32 / ppp, h as f32 / ppp));
        }
        for id in &textures.free {
            self.textures.remove(id);
        }
        if !bb.is_positive() {
            return Drawn::Empty;
        }
        let x0 = ((bb.min.x * ppp).floor() as i32 - 1).clamp(0, w);
        let y0 = ((bb.min.y * ppp).floor() as i32 - 1).clamp(0, h);
        let x1 = ((bb.max.x * ppp).ceil() as i32 + 1).clamp(0, w);
        let y1 = ((bb.max.y * ppp).ceil() as i32 + 1).clamp(0, h);
        let (bw, bh) = ((x1 - x0) as usize, (y1 - y0) as usize);
        if bw == 0 || bh == 0 {
            return Drawn::Same;
        }
        let stride = bw * 4;
        let area = [x0, y0, bw as i32, bh as i32];

        // Empreinte et emprise (pixels du tampon) de chaque maillage.
        let prims_info: Vec<(u64, [i32; 4])> = prims
            .iter()
            .map(|p| match &p.primitive {
                Primitive::Mesh(m) => {
                    let mut r = Rect::NOTHING;
                    for v in &m.vertices {
                        r.extend_with(v.pos);
                    }
                    let r = r.intersect(p.clip_rect);
                    let px = [
                        ((r.min.x * ppp).floor() as i32 - 1 - x0).clamp(0, bw as i32),
                        ((r.min.y * ppp).floor() as i32 - 1 - y0).clamp(0, bh as i32),
                        ((r.max.x * ppp).ceil() as i32 + 1 - x0).clamp(0, bw as i32),
                        ((r.max.y * ppp).ceil() as i32 + 1 - y0).clamp(0, bh as i32),
                    ];
                    (mesh_hash(p.clip_rect, m), px)
                }
                _ => (0, [0; 4]),
            })
            .collect();

        // Seule la zone qui a changé est redessinée (un survol ne touche qu'un bouton) ; rien
        // n'a changé : rien à faire. Toute l'image si la taille, le fond ou les textures changent.
        let textures_changed = !textures.set.is_empty() || !textures.free.is_empty();
        let damage = match &self.last {
            Some(l) if l.area == area && l.backdrop == backdrop && !textures_changed => {
                let mut before: HashMap<u64, Vec<[i32; 4]>> = HashMap::new();
                for (h, r) in &l.prims {
                    before.entry(*h).or_default().push(*r);
                }
                let mut d = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
                let mut add = |r: [i32; 4]| {
                    if r[0] < r[2] && r[1] < r[3] {
                        d = [d[0].min(r[0]), d[1].min(r[1]), d[2].max(r[2]), d[3].max(r[3])];
                    }
                };
                for (h, r) in &prims_info {
                    match before.get_mut(h).and_then(|v| v.pop()) {
                        Some(_) => {}
                        None => add(*r),
                    }
                }
                for rs in before.values() {
                    for r in rs {
                        add(*r);
                    }
                }
                if d[0] >= d[2] {
                    return Drawn::Same;
                }
                Some((l.buffer, d))
            }
            _ => None,
        };

        let mut buf = std::mem::take(&mut self.buffers[self.next]);
        buf.clear();
        let full = [0, 0, bw as i32, bh as i32];
        let zone = match damage {
            Some((prev, d)) => {
                // Copie de l'image précédente, zone abîmée remise à neuf (fond ou transparent).
                buf.extend_from_slice(&self.buffers[prev]);
                let row = (d[2] - d[0]) as usize * 4;
                for y in d[1]..d[3] {
                    let o = y as usize * stride + d[0] as usize * 4;
                    if backdrop {
                        let src = &self.backdrop(w as usize, h as usize)[o..o + row];
                        buf[o..o + row].copy_from_slice(src);
                    } else {
                        buf[o..o + row].fill(0);
                    }
                }
                d
            }
            None => {
                if backdrop {
                    buf.extend_from_slice(self.backdrop(w as usize, h as usize));
                } else {
                    buf.resize(stride * bh, 0);
                }
                full
            }
        };
        let mut canvas = Canvas { buf: &mut buf, w: bw, h: bh, ox: x0 as f32, oy: y0 as f32 };
        for (p, (_, r)) in prims.iter().zip(&prims_info) {
            let Primitive::Mesh(m) = &p.primitive else { continue };
            if r[2] <= zone[0] || r[0] >= zone[2] || r[3] <= zone[1] || r[1] >= zone[3] {
                continue;
            }
            let Some(tex) = self.textures.get(&m.texture_id) else { continue };
            let clip = [
                ((p.clip_rect.min.x * ppp).round() as i32 - x0).clamp(zone[0], zone[2]),
                ((p.clip_rect.min.y * ppp).round() as i32 - y0).clamp(zone[1], zone[3]),
                ((p.clip_rect.max.x * ppp).round() as i32 - x0).clamp(zone[0], zone[2]),
                ((p.clip_rect.max.y * ppp).round() as i32 - y0).clamp(zone[1], zone[3]),
            ];
            if clip[0] >= clip[2] || clip[1] >= clip[3] {
                continue;
            }
            for t in m.indices.chunks_exact(3) {
                let v = [&m.vertices[t[0] as usize], &m.vertices[t[1] as usize], &m.vertices[t[2] as usize]];
                canvas.triangle(v, tex, ppp, clip);
            }
        }
        self.last = Some(Last { area, buffer: self.next, backdrop, prims: prims_info });
        self.buffers[self.next] = buf;
        self.next = (self.next + 1) % BUFFERS;
        Drawn::New { x: x0, y: y0, w: bw, h: bh }
    }

    /// Retire la superposition et rend la mémoire des tampons.
    pub fn hide(&mut self, mpv: &Mpv) {
        if self.shown {
            mpv.command_sync(&["overlay-remove", self.ui_id()]);
            self.shown = false;
            self.last = None;
            for b in &mut self.buffers {
                *b = Vec::new();
            }
        }
    }

    /// Fond de l'accueil : trois lumières en dégradés radiaux, avec un tramage léger qui supprime
    /// les marches des dégradés sombres.
    fn backdrop(&mut self, w: usize, h: usize) -> &[u8] {
        if self.backdrop.as_ref().map(|b| (b.0, b.1)) != Some((w, h)) {
            let mut px = vec![0u8; w * h * 4];
            let aspect = w as f32 / h as f32;
            let glows = [
                (0.18f32, 0.25f32, 0.75f32, [0.16f32, 0.30, 0.75], 0.55f32),
                (0.85, 0.80, 0.85, [0.50, 0.22, 0.85], 0.45),
                (0.60, 0.08, 0.50, [0.10, 0.70, 0.65], 0.25),
            ];
            for y in 0..h {
                let v = y as f32 / h as f32;
                for x in 0..w {
                    let u = x as f32 / w as f32;
                    let mut c = [0.020f32, 0.027, 0.047];
                    for (cx, cy, r, col, k) in glows {
                        let dx = (u - cx) * aspect;
                        let dy = v - cy;
                        let e = (-(dx * dx + dy * dy) / (r * r) * 2.2).exp() * k;
                        for i in 0..3 {
                            c[i] += col[i] * e;
                        }
                    }
                    // Tramage : bruit de ±0,75 niveau, déterministe.
                    let n = ((x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77)).wrapping_mul(0xC2B2_AE3D) >> 24;
                    let d = (n as f32 / 255.0 - 0.5) * 1.5;
                    let o = (y * w + x) * 4;
                    px[o] = (c[2] * 255.0 + d).clamp(0.0, 255.0) as u8;
                    px[o + 1] = (c[1] * 255.0 + d).clamp(0.0, 255.0) as u8;
                    px[o + 2] = (c[0] * 255.0 + d).clamp(0.0, 255.0) as u8;
                    px[o + 3] = 255;
                }
            }
            self.backdrop = Some((w, h, px));
        }
        &self.backdrop.as_ref().unwrap().2
    }
}

/// Zone de la superposition en cours de dessin (BGRA prémultiplié).
/// Empreinte d'un maillage : sommets, indices, texture et découpe.
fn mesh_hash(clip: Rect, m: &egui::Mesh) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |x: u64| h = (h ^ x).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(5);
    for f in [clip.min.x, clip.min.y, clip.max.x, clip.max.y] {
        mix(f.to_bits() as u64);
    }
    mix(match m.texture_id {
        TextureId::Managed(n) => n,
        TextureId::User(n) => n ^ (1 << 63),
    });
    for v in &m.vertices {
        mix((v.pos.x.to_bits() as u64) << 32 | v.pos.y.to_bits() as u64);
        mix((v.uv.x.to_bits() as u64) << 32 | v.uv.y.to_bits() as u64);
        mix(u32::from_ne_bytes(v.color.to_array()) as u64);
    }
    for i in &m.indices {
        mix(*i as u64);
    }
    h
}

struct Canvas<'a> {
    buf: &'a mut Vec<u8>,
    w: usize,
    h: usize,
    ox: f32,
    oy: f32,
}

fn rgba(c: Color32) -> [f32; 4] {
    [c.r() as f32, c.g() as f32, c.b() as f32, c.a() as f32]
}

impl Canvas<'_> {
    /// Remplit un triangle : couleurs interpolées, texture (glyphes) échantillonnée en bilinéaire,
    /// mélange « over » prémultiplié. Règle haut-gauche : une arête partagée par deux triangles
    /// n'est dessinée qu'une fois (pas de liseré sur les aplats semi-transparents).
    fn triangle(&mut self, v: [&Vertex; 3], tex: &Texture, ppp: f32, clip: [i32; 4]) {
        let p: [(f32, f32); 3] = std::array::from_fn(|i| (v[i].pos.x * ppp - self.ox, v[i].pos.y * ppp - self.oy));
        let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[1].1 - p[0].1) * (p[2].0 - p[0].0);
        if area.abs() < 1e-6 {
            return;
        }
        // Orientation uniforme : sommets dans le sens où l'aire est positive.
        let (p, v) = if area < 0.0 { ([p[0], p[2], p[1]], [v[0], v[2], v[1]]) } else { (p, v) };
        let area = area.abs();
        let col: [[f32; 4]; 3] = std::array::from_fn(|i| rgba(v[i].color));
        let textured = v.iter().any(|x| x.uv != egui::epaint::WHITE_UV);
        let uv: [(f32, f32); 3] = std::array::from_fn(|i| (v[i].uv.x * tex.w as f32 - 0.5, v[i].uv.y * tex.h as f32 - 0.5));

        let minx = p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor().max(clip[0] as f32) as i32;
        let maxx = p.iter().map(|q| q.0).fold(f32::MIN, f32::max).ceil().min(clip[2] as f32) as i32;
        let miny = p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor().max(clip[1] as f32) as i32;
        let maxy = p.iter().map(|q| q.1).fold(f32::MIN, f32::max).ceil().min(clip[3] as f32) as i32;
        if minx >= maxx || miny >= maxy {
            return;
        }
        // Fonctions d'arête : e_i(x, y) = a_i x + b_i y + c_i, positives à l'intérieur.
        let edge = |a: (f32, f32), b: (f32, f32)| {
            let (ea, eb) = (a.1 - b.1, b.0 - a.0);
            let ec = a.0 * b.1 - a.1 * b.0;
            // Arête « haut » ou « gauche » : incluse quand le centre du pixel tombe pile dessus.
            let top_left = (ea == 0.0 && eb < 0.0) || ea > 0.0;
            (ea, eb, ec, if top_left { 0.0 } else { f32::MIN_POSITIVE })
        };
        let e = [edge(p[1], p[2]), edge(p[2], p[0]), edge(p[0], p[1])];
        let inv = 1.0 / area;
        for y in miny..maxy {
            let py = y as f32 + 0.5;
            let row = y as usize * self.w * 4;
            for x in minx..maxx {
                let px = x as f32 + 0.5;
                let w0 = e[0].0 * px + e[0].1 * py + e[0].2;
                let w1 = e[1].0 * px + e[1].1 * py + e[1].2;
                let w2 = e[2].0 * px + e[2].1 * py + e[2].2;
                if w0 < e[0].3 || w1 < e[1].3 || w2 < e[2].3 {
                    continue;
                }
                let (b0, b1, b2) = (w0 * inv, w1 * inv, w2 * inv);
                let mut s = [0f32; 4];
                for k in 0..4 {
                    s[k] = col[0][k] * b0 + col[1][k] * b1 + col[2][k] * b2;
                }
                if textured {
                    let t = sample(tex, uv[0].0 * b0 + uv[1].0 * b1 + uv[2].0 * b2, uv[0].1 * b0 + uv[1].1 * b1 + uv[2].1 * b2);
                    for k in 0..4 {
                        s[k] = s[k] * t[k] / 255.0;
                    }
                }
                if s[3] <= 0.0 {
                    continue;
                }
                let o = row + x as usize * 4;
                let d = &mut self.buf[o..o + 4];
                let k = 1.0 - s[3] / 255.0;
                // Tampon BGRA : s est en RGBA.
                d[0] = (s[2] + d[0] as f32 * k).min(255.0) as u8;
                d[1] = (s[1] + d[1] as f32 * k).min(255.0) as u8;
                d[2] = (s[0] + d[2] as f32 * k).min(255.0) as u8;
                d[3] = (s[3] + d[3] as f32 * k).min(255.0) as u8;
            }
        }
        let _ = self.h;
    }
}

/// Miniature mise à la taille voulue (bilinéaire), coins arrondis adoucis, en BGRA prémultiplié.
fn scale_rounded(img: &egui::ColorImage, w: usize, h: usize, radius: f32) -> Vec<u8> {
    let [iw, ih] = img.size;
    let mut out = vec![0u8; w * h * 4];
    if iw == 0 || ih == 0 {
        return out;
    }
    // Recadrage au rapport de la vignette (centre gardé).
    let (ta, ra) = (iw as f32 / ih as f32, w as f32 / h as f32);
    let (sw, sh) = if ta > ra { (ih as f32 * ra, ih as f32) } else { (iw as f32, iw as f32 / ra) };
    let (sx0, sy0) = ((iw as f32 - sw) / 2.0, (ih as f32 - sh) / 2.0);
    let px = |x: usize, y: usize| img.pixels[y.min(ih - 1) * iw + x.min(iw - 1)];
    for y in 0..h {
        let fy = (sy0 + (y as f32 + 0.5) * sh / h as f32 - 0.5).max(0.0);
        let (y0, ty) = (fy as usize, fy.fract());
        for x in 0..w {
            let fx = (sx0 + (x as f32 + 0.5) * sw / w as f32 - 0.5).max(0.0);
            let (x0, tx) = (fx as usize, fx.fract());
            let (a, b, c, d) = (px(x0, y0), px(x0 + 1, y0), px(x0, y0 + 1), px(x0 + 1, y0 + 1));
            let mix = |k: usize| {
                let top = a[k] as f32 + (b[k] as f32 - a[k] as f32) * tx;
                let bot = c[k] as f32 + (d[k] as f32 - c[k] as f32) * tx;
                top + (bot - top) * ty
            };
            // Couverture du coin arrondi (distance au centre de l'arc, bord adouci d'un pixel).
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            let dx = (radius - cx).max(cx - (w as f32 - radius)).max(0.0);
            let dy = (radius - cy).max(cy - (h as f32 - radius)).max(0.0);
            let cover = if dx > 0.0 && dy > 0.0 { (radius - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0) } else { 1.0 };
            let o = (y * w + x) * 4;
            out[o] = (mix(2) * cover) as u8;
            out[o + 1] = (mix(1) * cover) as u8;
            out[o + 2] = (mix(0) * cover) as u8;
            out[o + 3] = (255.0 * cover) as u8;
        }
    }
    out
}

/// Lecture bilinéaire, bords répétés.
fn sample(t: &Texture, x: f32, y: f32) -> [f32; 4] {
    let (xf, yf) = (x.floor(), y.floor());
    let (fx, fy) = (x - xf, y - yf);
    let cx = |i: i32| i.clamp(0, t.w as i32 - 1) as usize;
    let cy = |i: i32| i.clamp(0, t.h as i32 - 1) as usize;
    let (x0, y0) = (xf as i32, yf as i32);
    let px = |x: i32, y: i32| t.px[cy(y) * t.w + cx(x)];
    let (p00, p10, p01, p11) = (px(x0, y0), px(x0 + 1, y0), px(x0, y0 + 1), px(x0 + 1, y0 + 1));
    std::array::from_fn(|k| {
        let a = p00[k] as f32 + (p10[k] as f32 - p00[k] as f32) * fx;
        let b = p01[k] as f32 + (p11[k] as f32 - p01[k] as f32) * fx;
        a + (b - a) * fy
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Une image d'interface : un grand panneau, du texte, et une rangée de boutons dont
    /// `lit` est éclairé (comme un survol).
    fn frame(ctx: &egui::Context, lit: usize) -> (Vec<egui::ClippedPrimitive>, egui::TexturesDelta) {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(640.0, 400.0))),
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            let p = ui.painter();
            p.rect_filled(Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(600.0, 360.0)), 24.0, Color32::from_black_alpha(140));
            p.text(egui::pos2(40.0, 40.0), egui::Align2::LEFT_TOP, "Décalage audio 0 ms", egui::FontId::proportional(16.0), Color32::WHITE);
            for i in 0..6 {
                let c = egui::pos2(80.0 + i as f32 * 90.0, 300.0);
                let a = if i == lit { 90 } else { 20 };
                p.circle_filled(c, 20.0, Color32::from_white_alpha(a));
                p.circle_stroke(c, 20.0, egui::Stroke::new(1.0, Color32::from_white_alpha(40)));
            }
        });
        (ctx.tessellate(out.shapes, out.pixels_per_point), out.textures_delta)
    }

    #[test]
    fn partial_redraw_matches_full() {
        let ctx = egui::Context::default();
        crate::ui::fonts(&ctx);
        let ppp = 1.5;
        ctx.set_pixels_per_point(ppp);
        let size = [960, 600];
        let mut partial = Overlay::new().unwrap();
        let mut full = Overlay::new().unwrap();
        // Quelques images pour que les polices et la mise en page se stabilisent.
        for _ in 0..3 {
            let (p, mut t) = frame(&ctx, 0);
            partial.render(&p, &mut t, size, ppp, false);
            full.render(&p, &mut t, size, ppp, false);
        }
        for lit in [1, 4, 4, 2] {
            let (p, mut t) = frame(&ctx, lit);
            assert!(t.set.is_empty(), "les glyphes ne doivent pas changer");
            let a = partial.render(&p, &mut t, size, ppp, false);
            full.last = None;
            full.render(&p, &mut t, size, ppp, false);
            let pa = &partial.buffers[partial.last.as_ref().unwrap().buffer];
            let fa = &full.buffers[full.last.as_ref().unwrap().buffer];
            if !matches!(a, Drawn::Same) {
                assert_eq!(pa, fa, "rendu partiel différent du rendu complet (bouton {lit})");
            }
        }
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// Coût d'une image complète de l'accueil en miniatures (hors images, posées par mpv) (plein écran 2560×1440, 15 vignettes).
    /// `cargo test --release -- --ignored --nocapture grid_cost`
    #[test]
    #[ignore]
    fn grid_cost() {
        let ctx = egui::Context::default();
        crate::ui::fonts(&ctx);
        let ppp = 1.6;
        ctx.set_pixels_per_point(ppp);
        let img = egui::ColorImage::from_rgb([320, 180], &vec![120u8; 320 * 180 * 3]);
        let tex = ctx.load_texture("t", img, egui::TextureOptions::LINEAR);
        let mut o = Overlay::new().unwrap();
        let size = [2560, 1440];
        let mut times = Vec::new();
        for k in 0..8 {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0))), ..Default::default() };
            let out = ctx.run_ui(raw, |ui| {
                let p = ui.painter();
                for i in 0..15 {
                    let r = Rect::from_min_size(egui::pos2(28.0 + (i % 5) as f32 * 310.0, 90.0 + (i / 5) as f32 * 250.0 + k as f32 * 7.0), egui::vec2(294.0, 165.0));
                    // Les images sont posées par mpv : ici le liseré, le rail d'avancée et la durée.
                    let _ = &tex;
                    p.rect_stroke(r, 12.0, egui::Stroke::new(1.0, Color32::from_white_alpha(34)), egui::StrokeKind::Inside);
                    let rail = Rect::from_min_size(egui::pos2(r.min.x + 9.0, r.max.y - 11.0), egui::vec2(r.width() - 18.0, 4.0));
                    p.rect_filled(rail, 2.0, Color32::from_white_alpha(52));
                    p.text(r.right_top() + egui::vec2(-10.0, 9.0), egui::Align2::RIGHT_TOP, "1:42:10", egui::FontId::proportional(11.0), Color32::WHITE);
                    p.text(r.left_bottom() + egui::vec2(2.0, 8.0), egui::Align2::LEFT_TOP, "Un titre de film assez long", egui::FontId::proportional(13.5), Color32::WHITE);
                }
            });
            let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
            let mut t = out.textures_delta;
            let s = std::time::Instant::now();
            o.render(&prims, &mut t, size, ppp, false);
            times.push(s.elapsed().as_secs_f64() * 1000.0);
        }
        eprintln!("rendu par image (ms) : {times:.1?}");
    }
}
