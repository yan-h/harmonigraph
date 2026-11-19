//! Shared fold chrome: workspace sections, analyzer regions and settings
//! headings.

use egui::{Rect, Response, Vec2};

use crate::theme;

/// Which way a folded pane's [`paint_rail`] runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Rail {
    /// Down a column: the fold button at the top, the title turned to read
    /// upward under it.
    Column,
    /// Across a row: the fold button at the left, the title after it.
    Row,
}

/// Where a caller's [`paint_rail`] still differs from the other's. The two
/// rails were copies that drifted apart, and which look they share is Yan's to
/// choose (#1851), so each caller keeps the one it had.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RailLook {
    /// The side of the fold button's square cell, at the rail's start.
    pub button: f32,
    /// Points between the button's cell and the title, as drawn (not scaled
    /// here).
    pub gap: f32,
    /// Whether the rail names itself, "Expand" and its title, to a hover and
    /// to AccessKit, and leaves out a title too long for it rather than
    /// letting the rail's end clip it.
    pub named: bool,
}

/// The rail a folded pane leaves, a workspace section's or an analyzer
/// region's, as one control: the whole rail answers a click by unfolding the
/// pane, with the fold button at its start pointing `direction` and the
/// pane's `title` beside it, as the caller's [`RailLook`] says. Returns
/// whether it was clicked.
pub(crate) fn paint_rail(
    ui: &egui::Ui,
    rect: Rect,
    id: egui::Id,
    rail: Rail,
    direction: Vec2,
    title: &str,
    look: RailLook,
) -> bool {
    let RailLook { button, gap, named } = look;
    let response = ui.interact(rect, id, egui::Sense::click());
    let expand = format!("Expand {title}");
    if named {
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, expand.clone())
        });
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, egui::CornerRadius::ZERO, theme::header());
    // The button's cell stands at the rail's start, centred across it.
    let (center, length) = match rail {
        Rail::Column => (egui::pos2(rect.center().x, rect.top() + button * 0.5), rect.height()),
        Rail::Row => (egui::pos2(rect.left() + button * 0.5, rect.center().y), rect.width()),
    };
    paint_fold(ui, &response, Rect::from_center_size(center, Vec2::splat(button)), direction);
    let galley = painter.layout_no_wrap(
        title.to_owned(),
        egui::TextStyle::Heading.resolve(ui.style()),
        theme::text(),
    );
    if !named || galley.size().x + button + 2.0 * gap <= length {
        let (anchor, angle) = match rail {
            Rail::Column => (
                egui::pos2(
                    rect.center().x - galley.size().y * 0.5,
                    rect.top() + button + gap + galley.size().x,
                ),
                -std::f32::consts::FRAC_PI_2,
            ),
            Rail::Row => (
                egui::pos2(rect.left() + button + gap, rect.center().y - galley.size().y * 0.5),
                0.0,
            ),
        };
        painter.add(egui::epaint::TextShape::new(anchor, galley, theme::text()).with_angle(angle));
    }
    if named {
        response.on_hover_text(expand).clicked()
    } else {
        response.clicked()
    }
}

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
    theme::round_ended(
        painter,
        vec![
            center - direction * 2.0 * scale + cross * 4.0 * scale,
            center + direction * 2.0 * scale,
            center - direction * 2.0 * scale - cross * 4.0 * scale,
        ],
        egui::Stroke::new(
            theme::LINE_WIDTH * scale,
            if hot { theme::text() } else { theme::text_dim() },
        ),
    );
}
