//! Shared fold chrome: workspace sections, analyzer regions and settings
//! headings.

use egui::{Rect, Response, Vec2};

use crate::theme;

/// A pane's fold button — a workspace section's or an analyzer region's — as
/// a square with a double chevron pointing the way the pane's edge will move
/// when it is clicked: toward the edge on an open pane, out of the rail on a
/// folded one.
///
/// Double, where a settings heading's is single, because the two say
/// different things: a heading's chevron is the STATE of its section (at the
/// name while folded, down while open), which is how disclosure carets read
/// everywhere, while this one is the ACTION, which is how a sidebar toggle
/// reads. One glyph for both made an open pane's arrow and an open section's
/// look like they disagreed.
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
    for step in [-1.0, 1.0] {
        paint_chevron(
            &painter,
            cell.center() + direction * step * 2.0 * scale,
            direction,
            hot,
            scale,
        );
    }
}

/// The fold mark every fold in the editor draws — once on a settings heading
/// or subsection, twice on a pane's [`paint_fold`] button — centred on
/// `center` and pointing along `direction`, brighter while `hot`.
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
