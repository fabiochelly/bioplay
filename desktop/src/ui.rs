//! Interface plein écran : barre de verre en bas, tiroirs au-dessus, messages en haut. egui
//! ne dessine que le texte et les icônes ; les plaques de verre sont rendues par `gfx`.

use std::time::{Duration, Instant};

use egui::{
    pos2, vec2, Align2, Color32, CornerRadius, FontFamily, FontId, Id, Order, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui,
};

use crate::gfx::Glass;
use crate::mpv::{track_detail, track_label};
use crate::player::{clock, speed_label, Panel, Player, Zone};

pub const INK: Color32 = Color32::from_rgb(247, 248, 250);
pub const INK2: Color32 = Color32::from_rgba_premultiplied(181, 182, 184, 184);
pub const FAINT: Color32 = Color32::from_rgba_premultiplied(99, 99, 100, 102);
pub const ACCENT: Color32 = Color32::from_rgb(143, 216, 255);
pub const ACCENT_DEEP: Color32 = Color32::from_rgb(79, 178, 240);

pub fn fonts(ctx: &egui::Context) {
    let mut f = egui::FontDefinitions::default();
    let medium = include_bytes!("../../app/src/main/assets/inter-medium.ttf");
    let semi = include_bytes!("../../app/src/main/assets/inter-semibold.ttf");
    f.font_data.insert("inter".into(), egui::FontData::from_static(medium).into());
    f.font_data.insert("inter-semi".into(), egui::FontData::from_static(semi).into());
    f.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "inter".into());
    f.families.insert(FontFamily::Name("semi".into()), vec!["inter-semi".into(), "inter".into()]);
    ctx.set_fonts(f);
    ctx.all_styles_mut(|s| {
        s.visuals = egui::Visuals::dark();
        s.visuals.window_fill = Color32::TRANSPARENT;
        s.visuals.panel_fill = Color32::TRANSPARENT;
        s.interaction.selectable_labels = false;
        s.spacing.scroll.floating = true;
    });
}

fn semi(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semi".into()))
}

fn med(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub enum Action {
    Open,
    Play(String),
    Forget(String),
}

/// Plaques de verre de l'image en cours, en pixels.
pub struct Plates {
    pub list: Vec<Glass>,
    ppp: f32,
    alpha: f32,
}

impl Plates {
    fn add(&mut self, r: Rect, radius: f32, tint: f32) {
        self.list.push(Glass {
            x: r.min.x * self.ppp,
            y: r.min.y * self.ppp,
            w: r.width() * self.ppp,
            h: r.height() * self.ppp,
            radius: radius * self.ppp,
            tint,
            alpha: self.alpha,
        });
    }
}

pub fn draw(ctx: &egui::Context, p: &mut Player, ppp: f32) -> (Vec<Glass>, Vec<Action>) {
    let mut plates = Plates { list: Vec::new(), ppp, alpha: 1.0 };
    let mut actions = Vec::new();
    let screen = ctx.content_rect();

    if !p.has_file() {
        home(ctx, p, &mut plates, &mut actions, screen);
        return (plates.list, actions);
    }

    let idle = p.activity.elapsed() > Duration::from_millis(2200);
    // Survoler la barre ne la garde pas affichée : seule une activité récente compte.
    let show = !idle || p.mpv.state.paused || p.panel.is_some() || p.scrub.is_some();
    let alpha = ctx.animate_bool_with_time(Id::new("overlay"), show, if show { 0.14 } else { 0.35 });

    surface(ctx, p, screen);

    if alpha > 0.0 {
        plates.alpha = alpha;
        bar(ctx, p, &mut plates, screen, alpha);
        if let Some(panel) = p.panel {
            drawer(ctx, p, &mut plates, screen, panel);
        }
    }
    plates.alpha = 1.0;
    if let Some(s) = &p.scrub {
        scrub_osd(ctx, p, s, &mut plates, screen);
    }
    toast(ctx, p, &mut plates, screen);
    (plates.list, actions)
}

// --- Surface vidéo : glissé par crans, clics transmis à mpv ------------------------------------

fn surface(ctx: &egui::Context, p: &mut Player, screen: Rect) {
    egui::Area::new(Id::new("surface")).order(Order::Background).fixed_pos(screen.min).show(ctx, |ui| {
        let r = ui.allocate_rect(screen, Sense::click_and_drag());
        if r.drag_started() {
            let y = r.interact_pointer_pos().map(|q| (q.y - screen.min.y) / screen.height()).unwrap_or(0.5);
            let zone = if y < 0.36 { Zone::Minute } else if y < 0.64 { Zone::Ten } else { Zone::Line };
            p.scrub = Some(p.preview(zone, 0, p.mpv.state.position));
        }
        if r.dragged() {
            if let (Some(s), Some(origin), Some(now)) = (&p.scrub, ctx.input(|i| i.pointer.press_origin()), r.interact_pointer_pos()) {
                let step_px = match s.zone {
                    Zone::Line => 40.0,
                    Zone::Ten => 26.0,
                    Zone::Minute => 34.0,
                };
                let steps = ((now.x - origin.x) / step_px) as i32;
                if steps != s.steps {
                    let next = p.preview(s.zone, steps, s.from);
                    if steps != 0 && (next.zone != Zone::Line || next.cue.is_some()) {
                        p.mpv.seek(next.target, false);
                    }
                    p.scrub = Some(next);
                }
            }
        }
        if r.drag_stopped() {
            if let Some(s) = p.scrub.take() {
                if s.steps != 0 {
                    if s.zone == Zone::Line && s.cue.is_none() {
                        p.mpv.command(&["sub-seek", &s.steps.to_string()]);
                    } else {
                        p.mpv.seek(s.target, true);
                    }
                } else {
                    p.mpv.seek(s.from, true);
                }
            }
        }
        if r.clicked() {
            if p.panel.take().is_none() {
                p.mpv.command(&["keypress", "MBTN_LEFT"]);
            }
        }
        if r.double_clicked() {
            p.mpv.command(&["keypress", "MBTN_LEFT_DBL"]);
        }
        if r.secondary_clicked() {
            p.mpv.command(&["keypress", "MBTN_RIGHT"]);
        }
        if r.middle_clicked() {
            p.mpv.command(&["keypress", "MBTN_MID"]);
        }
    });
}

// --- Barre du bas ---------------------------------------------------------------------------------

fn bar(ctx: &egui::Context, p: &mut Player, plates: &mut Plates, screen: Rect, alpha: f32) {
    let w = (screen.width() - 48.0).min(1180.0);
    let h = 100.0;
    let rect = Rect::from_min_size(pos2(screen.center().x - w / 2.0, screen.max.y - h - 22.0), vec2(w, h));
    plates.add(rect, 30.0, 0.42);
    let mut over = false;
    egui::Area::new(Id::new("bar")).order(Order::Middle).fixed_pos(rect.min).show(ctx, |ui| {
        ui.set_opacity(alpha);
        let (_, bg) = ui.allocate_exact_size(rect.size(), Sense::hover());
        over = bg.hovered() || ui.rect_contains_pointer(rect);
        let s = &p.mpv.state;
        let inner = rect.shrink2(vec2(22.0, 14.0));

        // Ligne 1 : temps, progression, reste.
        let row1 = Rect::from_min_size(inner.min, vec2(inner.width(), 26.0));
        let painter = ui.painter().clone();
        painter.text(row1.left_center(), Align2::LEFT_CENTER, clock(s.position), semi(13.0), INK2);
        painter.text(row1.right_center(), Align2::RIGHT_CENTER, clock(-(s.duration - s.position)), semi(13.0), INK2);
        let seek_rect = Rect::from_min_max(pos2(row1.min.x + 64.0, row1.min.y), pos2(row1.max.x - 64.0, row1.max.y));
        seekbar(ui, p, seek_rect);

        // Ligne 2 : lignes / lecture à gauche, titre au centre, réglages à droite.
        let row2 = Rect::from_min_max(pos2(inner.min.x - 6.0, inner.min.y + 32.0), pos2(inner.max.x + 6.0, inner.max.y));
        let cy = row2.center().y;
        let mut x = row2.min.x;
        if icon_button(ui, Rect::from_center_size(pos2(x + 20.0, cy), vec2(40.0, 40.0)), Icon::PrevLine).clicked() {
            p.step_sub(-1);
        }
        x += 44.0;
        let paused = p.mpv.state.paused;
        if icon_button(ui, Rect::from_center_size(pos2(x + 24.0, cy), vec2(48.0, 48.0)), if paused { Icon::Play } else { Icon::Pause }).clicked() {
            p.mpv.command(&["cycle", "pause"]);
        }
        x += 52.0;
        if icon_button(ui, Rect::from_center_size(pos2(x + 20.0, cy), vec2(40.0, 40.0)), Icon::NextLine).clicked() {
            p.step_sub(1);
        }

        // À droite, de droite à gauche.
        let mut rx = row2.max.x;
        rx -= 40.0;
        if icon_button(ui, Rect::from_min_size(pos2(rx, cy - 20.0), vec2(40.0, 40.0)), Icon::List).clicked() {
            toggle(p, Panel::List);
        }
        rx -= 44.0;
        let zoomed = p.mpv.state.zoom.abs() > 0.01;
        if icon_button_tint(ui, Rect::from_min_size(pos2(rx, cy - 20.0), vec2(40.0, 40.0)), Icon::Zoom, if zoomed { ACCENT } else { INK }).clicked() {
            toggle(p, Panel::Zoom);
        }
        let sp = speed_label(p.mpv.state.speed);
        let sw = pill_width(ui, &sp, true);
        rx -= sw + 4.0;
        let tint = if (p.mpv.state.speed - 1.0).abs() > 0.001 { ACCENT } else { INK };
        if pill(ui, Rect::from_min_size(pos2(rx, cy - 20.0), vec2(sw, 40.0)), Some(Icon::Speed), &sp, tint).clicked() {
            toggle(p, Panel::Speed);
        }
        let ts = track_summary(p);
        let tw = pill_width(ui, &ts, true);
        rx -= tw + 4.0;
        if pill(ui, Rect::from_min_size(pos2(rx, cy - 20.0), vec2(tw, 40.0)), Some(Icon::Tracks), &ts, INK).clicked() {
            toggle(p, Panel::Tracks);
        }

        // Titre, entre les deux groupes.
        let title_rect = Rect::from_min_max(pos2(x + 56.0, row2.min.y), pos2(rx - 16.0, row2.max.y));
        let title = Player::title(p);
        let galley = painter.layout(title, semi(15.0), INK, f32::INFINITY);
        let clip = painter.with_clip_rect(title_rect);
        let tx = (title_rect.center().x - galley.size().x / 2.0).max(title_rect.min.x);
        clip.galley(pos2(tx, cy - galley.size().y / 2.0), galley, INK);
    });
    p.over_ui = over;
}

impl Player {
    fn title(p: &Player) -> String {
        let s = &p.mpv.state;
        let t = if s.title.is_empty() { &s.path } else { &s.title };
        let name = std::path::Path::new(t).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| t.clone());
        if p.indexing { format!("{name}  ·  lecture des sous-titres…") } else { name }
    }
}

fn toggle(p: &mut Player, panel: Panel) {
    p.panel = if p.panel == Some(panel) { None } else { Some(panel) };
}

fn track_summary(p: &Player) -> String {
    let s = &p.mpv.state;
    let short = |t: Option<&crate::mpv::Track>| {
        t.map(|t| {
            let base = if !t.lang.is_empty() && t.lang != "und" { t.lang.clone() } else { track_label(t) };
            base.chars().take(3).collect::<String>().to_uppercase()
        })
    };
    let audio = short(s.tracks.iter().find(|t| t.kind == "audio" && Some(t.id) == s.aid));
    let sub = short(p.sub_track());
    format!("{} · {}", audio.unwrap_or("—".into()), sub.unwrap_or("Off".into()))
}

fn seekbar(ui: &mut Ui, p: &mut Player, rect: Rect) {
    let r = ui.interact(rect, Id::new("seek"), Sense::click_and_drag());
    let s = &p.mpv.state;
    let dur = s.duration.max(0.001);
    let hover = r.hovered() || r.dragged();
    let frac_at = |x: f32| ((x - rect.min.x) / rect.width()).clamp(0.0, 1.0) as f64;
    if r.dragged() || r.clicked() {
        if let Some(q) = r.interact_pointer_pos() {
            p.mpv.seek(frac_at(q.x) * dur, !r.dragged());
        }
    }
    if r.drag_stopped() {
        if let Some(q) = r.interact_pointer_pos() {
            p.mpv.seek(frac_at(q.x) * dur, true);
        }
    }
    let s = &p.mpv.state;
    let f = (s.position / dur).clamp(0.0, 1.0) as f32;
    let cache = (s.cached / dur).clamp(0.0, 1.0) as f32;
    let h = ui.ctx().animate_bool_with_time(Id::new("seek-h"), hover, 0.12) * 4.0 + 5.0;
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), h));
    let painter = ui.painter();
    let cr = CornerRadius::same((h / 2.0) as u8);
    painter.rect_filled(track, cr, Color32::from_white_alpha(52));
    if cache > f {
        painter.rect_filled(Rect::from_min_size(track.min, vec2(track.width() * cache, h)), cr, Color32::from_white_alpha(40));
    }
    let done = Rect::from_min_size(track.min, vec2(track.width() * f, h));
    gradient(painter, done, ACCENT_DEEP, Color32::WHITE, cr);
    let x = track.min.x + track.width() * f;
    if hover {
        painter.circle_filled(pos2(x, track.center().y), h * 2.2, ACCENT.gamma_multiply(0.25));
    }
    painter.circle_filled(pos2(x, track.center().y), h * 1.15, Color32::WHITE);
    // Survol : l'heure visée et la ligne de sous-titre à ce moment.
    if let Some(q) = r.hover_pos() {
        let t = frac_at(q.x) * dur;
        let mut label = clock(t);
        if let Some(c) = p.cue_at(t) {
            let line = c.text.replace('\n', " ");
            let short: String = line.chars().take(60).collect();
            label = format!("{label}  ·  {short}");
        }
        let g = painter.layout_no_wrap(label, semi(13.0), INK);
        let bubble = Rect::from_center_size(pos2(q.x.clamp(rect.min.x + g.size().x / 2.0, rect.max.x - g.size().x / 2.0), rect.min.y - 22.0), g.size() + vec2(20.0, 10.0));
        painter.rect_filled(bubble, CornerRadius::same(12), Color32::from_black_alpha(190));
        painter.galley(bubble.center() - g.size() / 2.0, g, INK);
    }
}

fn gradient(painter: &egui::Painter, r: Rect, a: Color32, b: Color32, cr: CornerRadius) {
    if r.width() < 1.0 {
        return;
    }
    // Dégradé horizontal : mesh à 4 sommets, coins arrondis par un rectangle plein à gauche.
    painter.rect_filled(Rect::from_min_size(r.min, vec2(r.height().min(r.width()), r.height())), cr, a);
    let inner = Rect::from_min_max(pos2(r.min.x + r.height() / 2.0, r.min.y), r.max);
    if inner.width() <= 0.0 {
        return;
    }
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(inner.left_top(), a);
    mesh.colored_vertex(inner.right_top(), b);
    mesh.colored_vertex(inner.right_bottom(), b);
    mesh.colored_vertex(inner.left_bottom(), a);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
    painter.circle_filled(pos2(r.max.x, r.center().y), r.height() / 2.0, b);
}

// --- Boutons et icônes ----------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Icon {
    Play,
    Pause,
    PrevLine,
    NextLine,
    Tracks,
    Speed,
    Zoom,
    List,
    Folder,
    Plus,
    Minus,
    Close,
}

fn icon(painter: &egui::Painter, c: Pos2, size: f32, icon: Icon, color: Color32) {
    let s = size / 24.0;
    let p = |x: f32, y: f32| pos2(c.x + (x - 12.0) * s, c.y + (y - 12.0) * s);
    let stroke = Stroke::new(2.0 * s, color);
    match icon {
        Icon::Play => {
            painter.add(Shape::convex_polygon(vec![p(8.0, 5.0), p(19.5, 12.0), p(8.0, 19.0)], color, Stroke::NONE));
        }
        Icon::Pause => {
            painter.rect_filled(Rect::from_min_max(p(6.0, 4.5), p(10.5, 19.5)), CornerRadius::same((1.5 * s) as u8), color);
            painter.rect_filled(Rect::from_min_max(p(13.5, 4.5), p(18.0, 19.5)), CornerRadius::same((1.5 * s) as u8), color);
        }
        Icon::PrevLine => {
            painter.rect_filled(Rect::from_min_max(p(6.0, 6.0), p(8.0, 18.0)), CornerRadius::ZERO, color);
            painter.add(Shape::convex_polygon(vec![p(18.0, 6.0), p(18.0, 18.0), p(9.5, 12.0)], color, Stroke::NONE));
        }
        Icon::NextLine => {
            painter.rect_filled(Rect::from_min_max(p(16.0, 6.0), p(18.0, 18.0)), CornerRadius::ZERO, color);
            painter.add(Shape::convex_polygon(vec![p(6.0, 6.0), p(14.5, 12.0), p(6.0, 18.0)], color, Stroke::NONE));
        }
        Icon::Tracks => {
            painter.rect_stroke(Rect::from_min_max(p(3.0, 5.0), p(21.0, 19.0)), CornerRadius::same((2.5 * s) as u8), stroke, StrokeKind::Inside);
            painter.line_segment([p(7.0, 13.0), p(11.0, 13.0)], stroke);
            painter.line_segment([p(13.0, 13.0), p(17.0, 13.0)], stroke);
            painter.line_segment([p(7.0, 16.0), p(15.0, 16.0)], stroke);
        }
        Icon::Speed => {
            painter.add(Shape::Path(egui::epaint::PathShape::line(
                (0..=16).map(|i| {
                    let a = std::f32::consts::PI * (0.85 + 1.3 * i as f32 / 16.0);
                    p(12.0 + 8.0 * a.cos(), 14.0 + 8.0 * a.sin())
                }).collect(),
                stroke,
            )));
            painter.line_segment([p(12.0, 14.0), p(16.5, 8.5)], stroke);
            painter.circle_filled(p(12.0, 14.0), 1.8 * s, color);
        }
        Icon::Zoom => {
            for (a, b, d) in [((5.0, 9.5), (5.0, 5.0), (9.5, 5.0)), ((14.5, 5.0), (19.0, 5.0), (19.0, 9.5)), ((19.0, 14.5), (19.0, 19.0), (14.5, 19.0)), ((9.5, 19.0), (5.0, 19.0), (5.0, 14.5))] {
                painter.add(Shape::line(vec![p(a.0, a.1), p(b.0, b.1), p(d.0, d.1)], stroke));
            }
        }
        Icon::List => {
            for y in [7.0, 12.0, 17.0] {
                painter.circle_filled(p(5.0, y), 1.3 * s, color);
                painter.line_segment([p(8.5, y), p(20.0, y)], stroke);
            }
        }
        Icon::Folder => {
            painter.add(Shape::line(vec![p(3.0, 18.0), p(3.0, 6.0), p(9.0, 6.0), p(11.0, 8.0), p(21.0, 8.0), p(21.0, 18.0), p(3.0, 18.0)], stroke));
        }
        Icon::Plus => {
            painter.line_segment([p(6.0, 12.0), p(18.0, 12.0)], stroke);
            painter.line_segment([p(12.0, 6.0), p(12.0, 18.0)], stroke);
        }
        Icon::Minus => {
            painter.line_segment([p(6.0, 12.0), p(18.0, 12.0)], stroke);
        }
        Icon::Close => {
            painter.line_segment([p(7.0, 7.0), p(17.0, 17.0)], stroke);
            painter.line_segment([p(17.0, 7.0), p(7.0, 17.0)], stroke);
        }
    }
}

fn icon_button(ui: &mut Ui, r: Rect, i: Icon) -> Response {
    icon_button_tint(ui, r, i, INK)
}

fn icon_button_tint(ui: &mut Ui, r: Rect, i: Icon, color: Color32) -> Response {
    let resp = ui.interact(r, ui.id().with((r.min.x as i32, r.min.y as i32)), Sense::click());
    let k = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    let press = if resp.is_pointer_button_down_on() { 0.9 } else { 1.0 };
    if k > 0.0 {
        ui.painter().circle_filled(r.center(), r.width() / 2.0, Color32::from_white_alpha((28.0 * k) as u8));
    }
    icon(ui.painter(), r.center(), r.width() * 0.52 * press, i, color);
    resp
}

const PILL_PAD: f32 = 16.0;
const PILL_ICON: f32 = 28.0;

/// Largeur d'une capsule d'après son texte réel : le libellé ne déborde jamais.
fn pill_width(ui: &Ui, label: &str, with_icon: bool) -> f32 {
    let w = ui.painter().layout_no_wrap(label.to_owned(), semi(14.0), INK).size().x;
    PILL_PAD * 2.0 + w + if with_icon { PILL_ICON } else { 0.0 }
}

/// Capsule icône + texte, contenu centré dans la capsule.
fn pill(ui: &mut Ui, r: Rect, i: Option<Icon>, label: &str, color: Color32) -> Response {
    let resp = ui.interact(r, ui.id().with(label).with((r.min.x as i32, 7)), Sense::click());
    let k = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    if k > 0.0 {
        ui.painter().rect_filled(r, CornerRadius::same((r.height() / 2.0) as u8), Color32::from_white_alpha((28.0 * k) as u8));
    }
    let g = ui.painter().layout_no_wrap(label.to_owned(), semi(14.0), color);
    let content = g.size().x + if i.is_some() { PILL_ICON } else { 0.0 };
    let mut x = r.center().x - content / 2.0;
    if let Some(i) = i {
        icon(ui.painter(), pos2(x + 10.0, r.center().y), 21.0, i, color);
        x += PILL_ICON;
    }
    ui.painter().galley(pos2(x, r.center().y - g.size().y / 2.0), g, color);
    resp
}

fn chip(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let g = ui.painter().layout_no_wrap(label.to_owned(), semi(14.0), INK);
    let size = g.size() + vec2(30.0, 16.0);
    let (r, resp) = ui.allocate_exact_size(size, Sense::click());
    let cr = CornerRadius::same((size.y / 2.0) as u8);
    let hover = resp.hovered();
    if selected {
        ui.painter().rect_filled(r, cr, ACCENT);
    } else {
        ui.painter().rect_filled(r, cr, Color32::from_white_alpha(if hover { 34 } else { 22 }));
        ui.painter().rect_stroke(r, cr, Stroke::new(1.0, Color32::from_white_alpha(36)), StrokeKind::Inside);
    }
    let color = if selected { Color32::from_rgb(6, 19, 28) } else { INK };
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label, semi(14.0), color);
    resp
}

fn choice(ui: &mut Ui, selected: bool, title: &str, detail: &str) -> Response {
    let w = ui.available_width();
    let h = if detail.is_empty() { 40.0 } else { 52.0 };
    let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let cr = CornerRadius::same(14);
    if selected {
        ui.painter().rect_filled(r, cr, ACCENT.gamma_multiply(0.16));
        ui.painter().rect_stroke(r, cr, Stroke::new(1.0, ACCENT.gamma_multiply(0.45)), StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, cr, Color32::from_white_alpha(16));
    }
    let color = if selected { ACCENT } else { INK };
    let x = r.min.x + 14.0;
    if detail.is_empty() {
        ui.painter().text(pos2(x, r.center().y), Align2::LEFT_CENTER, title, semi(15.0), color);
    } else {
        ui.painter().text(pos2(x, r.min.y + 17.0), Align2::LEFT_CENTER, title, semi(15.0), color);
        ui.painter().text(pos2(x, r.min.y + 36.0), Align2::LEFT_CENTER, detail, med(12.0), INK2);
    }
    if selected {
        let c = pos2(r.max.x - 20.0, r.center().y);
        ui.painter().add(Shape::line(vec![c + vec2(-6.0, 0.0), c + vec2(-2.0, 4.0), c + vec2(6.0, -5.0)], Stroke::new(2.2, ACCENT)));
    }
    resp
}

// --- Tiroirs --------------------------------------------------------------------------------------

fn drawer(ctx: &egui::Context, p: &mut Player, plates: &mut Plates, screen: Rect, panel: Panel) {
    let w: f32 = match panel {
        Panel::Tracks | Panel::List => 780.0f32,
        _ => 580.0,
    }
    .min(screen.width() - 48.0);
    let bottom = screen.max.y - 22.0 - 100.0 - 14.0;
    let id = Id::new(("drawer", panel as u8));
    // Hauteur mesurée à l'image précédente : le verre épouse le contenu.
    // Place disponible au-dessus de la barre : le tiroir défile plutôt que de déborder.
    let avail = (bottom - screen.min.y - 16.0).max(140.0);
    let h: f32 = ctx.data(|d| d.get_temp::<f32>(id)).unwrap_or(200.0).min(avail);
    let rect = Rect::from_min_size(pos2(screen.center().x - w / 2.0, bottom - h), vec2(w, h));
    plates.add(rect, 28.0, 0.42);
    let out = egui::Area::new(id).order(Order::Foreground).fixed_pos(rect.min).constrain(false).show(ctx, |ui| {
        ui.set_width(w);
        egui::Frame::NONE.inner_margin(22.0).show(ui, |ui| {
            ui.set_width(w - 44.0);
            let inner = avail - 44.0;
            match panel {
                Panel::List => list_panel(ui, p, inner),
                _ => {
                    egui::ScrollArea::vertical().id_salt("drawer").max_height(inner).auto_shrink([false, true]).show(ui, |ui| match panel {
                        Panel::Tracks => tracks_panel(ui, p),
                        Panel::Speed => speed_panel(ui, p),
                        _ => zoom_panel(ui, p),
                    });
                }
            }
        });
    });
    let measured = out.response.rect.height();
    if (measured - h).abs() > 0.5 {
        ctx.data_mut(|d| d.insert_temp(id, measured));
        ctx.request_repaint();
    }
}

fn header(ui: &mut Ui, title: &str, trailing: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).font(semi(13.0)).color(INK2));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(trailing).font(med(13.0)).color(FAINT));
        });
    });
    ui.add_space(6.0);
}

fn tracks_panel(ui: &mut Ui, p: &mut Player) {
    let audio: Vec<_> = p.mpv.state.tracks.iter().filter(|t| t.kind == "audio").cloned().collect();
    let subs: Vec<_> = p.mpv.state.tracks.iter().filter(|t| t.kind == "sub").cloned().collect();
    ui.columns(2, |cols| {
        header(&mut cols[0], "Audio", &audio.len().to_string());
        egui::ScrollArea::vertical().id_salt("audio").max_height(250.0).show(&mut cols[0], |ui| {
            for t in &audio {
                if choice(ui, p.mpv.state.aid == Some(t.id), &track_label(t), &track_detail(t)).clicked() {
                    p.mpv.set("aid", &t.id.to_string());
                    p.say(format!("Audio : {}", track_label(t)));
                }
            }
        });
        header(&mut cols[1], "Sous-titres", &subs.len().to_string());
        egui::ScrollArea::vertical().id_salt("subs").max_height(250.0).show(&mut cols[1], |ui| {
            if choice(ui, p.mpv.state.sid.is_none(), "Désactivés", "").clicked() {
                p.mpv.set("sid", "no");
                p.say("Sous-titres désactivés");
            }
            for t in &subs {
                if choice(ui, p.mpv.state.sid == Some(t.id), &track_label(t), &track_detail(t)).clicked() {
                    p.mpv.set("sid", &t.id.to_string());
                    p.say(format!("Sous-titres : {}", track_label(t)));
                }
            }
            ui.label(egui::RichText::new("Glissez un fichier .srt / .ass sur la fenêtre pour l'ajouter").font(med(12.0)).color(FAINT));
        });
    });
    ui.add_space(12.0);
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(line, CornerRadius::ZERO, Color32::from_white_alpha(28));
    ui.add_space(12.0);
    ui.columns(2, |cols| {
        delay(&mut cols[0], p, "Décalage audio", "audio-delay", 0.05);
        delay(&mut cols[1], p, "Décalage sous-titres", "sub-delay", 0.1);
    });
}

fn delay(ui: &mut Ui, p: &mut Player, label: &str, prop: &str, step: f64) {
    let v = if prop == "audio-delay" { p.mpv.state.audio_delay } else { p.mpv.state.sub_delay };
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(label).font(semi(13.0)).color(INK2));
            let ms = (v * 1000.0).round() as i64;
            let txt = if ms == 0 { "0 ms".to_string() } else { format!("{ms:+} ms").replace('-', "−") };
            let r = ui.add(egui::Label::new(egui::RichText::new(txt).font(semi(17.0)).color(if ms == 0 { INK } else { ACCENT })).sense(Sense::click()));
            if r.clicked() {
                p.mpv.set_f64(prop, 0.0);
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
            if round_button(ui, r, Icon::Plus).clicked() {
                p.mpv.set_f64(prop, v + step);
            }
            let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
            if round_button(ui, r, Icon::Minus).clicked() {
                p.mpv.set_f64(prop, v - step);
            }
        });
    });
}

fn round_button(ui: &mut Ui, r: Rect, i: Icon) -> Response {
    // Rayon un peu en retrait : le liseré reste entier, même contre le bord de la colonne.
    let radius = r.width() / 2.0 - 2.0;
    ui.painter().circle_filled(r.center(), radius, Color32::from_white_alpha(20));
    ui.painter().circle_stroke(r.center(), radius, Stroke::new(1.0, Color32::from_white_alpha(40)));
    icon_button(ui, r.shrink(2.0), i)
}

fn big_value(ui: &mut Ui, title: &str, hint: &str, value: &str, active: bool) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(title).font(semi(13.0)).color(INK2));
            ui.label(egui::RichText::new(hint).font(med(12.0)).color(FAINT));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(value).font(semi(34.0)).color(if active { ACCENT } else { INK }));
        });
    });
    ui.add_space(10.0);
}

fn slider(ui: &mut Ui, value: f64, min: f64, max: f64, step: f64, marks: &[f64]) -> Option<f64> {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click_and_drag());
    let f = ((value - min) / (max - min)).clamp(0.0, 1.0) as f32;
    let h = 8.0;
    let track = Rect::from_center_size(r.center(), vec2(r.width(), h));
    let painter = ui.painter();
    painter.rect_filled(track, CornerRadius::same(4), Color32::from_white_alpha(40));
    gradient(painter, Rect::from_min_size(track.min, vec2(track.width() * f, h)), ACCENT_DEEP, ACCENT, CornerRadius::same(4));
    for m in marks {
        let x = track.min.x + track.width() * ((m - min) / (max - min)) as f32;
        painter.circle_filled(pos2(x, track.center().y), 2.0, Color32::from_white_alpha(140));
    }
    let x = track.min.x + track.width() * f;
    painter.circle_filled(pos2(x, track.center().y + 1.0), 13.0, Color32::from_black_alpha(64));
    painter.circle_filled(pos2(x, track.center().y), 12.0, Color32::WHITE);
    if resp.dragged() || resp.clicked() {
        let q = resp.interact_pointer_pos()?;
        let v = min + ((q.x - track.min.x) / track.width()).clamp(0.0, 1.0) as f64 * (max - min);
        let s = (v / step).round() * step;
        let snapped = marks.iter().copied().find(|m| (m - s).abs() < step * 0.9).unwrap_or(s.clamp(min, max));
        return Some(snapped);
    }
    None
}

fn speed_panel(ui: &mut Ui, p: &mut Player) {
    let s = p.mpv.state.speed;
    big_value(ui, "Vitesse", "Voix préservée, sans distorsion", &speed_label(s), (s - 1.0).abs() > 0.001);
    if let Some(v) = slider(ui, s, 0.5, 2.0, 0.05, &[1.0, 1.5]) {
        p.mpv.set_f64("speed", v);
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        for v in [0.75, 1.0, 1.1, 1.25, 1.4, 1.5] {
            if chip(ui, &speed_label(v), (s - v).abs() < 0.001).clicked() {
                p.mpv.set_f64("speed", v);
            }
        }
    });
}

fn zoom_panel(ui: &mut Ui, p: &mut Player) {
    let z = 2f64.powf(p.mpv.state.zoom);
    big_value(ui, "Image", "Ctrl + molette pour zoomer", &format!("{z:.1}×"), z > 1.01);
    if let Some(v) = slider(ui, z, 1.0, 4.0, 0.1, &[2.0, 3.0]) {
        p.mpv.set_f64("video-zoom", v.log2());
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        let fill = p.mpv.get("panscan").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) > 0.5;
        let unscaled = p.mpv.get("video-unscaled").as_deref() == Some("yes");
        if chip(ui, "Image entière", !fill && !unscaled && z <= 1.01).clicked() {
            p.mpv.set_f64("video-zoom", 0.0);
            p.mpv.set_f64("video-pan-x", 0.0);
            p.mpv.set_f64("video-pan-y", 0.0);
            p.mpv.set_f64("panscan", 0.0);
            p.mpv.set("video-unscaled", "no");
        }
        if chip(ui, "Remplir l'écran", fill).clicked() {
            p.mpv.set_f64("panscan", if fill { 0.0 } else { 1.0 });
        }
        if chip(ui, "Taille réelle", unscaled).clicked() {
            p.mpv.set("video-unscaled", if unscaled { "no" } else { "yes" });
        }
    });
}

fn list_panel(ui: &mut Ui, p: &mut Player, max_height: f32) {
    let label = p.sub_track().map(|t| format!("Sous-titres · {}", track_label(t))).unwrap_or("Sous-titres".into());
    let count = p.sub_index().map(|i| format!("{} lignes", i.cues.len())).unwrap_or_default();
    header(ui, &label, &count);
    let Some(idx) = p.sub_index() else {
        let why = if p.sub_track().is_none() {
            "Choisissez une piste de sous-titres."
        } else if p.indexing {
            "Lecture des sous-titres…"
        } else {
            "Liste indisponible pour ce format. Les sauts ligne à ligne passent par mpv."
        };
        ui.label(egui::RichText::new(why).font(med(15.0)).color(INK2));
        return;
    };
    let current = idx.at(p.sub_clock(p.mpv.state.position));
    let row_h = 46.0;
    let height = (max_height - 34.0).clamp(80.0, 520.0);
    let id = Id::new("list-follow");
    let last: Option<usize> = ui.ctx().data(|d| d.get_temp(id)).flatten();
    let mut area = egui::ScrollArea::vertical().id_salt("cues").min_scrolled_height(height).max_height(height).auto_shrink([false, false]);
    // Suit la lecture, sauf si l'utilisateur vient de faire défiler à la main.
    let touched: Option<Instant> = ui.ctx().data(|d| d.get_temp(Id::new("list-touched")));
    if current != last && touched.is_none_or(|t| t.elapsed() > Duration::from_secs(4)) {
        if let Some(c) = current {
            area = area.vertical_scroll_offset((c as f32 - 2.0).max(0.0) * row_h);
        }
        ui.ctx().data_mut(|d| d.insert_temp(id, current));
    }
    let mut seek = None;
    let out = area.show_rows(ui, row_h, idx.cues.len(), |ui, range| {
        for i in range {
            let c = &idx.cues[i];
            let now = Some(i) == current;
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
            if now {
                ui.painter().rect_filled(r, CornerRadius::same(14), ACCENT.gamma_multiply(0.14));
            } else if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(14), Color32::from_white_alpha(14));
            }
            ui.painter().text(pos2(r.min.x + 12.0, r.center().y), Align2::LEFT_CENTER, clock(c.start as f64 / 1000.0), semi(13.0), if now { ACCENT } else { FAINT });
            let text = c.text.replace('\n', "  ");
            let clip = ui.painter().with_clip_rect(Rect::from_min_max(pos2(r.min.x + 84.0, r.min.y), pos2(r.max.x - 10.0, r.max.y)));
            clip.text(pos2(r.min.x + 84.0, r.center().y), Align2::LEFT_CENTER, text, if now { semi(15.0) } else { med(15.0) }, if now { INK } else { INK2 });
            if resp.clicked() {
                seek = Some(c.start as f64 / 1000.0);
            }
        }
    });
    if ui.ctx().input(|i| i.smooth_scroll_delta.y != 0.0) && out.inner_rect.contains(ui.ctx().pointer_hover_pos().unwrap_or(Pos2::ZERO)) {
        ui.ctx().data_mut(|d| d.insert_temp(Id::new("list-touched"), Instant::now()));
    }
    if let Some(t) = seek {
        let d = p.mpv.state.sub_delay;
        p.mpv.seek(t + d, true);
    }
}

// --- Messages ------------------------------------------------------------------------------------

fn toast(ctx: &egui::Context, p: &mut Player, plates: &mut Plates, screen: Rect) {
    let Some((text, at)) = &p.message else { return };
    let age = at.elapsed().as_secs_f32();
    if age > 1.7 {
        p.message = None;
        return;
    }
    let a = (age / 0.12).min(1.0) * ((1.7 - age) / 0.3).min(1.0);
    let g = ctx.fonts_mut(|f| f.layout_no_wrap(text.clone(), semi(15.0), INK));
    let size = g.size() + vec2(40.0, 22.0);
    let rect = Rect::from_center_size(pos2(screen.center().x, screen.min.y + 30.0 + size.y / 2.0), size);
    plates.alpha = a;
    plates.add(rect, size.y / 2.0, 0.4);
    plates.alpha = 1.0;
    egui::Area::new(Id::new("toast")).order(Order::Tooltip).fixed_pos(rect.min).interactable(false).show(ctx, |ui| {
        ui.set_opacity(a);
        ui.painter().galley(rect.center() - g.size() / 2.0, g, INK);
    });
    ctx.request_repaint();
}

fn scrub_osd(ctx: &egui::Context, p: &Player, s: &crate::player::Scrub, plates: &mut Plates, screen: Rect) {
    let sign = if s.steps > 0 { "+" } else if s.steps < 0 { "−" } else { "" };
    let n = s.steps.abs();
    let head = match s.zone {
        Zone::Line if n == 0 => "Sous-titres".to_string(),
        Zone::Line => format!("{sign}{n} {}", if n > 1 { "lignes" } else { "ligne" }),
        Zone::Ten if n == 0 => "10 s".into(),
        Zone::Ten => format!("{sign}{} s", n * 10),
        Zone::Minute if n == 0 => "1 min".into(),
        Zone::Minute => format!("{sign}{n} min"),
    };
    let sub = if s.zone == Zone::Line && s.cue.is_none() && p.sub_index().is_none() {
        "saut par mpv".to_string()
    } else {
        format!("{}  /  {}", clock(s.target), clock(p.mpv.state.duration))
    };
    let w = 520.0f32.min(screen.width() - 48.0);
    let id = Id::new("scrub");
    let h: f32 = ctx.data(|d| d.get_temp(id)).unwrap_or(150.0);
    let rect = Rect::from_center_size(screen.center(), vec2(w, h));
    plates.add(rect, 28.0, 0.45);
    let out = egui::Area::new(id).order(Order::Tooltip).fixed_pos(rect.min).interactable(false).show(ctx, |ui| {
        ui.set_width(w);
        egui::Frame::NONE.inner_margin(egui::Margin::symmetric(26, 18)).show(ui, |ui| {
            ui.set_width(w - 52.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(head).font(semi(30.0)).color(INK));
                ui.label(egui::RichText::new(sub).font(med(15.0)).color(INK2));
                if let Some(c) = &s.cue {
                    ui.add_space(8.0);
                    ui.add(egui::Label::new(egui::RichText::new(&c.text).font(med(17.0)).color(ACCENT)).wrap());
                }
            });
        });
    });
    let measured = out.response.rect.height();
    if (measured - h).abs() > 0.5 {
        ctx.data_mut(|d| d.insert_temp(id, measured));
        ctx.request_repaint();
    }
}

// --- Accueil -------------------------------------------------------------------------------------

fn home(ctx: &egui::Context, p: &mut Player, plates: &mut Plates, actions: &mut Vec<Action>, screen: Rect) {
    egui::Area::new(Id::new("home-bg")).order(Order::Background).fixed_pos(screen.min).show(ctx, |ui| {
        ui.allocate_rect(screen, Sense::hover());
        // Les lumières de fond sont dessinées par le GPU (gfx.rs), sans paliers.
    });
    let w = 560.0f32.min(screen.width() - 48.0);
    let id = Id::new("home");
    let h: f32 = ctx.data(|d| d.get_temp(id)).unwrap_or(260.0);
    let rect = Rect::from_center_size(screen.center(), vec2(w, h));
    plates.add(rect, 32.0, 0.3);
    let out = egui::Area::new(id).order(Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
        ui.set_width(w);
        egui::Frame::NONE.inner_margin(28.0).show(ui, |ui| {
            ui.set_width(w - 56.0);
            ui.label(egui::RichText::new("bioplay").font(semi(40.0)).color(INK));
            ui.label(egui::RichText::new("Glissez une vidéo ici, ou ouvrez-la.").font(med(15.0)).color(INK2));
            ui.add_space(18.0);
            let (r, _) = ui.allocate_exact_size(vec2(pill_width(ui, "Ouvrir une vidéo", true) + 8.0, 46.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(23), Color32::from_white_alpha(26));
            ui.painter().rect_stroke(r, CornerRadius::same(23), Stroke::new(1.0, Color32::from_white_alpha(60)), StrokeKind::Inside);
            if pill(ui, r, Some(Icon::Folder), "Ouvrir une vidéo", INK).clicked() {
                actions.push(Action::Open);
            }
            if !p.recents.is_empty() {
                ui.add_space(20.0);
                ui.label(egui::RichText::new("Reprendre").font(semi(13.0)).color(INK2));
                ui.add_space(4.0);
                for r in &p.recents {
                    let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(row, CornerRadius::same(16), Color32::from_white_alpha(16));
                    }
                    let clip = ui.painter().with_clip_rect(row.shrink2(vec2(0.0, 0.0)).with_max_x(row.max.x - 44.0));
                    let name = std::path::Path::new(&r.name).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or(r.name.clone());
                    clip.text(pos2(row.min.x + 12.0, row.min.y + 17.0), Align2::LEFT_CENTER, name, semi(15.0), INK);
                    let rest = if r.duration > 0.0 && r.position > 5.0 { format!("Reste {}", clock(r.duration - r.position)) } else { clock(r.duration) };
                    clip.text(pos2(row.min.x + 12.0, row.min.y + 35.0), Align2::LEFT_CENTER, rest, med(12.0), INK2);
                    let f = if r.duration > 0.0 { (r.position / r.duration).clamp(0.0, 1.0) as f32 } else { 0.0 };
                    let bar = Rect::from_min_size(pos2(row.min.x + 12.0, row.max.y - 7.0), vec2(row.width() - 68.0, 3.0));
                    ui.painter().rect_filled(bar, CornerRadius::same(2), Color32::from_white_alpha(36));
                    ui.painter().rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * f, 3.0)), CornerRadius::same(2), ACCENT);
                    let close = Rect::from_center_size(pos2(row.max.x - 22.0, row.center().y), vec2(32.0, 32.0));
                    if icon_button_tint(ui, close, Icon::Close, FAINT).clicked() {
                        actions.push(Action::Forget(r.path.clone()));
                    } else if resp.clicked() {
                        actions.push(Action::Play(r.path.clone()));
                    }
                }
            }
        });
    });
    let measured = out.response.rect.height();
    if (measured - h).abs() > 0.5 {
        ctx.data_mut(|d| d.insert_temp(id, measured));
        ctx.request_repaint();
    }
}
