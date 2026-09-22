//! Shared fold chrome for workspace sections and analyzer regions.

use egui::{Rect, Response, Vec2};

use crate::theme;

pub(crate) fn paint_fold(ui: &egui::Ui, response: &Response, cell: Rect, direction: Vec2) {
    let hot = response.hovered() || response.has_focus();
    let painter = ui.painter_at(cell);
    painter.rect_filled(
        cell,
        0.0,
        if response.is_pointer_button_down_on() {
            theme::accent_active()
        } else if hot {
            theme::widget_hover()
        } else {
            theme::well()
        },
    );
    if hot {
        painter.rect_stroke(
            cell,
            0.0,
            egui::Stroke::new(1.0, theme::accent_edge()),
            egui::StrokeKind::Inside,
        );
    }
    let scale = theme::ui_scale(ui.ctx());
    let center = cell.center();
    let cross = egui::vec2(-direction.y, direction.x);
    painter.add(egui::Shape::line(
        vec![
            center - direction * 2.0 * scale + cross * 4.0 * scale,
            center + direction * 2.0 * scale,
            center - direction * 2.0 * scale - cross * 4.0 * scale,
        ],
        egui::Stroke::new(1.5 * scale, if hot { theme::text() } else { theme::text_dim() }),
    ));
}
