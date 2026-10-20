//! Icons are painted as geometry, not as font glyphs - no missing-codepoint tofu,
//! crisp at any size and always in the theme colour.

use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use crate::theme;

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Play,
    Pause,
    Refresh,
    Save,
    Plus,
    Close,
    ChevronUp,
    ChevronDown,
    Search,
    Split,
    Grid,
}

fn p(r: Rect, x: f32, y: f32) -> Pos2 {
    Pos2::new(r.left() + x * r.width(), r.top() + y * r.height())
}

pub fn paint(painter: &Painter, r: Rect, icon: Icon, color: Color32) {
    let w = (r.width() * 0.10).clamp(1.2, 2.2);
    let stroke = Stroke::new(w, color);
    let fill = color;
    match icon {
        Icon::Play => {
            painter.add(egui::Shape::convex_polygon(
                vec![p(r, 0.24, 0.12), p(r, 0.86, 0.5), p(r, 0.24, 0.88)],
                fill,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            painter.rect_filled(
                Rect::from_min_max(p(r, 0.24, 0.14), p(r, 0.43, 0.86)),
                CornerRadius::same(2),
                fill,
            );
            painter.rect_filled(
                Rect::from_min_max(p(r, 0.57, 0.14), p(r, 0.76, 0.86)),
                CornerRadius::same(2),
                fill,
            );
        }
        Icon::Refresh => {
            let c = p(r, 0.5, 0.52);
            let radius = r.width() * 0.34;
            let mut pts = Vec::new();
            let start = -0.9f32;
            let end = 4.4f32;
            let steps = 28;
            for i in 0..=steps {
                let a = start + (end - start) * i as f32 / steps as f32;
                pts.push(Pos2::new(c.x + radius * a.cos(), c.y + radius * a.sin()));
            }
            painter.add(egui::Shape::line(pts, stroke));
            let tip = Pos2::new(
                c.x + radius * end.cos(),
                c.y + radius * end.sin(),
            );
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(tip.x - r.width() * 0.26, tip.y - r.height() * 0.24),
                    Pos2::new(tip.x + r.width() * 0.06, tip.y - r.height() * 0.02),
                    Pos2::new(tip.x - r.width() * 0.10, tip.y + r.height() * 0.26),
                ],
                fill,
                Stroke::NONE,
            ));
        }
        Icon::Save => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.14, 0.14), p(r, 0.86, 0.86)),
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.30, 0.14), p(r, 0.70, 0.42)),
                CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.30, 0.56), p(r, 0.70, 0.86)),
                CornerRadius::same(1),
                stroke,
                egui::StrokeKind::Inside,
            );
        }
        Icon::Plus => {
            painter.line_segment([p(r, 0.5, 0.16), p(r, 0.5, 0.84)], stroke);
            painter.line_segment([p(r, 0.16, 0.5), p(r, 0.84, 0.5)], stroke);
        }
        Icon::Close => {
            painter.line_segment([p(r, 0.22, 0.22), p(r, 0.78, 0.78)], stroke);
            painter.line_segment([p(r, 0.78, 0.22), p(r, 0.22, 0.78)], stroke);
        }
        Icon::ChevronUp => {
            painter.add(egui::Shape::line(
                vec![p(r, 0.22, 0.66), p(r, 0.5, 0.36), p(r, 0.78, 0.66)],
                stroke,
            ));
        }
        Icon::ChevronDown => {
            painter.add(egui::Shape::line(
                vec![p(r, 0.22, 0.34), p(r, 0.5, 0.64), p(r, 0.78, 0.34)],
                stroke,
            ));
        }
        Icon::Search => {
            painter.circle_stroke(p(r, 0.42, 0.42), r.width() * 0.24, stroke);
            painter.line_segment([p(r, 0.60, 0.60), p(r, 0.84, 0.84)], stroke);
        }
        Icon::Split => {
            painter.rect_stroke(
                Rect::from_min_max(p(r, 0.12, 0.12), p(r, 0.88, 0.88)),
                CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.line_segment([p(r, 0.5, 0.12), p(r, 0.5, 0.88)], stroke);
            painter.line_segment([p(r, 0.12, 0.5), p(r, 0.88, 0.5)], stroke);
        }
        Icon::Grid => {
            for i in 0..4 {
                let f = 0.2 + i as f32 * 0.2;
                painter.line_segment([p(r, f, 0.14), p(r, f, 0.86)], stroke);
                painter.line_segment([p(r, 0.14, f), p(r, 0.86, f)], stroke);
            }
        }
    }
}

/// Square icon button with hover background.
pub fn button(ui: &mut Ui, icon: Icon, tip: &str) -> bool {
    let size = Vec2::splat(26.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let bg = if resp.hovered() {
        theme::CARD_HI
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(7), bg);
    let color = if resp.hovered() {
        Color32::WHITE
    } else {
        theme::TEXT_DIM
    };
    paint(painter, rect.shrink(7.0), icon, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip)
        .clicked()
}

/// Button with an icon and a label.
pub fn labelled(ui: &mut Ui, icon: Icon, label: &str, primary: bool) -> bool {
    let font = egui::FontId::proportional(12.5);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::WHITE);
    let height = 26.0;
    let width = galley.size().x + 22.0 + 20.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let painter = ui.painter();
    let (fill, border) = if primary {
        (
            if resp.hovered() {
                theme::ACCENT
            } else {
                theme::ACCENT.gamma_multiply(0.85)
            },
            theme::ACCENT,
        )
    } else if resp.hovered() {
        (theme::CARD_HI, theme::BORDER_HI)
    } else {
        (Color32::TRANSPARENT, Color32::TRANSPARENT)
    };
    painter.rect(
        rect,
        CornerRadius::same(8),
        fill,
        Stroke::new(1.0_f32, border),
        egui::StrokeKind::Inside,
    );
    let fg = if primary {
        Color32::WHITE
    } else if resp.hovered() {
        Color32::WHITE
    } else {
        theme::TEXT_DIM
    };
    paint(
        painter,
        Rect::from_min_size(
            Pos2::new(rect.left() + 9.0, rect.center().y - 6.5),
            Vec2::splat(13.0),
        ),
        icon,
        fg,
    );
    painter.galley(
        Pos2::new(rect.left() + 28.0, rect.center().y - galley.size().y / 2.0),
        galley,
        fg,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
        .clicked()
}
