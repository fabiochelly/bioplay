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

pub struct Overlay {
    textures: HashMap<TextureId, Texture>,
    buffers: [Vec<u8>; BUFFERS],
    next: usize,
    shown: bool,
    /// Fond de l'accueil, calculé une fois par taille d'écran (BGRA).
    backdrop: Option<(usize, usize, Vec<u8>)>,
}

impl Overlay {
    pub fn new() -> Result<Self, String> {
        Ok(Self { textures: HashMap::new(), buffers: Default::default(), next: 0, shown: false, backdrop: None })
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
        backdrop: bool,
    ) {
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
            self.hide(mpv);
            return;
        }
        let x0 = ((bb.min.x * ppp).floor() as i32 - 1).clamp(0, w);
        let y0 = ((bb.min.y * ppp).floor() as i32 - 1).clamp(0, h);
        let x1 = ((bb.max.x * ppp).ceil() as i32 + 1).clamp(0, w);
        let y1 = ((bb.max.y * ppp).ceil() as i32 + 1).clamp(0, h);
        let (bw, bh) = ((x1 - x0) as usize, (y1 - y0) as usize);
        if bw == 0 || bh == 0 {
            return;
        }
        let stride = bw * 4;
        let mut buf = std::mem::take(&mut self.buffers[self.next]);
        buf.clear();
        if backdrop {
            buf.extend_from_slice(self.backdrop(w as usize, h as usize));
        } else {
            buf.resize(stride * bh, 0);
        }
        let mut canvas = Canvas { buf: &mut buf, w: bw, h: bh, ox: x0 as f32, oy: y0 as f32 };
        for p in prims {
            let Primitive::Mesh(m) = &p.primitive else { continue };
            let Some(tex) = self.textures.get(&m.texture_id) else { continue };
            let clip = [
                ((p.clip_rect.min.x * ppp).round() as i32 - x0).clamp(0, bw as i32),
                ((p.clip_rect.min.y * ppp).round() as i32 - y0).clamp(0, bh as i32),
                ((p.clip_rect.max.x * ppp).round() as i32 - x0).clamp(0, bw as i32),
                ((p.clip_rect.max.y * ppp).round() as i32 - y0).clamp(0, bh as i32),
            ];
            if clip[0] >= clip[2] || clip[1] >= clip[3] {
                continue;
            }
            for t in m.indices.chunks_exact(3) {
                let v = [&m.vertices[t[0] as usize], &m.vertices[t[1] as usize], &m.vertices[t[2] as usize]];
                canvas.triangle(v, tex, ppp, clip);
            }
        }
        let addr = format!("&{}", buf.as_ptr() as usize);
        mpv.command_sync(&["overlay-add", "0", &x0.to_string(), &y0.to_string(), &addr, "0", "bgra", &bw.to_string(), &bh.to_string(), &stride.to_string()]);
        self.buffers[self.next] = buf;
        self.next = (self.next + 1) % BUFFERS;
        self.shown = true;
    }

    /// Retire la superposition et rend la mémoire des tampons.
    pub fn hide(&mut self, mpv: &Mpv) {
        if self.shown {
            mpv.command_sync(&["overlay-remove", "0"]);
            self.shown = false;
            for b in &mut self.buffers {
                *b = Vec::new();
            }
            self.backdrop = None;
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
