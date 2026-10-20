// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Patrick Weppelmann

//! Look and feel.
//!
//! The app lives in a dark room next to a glowing screen, so the chrome is a
//! quiet graphite "wall" and the only loud thing is light: the warm-white LED
//! amber accent and the live light spill around the preview. Everything else
//! stays out of the way.

use egui::{
    epaint::{Mesh, Vertex},
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Pos2, Rect,
    Response, RichText, Sense, Stroke, TextStyle, Ui, Vec2,
};

// ------------------------------------------------------------------ tokens

/// window background - the wall behind the screen
pub const WALL: Color32 = Color32::from_rgb(18, 20, 25);
/// side panels
pub const PANEL: Color32 = Color32::from_rgb(21, 23, 29);
/// cards and inputs
pub const SURFACE: Color32 = Color32::from_rgb(27, 30, 38);
/// hover / selected surface
pub const RAISED: Color32 = Color32::from_rgb(36, 40, 50);
/// the matte the preview sits in
pub const MATTE: Color32 = Color32::from_rgb(12, 13, 17);
pub const LINE: Color32 = Color32::from_rgb(43, 48, 60);
pub const LINE_HI: Color32 = Color32::from_rgb(64, 70, 86);
pub const INK: Color32 = Color32::from_rgb(232, 234, 240);
pub const INK_DIM: Color32 = Color32::from_rgb(163, 169, 184);
pub const INK_FAINT: Color32 = Color32::from_rgb(110, 117, 133);
/// warm-white LED amber - the one accent
pub const AMBER: Color32 = Color32::from_rgb(255, 196, 107);
/// text on amber
pub const AMBER_INK: Color32 = Color32::from_rgb(42, 29, 5);
pub const MINT: Color32 = Color32::from_rgb(91, 227, 166);
pub const CORAL: Color32 = Color32::from_rgb(255, 107, 107);

pub const R_SM: u8 = 6;
pub const R_MD: u8 = 9;
pub const R_LG: u8 = 14;

// ------------------------------------------------------------------ fonts

const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");

fn semibold_family() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Font for headings and emphasised numbers.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, semibold_family())
}

pub fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "inter".into(),
        std::sync::Arc::new(FontData::from_static(INTER_REGULAR)),
    );
    fonts.font_data.insert(
        "inter-semibold".into(),
        std::sync::Arc::new(FontData::from_static(INTER_SEMIBOLD)),
    );
    // keep egui's own fonts as fallback for symbols and emoji
    let fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mut prop = vec!["inter".to_owned()];
    prop.extend(fallback.iter().cloned());
    fonts.families.insert(FontFamily::Proportional, prop);
    let mut bold = vec!["inter-semibold".to_owned(), "inter".to_owned()];
    bold.extend(fallback);
    fonts.families.insert(semibold_family(), bold);
    ctx.set_fonts(fonts);
}

pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);

    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    let mut v = egui::Visuals::dark();
    v.panel_fill = WALL;
    v.window_fill = PANEL;
    v.extreme_bg_color = MATTE;
    v.faint_bg_color = SURFACE;
    v.code_bg_color = SURFACE;
    v.override_text_color = Some(INK);
    v.hyperlink_color = AMBER;
    v.weak_text_color = Some(INK_FAINT);
    // also the filled part of sliders
    v.selection.bg_fill = Color32::from_rgb(201, 150, 78);
    v.selection.stroke = Stroke::new(1.0_f32, AMBER);
    v.text_cursor.stroke = Stroke::new(2.0_f32, AMBER);
    v.window_corner_radius = CornerRadius::same(R_LG);
    v.menu_corner_radius = CornerRadius::same(R_MD);
    v.window_stroke = Stroke::new(1.0_f32, LINE);
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(150),
    };
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 22,
        spread: 0,
        color: Color32::from_black_alpha(160),
    };
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = false;
    v.indent_has_left_vline = false;

    let r = CornerRadius::same(R_SM);
    v.widgets.noninteractive = egui::style::WidgetVisuals {
        bg_fill: PANEL,
        weak_bg_fill: PANEL,
        bg_stroke: Stroke::new(1.0_f32, LINE),
        corner_radius: r,
        fg_stroke: Stroke::new(1.0_f32, INK_DIM),
        expansion: 0.0,
    };
    v.widgets.inactive = egui::style::WidgetVisuals {
        bg_fill: RAISED,
        weak_bg_fill: SURFACE,
        bg_stroke: Stroke::new(1.0_f32, LINE),
        corner_radius: r,
        fg_stroke: Stroke::new(1.0_f32, INK),
        expansion: 0.0,
    };
    v.widgets.hovered = egui::style::WidgetVisuals {
        bg_fill: Color32::from_rgb(48, 53, 66),
        weak_bg_fill: RAISED,
        bg_stroke: Stroke::new(1.0_f32, LINE_HI),
        corner_radius: r,
        fg_stroke: Stroke::new(1.5_f32, Color32::WHITE),
        expansion: 0.0,
    };
    v.widgets.active = egui::style::WidgetVisuals {
        bg_fill: AMBER,
        weak_bg_fill: RAISED,
        bg_stroke: Stroke::new(1.0_f32, AMBER),
        corner_radius: r,
        fg_stroke: Stroke::new(1.5_f32, Color32::WHITE),
        expansion: 0.0,
    };
    v.widgets.open = v.widgets.hovered;
    style.visuals = v;

    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.interact_size = Vec2::new(34.0, 28.0);
    style.spacing.slider_width = 180.0;
    style.spacing.slider_rail_height = 4.0;
    style.spacing.combo_width = 120.0;
    style.spacing.indent = 14.0;
    style.spacing.menu_margin = Margin::same(6);
    style.spacing.scroll = egui::style::ScrollStyle::floating();
    style.text_styles = [
        (TextStyle::Heading, semibold(16.0)),
        (TextStyle::Body, regular(13.0)),
        (TextStyle::Button, regular(13.0)),
        (TextStyle::Small, regular(11.5)),
        (TextStyle::Monospace, FontId::monospace(12.0)),
    ]
    .into();
    style.interaction.tooltip_delay = 0.35;
    ctx.set_style_of(egui::Theme::Dark, style);
}

// ------------------------------------------------------------------ text

/// Panel / group title, sentence case.
pub fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(semibold(13.5)).color(INK));
}

/// Small label in front of a control.
pub fn field_label(ui: &mut Ui, text: &str) -> Response {
    ui.label(RichText::new(text).size(12.0).color(INK_DIM))
}

pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(11.5).color(INK_FAINT));
}

// ------------------------------------------------------------------ containers

pub fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) -> Response {
    card_with(ui, SURFACE, LINE, add)
}

pub fn card_with(
    ui: &mut Ui,
    fill: Color32,
    stroke: Color32,
    add: impl FnOnce(&mut Ui),
) -> Response {
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, stroke))
        .corner_radius(CornerRadius::same(R_LG - 2))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        })
        .response
}

// ------------------------------------------------------------------ cursor

/// Hand cursor for anything clickable (egui does not do this by itself).
pub trait HandCursor {
    fn hand_cursor(self) -> Self;
}

impl HandCursor for Response {
    fn hand_cursor(self) -> Self {
        self.on_hover_cursor(egui::CursorIcon::PointingHand)
    }
}

// ------------------------------------------------------------------ widgets

/// Status pill: coloured dot + text.
pub fn status_pill(ui: &mut Ui, text: &str, color: Color32) -> Response {
    let font = regular(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_owned(), font, INK);
    let size = Vec2::new(galley.size().x + 30.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect(
        rect,
        CornerRadius::same(13),
        color.gamma_multiply(0.10),
        Stroke::new(1.0_f32, color.gamma_multiply(0.35)),
        egui::StrokeKind::Inside,
    );
    let c = Pos2::new(rect.left() + 13.0, rect.center().y);
    p.circle_filled(c, 6.0, color.gamma_multiply(0.25));
    p.circle_filled(c, 3.5, color);
    p.galley(
        Pos2::new(rect.left() + 23.0, rect.center().y - galley.size().y / 2.0),
        galley,
        INK,
    );
    resp
}

/// Quiet metric: "label value".
pub fn metric(ui: &mut Ui, label: &str, value: &str) {
    ui.label(RichText::new(value).font(semibold(12.0)).color(INK_DIM));
    ui.label(RichText::new(label).size(11.5).color(INK_FAINT));
}

/// iOS-style switch with a label on the right. Returns true when toggled.
pub fn switch(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let font = regular(12.5);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, INK);
    let track = Vec2::new(30.0, 17.0);
    let gap = if label.is_empty() { 0.0 } else { 8.0 };
    let size = Vec2::new(track.x + gap + galley.size().x, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let tr = Rect::from_min_size(
        Pos2::new(rect.left(), rect.center().y - track.y / 2.0),
        track,
    );
    let p = ui.painter();
    let off_fill = if resp.hovered() { LINE_HI } else { LINE };
    let fill = lerp_color(off_fill, AMBER, t);
    p.rect_filled(tr, CornerRadius::same(9), fill);
    let knob_x = egui::lerp(tr.left() + 8.5..=tr.right() - 8.5, t);
    p.circle_filled(
        Pos2::new(knob_x, tr.center().y),
        6.5,
        lerp_color(INK_DIM, AMBER_INK, t),
    );
    if !label.is_empty() {
        p.galley_with_override_text_color(
            Pos2::new(tr.right() + gap, rect.center().y - galley.size().y / 2.0),
            galley,
            if *on { INK } else { INK_DIM },
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Segmented control, at least `width` wide. Returns true when the selection changed.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    value: &mut T,
    options: &[(T, &str)],
    width: Option<f32>,
) -> bool {
    let font = regular(12.5);
    let galleys: Vec<_> = options
        .iter()
        .map(|(_, l)| {
            ui.painter()
                .layout_no_wrap((*l).to_owned(), font.clone(), INK)
        })
        .collect();
    let natural: f32 = galleys.iter().map(|g| g.size().x + 22.0).sum::<f32>() + 4.0;
    let w = width.map_or(natural, |w| w.max(natural));
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(R_MD),
        MATTE,
        Stroke::new(1.0_f32, LINE),
        egui::StrokeKind::Inside,
    );
    let seg_w = (rect.width() - 4.0) / options.len().max(1) as f32;
    let mut changed = false;
    for (i, ((opt, _), galley)) in options.iter().zip(galleys).enumerate() {
        let r = Rect::from_min_size(
            Pos2::new(rect.left() + 2.0 + i as f32 * seg_w, rect.top() + 2.0),
            Vec2::new(seg_w, rect.height() - 4.0),
        );
        let resp = ui
            .interact(
                r,
                ui.scope_id()
                    .with(("seg", i, rect.left() as i32, rect.top() as i32)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let sel = *value == *opt;
        if resp.clicked() && !sel {
            *value = *opt;
            changed = true;
        }
        let p = ui.painter();
        if sel {
            p.rect_filled(r, CornerRadius::same(R_SM + 1), RAISED);
            p.rect_stroke(
                r,
                CornerRadius::same(R_SM + 1),
                Stroke::new(1.0_f32, LINE_HI),
                egui::StrokeKind::Inside,
            );
        } else if resp.hovered() {
            p.rect_filled(r, CornerRadius::same(R_SM + 1), SURFACE);
        }
        p.galley_with_override_text_color(
            r.center() - galley.size() / 2.0,
            galley,
            if sel { INK } else { INK_DIM },
        );
    }
    changed
}

/// Label + value on one line, full width slider underneath.
pub fn slider_row(
    ui: &mut Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    fmt: impl Fn(f32) -> String,
) -> bool {
    ui.horizontal(|ui| {
        field_label(ui, label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(fmt(*value)).font(semibold(12.0)).color(INK));
        });
    });
    let old = (ui.spacing().slider_width, ui.spacing().interact_size.y);
    ui.spacing_mut().slider_width = ui.available_width();
    // smaller knob than egui's default
    ui.spacing_mut().interact_size.y = 18.0;
    let r = ui
        .add(egui::Slider::new(value, range).show_value(false))
        .hand_cursor();
    ui.spacing_mut().slider_width = old.0;
    ui.spacing_mut().interact_size.y = old.1;
    ui.add_space(2.0);
    r.changed()
}

/// Dropdown with a fixed set of values.
pub fn combo<T: PartialEq + Copy>(
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
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

/// Thin usage bar (e.g. LEDs assigned to zones / LEDs on the strip).
pub fn usage_bar(ui: &mut Ui, used: f32, total: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 4.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(2), MATTE);
    if total <= 0.0 {
        return;
    }
    let frac = (used / total).clamp(0.0, 1.0);
    let over = used > total;
    let mut fill = rect;
    fill.set_width(rect.width() * frac);
    p.rect_filled(
        fill,
        CornerRadius::same(2),
        if over { CORAL } else { INK_DIM },
    );
}

/// The LED tape: one segment per LED, in the colour that is being sent.
pub fn led_tape(ui: &mut Ui, colors: &[[u8; 3]], height: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(R_SM), MATTE);
    if !colors.is_empty() {
        let inner = rect.shrink(2.0);
        let n = colors.len() as f32;
        let seg = inner.width() / n;
        let gap = if seg > 3.0 { 1.0 } else { 0.0 };
        for (i, c) in colors.iter().enumerate() {
            let x0 = inner.left() + i as f32 * seg;
            let x1 = (x0 + seg - gap).min(inner.right());
            if x1 <= x0 {
                continue;
            }
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(x0, inner.top()), Pos2::new(x1, inner.bottom())),
                if seg > 6.0 { 2 } else { 0 },
                Color32::from_rgb(c[0], c[1], c[2]),
            );
        }
    }
    painter.rect_stroke(
        rect,
        CornerRadius::same(R_SM),
        Stroke::new(1.0_f32, LINE),
        egui::StrokeKind::Inside,
    );
}

/// Label chip used for zone names on the canvas.
pub fn chip(painter: &egui::Painter, pos: Pos2, text: &str, fg: Color32) -> Rect {
    let font = regular(11.5);
    let galley = painter.layout_no_wrap(text.to_owned(), font, fg);
    let rect = Rect::from_min_size(pos, galley.size() + Vec2::new(14.0, 7.0));
    painter.rect_filled(
        rect,
        CornerRadius::same(R_SM),
        Color32::from_black_alpha(190),
    );
    painter.galley(pos + Vec2::new(7.0, 3.5), galley, fg);
    rect
}

// ------------------------------------------------------------------ light spill

/// Which way the light spills from an edge segment.
#[derive(Clone, Copy)]
pub enum Spill {
    Up,
    Down,
    Left,
    Right,
}

/// Paint the light thrown onto the "wall" around the preview: a soft gradient
/// from the edge outwards, one slice per LED, in the colour that is sent.
pub fn light_spill(
    painter: &egui::Painter,
    edge: [Pos2; 2],
    dir: Spill,
    colors: &[Color32],
    depth: f32,
) {
    if colors.is_empty() {
        return;
    }
    let out = match dir {
        Spill::Up => Vec2::new(0.0, -depth),
        Spill::Down => Vec2::new(0.0, depth),
        Spill::Left => Vec2::new(-depth, 0.0),
        Spill::Right => Vec2::new(depth, 0.0),
    };
    let mut mesh = Mesh::default();
    let n = colors.len();
    // vertices along the edge (inner, mid, outer) per LED boundary; neighbouring
    // LEDs share vertices, so the colours blend into a continuous glow
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let p = edge[0] + (edge[1] - edge[0]) * t;
        // colour at a boundary = mean of the two neighbouring LEDs
        let a = colors[i.saturating_sub(1).min(n - 1)];
        let b = colors[i.min(n - 1)];
        let c = Color32::from_rgb(
            ((a.r() as u16 + b.r() as u16) / 2) as u8,
            ((a.g() as u16 + b.g() as u16) / 2) as u8,
            ((a.b() as u16 + b.b() as u16) / 2) as u8,
        );
        let base = mesh.vertices.len() as u32;
        mesh.vertices.push(Vertex {
            pos: p,
            uv: egui::epaint::WHITE_UV,
            color: c.gamma_multiply(0.85),
        });
        mesh.vertices.push(Vertex {
            pos: p + out * 0.3,
            uv: egui::epaint::WHITE_UV,
            color: c.gamma_multiply(0.38),
        });
        mesh.vertices.push(Vertex {
            pos: p + out,
            uv: egui::epaint::WHITE_UV,
            color: Color32::TRANSPARENT,
        });
        if i > 0 {
            let prev = base - 3;
            for k in 0..2 {
                mesh.add_triangle(prev + k, base + k, prev + k + 1);
                mesh.add_triangle(prev + k + 1, base + k, base + k + 1);
            }
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(
        l(a.r(), b.r()),
        l(a.g(), b.g()),
        l(a.b(), b.b()),
        l(a.a(), b.a()),
    )
}
