//! Look and feel: dark, calm, one accent colour, generous rounding.

use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, RichText, Sense,
    Stroke, TextStyle, Ui, Vec2,
};

pub const BG: Color32 = Color32::from_rgb(16, 17, 21);
pub const PANEL: Color32 = Color32::from_rgb(21, 23, 28);
pub const CARD: Color32 = Color32::from_rgb(28, 31, 38);
pub const CARD_HI: Color32 = Color32::from_rgb(35, 39, 48);
pub const BORDER: Color32 = Color32::from_rgb(44, 48, 58);
pub const BORDER_HI: Color32 = Color32::from_rgb(66, 72, 86);
pub const TEXT: Color32 = Color32::from_rgb(228, 231, 238);
pub const TEXT_DIM: Color32 = Color32::from_rgb(150, 157, 170);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(106, 113, 126);
pub const ACCENT: Color32 = Color32::from_rgb(96, 140, 255);
pub const GOOD: Color32 = Color32::from_rgb(74, 222, 128);
pub const WARN: Color32 = Color32::from_rgb(245, 196, 81);
pub const BAD: Color32 = Color32::from_rgb(248, 113, 113);

pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);

    let mut style = (*ctx.style()).clone();
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = Color32::from_rgb(11, 12, 15);
    v.faint_bg_color = CARD;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 10],
        blur: 26,
        spread: 0,
        color: Color32::from_black_alpha(140),
    };
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(150),
    };

    let radius = CornerRadius::same(8);
    v.widgets.noninteractive = egui::style::WidgetVisuals {
        bg_fill: PANEL,
        weak_bg_fill: PANEL,
        bg_stroke: Stroke::new(1.0_f32, BORDER),
        corner_radius: radius,
        fg_stroke: Stroke::new(1.0_f32, TEXT_DIM),
        expansion: 0.0,
    };
    v.slider_trailing_fill = true;
    v.widgets.inactive = egui::style::WidgetVisuals {
        bg_fill: Color32::from_rgb(40, 45, 55),
        weak_bg_fill: CARD,
        bg_stroke: Stroke::new(1.0_f32, BORDER),
        corner_radius: radius,
        fg_stroke: Stroke::new(1.0_f32, TEXT),
        expansion: 0.0,
    };
    v.widgets.hovered = egui::style::WidgetVisuals {
        bg_fill: Color32::from_rgb(50, 56, 68),
        weak_bg_fill: Color32::from_rgb(50, 56, 68),
        bg_stroke: Stroke::new(1.0_f32, BORDER_HI),
        corner_radius: radius,
        fg_stroke: Stroke::new(1.5_f32, Color32::WHITE),
        expansion: 0.5,
    };
    v.widgets.active = egui::style::WidgetVisuals {
        bg_fill: ACCENT.gamma_multiply(0.35),
        weak_bg_fill: ACCENT.gamma_multiply(0.35),
        bg_stroke: Stroke::new(1.0_f32, ACCENT),
        corner_radius: radius,
        fg_stroke: Stroke::new(1.5_f32, Color32::WHITE),
        expansion: 0.0,
    };
    v.widgets.open = v.widgets.hovered;
    style.visuals = v;

    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(11.0, 6.0);
    style.spacing.interact_size = Vec2::new(34.0, 26.0);
    style.spacing.slider_width = 150.0;
    style.spacing.indent = 14.0;
    style.text_styles = [
        (TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
    ]
    .into();
    ctx.set_style(style);
}

fn install_fonts(ctx: &egui::Context) {
    let candidates: [(&str, &[&str]); 1] = [(
        "noto",
        &[
            "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/TTF/NotoSans-Regular.ttf",
        ],
    )];
    let _ = &candidates;
    let Ok(regular) = std::fs::read("/usr/share/fonts/google-noto/NotoSans-Regular.ttf") else {
        return;
    };
    let medium = std::fs::read("/usr/share/fonts/google-noto/NotoSans-Medium.ttf").ok();
    let semibold = std::fs::read("/usr/share/fonts/google-noto/NotoSans-SemiBold.ttf").ok();

    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("ui".to_owned(), std::sync::Arc::new(FontData::from_owned(regular)));
    if let Some(m) = medium {
        fonts
            .font_data
            .insert("ui-medium".to_owned(), std::sync::Arc::new(FontData::from_owned(m)));
    }
    if let Some(s) = semibold {
        fonts
            .font_data
            .insert("ui-bold".to_owned(), std::sync::Arc::new(FontData::from_owned(s)));
    }
    let mut proportional = vec!["ui".to_owned()];
    if fonts.font_data.contains_key("ui-medium") {
        proportional.push("ui-medium".to_owned());
    }
    if fonts.font_data.contains_key("ui-bold") {
        proportional.push("ui-bold".to_owned());
    }
    // keep egui's fallbacks (symbols, emoji) behind our font
    if let Some(existing) = fonts.families.get(&FontFamily::Proportional) {
        proportional.extend(existing.iter().cloned());
    }
    fonts
        .families
        .insert(FontFamily::Proportional, proportional);
    ctx.set_fonts(fonts);
}

// ------------------------------------------------------------------ widgets

pub fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}


pub fn section(ui: &mut Ui, title: &str) {
    ui.add_space(2.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .size(10.5)
            .color(TEXT_FAINT)
            .strong(),
    );
    ui.add_space(2.0);
}

pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.0).color(TEXT_FAINT));
}

pub fn pill(ui: &mut Ui, text: impl Into<String>, color: Color32, filled: bool) {
    let text = text.into();
    let font = FontId::new(11.5, FontFamily::Proportional);
    let galley = ui.painter().layout_no_wrap(text, font, color);
    let size = Vec2::new(galley.size().x + 16.0, galley.size().y + 6.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    let bg = if filled {
        color.gamma_multiply(0.18)
    } else {
        Color32::TRANSPARENT
    };
    p.rect(
        rect,
        CornerRadius::same(20),
        bg,
        Stroke::new(1.0_f32, color.gamma_multiply(if filled { 0.35 } else { 0.55 })),
        egui::StrokeKind::Inside,
    );
    p.galley(
        rect.center() - galley.size() / 2.0,
        galley,
        color,
    );
}

/// Hand cursor for anything clickable (egui does not do this by itself).
pub trait HandCursor {
    fn hand_cursor(self) -> Self;
}

impl HandCursor for egui::Response {
    fn hand_cursor(self) -> Self {
        self.on_hover_cursor(egui::CursorIcon::PointingHand)
    }
}

pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}





/// Dropdown with a fixed set of values.
pub fn combo<T: PartialEq + Copy>(
    ui: &mut Ui,
    id: &str,
    width: f32,
    value: &mut T,
    label: impl Fn(T) -> String,
    options: &[T],
) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(id)
        .width(width)
        .selected_text(label(*value))
        .show_ui(ui, |ui| {
            for opt in options {
                if ui
                    .selectable_value(value, *opt, label(*opt))
                    .hand_cursor()
                    .clicked()
                {
                    changed = true;
                }
            }
        })
        .response
        .hand_cursor();
    changed
}

/// The LED bar at the bottom: one rounded blob with a segment per LED.
pub fn led_tape(ui: &mut Ui, colors: &[[u8; 3]], height: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(7), Color32::from_rgb(11, 12, 15));
    if colors.is_empty() {
        return;
    }
    let n = colors.len() as f32;
    let seg = rect.width() / n;
    let bottom = rect.bottom() - 1.0;
    for (i, c) in colors.iter().enumerate() {
        let x0 = rect.left() + 1.0 + i as f32 * seg;
        let x1 = (x0 + seg - if seg > 2.5 { 0.8 } else { 0.0 }).min(rect.right() - 1.0);
        if x1 <= x0 {
            continue;
        }
        let color = Color32::from_rgb(c[0], c[1], c[2]);
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(x0, rect.top() + 1.0),
                egui::pos2(x1, bottom),
            ),
            0,
            color,
        );
    }
    painter.rect_stroke(
        rect,
        CornerRadius::same(7),
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );
}

/// Label chip used for zone names on the canvas.
pub fn chip(painter: &egui::Painter, pos: egui::Pos2, text: &str, fg: Color32) {
    let font = FontId::new(11.0, FontFamily::Proportional);
    let galley = painter.layout_no_wrap(text.to_owned(), font, fg);
    let rect = egui::Rect::from_min_size(pos, galley.size() + Vec2::new(12.0, 5.0));
    painter.rect_filled(rect, CornerRadius::same(5), Color32::from_black_alpha(170));
    painter.galley(pos + Vec2::new(6.0, 2.5), galley, fg);
}
