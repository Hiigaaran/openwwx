//! Drawing cards. Until card art can be loaded, a card is a colored frame
//! with its printed stats and text.

use eframe::egui::{
    self, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, vec2,
};
use rawdeal_core::{CardDef, CardKind};

use crate::text;

pub const SIZE: Vec2 = vec2(112.0, 156.0);
pub const SMALL: Vec2 = vec2(84.0, 117.0);
pub const PILE: Vec2 = vec2(60.0, 84.0);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Look {
    Normal,
    /// Can be clicked now.
    Playable,
    Selected,
}

fn color(kind: CardKind) -> Color32 {
    match kind {
        CardKind::Maneuver(_) => Color32::from_rgb(196, 152, 52),
        CardKind::Action => Color32::from_rgb(62, 104, 172),
        CardKind::Reversal => Color32::from_rgb(166, 56, 56),
        CardKind::PreMatch(_) => Color32::from_rgb(120, 72, 150),
    }
}

/// Draws a card face (or its back, if `def` is hidden) and returns the click response.
pub fn card(ui: &mut Ui, def: Option<&CardDef>, size: Vec2, look: Look) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let lift = if look == Look::Playable && response.hovered() { -4.0 } else { 0.0 };
    paint(&ui.painter_at(rect.expand(6.0)), rect.translate(vec2(0.0, lift)), def, look, 1.0);
    match def {
        Some(def) => response.on_hover_ui(|ui| details(ui, def)),
        None => response,
    }
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

    // Opaque base so cards flying over the board do not show through.
    painter.rect_filled(rect, radius, fade(Color32::from_rgb(27, 27, 27)));
    painter.rect_filled(rect, radius, fade(color(def.kinds[0]).gamma_multiply(0.35)));
    // A band per kind across the top, so hybrids show both colors.
    let band_h = 6.0 * scale;
    let band_w = rect.width() / def.kinds.len() as f32;
    for (i, &k) in def.kinds.iter().enumerate() {
        let x = rect.left() + band_w * i as f32;
        let band = Rect::from_min_size(Pos2::new(x, rect.top()), vec2(band_w, band_h));
        painter.rect_filled(band, CornerRadius::ZERO, fade(color(k)));
    }
    let stroke = match look {
        Look::Normal => Stroke::new(1.0, fade(Color32::from_gray(90))),
        Look::Playable => Stroke::new(2.5, fade(Color32::from_rgb(90, 220, 120))),
        Look::Selected => Stroke::new(3.0, fade(Color32::from_rgb(255, 210, 60))),
    };

    let pad = 5.0 * scale;
    let width = rect.width() - 2.0 * pad;
    let mut y = rect.top() + band_h + pad * 0.6;
    let title = painter.layout(def.title.clone(), FontId::proportional(12.0 * scale), fade(Color32::WHITE), width);
    let title_h = title.size().y.min(44.0 * scale);
    painter.galley(Pos2::new(rect.left() + pad, y), title, Color32::WHITE);
    y += title_h + 2.0 * scale;

    let kinds: Vec<String> = def.kinds.iter().map(|&k| text::kind(k)).collect();
    let kinds =
        painter.layout(kinds.join(" / "), FontId::proportional(9.0 * scale), fade(Color32::from_gray(210)), width);
    let kinds_h = kinds.size().y;
    painter.galley(Pos2::new(rect.left() + pad, y), kinds, Color32::WHITE);
    y += kinds_h + 3.0 * scale;

    let stats_h = 16.0 * scale;
    let text_rect = Rect::from_min_max(
        Pos2::new(rect.left() + pad, y),
        Pos2::new(rect.right() - pad, rect.bottom() - stats_h - pad),
    );
    if text_rect.height() > 8.0 && !def.text.is_empty() {
        let body =
            painter.layout(def.text.clone(), FontId::proportional(8.0 * scale), fade(Color32::from_gray(225)), width);
        painter.with_clip_rect(text_rect).galley(text_rect.min, body, Color32::WHITE);
    }

    let stats = format!("F {}   D {}   SV {}", def.fortitude, def.damage, def.stun_value);
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - pad),
        egui::Align2::CENTER_BOTTOM,
        stats,
        FontId::monospace(10.0 * scale),
        fade(Color32::WHITE),
    );
    painter.rect_stroke(rect, radius, stroke, StrokeKind::Inside);
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
    response
}

/// Full card details, for tooltips.
pub fn details(ui: &mut Ui, def: &CardDef) {
    ui.set_max_width(320.0);
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
