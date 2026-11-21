//! Drapeaux des langues courantes, peints en formes simples puis pris dans une dalle de verre.
//! Même jeu de drapeaux et même détection que l'app Android (ui/Flags.kt).

use egui::{pos2, vec2, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind};

const CODES: &[(&str, &str)] = &[
    ("fre", "fr"), ("fra", "fr"), ("eng", "en"), ("ger", "de"), ("deu", "de"), ("spa", "es"), ("ita", "it"),
    ("por", "pt"), ("jpn", "ja"), ("chi", "zh"), ("zho", "zh"), ("kor", "ko"), ("rus", "ru"), ("dut", "nl"),
    ("nld", "nl"), ("pol", "pl"), ("swe", "sv"), ("tur", "tr"),
];
pub const FLAGS: &[&str] = &["fr", "en", "de", "es", "it", "pt", "ja", "zh", "ko", "ru", "nl", "pl", "sv", "tr"];
const NAMES: &[(&str, &[&str])] = &[
    ("fr", &["français", "francais", "french", "vff", "vfq", "vf"]),
    ("en", &["english", "anglais"]),
    ("de", &["deutsch", "german", "allemand"]),
    ("es", &["español", "espanol", "spanish", "espagnol", "castellano"]),
    ("it", &["italiano", "italian", "italien"]),
    ("pt", &["português", "portugues", "portuguese", "portugais"]),
    ("ja", &["日本語", "japanese", "japonais"]),
    ("zh", &["中文", "chinese", "chinois", "mandarin"]),
    ("ko", &["한국어", "korean", "coréen"]),
    ("ru", &["русский", "russian", "russe"]),
    ("nl", &["nederlands", "dutch", "néerlandais"]),
    ("pl", &["polski", "polish", "polonais"]),
    ("sv", &["svenska", "swedish", "suédois"]),
    ("tr", &["türkçe", "turkish", "turc"]),
];

/// Drapeau d'une piste : d'après son code de langue, sinon d'après un mot de son titre.
pub fn flag_for(lang: &str, title: &str) -> Option<&'static str> {
    let l = lang.to_lowercase();
    let l = l.split('-').next().unwrap_or("");
    let code = CODES.iter().find(|(k, _)| *k == l).map(|(_, v)| *v).unwrap_or(l);
    if let Some(f) = FLAGS.iter().find(|f| **f == code) {
        return Some(f);
    }
    let t = title.to_lowercase();
    let words: Vec<&str> = t.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    NAMES.iter().find(|(_, names)| names.iter().any(|n| words.contains(n) || (!n.is_ascii() && t.contains(n)))).map(|(c, _)| *c)
}

fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

const WHITE: (u8, u8, u8) = (244, 245, 247);

/// Étoile à cinq branches, en triangles autour du centre (forme non convexe).
fn star(p: &Painter, c: Pos2, r: f32, col: Color32, rot: f32) {
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

fn bands(p: &Painter, r: Rect, vertical: bool, list: &[(f32, Color32)]) {
    let mut t = 0.0;
    for (f, c) in list {
        let band = if vertical {
            Rect::from_min_size(pos2(r.min.x + t * r.width(), r.min.y), vec2(f * r.width() + 0.5, r.height()))
        } else {
            Rect::from_min_size(pos2(r.min.x, r.min.y + t * r.height()), vec2(r.width(), f * r.height() + 0.5))
        };
        p.rect_filled(band, CornerRadius::ZERO, *c);
        t += f;
    }
}

fn paint(p: &Painter, r: Rect, code: &str) {
    let (w, h) = (r.width(), r.height());
    let at = |x: f32, y: f32| pos2(r.min.x + x * w, r.min.y + y * h);
    let white = rgb(WHITE.0, WHITE.1, WHITE.2);
    let yellow = rgb(245, 194, 60);
    match code {
        "fr" => bands(p, r, true, &[(1.0 / 3.0, rgb(47, 93, 181)), (1.0 / 3.0, white), (1.0 / 3.0, rgb(224, 71, 63))]),
        "it" => bands(p, r, true, &[(1.0 / 3.0, rgb(46, 154, 90)), (1.0 / 3.0, white), (1.0 / 3.0, rgb(224, 71, 63))]),
        "de" => bands(p, r, false, &[(1.0 / 3.0, rgb(30, 30, 34)), (1.0 / 3.0, rgb(217, 58, 51)), (1.0 / 3.0, yellow)]),
        "es" => bands(p, r, false, &[(0.25, rgb(200, 53, 46)), (0.5, yellow), (0.25, rgb(200, 53, 46))]),
        "ru" => bands(p, r, false, &[(1.0 / 3.0, white), (1.0 / 3.0, rgb(43, 87, 176)), (1.0 / 3.0, rgb(224, 71, 63))]),
        "nl" => bands(p, r, false, &[(1.0 / 3.0, rgb(200, 53, 46)), (1.0 / 3.0, white), (1.0 / 3.0, rgb(42, 76, 154))]),
        "pl" => bands(p, r, false, &[(0.5, white), (0.5, rgb(220, 53, 72))]),
        "pt" => {
            bands(p, r, true, &[(0.4, rgb(46, 139, 78)), (0.6, rgb(224, 71, 63))]);
            p.circle_filled(at(0.4, 0.5), h * 0.22, yellow);
            p.circle_filled(at(0.4, 0.5), h * 0.13, rgb(224, 71, 63));
        }
        "ja" => {
            p.rect_filled(r, CornerRadius::ZERO, white);
            p.circle_filled(r.center(), h * 0.3, rgb(200, 53, 46));
        }
        "zh" => {
            p.rect_filled(r, CornerRadius::ZERO, rgb(215, 56, 46));
            star(p, at(0.17, 0.28), h * 0.17, yellow, 0.0);
            for (x, y) in [(0.33, 0.1), (0.4, 0.22), (0.4, 0.38), (0.33, 0.5)] {
                star(p, at(x, y), h * 0.06, yellow, 0.0);
            }
        }
        "ko" => {
            p.rect_filled(r, CornerRadius::ZERO, white);
            let c = r.center();
            let rr = h * 0.25;
            // Disque coupé en deux : rouge en haut, bleu en bas.
            for (top, col) in [(true, rgb(205, 58, 63)), (false, rgb(43, 91, 168))] {
                let mut pts = vec![c];
                for i in 0..=16 {
                    let a = std::f32::consts::PI * (if top { 1.0 } else { 0.0 } + i as f32 / 16.0);
                    pts.push(pos2(c.x + rr * a.cos(), c.y + rr * a.sin()));
                }
                p.add(Shape::convex_polygon(pts, col, Stroke::NONE));
            }
            let dark = rgb(38, 38, 43);
            for (x, y) in [(0.16, 0.22), (0.84, 0.22), (0.16, 0.78), (0.84, 0.78)] {
                for k in -1..=1 {
                    p.rect_filled(Rect::from_center_size(at(x, y + k as f32 * 0.07), vec2(w * 0.12, h * 0.05)), CornerRadius::ZERO, dark);
                }
            }
        }
        "sv" => {
            p.rect_filled(r, CornerRadius::ZERO, rgb(45, 108, 181));
            p.rect_filled(Rect::from_min_size(at(0.3, 0.0), vec2(w * 0.13, h)), CornerRadius::ZERO, yellow);
            p.rect_filled(Rect::from_min_size(at(0.0, 0.4), vec2(w, h * 0.2)), CornerRadius::ZERO, yellow);
        }
        "tr" => {
            let red = rgb(215, 56, 46);
            p.rect_filled(r, CornerRadius::ZERO, red);
            p.circle_filled(at(0.38, 0.5), h * 0.27, white);
            p.circle_filled(at(0.43, 0.5), h * 0.22, red);
            star(p, at(0.62, 0.5), h * 0.13, white, -0.3);
        }
        _ => {
            // Royaume-Uni : sautoirs puis croix, proportions officielles ramenées à la hauteur.
            let (navy, red) = (rgb(35, 63, 140), rgb(210, 58, 72));
            p.rect_filled(r, CornerRadius::ZERO, navy);
            let (tl, br, bl, tr) = (r.left_top() - vec2(1.0, 1.0), r.right_bottom() + vec2(1.0, 1.0), r.left_bottom() + vec2(-1.0, 1.0), r.right_top() + vec2(1.0, -1.0));
            p.line_segment([tl, br], Stroke::new(h * 0.2, white));
            p.line_segment([bl, tr], Stroke::new(h * 0.2, white));
            p.line_segment([tl, br], Stroke::new(h * 0.067, red));
            p.line_segment([bl, tr], Stroke::new(h * 0.067, red));
            p.rect_filled(Rect::from_center_size(r.center(), vec2(h * 0.333, h)), CornerRadius::ZERO, white);
            p.rect_filled(Rect::from_center_size(r.center(), vec2(w, h * 0.333)), CornerRadius::ZERO, white);
            p.rect_filled(Rect::from_center_size(r.center(), vec2(h * 0.2, h)), CornerRadius::ZERO, red);
            p.rect_filled(Rect::from_center_size(r.center(), vec2(w, h * 0.2)), CornerRadius::ZERO, red);
        }
    }
}

/// Drapeau dans une dalle de verre. Peint opaque (les bandes superposées ne se voient pas à
/// travers) ; une langue inactive est estompée par un voile uniforme posé par-dessus.
pub fn draw(painter: &Painter, r: Rect, code: &str, alpha: f32) {
    let p = painter.with_clip_rect(r);
    paint(&p, r, code);
    if alpha < 1.0 {
        p.rect_filled(r, CornerRadius::ZERO, Color32::from_rgba_premultiplied(14, 16, 22, ((1.0 - alpha) * 230.0) as u8));
    }
    let h = r.height();
    let mut mesh = egui::Mesh::default();
    let top = Color32::from_white_alpha(107);
    let mid = Color32::from_white_alpha(15);
    let shade = Color32::from_black_alpha(56);
    let ym = r.min.y + h * 0.5;
    for (pt, col) in [(r.left_top(), top), (r.right_top(), top), (pos2(r.max.x, ym), mid), (pos2(r.min.x, ym), mid), (r.right_bottom(), shade), (r.left_bottom(), shade)] {
        mesh.colored_vertex(pt, col);
    }
    for t in [[0, 1, 2], [0, 2, 3], [3, 2, 4], [3, 4, 5]] {
        mesh.add_triangle(t[0], t[1], t[2]);
    }
    p.add(Shape::mesh(mesh));
    painter.rect_stroke(r, CornerRadius::same(3), Stroke::new(1.0, Color32::from_white_alpha((128.0 * alpha) as u8)), StrokeKind::Inside);
}
