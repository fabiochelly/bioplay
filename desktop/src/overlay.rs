//! L'interface, rendue en Rust pur (sans OpenGL ni pilote graphique) puis confiée à mpv comme
//! superposition (`overlay-add`). egui fournit des triangles ; ils sont remplis ici, sur le CPU,
//! dans le tampon même que lit mpv. Seule la zone dessinée est calculée et transmise, et
//! seulement quand l'interface change : interface masquée, coût nul et tampons libérés.

use std::collections::HashMap;

use egui::epaint::{ImageData, Primitive, TextureId, Vertex};
use egui::{Color32, Rect};

use crate::mpv::{Mpv, OVERLAY_TAG};

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
    /// Numéro d'envoi de chaque tampon : mpv le lit en traitant la commande, plus tard ; on n'y
    /// touche pas (ni réécriture ni réallocation) avant sa réponse.
    sent: [u64; BUFFERS],
    seq: u64,
    /// Images envoyées à mpv (mesures).
    pub sends: u64,
    next: usize,
    shown: bool,
    last: Option<Last>,
    /// Fond de l'accueil, calculé une fois par taille (pleine et demi-résolution), BGRA.
    backdrop: Vec<(usize, usize, Vec<u8>)>,
    /// Miniatures de l'accueil mises à la taille d'affichage (coins arrondis compris), BGRA
    /// prémultiplié : posées dans l'image par simple copie de lignes, gardées tant qu'elles servent.
    scaled: HashMap<(String, usize, usize, usize), Vec<u8>>,
}

/// Une miniature à poser (points de l'interface), découpée par `clip`.
pub struct Thumb {
    pub key: String,
    pub rect: Rect,
    pub clip: Rect,
    pub radius: f32,
    pub image: std::sync::Arc<egui::ColorImage>,
}

impl Overlay {
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            textures: HashMap::new(),
            buffers: Default::default(),
            sent: [0; BUFFERS],
            seq: 0,
            sends: 0,
            next: 0,
            shown: false,
            last: None,
            backdrop: Vec::new(),
            scaled: HashMap::new(),
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
    /// Une seule incrustation, fond et miniatures compris : mpv réassemble et renvoie toutes ses
    /// incrustations à la carte graphique à chaque changement, une seule commande par image.
    pub fn present(
        &mut self,
        mpv: &Mpv,
        prims: &[egui::ClippedPrimitive],
        marks: &[(u64, Rect)],
        textures: &mut egui::TexturesDelta,
        size: [u32; 2],
        ppp: f32,
        home: bool,
        thumbs: &[Thumb],
        reduce: u32,
    ) -> bool {
        if !home && !self.scaled.is_empty() {
            self.scaled = HashMap::new();
        }
        // En mouvement (défilement), image dessinée en résolution réduite (`reduce` = 2 : quatre
        // fois moins de pixels à dessiner et à envoyer) et agrandie par mpv sur la carte graphique ;
        // l'image nette suit dès l'arrêt. `prims` sont déjà découpés à cette échelle.
        let r = reduce.max(1);
        let rsize = [size[0].div_ceil(r), size[1].div_ceil(r)];
        self.release(mpv);
        if self.sent[self.next] > mpv.overlay_done.get() {
            // mpv n'a pas encore lu ce tampon (cœur occupé : saut, ouverture…) : on attend.
            self.update_textures(textures);
            return false;
        }
        let t0 = std::time::Instant::now();
        let drawn = self.render(prims, Some(marks), textures, rsize, ppp / r as f32, home, thumbs);
        let t1 = std::time::Instant::now();
        static DBG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let dbg = *DBG.get_or_init(|| std::env::var_os("BIOPLAY_DEBUG_DAMAGE").is_some());
        match drawn {
            Drawn::Empty => self.hide(mpv),
            Drawn::Same => {}
            Drawn::New { x, y, w, h } => {
                let i = self.last.as_ref().map_or(0, |l| l.buffer);
                let addr = format!("&{}", self.buffers[i].as_ptr() as usize);
                let (dx, dy, dw, dh) = (x * r as i32, y * r as i32, w * r as usize, h * r as usize);
                // Sans attendre : mpv copie l'image en traitant la commande ; le tampon reste
                // intact jusqu'à sa réponse (voir `sent`).
                self.seq += 1;
                self.sends += 1;
                self.sent[i] = self.seq;
                mpv.command_reply(OVERLAY_TAG + self.seq, &[
                    "overlay-add", "0", &dx.to_string(), &dy.to_string(), &addr, "0", "bgra",
                    &w.to_string(), &h.to_string(), &(w * 4).to_string(), &dw.to_string(), &dh.to_string(),
                ]);
                self.shown = true;
                if dbg {
                    eprintln!("rendu {:.1} ms ({w}x{h}), envoi {:.1} ms", (t1 - t0).as_secs_f64() * 1e3, t1.elapsed().as_secs_f64() * 1e3);
                }
            }
        }
        true
    }

    /// Remplit le prochain tampon ; ne touche pas à mpv.
    fn render(&mut self, prims: &[egui::ClippedPrimitive], marks: Option<&[(u64, Rect)]>, textures: &mut egui::TexturesDelta, size: [u32; 2], ppp: f32, backdrop: bool, thumbs: &[Thumb]) -> Drawn {
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

        // Miniatures : place (pixels du tampon), découpe, image à la bonne taille.
        let mut used = std::collections::HashSet::new();
        let mut placed: Vec<([i32; 4], [i32; 4], (String, usize, usize, usize))> = Vec::new();
        for t in thumbs {
            let tx = (t.rect.min.x * ppp).round() as i32 - x0;
            let ty = (t.rect.min.y * ppp).round() as i32 - y0;
            let tw = ((t.rect.max.x * ppp).round() as i32 - x0 - tx).max(1) as usize;
            let th = ((t.rect.max.y * ppp).round() as i32 - y0 - ty).max(1) as usize;
            let c = [
                tx.max((t.clip.min.x * ppp).round() as i32 - x0).max(0),
                ty.max((t.clip.min.y * ppp).round() as i32 - y0).max(0),
                (tx + tw as i32).min((t.clip.max.x * ppp).round() as i32 - x0).min(bw as i32),
                (ty + th as i32).min((t.clip.max.y * ppp).round() as i32 - y0).min(bh as i32),
            ];
            if c[0] >= c[2] || c[1] >= c[3] {
                continue;
            }
            let key = (t.key.clone(), tw, th, (t.radius * ppp).round() as usize);
            used.insert(key.clone());
            placed.push(([tx, ty, tw as i32, th as i32], c, key));
        }
        // Nouvelles miniatures (une rangée qui apparaît) : mises à l'échelle en parallèle.
        let missing: Vec<(&(String, usize, usize, usize), &Thumb)> = placed
            .iter()
            .filter(|(_, _, k)| !self.scaled.contains_key(k))
            .filter_map(|(_, _, k)| thumbs.iter().find(|t| t.key == k.0).map(|t| (k, t)))
            .collect();
        let made: Vec<Vec<u8>> = std::thread::scope(|sc| {
            let hs: Vec<_> = missing.iter().map(|(k, t)| sc.spawn(move || scale_rounded(&t.image, k.1, k.2, k.3 as f32))).collect();
            hs.into_iter().map(|h| h.join().unwrap_or_default()).collect()
        });
        for ((k, _), px) in missing.iter().zip(made) {
            self.scaled.insert((*k).clone(), px);
        }
        if self.scaled.len() > 160 {
            self.scaled.retain(|k, _| used.contains(k));
        }
        // Ce qui a changé, élément par élément (texte, plaque, icône) quand les empreintes des
        // formes sont fournies : egui fond toute une zone en un seul maillage, si bien qu'une
        // seconde de l'horloge ferait sinon redessiner toute la barre.
        let to_px = |r: Rect| {
            [
                ((r.min.x * ppp).floor() as i32 - 2 - x0).clamp(0, bw as i32),
                ((r.min.y * ppp).floor() as i32 - 2 - y0).clamp(0, bh as i32),
                ((r.max.x * ppp).ceil() as i32 + 2 - x0).clamp(0, bw as i32),
                ((r.max.y * ppp).ceil() as i32 + 2 - y0).clamp(0, bh as i32),
            ]
        };
        let mut all_info = match marks {
            Some(m) => m.iter().map(|(h, r)| (*h, to_px(*r))).collect(),
            None => prims_info.clone(),
        };
        for (r, c, k) in &placed {
            let mut hsh = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(&(k, r, c), &mut hsh);
            all_info.push((std::hash::Hasher::finish(&hsh), *c));
        }

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
                for (h, r) in &all_info {
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

        let full = [0, 0, bw as i32, bh as i32];
        // Zone presque entière (défilement) : tout refaire coûte moins que recopier puis refaire.
        let (zone, prev) = match damage {
            Some((prev, d)) if ((d[2] - d[0]) as usize * (d[3] - d[1]) as usize) < bw * bh * 3 / 5 => (d, Some(prev)),
            _ => (full, None),
        };
        if backdrop {
            self.backdrop(w as usize, h as usize);
        }
        let mut buf = std::mem::take(&mut self.buffers[self.next]);
        buf.resize(stride * bh, 0);
        let prev_buf: Option<&[u8]> = prev.map(|i| self.buffers[i].as_slice());
        let back: Option<&[u8]> = if backdrop { self.backdrop.iter().find(|b| (b.0, b.1) == (w as usize, h as usize)).map(|b| b.2.as_slice()) } else { None };
        let (textures, scaled) = (&self.textures, &self.scaled);
        // Dessin par bandes horizontales, une par cœur : chaque bande fait tout pour ses lignes.
        // Petite zone (une seconde de l'horloge, un bouton survolé) : peu de fils, car chaque fil
        // reparcourt les triangles et coûte du processeur pour rien (≈ un fil par 200 000 pixels).
        let zone_px = (zone[2] - zone[0]).max(0) as usize * (zone[3] - zone[1]).max(0) as usize;
        let threads = (zone_px / 200_000 + 1).min(std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8));
        let band_rows = bh.div_ceil(threads).max(16);
        std::thread::scope(|sc| {
            for (bi, band) in buf.chunks_mut(stride * band_rows).enumerate() {
                let b0 = (bi * band_rows) as i32;
                let rows = (band.len() / stride) as i32;
                let (placed, prims, prims_info) = (&placed, prims, &prims_info);
                sc.spawn(move || {
                    // Lignes de la zone dans cette bande.
                    let zy0 = zone[1].max(b0);
                    let zy1 = zone[3].min(b0 + rows);
                    if let Some(pb) = prev_buf {
                        band.copy_from_slice(&pb[b0 as usize * stride..b0 as usize * stride + band.len()]);
                    }
                    if zy0 >= zy1 {
                        return;
                    }
                    let row = (zone[2] - zone[0]) as usize * 4;
                    for y in zy0..zy1 {
                        let o = (y - b0) as usize * stride + zone[0] as usize * 4;
                        match back {
                            Some(bd) => {
                                let g = y as usize * stride + zone[0] as usize * 4;
                                band[o..o + row].copy_from_slice(&bd[g..g + row]);
                            }
                            None => band[o..o + row].fill(0),
                        }
                    }
                    // Miniatures (sous l'interface) : lignes opaques copiées, coins mélangés.
                    for (r, c, k) in placed {
                        let z = [c[0].max(zone[0]), c[1].max(zy0), c[2].min(zone[2]), c[3].min(zy1)];
                        if z[0] >= z[2] || z[1] >= z[3] {
                            continue;
                        }
                        let src = &scaled[k];
                        let sw = r[2] as usize;
                        let radius = k.3 as i32;
                        let n = (z[2] - z[0]) as usize * 4;
                        for y in z[1]..z[3] {
                            let so = ((y - r[1]) as usize * sw + (z[0] - r[0]) as usize) * 4;
                            let d0 = (y - b0) as usize * stride + z[0] as usize * 4;
                            let (s_row, d_row) = (&src[so..so + n], &mut band[d0..d0 + n]);
                            let ly = y - r[1];
                            if ly >= radius && ly < r[3] - radius && z[0] - r[0] >= radius && r[0] + r[2] - z[2] >= radius {
                                d_row.copy_from_slice(s_row);
                                continue;
                            }
                            for (sp, dp) in s_row.chunks_exact(4).zip(d_row.chunks_exact_mut(4)) {
                                match sp[3] {
                                    255 => dp.copy_from_slice(sp),
                                    0 => {}
                                    a => {
                                        let k = 255 - a as u32;
                                        for i in 0..4 {
                                            dp[i] = (sp[i] as u32 + (dp[i] as u32 * k + 127) / 255).min(255) as u8;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let mut canvas = Canvas { buf: band, w: bw, h: rows as usize, ox: x0 as f32, oy: (y0 + b0) as f32 };
                    for (p, (_, r)) in prims.iter().zip(prims_info) {
                        let Primitive::Mesh(m) = &p.primitive else { continue };
                        if r[2] <= zone[0] || r[0] >= zone[2] || r[3] <= zy0 || r[1] >= zy1 {
                            continue;
                        }
                        let Some(tex) = textures.get(&m.texture_id) else { continue };
                        let clip = [
                            ((p.clip_rect.min.x * ppp).round() as i32 - x0).clamp(zone[0], zone[2]),
                            ((p.clip_rect.min.y * ppp).round() as i32 - y0).clamp(zy0, zy1) - b0,
                            ((p.clip_rect.max.x * ppp).round() as i32 - x0).clamp(zone[0], zone[2]),
                            ((p.clip_rect.max.y * ppp).round() as i32 - y0).clamp(zy0, zy1) - b0,
                        ];
                        if clip[0] >= clip[2] || clip[1] >= clip[3] {
                            continue;
                        }
                        for t in m.indices.chunks_exact(3) {
                            let v = [&m.vertices[t[0] as usize], &m.vertices[t[1] as usize], &m.vertices[t[2] as usize]];
                            canvas.triangle(v, tex, ppp, clip);
                        }
                    }
                });
            }
        });
        self.last = Some(Last { area, buffer: self.next, backdrop, prims: all_info });
        self.buffers[self.next] = buf;
        self.next = (self.next + 1) % BUFFERS;
        Drawn::New { x: x0, y: y0, w: bw, h: bh }
    }

    /// Dernière image dessinée, en PNG (mesures et vérifications sans fenêtre).
    pub fn snapshot(&self, path: &std::path::Path) {
        let Some(l) = &self.last else { return };
        let buf = &self.buffers[l.buffer];
        let rgba: Vec<u8> = buf.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        let Ok(f) = std::fs::File::create(path) else { return };
        let mut e = png::Encoder::new(std::io::BufWriter::new(f), l.area[2] as u32, l.area[3] as u32);
        e.set_color(png::ColorType::Rgba);
        let _ = e.write_header().and_then(|mut w| w.write_image_data(&rgba));
    }

    /// Retire la superposition et rend la mémoire des tampons.
    pub fn hide(&mut self, mpv: &Mpv) {
        if self.shown {
            // Retrait sans attendre, dans la même file que les envois. Surtout pas de commande
            // synchrone : mpv l'exécute tout de suite, AVANT les `overlay-add` encore en file, qui
            // liraient ensuite des tampons libérés (plantage). Les tampons sont rendus plus tard,
            // une fois tous les envois traités (`release`).
            self.seq += 1;
            mpv.command_reply(OVERLAY_TAG + self.seq, &["overlay-remove", "0"]);
            self.shown = false;
            self.last = None;
        }
    }

    /// Rend la mémoire des tampons, quand rien n'est affiché et que mpv a tout traité.
    pub fn release(&mut self, mpv: &Mpv) {
        if !self.shown && mpv.overlay_done.get() >= self.seq && self.buffers.iter().any(|b| b.capacity() > 0) {
            for b in &mut self.buffers {
                *b = Vec::new();
            }
        }
    }

    /// Fond de l'accueil : trois lumières en dégradés radiaux, avec un tramage léger qui supprime
    /// les marches des dégradés sombres.
    fn backdrop(&mut self, w: usize, h: usize) -> &[u8] {
        if !self.backdrop.iter().any(|b| (b.0, b.1) == (w, h)) {
            let mut px = vec![0u8; w * h * 4];
            let aspect = w as f32 / h as f32;
            let glows = [
                (0.18f32, 0.25f32, 0.75f32, [0.16f32, 0.30, 0.75], 0.55f32),
                (0.85, 0.80, 0.85, [0.50, 0.22, 0.85], 0.45),
                (0.60, 0.08, 0.50, [0.10, 0.70, 0.65], 0.25),
            ];
            // Calculé par bandes, une par cœur (une fois par taille d'écran).
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16);
            let band = h.div_ceil(threads).max(1);
            std::thread::scope(|sc| {
                for (bi, chunk) in px.chunks_mut(w * 4 * band).enumerate() {
                    sc.spawn(move || {
                        for (ry, line) in chunk.chunks_exact_mut(w * 4).enumerate() {
                            let y = bi * band + ry;
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
                                let o = x * 4;
                                line[o] = (c[2] * 255.0 + d).clamp(0.0, 255.0) as u8;
                                line[o + 1] = (c[1] * 255.0 + d).clamp(0.0, 255.0) as u8;
                                line[o + 2] = (c[0] * 255.0 + d).clamp(0.0, 255.0) as u8;
                                line[o + 3] = 255;
                            }
                        }
                    });
                }
            });
            if self.backdrop.len() >= 2 {
                self.backdrop.remove(0);
            }
            self.backdrop.push((w, h, px));
        }
        &self.backdrop.iter().find(|b| (b.0, b.1) == (w, h)).unwrap().2
    }
}

/// Zone de la superposition en cours de dessin (BGRA prémultiplié).
/// Empreinte et emprise (points) de chaque forme egui, avant leur fusion en maillages : sert à
/// ne redessiner que les éléments qui ont changé. Les textes sont identifiés par leur contenu
/// (`LayoutJob`), pas par l'adresse de leur mise en page (réutilisable par un autre texte).
pub fn shape_marks(shapes: &[egui::epaint::ClippedShape]) -> Vec<(u64, Rect)> {
    use std::hash::{Hash, Hasher};
    use std::io::Write;
    struct W<'a>(&'a mut std::collections::hash_map::DefaultHasher);
    impl Write for W<'_> {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.write(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut out = Vec::with_capacity(shapes.len());
    let mut stack: Vec<(Rect, &egui::Shape)> = shapes.iter().map(|c| (c.clip_rect, &c.shape)).rev().collect();
    while let Some((clip, shape)) = stack.pop() {
        if let egui::Shape::Vec(v) = shape {
            stack.extend(v.iter().rev().map(|s| (clip, s)));
            continue;
        }
        if matches!(shape, egui::Shape::Noop) {
            continue;
        }
        let r = shape.visual_bounding_rect().intersect(clip);
        if !r.is_positive() {
            continue;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for f in [clip.min.x, clip.min.y, clip.max.x, clip.max.y] {
            f.to_bits().hash(&mut h);
        }
        match shape {
            egui::Shape::Text(t) => {
                t.galley.job.hash(&mut h);
                let _ = write!(W(&mut h), "{:?}{:?}{:?}{:?}{:?}{:?}", t.pos, t.underline, t.fallback_color, t.override_text_color, t.opacity_factor, t.angle);
            }
            other => {
                let _ = write!(W(&mut h), "{other:?}");
            }
        }
        out.push((h.finish(), r));
    }
    out
}

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
    buf: &'a mut [u8],
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
        let (p, t, _) = frame_marks(ctx, lit);
        (p, t)
    }

    fn frame_marks(ctx: &egui::Context, lit: usize) -> (Vec<egui::ClippedPrimitive>, egui::TexturesDelta, Vec<(u64, Rect)>) {
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
            // Horloge qui change (glyphes déjà connus grâce à la ligne fixe).
            p.text(egui::pos2(40.0, 80.0), egui::Align2::LEFT_TOP, "0123456789:−", egui::FontId::proportional(13.0), Color32::WHITE);
            p.text(egui::pos2(40.0, 200.0), egui::Align2::LEFT_TOP, format!("1:2{lit}"), egui::FontId::proportional(13.0), Color32::WHITE);
            p.text(egui::pos2(560.0, 200.0), egui::Align2::RIGHT_TOP, format!("−0:3{lit}"), egui::FontId::proportional(13.0), Color32::WHITE);
        });
        let marks = shape_marks(&out.shapes);
        (ctx.tessellate(out.shapes, out.pixels_per_point), out.textures_delta, marks)
    }

    #[test]
    fn partial_redraw_by_shape_matches_full() {
        let ctx = egui::Context::default();
        crate::ui::fonts(&ctx);
        let ppp = 2.0;
        ctx.set_pixels_per_point(ppp);
        let size = [1280, 800];
        let mut partial = Overlay::new().unwrap();
        let mut full = Overlay::new().unwrap();
        // Toutes les variantes une fois : leurs glyphes entrent dans l'atlas.
        for lit in [1, 2, 4, 0, 0] {
            let (p, mut t, m) = frame_marks(&ctx, lit);
            partial.render(&p, Some(&m), &mut t, size, ppp, false, &[]);
            full.render(&p, None, &mut t, size, ppp, false, &[]);
        }
        let mut smaller = 0;
        for lit in [1, 4, 4, 2, 0] {
            let (p, mut t, m) = frame_marks(&ctx, lit);
            assert!(t.set.is_empty(), "les glyphes ne doivent pas changer");
            let a = partial.render(&p, Some(&m), &mut t, size, ppp, false, &[]);
            full.last = None;
            full.render(&p, None, &mut t, size, ppp, false, &[]);
            let pa = &partial.buffers[partial.last.as_ref().unwrap().buffer];
            let fa = &full.buffers[full.last.as_ref().unwrap().buffer];
            // Découpage en bandes différent (nombre de fils selon la zone) : arrondis à ±1 près.
            let worst = pa.iter().zip(fa.iter()).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap_or(0);
            assert!(pa.len() == fa.len() && worst <= 1, "rendu partiel (par forme) différent du rendu complet ({lit}) : écart {worst}");
            if matches!(a, Drawn::New { .. }) {
                smaller += 1;
            }
        }
        assert!(smaller >= 3);
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
        for lit in [1, 2, 4, 0, 0] {
            let (p, mut t) = frame(&ctx, lit);
            partial.render(&p, None, &mut t, size, ppp, false, &[]);
            full.render(&p, None, &mut t, size, ppp, false, &[]);
        }
        for lit in [1, 4, 4, 2] {
            let (p, mut t) = frame(&ctx, lit);
            assert!(t.set.is_empty(), "les glyphes ne doivent pas changer");
            let a = partial.render(&p, None, &mut t, size, ppp, false, &[]);
            full.last = None;
            full.render(&p, None, &mut t, size, ppp, false, &[]);
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

    /// Coût d'une image de l'accueil en défilement, à 2880×1800 (échelle 2) : fond, miniatures
    /// et texte. `cargo test --release -- --ignored --nocapture grid_cost`
    #[test]
    #[ignore]
    fn grid_cost() {
        let ctx = egui::Context::default();
        crate::ui::fonts(&ctx);
        let ppp = 2.0;
        ctx.set_pixels_per_point(ppp);
        let img = std::sync::Arc::new(egui::ColorImage::from_rgb([320, 180], &vec![120u8; 320 * 180 * 3]));
        let mut o = Overlay::new().unwrap();
        let size = [2880, 1800];
        let mut times = Vec::new();
        for k in 0..12 {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0))), ..Default::default() };
            let body = Rect::from_min_max(egui::pos2(0.0, 80.0), egui::pos2(1440.0, 900.0));
            let mut thumbs = Vec::new();
            let out = ctx.run_ui(raw, |ui| {
                let p = ui.painter().with_clip_rect(body);
                for i in 0..20 {
                    let r = Rect::from_min_size(egui::pos2(28.0 + (i % 5) as f32 * 280.0, 60.0 + (i / 5) as f32 * 210.0 - k as f32 * 9.0), egui::vec2(264.0, 148.0));
                    thumbs.push(Thumb { key: format!("t{i}"), rect: r, clip: body, radius: 12.0, image: img.clone() });
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
            o.render(&prims, None, &mut t, size, ppp, true, &thumbs);
            times.push(s.elapsed().as_secs_f64() * 1000.0);
        }
        eprintln!("rendu par image (ms) : {times:.1?}");
    }
}

#[cfg(test)]
mod look {
    use super::*;

    /// Image d'un dossier dessiné, pour vérifier son allure sans ouvrir de fenêtre.
    /// `cargo test --release -- --ignored folder_png` → $TMPDIR/bioplay-folder.png
    #[test]
    #[ignore]
    fn folder_png() {
        let ctx = egui::Context::default();
        crate::ui::fonts(&ctx);
        let ppp = 2.0;
        ctx.set_pixels_per_point(ppp);
        let mut o = Overlay::new().unwrap();
        let mut prims = Vec::new();
        let mut tex = Default::default();
        for _ in 0..2 {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560.0, 180.0))), ..Default::default() };
            let out = ctx.run_ui(raw, |ui| {
                let p = ui.painter();
                p.rect_filled(Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(560.0, 180.0)), 0.0, Color32::from_rgb(10, 14, 26));
                crate::ui::folder_art(p, Rect::from_min_size(egui::pos2(16.0, 16.0), egui::vec2(256.0, 144.0)), false);
                crate::ui::folder_art(p, Rect::from_min_size(egui::pos2(288.0, 16.0), egui::vec2(256.0, 144.0)), true);
            });
            prims = ctx.tessellate(out.shapes, out.pixels_per_point);
            tex = out.textures_delta;
            o.render(&prims, None, &mut tex, [1120, 360], ppp, false, &[]);
        }
        let buf = &o.buffers[o.last.as_ref().unwrap().buffer];
        let l = o.last.as_ref().unwrap();
        let (w, h) = (l.area[2] as u32, l.area[3] as u32);
        let rgba: Vec<u8> = buf.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        let f = std::fs::File::create(std::env::temp_dir().join("bioplay-folder.png")).unwrap();
        let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
        e.set_color(png::ColorType::Rgba);
        e.write_header().unwrap().write_image_data(&rgba).unwrap();
    }
}

#[cfg(test)]
mod scale_bench {
    #[test]
    #[ignore]
    fn scale_cost() {
        let img = egui::ColorImage::from_rgb([320, 180], &vec![120u8; 320 * 180 * 3]);
        let t = std::time::Instant::now();
        for _ in 0..10 {
            std::hint::black_box(super::scale_rounded(&img, 528, 296, 24.0));
        }
        eprintln!("mise à l'échelle : {:.2} ms", t.elapsed().as_secs_f64() * 100.0);
        let t = std::time::Instant::now();
        let mut o = super::Overlay::new().unwrap();
        o.backdrop(2880, 1800);
        eprintln!("fond : {:.0} ms", t.elapsed().as_secs_f64() * 1000.0);
    }
}
