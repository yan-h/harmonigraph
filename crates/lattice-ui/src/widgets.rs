//! Custom controls. `ValueBar` is the workhorse: a flat, DAW-style
//! parameter bar (drag anywhere to set, double-click to type a value)
//! that replaces egui's rail-and-knob `Slider` + separate `DragValue`.

use std::ops::RangeInclusive;

use egui::{
    Align2, CornerRadius, Key, Response, Sense, TextEdit, TextStyle, Ui, Vec2,
};

use crate::theme;

/// Row height of a ValueBar (taller than the theme's interact_size: these
/// are the primary controls and carry two text runs).
const BAR_HEIGHT: f32 = 20.0;
/// Corner rounding, deliberately tighter than theme::WIDGET_RADIUS so the
/// bars read as a meter, not a button.
const BAR_RADIUS: u8 = 2;

pub struct ValueBar<'a> {
    value: &'a mut f32,
    range: RangeInclusive<f32>,
    label: &'a str,
    /// Ease the low end of the range (geometric when min > 0, cubic
    /// otherwise). Matches the intent of the old logarithmic sliders.
    eased: bool,
    decimals: usize,
    integer: bool,
    /// Read-only: shows the value but takes no drag/type input and paints
    /// dimmed. Used for the major-third bar while meantone mode drives it.
    locked: bool,
}

impl<'a> ValueBar<'a> {
    pub fn new(value: &'a mut f32, range: RangeInclusive<f32>, label: &'a str) -> Self {
        ValueBar { value, range, label, eased: false, decimals: 2, integer: false, locked: false }
    }

    pub fn eased(mut self, on: bool) -> Self {
        self.eased = on;
        self
    }

    /// Render the bar read-only (dimmed, non-interactive).
    pub fn locked(mut self, on: bool) -> Self {
        self.locked = on;
        self
    }

    pub fn decimals(mut self, n: usize) -> Self {
        self.decimals = n;
        self
    }

    pub fn integer(mut self) -> Self {
        self.integer = true;
        self.decimals = 0;
        self
    }

    fn min(&self) -> f32 {
        *self.range.start()
    }

    fn max(&self) -> f32 {
        *self.range.end()
    }

    /// Value -> fill fraction in [0, 1].
    fn to_t(&self, v: f32) -> f32 {
        let (min, max) = (self.min(), self.max());
        let v = v.clamp(min, max);
        if !self.eased {
            (v - min) / (max - min)
        } else if min > 0.0 {
            (v / min).ln() / (max / min).ln()
        } else {
            ((v - min) / (max - min)).cbrt()
        }
    }

    /// Fill fraction -> value, with integer snapping.
    fn value_at(&self, t: f32) -> f32 {
        let (min, max) = (self.min(), self.max());
        let t = t.clamp(0.0, 1.0);
        let v = if !self.eased {
            min + t * (max - min)
        } else if min > 0.0 {
            min * (max / min).powf(t)
        } else {
            min + t.powi(3) * (max - min)
        };
        if self.integer { v.round() } else { v }
    }

    fn format(&self, v: f32) -> String {
        format!("{:.*}", self.decimals, v)
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let width = ui.available_width();
        // Locked bars are read-only: sense hover (so a tooltip still works)
        // but not clicks/drags.
        let sense = if self.locked { Sense::hover() } else { Sense::click_and_drag() };
        let (rect, mut response) = ui.allocate_exact_size(Vec2::new(width, BAR_HEIGHT), sense);

        // Skip all editing (text-entry + drag) while locked; the value is
        // driven from elsewhere (meantone derives the third from the fifth).
        if !self.locked {
            let edit_id = response.id.with("edit");
            let focus_id = edit_id.with("focus_pending");

            // ---- Text-entry mode (double-click) --------------------------
            if let Some(mut text) = ui.data(|d| d.get_temp::<String>(edit_id)) {
                let output = ui.put(
                    rect,
                    TextEdit::singleline(&mut text)
                        .font(TextStyle::Monospace)
                        .horizontal_align(egui::Align::Center),
                );
                // TextEdit never takes focus by itself; grab it on the first
                // edit-mode frame so typing (and focus-loss commit) works at
                // all.
                if ui.data(|d| d.get_temp::<bool>(focus_id)).unwrap_or(false) {
                    output.request_focus();
                    ui.data_mut(|d| d.remove_temp::<bool>(focus_id));
                }
                // Escape cancels; everything that drops focus (Enter included
                // — egui surrenders TextEdit focus on both) commits. Enter is
                // NOT checked globally: that would commit every bar in edit
                // mode at once, focused or not.
                let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
                if cancelled || output.lost_focus() {
                    if !cancelled {
                        if let Ok(v) = text.trim().parse::<f32>() {
                            // Reject NaN/inf: NaN survives clamp() and would
                            // poison the param (and the host's automation lane
                            // in the plugin) in a state the bar can't display
                            // or drag back out of.
                            if v.is_finite() {
                                let v = v.clamp(self.min(), self.max());
                                *self.value = if self.integer { v.round() } else { v };
                                response.mark_changed();
                            }
                        }
                    }
                    ui.data_mut(|d| d.remove_temp::<String>(edit_id));
                } else {
                    ui.data_mut(|d| d.insert_temp(edit_id, text));
                }
                return response;
            }

            // ---- Interaction ---------------------------------------------
            if response.double_clicked() {
                ui.data_mut(|d| d.insert_temp(edit_id, self.format(*self.value)));
                ui.data_mut(|d| d.insert_temp(focus_id, true));
                return response;
            }
            // Drag-to-set only (no click-jump): a stray click can't yank a
            // carefully tuned parameter, and it can't fight the double-click.
            if response.dragged() {
                if let Some(pointer) = response.interact_pointer_pos() {
                    let t = (pointer.x - rect.left()) / rect.width().max(1.0);
                    let new_value = self.value_at(t);
                    if new_value != *self.value {
                        *self.value = new_value;
                        response.mark_changed();
                    }
                }
            }
        }

        // ---- Paint ----------------------------------------------------------
        let radius = CornerRadius::same(BAR_RADIUS);
        let painter = ui.painter();
        painter.rect_filled(rect, radius, theme::well());

        let t = self.to_t(*self.value);
        let fill_color = if self.locked {
            // Dimmed fill: reads as inactive/derived, not something to drag.
            theme::accent_fill().gamma_multiply(0.4)
        } else if response.dragged() {
            theme::accent_fill_drag()
        } else if response.hovered() {
            theme::accent_fill_hover()
        } else {
            theme::accent_fill()
        };
        let mut fill = rect;
        fill.set_width(rect.width() * t);
        painter.rect_filled(fill, radius, fill_color);

        let text_color = if !self.locked && (response.hovered() || response.dragged()) {
            theme::text()
        } else {
            theme::text_dim()
        };
        painter.text(
            rect.left_center() + Vec2::new(8.0, 0.0),
            Align2::LEFT_CENTER,
            self.label,
            TextStyle::Body.resolve(ui.style()),
            text_color,
        );
        // Values in monospace: digits align and don't wiggle as they
        // change. Dimmed too while locked, to match the fill.
        painter.text(
            rect.right_center() - Vec2::new(8.0, 0.0),
            Align2::RIGHT_CENTER,
            self.format(*self.value),
            TextStyle::Monospace.resolve(ui.style()),
            if self.locked { theme::text_dim() } else { theme::text() },
        );

        if self.locked {
            response
        } else {
            response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trips(range: RangeInclusive<f32>, eased: bool) {
        let mut value = 0.0;
        let bar = ValueBar::new(&mut value, range, "test").eased(eased);
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            let v = bar.value_at(t);
            assert!(
                (bar.to_t(v) - t).abs() < 1e-4,
                "t {t} -> value {v} -> t {}",
                bar.to_t(v)
            );
        }
    }

    #[test]
    fn linear_positions_round_trip() {
        round_trips(-600.0..=600.0, false);
    }

    #[test]
    fn eased_positions_round_trip() {
        // min > 0 exercises the geometric branch; min == 0 the cubic one.
        round_trips(0.001..=49.999, true);
        round_trips(0.0..=100.0, true);
    }

    #[test]
    fn integer_bars_snap() {
        let mut value = 0.0;
        let bar = ValueBar::new(&mut value, 1.0..=8.0, "test").integer();
        let v = bar.value_at(0.37);
        assert_eq!(v, v.round());
    }
}
