//! Drapeaux des langues, décrits une seule fois dans `shared/flags.txt` (formes simples) et
//! dessinés ici dans une dalle de verre. Même fichier et même détection que l'app Android.

use std::sync::OnceLock;

use egui::{pos2, vec2, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind};

const SOURCE: &str = include_str!("../../shared/flags.txt");

enum Op {
    Bands(bool, Vec<(f32, Color32)>),
    Rect([f32; 4], Color32),
    Circle([f32; 3], Color32),
    Ring([f32; 4], Color32),
    Star([f32; 3], f32, Color32),
    Line([f32; 5], Color32),
    Poly(Vec<[f32; 2]>, Color32),
}

pub struct Flag {
    pub key: String,
    codes: Vec<String>,
    names: Vec<String>,
    ops: Vec<Op>,
}

fn color(s: &str) -> Color32 {
    let h = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
    Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

fn parse() -> Vec<Flag> {
    let mut out: Vec<Flag> = Vec::new();
    for line in SOURCE.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if let Some(head) = line.strip_prefix('@') {
            let (left, names) = head.split_once('|').unwrap_or((head, ""));
            let mut it = left.split_whitespace();
            let key = it.next().unwrap_or("").to_owned();
            let codes = it.next().unwrap_or("").split(',').map(|c| c.trim().to_lowercase()).filter(|c| !c.is_empty()).collect();
            let names = names.split(',').map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty()).collect();
            out.push(Flag { key, codes, names, ops: Vec::new() });
            continue;
        }
        let Some(flag) = out.last_mut() else { continue };
        let mut parts = line.split_whitespace();
        let kind = parts.next().unwrap_or("");
        let args: Vec<&str> = parts.collect();
        let nums: Vec<f32> = args.iter().filter(|a| !a.starts_with('#')).filter_map(|a| a.parse().ok()).collect();
        let col = args.iter().find(|a| a.starts_with('#')).map(|c| color(c)).unwrap_or(Color32::WHITE);
        let op = match kind {
            "h" | "v" => {
                let bands: Vec<(f32, Color32)> = args.iter().filter_map(|a| a.split_once(':')).map(|(w, c)| (w.parse().unwrap_or(1.0), color(c))).collect();
                let total: f32 = bands.iter().map(|b| b.0).sum();
                Op::Bands(kind == "v", bands.into_iter().map(|(w, c)| (w / total, c)).collect())
            }
            "r" if nums.len() >= 4 => Op::Rect([nums[0], nums[1], nums[2], nums[3]], col),
            "c" if nums.len() >= 3 => Op::Circle([nums[0], nums[1], nums[2]], col),
            "o" if nums.len() >= 4 => Op::Ring([nums[0], nums[1], nums[2], nums[3]], col),
            // Étoile : la rotation éventuelle suit la couleur.
            "s" if nums.len() >= 3 => Op::Star([nums[0], nums[1], nums[2]], args.last().and_then(|a| a.parse().ok()).filter(|_| !args.last().unwrap().starts_with('#')).unwrap_or(0.0), col),
            "l" if nums.len() >= 5 => Op::Line([nums[0], nums[1], nums[2], nums[3], nums[4]], col),
            "p" if nums.len() >= 6 => Op::Poly(nums.chunks_exact(2).map(|c| [c[0], c[1]]).collect(), col),
            _ => continue,
        };
        flag.ops.push(op);
    }
    out
}

pub fn all() -> &'static [Flag] {
    static FLAGS: OnceLock<Vec<Flag>> = OnceLock::new();
    FLAGS.get_or_init(parse)
}

/// Drapeau d'une piste : d'après son code de langue (régional d'abord, « pt-br »), sinon
/// d'après un mot de son titre (« Français », « Brasileiro »…).
pub fn flag_for(lang: &str, title: &str) -> Option<&'static str> {
    let l = lang.trim().to_lowercase().replace('_', "-");
    if !l.is_empty() && l != "und" {
        if let Some(f) = all().iter().find(|f| f.codes.contains(&l)) {
            return Some(&f.key);
        }
        let primary = l.split('-').next().unwrap_or("").to_owned();
        if let Some(f) = all().iter().find(|f| f.codes.contains(&primary)) {
            return Some(&f.key);
        }
    }
    let t = title.to_lowercase();
    let words: Vec<&str> = t.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    all()
        .iter()
        .find(|f| f.names.iter().any(|n| if n.contains(' ') || !n.is_ascii() { t.contains(n.as_str()) } else { words.contains(&n.as_str()) }))
        .map(|f| f.key.as_str())
}

/// Étoile à cinq branches, en triangles autour du centre (forme non convexe).
fn star(p: &Painter, c: Pos2, r: f32, rot: f32, col: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(c, col);
    for i in 0..=10 {
        let rr = if i % 2 == 0 { r } else { r * 0.4 };
        let a = rot - std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
        mesh.colored_vertex(pos2(c.x + rr * a.cos(), c.y + rr * a.sin()), col);
    }
    for i in 1..=10u32 {
        mesh.add_triangle(0, i, i + 1);
    }
    p.add(Shape::mesh(mesh));
}

fn paint(p: &Painter, r: Rect, flag: &Flag) {
    let (w, h) = (r.width(), r.height());
    let at = |x: f32, y: f32| pos2(r.min.x + x * w, r.min.y + y * h);
    for op in &flag.ops {
        match op {
            Op::Bands(vertical, bands) => {
                let mut t = 0.0;
                for (f, c) in bands {
                    let band = if *vertical {
                        Rect::from_min_size(at(t, 0.0), vec2(f * w + 0.5, h))
                    } else {
                        Rect::from_min_size(at(0.0, t), vec2(w, f * h + 0.5))
                    };
                    p.rect_filled(band, CornerRadius::ZERO, *c);
                    t += f;
                }
            }
            Op::Rect([x, y, rw, rh], c) => {
                p.rect_filled(Rect::from_min_size(at(*x, *y), vec2(rw * w, rh * h)), CornerRadius::ZERO, *c);
            }
            Op::Circle([x, y, rad], c) => {
                p.circle_filled(at(*x, *y), rad * h, *c);
            }
            Op::Ring([x, y, rad, e], c) => {
                p.circle_stroke(at(*x, *y), rad * h, Stroke::new(e * h, *c));
            }
            Op::Star([x, y, rad], rot, c) => star(p, at(*x, *y), rad * h, *rot, *c),
            Op::Line([x1, y1, x2, y2, e], c) => {
                p.line_segment([at(*x1, *y1), at(*x2, *y2)], Stroke::new(e * h, *c));
            }
            Op::Poly(pts, c) => {
                // Éventail depuis le centre : convient aux formes « étoilées » (feuille, chevron).
                let n = pts.len() as f32;
                let cx = pts.iter().map(|q| q[0]).sum::<f32>() / n;
                let cy = pts.iter().map(|q| q[1]).sum::<f32>() / n;
                let mut mesh = egui::Mesh::default();
                mesh.colored_vertex(at(cx, cy), *c);
                for q in pts {
                    mesh.colored_vertex(at(q[0], q[1]), *c);
                }
                let k = pts.len() as u32;
                for i in 0..k {
                    mesh.add_triangle(0, 1 + i, 1 + (i + 1) % k);
                }
                p.add(Shape::mesh(mesh));
            }
        }
    }
}

/// Drapeau dans une dalle de verre. Peint opaque (les bandes superposées ne se voient pas à
/// travers) ; une langue inactive est estompée par un voile uniforme posé par-dessus. Coins
/// droits : le drapeau ne déborde jamais de son liseré, même tout petit.
pub fn draw(painter: &Painter, r: Rect, key: &str, alpha: f32) {
    let Some(flag) = all().iter().find(|f| f.key == key) else { return };
    let r = Rect::from_min_max(r.min.round(), r.max.round());
    let p = painter.with_clip_rect(r);
    paint(&p, r, flag);
    if alpha < 1.0 {
        p.rect_filled(r, CornerRadius::ZERO, Color32::from_rgba_premultiplied(14, 16, 22, ((1.0 - alpha) * 230.0) as u8));
    }
    // Reflet en haut, ombre en bas.
    let h = r.height();
    let mut mesh = egui::Mesh::default();
    let top = Color32::from_white_alpha(100);
    let mid = Color32::from_white_alpha(12);
    let shade = Color32::from_black_alpha(52);
    let ym = r.min.y + h * 0.5;
    for (pt, col) in [(r.left_top(), top), (r.right_top(), top), (pos2(r.max.x, ym), mid), (pos2(r.min.x, ym), mid), (r.right_bottom(), shade), (r.left_bottom(), shade)] {
        mesh.colored_vertex(pt, col);
    }
    for t in [[0, 1, 2], [0, 2, 3], [3, 2, 4], [3, 4, 5]] {
        mesh.add_triangle(t[0], t[1], t[2]);
    }
    p.add(Shape::mesh(mesh));
    // Liseré fin (un demi-point), lumineux sans être épais.
    painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(0.5, Color32::from_white_alpha((110.0 * alpha) as u8)), StrokeKind::Inside);
}
