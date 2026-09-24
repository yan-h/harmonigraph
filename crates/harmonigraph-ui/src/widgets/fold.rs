//! Shared fold chrome: workspace sections, analyzer regions and settings
//! headings.

use egui::{Rect, Response, Vec2};

use crate::theme;

pub(crate) fn paint_fold(ui: &egui::Ui, response: &Response, cell: Rect, direction: Vec2) {
    let hot = response.hovered() || response.has_focus();
    let painter = ui.painter_at(cell);
    let scale = theme::ui_scale(ui.ctx());
    let radius = egui::CornerRadius::same(theme::control_radius(scale));
    // The header's hit cell is taller than its controls. Keep that forgiving
    // target, but center the visible button on the same row as its neighbours.
    let button = Rect::from_center_size(
        cell.center(),
        Vec2::splat(theme::row_height(scale)).min(cell.size()),
    );
    // An ordinary button's fills, so the square reads against the panel, the
    // rail and a black picture alike, and hover is carried by fill alone.
    painter.rect_filled(
        button,
        radius,
        if response.is_pointer_button_down_on() {
            theme::accent_active()
        } else if hot {
            theme::widget_hover()
        } else {
            theme::widget()
        },
    );
    paint_chevron(&painter, cell.center(), direction, hot, scale);
}

/// The fold mark every fold in the editor draws — a workspace section's
/// button, an analyzer region's, a settings heading and a subsection alike —
/// centred on `center` and pointing along `direction`, brighter while `hot`.
pub(crate) fn paint_chevron(
    painter: &egui::Painter,
    center: egui::Pos2,
    direction: Vec2,
    hot: bool,
    scale: f32,
) {
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

/// [`paint_chevron`] as an `egui::CollapsingHeader` icon, in place of egui's
/// filled triangle: pointing at the name while folded, turning down as the
/// header opens.
pub(crate) fn fold_icon(ui: &mut egui::Ui, openness: f32, response: &Response) {
    let hot = response.hovered() || response.has_focus();
    let direction = egui::emath::Rot2::from_angle(openness * std::f32::consts::FRAC_PI_2) * Vec2::X;
    paint_chevron(ui.painter(), response.rect.center(), direction, hot, theme::ui_scale(ui.ctx()));
}
