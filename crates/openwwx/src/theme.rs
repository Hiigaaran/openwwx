//! The app's look: its own images (arena, logo, buttons, frames, ring pieces,
//! icons), the Anton display font and egui colors, plus small widgets built
//! from them. All images are original and come from `assets/`, made by
//! `assets/gen/make_assets.py`.

use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Id, Layout,
    Margin, Pos2, Rect, Response, Sense, Stroke, TextureHandle, TextureOptions, TextureWrapMode, Ui, UiBuilder, Vec2,
    pos2, vec2,
};

/// Red used for accents.
pub const RED: Color32 = Color32::from_rgb(200, 28, 30);
/// Gold for your side, blue for your rival's.
pub const GOLD: Color32 = Color32::from_rgb(240, 190, 70);
pub const BLUE: Color32 = Color32::from_rgb(70, 150, 255);
const METAL: Color32 = Color32::from_rgb(110, 110, 118);
const PANEL: Color32 = Color32::from_rgba_premultiplied(12, 11, 14, 225);

/// Icons in `assets/iconos`.
#[derive(Clone, Copy)]
pub enum Icon {
    Play,
    Ring,
    Globe,
    User,
    Group,
    Server,
    House,
    Link,
    Exit,
    Picture,
    Folder,
}

pub struct Theme {
    pub arena: TextureHandle,
    pub logo: TextureHandle,
    button: TextureHandle,
    button_hover: TextureHandle,
    bar: TextureHandle,
    frame_gold: TextureHandle,
    frame_blue: TextureHandle,
    pub canvas: TextureHandle,
    pub turnbuckle: TextureHandle,
    icons: Vec<TextureHandle>,
}

fn load(ctx: &Context, name: &str, bytes: &[u8], wrap: TextureWrapMode) -> TextureHandle {
    let image = image::load_from_memory(bytes).expect("bundled image").to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let image = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    let options = TextureOptions { wrap_mode: wrap, ..TextureOptions::LINEAR };
    ctx.load_texture(format!("theme-{name}"), image, options)
}

macro_rules! asset {
    ($ctx:expr, $file:literal) => {
        load($ctx, $file, include_bytes!(concat!("../assets/", $file)), TextureWrapMode::ClampToEdge)
    };
}

fn id() -> Id {
    Id::new("openwwx-theme")
}

/// The family for buttons, headings and banners.
pub fn display_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("display".into()))
}

/// Loads the images and font and sets egui's colors. Call once at start.
pub fn install(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("anton".into(), Arc::new(FontData::from_static(include_bytes!("../assets/Anton-Regular.ttf"))));
    let mut display = vec!["anton".to_string()];
    display.extend(fonts.families[&FontFamily::Proportional].iter().cloned());
    fonts.families.insert(FontFamily::Name("display".into()), display);
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.panel_fill = Color32::from_rgb(14, 13, 16);
        v.window_fill = Color32::from_rgb(20, 18, 22);
        v.window_stroke = Stroke::new(1.5, METAL);
        v.extreme_bg_color = Color32::from_rgb(8, 8, 10);
        v.selection.bg_fill = Color32::from_rgb(150, 20, 24);
        v.selection.stroke = Stroke::new(1.0, Color32::from_rgb(255, 200, 200));
        v.hyperlink_color = Color32::from_rgb(255, 120, 110);
        let w = &mut v.widgets;
        w.inactive.weak_bg_fill = Color32::from_rgb(44, 40, 44);
        w.inactive.bg_fill = Color32::from_rgb(44, 40, 44);
        w.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(80, 76, 82));
        w.hovered.weak_bg_fill = Color32::from_rgb(120, 22, 26);
        w.hovered.bg_fill = Color32::from_rgb(120, 22, 26);
        w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(230, 90, 80));
        w.active.weak_bg_fill = Color32::from_rgb(170, 26, 30);
        w.active.bg_fill = Color32::from_rgb(170, 26, 30);
        for s in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            s.corner_radius = CornerRadius::same(3);
        }
    });

    let icon = |name: &str, bytes: &[u8]| load(ctx, name, bytes, TextureWrapMode::ClampToEdge);
    let theme = Theme {
        arena: asset!(ctx, "arena.jpg"),
        logo: asset!(ctx, "logo.png"),
        button: asset!(ctx, "boton.png"),
        button_hover: asset!(ctx, "boton-hover.png"),
        bar: asset!(ctx, "barra.png"),
        frame_gold: asset!(ctx, "marco-oro.png"),
        frame_blue: asset!(ctx, "marco-azul.png"),
        canvas: load(ctx, "lona", include_bytes!("../assets/lona.jpg"), TextureWrapMode::Repeat),
        turnbuckle: asset!(ctx, "esquinero.png"),
        icons: vec![
            icon("jugar", include_bytes!("../assets/iconos/jugar.png")),
            icon("ring", include_bytes!("../assets/iconos/ring.png")),
            icon("globo", include_bytes!("../assets/iconos/globo.png")),
            icon("usuario", include_bytes!("../assets/iconos/usuario.png")),
            icon("grupo", include_bytes!("../assets/iconos/grupo.png")),
            icon("servidor", include_bytes!("../assets/iconos/servidor.png")),
            icon("casa", include_bytes!("../assets/iconos/casa.png")),
            icon("enlace", include_bytes!("../assets/iconos/enlace.png")),
            icon("salir", include_bytes!("../assets/iconos/salir.png")),
            icon("imagen", include_bytes!("../assets/iconos/imagen.png")),
            icon("carpeta", include_bytes!("../assets/iconos/carpeta.png")),
        ],
    };
    ctx.data_mut(|d| d.insert_temp(id(), Arc::new(theme)));
}

pub fn get(ctx: &Context) -> Arc<Theme> {
    ctx.data(|d| d.get_temp::<Arc<Theme>>(id())).expect("theme::install was called")
}

const FULL_UV: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

pub fn paint_image(ui: &Ui, texture: &TextureHandle, rect: Rect, tint: Color32) {
    ui.painter().image(texture.id(), rect, FULL_UV, tint);
}

pub fn paint_icon(ui: &Ui, icon: Icon, rect: Rect, tint: Color32) {
    let theme = get(ui.ctx());
    paint_image(ui, &theme.icons[icon as usize], rect, tint);
}

/// Fills `rect` with `texture`, cropped to keep its proportions.
pub fn cover(ui: &Ui, texture: &TextureHandle, rect: Rect) {
    let [w, h] = texture.size().map(|v| v as f32);
    let scale = (rect.width() / w).max(rect.height() / h);
    let shown = vec2(rect.width() / (w * scale), rect.height() / (h * scale));
    let uv = Rect::from_center_size(pos2(0.5, 0.5), shown);
    ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
}

/// A big angled red button with an icon, a title and an optional line under it.
pub fn big_button(ui: &mut Ui, icon: Icon, title: &str, width: f32) -> Response {
    let theme = get(ui.ctx());
    let size = vec2(width, width * 104.0 / 640.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hover = response.hovered();
    let texture = if hover { &theme.button_hover } else { &theme.button };
    let rect = if response.is_pointer_button_down_on() { rect.translate(vec2(1.0, 1.0)) } else { rect };
    paint_image(ui, texture, rect, Color32::WHITE);
    let u = rect.height() / 104.0;
    let icon_rect = Rect::from_center_size(pos2(rect.left() + 58.0 * u, rect.center().y), Vec2::splat(46.0 * u));
    paint_icon(ui, icon, icon_rect, Color32::WHITE);
    let text_pos = pos2(rect.left() + 118.0 * u, rect.center().y);
    let font = display_font(46.0 * u);
    ui.painter().text(
        text_pos + vec2(2.0, 2.0),
        Align2::LEFT_CENTER,
        title,
        font.clone(),
        Color32::from_black_alpha(160),
    );
    ui.painter().text(text_pos, Align2::LEFT_CENTER, title, font, Color32::WHITE);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A dark angled bar with an icon and a label, then `add` laid out left to right.
pub fn row<R>(ui: &mut Ui, icon: Icon, label: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let theme = get(ui.ctx());
    let height = 44.0;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    paint_image(ui, &theme.bar, rect, Color32::WHITE);
    let u = height / 60.0;
    let icon_rect = Rect::from_center_size(pos2(rect.left() + 36.0 * u, rect.center().y), Vec2::splat(30.0 * u));
    paint_icon(ui, icon, icon_rect, Color32::WHITE);
    let label_x = rect.left() + 82.0 * u;
    ui.painter().text(
        pos2(label_x, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        display_font(17.0),
        Color32::from_gray(235),
    );
    let inner =
        Rect::from_min_max(pos2(label_x + 118.0, rect.top() + 4.0), pos2(rect.right() - 18.0, rect.bottom() - 4.0));
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    add(&mut child)
}

/// A dark panel with a metal edge.
pub fn panel() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.5, METAL))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(Margin::same(10))
}

/// A heading in the display font.
pub fn heading(ui: &mut Ui, text: &str, size: f32) -> Response {
    ui.label(egui::RichText::new(text).font(display_font(size)).color(Color32::WHITE))
}

/// The metal frame (gold for you, blue for your rival) around a card at `card`.
pub fn card_frame(ui: &Ui, card: Rect, gold: bool) {
    let theme = get(ui.ctx());
    let texture = if gold { &theme.frame_gold } else { &theme.frame_blue };
    // The frame image is 370×490 with its opening at 42..328 × 42..448.
    let scale = card.width() / 286.0;
    let rect = Rect::from_min_size(card.min - Vec2::splat(42.0 * scale), vec2(370.0, 490.0) * scale);
    paint_image(ui, texture, rect, Color32::WHITE);
}

/// Size of a card that fills `width` inside its frame.
pub fn framed_card_size(width: f32) -> Vec2 {
    let card_w = width * 286.0 / 370.0;
    vec2(card_w, card_w * 406.0 / 286.0)
}

/// Text with a dark shadow, for banners over the ring.
pub fn shadow_text(ui: &Ui, pos: Pos2, align: Align2, text: &str, font: FontId, color: Color32) {
    for d in [vec2(3.0, 3.0), vec2(-1.0, 1.0), vec2(1.0, -1.0)] {
        ui.painter().text(pos + d, align, text, font.clone(), Color32::from_black_alpha(200));
    }
    ui.painter().text(pos, align, text, font, color);
}
