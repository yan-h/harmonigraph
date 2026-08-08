//! Custom controls. `ValueBar` is the workhorse: a flat, DAW-style
//! parameter bar (drag anywhere to set, double-click to type a value)
//! that replaces egui's rail-and-knob `Slider` + separate `DragValue`.
//! `RangeBar` is its two-handle sibling, for a pair of values that bound a
//! span rather than one value on a scale. `OctaveStrip` is the octave wheel's
//! own — two counts and the profile they produce, in one row.

use std::ops::RangeInclusive;

use egui::{CornerRadius, Key, Response, Sense, TextEdit, TextStyle, Ui, Vec2};
use harmonigraph_scene::{
    clamp_wheel, hue_circle, octave_layout, pitch_ramp_lut, PitchGradient, ViewConfig,
    DEFAULT_CENTER, DEFAULT_COUNT, HUE_CIRCLE_N, MAX_SPAN, MIN_SPAN, PITCH_LUT_N,
};

use crate::panes::scene_color;
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
    let scale = theme::ui_scale(ui.ctx());
    let switch = SWITCH_SIZE * scale;
    let gap = 6.0 * scale;
    let desired = Vec2::new(switch.x + gap + galley.size().x, switch.y.max(galley.size().y));
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
            egui::pos2(rect.left(), rect.center().y - switch.y / 2.0),
            switch,
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
            radius - 2.5 * scale,
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
    let scale = theme::ui_scale(ui.ctx());
    let dot_r = 5.0 * scale;
    let gap = 8.0 * scale;
    let pad = Vec2::new(10.0, 5.0) * scale;
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
        painter.rect_filled(rect, CornerRadius::same(theme::control_radius(scale)), bg);
        if response.hovered() {
            painter.rect_stroke(
                rect,
                CornerRadius::same(theme::control_radius(scale)),
                egui::Stroke::new(1.0, theme::accent_edge()),
                egui::StrokeKind::Inside,
            );
        }
        let dot = egui::pos2(rect.left() + pad.x + dot_r, rect.center().y);
        if *on {
            painter.circle_filled(dot, dot_r, theme::armed().gamma_multiply(alpha));
        } else {
            painter.circle_stroke(
                dot,
                dot_r - 0.75 * scale,
                egui::Stroke::new(1.5, theme::text_dim()),
            );
        }
        painter.galley(
            egui::pos2(rect.left() + pad.x + dot_r * 2.0 + gap, rect.center().y - galley.size().y / 2.0),
            galley,
            theme::text(),
        );
    }
    response
}

// The bars' own geometry, written at the design size — scale 1.0. Each is
// multiplied by `theme::ui_scale` where it is drawn, so a bar shrinks with the
// type inside it rather than keeping a 20-point row around 9-point text.

/// Row height of a ValueBar (taller than the theme's interact_size: these
/// are the primary controls and carry two text runs).
const BAR_HEIGHT: f32 = 20.0;
/// Corner rounding of the bar track — the shared control radius, so a bar and
/// a button beside it round the same.
fn bar_radius(scale: f32) -> u8 {
    theme::control_radius(scale)
}
/// Inset of a bar's name and its value readout from the bar's own ends.
const BAR_TEXT_PAD: f32 = 8.0;
/// Clear space kept between the two, so an elided name stops short of the
/// number rather than touching it.
const BAR_LABEL_GAP: f32 = 6.0;

/// How wide a bar draws: the width the layout offers, but never past the
/// visible edge of the pane. Shared by [`ValueBar`] and [`RangeBar`], so every
/// bar in a settings column comes out the same length and they all narrow
/// together as the column does.
///
/// `available_width` alone is not enough, and the reason is worth stating
/// because nothing about it is visible here: egui's
/// `Region::expand_to_include_rect` unions `max_rect` as well as `min_rect`, so
/// any control that overruns the column widens the region for everything AFTER
/// it, and a bar sizing itself from the layout inherits the overrun as a floor
/// it cannot shrink past. Each bar's minimum length is then the width of the
/// widest thing above it — several different minimums down one pane, the bars
/// under a wide control running their value readout off the pane edge while the
/// ones above compress properly.
///
/// [`button_row`] keeps rows and their button labels inside the column, which is
/// what removes the usual sources; this covers the ones with nowhere to wrap to,
/// like the record button and the Options field in a very narrow Video pane.
///
/// The limit comes from [`crate::panes::pane_content_right`], which the pane
/// records on the way in. Deliberately not the clip rect: that is the tab BODY,
/// a [`theme::pane_inner_margin`] wider than the content box on each side, so a
/// bar clamped to it comes out a margin longer than its neighbours and flush on
/// the pane border. Outside a pane — the widget's own tests — there is nothing
/// to hand a value over, and the clip is then the honest fallback.
fn bar_width(ui: &Ui) -> f32 {
    let right = ui
        .data(|d| d.get_temp::<f32>(crate::panes::pane_content_right()))
        .unwrap_or_else(|| ui.clip_rect().right());
    ui.available_width().min(right - ui.cursor().left()).max(0.0)
}

pub struct ValueBar<'a> {
    value: &'a mut f32,
    range: RangeInclusive<f32>,
    label: &'a str,
    /// Ease the low end of the range (geometric when min > 0, cubic
    /// otherwise), so the fine end of a wide range is draggable.
    eased: bool,
    decimals: usize,
    integer: bool,
    /// A word saying what is DRIVING the value, drawn at full brightness
    /// ahead of the bar's name. Used by the bar of an axis a tempered-out
    /// comma derives (the major third under meantone, the harmonic seventh
    /// under marvel): the number in the bar is then not the one the param
    /// holds, and the badge is what says so.
    badge: Option<&'a str>,
    /// `(target, tolerance)`: a value the bar is pulled to while an edit
    /// lands within `tolerance` of it (see [`ValueBar::magnet`]).
    magnet: Option<(f32, f32)>,
    /// How the value READS OUT, when plain decimals won't say it (see
    /// [`ValueBar::display`]). Never how it is typed in.
    display: Option<fn(f32) -> String>,
}

impl<'a> ValueBar<'a> {
    pub fn new(value: &'a mut f32, range: RangeInclusive<f32>, label: &'a str) -> Self {
        ValueBar {
            value,
            range,
            label,
            eased: false,
            decimals: 2,
            integer: false,
            badge: None,
            magnet: None,
            display: None,
        }
    }

    pub fn eased(mut self, on: bool) -> Self {
        self.eased = on;
        self
    }

    /// Mark the bar as driven from elsewhere, with `word` at the front of
    /// its name. The bar stays draggable: what drives it is a MODE, and the
    /// bar is where that mode is let go of.
    pub fn badge(mut self, word: &'a str) -> Self {
        self.badge = Some(word);
        self
    }

    /// Pull edits to `target` while they land within `tolerance` of it, so
    /// the bar holds the target exactly until a drag pulls clear of the
    /// window — the meantone third, held to four perfect fifths until it is
    /// dragged off them.
    ///
    /// Applied where the value is DECIDED, ahead of the paint, which is the
    /// whole point: a caller correcting the value afterwards would have to
    /// let the bar draw one frame at the pointer first, and a bar that
    /// follows the pointer for a frame and snaps back on the next is a bar
    /// that visibly does not snap while you drag it slowly.
    ///
    /// The value the caller gets back is snapped too, so "did this edit
    /// escape the magnet" is the same question as "did the value change".
    pub fn magnet(mut self, target: f32, tolerance: f32) -> Self {
        self.magnet = Some((target, tolerance));
        self
    }

    /// `v`, pulled to the magnet's target if it is inside the window.
    fn snapped(&self, v: f32) -> f32 {
        match self.magnet {
            Some((target, tolerance)) if (v - target).abs() <= tolerance => target,
            _ => v,
        }
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

    /// Read the value out as this rather than as plain decimals — for a
    /// value whose UNIT changes with its size, which a fixed suffix in the
    /// label can't say (seconds that become minutes and seconds).
    ///
    /// Display only. Typing still takes a bare number in the bar's own
    /// units, and double-click seeds the box with one, so whatever a
    /// formatter does to the readout the value stays typeable.
    pub fn display(mut self, display: fn(f32) -> String) -> Self {
        self.display = Some(display);
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

    /// The value as text to TYPE: always a bare number, so the text-entry
    /// box round-trips through `parse::<f32>` whatever the readout says.
    fn format(&self, v: f32) -> String {
        format!("{:.*}", self.decimals, v)
    }

    /// The value as text to READ.
    fn shown(&self, v: f32) -> String {
        match self.display {
            Some(display) => display(v),
            None => self.format(v),
        }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let scale = theme::ui_scale(ui.ctx());
        let width = bar_width(ui);
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::new(width, BAR_HEIGHT * scale), Sense::click_and_drag());

        let edit_id = response.id.with("edit");
        let focus_id = edit_id.with("focus_pending");

        // ---- Text-entry mode (double-click) ---------------------------------
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
                            let v = self.snapped(v.clamp(self.min(), self.max()));
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

        // ---- Interaction ----------------------------------------------------
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
                let new_value = self.snapped(self.value_at(t));
                if new_value != *self.value {
                    *self.value = new_value;
                    response.mark_changed();
                }
            }
        }

        // ---- Paint ----------------------------------------------------------
        let radius = CornerRadius::same(bar_radius(scale));
        let painter = ui.painter();
        painter.rect_filled(rect, radius, theme::well());

        let t = self.to_t(*self.value);
        let fill_color = if response.dragged() {
            theme::accent_fill_drag()
        } else if response.hovered() {
            theme::accent_fill_hover()
        } else {
            theme::accent_fill()
        };
        let mut fill = rect;
        fill.set_width(rect.width() * t);
        painter.rect_filled(fill, radius, fill_color);

        let text_color = if response.hovered() || response.dragged() {
            theme::text()
        } else {
            theme::text_dim()
        };
        // The value is laid out first and the name takes what is left, elided.
        // The number is what the bar is FOR — a name that runs over it, or out
        // past the pane edge, costs the reading the control exists to give.
        // Values in monospace: digits align and don't wiggle as they
        // change.
        let mono = TextStyle::Monospace.resolve(ui.style());
        let value = painter.layout_no_wrap(self.shown(*self.value), mono.clone(), theme::text());
        // Room kept clear for the readout, measured from the widest one the
        // bar's RANGE can produce rather than from the number currently in it.
        // Taking it from the current number makes the name re-elide the moment
        // the value gains a digit — the name wobbling under the pointer
        // mid-drag, which is exactly what the monospace face buys for the
        // digits themselves. The ends bound it for a plain decimal readout;
        // the current value is in the max as well so that a `display` whose
        // length is not monotonic in the value can still never be overlapped.
        let reserve = [self.shown(self.min()), self.shown(self.max()), self.shown(*self.value)]
            .into_iter()
            .map(|text| painter.layout_no_wrap(text, mono.clone(), theme::text()).size().x)
            .fold(0.0f32, f32::max);
        // A badged bar wears the word at the FRONT of its name, because that is
        // the end elision cannot reach. Spelled into the tail it is the first
        // thing dropped in a narrow column — and the bar then reads as an
        // ordinary one that ran out of room, which is worse than saying
        // nothing, since the unbadged name is the shorter of the two and draws
        // in full at the same width. Here rather than in the caller's label so
        // every badged bar gets it, and so the name is the same string badged
        // or not. Full brightness while the name beside it is dim: the badge
        // is state, not more label.
        let body = TextStyle::Body.resolve(ui.style());
        let mut job = egui::text::LayoutJob::default();
        if let Some(word) = self.badge {
            let format = egui::TextFormat::simple(body.clone(), theme::text());
            job.append(&format!("{word} · "), 0.0, format);
        }
        job.append(self.label, 0.0, egui::TextFormat::simple(body, text_color));
        let text_pad = BAR_TEXT_PAD * scale;
        job.wrap.max_width =
            (rect.width() - 2.0 * text_pad - BAR_LABEL_GAP * scale - reserve).max(0.0);
        job.wrap.max_rows = 1;
        job.wrap.overflow_character = Some('\u{2026}');
        let label = painter.layout_job(job);
        let centered = |galley: &egui::Galley, x: f32| {
            egui::pos2(x, rect.center().y - galley.size().y * 0.5)
        };
        painter.galley(centered(&label, rect.left() + text_pad), label, text_color);
        painter.galley(
            centered(&value, rect.right() - text_pad - value.size().x),
            value,
            theme::text(),
        );

        response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
    }
}

/// A read-only bar reporting how far something running in the background has
/// got: `fraction` of the track filled, `label` on the left and `value` read
/// out on the right.
///
/// The same shape as [`ValueBar`] — same track, fill, and text placement — so
/// a pane reads as one kind of control whether the number is one you set or
/// one you are being told. It senses hover only: there is nothing here to
/// drag, and a bar that moved under the pointer would claim there was.
///
/// `fraction` is `None` while the total is still unknown, and then the track
/// draws EMPTY rather than at zero — "no idea yet" and "none of it done" are
/// different things, and only one of them is a number.
///
/// Nothing keeps the name from re-eliding as the readout grows, unlike
/// `ValueBar`, which reserves the width of the widest value its range can
/// reach. There is no range to ask here, so a caller whose readout changes
/// width pads it to a fixed one instead (monospace, so that is enough).
pub fn progress_bar(ui: &mut Ui, fraction: Option<f32>, label: &str, value: &str) -> Response {
    let scale = theme::ui_scale(ui.ctx());
    let width = bar_width(ui);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, BAR_HEIGHT * scale), Sense::hover());

    let radius = CornerRadius::same(bar_radius(scale));
    let painter = ui.painter();
    painter.rect_filled(rect, radius, theme::well());
    if let Some(t) = fraction {
        let mut fill = rect;
        fill.set_width(rect.width() * t.clamp(0.0, 1.0));
        painter.rect_filled(fill, radius, theme::accent_fill());
    }

    // Value laid out first and the name elided into what is left, the order
    // and the reason `ValueBar` uses: the number is what the bar is for.
    let value = painter.layout_no_wrap(
        value.to_owned(),
        TextStyle::Monospace.resolve(ui.style()),
        theme::text(),
    );
    let mut job = egui::text::LayoutJob::simple_singleline(
        label.to_owned(),
        TextStyle::Body.resolve(ui.style()),
        theme::text_dim(),
    );
    let text_pad = BAR_TEXT_PAD * scale;
    job.wrap.max_width =
        (rect.width() - 2.0 * text_pad - BAR_LABEL_GAP * scale - value.size().x).max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.overflow_character = Some('\u{2026}');
    let label = painter.layout_job(job);
    let centered =
        |galley: &egui::Galley, x: f32| egui::pos2(x, rect.center().y - galley.size().y * 0.5);
    painter.galley(centered(&label, rect.left() + text_pad), label, theme::text_dim());
    painter.galley(
        centered(&value, rect.right() - text_pad - value.size().x),
        value,
        theme::text(),
    );
    response
}

/// How near a handle the pointer has to start for the drag to take that
/// handle rather than the span between them. Generous on purpose: grabbing
/// the span when you meant an end is the easy mistake, and the expensive one
/// — it moves BOTH values instead of the one you were aiming at.
///
/// The one length here the chrome scale leaves alone, because it is a reach
/// rather than a drawn thing: a bar dialled smaller is a smaller target, which
/// is the case for keeping the reach where it was rather than shrinking it
/// too. `HANDLE_REACH_SHARE` already stops it swallowing a narrow bar.
const GRAB_PX: f32 = 14.0;
/// Ceiling on that reach, as a share of the span from each side, so the two
/// handles can never claim the whole of a narrow range and leave nothing to
/// slide.
const HANDLE_REACH_SHARE: f32 = 0.35;
/// Width of a [`RangeBar`] handle grip.
const HANDLE_W: f32 = 6.0;
/// How far the value track is inset from the bar's ends, so a handle parked
/// at either limit still sits fully inside the bar with track visible past
/// it.
///
/// Without this the bar has no visible affordance in the state it starts
/// life in: the pitch range defaults to the FULL axis, which puts both
/// handles flush against the ends, under the corner rounding, where they
/// read as the bar's own border. The control then looks exactly like a
/// ValueBar filled to 100% — nothing about it says there is anything to take
/// hold of.
const HANDLE_INSET: f32 = HANDLE_W * 0.5 + 1.0;
/// Breathing room between a handle and its readout, and between a readout and
/// the bar's edge.
const TEXT_GAP: f32 = 5.0;

/// Which part of a [`RangeBar`] a drag took hold of. Decided once, at
/// drag-start, and remembered for the gesture — otherwise dragging one end
/// past the other would hand the drag to whichever handle is nearest now.
/// (`Default` is derived only to satisfy egui's `remove_temp` bound; the
/// value is always written by drag-start before anything reads it.)
#[derive(Clone, Copy, Default)]
enum Grab {
    #[default]
    Low,
    High,
    /// The whole span: `offset` is how far along it the pointer took hold,
    /// `width` how wide it was at that moment.
    ///
    /// Both are fixed for the whole gesture, and the span branch of `apply`
    /// reads neither end back. That is what makes squishing stable: deriving
    /// the width from the CURRENT pair instead would re-measure an already
    /// squished span every frame and shrink it further while the pointer sat
    /// perfectly still.
    Span { offset: f32, width: f32 },
}

impl Grab {
    /// What a drag starting at value `v` takes hold of: an end if the pointer
    /// is within `near` of one, otherwise the span between them, otherwise
    /// (again) the nearer end.
    ///
    /// That last fallback is the whole reason this is a function. A span that
    /// already fills the range has nowhere to slide, so panning it does
    /// nothing at all — and the range's default IS the full axis, so a bar
    /// that only panned from the middle would be dead exactly where everyone
    /// first meets it. When there's no room to pan, a middle drag takes the
    /// nearer end instead.
    fn at(v: f32, (lo, hi): (f32, f32), _range: (f32, f32), near: f32) -> Grab {
        // A handle's reach cannot eat the whole span, or a narrow range would
        // have no middle left to grab and could never be slid along the axis.
        let near = near.min((hi - lo) * HANDLE_REACH_SHARE);
        let (dl, dh) = ((v - lo).abs(), (v - hi).abs());
        if dl.min(dh) <= near {
            if dl <= dh { Grab::Low } else { Grab::High }
        } else if v > lo && v < hi {
            Grab::Span { offset: v - lo, width: hi - lo }
        } else if dl <= dh {
            Grab::Low
        } else {
            Grab::High
        }
    }

    /// Where the pair ends up when this grab is dragged to value `v`. Pure,
    /// so the invariants that actually matter — the ends never cross, the
    /// span never closes past `min_span`, and a slid span keeps its width
    /// while staying inside the range — are testable without a pointer.
    fn apply(self, v: f32, (lo, hi): (f32, f32), (min, max): (f32, f32), min_span: f32) -> (f32, f32) {
        match self {
            Grab::Low => (v.clamp(min, (hi - min_span).max(min)), hi),
            Grab::High => (lo, v.clamp((lo + min_span).min(max), max)),
            // Where the pointer wants the span, wall behavior aside. Running
            // past a wall pins the leading edge there and lets the trailing
            // edge carry on following the pointer, so the range squishes
            // against the end rather than refusing to move — down to
            // `min_span`. It springs back out on the way home, because this
            // reads only the gesture's own offset and width, never the
            // squished pair it produced.
            Grab::Span { offset, width } => {
                let (want_lo, want_hi) = (v - offset, v - offset + width);
                if want_lo < min {
                    (min, want_hi.clamp(min + min_span, max))
                } else if want_hi > max {
                    (want_lo.clamp(min, max - min_span), max)
                } else {
                    (want_lo, want_hi)
                }
            }
        }
    }
}

/// A two-handle [`ValueBar`]: one control for the pair of values that bound a
/// range. Drag either end to move it, drag between them to slide the whole
/// span at a fixed width, double-click to reset to the full range.
///
/// Positions are linear in the value, and that is the whole trick behind the
/// pitch-range control: its values are MIDI note numbers, and a scale linear
/// in MIDI note is by definition logarithmic in frequency. So the caller gets
/// a log-frequency control for free, and `display` formats each end however
/// suits it — the pitch range drags semitones and reads out Hz.
///
/// Double-click resets rather than opening text entry (ValueBar's use of the
/// gesture): a bar with two ends has no single value to type into it.
///
/// Unlabeled, unlike ValueBar. Each end reads out beside its own handle, which
/// is the only place a range's numbers mean anything — and a name across the
/// middle as well left three text runs competing in a 20pt row. The section
/// heading above the control does the naming.
pub struct RangeBar<'a> {
    low: &'a mut f32,
    high: &'a mut f32,
    range: RangeInclusive<f32>,
    /// Closest the two ends may come, in value units — the range can be
    /// narrowed but never collapsed.
    min_span: f32,
    /// Whether a drag lands on whole values only (see [`RangeBar::integer`]).
    integer: bool,
    display: fn(f32) -> String,
}

impl<'a> RangeBar<'a> {
    pub fn new(low: &'a mut f32, high: &'a mut f32, range: RangeInclusive<f32>) -> Self {
        RangeBar {
            low,
            high,
            range,
            min_span: 0.0,
            integer: false,
            display: |v| format!("{v:.2}"),
        }
    }

    pub fn min_span(mut self, span: f32) -> Self {
        self.min_span = span;
        self
    }

    /// Land on whole values only — for a range whose ends MEAN something at
    /// each step and whose readout says which one it is.
    ///
    /// The case for it is a range read out as a note name: left continuous,
    /// two ends a tenth of a semitone apart both read "C1" while one of them
    /// draws an indicator fewer, and an end exactly on an octave is
    /// unreachable except by luck.
    ///
    /// No `RangeBar` in the panes asks for this — the octave wheel is a count
    /// and a center, and snaps through [`ValueBar::integer`] instead. Kept as
    /// the pair to that one so a range whose ends MEAN something at each step
    /// has it available, and exercised by this module's tests.
    ///
    /// Snapped at the POINTER, ahead of the grab arithmetic, so the minimum
    /// span survives it: rounding the pair afterwards can take a semitone off
    /// a span that was exactly at the minimum, while rounding the value the
    /// gesture is reading leaves every bound it is clamped against whole.
    pub fn integer(mut self) -> Self {
        self.integer = true;
        self
    }

    /// How each end reads out (the bar itself never interprets the value).
    pub fn display(mut self, display: fn(f32) -> String) -> Self {
        self.display = display;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let scale = theme::ui_scale(ui.ctx());
        let width = bar_width(ui);
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::new(width, BAR_HEIGHT * scale), Sense::click_and_drag());
        let (min, max) = (*self.range.start(), *self.range.end());
        // Values live on an inset track, so both limits are positions a handle
        // can sit AT rather than edges it merges into. See HANDLE_INSET.
        let track = rect.shrink2(Vec2::new(HANDLE_INSET * scale, 0.0));
        let x_of =
            |v: f32| track.left() + track.width() * ((v - min) / (max - min)).clamp(0.0, 1.0);
        let value_at = |x: f32| {
            min + ((x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0) * (max - min)
        };

        // ---- Interaction ----------------------------------------------------
        let grab_id = response.id.with("grab");
        let near = GRAB_PX / track.width().max(1.0) * (max - min);
        if response.double_clicked() {
            *self.low = min;
            *self.high = max;
            response.mark_changed();
        }
        if response.dragged() {
            if let Some(p) = response.interact_pointer_pos() {
                let v = value_at(p.x);
                let v = if self.integer { v.round() } else { v };
                // Decided on the first frame of the gesture and remembered for
                // the rest of it, so dragging one end past the other doesn't
                // hand the drag to whichever handle is nearest now. Decided
                // HERE rather than under `drag_started` so a gesture whose
                // start frame was missed still does something.
                // Read and write are separate statements on purpose: nesting a
                // `data_mut` inside a `data` closure takes the context lock
                // twice, and nothing here is worth risking that on a path only
                // a real pointer reaches.
                let stored = ui.data(|d| d.get_temp::<Grab>(grab_id));
                let grab = match stored {
                    Some(grab) => grab,
                    None => {
                        let grab = Grab::at(v, (*self.low, *self.high), (min, max), near);
                        ui.data_mut(|d| d.insert_temp(grab_id, grab));
                        grab
                    }
                };
                let (lo, hi) = grab.apply(v, (*self.low, *self.high), (min, max), self.min_span);
                if lo != *self.low || hi != *self.high {
                    (*self.low, *self.high) = (lo, hi);
                    response.mark_changed();
                }
            }
        }
        if response.drag_stopped() {
            ui.data_mut(|d| d.remove_temp::<Grab>(grab_id));
        }

        // ---- Paint ----------------------------------------------------------
        let radius = CornerRadius::same(bar_radius(scale));
        let painter = ui.painter();
        painter.rect_filled(rect, radius, theme::well());

        let fill_color = if response.dragged() {
            theme::accent_fill_drag()
        } else if response.hovered() {
            theme::accent_fill_hover()
        } else {
            theme::accent_fill()
        };
        let (lx, hx) = (x_of(*self.low), x_of(*self.high));
        let mut span = rect;
        span.min.x = lx;
        span.max.x = hx;
        painter.rect_filled(span, radius, fill_color);

        // Each end's value beside its own handle. First choice is the empty
        // track outside the span, where a number sits on flat black and reads
        // cleanly; when the span has grown too close to that edge to leave
        // room, it moves inside instead, over the fill. (At the full range
        // there is no empty track at all, so both go inside.)
        let font = TextStyle::Monospace.resolve(ui.style());
        let handle_w = HANDLE_W * scale;
        let text_gap = TEXT_GAP * scale;
        let reach = handle_w * 0.5 + text_gap;
        for (x, value, outward) in
            [(lx, *self.low, -1.0f32), (hx, *self.high, 1.0f32)]
        {
            let galley = painter.layout_no_wrap((self.display)(value), font.clone(), theme::text());
            let w = galley.size().x;
            // Outside: the edge nearest the bar's own end. Inside: the other
            // side of the handle. Both are expressed as the text's LEFT edge.
            let outside = if outward < 0.0 { x - reach - w } else { x + reach };
            let inside = if outward < 0.0 { x + reach } else { x - reach - w };
            let fits = if outward < 0.0 {
                outside >= rect.left() + text_gap
            } else {
                outside + w <= rect.right() - text_gap
            };
            let left = if fits { outside } else { inside };
            // Never let a readout escape the bar, however cramped the row.
            let left = left.clamp(
                rect.left() + text_gap,
                (rect.right() - text_gap - w).max(rect.left() + text_gap),
            );
            let y = rect.center().y - galley.size().y * 0.5;
            painter.galley(egui::pos2(left, y), galley, theme::text());
        }

        // The handles go on top of everything, text included: they are the
        // part you operate, and a readout digit sliding under one is a better
        // outcome than a handle disappearing behind a digit. A flat light
        // thumb, no outline — a 1px dark stroke inside a 6px rounded rect
        // lands on fractional pixels and reads as a ragged edge, and the thumb
        // has plenty of contrast against both the filled span and the empty
        // track without one.
        for x in [lx, hx] {
            let grip = egui::Rect::from_center_size(
                egui::pos2(x, rect.center().y),
                Vec2::new(handle_w, rect.height() - 3.0 * scale),
            );
            let grip_radius = CornerRadius::same(theme::scaled_points(2, scale));
            painter.rect_filled(grip, grip_radius, theme::text());
        }

        // The cursor says which of the two gestures a press would start, so the
        // difference is visible BEFORE committing to a drag: an end resizes,
        // the middle picks the whole range up and slides it.
        let aimed_at = response
            .hover_pos()
            .map(|p| Grab::at(value_at(p.x), (*self.low, *self.high), (min, max), near));
        match aimed_at {
            Some(Grab::Span { .. }) => response.on_hover_cursor(egui::CursorIcon::Grab),
            Some(_) => response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal),
            None => response,
        }
    }
}

/// Gap between two of the octave strip's cells, so a wheel reads as a row of
/// separate octaves rather than one fill with steps in it.
const CELL_GAP: f32 = 1.0;

/// Shortest an octave strip cell is ever drawn. The thinnest extra there is
/// would come out under a pixel on a short bar, and a cell that is not there
/// says the octave is not either.
const CELL_MIN_H: f32 = 3.0;

/// Width of an octave strip's handles. Narrower than a [`RangeBar`]'s, because
/// this one sits ON a boundary between two cells and hides a slice of each,
/// and a slot is only a eleventh of the bar to begin with — but not under the
/// four points a handle needs to read as something to grab rather than as an
/// edge in the fill.
const STRIP_HANDLE_W: f32 = 4.0;

/// Which of the two counts a drag on the octave strip took hold of, decided on
/// the first frame of the gesture and remembered for the rest of it.
///
/// `Default` only because egui's temp-data store demands it of anything it can
/// remove; nothing reads the default, since the value is always written by
/// drag-start first.
///
/// BOTH variants carry the other count, so `apply` is a pure function of how
/// far out the pointer is and the gesture cannot read back a number it moved
/// itself. Each direction has its own way of moving one: raising the count
/// past the eleven-slice budget makes the extras yield, and taking the fringe
/// off a lone full-size octave opens the count to two (`clamp_wheel`'s answer
/// for an undrawable wheel). Either one read back mid-gesture makes dragging
/// out and home again a one-way trip.
#[derive(Clone, Copy)]
enum StripGrab {
    /// The full-size octaves, measured from the middle of the wheel.
    Count { extras: u32 },
    /// The fringe, measured from the edge of the count — so `count` is the
    /// gesture's own zero point as well as the number it must not move.
    Extras { count: u32 },
}

impl Default for StripGrab {
    fn default() -> Self {
        StripGrab::Extras { count: DEFAULT_COUNT }
    }
}

impl StripGrab {
    /// What a drag starting `reach` slots out from the middle of the wheel
    /// takes hold of: the full-size octaves while it is inside the handles,
    /// the fringe outside them.
    ///
    /// By REGION rather than by the nearer handle, which is what keeps the two
    /// apart at zero extras — there both handles sit on the wheel's outer edge
    /// and every press is equally near one, while the gesture a press there
    /// wants is unambiguous: outward is a fringe, inward is a count.
    fn at(reach: f32, count: u32, extras: u32) -> StripGrab {
        if reach <= count as f32 * 0.5 {
            StripGrab::Count { extras }
        } else {
            StripGrab::Extras { count }
        }
    }

    /// Where the two counts end up when this grab is dragged `reach` slots out
    /// from the middle. Pure, and a function of `reach` ALONE, so the things
    /// that actually matter — half a slot of travel per octave, a fringe
    /// measured from the edge of the count, a wheel that never overruns the
    /// budget, and a drag that comes home to where it started — are testable
    /// without a pointer.
    fn apply(self, reach: f32) -> (u32, u32) {
        // Half a slot per octave in both gestures: the count grows at both
        // ends at once, and so does the fringe.
        let (count, extras) = match self {
            StripGrab::Count { extras } => ((2.0 * reach).round() as u32, extras),
            StripGrab::Extras { count } => {
                let want = (reach - count as f32 * 0.5).max(0.0).round() as u32;
                // A lone full-size octave is only drawable with a pair to
                // flank it. Opening the count to two is what `clamp_wheel`
                // does about that, which is the right answer for a blob and
                // the wrong one for a gesture that is only holding the fringe:
                // the fringe stops at one instead, and the count stays where
                // the other gesture left it.
                let floor = if count < MIN_SPAN { 1 } else { 0 };
                (count, want.max(floor))
            }
        };
        clamp_wheel(count, extras)
    }
}

/// The octave wheel's two counts as one control: a strip of eleven slots — the
/// whole budget, since a wheel past eleven slices is one the boundary table
/// cannot hold — with the wheel drawn centered in it. The cells between the
/// two handles are the full-size octaves, the ones outside are the extras at
/// each end, and the empty track past those is the budget still unspent.
///
/// Drag inside the handles to set the count, outside them to set the extras;
/// double-click goes home to [`reset_wheel`]. Which one a drag takes is decided
/// by where it STARTS rather than by proximity to a handle, so the gesture
/// cannot change its mind halfway — and so the two are still told apart at
/// zero extras, where both handles sit on the wheel's outer edge and a
/// nearest-handle rule would have nothing to say.
///
/// Cell WIDTH is a slot of the budget; cell HEIGHT is how much of the ring
/// that octave takes AGAINST THE WIDEST ONE ON THIS WHEEL, so the full-size
/// octaves stand full height at every count and the axis reads "of a full-size
/// octave" rather than "of a turn". Two wheels cannot be compared by height —
/// a lone octave and one of eleven both draw full — and that is the right
/// trade, since against the turn the strip would spend its whole height on the
/// count and leave the fringe, which is what the bars below actually set, in
/// the bottom tenth.
///
/// Carrying the profile at all is the whole reason this is a strip and not two
/// bars: the fringe's size and blend have nowhere else to be seen before they
/// are dragged, and the thing they trade against — how much of the turn the
/// full-size octaves keep — is the same picture read the other way.
pub struct OctaveStrip<'a> {
    count: &'a mut u32,
    extras: &'a mut u32,
    /// The fringe knobs, read-only: they shape the profile the strip draws and
    /// have their own bars under it.
    size: f32,
    blend: f32,
}

/// The wheel a double-click on the strip goes home to: the one a fresh view
/// opens with.
///
/// Read off [`ViewConfig::default`] rather than restated as a literal, because
/// a reset that names its own pair drifts the moment the fresh wheel moves —
/// and it silently, since the strip goes on resetting to a wheel that is
/// merely no longer anyone's default. Landing on zero extras is the visible
/// end of that: the `Extra size` and `Extra blend` bars gate on there being a
/// fringe, so they gray out still holding the values the reset just orphaned.
fn reset_wheel() -> (u32, u32) {
    let fresh = ViewConfig::default();
    (fresh.octave_count, fresh.octave_extras)
}

impl<'a> OctaveStrip<'a> {
    pub fn new(count: &'a mut u32, extras: &'a mut u32, size: f32, blend: f32) -> Self {
        OctaveStrip { count, extras, size, blend }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let scale = theme::ui_scale(ui.ctx());
        let width = bar_width(ui);
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::new(width, BAR_HEIGHT * scale), Sense::click_and_drag());
        let slot = (rect.width() / MAX_SPAN as f32).max(1.0);
        let middle = rect.center().x;
        // How far from the middle of the wheel a point is, in slots — the one
        // measure both gestures are written in, since the wheel is symmetric
        // and both counts grow from its middle outward.
        let out = |x: f32| (x - middle).abs() / slot;

        // ---- Interaction ----------------------------------------------------
        let grab_id = response.id.with("grab");
        if response.double_clicked() {
            (*self.count, *self.extras) = reset_wheel();
            response.mark_changed();
        }
        if response.dragged() {
            if let Some(p) = response.interact_pointer_pos() {
                let reach = out(p.x);
                // Read and write are separate statements on purpose: nesting a
                // `data_mut` inside a `data` closure takes the context lock
                // twice, and nothing here is worth risking that on a path only
                // a real pointer reaches.
                let stored = ui.data(|d| d.get_temp::<StripGrab>(grab_id));
                let grab = match stored {
                    Some(grab) => grab,
                    None => {
                        // From where the press LANDED, which is not where the
                        // pointer is on the frame this first runs: egui calls
                        // a gesture a drag only once it has left a six-point
                        // click threshold, so by here it is already that far
                        // along, in the direction of travel. A `RangeBar`
                        // survives reading the live position because its
                        // handles carry fourteen points of reach; this control
                        // splits its two gestures on a hard line, and half of
                        // the drawn handle sits inside the six — so the live
                        // position hands "grab the handle, pull it outward",
                        // which is the count, to the fringe.
                        let start = ui
                            .ctx()
                            .input(|i| i.pointer.press_origin())
                            .map_or(reach, |p| out(p.x));
                        let grab = StripGrab::at(start, *self.count, *self.extras);
                        ui.data_mut(|d| d.insert_temp(grab_id, grab));
                        grab
                    }
                };
                let (count, extras) = grab.apply(reach);
                if (count, extras) != (*self.count, *self.extras) {
                    (*self.count, *self.extras) = (count, extras);
                    response.mark_changed();
                }
            }
        }
        if response.drag_stopped() {
            ui.data_mut(|d| d.remove_temp::<StripGrab>(grab_id));
        }

        // ---- Paint ----------------------------------------------------------
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(bar_radius(scale)), theme::well());

        let fill_color = if response.dragged() {
            theme::accent_fill_drag()
        } else if response.hovered() {
            theme::accent_fill_hover()
        } else {
            theme::accent_fill()
        };
        // The widths the wheel actually comes out at. The center pitch turns
        // each node's ring but never touches the widths, so which one this
        // asks for cannot show.
        let wheel = octave_layout(*self.count, DEFAULT_CENTER, *self.extras, self.size, self.blend);
        let span = wheel.span as usize;
        let cell_width = |i: usize| wheel.bounds[i + 1] - wheel.bounds[i];
        // Against the widest rather than against a whole turn, so the full-size
        // octaves stand at full height at every count and the strip's vertical
        // axis reads as "of a full-size octave".
        let widest = (0..span).map(cell_width).fold(0.0f32, f32::max).max(1e-6);
        let left = middle - 0.5 * span as f32 * slot;
        let gap = CELL_GAP * scale;
        let cell_radius = CornerRadius::same(theme::scaled_points(2, scale));
        for i in 0..span {
            let height = (rect.height() * cell_width(i) / widest).max(CELL_MIN_H * scale);
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(left + i as f32 * slot + 0.5 * gap, rect.bottom() - height),
                    egui::pos2(left + (i + 1) as f32 * slot - 0.5 * gap, rect.bottom()),
                ),
                cell_radius,
                fill_color,
            );
        }

        // Where the count ends and the fringe begins, which is also where a
        // drag changes its meaning — drawn whether or not there are extras
        // yet, since that is the edge you drag OUT from to get some.
        let handle_w = STRIP_HANDLE_W * scale;
        // Held inside the bar by half a handle, for the reason HANDLE_INSET
        // holds a RangeBar's ends off theirs: a wheel that spends the whole
        // budget puts this boundary on the bar's own edge, and a handle
        // centered there hangs half its width out over the pane, where the
        // part still on the bar reads as its border rather than as something
        // to grab. There is nothing outside it to drag toward at that width
        // anyway — a full count leaves no room for a fringe.
        let inset = 0.5 * handle_w;
        for side in [-1.0f32, 1.0] {
            let x = (middle + side * *self.count as f32 * 0.5 * slot)
                .clamp(rect.left() + inset, rect.right() - inset);
            painter.rect_filled(
                egui::Rect::from_center_size(
                    egui::pos2(x, rect.center().y),
                    Vec2::new(handle_w, rect.height() - 3.0 * scale),
                ),
                cell_radius,
                theme::text(),
            );
        }

        // Name and readout as a ValueBar wears them. The readout is the wheel
        // spelled out — the fringe, the count, the fringe — because the number
        // that matters depends on which of them is being dragged, and their
        // sum is what the eleven-slot budget is against.
        let text_color = if response.hovered() || response.dragged() {
            theme::text()
        } else {
            theme::text_dim()
        };
        let mono = TextStyle::Monospace.resolve(ui.style());
        let shown = if *self.extras > 0 {
            format!("{}+{}+{}", self.extras, self.count, self.extras)
        } else {
            format!("{}", self.count)
        };
        let value = painter.layout_no_wrap(shown, mono.clone(), theme::text());
        // Room kept clear for the widest readout the strip can produce rather
        // than for the one in it, so the name does not re-elide as the wheel
        // gains a digit mid-drag. Monospace, so any five characters measure
        // the same and a count of eleven (which can carry no extras) is
        // shorter than every fringed wheel there is.
        let reserve = painter.layout_no_wrap("0+0+0".into(), mono, theme::text()).size().x;
        let body = TextStyle::Body.resolve(ui.style());
        let mut job = egui::text::LayoutJob::default();
        job.append("Octaves", 0.0, egui::TextFormat::simple(body, text_color));
        let text_pad = BAR_TEXT_PAD * scale;
        job.wrap.max_width =
            (rect.width() - 2.0 * text_pad - BAR_LABEL_GAP * scale - reserve).max(0.0);
        job.wrap.max_rows = 1;
        job.wrap.overflow_character = Some('\u{2026}');
        let label = painter.layout_job(job);
        let centered =
            |galley: &egui::Galley, x: f32| egui::pos2(x, rect.center().y - galley.size().y * 0.5);
        painter.galley(centered(&label, rect.left() + text_pad), label, text_color);
        painter.galley(
            centered(&value, rect.right() - text_pad - value.size().x),
            value,
            theme::text(),
        );

        response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
    }
}

/// Segments the spectrum bar's track is drawn in. Twice [`HUE_CIRCLE_N`], so
/// every sample of the hue circle lands on a segment BOUNDARY where it is
/// drawn exactly, and only the vertices between two samples are interpolated.
const SPECTRUM_SEGMENTS: usize = HUE_CIRCLE_N * 2;

/// One turn of the hue circle, in degrees, and so the whole width of a
/// [`SpectrumBar`]'s track.
///
/// The bar can stand for the span knob's entire travel only because
/// [`PitchGradient::MAX_HUE_SPAN`] is exactly this. Widen that constant and
/// `sanitized` would accept spans the bar cannot draw: the handle would park at
/// the right edge and stop answering while the value went on growing, with
/// nothing to fail at compile time.
/// `the_spectrum_bar_track_is_a_whole_turn_of_the_span_knob` is what catches it.
const FULL_TURN: f32 = 360.0;

/// How far back the hues a gradient does not reach are held. Through alpha over
/// the well rather than a blend toward a fixed grey, so the bar sits on
/// whatever the pane is instead of ringing itself in a slightly wrong color.
const UNCLAIMED_ALPHA: f32 = 74.0 / 255.0;

/// Width of the flip button at the right end of a [`SpectrumBar`], taken out
/// of the row the bar already has rather than off a row of its own.
///
/// It costs the track that much travel, which is the whole trade and a cheap
/// one: the track stands for a whole turn at any length, so a shorter one is a
/// coarser drag and nothing else — 18pt of 400 is a twentieth of a degree per
/// pixel. A row costs 20pt of a column that already scrolls.
const FLIP_W: f32 = 18.0;

/// Which part of a [`SpectrumBar`]'s track a drag took hold of. Decided once,
/// at drag-start, and remembered for the gesture, exactly as [`Grab`] is: a
/// span dragged to nothing would otherwise hand the rest of the gesture to the
/// rotate branch the moment the handle reached the left edge.
#[derive(Clone, Copy, Default)]
enum SpectrumGrab {
    /// The far end of the arc — how far round the circle the gradient walks.
    #[default]
    Span,
    /// The circle itself, sliding under a fixed left edge. `held` is the hue
    /// that was under the pointer when the gesture started, and the whole
    /// gesture is "keep that hue under the pointer"; fixed for the gesture, so
    /// a turn never reads back the circle it is itself moving.
    Rotate { held: f32 },
}

/// The arc a double-click on the track goes home to: the one a fresh view
/// opens with.
///
/// Read off [`ViewConfig::default`] for the reason [`reset_wheel`] is, and
/// the drift it warns about is live here rather than hypothetical:
/// `ViewConfig::default` COMPOSES its gradient — a shorter arc over a
/// shallower brightness ramp — instead of taking `PitchGradient::default()`,
/// which is the type's own CIELAB-converted arc. Resetting to the type's
/// default lands the bar on a pair the plugin has never opened on, and the
/// bar carries no text entry to dial it back with, so the shipped arc would
/// be unrecoverable by gesture.
///
/// Only the two hue fields, because only those two are what the track sets.
fn reset_arc() -> (f32, f32) {
    let fresh = ViewConfig::default().pitch_gradient;
    (fresh.hue_start, fresh.hue_span)
}

/// The pitch gradient's hue arc, in one row: a full turn of the color circle
/// laid along a track, CUT at the arc's own start, with the stretch the
/// gradient walks filled from the left edge and the hues it does not reach
/// dimmed beyond it — and, in a gutter at the right end, the button that
/// reverses it.
///
/// Drag the handle to set how far round the circle the range walks; drag the
/// track to turn the whole circle under it; double-click to reset. Which
/// DIRECTION it runs is the flip button, not a gesture — see below.
///
/// **Cut at the start, which is what makes a circle fit on a bar.** Hue wraps,
/// so an arc laid on a fixed 0..360 track is drawn in two pieces whenever it
/// crosses the seam — and the default arc does, running 260 through 0 to 90,
/// which would put its two halves at opposite ends of the bar with the colors
/// it never uses in between. Pinning the START to the left edge instead means
/// the arc is one piece at every setting and always reads low-to-high, left to
/// right; what moves is the circle behind it. The cost is that the bar cannot
/// say where on the circle it is in absolute terms, which is a number nobody
/// reads a color off anyway — the track is painted in the colors themselves.
///
/// **It previews all four knobs, not just the one it sets.** The claimed
/// stretch is painted straight out of [`pitch_ramp_lut`], the same table the
/// lattice draws from, so brightness and chroma show up in it too and the
/// preview cannot drift from the picture. A swatch drawn from the widget's own
/// idea of the gradient would be a second definition of the color, wrong the
/// first time either changed. The dimmed remainder comes from [`hue_circle`]
/// at the gradient's BASE lightness and chroma — the middle of its brightness
/// ramp — so it reads as the same gradient continued rather than as decoration.
///
/// Which means it meets the claimed arc flush only when the ramp is FLAT: the
/// arc ends at the top of the ramp, and the remainder carries on from the
/// middle of it, so a steep ramp puts a step at the handle. Continuing the ramp
/// instead would close that step and pay for it at the top of the knob, where
/// an arc reaching `L*` 100 would dim out into a white band saying nothing
/// about which hues are left — and the remainder's whole job is to say that.
///
/// **The flip is a button because the track cannot carry the gesture.** The arc
/// is laid out from its own start, so both directions draw the same stretch of
/// color in the same place and there is nothing on the track to drag the other
/// way. It lives in the bar's own row rather than beside it because a settings
/// column is short of rows and not of width: the two things a reader wants
/// together are the arc and the direction it runs, and a row spent on one
/// button pushes every knob under it further down a pane that already scrolls.
///
/// **What the flip changes on screen is the sign, and the ramp's ends.** The
/// claimed stretch is the pitch ramp, low note at the left, so it reverses with
/// the gradient — which is exactly the change, drawn where the change is. The
/// readout spells the direction out on top of that, because at a span of zero
/// there is no stretch to reverse and a single-hue gradient with a brightness
/// ramp is a real setting.
pub struct SpectrumBar<'a> {
    gradient: &'a mut PitchGradient,
}

impl<'a> SpectrumBar<'a> {
    pub fn new(gradient: &'a mut PitchGradient) -> Self {
        SpectrumBar { gradient }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let scale = theme::ui_scale(ui.ctx());
        let width = bar_width(ui);
        // Space first, senses after: the row holds two controls side by side,
        // and each is interacted with over its OWN rectangle. That is what
        // keeps a press on the flip button out of the track. One rectangle
        // sensed for both would hand a sideways drag begun on the button
        // straight to the rotate branch, and asking the widget where the press
        // LANDED is no answer either — egui calls a press a drag only once it
        // has moved, by which time the pointer can be over the track.
        let (id, rect) = ui.allocate_space(Vec2::new(width, BAR_HEIGHT * scale));
        // A column too narrow to leave the track anything gives the button the
        // row: a coarse handle beats an unreachable one, but a button with no
        // width cannot be pressed at all.
        let flip_w = (FLIP_W * scale).min(rect.width());
        let split = rect.right() - flip_w;
        let track_rect =
            egui::Rect::from_min_max(rect.min, egui::pos2(split, rect.bottom()));
        let flip_rect = egui::Rect::from_min_max(egui::pos2(split, rect.top()), rect.max);
        let mut response = ui.interact(track_rect, id.with("track"), Sense::click_and_drag());
        let flip = ui
            .interact(flip_rect, id.with("flip"), Sense::click())
            .on_hover_text(
                "Run the spectrum the other way round the circle — the same \
                 colors, low and high swapped",
            );
        // The handle sits ON a position rather than between two, so the track
        // is inset by half of one at each end and both limits — a span of zero
        // and a whole turn — are places it can stand rather than edges it
        // merges into. Same reason as HANDLE_INSET.
        let track = track_rect.shrink2(Vec2::new(HANDLE_INSET * scale, 0.0));
        // Where a gradient puts itself on this track: which way round the
        // circle it runs, how much of the turn it claims, and where that leaves
        // the handle. A function rather than three bindings because the answer
        // is wanted TWICE — once for the gradient a gesture is aimed at, and
        // again for the one that gesture just wrote.
        let laid_out = |g: PitchGradient| {
            // A span of zero has no direction of its own, and opening rightward
            // is the useful reading: dragging the handle out of nothing then
            // grows an arc rather than needing the sign set first. `sanitized`
            // is what makes the test sound, by keeping -0.0 out of the field.
            let winding = if g.hue_span < 0.0 { -1.0f32 } else { 1.0 };
            let claimed = (g.hue_span / FULL_TURN).abs().clamp(0.0, 1.0);
            (winding, claimed, track.left() + track.width() * claimed)
        };

        // ---- Interaction ----------------------------------------------------
        // Ahead of the snapshot below, so the frame that flips is the frame
        // that draws it flipped — the same reason the paint re-reads the
        // gradient rather than the value a drag was aimed at.
        if flip.clicked() {
            // The arithmetic lives on the gradient rather than here: what a
            // flip IS — the far end becoming the near one, so the arc keeps its
            // place on the circle — is a property of the gradient that this bar
            // previews and a test pins, and a second spelling of it here is the
            // one that would drift.
            *self.gradient = self.gradient.flipped();
            response.mark_changed();
        }
        let aimed = self.gradient.sanitized();
        let (winding, _, handle_x) = laid_out(aimed);
        // Where a point across the track sits on the circle, as a signed
        // offset in degrees from the hue at the left edge.
        let offset_at = |x: f32| {
            ((x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0) * FULL_TURN * winding
        };
        let grab_id = response.id.with("spectrum_grab");
        if response.double_clicked() {
            let (hue_start, hue_span) = reset_arc();
            self.gradient.hue_start = hue_start;
            self.gradient.hue_span = hue_span;
            response.mark_changed();
        }
        if response.dragged() {
            if let Some(p) = response.interact_pointer_pos() {
                // Read and write as separate statements: nesting a `data_mut`
                // inside a `data` closure takes the context lock twice.
                let stored = ui.data(|d| d.get_temp::<SpectrumGrab>(grab_id));
                let grab = match stored {
                    Some(grab) => grab,
                    None => {
                        // Settled on the first live frame from where the
                        // pointer has REACHED, exactly as [`Grab`]'s is, so the
                        // two bars answer a mid-gesture jump the same way.
                        let grab = if (p.x - handle_x).abs() <= GRAB_PX * scale {
                            SpectrumGrab::Span
                        } else {
                            SpectrumGrab::Rotate { held: aimed.hue_start + offset_at(p.x) }
                        };
                        ui.data_mut(|d| d.insert_temp(grab_id, grab));
                        grab
                    }
                };
                let next = match grab {
                    // The magnitude only. Its SIGN is the flip button's, and
                    // leaving it there is what lets the handle reach zero
                    // without the arc turning inside out on the way past.
                    SpectrumGrab::Span => PitchGradient {
                        hue_span: winding * offset_at(p.x).abs(),
                        ..aimed
                    },
                    SpectrumGrab::Rotate { held } => PitchGradient {
                        hue_start: (held - offset_at(p.x)).rem_euclid(FULL_TURN),
                        ..aimed
                    },
                };
                if next != *self.gradient {
                    *self.gradient = next;
                    response.mark_changed();
                }
            }
        }
        if response.drag_stopped() {
            ui.data_mut(|d| d.remove_temp::<SpectrumGrab>(grab_id));
        }

        // ---- Paint ----------------------------------------------------------
        // The gradient read BACK, not the snapshot the gesture was aimed at. A
        // drag has just written it, and painting the value from before that
        // write leaves the handle, the arc, the strip and the readout a whole
        // frame behind the pointer for the length of the gesture — a step of
        // one value and about 17px at a brisk drag. ValueBar and RangeBar both
        // re-read their values here for the same reason.
        let g = self.gradient.sanitized();
        let (winding, claimed, handle_x) = laid_out(g);
        let lut = pitch_ramp_lut(g);
        let circle = hue_circle(g.lightness, g.chroma);
        let radius = CornerRadius::same(bar_radius(scale));
        let painter = ui.painter();
        // One well under both, so the track and the button read as one control
        // and not as a bar with something parked beside it.
        painter.rect_filled(rect, radius, theme::well());
        // The colors go inside that well, which is what rounds them: a mesh
        // has square corners, and the ring of well left showing reads as the
        // track's own border rather than as a gap.
        gradient_strip(painter, track_rect.shrink(scale.max(1.0)), SPECTRUM_SEGMENTS, |_, p| {
            if claimed > 0.0 && p <= claimed {
                // Along the gradient, not around the circle: the two agree by
                // construction here, and reading the table is what keeps them
                // agreeing if they ever stop.
                let f = (p / claimed).clamp(0.0, 1.0) * (PITCH_LUT_N - 1) as f32;
                let i0 = f.floor() as usize;
                scene_color(lut[i0].lerp(lut[(i0 + 1).min(PITCH_LUT_N - 1)], f - f.floor()), 1.0)
            } else {
                // The hues the gradient does not reach, held back far enough to
                // read as ground.
                let hue = g.hue_start + p * FULL_TURN * winding;
                let f = hue.rem_euclid(FULL_TURN) / FULL_TURN * HUE_CIRCLE_N as f32;
                let i0 = f.floor() as usize % HUE_CIRCLE_N;
                scene_color(
                    circle[i0].lerp(circle[(i0 + 1) % HUE_CIRCLE_N], f - f.floor()),
                    UNCLAIMED_ALPHA,
                )
            }
        });

        // How far round the circle the arc reaches, read out on the dimmed
        // side of the handle where it sits on flat color — and on the claimed
        // side when the arc has grown too wide to leave room there, which is
        // the same bargain a RangeBar's ends make. The sign is the direction,
        // and it is spelled out because the track cannot show it: an arc and
        // its flip claim exactly the same colors.
        let font = TextStyle::Monospace.resolve(ui.style());
        let text_color = if response.hovered() || response.dragged() {
            theme::text()
        } else {
            theme::text_dim()
        };
        let galley =
            painter.layout_no_wrap(format!("{:+.0}°", g.hue_span), font, text_color);
        let gap = TEXT_GAP * scale;
        let reach = HANDLE_W * 0.5 * scale + gap;
        let outside = handle_x + reach;
        let left = if outside + galley.size().x <= track_rect.right() - gap {
            outside
        } else {
            handle_x - reach - galley.size().x
        };
        let left = left.clamp(
            track_rect.left() + gap,
            (track_rect.right() - gap - galley.size().x).max(track_rect.left() + gap),
        );
        let y = track_rect.center().y - galley.size().y * 0.5;
        painter.galley(egui::pos2(left, y), galley, text_color);

        // The handle on top of everything, readout included: it is the part
        // you operate, and a digit sliding under it beats it disappearing
        // behind one.
        painter.rect_filled(
            egui::Rect::from_center_size(
                egui::pos2(handle_x, track_rect.center().y),
                Vec2::new(HANDLE_W * scale, track_rect.height() - 3.0 * scale),
            ),
            CornerRadius::same(theme::scaled_points(2, scale)),
            theme::text(),
        );

        // ---- The flip button ------------------------------------------------
        // A raised chip inside the well, so it reads as pressable against a
        // track that is painted in colors and cannot carry a resting fill of
        // its own.
        let chip = flip_rect.shrink(scale.max(1.0));
        painter.rect_filled(
            chip,
            radius,
            if flip.hovered() { theme::widget_hover() } else { theme::widget() },
        );
        let mark = if flip.hovered() { theme::text() } else { theme::text_dim() };
        flip_mark(painter, chip, mark, scale);

        // The cursor says which gesture a press would start before committing
        // to a drag, as a RangeBar's does: the handle resizes the arc, the
        // track turns the circle under it.
        match response.hover_pos() {
            Some(p) if (p.x - handle_x).abs() <= GRAB_PX * scale => {
                response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
            }
            Some(_) => response.on_hover_cursor(egui::CursorIcon::Grab),
            None => response,
        }
    }
}

/// The reverse mark on a [`SpectrumBar`]'s flip button: two arrows, one over
/// the other, pointing opposite ways.
///
/// Symmetric on purpose. A flip is its own undo and the track claims the same
/// colors either way round, so an arrow committing to a direction would be
/// pointing at nothing on screen; which way the arc currently runs is the sign
/// on the readout. Painted rather than set as a glyph, because the two product
/// faces are text faces and an arrow found in whichever fallback egui reaches
/// for would be the one piece of this UI drawn in an unknown font.
fn flip_mark(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32, scale: f32) {
    let head = 3.5 * scale; // length of an arrowhead
    let half = 2.5 * scale; // and half its width
    let gap = 3.0 * scale; // how far each arrow sits off the center line
    let shaft = (rect.width() - 4.0 * scale).max(head);
    let stroke = egui::Stroke::new((1.0 * scale).max(1.0), color);
    let (left, right) = (rect.center().x - shaft * 0.5, rect.center().x + shaft * 0.5);
    for (y, tip, tail) in [
        (rect.center().y - gap, left, right),
        (rect.center().y + gap, right, left),
    ] {
        painter.line_segment([egui::pos2(tail, y), egui::pos2(tip, y)], stroke);
        let back = tip + (tail - tip).signum() * head;
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(tip, y),
                egui::pos2(back, y - half),
                egui::pos2(back, y + half),
            ],
            color,
            egui::Stroke::NONE,
        ));
    }
}

/// A band of `segments + 1` colored columns across `rect`, each column's color
/// taken from `color` at its own index and at its position along the band (0 at
/// the left edge, 1 at the right), and interpolated between columns.
///
/// One builder for a [`SpectrumBar`]'s two bands — the track, whose color comes
/// off the hue circle and the pitch ramp either side of the handle, and the
/// pitch-order strip below it, which is the ramp end to end. A quad strip
/// written out twice is two places to get the vertex order or the first-column
/// case wrong, and the second copy is the one that quietly keeps the older
/// answer.
fn gradient_strip(
    painter: &egui::Painter,
    rect: egui::Rect,
    segments: usize,
    color: impl Fn(usize, f32) -> egui::Color32,
) {
    let mut mesh = egui::Mesh::default();
    for i in 0..=segments {
        let p = i as f32 / segments as f32;
        let x = rect.left() + rect.width() * p;
        let c = color(i, p);
        let v = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(x, rect.top()), c);
        mesh.colored_vertex(egui::pos2(x, rect.bottom()), c);
        if i > 0 {
            mesh.add_triangle(v - 2, v - 1, v);
            mesh.add_triangle(v - 1, v + 1, v);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// A horizontal row of controls in a settings column, sized up front to
/// framed-button height and wrapping onto further lines when the column is too
/// narrow to hold it.
///
/// Plain `ui.horizontal*` starts its row at `interact_size.y`, which is
/// shorter than a padded button: egui centers early widgets in that short
/// row, then grows the row downward under the first button it meets, so a
/// bare label (or checkbox) next to buttons sits a few pixels above their
/// text. Starting the row at button height centers everything on one line.
///
/// The single row helper, deliberately: a settings pane is a column whose width
/// the dock hands it, and a row that cannot wrap runs its last buttons out past
/// the pane edge where they can be neither read nor clicked. A non-wrapping
/// variant is only ever the wrong choice here, and having one to reach for is
/// what left the panes disagreeing about whether their buttons wrap at all —
/// Projection and Tilt overran a narrow column while Style and Palette wrapped.
///
/// Wrapping settles the harder half too, and not obviously: `horizontal_wrapped`
/// sets the row's wrap mode, so each BUTTON's own label wraps onto a second line
/// rather than extending past its frame. A single button too wide for the column
/// (Orthographic, at any column narrow enough) has nowhere to wrap TO, and would
/// otherwise overrun the pane whatever the row did — and take every control
/// under it along, since egui's `Region::expand_to_include_rect` unions
/// `max_rect` as well as `min_rect`.
pub fn button_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let height =
        ui.text_style_height(&TextStyle::Button) + 2.0 * ui.spacing().button_padding.y;
    ui.scope(|ui| {
        // The row ui reads this as its initial height; buttons already
        // size to at least it, so only the shorter widgets move.
        ui.style_mut().spacing.interact_size.y = height;
        ui.horizontal_wrapped(add).inner
    })
    .inner
}

/// A selectable option's label, set in MONOSPACE when the label is a bare
/// number ("1080", "16:9", "-1.5") and left alone when it is a word.
///
/// Numbers are monospace everywhere else in this UI — every bar readout, the
/// perf HUD, the lattice text — for the reason digits always want it: one
/// width per glyph, so a column of them lines up and none of them wiggles as
/// it changes. A row of number buttons is the same picture sideways. Set in
/// the proportional face, "1080" and "1440" come out different widths and the
/// row reads as four unrelated words rather than as a scale.
///
/// The FAMILY only, not [`TextStyle::Monospace`], which would also drop the
/// label to the monospace size and leave a number sitting smaller than the
/// words beside it.
///
/// Decided per label rather than per row, because rows mix: the frame-rate row
/// is "Uncapped" beside four numbers, and only the numbers want this.
pub fn option_label(label: &str) -> egui::RichText {
    let text = egui::RichText::new(label);
    // A digit, and nothing but digits and the punctuation numbers wear.
    let numeric = label.chars().any(|c| c.is_ascii_digit())
        && label.chars().all(|c| c.is_ascii_digit() || "+-±.,:/× ".contains(c));
    if numeric {
        text.family(egui::FontFamily::Monospace)
    } else {
        text
    }
}

/// A labelled row of mutually-exclusive choices for `value`: the standard
/// shape of every enum setting in the settings panes.
///
/// Each option is `(value, label, hover hint)`; an empty hint means no
/// tooltip. Adding a variant to a style enum is then one line here rather
/// than another copy of the label/loop/`selectable_value` scaffolding.
///
/// Number labels come out monospace — see [`option_label`], which the rows
/// built by hand out of `selectable_value` call for themselves.
///
/// A row is live or grayed as a WHOLE, and from an `add_enabled_ui` at the
/// call site rather than anything in here: an option that would do nothing is
/// a property of the section's state, not of the option, and a row whose
/// options disagree about it has no honest label to put on the row. That the
/// gate is outside is also what keeps the row wrapping — the scope
/// `add_enabled_ui` opens is a nested layout, and a nested layout inside
/// `button_row`'s `horizontal_wrapped` does not wrap, so a gate reached for
/// per option in here would run the row off the pane and take the section's
/// separators past the edge with it
/// (`no_settings_pane_overruns_a_narrow_column`).
///
/// The body is `Ui::selectable_value`'s: a `Button::selectable` and a click
/// test. The hint shows in both states (egui splits the two), since a grayed
/// option's tooltip is exactly where "and here is what would switch it on"
/// belongs.
pub fn choice_row<T: Copy + PartialEq>(
    ui: &mut Ui,
    name: &str,
    value: &mut T,
    options: &[(T, &str, &str)],
) {
    button_row(ui, |ui| {
        ui.label(name);
        for &(choice, label, hint) in options {
            let mut response =
                ui.add(egui::Button::selectable(*value == choice, option_label(label)));
            if response.clicked() && *value != choice {
                *value = choice;
                response.mark_changed();
            }
            if !hint.is_empty() {
                response = response.on_hover_text(hint);
                response.on_disabled_hover_text(hint);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_scene::{DEFAULT_EXTRA_SIZE, MIN_EXTRA_SIZE};

    /// Double-clicking the strip is the only way back to the stock wheel, so
    /// where it lands has to BE the stock wheel — not a pair that was the
    /// stock wheel when the gesture was written. The fringe is the half that
    /// goes wrong quietly: reset to zero extras and the two bars under the
    /// strip gray out holding a size and a blend nothing is drawing, which
    /// reads as the fringe knobs being unavailable rather than as the reset
    /// having thrown the fringe away.
    #[test]
    fn a_double_click_goes_home_to_the_wheel_a_fresh_view_opens_with() {
        let fresh = ViewConfig::default();
        assert_eq!(reset_wheel(), (fresh.octave_count, fresh.octave_extras));
        assert!(
            reset_wheel().1 > 0,
            "the stock wheel carries a fringe, so the reset must leave the \
             Extra size and Extra blend bars live",
        );
    }

    /// The analyzer's axis, the range bar's real caller.
    const AXIS: (f32, f32) = (12.0, 132.0);
    const OCTAVE: f32 = 12.0;

    /// Paint one range bar across a 300pt row and return what it emitted.
    fn paint_range_bar(low: f32, high: f32) -> Vec<egui::Shape> {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 100.0));
        let (mut lo, mut hi) = (low, high);
        let out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                RangeBar::new(&mut lo, &mut hi, AXIS.0..=AXIS.1).min_span(OCTAVE).show(ui);
            },
        );
        out.shapes.into_iter().map(|s| s.shape).collect()
    }

    /// Drag a range bar from `from` to `to` (fractions of its width) and
    /// answer where its two ends ended up. A real gesture through a real
    /// context: press, then move with the button still down, which is the only
    /// way to reach the pointer-value path `integer()` snaps in.
    fn drag_range_bar(
        (low, high): (f32, f32),
        (from, to): (f32, f32),
        integer: bool,
    ) -> (f32, f32) {
        const W: f32 = 300.0;
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(W, 100.0));
        let (mut lo, mut hi) = (low, high);
        let track = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut t = 0.0;
        let mut frame = |lo: &mut f32, hi: &mut f32, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let bar = RangeBar::new(lo, hi, AXIS.0..=AXIS.1).min_span(OCTAVE);
                    let response = if integer { bar.integer().show(ui) } else { bar.show(ui) };
                    track.set(response.rect);
                },
            );
        };
        // A frame with no input first: egui resolves the pointer against the
        // PREVIOUS pass's widget rects, so the bar has to have been laid out
        // once before a press can land on it — and this is also where the
        // gesture learns where the bar actually is.
        frame(&mut lo, &mut hi, vec![]);
        let bar = track.get();
        let at = |x: f32| egui::pos2(bar.left() + bar.width() * x, bar.center().y);
        frame(&mut lo, &mut hi, vec![egui::Event::PointerMoved(at(from))]);
        frame(&mut lo, &mut hi, vec![
            egui::Event::PointerMoved(at(from)),
            egui::Event::PointerButton {
                pos: at(from),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        // A step clear of egui's drag threshold first, then the rest of the
        // way. The grab is decided on the first frame the drag is LIVE, so a
        // gesture that jumps straight to its target decides it there — which
        // can be a different grab from the one the press was aimed at.
        let step = 12.0 / bar.width() * (to - from).signum();
        frame(&mut lo, &mut hi, vec![egui::Event::PointerMoved(at(from + step))]);
        frame(&mut lo, &mut hi, vec![egui::Event::PointerMoved(at(to))]);
        (lo, hi)
    }

    /// An `integer()` bar lands on whole values, and a plain one does not —
    /// the second half matters as much as the first, since a snap that came
    /// from the axis rather than from the flag would pass the first alone.
    ///
    /// The minimum span survives the snap too. That is the reason it is taken
    /// at the POINTER rather than by rounding the pair afterwards: a span
    /// sitting exactly at the minimum, rounded end by end, can come out a
    /// whole value short of it.
    #[test]
    fn an_integer_range_bar_lands_on_whole_values() {
        // Slide the whole span, which is the grab that could carry a
        // fraction furthest: it holds an OFFSET taken at the press and adds
        // it back on every frame, so a fractional one would leave both ends
        // off the grid for the rest of the gesture.
        let held = (48.0f32, 84.0f32);
        let (lo, hi) = drag_range_bar(held, (0.45, 0.6), true);
        assert_eq!((lo, hi), (lo.round(), hi.round()), "{lo}..{hi} is not whole");
        assert!(lo > held.0, "the drag moved nothing");
        assert_eq!(hi - lo, held.1 - held.0, "a slid span keeps its width");
        let (loose, _) = drag_range_bar(held, (0.45, 0.6), false);
        assert_ne!(loose, loose.round(), "the axis snaps by itself; the flag proves nothing");

        // Squeezed against the minimum span: the high end dragged down past
        // the low one, from a position that is not a whole value.
        let (lo, hi) = drag_range_bar((60.0, 96.0), (1.0, 0.371), true);
        assert_eq!((lo, hi), (lo.round(), hi.round()), "{lo}..{hi} is not whole");
        assert_eq!(hi - lo, OCTAVE, "the span stopped at {} rather than the minimum", hi - lo);
    }

    /// The filled rects, in paint order.
    fn filled_rects(shapes: &[egui::Shape]) -> Vec<(egui::Rect, egui::Color32)> {
        shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Rect(r) if r.fill != egui::Color32::TRANSPARENT => {
                    Some((r.rect, r.fill))
                }
                _ => None,
            })
            .collect()
    }

    /// The text runs and the boxes they occupy, in paint order.
    fn text_boxes(shapes: &[egui::Shape]) -> Vec<(egui::Rect, String)> {
        shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Text(t) => Some((
                    egui::Rect::from_min_size(t.pos, t.galley.size()),
                    t.galley.text().to_owned(),
                )),
                _ => None,
            })
            .collect()
    }

    /// The two handles, left to right.
    fn handles(shapes: &[egui::Shape]) -> Vec<egui::Rect> {
        let mut hs: Vec<_> = filled_rects(shapes)
            .into_iter()
            .filter(|(r, fill)| *fill == theme::text() && r.width() <= HANDLE_W + 0.01)
            .map(|(r, _)| r)
            .collect();
        hs.sort_by(|a, b| a.left().total_cmp(&b.left()));
        hs
    }

    /// The second bug this widget shipped with, and the harder one to see: at
    /// the full range — which is the pitch axis's DEFAULT — both handles sat
    /// flush against the bar's ends, 3px wide, under the corner rounding. The
    /// control was then pixel-identical to a ValueBar filled to 100%: nothing
    /// on screen said it had ends to grab, so it read as "not there at all".
    #[test]
    fn the_handles_read_as_handles_even_at_the_limits() {
        for (low, high) in [(AXIS.0, AXIS.1), (AXIS.0, AXIS.0 + OCTAVE), (60.0, 72.0)] {
            let shapes = paint_range_bar(low, high);
            let bar = filled_rects(&shapes)[0].0;
            let handles = handles(&shapes);
            assert_eq!(handles.len(), 2, "{low}..{high} did not paint two handles");
            for h in handles {
                assert!(
                    h.left() > bar.left() && h.right() < bar.right(),
                    "{low}..{high}: handle {h:?} is flush with the bar's edge, where it \
                     reads as the border rather than as something to grab",
                );
                assert!(h.width() >= 4.0, "a handle thinner than this vanishes into the fill");
            }
        }
    }

    /// Each end's number belongs to its own handle, and nothing else is
    /// written on the bar — one label plus a joined "low – high" readout put
    /// three text runs in a 20pt row, none of them attached to the thing they
    /// described.
    #[test]
    fn each_end_reads_out_beside_its_own_handle() {
        // Mid-axis, so there is empty track on both sides to sit in.
        let shapes = paint_range_bar(60.0, 72.0);
        let (texts, handles) = (text_boxes(&shapes), handles(&shapes));
        assert_eq!(texts.len(), 2, "only the two values, no label");
        assert_eq!(texts[0].1, "60.00");
        assert_eq!(texts[1].1, "72.00");
        assert!(texts[0].0.right() <= handles[0].left(), "low value sits outside its handle");
        assert!(texts[1].0.left() >= handles[1].right(), "high value sits outside its handle");
    }

    /// At the full range there is no empty track left to write in, so each
    /// readout moves to the inner side of its handle rather than off the bar.
    #[test]
    fn the_readouts_move_inside_when_the_span_leaves_no_room() {
        let shapes = paint_range_bar(AXIS.0, AXIS.1);
        let (texts, handles) = (text_boxes(&shapes), handles(&shapes));
        let bar = filled_rects(&shapes)[0].0;
        assert!(texts[0].0.left() >= handles[0].right(), "low value moved inside the span");
        assert!(texts[1].0.right() <= handles[1].left(), "high value moved inside the span");
        for (t, _) in &texts {
            assert!(t.left() >= bar.left() && t.right() <= bar.right(), "readout left the bar");
        }
    }

    /// The bug this widget shipped with: the pitch range's default IS the
    /// full axis, and a span that fills the range had nowhere to slide, so
    /// panning it moved nothing — while a drag anywhere but the outermost few
    /// pixels panned. The bar was dead exactly where you first meet it. Now a
    /// span drag squishes at the wall, so it always does something.
    #[test]
    fn a_range_filling_the_axis_still_drags_from_the_middle() {
        let full = (AXIS.0, AXIS.1);
        for v in [20.0, 60.0, 90.0, 125.0] {
            let grab = Grab::at(v, full, AXIS, 1.0);
            let moved = grab.apply(v - 10.0, full, AXIS, OCTAVE);
            assert!(moved != full, "dragging at {v} moved nothing");
        }
    }

    /// A middle drag takes the whole span, with the pointer's offset into it,
    /// rather than snapping an end to the pointer.
    #[test]
    fn a_middle_drag_takes_the_whole_span() {
        assert!(matches!(
            Grab::at(40.0, (24.0, 60.0), AXIS, 1.0),
            Grab::Span { offset, width } if offset == 16.0 && width == 36.0
        ));
    }

    /// Near an end, that end wins over the span — with a reach generous
    /// enough that aiming at a handle and hitting the span is hard, since
    /// that mistake moves both values instead of the one you aimed at.
    #[test]
    fn a_drag_near_an_end_takes_that_end() {
        assert!(matches!(Grab::at(25.0, (24.0, 60.0), AXIS, 2.0), Grab::Low));
        assert!(matches!(Grab::at(59.0, (24.0, 60.0), AXIS, 2.0), Grab::High));
        // Well inside the span, but still nearer the end than the reach.
        assert!(matches!(Grab::at(31.0, (24.0, 60.0), AXIS, 8.0), Grab::Low));
    }

    /// The reach still cannot swallow a narrow span whole, or a zoomed-in
    /// range would have no middle left to slide along the axis.
    #[test]
    fn the_handle_reach_leaves_a_narrow_span_pannable() {
        let narrow = (60.0, 60.0 + OCTAVE);
        let middle = 60.0 + OCTAVE / 2.0;
        assert!(matches!(Grab::at(middle, narrow, AXIS, 1_000.0), Grab::Span { .. }));
    }

    /// Dragging an end past its partner stops at the minimum span instead of
    /// crossing it — otherwise the pitch axis inverts and every pitch on it
    /// maps backwards.
    #[test]
    fn a_dragged_end_stops_at_the_minimum_span() {
        let (lo, hi) = Grab::Low.apply(200.0, (24.0, 60.0), AXIS, OCTAVE);
        assert_eq!((lo, hi), (48.0, 60.0), "low stops one octave below high");
        let (lo, hi) = Grab::High.apply(-200.0, (24.0, 60.0), AXIS, OCTAVE);
        assert_eq!((lo, hi), (24.0, 36.0), "high stops one octave above low");
    }

    /// Either end can still reach its own limit — clamping the pair must not
    /// cost you the full axis.
    #[test]
    fn the_ends_still_reach_the_limits() {
        assert_eq!(Grab::Low.apply(-200.0, (24.0, 60.0), AXIS, OCTAVE).0, AXIS.0);
        assert_eq!(Grab::High.apply(200.0, (24.0, 60.0), AXIS, OCTAVE).1, AXIS.1);
    }

    /// Mid-axis, a slid span just follows the pointer at its grabbed offset,
    /// keeping its width.
    #[test]
    fn a_slid_span_follows_the_pointer_at_its_grabbed_offset() {
        let grab = Grab::Span { offset: 6.0, width: 36.0 };
        assert_eq!(grab.apply(70.0, (24.0, 60.0), AXIS, OCTAVE), (64.0, 100.0));
    }

    /// Slid into an end, the span squishes against it: the leading edge pins
    /// and the trailing edge carries on with the pointer, down to the minimum
    /// span. Stopping dead at the wall instead made a drag feel jammed.
    #[test]
    fn a_slid_span_squishes_against_the_end_it_meets() {
        // Grabbed dead center of 30..90.
        let grab = Grab::Span { offset: 30.0, width: 60.0 };
        let start = (30.0, 90.0);

        let (lo, hi) = grab.apply(40.0, start, AXIS, OCTAVE);
        assert_eq!(lo, AXIS.0, "the leading edge pins to the floor");
        assert_eq!(hi, 70.0, "the trailing edge keeps following the pointer");

        assert_eq!(
            grab.apply(-500.0, start, AXIS, OCTAVE),
            (AXIS.0, AXIS.0 + OCTAVE),
            "squishing bottoms out at the minimum span, not at nothing",
        );
        assert_eq!(
            grab.apply(500.0, start, AXIS, OCTAVE),
            (AXIS.1 - OCTAVE, AXIS.1),
            "and the same against the ceiling",
        );
    }

    /// Squishing reads only the width the gesture began with, never the
    /// squished pair it just produced. Re-measuring would shrink the span
    /// again every frame while the pointer sat perfectly still — and would
    /// make the squish a one-way trip instead of springing back when you drag
    /// away from the wall.
    #[test]
    fn squishing_is_stable_and_reversible_within_the_gesture() {
        let grab = Grab::Span { offset: 30.0, width: 60.0 };
        let squished = (AXIS.0, AXIS.0 + OCTAVE);
        // Same pointer, already-squished input: the answer must not creep.
        assert_eq!(grab.apply(-500.0, squished, AXIS, OCTAVE), squished);
        // Back to where it was grabbed: the original width comes back.
        assert_eq!(grab.apply(60.0, squished, AXIS, OCTAVE), (30.0, 90.0));
    }

    fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        }
    }

    /// A [`SpectrumBar`] alone in a 300pt context, driven one frame at a time.
    ///
    /// Real events through a real context, because nothing less reaches the
    /// widget: what a gesture has hold of is decided on the first frame egui
    /// calls the press a drag and then remembered in context data, so a single
    /// synthetic call would exercise neither the decision nor the memory. A
    /// bare context is the design scale, 1.0, which is why the geometry below
    /// reads the constants unmultiplied.
    struct Spectrum {
        ctx: egui::Context,
        screen: egui::Rect,
        rect: egui::Rect,
        /// Where the pointer was on the frame egui first called the press a
        /// drag — the frame the widget settles what it has hold of, and so the
        /// position a gesture's own arithmetic is anchored to.
        live_at: egui::Pos2,
        t: f64,
    }

    impl Spectrum {
        /// Laid out once before anything is aimed at it: egui resolves the
        /// pointer against the PREVIOUS pass's rects, so a press cannot land on
        /// a bar that has never been drawn.
        fn settled(g: &mut PitchGradient) -> Spectrum {
            let ctx = egui::Context::default();
            crate::theme::apply_theme(&ctx);
            let mut h = Spectrum {
                ctx,
                screen: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 100.0)),
                rect: egui::Rect::NOTHING,
                live_at: egui::Pos2::ZERO,
                t: 0.0,
            };
            h.frame(g, vec![]);
            h
        }

        fn frame(&mut self, g: &mut PitchGradient, events: Vec<egui::Event>) -> Vec<egui::Shape> {
            self.t += 1.0 / 60.0;
            let rect = std::cell::Cell::new(egui::Rect::NOTHING);
            let out = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(self.screen),
                    time: Some(self.t),
                    events,
                    ..Default::default()
                },
                |ui| rect.set(SpectrumBar::new(g).show(ui).rect),
            );
            self.rect = rect.get();
            out.shapes.into_iter().map(|s| s.shape).collect()
        }

        /// The track a handle actually travels. The bar hands back the TRACK's
        /// rect rather than the whole row it allocated — the flip button beside
        /// it carries its own tooltip — so the inset at either end is all that
        /// separates the two.
        fn track(&self) -> egui::Rect {
            self.rect.shrink2(Vec2::new(HANDLE_INSET, 0.0))
        }

        /// The middle of the flip button, in the gutter past the track's right
        /// edge.
        fn on_flip(&self) -> egui::Pos2 {
            egui::pos2(self.rect.right() + FLIP_W * 0.5, self.rect.center().y)
        }

        /// Where a span of `span` degrees stands the handle.
        fn at_span(&self, span: f32) -> egui::Pos2 {
            let track = self.track();
            let across = (span / FULL_TURN).abs().clamp(0.0, 1.0);
            egui::pos2(track.left() + track.width() * across, track.center().y)
        }

        /// Press and release at one spot, answering what the frame carrying
        /// the release painted — which is the frame a click lands on.
        fn click(&mut self, g: &mut PitchGradient, at: egui::Pos2) -> Vec<egui::Shape> {
            self.frame(g, vec![egui::Event::PointerMoved(at)]);
            self.frame(g, vec![press(at, true)]);
            self.frame(g, vec![press(at, false)])
        }

        /// Press at `from` and drag to `to`, answering what the arriving frame
        /// painted. A step clear of egui's drag threshold comes first, since
        /// a gesture that jumps straight to its target settles its grab there.
        fn drag(
            &mut self,
            g: &mut PitchGradient,
            from: egui::Pos2,
            to: egui::Pos2,
        ) -> Vec<egui::Shape> {
            self.frame(g, vec![egui::Event::PointerMoved(from)]);
            self.frame(g, vec![egui::Event::PointerMoved(from), press(from, true)]);
            self.live_at = from + (to - from).normalized() * 12.0;
            self.frame(g, vec![egui::Event::PointerMoved(self.live_at)]);
            self.frame(g, vec![egui::Event::PointerMoved(to)])
        }

        /// Two clicks at one spot, close enough together to be one gesture.
        fn double_click(&mut self, g: &mut PitchGradient, at: egui::Pos2) {
            self.frame(g, vec![egui::Event::PointerMoved(at)]);
            for _ in 0..2 {
                self.frame(g, vec![press(at, true)]);
                self.frame(g, vec![press(at, false)]);
            }
        }
    }

    /// The one text run a spectrum bar paints: its span, signed.
    fn spectrum_readout(shapes: &[egui::Shape]) -> String {
        let texts: Vec<String> = text_boxes(shapes).into_iter().map(|(_, s)| s).collect();
        assert_eq!(texts.len(), 1, "a spectrum bar draws one readout, not {texts:?}");
        texts.into_iter().next().expect("checked just above")
    }

    /// Where the bar drew its handle.
    fn spectrum_handle_x(shapes: &[egui::Shape]) -> f32 {
        let hs = handles(shapes);
        assert_eq!(hs.len(), 1, "a spectrum bar draws one handle, not {hs:?}");
        hs[0].center().x
    }

    /// The hue the bar paints at `p`, worked out the way the bar lays a circle
    /// on a track: cut at the arc's own start, one whole turn across.
    fn hue_under(g: PitchGradient, track: egui::Rect, p: egui::Pos2) -> f32 {
        let across = ((p.x - track.left()) / track.width()).clamp(0.0, 1.0);
        let winding = if g.hue_span < 0.0 { -1.0 } else { 1.0 };
        (g.hue_start + across * FULL_TURN * winding).rem_euclid(FULL_TURN)
    }

    /// The track stands for the span knob's whole travel, which holds only
    /// while the two numbers are the same one.
    #[test]
    fn the_spectrum_bar_track_is_a_whole_turn_of_the_span_knob() {
        assert_eq!(
            FULL_TURN,
            PitchGradient::MAX_HUE_SPAN,
            "the track draws one turn while the span reaches {}, so the handle \
             parks at the right edge with the value still growing",
            PitchGradient::MAX_HUE_SPAN,
        );
    }

    /// The handle stands at the fraction of the turn the arc claims, and says
    /// so in the same picture.
    #[test]
    fn the_spectrum_handle_stands_where_its_readout_says() {
        for span in [0.0f32, 45.0, 190.0, 360.0, -190.0, -360.0] {
            let mut g = PitchGradient { hue_span: span, ..PitchGradient::default() };
            let mut h = Spectrum::settled(&mut g);
            let shapes = h.frame(&mut g, vec![]);
            let track = h.track();
            let want = track.left() + track.width() * (span / FULL_TURN).abs();
            let drawn = spectrum_handle_x(&shapes);
            assert!(
                (drawn - want).abs() < 0.51,
                "a span of {span} put the handle at {drawn} rather than {want}",
            );
            assert_eq!(spectrum_readout(&shapes), format!("{span:+.0}°"));
        }

        // Both limits are places the handle can STAND rather than edges it
        // merges into, which is the whole of what the inset buys: at neither
        // one does it hang off the bar or disappear under the rounding.
        let mut nothing = PitchGradient { hue_span: 0.0, ..PitchGradient::default() };
        let mut h = Spectrum::settled(&mut nothing);
        let shapes = h.frame(&mut nothing, vec![]);
        assert!(handles(&shapes)[0].left() >= h.rect.left(), "a zero span hangs the handle off");
        let mut whole = PitchGradient { hue_span: 360.0, ..PitchGradient::default() };
        let mut h = Spectrum::settled(&mut whole);
        let shapes = h.frame(&mut whole, vec![]);
        assert!(handles(&shapes)[0].right() <= h.rect.right(), "a whole turn hangs the handle off");
    }

    /// The frame that moves the arc is the frame that DRAWS it moved.
    ///
    /// A bar that snapshots its value before the interaction block and paints
    /// from the snapshot is right in the end and wrong the whole way through:
    /// the handle, the arc, the strip and the readout all show the previous
    /// frame's value for the length of the gesture. Only a live drag catches
    /// it — a settled bar draws the same picture either way, which is why every
    /// other test here would pass against the lag.
    #[test]
    fn a_spectrum_drag_draws_the_arc_it_just_set() {
        let mut g = PitchGradient { hue_span: 90.0, ..PitchGradient::default() };
        let mut h = Spectrum::settled(&mut g);
        let (from, to) = (h.at_span(90.0), h.at_span(270.0));
        let shapes = h.drag(&mut g, from, to);
        assert!(
            (g.hue_span - 270.0).abs() < 2.0,
            "the drag left the span at {} rather than near 270",
            g.hue_span,
        );
        let drawn = spectrum_handle_x(&shapes);
        assert!(
            (drawn - to.x).abs() < 1.0,
            "the pointer is at {} and the handle was drawn at {drawn}, a frame behind",
            to.x,
        );
        assert_eq!(
            spectrum_readout(&shapes),
            format!("{:+.0}°", g.hue_span),
            "the readout names a span other than the one the drag just set",
        );
    }

    /// The flip button reverses the arc, in the frame it is clicked.
    ///
    /// The value half is what says the button is wired to the gradient's own
    /// flip and not to a second spelling of it here; the picture half is the
    /// same claim `a_spectrum_drag_draws_the_arc_it_just_set` makes about the
    /// handle, and it fails the same way — a click handled after the snapshot
    /// the paint reads leaves the bar a frame behind the button.
    #[test]
    fn the_flip_button_reads_the_arc_backwards() {
        let mut g =
            PitchGradient { hue_start: 260.0, hue_span: 190.0, ..PitchGradient::default() };
        let before = g.sanitized();
        let mut h = Spectrum::settled(&mut g);
        let shapes = h.click(&mut g, h.on_flip());
        assert_eq!(g, before.flipped(), "the click left the arc at {g:?}");
        assert_eq!(
            spectrum_readout(&shapes),
            format!("{:+.0}°", g.hue_span),
            "the readout names a direction other than the one the click just set",
        );
    }

    /// The button is a button and the track beside it is a track: a sideways
    /// drag begun on the button turns nothing.
    ///
    /// They share a row and a well, and the only thing keeping them apart is
    /// that each is interacted with over its OWN rectangle. Sensed together, a
    /// press that began on the button reaches the track's rotate branch the
    /// moment egui calls it a drag, and the circle spins under a pointer that
    /// pressed a button.
    #[test]
    fn a_drag_begun_on_the_flip_button_turns_nothing() {
        let before =
            PitchGradient { hue_start: 0.0, hue_span: 90.0, ..PitchGradient::default() };
        let mut g = before;
        let mut h = Spectrum::settled(&mut g);
        let to = h.at_span(120.0);
        h.drag(&mut g, h.on_flip(), to);
        assert_eq!(g, before, "a drag begun on the flip button moved the arc to {g:?}");

        // The same drag from just inside the track does move it, so the harness
        // is delivering something the widget can act on.
        let mut g = before;
        let mut h = Spectrum::settled(&mut g);
        let from = egui::pos2(h.track().right(), h.track().center().y);
        h.drag(&mut g, from, to);
        assert_ne!(g, before, "the same drag on the track moved nothing, so this proves nothing");
    }

    /// The arc a double-click goes home to is the one a fresh view OPENS on,
    /// which is not the gradient type's own default.
    ///
    /// The same argument [`reset_wheel`] is written out for, one control over:
    /// a reset that names its own value drifts the moment the fresh look
    /// moves, and does it silently, because nothing reads out the pair it
    /// resets to. `ViewConfig::default` composes its gradient rather than
    /// taking `PitchGradient::default()` — a shorter arc over a shallower
    /// brightness ramp — and says at the field that it is free to differ.
    /// A reset that lands on the type's default therefore puts the bar
    /// somewhere the plugin has never opened, and the bar has no text entry
    /// to dial it back with.
    #[test]
    fn a_double_click_on_the_spectrum_goes_home_to_the_arc_a_fresh_view_opens_on() {
        let fresh = ViewConfig::default().pitch_gradient;
        let dialled = PitchGradient { hue_start: 12.0, hue_span: 33.0, ..fresh };

        let mut g = dialled;
        let mut h = Spectrum::settled(&mut g);
        let at = h.track().center();
        h.double_click(&mut g, at);
        assert_eq!(
            (g.hue_start, g.hue_span),
            (fresh.hue_start, fresh.hue_span),
            "the reset landed on an arc no fresh view opens on",
        );
    }

    /// A turn slides the circle under a fixed left edge, and the hue the
    /// gesture took hold of stays under the pointer for the length of it.
    #[test]
    fn turning_the_spectrum_keeps_the_grabbed_hue_under_the_pointer() {
        let mut g = PitchGradient { hue_start: 0.0, hue_span: 90.0, ..PitchGradient::default() };
        let before = g;
        let mut h = Spectrum::settled(&mut g);
        // Well clear of the handle, which a quarter-turn arc stands at 0.25.
        let (from, to) = (h.at_span(300.0), h.at_span(200.0));
        h.drag(&mut g, from, to);
        let track = h.track();
        let held = hue_under(before, track, h.live_at);
        let now = hue_under(g, track, to);
        // Within a degree either side, the far side being the seam: two hues a
        // whisker apart across 0 are 359 apart by subtraction.
        let apart = (now - held).abs();
        assert!(
            !(1.0..=FULL_TURN - 1.0).contains(&apart),
            "the hue under the pointer went from {held} to {now} during the turn",
        );
        assert_ne!(g.hue_start, before.hue_start, "the turn moved nothing");
        assert_eq!(g.hue_span, before.hue_span, "a turn changed how wide the arc is");
    }

    /// A flip lands on the same arc read backwards — the promise the Flip
    /// button beside the bar makes, and the reason the bar draws the same
    /// stretch of color either way round.
    #[test]
    fn a_flip_is_the_same_arc_read_backwards() {
        for (start, span) in [(260.0f32, 190.0f32), (0.0, 360.0), (95.0, -45.0), (12.0, 0.0)] {
            let before =
                PitchGradient { hue_start: start, hue_span: span, ..PitchGradient::default() }
                    .sanitized();
            let after = before.flipped();
            let ends = |g: PitchGradient| (g.lightness_and_hue(0.0).1, g.lightness_and_hue(1.0).1);
            let (low, high) = ends(before);
            let (flipped_low, flipped_high) = ends(after);
            assert!(
                (flipped_low - high).abs() < 1e-3 && (flipped_high - low).abs() < 1e-3,
                "{start}/{span}: {low}..{high} came back as {flipped_low}..{flipped_high}",
            );
            assert_eq!(
                after.hue_span.abs(),
                before.hue_span.abs(),
                "{start}/{span}: a flip changed how much of the circle the arc claims",
            );
            assert_eq!(
                after.flipped(),
                before,
                "{start}/{span}: flipping twice is not the identity",
            );
        }
    }

    /// A span of nothing is written with the one sign that reads as no
    /// direction.
    ///
    /// `-0.0` is the sign that lies: it is not `< 0.0`, so everything asking
    /// which way the arc runs takes it for rightward, while `{:+}` prints it as
    /// running left — a bar that says one direction and behaves as the other.
    /// Flipping a zero span makes one, and so does dragging a flipped arc down
    /// to nothing.
    #[test]
    fn a_span_of_nothing_reads_out_with_no_direction() {
        let flipped = PitchGradient { hue_span: 0.0, ..PitchGradient::default() }.flipped();
        assert!(
            flipped.hue_span.is_sign_positive(),
            "flipping a span of nothing left it at {}",
            flipped.hue_span,
        );

        let mut g = PitchGradient { hue_span: -120.0, ..PitchGradient::default() };
        let mut h = Spectrum::settled(&mut g);
        let from = h.at_span(-120.0);
        let to = egui::pos2(h.track().left() - 40.0, h.track().center().y);
        let shapes = h.drag(&mut g, from, to);
        assert_eq!(g.hue_span, 0.0, "the drag left the span at {}", g.hue_span);
        assert!(
            g.sanitized().hue_span.is_sign_positive(),
            "a flipped arc dragged to nothing kept a direction it cannot have",
        );
        assert_eq!(
            spectrum_readout(&shapes),
            "+0°",
            "a span of nothing read out as running left",
        );
    }

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

    /// The magnet holds the bar at its target until an edit pulls clear of
    /// the window, and hands back an escaped value untouched. Both halves
    /// matter to the caller: inside the window the value it gets back is the
    /// target, which is what makes "did this edit escape" the same question
    /// as "did the value change".
    #[test]
    fn a_magnet_holds_the_bar_until_an_edit_pulls_clear() {
        let mut value = 400.0;
        let bar = ValueBar::new(&mut value, 380.0..=420.0, "Major third (¢)").magnet(400.0, 5.0);
        for v in [400.0f32, 396.0, 404.5, 395.0, 405.0] {
            assert_eq!(bar.snapped(v), 400.0, "{v} is inside the window");
        }
        for v in [394.9f32, 405.1, 380.0, 420.0] {
            assert_eq!(bar.snapped(v), v, "{v} is past the window");
        }
        let mut value = 400.0;
        let plain = ValueBar::new(&mut value, 380.0..=420.0, "Major third (¢)");
        assert_eq!(plain.snapped(396.0), 396.0, "no magnet, no pull");
    }

    #[test]
    fn integer_bars_snap() {
        let mut value = 0.0;
        let bar = ValueBar::new(&mut value, 1.0..=8.0, "test").integer();
        let v = bar.value_at(0.37);
        assert_eq!(v, v.round());
    }

    /// Every text run a `choice_row` of these options paints, as
    /// `text -> (family, size)`.
    fn choice_row_fonts(options: &[(u32, &str, &str)]) -> Vec<(String, egui::FontId)> {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 100.0));
        let mut value = 0u32;
        let out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| choice_row(ui, "Row", &mut value, options),
        );
        out.shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Text(t) => Some((
                    t.galley.text().to_owned(),
                    t.galley.job.sections[0].format.font_id.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    /// An option label that is a bare number is set in the monospace face, and
    /// one that is a word is not — decided per label, since a row can hold
    /// both. The size is the row's own either way: taking the whole monospace
    /// TEXT STYLE would shrink the numbers, leaving "30" visibly smaller than
    /// the "Uncapped" beside it.
    #[test]
    fn number_option_labels_are_monospace_at_the_rows_own_size() {
        let painted = choice_row_fonts(&[
            (0, "Uncapped", ""),
            (1, "30", ""),
            (2, "144", ""),
            (3, "16:9", ""),
            (4, "-1.5", ""),
            (5, "12-TET", ""),
        ]);
        let numbers = ["30", "144", "16:9", "-1.5"];
        let words = ["Row", "Uncapped", "12-TET"];
        let row_size = painted
            .iter()
            .find(|(text, _)| text == "Uncapped")
            .map(|(_, font)| font.size)
            .expect("the row painted no 'Uncapped'");
        for (text, font) in &painted {
            let wanted = if numbers.contains(&text.as_str()) {
                egui::FontFamily::Monospace
            } else {
                assert!(words.contains(&text.as_str()), "unexpected run {text:?}");
                egui::FontFamily::Proportional
            };
            assert_eq!(font.family, wanted, "{text:?} was painted in {:?}", font.family);
            assert_eq!(font.size, row_size, "{text:?} was painted at {}pt", font.size);
        }
        assert_eq!(painted.len(), numbers.len() + words.len(), "a label went unpainted");
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

    /// A bare label centers on the button text in a `button_row`. The
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
    }

    /// A bar's two text runs keep out of each other's way and stay inside the
    /// track, at every width.
    ///
    /// Three separate things in the paint have to hold for that, and nothing
    /// else in the suite relates one run to the other or either to the track:
    ///
    /// - the name's budget subtracts the room the readout needs, or the name
    ///   runs over the number (measured: 6pt of overlap at 160, 16pt at 120);
    /// - the name is held to ONE row, or it wraps to two and spills above and
    ///   below into the bars either side (a 29pt galley in a 20pt track);
    /// - both runs are offset by half their own height, or they sit a half-line
    ///   low with 7pt of a 17pt line below the track.
    ///
    /// Each is a live regression rather than a hypothetical: all three are
    /// clippy-clean and leave the rest of the suite green.
    /// `LayoutJob::simple_singleline` invites the second in particular — the
    /// name says the row cap is already set, and it is not.
    #[test]
    fn a_bars_name_and_readout_never_collide_or_leave_the_track() {
        let cases: [(&str, f32, RangeInclusive<f32>); 3] = [
            ("Harmonic seventh (¢)", 1000.0, SEVENTH_RANGE),
            ("Perfect fifth (¢)", 701.96, 680.0..=720.0),
            ("Sevenths angle", 45.0, 0.0..=90.0),
        ];
        for width in [400.0f32, 240.0, 180.0, 157.0, 120.0] {
            for (label, value, range) in cases.clone() {
                let ctx = egui::Context::default();
                crate::theme::apply_theme(&ctx);
                let screen =
                    egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(width, 100.0));
                let mut value = value;
                let out = ctx.run_ui(
                    egui::RawInput { screen_rect: Some(screen), ..Default::default() },
                    |ui| {
                        ValueBar::new(&mut value, range.clone(), label).show(ui);
                    },
                );
                let mut runs: Vec<egui::Rect> = out
                    .shapes
                    .iter()
                    .filter_map(|cs| match &cs.shape {
                        egui::Shape::Text(t) => Some(t.visual_bounding_rect()),
                        _ => None,
                    })
                    .collect();
                runs.sort_by(|a, b| a.left().total_cmp(&b.left()));
                assert_eq!(runs.len(), 2, "{label} at {width}pt painted {} runs", runs.len());
                let track = out
                    .shapes
                    .iter()
                    .find_map(|cs| match &cs.shape {
                        egui::Shape::Rect(r) if r.fill == crate::theme::well() => Some(r.rect),
                        _ => None,
                    })
                    .expect("the bar painted no track");
                assert!(
                    runs[0].right() <= runs[1].left() + 0.5,
                    "{label} at {width}pt: the name reaches {} and the readout starts at {}",
                    runs[0].right(),
                    runs[1].left()
                );
                for run in &runs {
                    assert!(
                        run.top() >= track.top() - 0.5 && run.bottom() <= track.bottom() + 0.5,
                        "{label} at {width}pt: a run spans y {}..{} in a track of {}..{}",
                        run.top(),
                        run.bottom(),
                        track.top(),
                        track.bottom()
                    );
                }
            }
        }
    }

    /// What a galley actually PUTS ON SCREEN. `Galley::text()` answers with the
    /// source string, so it cannot see an elision; the glyphs can.
    fn painted_text(galley: &egui::Galley) -> String {
        galley.rows.iter().flat_map(|row| row.glyphs.iter()).map(|g| g.chr).collect()
    }

    /// A badged bar still says what drives it when its name has to be elided.
    ///
    /// Elision eats the tail, so a badge spelled into the END of a name is the
    /// first thing to go — and worse than merely lost: the UNBADGED name is the
    /// shorter of the two and draws in full at the same width, so the badged
    /// bar is the one that looks like it ran out of room. State has to sit
    /// where elision cannot reach it, which is the front.
    ///
    /// 157pt is the bar a 173pt column gives, and 173 is where the column
    /// floors — one separator drag from the default window, no resize needed.
    #[test]
    fn a_badged_bar_says_so_even_when_its_name_is_elided() {
        for width in [157.0f32, 180.0, 200.0, 400.0] {
            let ctx = egui::Context::default();
            crate::theme::apply_theme(&ctx);
            let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(width, 100.0));
            let mut value = 386.31;
            let out = ctx.run_ui(
                egui::RawInput { screen_rect: Some(screen), ..Default::default() },
                |ui| {
                    ValueBar::new(&mut value, 380.0..=420.0, "Major third (¢)")
                        .badge("Meantone")
                        .show(ui);
                },
            );
            let name = out
                .shapes
                .iter()
                .filter_map(|cs| match &cs.shape {
                    egui::Shape::Text(t) => Some((t.pos.x, painted_text(&t.galley))),
                    _ => None,
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .expect("the bar painted no text")
                .1;
            assert!(
                name.to_lowercase().contains("meantone"),
                "a {width}pt badged bar painted its name as {name:?}, which does not say so"
            );
        }
    }

    /// The name painted by a bar of `width` holding `value`, as its rendered
    /// (post-elision) width. The name is the left-hand run; the readout is
    /// right-aligned, so smallest x picks the name out.
    fn painted_name_width(width: f32, value: f32) -> f32 {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(width, 100.0));
        let mut value = value;
        let out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                ValueBar::new(&mut value, SEVENTH_RANGE, "Harmonic seventh (¢)").show(ui);
            },
        );
        out.shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Text(t) => Some((t.pos.x, t.galley.size().x)),
                _ => None,
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .expect("the bar painted no text")
            .1
    }

    /// The harmonic-seventh bar's range, which straddles a digit boundary: the
    /// 12-TET value (and the param's default) is 1000.00, the just one 968.83.
    const SEVENTH_RANGE: RangeInclusive<f32> = 928.83..=1008.83;

    /// A bar's NAME holds still while its number changes width.
    ///
    /// The name is elided against the room the readout leaves it, so a budget
    /// measured from the CURRENT readout re-elides the name the moment the
    /// value gains a digit — the name reflowing under the pointer mid-drag,
    /// which is the very thing the monospace readout buys for the digits. The
    /// budget has to come from the widest readout the bar's RANGE can produce.
    ///
    /// Swept across the band where it bites. Iosevka is 6pt per glyph and epaint
    /// rounds `wrap.max_width` only to whole points, so a digit is twelve times
    /// the rounding granularity and there is nothing to absorb it.
    #[test]
    fn a_bars_name_holds_still_while_its_number_changes_width() {
        for width in [174.0f32, 180.0, 184.0, 187.0, 200.0, 260.0] {
            let narrow = painted_name_width(width, 999.99);
            let wide = painted_name_width(width, 1000.00);
            assert!(
                (narrow - wide).abs() < 0.01,
                "a {width}pt bar draws its name {narrow}pt wide at 999.99 and {wide}pt at \
                 1000.00 — the name re-elides when the number gains a digit"
            );
        }
    }

    /// A row of buttons too wide for its column stays inside the column: the
    /// buttons take further lines, and a button whose own label cannot fit on
    /// one line wraps that label rather than extending past its frame.
    ///
    /// Both halves come from `horizontal_wrapped` and neither is visible at the
    /// call site, which is the reason to pin them: what the panes need from
    /// [`button_row`] is that nothing it holds can leave the column, and a
    /// non-wrapping row helper looks identical in the code that calls it.
    ///
    /// 90pt because the second half does not start until 95: above that every
    /// label fits on one line, and turning per-button wrapping off changes
    /// nothing the asserts can see. At 90 the widest label wraps to two rows,
    /// leaving 2.2pt of slack on the passing side and failing by 5.2pt without
    /// it. Wider would pin only the first half, which is what 120 did.
    #[test]
    fn a_row_too_wide_for_its_column_wraps_inside_it() {
        const COLUMN: f32 = 90.0;
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(COLUMN, 400.0));
        let mut rects = Vec::new();
        let _ = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                button_row(ui, |ui| {
                    ui.label("Projection");
                    for label in ["Perspective", "Orthographic", "Cabinet"] {
                        rects.push(ui.button(label).rect);
                    }
                });
            },
        );
        for (label, rect) in ["Perspective", "Orthographic", "Cabinet"].iter().zip(&rects) {
            assert!(
                rect.right() <= COLUMN + 1.0,
                "{label} reached {} in a {COLUMN}px column",
                rect.right()
            );
        }
        // And they really did stack rather than all landing on one line.
        assert!(
            rects[2].top() > rects[0].top(),
            "three wide buttons stayed on one line: {rects:?}"
        );
        // The second half, made self-evident rather than incidental: a button
        // taller than one padded text row is one whose label wrapped.
        let row = 25.0;
        assert!(
            rects.iter().any(|r| r.height() > row + 5.0),
            "no label wrapped, so only the row-wrap half is under test: {rects:?}"
        );
    }

    /// Paint one octave strip across a 300pt row and return what it emitted.
    fn paint_octave_strip(count: u32, extras: u32, size: f32, blend: f32) -> Vec<egui::Shape> {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 100.0));
        let (mut c, mut e) = (count, extras);
        let out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                OctaveStrip::new(&mut c, &mut e, size, blend).show(ui);
            },
        );
        out.shapes.into_iter().map(|s| s.shape).collect()
    }

    /// The strip's cells, left to right: the accent-filled rects, which the
    /// well behind them and the two handles over them are not.
    fn cells(shapes: &[egui::Shape]) -> Vec<egui::Rect> {
        let mut cells: Vec<_> = filled_rects(shapes)
            .into_iter()
            .filter(|(_, fill)| *fill == theme::accent_fill())
            .map(|(r, _)| r)
            .collect();
        cells.sort_by(|a, b| a.left().total_cmp(&b.left()));
        cells
    }

    /// One cell per octave the wheel draws, and its HEIGHT is the share of the
    /// ring that octave takes — which is the whole of what the strip says that
    /// two count bars could not.
    #[test]
    fn the_strip_draws_one_cell_per_octave_at_its_own_width() {
        // An even wheel: every octave the same, and the cells with it.
        let shapes = paint_octave_strip(5, 0, DEFAULT_EXTRA_SIZE, 0.0);
        let bar = filled_rects(&shapes)[0].0;
        let even = cells(&shapes);
        assert_eq!(even.len(), 5, "one cell per octave");
        for cell in &even {
            assert!(
                (cell.height() - even[0].height()).abs() < 0.01,
                "an even wheel drew cells of different heights",
            );
            // Against the widest octave on the wheel, so a full-size one is
            // the whole row — the scale, not just the ordering, which nothing
            // else here would notice.
            assert!(
                (cell.height() - bar.height()).abs() < 0.01,
                "a full-size octave is {} of the row, not all of it",
                cell.height() / bar.height()
            );
        }
        // A flat fringe: two tiers, the extras equal and shorter, symmetric.
        let flat = cells(&paint_octave_strip(3, 2, 0.4, 0.0));
        assert_eq!(flat.len(), 7, "three full-size octaves and two extras each end");
        let (extra, full) = (flat[0].height(), flat[2].height());
        assert!(extra < full, "the extras are not shorter than the full-size octaves");
        assert!((flat[1].height() - extra).abs() < 0.01, "a flat fringe is not flat");
        assert!((flat[6].height() - extra).abs() < 0.01, "the fringe is lopsided");
        assert!((flat[3].height() - full).abs() < 0.01, "the full-size octaves differ");
        // A graded one: the inner extra stands between the two tiers.
        let ramp = cells(&paint_octave_strip(3, 2, 0.4, 1.0));
        assert!(
            ramp[0].height() < ramp[1].height() && ramp[1].height() < ramp[2].height(),
            "the blend did not grade the fringe: {:?}",
            ramp.iter().map(egui::Rect::height).collect::<Vec<_>>()
        );
    }

    /// The thinnest extra there is comes out under a pixel of a 20pt row, and
    /// a cell that is not there says the octave is not either — which is what
    /// `CELL_MIN_H` exists to stop. No other fixture reaches it: the floor
    /// only binds under about a seventh of a full-size octave, and every other
    /// strip painted here sits well above that.
    #[test]
    fn the_thinnest_extra_still_draws() {
        let shapes = paint_octave_strip(5, 3, MIN_EXTRA_SIZE, 0.0);
        let bar = filled_rects(&shapes)[0].0;
        let cells = cells(&shapes);
        assert_eq!(cells.len(), 11, "five full-size octaves and three extras each end");
        // The fixture has to actually reach the floor, or this passes on a
        // cell the clamp never touched. Unclamped it is 0.1 of an even slice
        // against a full-size octave of 2.08 — 0.96pt of a 20pt row.
        let wheel = octave_layout(5, DEFAULT_CENTER, 3, MIN_EXTRA_SIZE, 0.0);
        let ratio = (wheel.bounds[1] - wheel.bounds[0]) / (wheel.bounds[4] - wheel.bounds[3]);
        let unclamped = ratio * bar.height();
        assert!(unclamped < CELL_MIN_H, "the fixture does not reach the floor: {unclamped}pt");
        let extra = cells[0].height();
        assert!((extra - CELL_MIN_H).abs() < 0.01, "the thinnest extra drew {extra}pt");
    }

    /// What the strip says in words. The readout carries three numbers where
    /// every other bar in the pane carries one, and their ORDER is the whole
    /// of what tells a fringed wheel from its transpose — "2+5+2" and "5+2+5"
    /// are both plausible-looking readouts for the same three digits.
    #[test]
    fn the_strip_reads_out_the_wheel_it_draws() {
        let texts = |shapes: &[egui::Shape]| {
            text_boxes(shapes).into_iter().map(|(_, t)| t).collect::<Vec<_>>()
        };
        assert_eq!(
            texts(&paint_octave_strip(5, 2, 0.4, 0.0)),
            vec!["Octaves".to_owned(), "2+5+2".to_owned()],
            "a fringed wheel reads out fringe, count, fringe"
        );
        // No fringe, no plus signs: a bare count, so the readout says there is
        // nothing outside the handles rather than spelling out a zero.
        assert_eq!(
            texts(&paint_octave_strip(MAX_SPAN, 0, 0.4, 0.0)),
            vec!["Octaves".to_owned(), "11".to_owned()],
            "an unfringed wheel reads out its count alone"
        );
    }

    /// The wheel sits centered in the eleven slots, so the empty track at each
    /// end is what is left of the budget — and a slot is the same width
    /// whatever the wheel, which is what makes the strip a fixed axis to drag
    /// on rather than one that stretches under the pointer.
    #[test]
    fn the_strips_slots_are_a_fixed_axis_the_wheel_is_centered_on() {
        let bar = filled_rects(&paint_octave_strip(5, 0, DEFAULT_EXTRA_SIZE, 0.0))[0].0;
        let slot = bar.width() / MAX_SPAN as f32;
        for (count, extras) in [(5u32, 0u32), (5, 3), (11, 0), (1, 1)] {
            let drawn_cells = cells(&paint_octave_strip(count, extras, 0.4, 0.0));
            let span = count + 2 * extras;
            assert_eq!(drawn_cells.len(), span as usize, "{count}+2x{extras}: wrong cell count");
            let drawn = drawn_cells[drawn_cells.len() - 1].right() - drawn_cells[0].left();
            assert!(
                (drawn - span as f32 * slot).abs() < 1.5,
                "{count}+2x{extras} spans {drawn} of the {} its slots are worth",
                span as f32 * slot,
            );
            let middle =
                0.5 * (drawn_cells[0].left() + drawn_cells[drawn_cells.len() - 1].right());
            assert!((middle - bar.center().x).abs() < 0.5, "{count}+2x{extras} is off center");
        }
    }

    /// The handles are what say the strip has two gestures at all, and at zero
    /// extras — the state a fresh view is in — they are the only mark on it
    /// saying where one ends and the other begins. Same lesson the range bar
    /// learned: a handle under four points reads as an edge in the fill.
    #[test]
    fn the_strips_handles_sit_on_the_wheels_edges_and_read_as_handles() {
        let shapes = paint_octave_strip(5, 0, DEFAULT_EXTRA_SIZE, 0.0);
        let hs = handles(&shapes);
        assert_eq!(hs.len(), 2, "the strip did not paint two handles");
        for h in &hs {
            assert!(h.width() >= 4.0, "a handle thinner than this vanishes into the fill");
        }
        // On the outer edge of the wheel, which is what a fringe is dragged
        // out from and the count is dragged in from.
        let plain = cells(&shapes);
        let edges = (plain[0].left(), plain[4].right());
        assert!((hs[0].center().x - edges.0).abs() < 1.0, "the low handle left the edge");
        assert!((hs[1].center().x - edges.1).abs() < 1.0, "and the high one");

        // And with a fringe, where the boundary is INSIDE the wheel rather
        // than on its edge — at zero extras the two coincide, so a strip drawn
        // only there cannot tell the count's boundary from the wheel's.
        let shapes = paint_octave_strip(5, 2, 0.4, 0.0);
        let (hs, fringed) = (handles(&shapes), cells(&shapes));
        assert_eq!(fringed.len(), 9, "five full-size octaves and two extras each end");
        assert!(
            (hs[0].center().x - fringed[2].left()).abs() < 1.0,
            "the low handle is not where the fringe ends and the count starts"
        );
        assert!((hs[1].center().x - fringed[6].right()).abs() < 1.0, "nor the high one");
    }

    /// The widest wheel puts its boundary on the bar's own edge, and a handle
    /// centered there hangs half its width outside — where it reads as the
    /// border rather than as something to grab. That is the bug
    /// `the_handles_read_as_handles_even_at_the_limits` pins for the range
    /// bar, and it arrives here by a different route: not a value at the end
    /// of a scale, but a count that fills every slot of the budget.
    #[test]
    fn the_strips_handles_stay_inside_the_bar_at_the_widest_wheel() {
        let shapes = paint_octave_strip(MAX_SPAN, 0, DEFAULT_EXTRA_SIZE, 0.0);
        let bar = filled_rects(&shapes)[0].0;
        let hs = handles(&shapes);
        assert_eq!(hs.len(), 2, "the widest wheel did not paint two handles");
        for h in &hs {
            assert!(
                h.left() >= bar.left() - 0.01 && h.right() <= bar.right() + 0.01,
                "handle {h:?} hangs outside the bar {bar:?}"
            );
            assert!(h.width() >= 4.0, "a handle thinner than this vanishes into the fill");
        }
    }

    /// Which gesture a press starts is decided by the region it lands in, and
    /// the handles are the border between them. At zero extras both handles
    /// sit on the wheel's outer edge, where a nearest-handle rule would have
    /// nothing to say — and that is exactly the state you first meet.
    #[test]
    fn a_strip_press_takes_the_count_inside_the_handles_and_the_fringe_outside() {
        let inside = |reach: f32, count: u32| {
            matches!(StripGrab::at(reach, count, 0), StripGrab::Count { .. })
        };
        assert!(inside(0.0, 5), "the middle of the wheel is the count");
        assert!(inside(2.4, 5), "just inside the handle is the count");
        assert!(!inside(2.6, 5), "just outside it is the fringe");
        assert!(!inside(4.0, 5), "and so is the empty track past the wheel");
        // The grab remembers the extras it started with, not the ones the
        // budget later leaves — see the round trip below.
        assert!(matches!(StripGrab::at(0.0, 5, 3), StripGrab::Count { extras: 3 }));
    }

    /// Half a slot of travel per octave, in both gestures: the count grows at
    /// both ends of the wheel at once and so does the fringe, so the pointer
    /// is always on the boundary it is dragging.
    #[test]
    fn a_strip_drag_moves_half_a_slot_an_octave() {
        let count = StripGrab::Count { extras: 0 };
        for (reach, want) in [(2.5f32, 5u32), (3.5, 7), (1.5, 3), (5.5, 11)] {
            assert_eq!(count.apply(reach).0, want, "{reach} slots out is not {want} octaves");
        }
        // Measured from the edge of the count, so the fringe reads as octaves
        // added to the wheel rather than as a position on the strip.
        for (reach, want) in [(2.5f32, 0u32), (3.4, 1), (4.5, 2), (5.5, 3)] {
            assert_eq!(
                StripGrab::Extras { count: 5 }.apply(reach).1,
                want,
                "{reach} slots out is not {want} extras past a count of five",
            );
        }
    }

    /// The budget is eleven slices, and the count is what wins inside it: a
    /// drag that raises the count past what the fringe leaves takes the
    /// extras with it, and dragging home again brings them back. The grab
    /// holding the extras the gesture STARTED with is what buys the second
    /// half — re-reading the yielded number every frame would make one drag
    /// out and back a one-way trip.
    #[test]
    fn raising_the_count_yields_the_extras_and_dragging_home_restores_them() {
        let grab = StripGrab::at(0.0, 5, 3);
        assert_eq!(grab.apply(2.5), (5, 3), "the wheel it started on");
        assert_eq!(grab.apply(4.5), (9, 1), "the extras did not yield to the count");
        assert_eq!(grab.apply(5.5), (11, 0), "and the last of them at the ceiling");
        assert_eq!(grab.apply(2.5), (5, 3), "dragging home did not restore the fringe");
        // The fringe cannot overrun the budget either, and a count of one is
        // only drawable with a pair to flank it.
        assert_eq!(
            StripGrab::Extras { count: 5 }.apply(9.0),
            (5, 3),
            "the fringe overran the budget"
        );
        assert_eq!(StripGrab::Count { extras: 0 }.apply(0.0), (MIN_SPAN, 0));
        assert_eq!(StripGrab::Count { extras: 2 }.apply(0.0), (1, 2));
    }

    /// The mirror of the round trip above, and the one the FRINGE gesture
    /// needs for itself. `clamp_wheel` opens a lone full-size octave to two
    /// when the fringe leaves it — its answer for a blob that asks for an
    /// undrawable wheel — so a fringe drag that reads the live count back
    /// moves the count, and then measures every later frame of the same drag
    /// from the number it just moved. Dragging the fringe in off a lone octave
    /// and back out has to land where it started.
    #[test]
    fn a_fringe_drag_off_a_lone_octave_comes_home() {
        // Frame by frame the way `show` runs it: the grab is taken once at the
        // press and every frame after re-applies it.
        let played = |start: (u32, u32), press: f32, frames: &[f32]| {
            let grab = StripGrab::at(press, start.0, start.1);
            assert!(matches!(grab, StripGrab::Extras { .. }), "{press} slots out is the fringe");
            let mut wheel = start;
            for &reach in frames {
                wheel = grab.apply(reach);
            }
            wheel
        };
        // A lone octave with one extra a side — the picture MIN_COUNT exists
        // for — nudged in half a slot and back out to the pixel it started on.
        assert_eq!(
            played((1, 1), 1.4, &[1.4, 0.9, 1.4]),
            (1, 1),
            "a fringe drag out and home moved the count"
        );
        // And the long version, across the whole strip.
        assert_eq!(
            played((1, 5), 5.4, &[5.4, 3.5, 0.9, 3.5, 5.4]),
            (1, 5),
            "the widest fringed wheel did not come home"
        );
    }

    /// Drive real gestures over a 300pt strip: for each `(press, release)`,
    /// press that many slots out from the middle, drag to the second and let
    /// go. Answers the wheel left behind. Through a real `egui::Context` with
    /// real pointer events, which is the only way to reach what the widget
    /// does with egui's own drag threshold — and, across two gestures, with
    /// the grab it remembers between them.
    fn drag_strip(start: (u32, u32), gestures: &[(f32, f32)]) -> (u32, u32) {
        const W: f32 = 300.0;
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(W, 100.0));
        let (mut count, mut extras) = start;
        let track = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut t = 0.0;
        let mut frame = |count: &mut u32, extras: &mut u32, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let response = OctaveStrip::new(count, extras, 0.4, 0.0).show(ui);
                    track.set(response.rect);
                },
            );
        };
        // A frame with no input first: egui resolves the pointer against the
        // PREVIOUS pass's widget rects, so the strip has to have been laid out
        // once before a press can land on it.
        frame(&mut count, &mut extras, vec![]);
        let bar = track.get();
        let at = |slots: f32| {
            egui::pos2(bar.left() + bar.width() * (0.5 + slots / MAX_SPAN as f32), bar.center().y)
        };
        for &(press, release) in gestures {
            let toward = (release - press).signum();
            frame(&mut count, &mut extras, vec![egui::Event::PointerMoved(at(press))]);
            frame(&mut count, &mut extras, vec![
                egui::Event::PointerMoved(at(press)),
                egui::Event::PointerButton {
                    pos: at(press),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            // A small step first, then the rest of the way. egui does not call
            // a gesture a drag until the pointer has left a six-point click
            // threshold, so this step is where the widget first sees one — and
            // a step of about half a slot is what a real hand produces at
            // 60fps.
            let step = at(press + 0.44 * toward);
            frame(&mut count, &mut extras, vec![egui::Event::PointerMoved(step)]);
            frame(&mut count, &mut extras, vec![egui::Event::PointerMoved(at(release))]);
            // And let go, which is the only thing that forgets the grab.
            frame(&mut count, &mut extras, vec![egui::Event::PointerButton {
                pos: at(release),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]);
        }
        (count, extras)
    }

    /// The wiring, once: a real press inside the handles dragged outward
    /// changes the COUNT and not the fringe, however far past the handles it
    /// ends up — the grab is taken at the press.
    #[test]
    fn a_real_drag_on_the_strip_keeps_the_gesture_it_started() {
        // A five-octave count reaches 2.5 slots of eleven either side of the
        // middle, so this presses inside the right-hand handle and drags well
        // past it into what is fringe.
        assert_eq!(
            drag_strip((5, 2), &[(2.0, 4.5)]),
            (9, 1),
            "the drag did not carry the count out to the pointer, or the fringe \
             yielded more than the budget demanded"
        );
    }

    /// Letting go forgets which gesture was being held. egui's temp store has
    /// no expiry, so a grab left behind is inherited by the NEXT press — and
    /// since it is read before `at` is consulted, that press never gets to
    /// choose. One stale count grab would make the fringe unreachable for the
    /// rest of the session.
    #[test]
    fn a_second_gesture_on_the_strip_chooses_for_itself() {
        // Drag the count out to nine, let go, then press in the fringe past
        // its new boundary at 4.5 slots and pull outward. Holding the first
        // grab, that second drag would read as a count and land on eleven.
        assert_eq!(
            drag_strip((5, 0), &[(2.0, 4.5), (5.0, 5.4)]),
            (9, 1),
            "the second gesture inherited the first one's grab"
        );
    }

    /// A press within egui's own six-point click threshold of a handle, which
    /// is where half of the handle IS. egui reports no drag until the pointer
    /// has moved that far, so the first frame the widget can decide anything
    /// on is already past the boundary — and this control splits its two
    /// gestures on a hard line, where a `RangeBar` handle has fourteen points
    /// of reach around it. Reading the live pointer there hands the canonical
    /// gesture, grab the handle and pull it out, to the fringe.
    #[test]
    fn a_press_just_inside_a_handle_still_takes_the_count() {
        // 2.4 slots out on a 300pt strip is 2.7pt inside the boundary, and the
        // handle is drawn 2pt either side of it — so this is a press ON the
        // affordance, dragged the way it invites.
        assert_eq!(
            drag_strip((5, 0), &[(2.4, 4.5)]),
            (9, 0),
            "a press on the handle's inner half grew a fringe instead of the count"
        );
        // The mirror: just OUTSIDE the handle, dragging inward, is the fringe.
        assert_eq!(
            drag_strip((5, 2), &[(2.6, 3.6)]),
            (5, 1),
            "a press just outside the handle moved the count"
        );
    }
}
