//! Custom controls. `ValueBar` is the workhorse: a flat, DAW-style
//! parameter bar (drag anywhere to set, double-click to type a value)
//! that replaces egui's rail-and-knob `Slider` + separate `DragValue`.

use std::ops::RangeInclusive;

use egui::{
    Align2, CornerRadius, Key, Response, Sense, TextEdit, TextStyle, Ui, Vec2,
};

use crate::theme;

pub struct ValueBar<'a> {
    value: &'a mut f32,
    range: RangeInclusive<f32>,
    label: &'a str,
    /// Ease the low end of the range (geometric when min > 0, cubic
    /// otherwise). Matches the intent of the old logarithmic sliders.
    eased: bool,
    decimals: usize,
    integer: bool,
}

impl<'a> ValueBar<'a> {
    pub fn new(value: &'a mut f32, range: RangeInclusive<f32>, label: &'a str) -> Self {
        ValueBar { value, range, label, eased: false, decimals: 2, integer: false }
    }

    pub fn eased(mut self, on: bool) -> Self {
        self.eased = on;
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
    fn from_t(&self, t: f32) -> f32 {
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
        let height = 20.0;
        let width = ui.available_width();
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
        let edit_id = response.id.with("edit");

        // ---- Text-entry mode (double-click) ------------------------------
        if let Some(mut text) = ui.data(|d| d.get_temp::<String>(edit_id)) {
            let output = ui.put(
                rect,
                TextEdit::singleline(&mut text)
                    .font(TextStyle::Body)
                    .horizontal_align(egui::Align::Center),
            );
            let committed = ui.input(|i| i.key_pressed(Key::Enter));
            let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
            if committed || cancelled || output.lost_focus() {
                if !cancelled {
                    if let Ok(v) = text.trim().parse::<f32>() {
                        let v = v.clamp(self.min(), self.max());
                        *self.value = if self.integer { v.round() } else { v };
                        response.mark_changed();
                    }
                }
                ui.data_mut(|d| d.remove_temp::<String>(edit_id));
            } else {
                ui.data_mut(|d| d.insert_temp(edit_id, text));
            }
            return response;
        }

        // ---- Interaction ---------------------------------------------------
        if response.double_clicked() {
            ui.data_mut(|d| d.insert_temp(edit_id, self.format(*self.value)));
            return response;
        }
        // Drag-to-set only (no click-jump): a stray click can't yank a
        // carefully tuned parameter, and it can't fight the double-click.
        if response.dragged() {
            if let Some(pointer) = response.interact_pointer_pos() {
                let t = (pointer.x - rect.left()) / rect.width().max(1.0);
                let new_value = self.from_t(t);
                if new_value != *self.value {
                    *self.value = new_value;
                    response.mark_changed();
                }
            }
        }

        // ---- Paint ----------------------------------------------------------
        let radius = CornerRadius::same(2);
        let painter = ui.painter();
        painter.rect_filled(rect, radius, theme::WELL);

        let t = self.to_t(*self.value);
        let fill_color = if response.dragged() {
            theme::ACCENT_FILL_DRAG
        } else if response.hovered() {
            theme::ACCENT_FILL_HOVER
        } else {
            theme::ACCENT_FILL
        };
        let mut fill = rect;
        fill.set_width(rect.width() * t);
        painter.rect_filled(fill, radius, fill_color);

        let text_color = if response.hovered() || response.dragged() {
            theme::TEXT
        } else {
            theme::TEXT_DIM
        };
        painter.text(
            rect.left_center() + Vec2::new(8.0, 0.0),
            Align2::LEFT_CENTER,
            self.label,
            TextStyle::Body.resolve(ui.style()),
            text_color,
        );
        painter.text(
            rect.right_center() - Vec2::new(8.0, 0.0),
            Align2::RIGHT_CENTER,
            self.format(*self.value),
            TextStyle::Body.resolve(ui.style()),
            theme::TEXT,
        );

        response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
    }
}
