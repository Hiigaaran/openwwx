//! Drawing cards. A card with art is drawn as its scanned image; otherwise it
//! gets a Raw Deal style frame (title, picture, type line, text box and stats)
//! whose picture is a vector drawing of the card's type.

use eframe::egui::{
    self, Color32, Context, CornerRadius, FontId, Id, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind,
    TextureHandle, Ui, Vec2, epaint::RectShape, pos2, vec2,
};
use rawdeal_core::{CardDef, CardKind, Effect, ManeuverType, SuperstarDef, Who};

use crate::{art, text};

pub const SIZE: Vec2 = vec2(112.0, 156.0);
pub const SMALL: Vec2 = vec2(84.0, 117.0);
pub const PILE: Vec2 = vec2(60.0, 84.0);
pub const PREVIEW: Vec2 = vec2(240.0, 334.0);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Normal,
    /// Can be clicked now.
    Playable,
    Selected,
}

/// The card last under the mouse, shown large in the preview panel.
#[derive(Clone)]
pub enum Preview {
    Card(Box<CardDef>),
    Superstar(Box<SuperstarDef>),
}

fn preview_id() -> Id {
    Id::new("openwwx-preview")
}

pub fn set_preview(ctx: &Context, preview: Preview) {
    ctx.data_mut(|d| d.insert_temp(preview_id(), preview));
}

pub fn preview(ctx: &Context) -> Option<Preview> {
    ctx.data(|d| d.get_temp(preview_id()))
}

fn color(kind: CardKind) -> Color32 {
    match kind {
        CardKind::Maneuver(_) => Color32::from_rgb(196, 152, 52),
        CardKind::Action => Color32::from_rgb(62, 104, 172),
        CardKind::Reversal => Color32::from_rgb(166, 56, 56),
        CardKind::PreMatch(_) => Color32::from_rgb(120, 72, 150),
        CardKind::Enforcer | CardKind::Backstage => Color32::from_rgb(60, 140, 110),
        CardKind::Antic => Color32::from_rgb(70, 150, 70),
    }
}

const STAR_COLOR: Color32 = Color32::from_rgb(150, 156, 170);

/// Draws a card face (or its back, if `def` is hidden) and returns the click response.
pub fn card(ui: &mut Ui, def: Option<&CardDef>, size: Vec2, look: Look) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let lift = if look == Look::Playable && response.hovered() { -4.0 } else { 0.0 };
    paint(&ui.painter_at(rect.expand(6.0)), rect.translate(vec2(0.0, lift)), def, look, 1.0);
    if let (Some(def), true) = (def, response.hovered()) {
        set_preview(ui.ctx(), Preview::Card(Box::new(def.clone())));
    }
    response
}

/// A Superstar card, with its label underneath like the piles.
pub fn superstar(ui: &mut Ui, star: &SuperstarDef) -> Response {
    let (rect, response) = ui.allocate_exact_size(PILE + vec2(0.0, 16.0), Sense::hover());
    let painter = ui.painter_at(rect.expand(4.0));
    paint_superstar(&painter, Rect::from_min_size(rect.min, PILE), star, 1.0);
    painter.text(
        Pos2::new(rect.center().x, rect.bottom()),
        egui::Align2::CENTER_BOTTOM,
        "Superstar",
        FontId::proportional(11.0),
        Color32::from_gray(200),
    );
    if response.hovered() {
        set_preview(ui.ctx(), Preview::Superstar(Box::new(star.clone())));
    }
    response
}

fn image(painter: &Painter, rect: Rect, texture: &TextureHandle, radius: CornerRadius, opacity: f32) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    let tint = Color32::WHITE.gamma_multiply(opacity);
    painter.add(RectShape::filled(rect, radius, tint).with_texture(texture.id(), uv));
}

fn outline(painter: &Painter, rect: Rect, look: Look, opacity: f32) {
    let stroke = match look {
        Look::Normal => Stroke::new(1.0, Color32::from_gray(90)),
        Look::Playable => Stroke::new(2.5, Color32::from_rgb(90, 220, 120)),
        Look::Selected => Stroke::new(3.0, Color32::from_rgb(255, 210, 60)),
    };
    let stroke = Stroke::new(stroke.width, stroke.color.gamma_multiply(opacity));
    painter.rect_stroke(rect, CornerRadius::same(6), stroke, StrokeKind::Inside);
}

/// Paints a card in `rect`, at `opacity` (0 to 1).
pub fn paint(painter: &Painter, rect: Rect, def: Option<&CardDef>, look: Look, opacity: f32) {
    let radius = CornerRadius::same(6);
    let scale = rect.width() / SIZE.x;
    let fade = |c: Color32| c.gamma_multiply(opacity);

    let Some(def) = def else {
        painter.rect_filled(rect, radius, fade(Color32::from_rgb(38, 38, 46)));
        painter.rect_stroke(
            rect.shrink(4.0 * scale),
            radius,
            Stroke::new(1.5, fade(Color32::from_rgb(150, 30, 30))),
            StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "RAW\nDEAL",
            FontId::proportional(16.0 * scale),
            fade(Color32::from_rgb(200, 200, 210)),
        );
        return;
    };

    if let Some(texture) = art::card(painter.ctx(), def) {
        image(painter, rect, &texture, radius, opacity);
        outline(painter, rect, look, opacity);
        return;
    }

    // Opaque base so cards flying over the board do not show through.
    painter.rect_filled(rect, radius, fade(Color32::from_rgb(20, 20, 22)));
    let frame = color(def.kinds[0]);
    painter.rect_filled(rect.shrink(2.0 * scale), radius, fade(frame.gamma_multiply(0.55)));
    // Hybrids get both colors on the right half of the frame.
    if def.kinds.len() > 1 {
        let half = Rect::from_min_max(pos2(rect.center().x, rect.top()), rect.max).shrink(2.0 * scale);
        painter.rect_filled(half, CornerRadius::ZERO, fade(color(def.kinds[1]).gamma_multiply(0.55)));
    }

    let pad = 5.0 * scale;
    let inner = rect.shrink(pad);

    // Title bar.
    let title_bar = Rect::from_min_size(inner.min, vec2(inner.width(), 15.0 * scale));
    painter.rect_filled(title_bar, CornerRadius::same(3), fade(Color32::from_rgb(16, 16, 18)));
    fit_text(painter, title_bar.shrink2(vec2(3.0 * scale, 0.0)), &def.title, 10.0 * scale, fade(Color32::WHITE));

    // Picture, with Fortitude and Damage in its top corners.
    let picture =
        Rect::from_min_size(pos2(inner.left(), title_bar.bottom() + 2.0 * scale), vec2(inner.width(), 50.0 * scale));
    painter.rect_filled(picture, CornerRadius::same(3), fade(Color32::from_rgb(30, 30, 34)));
    let parts = def.kinds.len().max(1) as f32;
    for (i, &k) in def.kinds.iter().enumerate() {
        let w = picture.width() / parts;
        let part = Rect::from_min_size(pos2(picture.left() + w * i as f32, picture.top()), vec2(w, picture.height()));
        painter.rect_filled(part.shrink(1.0), CornerRadius::same(3), fade(color(k).gamma_multiply(0.25)));
        draw_icon(&painter.with_clip_rect(part), part, Icon::of(k), opacity);
    }
    for (i, chip) in chips(def).iter().take(2).enumerate() {
        let y = picture.bottom() - (3.0 + 10.0 * (i as f32 + 1.0)) * scale;
        badge(
            painter,
            pos2(picture.left() + 3.0 * scale, y),
            chip,
            6.5 * scale,
            Color32::from_rgb(235, 235, 240),
            opacity,
        );
    }
    stat(
        painter,
        pos2(picture.left() + 2.0 * scale, picture.top() + 2.0 * scale),
        "F",
        def.fortitude,
        scale,
        opacity,
        false,
    );
    stat(
        painter,
        pos2(picture.right() - 2.0 * scale, picture.top() + 2.0 * scale),
        "D",
        def.damage,
        scale,
        opacity,
        true,
    );

    // Type line.
    let kinds: Vec<String> = def.kinds.iter().map(|&k| text::kind(k)).collect();
    let type_line =
        Rect::from_min_size(pos2(inner.left(), picture.bottom() + 2.0 * scale), vec2(inner.width(), 10.0 * scale));
    painter.rect_filled(type_line, CornerRadius::same(2), fade(Color32::from_rgb(16, 16, 18)));
    fit_text(
        painter,
        type_line.shrink2(vec2(3.0 * scale, 0.0)),
        &kinds.join(" / "),
        7.5 * scale,
        fade(Color32::from_gray(220)),
    );

    // Text box and the bottom line with Stun Value and traits.
    let bottom_h = 10.0 * scale;
    let text_box = Rect::from_min_max(
        pos2(inner.left(), type_line.bottom() + 2.0 * scale),
        pos2(inner.right(), inner.bottom() - bottom_h),
    );
    painter.rect_filled(text_box, CornerRadius::same(2), fade(Color32::from_rgb(232, 226, 210)));
    if !def.text.is_empty() {
        let body = painter.layout(
            def.text.clone(),
            FontId::proportional(7.0 * scale),
            fade(Color32::from_rgb(25, 25, 25)),
            text_box.width() - 4.0 * scale,
        );
        painter.with_clip_rect(text_box.shrink(1.0)).galley(
            text_box.min + vec2(2.0, 1.5) * scale,
            body,
            Color32::BLACK,
        );
    }
    let bottom = Rect::from_min_max(pos2(inner.left(), inner.bottom() - bottom_h + 1.0 * scale), inner.max);
    painter.text(
        bottom.left_center(),
        egui::Align2::LEFT_CENTER,
        format!("SV {}", def.stun_value),
        FontId::monospace(7.5 * scale),
        fade(Color32::WHITE),
    );
    let traits: Vec<String> = def.traits.iter().map(|t| format!("{t:?}")).take(2).collect();
    if !traits.is_empty() {
        painter.text(
            bottom.right_center(),
            egui::Align2::RIGHT_CENTER,
            traits.join(" · "),
            FontId::proportional(7.0 * scale),
            fade(Color32::from_gray(225)),
        );
    }
    outline(painter, rect, look, opacity);
}

/// Paints a Superstar card: name, picture, ability text, Hand Size and Superstar Value.
pub fn paint_superstar(painter: &Painter, rect: Rect, star: &SuperstarDef, opacity: f32) {
    let radius = CornerRadius::same(6);
    let scale = rect.width() / SIZE.x;
    let fade = |c: Color32| c.gamma_multiply(opacity);
    if let Some(texture) = art::superstar(painter.ctx(), star) {
        image(painter, rect, &texture, radius, opacity);
        outline(painter, rect, Look::Normal, opacity);
        return;
    }
    painter.rect_filled(rect, radius, fade(Color32::from_rgb(20, 20, 22)));
    painter.rect_filled(rect.shrink(2.0 * scale), radius, fade(STAR_COLOR.gamma_multiply(0.45)));
    let inner = rect.shrink(5.0 * scale);
    let title_bar = Rect::from_min_size(inner.min, vec2(inner.width(), 15.0 * scale));
    painter.rect_filled(title_bar, CornerRadius::same(3), fade(Color32::from_rgb(16, 16, 18)));
    fit_text(painter, title_bar.shrink2(vec2(3.0 * scale, 0.0)), &star.name, 10.0 * scale, fade(Color32::WHITE));

    let picture =
        Rect::from_min_size(pos2(inner.left(), title_bar.bottom() + 2.0 * scale), vec2(inner.width(), 70.0 * scale));
    painter.rect_filled(picture, CornerRadius::same(3), fade(Color32::from_rgb(34, 36, 44)));
    draw_icon(&painter.with_clip_rect(picture), picture, Icon::Superstar, opacity);

    let stats_h = 14.0 * scale;
    let text_box = Rect::from_min_max(
        pos2(inner.left(), picture.bottom() + 2.0 * scale),
        pos2(inner.right(), inner.bottom() - stats_h),
    );
    painter.rect_filled(text_box, CornerRadius::same(2), fade(Color32::from_rgb(232, 226, 210)));
    let body = painter.layout(
        star.text.clone(),
        FontId::proportional(6.5 * scale),
        fade(Color32::from_rgb(25, 25, 25)),
        text_box.width() - 4.0 * scale,
    );
    painter.with_clip_rect(text_box.shrink(1.0)).galley(text_box.min + vec2(2.0, 1.5) * scale, body, Color32::BLACK);
    let y = inner.bottom() - stats_h / 2.0 + 1.0 * scale;
    badge(
        painter,
        pos2(inner.left(), y - 5.0 * scale),
        &format!("Mano {}", star.hand_size),
        7.5 * scale,
        Color32::WHITE,
        opacity,
    );
    let sv = format!("SV {}", star.superstar_value);
    let w =
        8.0 * scale + painter.layout_no_wrap(sv.clone(), FontId::proportional(7.5 * scale), Color32::WHITE).size().x;
    badge(painter, pos2(inner.right() - w, y - 5.0 * scale), &sv, 7.5 * scale, Color32::WHITE, opacity);
    outline(painter, rect, Look::Normal, opacity);
}

/// Single-line text, shrunk to fit `rect`'s width.
fn fit_text(painter: &Painter, rect: Rect, text: &str, size: f32, color: Color32) {
    let mut size = size;
    let mut galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(size), color);
    if galley.size().x > rect.width() && galley.size().x > 0.0 {
        size = (size * rect.width() / galley.size().x).max(size * 0.6);
        galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(size), color);
    }
    let pos = pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
    painter.with_clip_rect(rect).galley(pos, galley, color);
}

/// A Fortitude or Damage value in a dark circle.
fn stat(painter: &Painter, corner: Pos2, label: &str, value: u8, scale: f32, opacity: f32, right: bool) {
    let r = 9.0 * scale;
    let center = if right { pos2(corner.x - r, corner.y + r) } else { pos2(corner.x + r, corner.y + r) };
    painter.circle(
        center,
        r,
        Color32::from_rgba_unmultiplied(12, 12, 14, 230).gamma_multiply(opacity),
        Stroke::new(1.0 * scale, Color32::from_gray(200).gamma_multiply(opacity)),
    );
    painter.text(
        center - vec2(0.0, 2.5 * scale),
        egui::Align2::CENTER_CENTER,
        value.to_string(),
        FontId::proportional(9.0 * scale),
        Color32::WHITE.gamma_multiply(opacity),
    );
    painter.text(
        center + vec2(0.0, 4.5 * scale),
        egui::Align2::CENTER_CENTER,
        label,
        FontId::proportional(5.0 * scale),
        Color32::from_gray(200).gamma_multiply(opacity),
    );
}

/// A small dark label with rounded corners.
fn badge(painter: &Painter, at: Pos2, text: &str, size: f32, color: Color32, opacity: f32) {
    let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(size), color.gamma_multiply(opacity));
    let pad = vec2(size * 0.5, size * 0.15);
    let rect = Rect::from_min_size(at, galley.size() + pad * 2.0);
    painter.rect_filled(
        rect,
        CornerRadius::same(3),
        Color32::from_rgba_unmultiplied(10, 10, 12, 210).gamma_multiply(opacity),
    );
    painter.galley(at + pad, galley, color);
}

/// Short labels for what a card does, shown over its picture.
fn chips(def: &CardDef) -> Vec<String> {
    let mut out = Vec::new();
    let mut effects: Vec<&Effect> = def.script.on_play.iter().collect();
    if let Some(a) = &def.script.as_action {
        effects.extend(a.on_play.iter());
    }
    while let Some(e) = effects.pop() {
        let chip = match e {
            Effect::Draw { who: Who::You, n } | Effect::DrawUpTo { who: Who::You, n } => format!("Robas {n}"),
            Effect::Discard { who: Who::Opponent, n, .. } => format!("Rival descarta {n}"),
            Effect::Damage { who: Who::Opponent, n } => format!("+{n} daño"),
            Effect::RingsideToHand { who: Who::You, .. } => "Recupera".into(),
            Effect::Search { .. } => "Busca".into(),
            Effect::LookAtHand { .. } => "Mira la mano".into(),
            Effect::SkipNextTurn { who: Who::Opponent } => "Pierde turno".into(),
            Effect::Mill { who: Who::Opponent, .. } | Effect::MillChosen { who: Who::Opponent, .. } => {
                "Al Ringside".into()
            }
            Effect::WinGame => "Gana".into(),
            Effect::Optional(inner) | Effect::AtEndOfTurn(inner) => {
                effects.extend(inner.iter());
                continue;
            }
            Effect::Choice(options) => {
                effects.extend(options.iter().flatten());
                continue;
            }
            _ => continue,
        };
        if !out.contains(&chip) {
            out.push(chip);
        }
    }
    if def.script.irreversible {
        out.insert(0, "Irreversible".into());
    }
    out
}

#[derive(Clone, Copy)]
enum Icon {
    Strike,
    Grapple,
    Submission,
    HighRisk,
    Finisher,
    Action,
    Reversal,
    PreMatch,
    Superstar,
}

impl Icon {
    fn of(kind: CardKind) -> Icon {
        match kind {
            CardKind::Maneuver(ManeuverType::Strike | ManeuverType::Assault) => Icon::Strike,
            CardKind::Maneuver(ManeuverType::Grapple | ManeuverType::Throw) => Icon::Grapple,
            CardKind::Maneuver(ManeuverType::Submission | ManeuverType::Hold) => Icon::Submission,
            CardKind::Maneuver(ManeuverType::HighRisk | ManeuverType::Extreme) => Icon::HighRisk,
            CardKind::Maneuver(ManeuverType::TrademarkFinisher | ManeuverType::Trademark) => Icon::Finisher,
            CardKind::Action | CardKind::Antic => Icon::Action,
            CardKind::Reversal => Icon::Reversal,
            CardKind::PreMatch(_) => Icon::PreMatch,
            CardKind::Enforcer | CardKind::Backstage => Icon::Superstar,
        }
    }
}

fn arc(center: Pos2, r: f32, from: f32, to: f32) -> Vec<Pos2> {
    let n = 24;
    (0..=n)
        .map(|i| {
            let a = from + (to - from) * i as f32 / n as f32;
            center + vec2(a.cos(), a.sin()) * r
        })
        .collect()
}

fn star_points(center: Pos2, outer: f32, inner: f32, points: usize, turn: f32) -> Vec<Pos2> {
    (0..points * 2)
        .map(|i| {
            let r = if i % 2 == 0 { outer } else { inner };
            let a = turn + std::f32::consts::PI * i as f32 / points as f32;
            center + vec2(a.cos(), a.sin()) * r
        })
        .collect()
}

fn arrow_head(painter: &Painter, tip: Pos2, dir: Vec2, size: f32, color: Color32) {
    let dir = dir.normalized();
    let side = vec2(-dir.y, dir.x);
    let back = tip - dir * size;
    painter.add(Shape::convex_polygon(
        vec![tip, back + side * size * 0.6, back - side * size * 0.6],
        color,
        Stroke::NONE,
    ));
}

/// A vector picture for a card type, drawn inside `rect`.
fn draw_icon(painter: &Painter, rect: Rect, icon: Icon, opacity: f32) {
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    let c = rect.center();
    let u = rect.width().min(rect.height()) * 0.42;
    let fade = |c: Color32| c.gamma_multiply(opacity);
    let light = fade(Color32::from_rgb(245, 238, 220));
    let thick = Stroke::new(u * 0.16, light);
    match icon {
        Icon::Strike => {
            // Impact burst with speed lines.
            let burst = star_points(c + vec2(u * 0.25, 0.0), u * 0.95, u * 0.45, 9, 0.2);
            painter
                .add(Shape::closed_line(burst.clone(), Stroke::new(u * 0.08, fade(Color32::from_rgb(255, 190, 60)))));
            for p in burst.windows(2).step_by(2) {
                painter.add(Shape::convex_polygon(
                    vec![c + vec2(u * 0.25, 0.0), p[0], p[1]],
                    fade(Color32::from_rgb(255, 170, 40)),
                    Stroke::NONE,
                ));
            }
            painter.circle_filled(c + vec2(u * 0.25, 0.0), u * 0.42, fade(Color32::from_rgb(255, 225, 120)));
            for (dy, len) in [(-0.45, 0.7), (0.0, 0.95), (0.45, 0.7)] {
                let y = c.y + dy * u;
                painter.line_segment(
                    [pos2(c.x - u * 1.6, y), pos2(c.x - u * (1.6 - len), y)],
                    Stroke::new(u * 0.09, light),
                );
            }
        }
        Icon::Grapple => {
            // Two locked rings.
            painter.add(Shape::line(arc(c - vec2(u * 0.4, 0.0), u * 0.6, 0.0, TAU), thick));
            painter.add(Shape::line(
                arc(c + vec2(u * 0.4, 0.0), u * 0.6, 0.0, TAU),
                Stroke::new(u * 0.16, fade(Color32::from_rgb(255, 200, 90))),
            ));
            painter.add(Shape::line(arc(c - vec2(u * 0.4, 0.0), u * 0.6, -0.6, 0.6), thick));
        }
        Icon::Submission => {
            // Padlock.
            let body = Rect::from_center_size(c + vec2(0.0, u * 0.3), vec2(u * 1.3, u * 0.95));
            painter.add(Shape::line(arc(c - vec2(0.0, u * 0.18), u * 0.45, PI, TAU), thick));
            painter.line_segment([body.left_top() + vec2(u * 0.2, 0.0), pos2(c.x - u * 0.45, c.y - u * 0.18)], thick);
            painter.line_segment([body.right_top() - vec2(u * 0.2, 0.0), pos2(c.x + u * 0.45, c.y - u * 0.18)], thick);
            painter.rect_filled(body, CornerRadius::same(3), light);
            painter.circle_filled(body.center() - vec2(0.0, u * 0.08), u * 0.13, fade(Color32::from_rgb(60, 50, 30)));
        }
        Icon::HighRisk => {
            // A leap off the top rope.
            let start = pos2(c.x - u * 1.5, c.y + u * 0.8);
            let peak = pos2(c.x, c.y - u * 0.9);
            let end = pos2(c.x + u * 1.4, c.y + u * 0.8);
            let path: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let t = i as f32 / 20.0;
                    let a = start.lerp(peak, t);
                    let b = peak.lerp(end, t);
                    a.lerp(b, t)
                })
                .collect();
            for w in path.windows(2).step_by(2) {
                painter.line_segment([w[0], w[1]], Stroke::new(u * 0.12, light));
            }
            arrow_head(painter, end, end - path[18], u * 0.4, light);
            painter.line_segment(
                [pos2(c.x - u * 1.8, c.y + u * 0.85), pos2(c.x - u * 1.1, c.y + u * 0.85)],
                Stroke::new(u * 0.08, light),
            );
        }
        Icon::Finisher => {
            let burst = star_points(c, u * 1.05, u * 0.6, 12, 0.0);
            painter.add(Shape::closed_line(burst, Stroke::new(u * 0.07, fade(Color32::from_rgb(255, 120, 60)))));
            let star = star_points(c, u * 0.85, u * 0.36, 5, -FRAC_PI_2);
            for i in 0..10 {
                painter.add(Shape::convex_polygon(
                    vec![c, star[i], star[(i + 1) % 10]],
                    fade(Color32::from_rgb(255, 215, 70)),
                    Stroke::NONE,
                ));
            }
        }
        Icon::Action => {
            // Lightning bolt.
            let p = |x: f32, y: f32| c + vec2(x, y) * u;
            let gold = fade(Color32::from_rgb(255, 225, 90));
            painter.add(Shape::convex_polygon(vec![p(0.15, -1.0), p(-0.55, 0.15), p(0.1, 0.1)], gold, Stroke::NONE));
            painter.add(Shape::convex_polygon(vec![p(-0.1, -0.1), p(0.55, -0.15), p(-0.15, 1.0)], gold, Stroke::NONE));
            painter.add(Shape::convex_polygon(
                vec![p(-0.2, 0.15), p(0.15, -0.15), p(0.15, 0.1), p(-0.2, 0.1)],
                gold,
                Stroke::NONE,
            ));
        }
        Icon::Reversal => {
            // Two arrows turning around.
            let r = u * 0.75;
            let a = arc(c, r, -PI * 0.9, -PI * 0.15);
            let b = arc(c, r, PI * 0.1, PI * 0.85);
            painter.add(Shape::line(a.clone(), thick));
            painter.add(Shape::line(b.clone(), thick));
            arrow_head(
                painter,
                a[a.len() - 1] + (a[a.len() - 1] - a[a.len() - 2]).normalized() * u * 0.3,
                a[a.len() - 1] - a[a.len() - 2],
                u * 0.45,
                light,
            );
            arrow_head(
                painter,
                b[b.len() - 1] + (b[b.len() - 1] - b[b.len() - 2]).normalized() * u * 0.3,
                b[b.len() - 1] - b[b.len() - 2],
                u * 0.45,
                light,
            );
        }
        Icon::PreMatch => {
            // Spotlight on the ring.
            painter.add(Shape::convex_polygon(
                vec![
                    c + vec2(-u * 0.2, -u * 1.1),
                    c + vec2(u * 0.2, -u * 1.1),
                    c + vec2(u * 1.0, u * 0.8),
                    c + vec2(-u * 1.0, u * 0.8),
                ],
                fade(Color32::from_rgba_unmultiplied(255, 240, 200, 70)),
                Stroke::NONE,
            ));
            painter
                .line_segment([c + vec2(-u * 1.5, u * 0.8), c + vec2(u * 1.5, u * 0.8)], Stroke::new(u * 0.1, light));
            let star = star_points(c + vec2(0.0, u * 0.1), u * 0.45, u * 0.2, 5, -FRAC_PI_2);
            for i in 0..10 {
                painter.add(Shape::convex_polygon(
                    vec![c + vec2(0.0, u * 0.1), star[i], star[(i + 1) % 10]],
                    light,
                    Stroke::NONE,
                ));
            }
        }
        Icon::Superstar => {
            // Silhouette with a title belt.
            let dark = fade(Color32::from_rgb(14, 14, 18));
            let head = c - vec2(0.0, u * 0.55);
            painter.circle_filled(head, u * 0.38, dark);
            let shoulders = arc(c + vec2(0.0, u * 1.25), u * 1.05, PI, TAU);
            painter.add(Shape::convex_polygon(shoulders, dark, Stroke::NONE));
            let belt = Rect::from_center_size(c + vec2(0.0, u * 0.75), vec2(u * 1.7, u * 0.32));
            painter.rect_filled(belt, CornerRadius::same(2), fade(Color32::from_rgb(210, 175, 60)));
            painter.circle_filled(belt.center(), u * 0.28, fade(Color32::from_rgb(240, 210, 90)));
            painter.circle_stroke(belt.center(), u * 0.28, Stroke::new(u * 0.05, dark));
        }
    }
}

/// A face-down or face-up pile with its card count. Returns its rect.
pub fn pile(ui: &mut Ui, label: &str, top: Option<&CardDef>, count: usize, face_up: bool) -> Response {
    let (rect, response) = ui.allocate_exact_size(PILE + vec2(0.0, 16.0), Sense::click());
    let card = Rect::from_min_size(rect.min, PILE);
    let painter = ui.painter_at(rect.expand(4.0));
    if count == 0 {
        painter.rect_stroke(card, CornerRadius::same(6), Stroke::new(1.0, Color32::from_gray(70)), StrokeKind::Inside);
    } else {
        for i in (1..count.min(4)).rev() {
            let offset = vec2(i as f32 * 1.5, -(i as f32) * 1.5);
            paint(&painter, card.translate(offset), None, Look::Normal, 0.8);
        }
        paint(&painter, card, if face_up { top } else { None }, Look::Normal, 1.0);
    }
    painter.text(
        Pos2::new(rect.center().x, rect.bottom()),
        egui::Align2::CENTER_BOTTOM,
        format!("{label} {count}"),
        FontId::proportional(11.0),
        Color32::from_gray(200),
    );
    if let (Some(top), true) = (top, face_up && count > 0 && response.hovered()) {
        set_preview(ui.ctx(), Preview::Card(Box::new(top.clone())));
    }
    response
}

/// The preview panel: the card last under the mouse, large, and its details.
pub fn preview_panel(ui: &mut Ui) {
    ui.heading("Carta");
    let Some(preview) = preview(ui.ctx()) else {
        ui.label(egui::RichText::new("Pasa el mouse sobre una carta para verla aquí.").weak());
        return;
    };
    let width = ui.available_width().min(PREVIEW.x);
    let size = vec2(width, width * PREVIEW.y / PREVIEW.x);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect.expand(2.0));
    match &preview {
        Preview::Card(def) => paint(&painter, rect, Some(def), Look::Normal, 1.0),
        Preview::Superstar(star) => paint_superstar(&painter, rect, star, 1.0),
    }
    ui.add_space(6.0);
    egui::ScrollArea::vertical().id_salt("preview-text").show(ui, |ui| match &preview {
        Preview::Card(def) => details(ui, def),
        Preview::Superstar(star) => {
            ui.strong(&star.name);
            ui.label(format!("Hand Size {}  ·  Superstar Value {}", star.hand_size, star.superstar_value));
            if !star.text.is_empty() {
                ui.separator();
                ui.label(&star.text);
            }
        }
    });
}

/// Full card details, under the preview.
pub fn details(ui: &mut Ui, def: &CardDef) {
    ui.strong(&def.title);
    let kinds: Vec<String> = def.kinds.iter().map(|&k| text::kind(k)).collect();
    ui.label(kinds.join(" / "));
    ui.label(format!("Fortitude {}  ·  Daño {}  ·  Stun Value {}", def.fortitude, def.damage, def.stun_value));
    let mut traits: Vec<String> = def.traits.iter().map(|t| format!("{t:?}")).collect();
    traits.extend(def.other_traits.iter().cloned());
    if !traits.is_empty() {
        ui.label(traits.join(", "));
    }
    if !def.text.is_empty() {
        ui.separator();
        ui.label(&def.text);
    }
}
