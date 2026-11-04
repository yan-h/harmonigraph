//! Square buttons bearing a painted glyph rather than a word, for actions
//! repeated on every row of a list, where a word per row would crowd out
//! the row's own name.

use egui::{Atom, Rect, Response, Ui, Vec2};

use crate::theme;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Icon {
    Up,
    Down,
    Delete,
}

/// A square button as tall as a row, bearing `icon` and named `label` for
/// screen readers; the caller gives it the hover text. Painted inside the
/// enabled scope, so a disabled one fades as a disabled word would.
pub(crate) fn icon_button(ui: &mut Ui, icon: Icon, enabled: bool, label: &str) -> Response {
    ui.add_enabled_ui(enabled, |ui| {
        let glyph = ui.id().with(("icon", label));
        let response = egui::Button::new(Atom::custom(glyph, Vec2::splat(ui.spacing().icon_width)))
            .min_size(Vec2::splat(ui.spacing().interact_size.y))
            .atom_ui(ui);
        if let Some(rect) = response.rect(glyph) {
            let hot = response.response.hovered() || response.response.has_focus();
            paint(ui, icon, rect, if hot { theme::text() } else { theme::text_dim() });
        }
        let response = response.response;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
        });
        response
    })
    .inner
}

/// The glyphs in the fold chevrons' stroke. Up and down are arrows rather
/// than chevrons, because a chevron here already means a fold's state.
fn paint(ui: &Ui, icon: Icon, rect: Rect, color: egui::Color32) {
    let scale = theme::ui_scale(ui.ctx());
    let stroke = egui::Stroke::new(theme::LINE_WIDTH * scale, color);
    let c = rect.center();
    let at = |x: f32, y: f32| c + egui::vec2(x, y) * scale;
    let strokes = match icon {
        Icon::Up => {
            [vec![at(0.0, 4.5), at(0.0, -4.0)], vec![at(-4.0, 0.0), at(0.0, -4.0), at(4.0, 0.0)]]
        }
        Icon::Down => {
            [vec![at(0.0, -4.5), at(0.0, 4.0)], vec![at(-4.0, 0.0), at(0.0, 4.0), at(4.0, 0.0)]]
        }
        Icon::Delete => [vec![at(-3.5, -3.5), at(3.5, 3.5)], vec![at(-3.5, 3.5), at(3.5, -3.5)]],
    };
    for points in strokes {
        theme::round_ended(ui.painter(), points, stroke);
    }
}
