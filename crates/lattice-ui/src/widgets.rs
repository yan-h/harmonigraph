//! Custom controls. `ValueBar` is the workhorse: a flat, DAW-style
//! parameter bar (drag anywhere to set, double-click to type a value)
//! that replaces egui's rail-and-knob `Slider` + separate `DragValue`.

use std::ops::RangeInclusive;

use egui::{
    Align2, CornerRadius, Key, Response, Sense, TextEdit, TextStyle, Ui, Vec2,
};

use crate::theme;

/// Track size of a [`toggle_switch`] pill.
const SWITCH_SIZE: Vec2 = Vec2::new(26.0, 15.0);

/// A labeled sliding-knob switch for boolean *modes* (Meantone, Learn).
/// Buttons with a `selected` fill read exactly like the momentary preset
/// buttons they sit next to (Just, 12-TET); the pill-and-knob shape is
/// unmistakably persistent state.
///
/// Toggle vs checkbox, the house rule: a switch means "this mode is
/// ENGAGED" — an ongoing behavior with side effects (Learn keeps
/// rewriting tuning params; Meantone locks the third), especially next
/// to action buttons it could be confused with. A checkbox means
/// "include this element" — a display preference among peers in a
/// settings stack (Fill, Peak hold, Note labels, ...). When adding a
/// boolean control, default to a checkbox unless it's a mode that keeps
/// acting after the click.
pub fn toggle_switch(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        TextStyle::Button.resolve(ui.style()),
        theme::text(),
    );
    let gap = 6.0;
    let desired = Vec2::new(
        SWITCH_SIZE.x + gap + galley.size().x,
        SWITCH_SIZE.y.max(galley.size().y),
    );
    let (rect, mut response) = ui.allocate_exact_size(desired, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, label)
    });

    if ui.is_rect_visible(rect) {
        // Same animation the stock egui toggle demo uses: the knob glides,
        // the track cross-fades.
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let track = egui::Rect::from_min_size(
            egui::pos2(rect.left(), rect.center().y - SWITCH_SIZE.y / 2.0),
            SWITCH_SIZE,
        );
        let radius = track.height() / 2.0;
        let mix = |a: egui::Color32, b: egui::Color32| -> egui::Color32 {
            egui::lerp(egui::Rgba::from(a)..=egui::Rgba::from(b), t).into()
        };
        let painter = ui.painter();
        painter.rect_filled(track, radius, mix(theme::well(), theme::accent_active()));
        if response.hovered() || response.dragged() {
            painter.rect_stroke(
                track,
                radius,
                egui::Stroke::new(1.0, theme::accent_edge()),
                egui::StrokeKind::Inside,
            );
        }
        let knob_x = egui::lerp(
            (track.left() + radius)..=(track.right() - radius),
            t,
        );
        painter.circle_filled(
            egui::pos2(knob_x, track.center().y),
            radius - 2.5,
            theme::text(),
        );
        painter.galley(
            egui::pos2(track.right() + gap, rect.center().y - galley.size().y / 2.0),
            galley,
            theme::text(),
        );
    }
    response
}

/// A record control that doubles as its own live indicator. Press to arm; the
/// dot shows the state — a hollow ring when idle, a breathing dot while armed
/// and waiting for the transport, a solid dot while actually capturing. Press
/// again to stop (which the On-disarm trigger needs, and which serves as a
/// manual early stop under the others).
///
/// Recording is a mode with ongoing side effects, the reason a plain switch was
/// chosen before — but its "off" is usually reached automatically (the
/// transport stopping, a loop ending), so a two-way slider overstated the
/// manual control. A record button that pulses while writing says "capturing a
/// file" at least as clearly, without pretending you drag it both ways.
pub fn record_button(ui: &mut Ui, on: &mut bool, rolling: bool, label: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        TextStyle::Button.resolve(ui.style()),
        theme::text(),
    );
    let dot_r = 5.0;
    let gap = 8.0;
    let pad = Vec2::new(10.0, 5.0);
    let inner = Vec2::new(dot_r * 2.0 + gap + galley.size().x, galley.size().y.max(dot_r * 2.0));
    let (rect, mut response) = ui.allocate_exact_size(inner + pad * 2.0, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, ui.is_enabled(), *on, label)
    });

    if ui.is_rect_visible(rect) {
        // Breathing while armed-and-waiting; solid once the transport rolls.
        // Keep repainting so the breath animates even when nothing else moves.
        let alpha = if *on && !rolling {
            let t = ui.ctx().input(|i| i.time);
            ui.ctx().request_repaint();
            0.4 + 0.6 * (0.5 + 0.5 * (t * std::f64::consts::TAU * 1.1).sin()) as f32
        } else {
            1.0
        };
        let painter = ui.painter();
        let bg = if response.hovered() { theme::panel() } else { theme::well() };
        painter.rect_filled(rect, CornerRadius::same(4), bg);
        if response.hovered() {
            painter.rect_stroke(
                rect,
                CornerRadius::same(4),
                egui::Stroke::new(1.0, theme::accent_edge()),
                egui::StrokeKind::Inside,
            );
        }
        let dot = egui::pos2(rect.left() + pad.x + dot_r, rect.center().y);
        if *on {
            painter.circle_filled(dot, dot_r, theme::armed().gamma_multiply(alpha));
        } else {
            painter.circle_stroke(dot, dot_r - 0.75, egui::Stroke::new(1.5, theme::text_dim()));
        }
        painter.galley(
            egui::pos2(rect.left() + pad.x + dot_r * 2.0 + gap, rect.center().y - galley.size().y / 2.0),
            galley,
            theme::text(),
        );
    }
    response
}

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

/// A horizontal row sized up front to framed-button height.
///
/// Plain `ui.horizontal*` starts its row at `interact_size.y`, which is
/// shorter than a padded button: egui centers early widgets in that short
/// row, then grows the row downward under the first button it meets, so a
/// bare label (or checkbox) next to buttons sits a few pixels above their
/// text. Starting the row at button height centers everything on one line.
pub fn button_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    button_row_impl(ui, false, add)
}

/// [`button_row`], wrapping onto new rows like `horizontal_wrapped`.
pub fn button_row_wrapped<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    button_row_impl(ui, true, add)
}

/// A labelled row of mutually-exclusive choices for `value`: the standard
/// shape of every enum setting in the settings panes.
///
/// Each option is `(value, label, hover hint)`; an empty hint means no
/// tooltip. Adding a variant to a style enum is then one line here rather
/// than another copy of the label/loop/`selectable_value` scaffolding.
pub fn choice_row<T: Copy + PartialEq>(
    ui: &mut Ui,
    name: &str,
    value: &mut T,
    options: &[(T, &str, &str)],
) {
    button_row_wrapped(ui, |ui| {
        ui.label(name);
        for (option, label, hint) in options {
            let response = ui.selectable_value(value, *option, *label);
            if !hint.is_empty() {
                response.on_hover_text(*hint);
            }
        }
    });
}

fn button_row_impl<R>(ui: &mut Ui, wrap: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
    let height =
        ui.text_style_height(&TextStyle::Button) + 2.0 * ui.spacing().button_padding.y;
    ui.scope(|ui| {
        // The row ui reads this as its initial height; buttons already
        // size to at least it, so only the shorter widgets move.
        ui.style_mut().spacing.interact_size.y = height;
        if wrap {
            ui.horizontal_wrapped(add).inner
        } else {
            ui.horizontal(add).inner
        }
    })
    .inner
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

    /// A row-builder: lays out a control row, calling back to add its label
    /// and button. Aliased so clippy doesn't flag the nested `dyn FnMut`.
    type RowFn = fn(&mut Ui, &mut dyn FnMut(&mut Ui));

    /// Label center minus button center in a row built by `row`, under the
    /// theme's geometry (short interact_size, padded buttons). Not
    /// `__run_test_ui`: that empties the fonts, text measures zero-height,
    /// and the too-short row this guards against never happens.
    fn row_offset(row: RowFn) -> f32 {
        let mut offset = 0.0;
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(Default::default(), |ui| {
            ui.style_mut().spacing.interact_size.y = 17.0;
            ui.style_mut().spacing.button_padding = Vec2::new(9.0, 4.0);
            row(ui, &mut |ui| {
                let label = ui.label("Node style").rect.center().y;
                let button = ui.button("Steady").rect.center().y;
                offset = label - button;
            });
        });
        offset
    }

    /// A bare label centers on the button text in both row variants. The
    /// companion assert shows plain `horizontal` still misaligns them —
    /// when an egui upgrade fixes row sizing upstream, that assert fails
    /// and this whole workaround becomes deletable.
    #[test]
    fn button_row_centers_label_with_button_text() {
        let plain = row_offset(|ui, add| {
            ui.horizontal(|ui| add(ui));
        });
        assert!(plain < -1.0, "egui centers rows itself now ({plain}); drop button_row?");

        let fixed = row_offset(|ui, add| {
            button_row(ui, |ui| add(ui));
        });
        assert!(fixed.abs() < 0.5, "button_row label off by {fixed}px");

        let wrapped = row_offset(|ui, add| {
            button_row_wrapped(ui, |ui| add(ui));
        });
        assert!(wrapped.abs() < 0.5, "button_row_wrapped label off by {wrapped}px");
    }
}
