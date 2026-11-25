//! Colors, fonts, and small painting helpers shared by the app.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Rect, Stroke,
};

pub const BG: Color32 = Color32::from_rgb(11, 11, 15);
pub const CARD: Color32 = Color32::from_rgb(18, 18, 24);
pub const CARD_BACK: Color32 = Color32::from_rgb(23, 23, 31);
pub const PANEL_HOVER: Color32 = Color32::from_rgb(32, 32, 42);
pub const BORDER: Color32 = Color32::from_rgb(41, 41, 53);
pub const TEXT: Color32 = Color32::from_rgb(246, 243, 237);
pub const MUTED: Color32 = Color32::from_rgb(151, 148, 161);
pub const MUTED_DIM: Color32 = Color32::from_rgb(102, 100, 113);
pub const ACCENT: Color32 = Color32::from_rgb(255, 77, 109);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(255, 106, 131);
pub const ACCENT_PRESSED: Color32 = Color32::from_rgb(226, 57, 88);
pub const INK: Color32 = Color32::from_rgb(24, 10, 15);
pub const GOOD: Color32 = Color32::from_rgb(190, 235, 130);

pub fn display_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("display".into()))
}

pub fn bold_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

pub fn body_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

/// Install the bundled fonts and the dark "photo booth" styling.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    fonts.font_data.insert(
        "body".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/SpaceGrotesk-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "body_bold".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/SpaceGrotesk-Bold.ttf"
        ))),
    );
    fonts.font_data.insert(
        "display".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/ArchivoBlack-Regular.ttf"
        ))),
    );

    let fallbacks = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();

    let mut body = vec!["body".to_owned()];
    body.extend(fallbacks.iter().cloned());
    fonts.families.insert(FontFamily::Proportional, body);

    let mut display = vec!["display".to_owned(), "body".to_owned()];
    display.extend(fallbacks.iter().cloned());
    fonts
        .families
        .insert(FontFamily::Name("display".into()), display);

    let mut bold = vec!["body_bold".to_owned(), "body".to_owned()];
    bold.extend(fallbacks.iter().cloned());
    fonts.families.insert(FontFamily::Name("bold".into()), bold);

    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = CARD;
    visuals.window_corner_radius = CornerRadius::same(16);
    visuals.extreme_bg_color = Color32::from_rgb(12, 12, 17);
    visuals.faint_bg_color = CARD_BACK;
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0_f32, INK);
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);

    visuals.widgets.noninteractive.bg_fill = CARD;
    visuals.widgets.noninteractive.weak_bg_fill = CARD;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, MUTED);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(8);

    visuals.widgets.inactive.bg_fill = CARD_BACK;
    visuals.widgets.inactive.weak_bg_fill = CARD_BACK;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
    visuals.widgets.inactive.expansion = 0.0;

    visuals.widgets.hovered.bg_fill = PANEL_HOVER;
    visuals.widgets.hovered.weak_bg_fill = PANEL_HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
    visuals.widgets.hovered.expansion = 0.0;

    visuals.widgets.active.bg_fill = Color32::from_rgb(28, 28, 37);
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(28, 28, 37);
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.active.corner_radius = CornerRadius::same(8);
    visuals.widgets.active.expansion = 0.0;

    visuals.widgets.open.bg_fill = PANEL_HOVER;
    visuals.widgets.open.weak_bg_fill = PANEL_HOVER;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.open.corner_radius = CornerRadius::same(8);

    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 8.0);
    style.spacing.interact_size.y = 32.0;
    style.animation_time = 0.16;
    style.text_styles = [
        (egui::TextStyle::Heading, display_font(22.0)),
        (egui::TextStyle::Body, body_font(15.0)),
        (egui::TextStyle::Button, bold_font(14.0)),
        (egui::TextStyle::Small, body_font(11.5)),
        (
            egui::TextStyle::Monospace,
            FontId::new(12.0, FontFamily::Monospace),
        ),
    ]
    .into();

    ctx.set_style(style);
}

/// Fit a rectangle with the given width/height ratio inside `container`.
pub fn fit_rect(container: Rect, aspect: f32) -> Rect {
    if container.width() <= 0.0 || container.height() <= 0.0 || aspect <= 0.0 {
        return container;
    }

    let container_aspect = container.width() / container.height();
    let size = if aspect > container_aspect {
        egui::vec2(container.width(), container.width() / aspect)
    } else {
        egui::vec2(container.height() * aspect, container.height())
    };

    Rect::from_center_size(container.center(), size)
}

/// Little crop marks in the corners of the photo stage.
pub fn corner_marks(painter: &egui::Painter, rect: Rect) {
    let rect = rect.shrink(15.0);
    let color = Color32::from_rgba_unmultiplied(ACCENT.r(), ACCENT.g(), ACCENT.b(), 80);
    let stroke = Stroke::new(1.5_f32, color);
    let len = 14.0;

    for (corner, x_dir, y_dir) in [
        (rect.left_top(), 1.0, 1.0),
        (rect.right_top(), -1.0, 1.0),
        (rect.left_bottom(), 1.0, -1.0),
        (rect.right_bottom(), -1.0, -1.0),
    ] {
        painter.line_segment([corner, corner + egui::vec2(len * x_dir, 0.0)], stroke);
        painter.line_segment([corner, corner + egui::vec2(0.0, len * y_dir)], stroke);
    }
}

/// Keep long folder and file names readable in tight spaces.
pub fn truncate_middle(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars || max_chars < 6 {
        return text.to_owned();
    }

    let keep = max_chars - 1;
    let head_len = keep.div_ceil(2);
    let tail_len = keep - head_len;
    let head: String = text.chars().take(head_len).collect();
    let tail: String = text.chars().skip(count - tail_len).collect();
    format!("{head}…{tail}")
}
