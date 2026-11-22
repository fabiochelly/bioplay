//! Interface plein écran : barre de verre en bas, tiroirs au-dessus, messages en haut. egui
//! ne dessine que le texte et les icônes ; les plaques de verre sont rendues par `gfx`.

use std::time::{Duration, Instant};

use egui::{
    pos2, vec2, Align2, Color32, CornerRadius, FontFamily, FontId, Id, Order, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui,
};

use crate::i18n::{en, tr};
use crate::mpv::{track_detail, track_label};
use crate::player::{clock, message_alpha, speed_label, Panel, Player, Zone, MESSAGE_LIFE};

/// Le contrôleur s'efface 1,5 s après le dernier mouvement de souris…
pub const HIDE_AFTER: Duration = Duration::from_millis(1500);
/// … ou 3 s si la souris reste posée dessus (le temps de viser un bouton).
pub const HIDE_AFTER_HOVER: Duration = Duration::from_millis(3000);

pub const INK: Color32 = Color32::from_rgb(247, 248, 250);
pub const INK2: Color32 = Color32::from_rgba_premultiplied(181, 182, 184, 184);
pub const FAINT: Color32 = Color32::from_rgba_premultiplied(99, 99, 100, 102);
pub const ACCENT: Color32 = Color32::from_rgb(143, 216, 255);
pub const ACCENT_DEEP: Color32 = Color32::from_rgb(79, 178, 240);

pub fn fonts(ctx: &egui::Context) {
    // Inter seule : les polices par défaut d'egui (Ubuntu, Hack, emoji) ne sont pas chargées.
    let mut f = egui::FontDefinitions::empty();
    let medium = include_bytes!("../../app/src/main/assets/inter-medium.ttf");
    let semi = include_bytes!("../../app/src/main/assets/inter-semibold.ttf");
    f.font_data.insert("inter".into(), egui::FontData::from_static(medium).into());
    f.font_data.insert("inter-semi".into(), egui::FontData::from_static(semi).into());
    f.families.insert(FontFamily::Proportional, vec!["inter".into()]);
    f.families.insert(FontFamily::Monospace, vec!["inter".into()]);
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
    AddFolder,
    Play(String),
}

/// Plaques de verre de l'image en cours, en pixels.
/// Plaque de verre, en pixels de la fenêtre : le shader de mpv floute la vidéo dessous.
pub struct Plate {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub radius: f32,
    pub alpha: f32,
}

pub struct Plates {
    pub list: Vec<Plate>,
    ppp: f32,
    alpha: f32,
    painter: egui::Painter,
}

impl Plates {
    /// Une plaque : le flou vient du shader ; ici la teinte, le reflet et le liseré, dessinés
    /// sous le contenu. Sur les bandes noires (hors vidéo), la teinte seule fait le verre.
    fn add(&mut self, r: Rect, radius: f32, tint: f32) {
        let a = self.alpha;
        self.list.push(Plate {
            x: r.min.x * self.ppp,
            y: r.min.y * self.ppp,
            w: r.width() * self.ppp,
            h: r.height() * self.ppp,
            radius: radius * self.ppp,
            alpha: a,
        });
        let cr = CornerRadius::same(radius.min(r.height() / 2.0).min(127.0) as u8);
        let painter = &self.painter;
        painter.rect_filled(r, cr, Color32::from_black_alpha((255.0 * tint * a) as u8));
        // Liseré : fin et régulier, sans trait plus vif en haut.
        painter.rect_stroke(r, cr, Stroke::new(1.0, Color32::from_white_alpha((38.0 * a) as u8)), StrokeKind::Inside);
    }
}

pub fn draw(ctx: &egui::Context, p: &mut Player, ppp: f32) -> (Vec<Plate>, Vec<Action>) {
    let painter = ctx.layer_painter(egui::LayerId::new(Order::Background, Id::new("plates")));
    let mut plates = Plates { list: Vec::new(), ppp, alpha: 1.0, painter };
    let mut actions = Vec::new();
    let screen = ctx.content_rect();

    p.thumbs.clear();
    if !p.has_file() {
        home(ctx, p, &mut plates, &mut actions, screen);
        return (plates.list, actions);
    }

    let idle = p.activity.elapsed() > HIDE_AFTER;
    // La souris posée sur l'interface la garde affichée (on ne clique pas dans le vide), mais
    // pas indéfiniment : après 3 s d'immobilité elle s'efface quand même (reprise de lecture).
    let hovering = p.over_ui && p.activity.elapsed() < HIDE_AFTER_HOVER;
    let show = !idle || hovering || p.mpv.state.paused || p.panel.is_some() || p.scrub.is_some();
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
                        p.mpv.own_seeks.set(p.mpv.own_seeks.get() + 1);
                        p.mpv.command(&["sub-seek", &s.steps.to_string()]);
                    } else {
                        p.mpv.seek(s.target, true);
                    }
                } else {
                    p.mpv.seek(s.from, true);
                }
            }
        }
        // Un clic sur la vidéo ferme le tiroir ouvert ; le reste des clics (double clic,
        // clic droit…) est traité par mpv lui-même, selon input.conf.
        if r.clicked() {
            p.panel = None;
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
        let (audio_name, sub_name) = track_names(p);
        let tflags = track_flags(p);
        let tw = tracks_width(ui, &audio_name, sub_name.as_deref(), tflags);
        rx -= tw + 6.0;
        if tracks_button(ui, Rect::from_min_size(pos2(rx, cy - 21.0), vec2(tw, 42.0)), &audio_name, sub_name.as_deref(), tflags).clicked() {
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
        if p.indexing { format!("{name}  ·  {}", tr("lecture des sous-titres…", "reading subtitles…")) } else { name }
    }
}

/// Langues affichées sur le bouton des pistes : audio, et sous-titres (None = désactivés).
fn track_names(p: &Player) -> (String, Option<String>) {
    let s = &p.mpv.state;
    let name = |t: &crate::mpv::Track| {
        let n = crate::mpv::language(&t.lang).unwrap_or_else(|| track_label(t));
        if n.chars().count() > 14 { n.chars().take(13).collect::<String>() + "…" } else { n }
    };
    let audio = s.tracks.iter().find(|t| t.kind == "audio" && Some(t.id) == s.aid).map(name).unwrap_or("—".into());
    (audio, p.sub_track().map(name))
}

/// Drapeaux des pistes audio et sous-titres choisies.
fn track_flags(p: &Player) -> (Option<&'static str>, Option<&'static str>) {
    let s = &p.mpv.state;
    let audio = s.tracks.iter().find(|t| t.kind == "audio" && Some(t.id) == s.aid).and_then(|t| crate::flags::flag_for(&t.lang, &t.title));
    (audio, p.sub_track().and_then(|t| crate::flags::flag_for(&t.lang, &t.title)))
}

const TB_ICON: f32 = 22.0;

fn tracks_width(ui: &Ui, audio: &str, sub: Option<&str>, flags: (Option<&str>, Option<&str>)) -> f32 {
    let w = |t: &str| ui.painter().layout_no_wrap(t.to_owned(), semi(13.0), INK).size().x;
    let sub_w = w(sub.unwrap_or(tr("Aucun", "Off")));
    let flag_w = |f: Option<&str>| if f.is_some() { 27.0 } else { 0.0 };
    14.0 + TB_ICON + 7.0 + flag_w(flags.0) + w(audio) + 13.0 + 1.0 + 13.0 + TB_ICON + 7.0 + flag_w(flags.1) + sub_w + 16.0
}

/// Bouton des pistes : haut-parleur + langue audio, sous-titres + leur langue, liseré d'accent.
fn tracks_button(ui: &mut Ui, r: Rect, audio: &str, sub: Option<&str>, flags: (Option<&str>, Option<&str>)) -> Response {
    let resp = ui.interact(r, ui.id().with("tracks-button"), Sense::click());
    let k = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let cr = CornerRadius::same((r.height() / 2.0) as u8);
    let painter = ui.painter();
    painter.rect_filled(r, cr, Color32::from_white_alpha((14.0 + 20.0 * k) as u8));
    // Liseré : dégradé d'accent, plus vif au survol.
    painter.rect_stroke(r, cr, Stroke::new(1.0, ACCENT.gamma_multiply(0.30 + 0.45 * k)), StrokeKind::Inside);

    let cy = r.center().y;
    let mut x = r.min.x + 14.0;
    icon(painter, pos2(x + TB_ICON / 2.0, cy), TB_ICON, Icon::Speaker, INK);
    x += TB_ICON + 7.0;
    if let Some(f) = flags.0 {
        crate::flags::draw(painter, Rect::from_min_size(pos2(x, cy - 7.0), vec2(21.0, 14.0)), f, 1.0);
        x += 27.0;
    }
    let g = painter.layout_no_wrap(audio.to_owned(), semi(13.0), INK);
    let w = g.size().x;
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, INK);
    x += w + 13.0;
    painter.line_segment([pos2(x, cy - 10.0), pos2(x, cy + 10.0)], Stroke::new(1.0, Color32::from_white_alpha(50)));
    x += 14.0;
    let (ic, col, label) = match sub {
        Some(s) => (Icon::Tracks, ACCENT, s.to_owned()),
        None => (Icon::SubtitlesOff, INK2, tr("Aucun", "Off").to_owned()),
    };
    icon(painter, pos2(x + TB_ICON / 2.0, cy), TB_ICON, ic, col);
    x += TB_ICON + 7.0;
    if let Some(f) = flags.1 {
        crate::flags::draw(painter, Rect::from_min_size(pos2(x, cy - 7.0), vec2(21.0, 14.0)), f, 1.0);
        x += 27.0;
    }
    let g = painter.layout_no_wrap(label, semi(13.0), col);
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, col);
    resp
}

fn toggle(p: &mut Player, panel: Panel) {
    p.panel = if p.panel == Some(panel) { None } else { Some(panel) };
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
    Up,
    Down,
    Close,
    Speaker,
    SubtitlesOff,
    Back,
    Open,
    Grid,
    Rows,
}

fn icon(painter: &egui::Painter, c: Pos2, size: f32, which: Icon, color: Color32) {
    let s = size / 24.0;
    let p = |x: f32, y: f32| pos2(c.x + (x - 12.0) * s, c.y + (y - 12.0) * s);
    let stroke = Stroke::new(2.0 * s, color);
    match which {
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
            // Sous-titres : cadre arrondi à bord fin, deux lignes de texte en pastilles.
            painter.rect_stroke(Rect::from_min_max(p(2.0, 4.0), p(22.0, 20.0)), CornerRadius::same((3.5 * s) as u8), Stroke::new(1.7 * s, color), StrokeKind::Inside);
            let pill = CornerRadius::same((0.8 * s).max(1.0) as u8);
            for (x0, x1, y) in [(5.9, 14.3, 11.4), (14.5, 18.1, 11.4), (5.9, 10.1, 14.6), (10.3, 18.1, 14.6)] {
                painter.rect_filled(Rect::from_min_max(p(x0 + 0.4, y), p(x1 - 0.4, y + 1.6)), pill, color);
            }
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
        Icon::Up => {
            painter.add(Shape::line(vec![p(6.5, 15.0), p(12.0, 9.0), p(17.5, 15.0)], stroke));
        }
        Icon::Down => {
            painter.add(Shape::line(vec![p(6.5, 9.0), p(12.0, 15.0), p(17.5, 9.0)], stroke));
        }
        Icon::Speaker => {
            painter.rect_filled(Rect::from_min_max(p(3.5, 9.0), p(8.0, 15.0)), CornerRadius::same((1.0 * s) as u8), color);
            painter.add(Shape::convex_polygon(vec![p(7.5, 9.0), p(13.0, 4.5), p(13.0, 19.5), p(7.5, 15.0)], color, Stroke::NONE));
            for r in [3.6f32, 7.0] {
                let pts = (0..=10).map(|i| {
                    let a = (-50.0 + 10.0 * i as f32).to_radians();
                    p(14.0 + r * a.cos(), 12.0 + r * a.sin())
                }).collect();
                painter.add(Shape::line(pts, Stroke::new(1.8 * s, color)));
            }
        }
        Icon::SubtitlesOff => {
            icon(painter, c, size, Icon::Tracks, color);
            painter.line_segment([p(3.0, 21.0), p(21.0, 3.0)], Stroke::new(2.6 * s, Color32::from_black_alpha(150)));
            painter.line_segment([p(3.5, 20.5), p(20.5, 3.5)], stroke);
        }
        Icon::Open => {
            // Dossier ouvert habituel : le fond, puis le rabat avant incliné.
            let thin = Stroke::new(1.9 * s, color);
            painter.add(Shape::line(vec![p(3.5, 18.5), p(3.5, 5.5), p(9.3, 5.5), p(11.3, 7.5), p(19.5, 7.5), p(19.5, 10.3)], thin));
            painter.add(Shape::closed_line(vec![p(3.5, 18.5), p(6.4, 10.3), p(21.8, 10.3), p(18.9, 18.5)], thin));
        }
        Icon::Back => {
            painter.add(Shape::line(vec![p(14.5, 6.0), p(8.5, 12.0), p(14.5, 18.0)], stroke));
        }
        Icon::Grid => {
            for (x, y) in [(4.0, 4.0), (13.0, 4.0), (4.0, 13.0), (13.0, 13.0)] {
                painter.rect_filled(Rect::from_min_max(p(x, y), p(x + 7.0, y + 7.0)), CornerRadius::same((1.5 * s) as u8), color);
            }
        }
        Icon::Rows => {
            for y in [4.5, 10.0, 15.5] {
                painter.rect_filled(Rect::from_min_max(p(3.0, y), p(8.0, y + 4.0)), CornerRadius::same((1.0 * s) as u8), color);
                painter.rect_filled(Rect::from_min_max(p(10.0, y + 1.0), p(21.0, y + 3.0)), CornerRadius::same((1.0 * s) as u8), color);
            }
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

fn choice(ui: &mut Ui, selected: bool, title: &str, detail: &str, flag: Option<&str>) -> Response {
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
    let mut x = r.min.x + 14.0;
    if let Some(f) = flag {
        crate::flags::draw(ui.painter(), Rect::from_min_size(pos2(x, r.center().y - 9.0), vec2(27.0, 18.0)), f, if selected { 1.0 } else { 0.8 });
        x += 39.0;
    }
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

/// Bascule de langue : un bouton double, drapeau discret et code, la langue active teintée.
fn language_switch(ui: &mut Ui) {
    // Même hauteur que les boutons ronds voisins (40 pt).
    let half = vec2(68.0, 40.0);
    let (r, _) = ui.allocate_exact_size(vec2(half.x * 2.0, half.y), Sense::hover());
    let cr = CornerRadius::same(20);
    let painter = ui.painter().clone();
    painter.rect_filled(r, cr, Color32::from_white_alpha(15));
    for (i, (code, french)) in [("FR", true), ("EN", false)].into_iter().enumerate() {
        let seg = Rect::from_min_size(pos2(r.min.x + i as f32 * half.x, r.min.y), half);
        let selected = en() != french;
        let resp = ui.interact(seg, ui.id().with(("lang", i)), Sense::click());
        // Sélection et survol éclairent la demi-capsule, jamais une forme complète.
        let seg_cr = if i == 0 { CornerRadius { nw: 20, sw: 20, ne: 0, se: 0 } } else { CornerRadius { nw: 0, sw: 0, ne: 20, se: 20 } };
        if selected {
            painter.rect_filled(seg, seg_cr, ACCENT.gamma_multiply(0.2));
        } else if resp.hovered() {
            painter.rect_filled(seg, seg_cr, Color32::from_white_alpha(14));
        }
        let flag = Rect::from_min_size(pos2(seg.min.x + 11.0, seg.center().y - 8.0), vec2(24.0, 16.0));
        crate::flags::draw(&painter, flag, if french { "fr" } else { "en" }, if selected { 1.0 } else { 0.55 });
        painter.text(pos2(flag.max.x + 7.0, seg.center().y), Align2::LEFT_CENTER, code, semi(12.5), if selected { ACCENT } else { INK2 });
        if resp.clicked() && !selected {
            crate::i18n::toggle();
        }
    }
    painter.rect_stroke(r, cr, Stroke::new(1.0, Color32::from_white_alpha(46)), StrokeKind::Inside);
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
                // Pas de défilement du tiroir entier : il épouse son contenu, et seules ses
                // listes raccourcissent quand la place manque.
                Panel::List => list_panel(ui, p, inner),
                Panel::Tracks => tracks_panel(ui, p, inner),
                Panel::Speed => speed_panel(ui, p),
                Panel::Zoom => zoom_panel(ui, p),
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

fn tracks_panel(ui: &mut Ui, p: &mut Player, max_height: f32) {
    // En-têtes, séparateur et deux lignes de réglages occupent ~166 pt ; les listes prennent le reste.
    let lists = (max_height - 166.0).clamp(56.0, 420.0);
    let audio: Vec<_> = p.mpv.state.tracks.iter().filter(|t| t.kind == "audio").cloned().collect();
    let subs: Vec<_> = p.mpv.state.tracks.iter().filter(|t| t.kind == "sub").cloned().collect();
    ui.columns(2, |cols| {
        header(&mut cols[0], tr("Audio", "Audio"), &audio.len().to_string());
        egui::ScrollArea::vertical().id_salt("audio").max_height(lists).show(&mut cols[0], |ui| {
            for t in &audio {
                if choice(ui, p.mpv.state.aid == Some(t.id), &track_label(t), &track_detail(t), crate::flags::flag_for(&t.lang, &t.title)).clicked() {
                    p.mpv.set("aid", &t.id.to_string());
                    p.say(format!("{} : {}", tr("Audio", "Audio"), track_label(t)));
                }
            }
        });
        header(&mut cols[1], tr("Sous-titres", "Subtitles"), &subs.len().to_string());
        egui::ScrollArea::vertical().id_salt("subs").max_height(lists).show(&mut cols[1], |ui| {
            if choice(ui, p.mpv.state.sid.is_none(), tr("Désactivés", "Off"), "", None).clicked() {
                p.mpv.set("sid", "no");
                p.say(tr("Sous-titres désactivés", "Subtitles off"));
            }
            for t in &subs {
                if choice(ui, p.mpv.state.sid == Some(t.id), &track_label(t), &track_detail(t), crate::flags::flag_for(&t.lang, &t.title)).clicked() {
                    p.mpv.set("sid", &t.id.to_string());
                    p.say(format!("{} : {}", tr("Sous-titres", "Subtitles"), track_label(t)));
                }
            }
            ui.label(egui::RichText::new(tr("Glissez un fichier .srt / .ass sur la fenêtre pour l'ajouter", "Drop a .srt / .ass file on the window to add it")).font(med(12.0)).color(FAINT));
        });
    });
    ui.add_space(12.0);
    let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().rect_filled(line, CornerRadius::ZERO, Color32::from_white_alpha(28));
    ui.add_space(12.0);
    // Décalages et langue sur une seule ligne : la place va aux listes de pistes.
    // Mise en page explicite : hauteur fixe, tout centré sur la même ligne, marges entre groupes.
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::hover());
    let (switch_w, gap) = (136.0, 28.0);
    let delay_w = (row.width() - switch_w - 2.0 * gap) / 2.0;
    let audio = Rect::from_min_size(row.min, vec2(delay_w, row.height()));
    let sub = Rect::from_min_size(pos2(audio.max.x + gap, row.min.y), vec2(delay_w, row.height()));
    let switch = Rect::from_center_size(pos2(row.max.x - switch_w / 2.0, row.center().y), vec2(switch_w, 40.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(audio), |ui| delay(ui, p, tr("Décalage audio", "Audio delay"), "audio-delay", 0.05));
    ui.scope_builder(egui::UiBuilder::new().max_rect(sub), |ui| delay(ui, p, tr("Décalage sous-titres", "Subtitle delay"), "sub-delay", 0.1));
    ui.scope_builder(egui::UiBuilder::new().max_rect(switch), language_switch);

    // Taille et hauteur des sous-titres (aussi Maj + pavé +/− et Maj + ↑/↓), communes à toutes les vidéos.
    ui.add_space(6.0);
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::hover());
    let size = Rect::from_min_size(row.min, vec2(delay_w, row.height()));
    let height = Rect::from_min_size(pos2(size.max.x + gap, row.min.y), vec2(delay_w, row.height()));
    let pct = (p.sub_scale * 100.0).round() as i64;
    let size_txt = format!("{pct} %");
    let r = ui.scope_builder(egui::UiBuilder::new().max_rect(size), |ui| stepper(ui, tr("Taille des sous-titres", "Subtitle size"), &size_txt, pct != 100, Icon::Minus, Icon::Plus)).inner;
    match r {
        Some(0) => {
            p.sub_scale = 1.0;
            p.sub_size(0);
        }
        Some(d) => p.sub_size(d),
        None => {}
    }
    let lift = crate::player::lift_label(p.sub_lift);
    let r = ui.scope_builder(egui::UiBuilder::new().max_rect(height), |ui| stepper(ui, tr("Hauteur des sous-titres", "Subtitle height"), &lift, p.sub_lift != 0, Icon::Down, Icon::Up)).inner;
    match r {
        Some(0) => p.sub_height(-p.sub_lift / 6),
        Some(d) => p.sub_height(d),
        None => {}
    }
}

fn delay(ui: &mut Ui, p: &mut Player, label: &str, prop: &str, step: f64) {
    let v = if prop == "audio-delay" { p.mpv.state.audio_delay } else { p.mpv.state.sub_delay };
    let ms = (v * 1000.0).round() as i64;
    let txt = if ms == 0 { "0 ms".to_string() } else { format!("{ms:+} ms").replace('-', "−") };
    match stepper(ui, label, &txt, ms != 0, Icon::Minus, Icon::Plus) {
        Some(0) => p.mpv.set_f64(prop, 0.0),
        Some(d) => p.mpv.set_f64(prop, v + d as f64 * step),
        None => {}
    }
}

/// Ligne « libellé + valeur » avec deux boutons ronds : renvoie −1, +1, ou 0 si la valeur
/// (cliquable) est remise à zéro.
fn stepper(ui: &mut Ui, label: &str, value: &str, active: bool, less: Icon, more: Icon) -> Option<i32> {
    let full = ui.max_rect();
    let mut out = None;
    // Boutons ronds centrés verticalement sur la ligne, comme le bouton de langue.
    let plus = Rect::from_center_size(pos2(full.max.x - 20.0, full.center().y), vec2(40.0, 40.0));
    let minus = Rect::from_center_size(pos2(full.max.x - 68.0, full.center().y), vec2(40.0, 40.0));
    if round_button(ui, plus, more).clicked() {
        out = Some(1);
    }
    if round_button(ui, minus, less).clicked() {
        out = Some(-1);
    }
    let text = Rect::from_min_max(full.min, pos2(minus.min.x - 8.0, full.max.y));
    ui.scope_builder(egui::UiBuilder::new().max_rect(text), |ui| {
        ui.vertical(|ui| {
            // Bloc de texte (~42 pt) centré sur la ligne, comme les boutons.
            ui.add_space(((full.height() - 42.0) / 2.0).max(0.0));
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(egui::RichText::new(label).font(semi(13.0)).color(INK2));
            let r = ui.add(egui::Label::new(egui::RichText::new(value).font(semi(17.0)).color(if active { ACCENT } else { INK })).sense(Sense::click()));
            if r.clicked() {
                out = Some(0);
            }
        });
    });
    out
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
    big_value(ui, tr("Vitesse", "Speed"), tr("Voix préservée, sans distorsion", "Pitch preserved, no distortion"), &speed_label(s), (s - 1.0).abs() > 0.001);
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
    big_value(ui, tr("Image", "Picture"), tr("Ctrl + molette pour zoomer", "Ctrl + wheel to zoom"), &format!("{z:.1}×"), z > 1.01);
    if let Some(v) = slider(ui, z, 1.0, 4.0, 0.1, &[2.0, 3.0]) {
        p.mpv.set_f64("video-zoom", v.log2());
    }
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        let fill = p.mpv.get("panscan").and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0) > 0.5;
        let unscaled = p.mpv.get("video-unscaled").as_deref() == Some("yes");
        if chip(ui, tr("Image entière", "Fit"), !fill && !unscaled && z <= 1.01).clicked() {
            p.mpv.set_f64("video-zoom", 0.0);
            p.mpv.set_f64("video-pan-x", 0.0);
            p.mpv.set_f64("video-pan-y", 0.0);
            p.mpv.set_f64("panscan", 0.0);
            p.mpv.set("video-unscaled", "no");
        }
        if chip(ui, tr("Remplir l'écran", "Fill screen"), fill).clicked() {
            p.mpv.set_f64("panscan", if fill { 0.0 } else { 1.0 });
        }
        if chip(ui, tr("Taille réelle", "Actual size"), unscaled).clicked() {
            p.mpv.set("video-unscaled", if unscaled { "no" } else { "yes" });
        }
    });
}

fn list_panel(ui: &mut Ui, p: &mut Player, max_height: f32) {
    let label = p.sub_track().map(|t| format!("{} · {}", tr("Sous-titres", "Subtitles"), track_label(t))).unwrap_or(tr("Sous-titres", "Subtitles").into());
    let count = p.sub_index().map(|i| format!("{} {}", i.cues.len(), tr("lignes", "lines"))).unwrap_or_default();
    header(ui, &label, &count);
    let Some(idx) = p.sub_index() else {
        let why = if p.sub_track().is_none() {
            tr("Choisissez une piste de sous-titres.", "Pick a subtitle track.")
        } else if p.indexing {
            tr("Lecture des sous-titres…", "Reading subtitles…")
        } else {
            tr("Liste indisponible pour ce format. Les sauts ligne à ligne passent par mpv.", "No list for this format. Line jumps go through mpv.")
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
    // Bulle brève : elle confirme l'action sans gêner la lecture.
    if age > MESSAGE_LIFE {
        p.message = None;
        return;
    }
    let a = message_alpha(age);
    let top = screen.min.y + 30.0;
    if let Some((value, max)) = p.gauge {
        // Volume : largeur fixe, nom à gauche et valeur à droite (rien ne bouge quand les
        // chiffres défilent), fine jauge dessous. Au-delà de 100 %, la jauge continue dans un
        // second tronçon, en couleur d'accent.
        let (name, number) = text.split_once(' ').unwrap_or((text.as_str(), ""));
        let left = ctx.fonts_mut(|f| f.layout_no_wrap(name.to_string(), semi(15.0), INK));
        let right = ctx.fonts_mut(|f| f.layout_no_wrap(number.to_string(), semi(15.0), INK));
        let rect = Rect::from_min_size(pos2(screen.center().x - 110.0, top), vec2(220.0, 54.0));
        plates.alpha = a;
        plates.add(rect, 18.0, 0.4);
        plates.alpha = 1.0;
        let fade = |c: Color32| c.gamma_multiply(a);
        let painter = ctx.layer_painter(egui::LayerId::new(Order::Tooltip, Id::new("toast")));
        let inner = rect.shrink2(vec2(20.0, 0.0));
        let line = rect.min.y + 13.0;
        painter.galley(pos2(inner.min.x, line), left, fade(INK));
        painter.galley(pos2(inner.max.x - right.size().x, line), right, fade(INK));
        let (h, gap) = (3.0, 3.0);
        let y = rect.max.y - 15.0;
        let max = max.max(100.0) as f32;
        let over = max > 100.5;
        // Tronçon 0-100 %, puis (si mpv va plus haut) tronçon 100 %-maximum.
        let w100 = if over { (inner.width() - gap) * 100.0 / max } else { inner.width() };
        let segment = |x0: f32, w: f32, from: f32, to: f32, color: Color32| {
            let track = Rect::from_min_size(pos2(x0, y), vec2(w, h));
            painter.rect_filled(track, 1.5, fade(Color32::from_white_alpha(46)));
            let f = ((value as f32 - from) / (to - from)).clamp(0.0, 1.0);
            if f > 0.0 {
                painter.rect_filled(Rect::from_min_size(track.min, vec2((w * f).max(h), h)), 1.5, fade(color));
            }
        };
        segment(inner.min.x, w100, 0.0, 100.0, INK);
        if over {
            segment(inner.min.x + w100 + gap, inner.width() - w100 - gap, 100.0, max, ACCENT);
        }
    } else {
        let g = ctx.fonts_mut(|f| f.layout_no_wrap(text.clone(), semi(15.0), INK));
        let size = g.size() + vec2(40.0, 22.0);
        let rect = Rect::from_center_size(pos2(screen.center().x, top + size.y / 2.0), size);
        plates.alpha = a;
        plates.add(rect, size.y / 2.0, 0.4);
        plates.alpha = 1.0;
        egui::Area::new(Id::new("toast")).order(Order::Tooltip).fixed_pos(rect.min).interactable(false).show(ctx, |ui| {
            ui.set_opacity(a);
            ui.painter().galley(rect.center() - g.size() / 2.0, g, INK);
        });
    }
    ctx.request_repaint();
}

fn scrub_osd(ctx: &egui::Context, p: &Player, s: &crate::player::Scrub, plates: &mut Plates, screen: Rect) {
    let sign = if s.steps > 0 { "+" } else if s.steps < 0 { "−" } else { "" };
    let n = s.steps.abs();
    let head = match s.zone {
        Zone::Line if n == 0 => tr("Sous-titres", "Subtitles").to_string(),
        Zone::Line => format!("{sign}{n} {}", if n > 1 { tr("lignes", "lines") } else { tr("ligne", "line") }),
        Zone::Ten if n == 0 => "10 s".into(),
        Zone::Ten => format!("{sign}{} s", n * 10),
        Zone::Minute if n == 0 => "1 min".into(),
        Zone::Minute => format!("{sign}{n} min"),
    };
    let sub = if s.zone == Zone::Line && s.cue.is_none() && p.sub_index().is_none() {
        tr("saut par mpv", "jump via mpv").to_string()
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

const NAV_H: f32 = 44.0;
const NAV_Y: f32 = 20.0;
const SIDE: f32 = 28.0;

/// Accueil : une barre compacte (ouvrir, dossiers, vue, langue) puis les vidéos des dossiers
/// surveillés, en miniatures ou en liste, chacune avec son avancée de lecture.
fn home(ctx: &egui::Context, p: &mut Player, plates: &mut Plates, actions: &mut Vec<Action>, screen: Rect) {
    egui::Area::new(Id::new("home-bg")).order(Order::Background).fixed_pos(screen.min).show(ctx, |ui| {
        ui.allocate_rect(screen, Sense::hover());
    });
    nav_bar(ctx, p, plates, actions, screen);
    let body = Rect::from_min_max(pos2(screen.min.x, screen.min.y + NAV_Y + NAV_H + 14.0), screen.max);
    egui::Area::new(Id::new("home-body")).order(Order::Middle).fixed_pos(body.min).show(ctx, |ui| {
        ui.set_clip_rect(body);
        ui.set_min_size(body.size());
        ui.set_max_size(body.size());
        let lib = &p.library;
        if lib.folders.is_empty() {
            empty(ui, body, tr("Choisissez les dossiers où sont vos vidéos.", "Choose the folders holding your videos."), true, actions);
        } else if lib.videos.is_empty() {
            let msg = if lib.scanning { tr("Recherche des vidéos…", "Looking for videos…") } else { tr("Aucune vidéo dans ces dossiers.", "No video in these folders.") };
            empty(ui, body, msg, false, actions);
        } else {
            let shelf = lib.shelf(&p.place);
            p.place = shelf.place.clone();
            let mut body = body;
            if let Some(up) = lib.up(&shelf.place) {
                // Dossier en cours : bouton pour remonter et son nom.
                let b = Rect::from_min_size(pos2(body.min.x + SIDE, body.min.y), vec2(38.0, 38.0));
                ui.painter().circle_filled(b.center(), 19.0, Color32::from_white_alpha(22));
                ui.painter().circle_stroke(b.center(), 19.0, Stroke::new(1.0, Color32::from_white_alpha(50)));
                if icon_button(ui, b, Icon::Back).on_hover_text(tr("Remonter (Échap)", "Up (Esc)")).clicked() {
                    p.place = up;
                }
                ui.painter().text(pos2(b.max.x + 12.0, b.center().y), Align2::LEFT_CENTER, &shelf.title, semi(18.0), INK);
                body.min.y += 50.0;
            }
            if p.library.grid {
                grid(ui, p, &shelf, body, actions);
            } else {
                list(ui, p, &shelf, body, actions);
            }
        }
    });
    if p.folders_open {
        folders_sheet(ctx, p, plates, actions, screen);
    }
    if std::env::var_os("BIOPLAY_FLAGS").is_some() {
        flag_gallery(ctx, screen);
    }
}

fn nav_bar(ctx: &egui::Context, p: &mut Player, plates: &mut Plates, actions: &mut Vec<Action>, screen: Rect) {
    egui::Area::new(Id::new("home-nav")).order(Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let y = screen.min.y + NAV_Y;
        let open = Rect::from_min_size(pos2(screen.min.x + SIDE, y), vec2(NAV_H, NAV_H));
        plates.add(open, NAV_H / 2.0, 0.42);
        if icon_button(ui, open, Icon::Open).on_hover_text(tr("Ouvrir une vidéo", "Open a video")).clicked() {
            actions.push(Action::Open);
        }
        let label = tr("Dossiers", "Folders");
        let fr = Rect::from_min_size(pos2(open.max.x + 10.0, y), vec2(pill_width(ui, label, true) + 6.0, NAV_H));
        plates.add(fr, NAV_H / 2.0, 0.42);
        if pill(ui, fr, Some(Icon::Folder), label, INK).clicked() {
            p.folders_open = !p.folders_open;
        }
        // À droite : langue (drapeaux seuls), et juste avant, la bascule liste / miniatures.
        let flags = Rect::from_min_size(pos2(screen.max.x - SIDE - 92.0, y), vec2(92.0, NAV_H));
        plates.add(flags, NAV_H / 2.0, 0.42);
        flag_switch(ui, flags);
        let view = Rect::from_min_size(pos2(flags.min.x - 10.0 - NAV_H, y), vec2(NAV_H, NAV_H));
        plates.add(view, NAV_H / 2.0, 0.42);
        let (i, tip) = if p.library.grid { (Icon::Rows, tr("Afficher en liste", "Show as list")) } else { (Icon::Grid, tr("Afficher en miniatures", "Show as thumbnails")) };
        if icon_button(ui, view, i).on_hover_text(tip).clicked() {
            p.library.toggle_view();
        }
    });
}

/// Langue de l'interface : deux drapeaux, l'actif éclairé, l'autre estompé.
fn flag_switch(ui: &mut Ui, r: Rect) {
    let painter = ui.painter().clone();
    let half = r.width() / 2.0;
    for (i, french) in [true, false].into_iter().enumerate() {
        let seg = Rect::from_min_size(pos2(r.min.x + i as f32 * half, r.min.y), vec2(half, r.height()));
        let selected = en() != french;
        let resp = ui.interact(seg, ui.id().with(("flag", i)), Sense::click());
        let pad = seg.shrink2(vec2(if i == 0 { 5.0 } else { 0.0 }, 5.0));
        let pad = if i == 0 { pad } else { pad.with_max_x(seg.max.x - 5.0) };
        if selected {
            painter.rect_filled(pad, CornerRadius::same(17), ACCENT.gamma_multiply(0.22));
        } else if resp.hovered() {
            painter.rect_filled(pad, CornerRadius::same(17), Color32::from_white_alpha(18));
        }
        let flag = Rect::from_center_size(pad.center(), vec2(24.0, 16.0));
        crate::flags::draw(&painter, flag, if french { "fr" } else { "en" }, if selected { 1.0 } else { 0.5 });
        if resp.clicked() && !selected {
            crate::i18n::toggle();
        }
    }
}

fn empty(ui: &mut Ui, body: Rect, message: &str, add: bool, actions: &mut Vec<Action>) {
    let c = pos2(body.center().x, body.min.y + body.height() * 0.42);
    ui.painter().text(c, Align2::CENTER_CENTER, message, med(16.0), INK2);
    if add {
        let label = tr("Ajouter un dossier", "Add a folder");
        let r = Rect::from_center_size(c + vec2(0.0, 52.0), vec2(pill_width(ui, label, true) + 8.0, 46.0));
        ui.painter().rect_filled(r, CornerRadius::same(23), Color32::from_white_alpha(26));
        ui.painter().rect_stroke(r, CornerRadius::same(23), Stroke::new(1.0, Color32::from_white_alpha(60)), StrokeKind::Inside);
        if pill(ui, r, Some(Icon::Plus), label, INK).clicked() {
            actions.push(Action::AddFolder);
        }
    }
}

/// Position mémorisée d'une vidéo (secondes), d'après les dernières lectures.
fn position(p: &Player, path: &str) -> f64 {
    p.recents.iter().find(|r| r.path == path).map_or(0.0, |r| r.position)
}

/// Miniature à coins arrondis, durée en haut à droite et avancée de lecture en bas.
fn thumbnail(ui: &mut Ui, p: &mut Player, v: &crate::library::Video, r: Rect, hovered: bool) {
    let cr = CornerRadius::same(12);
    let painter = ui.painter().clone();
    match p.library.image(v) {
        // L'image elle-même est posée par mpv sous l'interface ; ici, seulement ce qui la couvre.
        Some(image) => p.thumbs.push(crate::overlay::Thumb { key: v.thumb(), rect: r, clip: ui.clip_rect(), radius: 12.0, image }),
        None => {
            painter.rect_filled(r, cr, Color32::from_white_alpha(14));
            icon(&painter, r.center(), 26.0, Icon::Play, FAINT);
        }
    }
    if hovered {
        painter.rect_filled(r, cr, Color32::from_white_alpha(22));
    }
    painter.rect_stroke(r, cr, Stroke::new(1.0, Color32::from_white_alpha(if hovered { 90 } else { 34 })), StrokeKind::Inside);
    if v.duration > 0.0 {
        let g = painter.layout_no_wrap(clock(v.duration), semi(11.0), INK);
        let b = Rect::from_min_size(pos2(r.max.x - 8.0 - g.size().x - 12.0, r.min.y + 7.0), g.size() + vec2(12.0, 3.0));
        painter.rect_filled(b, CornerRadius::same((b.height() / 2.0) as u8), Color32::from_black_alpha(150));
        painter.galley(b.min + vec2(6.0, 1.5), g, INK);
        // Avancée : un rail discret sur chaque miniature, rempli jusqu'à la position.
        let f = (position(p, &v.path) / v.duration).clamp(0.0, 1.0) as f32;
        let rail = Rect::from_min_size(pos2(r.min.x + 9.0, r.max.y - 11.0), vec2(r.width() - 18.0, 4.0));
        painter.rect_filled(rail, CornerRadius::same(2), Color32::from_black_alpha(110));
        painter.rect_filled(rail, CornerRadius::same(2), Color32::from_white_alpha(52));
        if f > 0.0 {
            painter.rect_filled(Rect::from_min_size(rail.min, vec2((rail.width() * f).max(4.0), 4.0)), CornerRadius::same(2), ACCENT);
        }
    }
}

/// Titre sur deux lignes au plus, coupé proprement.
fn title_galley(ui: &Ui, text: String, width: f32, size: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text, semi(size), INK, width);
    job.wrap = egui::text::TextWrapping { max_width: width, max_rows: 2, break_anywhere: false, overflow_character: Some('…') };
    ui.painter().layout_job(job)
}

/// « 12 vidéos », « 3 dossiers · 12 vidéos ».
fn dir_count(d: &crate::library::Dir) -> String {
    let v = format!("{} {}", d.videos, if d.videos > 1 { tr("vidéos", "videos") } else { tr("vidéo", "video") });
    if d.dirs > 0 {
        format!("{} {} · {v}", d.dirs, if d.dirs > 1 { tr("dossiers", "folders") } else { tr("dossier", "folder") })
    } else {
        v
    }
}

/// Dossier dessiné : le dos et son onglet, deux feuilles qui dépassent, la face avant bleutée avec
/// un reflet. Dans le même cadre 16:9 que les miniatures.
pub(crate) fn folder_art(painter: &egui::Painter, r: Rect, hovered: bool) {
    let h = r.height() * 0.86;
    let w = (r.width() * 0.86).min(h * 1.42);
    let x0 = r.center().x - w / 2.0;
    let y0 = r.center().y - h / 2.0 + r.height() * 0.02;
    let cr = CornerRadius::same((h * 0.07) as u8);
    let lift = if hovered { -3.0 } else { 0.0 };
    let gradient = |rect: Rect, top: Color32, bottom: Color32| {
        // Dégradé vertical continu aux coins arrondis : deux bandeaux arrondis aux extrémités
        // (couleurs de bout) et, entre eux, un quadrilatère aux couleurs interpolées.
        let rr = cr.nw as f32;
        painter.rect_filled(Rect::from_min_max(rect.min, pos2(rect.max.x, rect.min.y + rr * 2.0)), cr, top);
        painter.rect_filled(Rect::from_min_max(pos2(rect.min.x, rect.max.y - rr * 2.0), rect.max), cr, bottom);
        let (y1, y2) = (rect.min.y + rr, rect.max.y - rr);
        let k = |y: f32| lerp_color(top, bottom, ((y - y1) / (y2 - y1).max(1.0)).clamp(0.0, 1.0));
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(pos2(rect.min.x, y1), k(y1));
        mesh.colored_vertex(pos2(rect.max.x, y1), k(y1));
        mesh.colored_vertex(pos2(rect.max.x, y2), k(y2));
        mesh.colored_vertex(pos2(rect.min.x, y2), k(y2));
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        painter.add(Shape::mesh(mesh));
    };
    let back_top = Color32::from_rgb(0x2B, 0x6D, 0xB3);
    let back_bottom = Color32::from_rgb(0x17, 0x3F, 0x70);
    painter.rect_filled(Rect::from_min_size(pos2(x0, y0), vec2(w * 0.38, h * 0.22)), cr, back_top);
    gradient(Rect::from_min_size(pos2(x0, y0 + h * 0.1), vec2(w, h * 0.9)), back_top, back_bottom);
    let sheet = CornerRadius::same((h * 0.04) as u8);
    painter.rect_filled(Rect::from_min_size(pos2(x0 + w * 0.08, y0 + h * 0.17 + lift), vec2(w * 0.8, h * 0.5)), sheet, Color32::from_rgb(0xCF, 0xDD, 0xEB));
    painter.rect_filled(Rect::from_min_size(pos2(x0 + w * 0.13, y0 + h * 0.23 + lift * 1.6), vec2(w * 0.76, h * 0.5)), sheet, Color32::from_rgb(0xF1, 0xF6, 0xFB));
    let fy = y0 + h * 0.33;
    let front = Rect::from_min_max(pos2(x0, fy), pos2(x0 + w, y0 + h));
    gradient(front, Color32::from_rgb(0x8F, 0xD8, 0xFF), Color32::from_rgb(0x2F, 0x86, 0xCF));
    painter.line_segment([pos2(x0 + cr.nw as f32, fy + 1.0), pos2(x0 + w - cr.ne as f32, fy + 1.0)], Stroke::new(1.5, Color32::from_white_alpha(170)));
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Un élément de l'endroit affiché : dossier (indice dans `shelf.dirs`) ou vidéo (dans la bibliothèque).
#[derive(Clone, Copy)]
enum Item {
    Dir(usize),
    Video(usize),
}

fn items(shelf: &crate::library::Shelf) -> Vec<Item> {
    (0..shelf.dirs.len()).map(Item::Dir).chain(shelf.videos.iter().map(|&i| Item::Video(i))).collect()
}

fn grid(ui: &mut Ui, p: &mut Player, shelf: &crate::library::Shelf, body: Rect, actions: &mut Vec<Action>) {
    let gap = 16.0;
    let inner = body.width() - SIDE * 2.0;
    let cols = ((inner + gap) / (230.0 + gap)).floor().max(1.0) as usize;
    let w = (inner - gap * (cols - 1) as f32) / cols as f32;
    let th = w * 9.0 / 16.0;
    let row_h = th + 50.0;
    let all = items(shelf);
    let rows = all.len().div_ceil(cols);
    // Un défilement par dossier : on retrouve sa place en remontant.
    egui::ScrollArea::vertical().id_salt(("grid", &shelf.place)).auto_shrink([false, false]).show_rows(ui, row_h, rows, |ui, range| {
        for row in range {
            let (line, _) = ui.allocate_exact_size(vec2(body.width(), row_h), Sense::hover());
            for c in 0..cols {
                let Some(&item) = all.get(row * cols + c) else { break };
                let cell = Rect::from_min_size(pos2(line.min.x + SIDE + c as f32 * (w + gap), line.min.y), vec2(w, row_h - 6.0));
                let art = Rect::from_min_size(cell.min, vec2(w, th));
                match item {
                    Item::Dir(i) => {
                        let d = &shelf.dirs[i];
                        let resp = ui.interact(cell, ui.id().with(("dir", &d.place)), Sense::click());
                        folder_art(ui.painter(), art, resp.hovered());
                        let clip = ui.painter().with_clip_rect(cell);
                        clip.text(pos2(cell.min.x + 2.0, cell.min.y + th + 15.0), Align2::LEFT_CENTER, &d.name, semi(13.5), INK);
                        clip.text(pos2(cell.min.x + 2.0, cell.min.y + th + 33.0), Align2::LEFT_CENTER, dir_count(d), med(11.5), INK2);
                        if resp.clicked() {
                            p.place = d.place.clone();
                        }
                    }
                    Item::Video(i) => {
                        let v = p.library.videos[i].clone();
                        let resp = ui.interact(cell, ui.id().with(("tile", &v.path)), Sense::click());
                        thumbnail(ui, p, &v, art, resp.hovered());
                        let g = title_galley(ui, v.title(), w - 4.0, 13.5);
                        ui.painter().galley(pos2(cell.min.x + 2.0, cell.min.y + th + 7.0), g, INK);
                        if resp.clicked() {
                            actions.push(Action::Play(v.path.clone()));
                        }
                    }
                }
            }
        }
        ui.add_space(20.0);
    });
}

fn list(ui: &mut Ui, p: &mut Player, shelf: &crate::library::Shelf, body: Rect, actions: &mut Vec<Action>) {
    let row_h = 86.0;
    let all = items(shelf);
    let width = (body.width() - SIDE * 2.0).min(980.0);
    let x0 = body.center().x - width / 2.0;
    egui::ScrollArea::vertical().id_salt(("list", &shelf.place)).auto_shrink([false, false]).show_rows(ui, row_h, all.len(), |ui, range| {
        for i in range {
            let (line, _) = ui.allocate_exact_size(vec2(body.width(), row_h), Sense::hover());
            let row = Rect::from_min_size(pos2(x0, line.min.y + 3.0), vec2(width, row_h - 6.0));
            let t = Rect::from_min_size(pos2(row.min.x + 6.0, row.min.y + 4.0), vec2(128.0, 72.0));
            let tx = t.max.x + 16.0;
            match all[i] {
                Item::Dir(k) => {
                    let d = &shelf.dirs[k];
                    let resp = ui.interact(row, ui.id().with(("dirrow", &d.place)), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(row, CornerRadius::same(16), Color32::from_white_alpha(14));
                    }
                    folder_art(ui.painter(), t, resp.hovered());
                    ui.painter().text(pos2(tx, row.center().y - 10.0), Align2::LEFT_CENTER, &d.name, semi(15.0), INK);
                    ui.painter().text(pos2(tx, row.center().y + 12.0), Align2::LEFT_CENTER, dir_count(d), med(12.0), INK2);
                    if resp.clicked() {
                        p.place = d.place.clone();
                    }
                }
                Item::Video(k) => {
                    let v = p.library.videos[k].clone();
                    let resp = ui.interact(row, ui.id().with(("row", &v.path)), Sense::click());
                    if resp.hovered() {
                        ui.painter().rect_filled(row, CornerRadius::same(16), Color32::from_white_alpha(14));
                    }
                    thumbnail(ui, p, &v, t, false);
                    let g = title_galley(ui, v.title(), row.max.x - tx - 12.0, 15.0);
                    let gh = g.size().y;
                    let top = row.center().y - (gh + 20.0) / 2.0;
                    ui.painter().galley(pos2(tx, top), g, INK);
                    let pos = position(p, &v.path);
                    let detail = if v.duration <= 0.0 {
                        String::new()
                    } else if pos > 0.0 {
                        format!("{} {}", tr("Reste", "Left"), clock(v.duration - pos))
                    } else {
                        clock(v.duration)
                    };
                    ui.painter().text(pos2(tx, top + gh + 4.0), Align2::LEFT_TOP, detail, med(12.0), INK2);
                    if resp.clicked() {
                        actions.push(Action::Play(v.path.clone()));
                    }
                }
            }
        }
        ui.add_space(20.0);
    });
}

fn folders_sheet(ctx: &egui::Context, p: &mut Player, plates: &mut Plates, actions: &mut Vec<Action>, screen: Rect) {
    // Un clic à côté ferme la fenêtre.
    egui::Area::new(Id::new("folders-scrim")).order(Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        if ui.allocate_rect(screen, Sense::click()).clicked() {
            p.folders_open = false;
        }
    });
    let w = 520.0f32.min(screen.width() - 32.0);
    let id = Id::new("folders");
    let h: f32 = ctx.data(|d| d.get_temp(id)).unwrap_or(220.0);
    let rect = Rect::from_min_size(pos2(screen.min.x + SIDE, screen.min.y + NAV_Y + NAV_H + 10.0), vec2(w, h));
    plates.add(rect, 26.0, 0.62);
    let mut remove = None;
    let out = egui::Area::new(id).order(Order::Tooltip).fixed_pos(rect.min).show(ctx, |ui| {
        ui.set_width(w);
        egui::Frame::NONE.inner_margin(20.0).show(ui, |ui| {
            ui.set_width(w - 40.0);
            let (head, _) = ui.allocate_exact_size(vec2(w - 40.0, 30.0), Sense::hover());
            ui.painter().text(head.left_center(), Align2::LEFT_CENTER, tr("Dossiers surveillés", "Watched folders"), semi(17.0), INK);
            if icon_button_tint(ui, Rect::from_center_size(pos2(head.max.x - 14.0, head.center().y), vec2(30.0, 30.0)), Icon::Close, INK2).clicked() {
                p.folders_open = false;
            }
            ui.label(egui::RichText::new(tr("Leurs vidéos, sous-dossiers compris, s'affichent sur l'accueil.", "Their videos, subfolders included, are shown on the home screen.")).font(med(12.5)).color(INK2));
            ui.add_space(10.0);
            for f in p.library.folders.clone() {
                let (row, _) = ui.allocate_exact_size(vec2(w - 40.0, 48.0), Sense::hover());
                icon(ui.painter(), pos2(row.min.x + 12.0, row.center().y), 20.0, Icon::Folder, ACCENT);
                let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.to_string_lossy().into_owned());
                let clip = ui.painter().with_clip_rect(row.with_max_x(row.max.x - 44.0));
                clip.text(pos2(row.min.x + 34.0, row.min.y + 15.0), Align2::LEFT_CENTER, name, semi(15.0), INK);
                let n = p.library.count(&f);
                let home = std::env::var("HOME").unwrap_or_default();
                let shown = f.to_string_lossy().replacen(&home, "~", 1);
                clip.text(pos2(row.min.x + 34.0, row.min.y + 33.0), Align2::LEFT_CENTER, format!("{shown} · {n} {}", tr("vidéos", "videos")), med(12.0), INK2);
                if icon_button_tint(ui, Rect::from_center_size(pos2(row.max.x - 18.0, row.center().y), vec2(32.0, 32.0)), Icon::Close, FAINT).clicked() {
                    remove = Some(f.clone());
                }
            }
            ui.add_space(10.0);
            let label = tr("Ajouter un dossier", "Add a folder");
            let (r, _) = ui.allocate_exact_size(vec2(pill_width(ui, label, true) + 8.0, 44.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(22), Color32::from_white_alpha(26));
            ui.painter().rect_stroke(r, CornerRadius::same(22), Stroke::new(1.0, Color32::from_white_alpha(60)), StrokeKind::Inside);
            if pill(ui, r, Some(Icon::Plus), label, INK).clicked() {
                actions.push(Action::AddFolder);
            }
        });
    });
    if let Some(f) = remove {
        p.library.remove_folder(&f);
    }
    let measured = out.response.rect.height();
    if (measured - h).abs() > 0.5 {
        ctx.data_mut(|d| d.insert_temp(id, measured));
        ctx.request_repaint();
    }
}

/// Galerie de contrôle (`BIOPLAY_FLAGS=1`) : tous les drapeaux, en grand et à la taille de la barre.
fn flag_gallery(ctx: &egui::Context, screen: Rect) {
    egui::Area::new(Id::new("flag-gallery")).order(Order::Tooltip).fixed_pos(pos2(screen.min.x + SIDE, screen.max.y - 260.0)).interactable(false).show(ctx, |ui| {
        let flags = crate::flags::all();
        let per_row = 12;
        let rows = flags.len().div_ceil(per_row);
        let (r, _) = ui.allocate_exact_size(vec2(per_row as f32 * 40.0, rows as f32 * 30.0 + 22.0), Sense::hover());
        for (i, f) in flags.iter().enumerate() {
            let o = pos2(r.min.x + (i % per_row) as f32 * 40.0, r.min.y + (i / per_row) as f32 * 30.0);
            crate::flags::draw(ui.painter(), Rect::from_min_size(o, vec2(33.0, 22.0)), &f.key, 1.0);
        }
        for (i, f) in flags.iter().take(per_row).enumerate() {
            crate::flags::draw(ui.painter(), Rect::from_min_size(pos2(r.min.x + i as f32 * 40.0, r.max.y - 14.0), vec2(21.0, 14.0)), &f.key, 1.0);
        }
    });
}
