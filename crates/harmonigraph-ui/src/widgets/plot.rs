//! Shared geometry and numeric entry for the settings' direct-manipulation controls.
use crate::theme;
use egui::{Color32, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};
use std::ops::RangeInclusive;

/// A focused plot handle's arrows and what each moves it, bare a coarse step
/// and with Shift a fine one: one table for what is claimed from the host and
/// what is read, matched exactly as the shell matches. Any other chord on an
/// arrow, and every other key, a DAW's Space among them, goes to the host.
const HANDLE_ARROWS: [(egui::KeyboardShortcut, Vec2); 8] = {
    use egui::{Key, KeyboardShortcut, Modifiers};
    const fn nudge(modifiers: Modifiers, key: Key, x: f32, y: f32) -> (KeyboardShortcut, Vec2) {
        let step = if modifiers.shift { 0.001 } else { 0.01 };
        (KeyboardShortcut::new(modifiers, key), Vec2::new(x * step, y * step))
    }
    let (none, shift) = (Modifiers::NONE, Modifiers::SHIFT);
    [
        nudge(none, Key::ArrowLeft, -1.0, 0.0),
        nudge(none, Key::ArrowRight, 1.0, 0.0),
        nudge(none, Key::ArrowUp, 0.0, 1.0),
        nudge(none, Key::ArrowDown, 0.0, -1.0),
        nudge(shift, Key::ArrowLeft, -1.0, 0.0),
        nudge(shift, Key::ArrowRight, 1.0, 0.0),
        nudge(shift, Key::ArrowUp, 0.0, 1.0),
        nudge(shift, Key::ArrowDown, 0.0, -1.0),
    ]
};

/// Bare Escape, which egui ends a focused handle's focus on: the editor's
/// while a handle holds the focus, as it was before the shell's key rule.
const HANDLE_ESCAPE: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Escape);

pub(super) struct Plot {
    pub rect: Rect,
    pub response: Response,
    fields: Rect,
}
impl Plot {
    /// Keep the picture a stable size beside exact values. There is no stacked
    /// layout for a narrow pane: the dock holds the settings column at the width
    /// this needs ([`theme::SETTINGS_MIN_CONTENT`]) wherever the window can.
    pub fn with_fields(ui: &mut Ui, label: &str, count: usize) -> Self {
        Self::new(ui, Some(label), count, false)
    }
    pub fn without_label(ui: &mut Ui, count: usize) -> Self {
        Self::new(ui, None, count, false)
    }
    pub fn square_with_fields(ui: &mut Ui, label: &str, count: usize) -> Self {
        Self::new(ui, Some(label), count, true)
    }
    fn new(ui: &mut Ui, label: Option<&str>, count: usize, square: bool) -> Self {
        let scale = theme::ui_scale(ui.ctx());
        let width = super::bar::bar_width(ui);
        let gap = ui.spacing().item_spacing.y;
        let title_beside = label.is_some() && count < 3;
        if title_beside {
            super::label::group_space(ui);
        } else if let Some(label) = label {
            super::label(ui, label);
        }
        let rows = count + usize::from(title_beside);
        let fields_height = (rows as f32 * (theme::row_height(scale) + gap) - gap).max(0.0);
        let (bounds, response) =
            ui.allocate_exact_size(Vec2::new(width, height(scale)), Sense::hover());
        let well = Rect::from_min_size(
            bounds.min,
            Vec2::new(if square { height(scale) } else { 112.0 * scale }, height(scale)),
        );
        ui.painter().rect_filled(well, super::bar::bar_radius(scale), theme::well());
        let mut fields = Rect::from_min_max(
            egui::pos2(well.right() + gap, bounds.center().y - fields_height / 2.0),
            egui::pos2(bounds.right(), bounds.center().y + fields_height / 2.0),
        );
        if let Some(label) = label.filter(|_| title_beside) {
            let mut title_rect = fields;
            title_rect.min.x += super::bar::BAR_TEXT_PAD * scale;
            let mut title = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(response.id.with("title"))
                    .max_rect(title_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            title
                .allocate_ui_with_layout(
                    Vec2::new(title_rect.width(), theme::row_height(scale)),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| ui.add(egui::Label::new(label).truncate()),
                )
                .inner
                .on_hover_text(label);
            fields.min.y += theme::row_height(scale) + gap;
        }
        // A 4pt handle plus its outline leaves a small visible margin at an endpoint.
        Self { rect: well.shrink(6.0 * scale), response, fields }
    }
    pub fn fields<R>(&self, ui: &mut Ui, draw: impl FnOnce(&mut Ui) -> R) -> R {
        // The outer control already allocated the whole rect. A child prevents
        // the value column from advancing its parent's cursor a second time.
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(self.response.id.with("values"))
                .max_rect(self.fields)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(self.fields.intersect(ui.clip_rect()));
        draw(&mut child)
    }
    pub fn point(&self, x: f32, y: f32) -> Pos2 {
        egui::pos2(
            self.rect.left() + x * self.rect.width(),
            self.rect.bottom() - y * self.rect.height(),
        )
    }
    pub fn line(&self, ui: &Ui, points: Vec<Pos2>, color: Color32) {
        let painter = ui.painter().with_clip_rect(self.rect.expand(2.0).intersect(ui.clip_rect()));
        theme::round_ended(&painter, points, Stroke::new(theme::LINE_WIDTH, color));
    }
    /// A straight line pressed out on the plot's empty space, as the two
    /// plot points it runs between, from the press to the pointer, for as long
    /// as the drag lasts. Registered before the handles, which then take any
    /// press that lands on one of them. The press point is held for the drag
    /// alone, as a bar holds its grab ([`super::bar::drag`]).
    pub fn stroke(&self, ui: &mut Ui) -> Option<(Vec2, Vec2)> {
        let scale = theme::ui_scale(ui.ctx());
        let id = self.response.id.with("stroke");
        let response = ui.interact(self.rect.expand(6.0 * scale), id, Sense::drag());
        let at = |p: Pos2| {
            egui::vec2(
                ((p.x - self.rect.left()) / self.rect.width().max(1.0)).clamp(0.0, 1.0),
                ((self.rect.bottom() - p.y) / self.rect.height().max(1.0)).clamp(0.0, 1.0),
            )
        };
        super::bar::drag(ui, &response, at).map(|(from, live)| (from, at(live)))
    }
    /// Separate handle IDs hold their identity even when values or curves cross.
    /// Arrow keys offer one-axis editing; Shift makes the step finer.
    pub fn handle(&self, ui: &mut Ui, key: &str, x: f32, y: f32) -> (Response, Option<Vec2>) {
        let at = self.point(x, y);
        let scale = theme::ui_scale(ui.ctx());
        let mut response = ui.interact(
            Rect::from_center_size(at, Vec2::splat(16.0 * scale)),
            self.response.id.with(key),
            Sense::click_and_drag(),
        );
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        let mut next = None;
        if response.dragged() {
            if let Some(p) = response.interact_pointer_pos() {
                next = Some(egui::vec2(
                    ((p.x - self.rect.left()) / self.rect.width().max(1.0)).clamp(0.0, 1.0),
                    ((self.rect.bottom() - p.y) / self.rect.height().max(1.0)).clamp(0.0, 1.0),
                ));
            }
        }
        // Egui's focus, not the window's: claimed while the host has the
        // keyboard too, so the first arrow after clicking back finds it the
        // editor's. Read under the same test, since a key reaches the editor
        // only while its window has the keyboard.
        if super::holds_focus(&response) {
            // Locked, or egui would spend each arrow moving focus to a
            // neighbouring widget as well.
            let arrows = egui::EventFilter {
                horizontal_arrows: true,
                vertical_arrows: true,
                ..Default::default()
            };
            ui.memory_mut(|m| m.set_focus_lock_filter(response.id, arrows));
            crate::keys::claim(ui.ctx(), &[HANDLE_ESCAPE]);
            let taken = crate::keys::take_each(ui.ctx(), &HANDLE_ARROWS.map(|(chord, _)| chord));
            let delta = taken
                .iter()
                .filter_map(|chord| HANDLE_ARROWS.iter().find(|(c, _)| c == chord))
                .fold(Vec2::ZERO, |sum, (_, step)| sum + *step);
            if delta != Vec2::ZERO {
                next = Some((egui::vec2(x, y) + delta).clamp(Vec2::ZERO, Vec2::splat(1.0)));
            }
        }
        if next.is_some() {
            response.mark_changed();
        }
        (response.on_hover_text(key), next)
    }
    /// Paint after every input has been applied, including numeric entry.
    pub fn dot(&self, ui: &Ui, x: f32, y: f32) {
        let scale = theme::ui_scale(ui.ctx());
        let painter =
            // The dot's radius, half its ring, and a pixel of feather.
            ui.painter().with_clip_rect(
                self.rect
                    .expand((4.0 + 0.5 * theme::LINE_WIDTH) * scale + 1.0)
                    .intersect(ui.clip_rect()),
            );
        painter.circle_filled(self.point(x, y), 4.0 * scale, theme::accent());
        painter.circle_stroke(
            self.point(x, y),
            4.0 * scale,
            Stroke::new(theme::LINE_WIDTH * scale, theme::text()),
        );
    }
}

/// A familiar scalar slider beside the combined gesture in the picture.
pub(super) fn value_bar(
    ui: &mut Ui,
    (value, fresh): (&mut f32, f32),
    range: RangeInclusive<f32>,
    labels: [&str; 2],
    scale: f32,
    suffix: &str,
) -> Response {
    let decimals = match suffix {
        " ms" => 0,
        "%" | " pp" | "°" | "¢" => 1,
        _ => 2,
    };
    super::ValueBar::new(value, range, labels[0])
        .caption(labels[1])
        .unit(scale, suffix)
        .decimals(decimals)
        .default_value(fresh)
        .show(ui)
}

pub(super) fn curve(plot: &Plot, ui: &Ui, sample: impl Fn(f32) -> (f32, f32), color: Color32) {
    plot.line(
        ui,
        (0..=64)
            .map(|i| {
                let (x, y) = sample(i as f32 / 64.0);
                plot.point(x, y)
            })
            .collect(),
        color,
    );
}

pub(crate) fn height(scale: f32) -> f32 {
    // Three slider rows and the two row gaps between them.
    (theme::ROW_HEIGHT * 3.0 + theme::ROW_GAP * 2.0) * scale
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::probe::{events_into, themed};

    /// A clicked handle claims the chord it reads, an arrow nudges it by that
    /// chord's step and leaves the focus on it, so the next arrow is still the
    /// handle's; a chord it does not read is not claimed, and goes to the host.
    #[test]
    fn a_clicked_handle_claims_its_arrows_and_keeps_them_through_a_nudge() {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let ctx = themed();
        let screen = egui::vec2(400.0, 400.0);
        let rect = Rect::from_min_size(Pos2::ZERO, screen);
        let mut at = Pos2::ZERO;
        let mut claimed = Vec::new();
        let mut moved = None;
        let mut frame = |events| {
            events_into(&ctx, screen, rect, events, |ui| {
                let response = ui.allocate_response(egui::vec2(200.0, 100.0), Sense::hover());
                // Directly above the neighbour, where an arrow would send the focus.
                at = response.rect.left_top() + egui::vec2(20.0, 90.0);
                let plot = Plot { rect: response.rect, response, fields: Rect::NOTHING };
                moved = plot.handle(ui, "handle", 0.1, 0.1).1;
                let _ = ui.button("neighbour");
                claimed = crate::keys::claimed(ui.ctx());
            });
            (at, claimed.clone(), moved)
        };
        let (at, _, _) = frame(vec![]);
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(vec![egui::Event::PointerMoved(at)]);
        frame(vec![press(true)]);
        frame(vec![press(false)]);
        let fine_down = KeyboardShortcut::new(Modifiers::SHIFT, Key::ArrowDown);
        let (_, claimed, _) = frame(vec![]);
        assert!(claimed.contains(&fine_down), "the clicked handle did not claim Shift-Down");
        let command_down = KeyboardShortcut::new(Modifiers::COMMAND, Key::ArrowDown);
        assert!(!claimed.contains(&command_down), "it claimed a chord it does not read");

        let (_, _, moved) = frame(vec![egui::Event::Key {
            key: Key::ArrowDown,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::SHIFT,
        }]);
        let moved = moved.expect("Shift-Down did not nudge the handle");
        assert!((moved.y - (0.1 - 0.001)).abs() < 1e-6 && moved.x == 0.1, "{moved:?}");
        // A bare arrow, which egui would also spend moving the focus on.
        let (_, _, moved) = frame(vec![egui::Event::Key {
            key: Key::ArrowDown,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        let moved = moved.expect("Down did not nudge the handle");
        assert!((moved.y - (0.1 - 0.01)).abs() < 1e-6, "{moved:?}");
        let (_, claimed, _) = frame(vec![]);
        assert!(claimed.contains(&fine_down), "the arrow moved the focus off the handle");
    }
}
