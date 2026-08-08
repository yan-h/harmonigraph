//! Custom controls. `ValueBar` is the workhorse: a flat, DAW-style
//! parameter bar (drag anywhere to set, double-click to type a value)
//! that replaces egui's rail-and-knob `Slider` + separate `DragValue`.
//! `RangeBar` is its two-handle sibling, for a pair of values that bound a
//! span rather than one value on a scale. `OctaveStrip` is the octave wheel's
//! own — two counts and the profile they produce, in one row.

use std::ops::RangeInclusive;

use egui::{CornerRadius, Key, Response, Sense, TextEdit, TextStyle, Ui, Vec2};
use harmonigraph_scene::{
    clamp_wheel, hue_circle, octave_layout, pitch_ramp_lut, Gradient, ViewConfig,
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

/// Segments a [`fade_span`](RangeBar::fade_span) fill is drawn in. The ramp
/// covers only part of that fill and the bar is a couple of hundred points
/// wide at most, so this is already finer than the pixels it lands on — and it
/// is a whole-fill count rather than a per-point one because the strip is
/// sampled over its own width, which the span does not fix.
const FADE_SEGMENTS: usize = 64;

/// Which part of a [`RangeBar`] a drag took hold of. Decided once, at
/// drag-start, and remembered for the gesture — otherwise dragging one end
/// past the other would hand the drag to whichever handle is nearest now.
/// (`Default` is derived only to satisfy egui's `remove_temp` bound; the
/// value is always written by drag-start before anything reads it.)
#[derive(Clone, Copy, Debug, Default)]
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
        // A CLOSED span has no middle to take hold of and no side to either
        // handle: both ends stand on one point, so which gesture a press
        // starts is a rule rather than a measurement. Below it the LOW end,
        // which is the only end that can open a closed span and opens it
        // downward; at or above it the span SLIDES, keeping its width.
        //
        // Both halves are load-bearing for a [`fade_span`](RangeBar::fade_span)
        // bar, where the pair is a reach and the fade that ends it: closed
        // means a HARD EDGE, which is an ordinary setting rather than a
        // degenerate one, and the two gestures it needs are widening the reach
        // without softening it (slide) and softening it (the low end). Without
        // this the tie below hands every press to `Low`, which is pinned
        // against `hi` and cannot move — a hard edge the bar can neither widen
        // nor soften.
        //
        // A bar that declares a `min_span` reaches this too, and its slide is
        // what repairs the pair rather than what breaks it: `min_span` bounds
        // what a bar PRODUCES, so a closed or inverted pair still arrives from
        // a host param or a blob. `apply` floors the slid width there.
        if hi <= lo {
            return if v < lo {
                Grab::Low
            } else {
                Grab::Span { offset: v - lo, width: 0.0 }
            };
        }
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
                // A width the gesture froze can be under the minimum, because
                // `min_span` bounds what this bar PRODUCES and not what it was
                // handed: a closed or inverted pair reaches it from a host
                // param or a blob, and closed is the one shape the slide is
                // the repair for. Floored here rather than at the grab, which
                // measures the pair it found; the walls below already open to
                // the minimum, so this is the interior agreeing with them.
                let width = width.max(min_span);
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
/// **Named on the bar, exactly where a [`ValueBar`] names itself**, so a range
/// costs the one row it is worth rather than a row for the control and a row
/// for a label above it. A settings column is then one shape repeated down its
/// whole length, which is what makes it scannable.
///
/// **Each end still reads out beside its own handle**, which is where a range's
/// numbers mean the most, and three text runs fit a 20pt row because the name's
/// zone is taken OUT of the room the numbers roam in — see [`Self::show`] for
/// the arithmetic that makes that provable rather than lucky.
///
/// The pair does NOT park together at the right, the way [`SpreadBar`]
/// spells its two ends into one readout, and the reason is the thumb rather
/// than the room: a parked run is crossed by any handle dragged past about
/// four fifths of the bar, which is where the Level bar's ceiling and the Band
/// bar's outer radius both sit at rest, and the thumb and the digits are the
/// same near-white — so whichever of the two paints last, the other cannot be
/// read. A number goes in a run of CLEAR bar instead, which is what keeps a
/// thumb's own width between it and every thumb; swept with the pitch range's
/// `hz_readout`, the widest readout any pane asks for, no thumb stands in a
/// number at 300pt or above, and the settings column opens around 423.
///
/// Under about 240pt that stops being reachable — a span narrower than the two
/// numbers it carries has no run of clear bar left that holds them — and what
/// the placement spends the remaining room on is reading ORDER, low then high
/// and both still on the bar. Order is what makes them a range rather than two
/// numbers.
///
/// **The NAME is crossed by the low handle** where the numbers are not, and
/// that is the trade this row makes rather than an oversight. A thumb roams the
/// whole track, so no fixed text can dodge it; the name is the run that can
/// afford it, because a word you already know survives losing a letter where a
/// number does not survive losing a digit. The name's own share of the bar is
/// about a sixth of the axis at the width the settings column opens at, a
/// tenth on a bar twice that wide.
///
/// Most bars only reach it while the low end is DRAGGED there: the two that
/// open at the full axis stand their low handle a point clear of the name, and
/// the Level and Band bars open at 40% and 66% of theirs. The two
/// [`fade_span`](RangeBar::fade_span) bars rest inside it, and the Gutter does
/// so at a fresh install — its low end is where the gutter stops being solid,
/// which on a nearly-fully-soft default is 1.4% of the axis, so the thumb
/// stands on the "G". That is the trade taken knowingly: the alternative is a
/// fresh look chosen to keep a handle off a letter, which is the picture
/// paying for the panel.
///
/// Letting the name slide out of the way instead was measured and dropped: it
/// has to snap back the moment the handle passes it, and a name jumping the
/// width of itself mid-drag reads worse than a covered letter.
pub struct RangeBar<'a> {
    low: &'a mut f32,
    high: &'a mut f32,
    range: RangeInclusive<f32>,
    label: &'a str,
    /// Closest the two ends may come, in value units — the range can be
    /// narrowed but never collapsed.
    min_span: f32,
    /// Whether a drag lands on whole values only (see [`RangeBar::integer`]).
    integer: bool,
    /// Whether the span is painted as a fade off the end of a fill (see
    /// [`RangeBar::fade_span`]) rather than as a filled slice of the track.
    fade_span: bool,
    display: fn(f32) -> String,
}

impl<'a> RangeBar<'a> {
    pub fn new(
        low: &'a mut f32,
        high: &'a mut f32,
        range: RangeInclusive<f32>,
        label: &'a str,
    ) -> Self {
        RangeBar {
            low,
            high,
            range,
            label,
            min_span: 0.0,
            integer: false,
            fade_span: false,
            display: |v| format!("{v:.2}"),
        }
    }

    pub fn min_span(mut self, span: f32) -> Self {
        self.min_span = span;
        self
    }

    /// Paint the pair as a REACH and the fade that ends it: the fill starts at
    /// the axis floor rather than at `low`, runs solid to `low`, and ramps out
    /// to nothing by `high`.
    ///
    /// For the pairs that describe a soft edge — the lattice's knockout gutter
    /// and the piano roll's note outline. Both are two distances from the same
    /// place (the node's rim, the note's edge), so they are already two points
    /// on one axis, and the ordinary two-handle reading of them is the true
    /// one: solid out to `low`, gone by `high`. What the fill adds is that the
    /// bar then LOOKS like the edge it sets — the ramp on the track is the ramp
    /// on screen — which the default paint, a bright slice floating over bare
    /// track, says the opposite of: it fills exactly the part that is fading
    /// and leaves the solid part bare.
    ///
    /// Only the paint. The gestures are a range's own, and they are everything
    /// a bar apiece would give: sliding the span is the reach at a fixed
    /// fade, and the low end is the fade at a fixed reach. Which is the whole
    /// reason this is one CONTROL and not one NUMBER — a fade tied to its
    /// reach as a fraction would make a wider edge always a blurrier one, and
    /// there would be no way to ask for a wide crisp gutter or a narrow soft
    /// one.
    pub fn fade_span(mut self) -> Self {
        self.fade_span = true;
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
        if self.fade_span {
            // One fill from the axis floor out to `high`, solid as far as
            // `low` and ramping to the well over the rest — the picture of the
            // edge this pair sets. Mixed toward the WELL rather than painted
            // with alpha, so the ramp ends on exactly the color the bare track
            // beyond it already is and the fill has no seam at its own end.
            //
            // It stands on the TRACK rather than on the bar's rect, so that
            // its extent is the reach and nothing else: the rect starts a
            // handle's half-width earlier (see HANDLE_INSET), which would
            // leave a stub of fill under an edge switched off entirely and
            // overstate every reach above it by the same amount. What that
            // costs is a sliver of bare track at the far left, which is the
            // same sliver every handle needs to sit clear in.
            let mut fill = rect;
            fill.min.x = track.left();
            fill.max.x = hx.max(track.left());
            // Where the ramp starts as a fraction of the FILL, which is what
            // `gradient_strip` measures its samples in. A hard edge closes the
            // span, which puts the whole fill solid and the ramp nowhere — and
            // is the one value the divide below cannot take.
            let solid = ((lx - fill.left()) / fill.width().max(1.0)).clamp(0.0, 1.0);
            gradient_strip(painter, fill, FADE_SEGMENTS, f32::from(bar_radius(scale)), |p| {
                if p <= solid || solid >= 1.0 {
                    fill_color
                } else {
                    let t = (p - solid) / (1.0 - solid);
                    egui::lerp(egui::Rgba::from(fill_color)..=egui::Rgba::from(theme::well()), t)
                        .into()
                }
            });
        } else {
            let mut span = rect;
            span.min.x = lx;
            span.max.x = hx;
            painter.rect_filled(span, radius, fill_color);
        }

        // The name first, in the same place and the same faces a ValueBar puts
        // its own. Values in monospace: digits align and don't wiggle as they
        // change.
        let text_color = if response.hovered() || response.dragged() {
            theme::text()
        } else {
            theme::text_dim()
        };
        let mono = TextStyle::Monospace.resolve(ui.style());
        let text_gap = TEXT_GAP * scale;
        let width_of =
            |text: String| painter.layout_no_wrap(text, mono.clone(), theme::text()).size().x;
        // Room kept clear for the two numbers, measured END BY END from the
        // widest string each end can produce rather than from the pair in the
        // bar now. Measuring what is in it makes the name re-elide the moment
        // a number gains a digit — the name wobbling under the pointer
        // mid-drag, which is exactly what the monospace face buys the digits
        // themselves. The ends of the RANGE bound each end's own maximum for a
        // plain decimal readout, and the value in hand is in the maximum as
        // well so that a `display` whose length is not monotonic in the value
        // can still never be overlapped.
        let widest_end = |current: f32| {
            [min, max, current]
                .into_iter()
                .map(|v| width_of((self.display)(v)))
                .fold(0.0f32, f32::max)
        };
        let reserve = widest_end(*self.low) + text_gap + widest_end(*self.high);
        let body = TextStyle::Body.resolve(ui.style());
        let mut job =
            egui::text::LayoutJob::simple_singleline(self.label.to_owned(), body, text_color);
        let text_pad = BAR_TEXT_PAD * scale;
        job.wrap.max_width =
            (rect.width() - 2.0 * text_pad - BAR_LABEL_GAP * scale - reserve).max(0.0);
        job.wrap.max_rows = 1;
        job.wrap.overflow_character = Some('\u{2026}');
        let label = painter.layout_job(job);
        let label_width = label.size().x;
        let centered =
            |galley: &egui::Galley, x: f32| egui::pos2(x, rect.center().y - galley.size().y * 0.5);
        painter.galley(centered(&label, rect.left() + text_pad), label, text_color);

        // What is left of the row once the name has taken its place, and the
        // only part of the bar the numbers are allowed into. It is what keeps
        // three text runs out of each other's way: the name was laid out
        // against a width with `reserve` already subtracted, so as long as the
        // name got the width it asked for, what remains holds both readouts
        // side by side — the name can no more be pushed off by a number than a
        // number can push into the name. A bar too narrow to grant even the
        // elided name its width is past that (the readouts then take what room
        // there is and the containment below is the only promise left), which
        // is well under the 120pt the panes are held to.
        let region_left = rect.left() + text_pad + label_width + BAR_LABEL_GAP * scale;
        let region_right = rect.right() - text_gap;

        let handle_w = HANDLE_W * scale;
        let half_handle = handle_w * 0.5;
        let reach = half_handle + text_gap;
        let low = painter.layout_no_wrap((self.display)(*self.low), mono.clone(), theme::text());
        let high = painter.layout_no_wrap((self.display)(*self.high), mono, theme::text());
        let (low_w, high_w) = (low.size().x, high.size().x);
        // The three runs of clear bar the two thumbs leave inside the region:
        // outside the span either side, and between the handles. Each is
        // clipped to the region, which is what holds the numbers off the name
        // — a handle parked under the name (the low end at the bottom of its
        // axis, where the two bars that open at the full range both sit) would
        // otherwise open a run that starts inside the name's own letters.
        let clipped = |(start, end): (f32, f32)| {
            (start.max(region_left), end.min(region_right))
        };
        let gaps = [
            clipped((region_left, lx - reach)),
            clipped((lx + reach, hx - reach)),
            clipped((hx + reach, region_right)),
        ];
        // First choice is each number beside its own handle, on the empty track
        // outside the span where it sits on flat black and reads cleanly —
        // snug against the handle it names. When the span has grown too close
        // to that end of the bar to leave room, it moves inside instead, over
        // the fill. (At the full range there is no empty track at all, so both
        // go inside.)
        let low_left = if gaps[0].1 - gaps[0].0 >= low_w { gaps[0].1 - low_w } else { gaps[1].0 };
        let high_left =
            if gaps[2].1 - gaps[2].0 >= high_w { gaps[2].0 } else { gaps[1].1 - high_w };
        // A number with a thumb standing in it is the one arrangement this bar
        // cannot ship: the thumb is drawn in the same near-white as the digits,
        // so the crossing swallows a character whichever paints last, and "-60
        // dB" reads "-60 B". A span narrower than the numbers it carries has no
        // room beside its handles for both, so when the first choice would be
        // crossed — or would run the two numbers into each other or into the
        // name — the pair travels TOGETHER into the widest clear run instead,
        // and reads as the pair it is a little way off the span it describes.
        let uncrossed = |left: f32, w: f32| {
            left >= region_left
                && left + w <= region_right
                && [lx, hx].iter().all(|&x| x + half_handle <= left || x - half_handle >= left + w)
        };
        let apart = low_left + low_w + text_gap <= high_left;
        let (low_left, high_left) =
            if uncrossed(low_left, low_w) && uncrossed(high_left, high_w) && apart {
                (low_left, high_left)
            } else {
                let pair = low_w + text_gap + high_w;
                let widest = gaps
                    .iter()
                    .copied()
                    .enumerate()
                    .max_by(|(_, a), (_, b)| (a.1 - a.0).total_cmp(&(b.1 - b.0)));
                match widest {
                    // Right-aligned in the run BELOW the span, left-aligned in
                    // either of the others, so the pair sits as near the span
                    // it names as the run allows.
                    Some((0, gap)) if gap.1 - gap.0 >= pair => (gap.1 - pair, gap.1 - high_w),
                    Some((_, gap)) if gap.1 - gap.0 >= pair => {
                        (gap.0, gap.0 + low_w + text_gap)
                    }
                    // No run holds both — a row this narrow has none left that
                    // does. What survives is reading ORDER: low then high,
                    // as near the span as the region allows, and a thumb
                    // crossing one of them. Order is what makes them still a
                    // range rather than two numbers, and it is the last thing
                    // worth spending the room on.
                    _ => {
                        let start = low_left
                            .max(region_left)
                            .min((region_right - pair).max(region_left));
                        (start, start + low_w + text_gap)
                    }
                }
            };
        // Never let a readout escape the bar, however cramped the row: off the
        // bar it is off the pane, where horizontal scrolling is deliberately
        // off and it can be neither read nor dragged to. `max`/`min` rather
        // than `clamp`, which asserts `min <= max` and takes the editor down
        // with it — see `SpectrumConfig::sanitize`, which names the same trap.
        let contain = |left: f32, w: f32| {
            let floor = rect.left() + text_gap;
            left.max(floor).min((rect.right() - text_gap - w).max(floor))
        };
        for (galley, left) in [(low, contain(low_left, low_w)), (high, contain(high_left, high_w))]
        {
            painter.galley(centered(&galley, left), galley, theme::text());
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
/// [`Gradient::MAX_HUE_SPAN`] is exactly this. Widen that constant and
/// `sanitized` would accept spans the bar cannot draw: the handle would park at
/// the right edge and stop answering while the value went on growing, with
/// nothing to fail at compile time.
/// `the_spectrum_bar_track_is_a_whole_turn_of_the_span_knob` is what catches it.
const FULL_TURN: f32 = 360.0;

/// How far back the hues a gradient does not reach are held. Through alpha over
/// the well rather than a blend toward a fixed grey, so the bar sits on
/// whatever the pane is instead of ringing itself in a slightly wrong color.
const UNCLAIMED_ALPHA: f32 = 74.0 / 255.0;

/// Height of the pitch-order strip under a [`SpectrumBar`]'s track. Shorter
/// than the track, because it is a picture and not a control: a press on it
/// is declined ([`SpectrumGrab::Outside`]), and a full-height second bar would
/// say otherwise.
const STRIP_H: f32 = 11.0;

/// The pane showing between the three pieces of a [`SpectrumBar`] — beside the
/// button, and between the track and the strip. ONE value for both, because
/// what it buys is that the three read as one control: two gaps of different
/// sizes would group the pieces, and the grouping would be wrong either way
/// round.
///
/// Narrow enough to be tighter than the pane's own row spacing, so the strip
/// belongs to the track above it rather than to the bar below.
const PIECE_GAP: f32 = 2.0;

/// A [`SpectrumBar`] stands two rows tall: the track, the strip under it, and
/// the gap between.
fn spectrum_bar_height(scale: f32) -> f32 {
    (BAR_HEIGHT + PIECE_GAP + STRIP_H) * scale
}

/// What a [`SpectrumBar`]'s track and strip measure in a column `column` points
/// wide: the column, less the flip button at the left end and the gap beside
/// it.
///
/// Shared with the settings tests, whose sweep pins every bar in a pane to the
/// width of its column. This is the one bar that is narrower, and the sweep has
/// to know by how much or it is choosing between failing on a bar that is
/// correct and passing on one that has stopped tracking the column.
pub(crate) fn spectrum_track_width(column: f32, scale: f32) -> f32 {
    (column - (FLIP_W + PIECE_GAP) * scale).max(0.0)
}

/// Width of the flip button at the LEFT end of a [`SpectrumBar`], taken out of
/// the row the bar already has rather than off a row of its own.
///
/// It costs the track that much travel, which is the whole trade and a cheap
/// one: the track stands for a whole turn at any length, so a shorter one is a
/// coarser drag and nothing else — 18pt of 400 is a twentieth of a degree per
/// pixel. A row costs 20pt of a column that already scrolls.
///
/// The left end rather than the right because a settings pane scrolls, and its
/// scroll bar is drawn INSIDE the column over the right edge of every bar in
/// it. A track under it loses nothing that can be read — the dimmed remainder
/// says the least of anything on the bar — but a button under it is a button
/// with a bar across it, and the last few pixels of one are not clickable at
/// all. Nothing else in the row minds the swap: the arc is laid out from the
/// track's own left edge either way, so it still reads low-to-high, left to
/// right.
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
    /// A press that landed off the track, which for this widget means on the
    /// pitch-order strip below it.
    ///
    /// A rectangle the strip is not inside is NOT enough to keep a press on it
    /// out of the track, and that is the whole reason this variant exists:
    /// egui's hit test gathers every widget within
    /// `interaction.interact_radius` of the pointer and, when the press hits
    /// none of them squarely, gives it to the nearest. At the default radius of
    /// 5 the track reaches five points below its own bottom edge — past
    /// [`PIECE_GAP`] and into the strip — so the widget is handed drags it has
    /// to decline by position.
    ///
    /// Remembered for the gesture like the other two, so a drag that started
    /// off the track does not catch hold the moment the pointer crosses onto
    /// it.
    Outside,
}

/// The gradient a double-click goes home to when the caller names none: the
/// lattice's, which a fresh view opens with.
///
/// Read off [`ViewConfig::default`] for the reason [`reset_wheel`] is, and
/// the drift it warns about is live here rather than hypothetical:
/// `ViewConfig::default` COMPOSES its gradient — a shorter arc over a
/// shallower brightness ramp — instead of taking `Gradient::default()`,
/// which is the type's own CIELAB-converted arc. Resetting to the type's
/// default lands the bar on a pair the plugin has never opened on, and the
/// bars carry no text entry to dial it back with, so the shipped arc would
/// be unrecoverable by gesture.
///
/// The same argument is why a bar over some OTHER gradient has to say so:
/// the Spectral pane's heatmap has a default of its own, and a double-click
/// there landing on the lattice's arc would be that same unrecoverable jump
/// one pane over. [`SpectrumBar::home`] and [`SpreadBar::home`] are where it
/// says so.
fn default_home() -> Gradient {
    ViewConfig::default().pitch_gradient
}

/// The pitch gradient's hue arc, as three pieces of one control: the button
/// that reverses it at the left end, and beside that a full turn of the color
/// circle laid along a track, CUT at the arc's own start, with the stretch the
/// gradient walks filled from the track's left edge and the hues it does not
/// reach dimmed beyond it. Under the track, a gap below it, the gradient itself
/// in pitch order, low note on the left.
///
/// Every piece wears the shared [`CONTROL_RADIUS`](theme::CONTROL_RADIUS) and
/// sits on the pane, with no frame drawn round the set: the button is a button
/// down to the table its colors are read out of, and the two bands round
/// exactly like the fill of a [`ValueBar`] above them. A well large enough to
/// hold all three would ring the control in a border nothing else in a settings
/// pane wears — see [`gradient_strip`], which is where the alternative was paid
/// for.
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
/// **It previews all six knobs, not just the one it sets.** The claimed
/// stretch is painted straight out of [`pitch_ramp_lut`], the same table the
/// lattice draws from, so brightness and chroma show up in it too and the
/// preview cannot drift from the picture. A swatch drawn from the widget's own
/// idea of the gradient would be a second definition of the color, wrong the
/// first time either changed. The dimmed remainder comes from [`hue_circle`]
/// at the gradient's BASE lightness and chroma — the middle of each of its two
/// ramps — so it reads as the same gradient continued rather than as decoration.
///
/// Which means it meets the claimed arc flush only when both ramps are FLAT:
/// the arc ends at the top of them, and the remainder carries on from the
/// middle, so a steep ramp puts a step at the handle. Continuing the ramp
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
/// **The strip is not redundant with the track.** They agree wherever both say
/// anything — the claimed stretch IS the gradient — but the claimed stretch is
/// as wide as the span, so at a span of zero it has no width at all, and a
/// single-hue gradient with a brightness ramp is a real setting that the track
/// alone would draw as nothing. It is also the one place the gradient is drawn
/// at a fixed scale: the track squeezes it into whatever fraction of the turn
/// the arc claims, so a narrow arc's ramp is a sliver there and full width
/// here.
///
/// It sits a hair under the track — [`PIECE_GAP`], the one gap this control
/// uses, which is also what separates the button from both — rather than a
/// bar's worth of space away. Three pieces on the pane at one rhythm read as
/// one control; anything looser reads as a bar with things near it.
///
/// **What the flip changes on screen is the sign, and both ramps' ends.** The
/// claimed stretch and the strip are the same pitch ramp, low note at the left,
/// so both reverse with the gradient — which is exactly the change, drawn where
/// the change is. The readout spells the direction out on top of that, because
/// an arc and its flip claim exactly the same colors.
pub struct SpectrumBar<'a> {
    gradient: &'a mut Gradient,
    home: Gradient,
}

impl<'a> SpectrumBar<'a> {
    pub fn new(gradient: &'a mut Gradient) -> Self {
        SpectrumBar { gradient, home: default_home() }
    }

    /// The gradient a double-click on the track takes the ARC home to — only
    /// its two hue fields, those being the only ones the track sets. Defaults
    /// to the lattice's; see [`default_home`] for why a bar over any other
    /// gradient owes its own.
    pub fn home(mut self, home: Gradient) -> Self {
        self.home = home;
        self
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
        let piece_gap = PIECE_GAP * scale;
        let (id, rect) = ui.allocate_space(Vec2::new(width, spectrum_bar_height(scale)));
        // The track's width is what the two bands and the settings sweep all
        // measure, so the row is laid out from it rather than from the button:
        // a column too narrow to leave the track anything then gives the button
        // the row, which is the right way round. A coarse handle beats an
        // unreachable one, but a button with no width cannot be pressed at all.
        let split = rect.right() - spectrum_track_width(rect.width(), scale);
        // The button stands the full height of both rows, because it reverses
        // what both of them draw.
        let flip_rect = egui::Rect::from_min_max(
            rect.min,
            egui::pos2((split - piece_gap).max(rect.left()), rect.bottom()),
        );
        let track_rect = egui::Rect::from_min_max(
            egui::pos2(split, rect.top()),
            egui::pos2(rect.right(), rect.top() + BAR_HEIGHT * scale),
        );
        // Only the track is sensed, and the strip is a picture — but sensing
        // stops short of the strip rather than keeping presses off it. What
        // does that is `on_track` below; see [`SpectrumGrab::Outside`].
        let strip_rect = egui::Rect::from_min_max(
            egui::pos2(track_rect.left(), track_rect.bottom() + piece_gap),
            rect.max,
        );
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
        let laid_out = |g: Gradient| {
            // A span of zero has no direction of its own, and opening rightward
            // is the useful reading: dragging the handle out of nothing then
            // grows an arc rather than needing the sign set first. `sanitized`
            // is what makes the test sound, by keeping -0.0 out of the field.
            let winding = if g.hue_span < 0.0 { -1.0f32 } else { 1.0 };
            let claimed = (g.hue_span / FULL_TURN).abs().clamp(0.0, 1.0);
            (winding, claimed, track.left() + track.width() * claimed)
        };
        // Whether a point is on the control, as opposed to on the picture below
        // it. The track's sensed rectangle stops at the gap, and this is still
        // the only thing standing between a press on the strip and a gesture —
        // see [`SpectrumGrab::Outside`] for why the rectangle is not enough.
        let on_track = |p: &egui::Pos2| track_rect.contains(*p);

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
        let clicked_track = response.interact_pointer_pos().is_some_and(|p| on_track(&p));
        if response.double_clicked() && clicked_track {
            let home = self.home.sanitized();
            self.gradient.hue_start = home.hue_start;
            self.gradient.hue_span = home.hue_span;
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
                        // Whether the gesture is ours is asked of where the
                        // press LANDED, and which grab it is of where the
                        // pointer has reached. Two positions on purpose: egui
                        // only calls a press a drag once it has moved past a
                        // threshold, so by this frame a press that began on the
                        // strip may already be over the track — while the
                        // handle-or-track question is settled on the first live
                        // frame exactly as [`Grab`]'s is, so the two bars answer
                        // a mid-gesture jump the same way.
                        let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(p);
                        let grab = if !on_track(&origin) {
                            SpectrumGrab::Outside
                        } else if (p.x - handle_x).abs() <= GRAB_PX * scale {
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
                    SpectrumGrab::Span => Some(Gradient {
                        hue_span: winding * offset_at(p.x).abs(),
                        ..aimed
                    }),
                    SpectrumGrab::Rotate { held } => Some(Gradient {
                        hue_start: (held - offset_at(p.x)).rem_euclid(FULL_TURN),
                        ..aimed
                    }),
                    SpectrumGrab::Outside => None,
                };
                if let Some(next) = next.filter(|next| *next != *self.gradient) {
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
        let corner = bar_radius(scale);
        let radius = CornerRadius::same(corner);
        let painter = ui.painter();
        // The pitch ramp at `p` along itself, read out of the same table the
        // lattice draws from so the preview cannot drift from the picture. Both
        // bands want it — the track squeezed into the claimed stretch, the strip
        // end to end — and reading the table twice is how they would stop
        // agreeing.
        let ramp_at = |p: f32| {
            let f = p.clamp(0.0, 1.0) * (PITCH_LUT_N - 1) as f32;
            let i0 = f.floor() as usize;
            lut[i0].lerp(lut[(i0 + 1).min(PITCH_LUT_N - 1)], f - f.floor())
        };
        // A well under the track and nothing under the strip, because the
        // dimmed hues are drawn with alpha and need a recessed ground to sit
        // on — the same ground the unfilled end of a ValueBar shows. The strip
        // is opaque end to end and covers whatever it is given.
        painter.rect_filled(track_rect, radius, theme::well());
        gradient_strip(painter, track_rect, SPECTRUM_SEGMENTS, corner as f32, |p| {
            if claimed > 0.0 && p <= claimed {
                // Along the gradient, not around the circle: the two agree by
                // construction here, and reading the table is what keeps them
                // agreeing if they ever stop.
                scene_color(ramp_at(p / claimed), 1.0)
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

        // How far round the circle the arc reaches, read out beside the handle
        // — on the dimmed side, where it sits on flat color, and on the claimed
        // side when the arc has grown too wide to leave room there, which is
        // the same bargain a [`RangeBar`]'s ends make. One number and one
        // handle, so it needs none of the arithmetic that keeps a range's TWO
        // roaming numbers out of each other and off the name. The sign is the
        // direction, and it is spelled out because the track cannot show it:
        // an arc and its flip claim exactly the same colors.
        let font = TextStyle::Monospace.resolve(ui.style());
        // Lit by a pointer ON the track, not merely by one egui has decided the
        // track is nearest to — which reaches into the strip below. A readout
        // that brightens while the pointer is over the picture says the picture
        // is the control.
        let pointing = response.hover_pos().filter(on_track);
        let text_color = if pointing.is_some() || response.dragged() {
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

        // ---- The same gradient, in pitch order ------------------------------
        // A gap under the track, aligned with it end to end: two rows of one
        // control, not a second bar. One column per table entry, so every color
        // in the table lands on a column of its own and only the vertices
        // between two of them are interpolated.
        let strip_corner = corner as f32;
        gradient_strip(painter, strip_rect, PITCH_LUT_N - 1, strip_corner, |p| {
            scene_color(ramp_at(p), 1.0)
        });

        // ---- The flip button ------------------------------------------------
        // Painted out of the theme's own widget visuals, state for state, and
        // not out of a set of colors chosen to look like them: the fill, the
        // edge and the corner are read from the same table egui hands a
        // `Button`, and the state is picked the way `Style::interact` picks it.
        // Naming the colors here instead is how it drifts — a resting fill
        // copied correctly and a hovered edge given a scaled width the theme
        // does not scale, and a pressed state simply forgotten, so the one
        // control in the pane that does not answer a click is this one.
        let visuals = if flip.is_pointer_button_down_on() {
            &ui.style().visuals.widgets.active
        } else if flip.hovered() {
            &ui.style().visuals.widgets.hovered
        } else {
            &ui.style().visuals.widgets.inactive
        };
        painter.rect_filled(flip_rect, visuals.corner_radius, visuals.weak_bg_fill);
        if visuals.bg_stroke.width > 0.0 {
            painter.rect_stroke(
                flip_rect,
                visuals.corner_radius,
                visuals.bg_stroke,
                egui::StrokeKind::Inside,
            );
        }
        // The MARK is what this widget invents; its color is the theme's, the
        // one a button's own label is drawn in.
        flip_mark(painter, flip_rect, visuals.fg_stroke.color, scale);

        // The cursor says which gesture a press would start before committing
        // to a drag, as a RangeBar's does: the handle resizes the arc, the
        // track turns the circle under it. Off the track it says neither,
        // because a press there starts nothing — see [`SpectrumGrab::Outside`].
        match pointing {
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

/// Samples taken through each of a band's corner arcs, on top of the columns
/// the band is already drawn in.
///
/// Those columns are far too coarse to round with on their own, and the strip
/// is the case that settles it: 63 columns across the column this pane opens
/// at puts them 6pt apart, wider than the radius, so a corner crosses fewer
/// than ONE of them and is drawn from its two endpoints — a diagonal cut. The
/// track's 192 are 2pt apart and give a corner three steps, which is a chamfer.
/// egui does not antialias a mesh edge, so how finely the arc is sampled is the
/// only smoothness there is.
///
/// Both figures move with the column, and in the direction that makes the
/// strip's case the one to size for: narrow the pane and every column narrows
/// while the radius holds, so the samples matter most where the pane is widest.
const CORNER_SAMPLES: usize = 8;

/// The `L*` axis a brightness pair stands on, both ends included: 0 is black
/// and 100 is white, and a gradient is allowed to sit on either — flat, since a
/// ramp there has nowhere to open.
const L_STAR_AXIS: (f32, f32) = (0.0, 100.0);

/// The axis a chroma pair stands on: the FRACTION of the color the gamut holds
/// at that point of the curve, 0 grey and 1 as vivid as the screen goes there.
/// Both ends are settings and a pair on either is flat, exactly as a brightness
/// pair parked on black is — see [`Gradient::chroma`] for why the axis is
/// a fraction of what is available rather than a chroma.
const CHROMA_AXIS: (f32, f32) = (0.0, 1.0);

/// Which of the gradient's two stretches a [`SpreadBar`] is a bar of.
///
/// They are one control with two settings rather than two controls that
/// resemble each other, and the gradient is what makes them so: each is a
/// middle and a SIGNED ramp about it, bounded by what that middle leaves on its
/// own axis, with the two ends at `middle ± ramp/2`. What differs is the axis
/// itself and how a number on it is spelled — everything below is those two
/// answers, and nothing else varies between the bars.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Spread {
    /// `L*`, black to white.
    Brightness,
    /// The share of the color the gamut holds, grey to as vivid as it goes.
    Chroma,
}

impl Spread {
    /// The name the row carries. On the [`Spread`] rather than handed in: a bar
    /// can only be one of two things, and a label passed separately is a way for
    /// a row to name the other one.
    fn label(self) -> &'static str {
        match self {
            Spread::Brightness => "Brightness",
            Spread::Chroma => "Chroma",
        }
    }

    /// Both ends of the axis, in the units the gradient stores.
    fn axis(self) -> (f32, f32) {
        match self {
            Spread::Brightness => L_STAR_AXIS,
            Spread::Chroma => CHROMA_AXIS,
        }
    }

    /// Readout units per stored unit: an `L*` reads out as itself, a chroma
    /// fraction as a percentage of the color available where it is drawn.
    ///
    /// Also the SNAP grid, one readout unit of it, so that a drag lands both
    /// ends on numbers the readout can say exactly — see [`Self::snapped`].
    fn per_unit(self) -> f32 {
        match self {
            Spread::Brightness => 1.0,
            Spread::Chroma => 100.0,
        }
    }

    /// What follows each number in the readout: the sign a percentage is
    /// spelled with, and nothing for an `L*`, which is a bare coordinate on an
    /// axis with no unit to name.
    fn suffix(self) -> &'static str {
        match self {
            Spread::Brightness => "",
            Spread::Chroma => "%",
        }
    }

    /// The pair as the gradient holds it: a middle and a signed ramp.
    fn of(self, g: Gradient) -> (f32, f32) {
        match self {
            Spread::Brightness => (g.lightness, g.lightness_ramp),
            Spread::Chroma => (g.chroma, g.chroma_ramp),
        }
    }

    fn set(self, g: &mut Gradient, pair: (f32, f32)) {
        match self {
            Spread::Brightness => (g.lightness, g.lightness_ramp) = pair,
            Spread::Chroma => (g.chroma, g.chroma_ramp) = pair,
        }
    }

    /// The pair a bar actually writes, snapped at the ENDS rather than at the
    /// pair itself: both ends on a whole readout unit and inside the axis,
    /// which is what the readout says of them, and a readout is worth nothing
    /// once it is not the number the picture draws. The middle then lands on a
    /// whole or a half — 41 and 86 are a perfectly good pair of ends, and their
    /// middle is 63.5.
    ///
    /// Snapping the pair instead is the version that cannot be honest: a whole
    /// middle and a whole ramp of 45 reaches 41.5 and 86.5, which no rounding of
    /// the readout can say without lying half a point at both ends.
    ///
    /// Clamping the ends is also all the axis needs: a `Gradient` accepts a
    /// ramp as wide as its middle leaves it, and both ends inside the axis is
    /// the same statement made about the same two numbers — in exact arithmetic,
    /// which is what [`Self::legal`] is for.
    fn snapped(self, (centre, spread): (f32, f32)) -> (f32, f32) {
        let (min, max) = self.axis();
        let unit = self.per_unit();
        let end = |v: f32| ((v * unit).round() / unit).clamp(min, max);
        let (low, high) = (end(centre - spread * 0.5), end(centre + spread * 0.5));
        ((low + high) * 0.5, high - low)
    }

    /// The pair as [`Gradient::sanitized`] leaves it — asked of the
    /// gradient rather than restated here, which is the whole of how a bar and
    /// the type it writes to are kept from disagreeing about which pairs are
    /// legal.
    ///
    /// The last step of a write, and not a formality. [`Self::snapped`] puts
    /// both ENDS on the axis where the gradient bounds the RAMP by what its
    /// middle leaves: the same statement in exact arithmetic, and not quite the
    /// same one in f32 once the axis is a fraction. Whole `L*` recomposes
    /// exactly, so this moves nothing a brightness bar writes — none of the
    /// 10201 whole-point end pairs. A hundredth is no binary fraction, so 42 of
    /// the same 10201 chroma pairs recompose to a ramp past what their own
    /// middle holds — every one of them by exactly one ulp, 6e-8, `7%..100%`
    /// the first — and a bar writing one would leave the gradient drawing a
    /// picture off the pair the bar reads out.
    fn legal(self, pair: (f32, f32)) -> (f32, f32) {
        let mut g = Gradient::default();
        self.set(&mut g, pair);
        self.of(g.sanitized())
    }

    /// The two ends the ramp reaches, in PITCH order: the bottom of the pitch
    /// range first, whatever it happens to carry.
    ///
    /// Concrete where a middle and a signed ramp are arithmetic — these are the
    /// numbers the lowest and highest notes are actually drawn at, and they name
    /// the two handles standing under them. It is also how the sign gets said:
    /// an inverted ramp reads out backwards, high to low, where a signed number
    /// leaves the reader to work out which end it means.
    ///
    /// A tenth of a readout unit where an end is not whole, and no decimal where
    /// it is. Whole is what a drag leaves, since [`Self::snapped`] puts both ends
    /// there — but a fresh view, a double-click and a saved blob all arrive
    /// without passing it, and `ViewConfig`'s own gradient is one of them: 53
    /// over a ramp of 31 stands its ends on 37.5 and 68.5. Spelled to the whole
    /// point those read `38 → 68`, a span of 30 over a gradient that spends 31 —
    /// the readout claiming a picture the bar is not drawing, which is the one
    /// thing it cannot do and stay worth reading.
    ///
    /// Rounded to that tenth BEFORE being asked whether it is whole, which is
    /// what keeps a snapped chroma end from reading `42.0%`: a hundredth is no
    /// binary fraction, so an end snapped to 0.42 is 41.999998 percent of the way
    /// up its axis, and 42 is both what it means and what a tenth of a unit can
    /// say.
    fn readout(self, (centre, spread): (f32, f32)) -> String {
        let end = |v: f32| {
            let v = (v * self.per_unit() * 10.0).round() / 10.0;
            let n = if v == v.round() { format!("{v:.0}") } else { format!("{v:.1}") };
            format!("{n}{}", self.suffix())
        };
        format!("{} \u{2192} {}", end(centre - spread * 0.5), end(centre + spread * 0.5))
    }

    /// The widest the readout goes, for the reserve the name is elided against.
    /// Only its LENGTH matters, the numbers being monospace: three digits and a
    /// tenth at each end, plus whatever follows them. No end can carry a sign,
    /// both of them living on an axis that starts at 0.
    ///
    /// Built from the axis rather than written out, so a bar cannot be added
    /// with a reserve measured for another one's numbers.
    fn widest_readout(self) -> String {
        let (end, suffix) = (self.axis().1 * self.per_unit(), self.suffix());
        format!("{end:.1}{suffix} \u{2192} {end:.1}{suffix}")
    }
}

/// Which part of a [`SpreadBar`] a drag took hold of. The same three a
/// [`Grab`] names, decided on the first frame of the gesture and remembered for
/// the rest of it for the same reason — and the memory earns more here, because
/// these two ends may CROSS: an end dragged past its partner swaps which side
/// of the bar it stands on, so "the handle nearest the pointer" names the other
/// end by the next frame.
///
/// The ends are named for the pitch they carry rather than for where they
/// stand: [`Low`](SpreadGrab::Low) is the bottom of the pitch range, which is
/// the left-hand handle at a positive ramp and the right-hand one at a negative.
///
/// (`Default` is derived only to satisfy egui's `remove_temp` bound; the value
/// is always written by drag-start before anything reads it.)
#[derive(Clone, Copy, Debug, Default)]
enum SpreadGrab {
    #[default]
    Low,
    High,
    /// The ramp itself, sliding along the axis at a fixed width: `offset` is how
    /// far from the middle the pointer took hold and `spread` how wide the ramp
    /// was at that moment, both fixed for the gesture. [`Grab::Span`] fixes its
    /// own two for the same reason, and the squish below is the same bargain.
    Middle { offset: f32, spread: f32 },
}

impl SpreadGrab {
    /// What a drag starting at value `v` takes hold of: an end if the pointer is
    /// near one, the ramp if it is inside, otherwise the nearer end. A [`Grab`]
    /// divides a bar the same way, and the share of the ramp a handle's reach
    /// may claim is the same constant.
    fn at(v: f32, (centre, spread): (f32, f32), near: f32) -> SpreadGrab {
        let (low_end, high_end) = (centre - spread * 0.5, centre + spread * 0.5);
        let (d_low, d_high) = ((v - low_end).abs(), (v - high_end).abs());
        let nearer = if d_low == d_high {
            // A tie is the FLAT ramp — the two ends stand on the same point, so
            // which one a press takes is a rule rather than a measurement. Take
            // the end on the side the pointer is, and the ramp opens the way it
            // is dragged: up lifts the top of the pitch range, down darkens the
            // bottom, and neither leaves the picture upside down. A fixed
            // choice inverts it in whichever direction it is not — and parked
            // on black or white that is the ONLY direction, so the right way
            // round would be unreachable from there.
            if v < centre { SpreadGrab::Low } else { SpreadGrab::High }
        } else if d_low < d_high {
            SpreadGrab::Low
        } else {
            SpreadGrab::High
        };
        // A handle's reach cannot eat the whole ramp, or a narrow one would
        // have no middle left to slide along the axis.
        let reach = near.min(spread.abs() * HANDLE_REACH_SHARE);
        // And when the ramp is too narrow for the ends to have room of their
        // own, the MIDDLE takes a full reach instead — at a flat ramp all three
        // stand on one point, and a bar that could not move brightness at
        // exactly the isoluminant setting would strand anyone who dialled their
        // way into it. This is the mirror of [`Grab::at`]'s own fallback, which
        // hands a span with nowhere to slide to the nearer end.
        if reach < near {
            return if (v - centre).abs() <= near {
                SpreadGrab::Middle { offset: v - centre, spread }
            } else {
                nearer
            };
        }
        if d_low.min(d_high) <= reach {
            nearer
        } else if v > low_end.min(high_end) && v < low_end.max(high_end) {
            SpreadGrab::Middle { offset: v - centre, spread }
        } else {
            nearer
        }
    }

    /// Where the pair ends up when this grab is dragged to value `v`. Pure, so
    /// what actually matters — both ends stay on the axis, an end moves without
    /// disturbing its partner, and an end dragged past that partner inverts the
    /// ramp rather than stopping against it — is testable without a pointer.
    ///
    /// An end drag reads the pair back to find the end it is NOT moving, as a
    /// [`Grab`] reads its own partner; a middle drag reads neither, working from
    /// the width and offset its own gesture began at. Neither reads back a
    /// number it is itself writing, which is what keeps a drag from creeping
    /// while the pointer sits still.
    fn apply(self, v: f32, (centre, spread): (f32, f32), (min, max): (f32, f32)) -> (f32, f32) {
        // The pair, as the two ends it draws — which is what the gestures below
        // are actually about, and what the readout says.
        let (low_end, high_end) = (centre - spread * 0.5, centre + spread * 0.5);
        let pair = |low: f32, high: f32| ((low + high) * 0.5, high - low);
        match self {
            // One end to the pointer, its partner untouched. Past that partner
            // the ramp INVERTS rather than stopping there — the gesture keeps
            // hold of the end it grabbed, so the two simply trade sides, and
            // that is the whole of how the bright end gets to the bottom of the
            // pitch range. A [`RangeBar`] forbids exactly this, and is right to:
            // its ends bound a pitch axis, which inverted maps every pitch on it
            // backwards.
            SpreadGrab::Low => pair(v.clamp(min, max), high_end),
            SpreadGrab::High => pair(low_end, v.clamp(min, max)),
            SpreadGrab::Middle { offset, spread } => {
                let half = spread.abs() * 0.5;
                let want = v - offset;
                // Against a wall the ramp squishes rather than the drag jamming,
                // the bargain [`Grab::Span`] makes: the leading end pins and the
                // trailing one carries on with the pointer, so brightness
                // dragged toward white keeps moving instead of stopping dead.
                // Reading the width the GESTURE began with rather than the
                // squished one it just wrote is what opens it back out on the
                // way home.
                let (lo, hi) = if want - half < min {
                    (min, (want + half).clamp(min, max))
                } else if want + half > max {
                    ((want - half).clamp(min, max), max)
                } else {
                    (want - half, want + half)
                };
                // Squishing changes the ramp's width, never its direction.
                let (centre, width) = pair(lo, hi);
                (centre, width.copysign(spread))
            }
        }
    }
}

/// The stretch of an axis the pitch range spends: a two-ended bar whose ends
/// ARE the gradient's ends, the bottom of the pitch range and the top.
///
/// One bar for the gradient's two stretches, brightness and chroma, because
/// they are one thing set twice (see [`Spread`]). Drag either end to move it,
/// drag between them to slide the ramp at a fixed width, drag one end past the
/// other to swap which end of the pitch range carries the most, and
/// double-click to reset.
///
/// **A [`RangeBar`] in behaviour, and two things apart from it.** The ends may
/// cross, because crossed is a real setting here and not a broken one — it is
/// the inverted picture — where a range bar's ends bound a pitch axis that
/// inverted maps every pitch backwards. And it writes a MIDDLE and a signed
/// ramp rather than the pair it draws, because that is what a gradient holds:
/// a value at the centre of the pitch range and a signed difference between its
/// ends, so the ends are `middle ± ramp/2` and the two shapes carry exactly
/// the same information. What that buys the pane is a row: a bar per number
/// names the same two numbers and draws neither the stretch they compose nor
/// the room the axis has left for it (see `spectrum_group`).
///
/// **Nothing marks the middle**, though it is the number the gradient stores.
/// It is not a thing a gesture takes hold of — the slide takes the whole ramp —
/// and a mark on a two-ended bar reads as a third handle whatever it is drawn
/// like. The two ends are what the picture is made of and what the readout
/// says; the middle is where they happen to average.
///
/// **The readout is the two ENDS, and it runs in pitch order.** They are what
/// the picture concretely does — the `L*` the darkest and brightest notes are
/// drawn at, the color the palest and most vivid ones carry — and each of them
/// names a handle standing under it, where a centre and a signed ramp name
/// neither. Pitch order is also the only place the SIGN can live: a ramp and
/// its negative put the two handles in exactly the same places, so the bar
/// cannot draw the difference, and an inverted ramp reads out backwards
/// instead, high to low. (What the sign means for the picture is one row up, on
/// the strip under the spectrum bar, which draws the gradient in pitch order
/// and so reverses with it.)
///
/// **Both ends stay on the axis at every setting.** That is the bar's own
/// geometry — a handle off the track is not a value it can express — and
/// [`Gradient::sanitized`] holds the same line for a pair that arrives
/// from a hand-edited file instead of through a gesture.
/// `the_bar_can_only_reach_pairs_sanitize_leaves_alone` is what keeps the two
/// from drifting into disagreeing about which pairs are legal, and
/// [`Spread::legal`] is how a write earns it.
pub struct SpreadBar<'a> {
    gradient: &'a mut Gradient,
    spread: Spread,
    home: Gradient,
}

impl<'a> SpreadBar<'a> {
    /// The `L*` the bottom and the top of the range are drawn at.
    pub fn brightness(gradient: &'a mut Gradient) -> Self {
        SpreadBar { gradient, spread: Spread::Brightness, home: default_home() }
    }

    /// How much of the color available to them the bottom and the top of the
    /// range carry.
    pub fn chroma(gradient: &'a mut Gradient) -> Self {
        SpreadBar { gradient, spread: Spread::Chroma, home: default_home() }
    }

    /// The gradient a double-click takes this bar's PAIR home to — its own
    /// stretch of it, the other left alone. Defaults to the lattice's; see
    /// [`default_home`] for why a bar over any other gradient owes its own.
    pub fn home(mut self, home: Gradient) -> Self {
        self.home = home;
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let scale = theme::ui_scale(ui.ctx());
        let width = bar_width(ui);
        let (rect, mut response) =
            ui.allocate_exact_size(Vec2::new(width, BAR_HEIGHT * scale), Sense::click_and_drag());
        let axis = self.spread.axis();
        let (min, max) = axis;
        // Values live on an inset track, so both limits are places a handle can
        // stand rather than edges it merges into. See HANDLE_INSET.
        let track = rect.shrink2(Vec2::new(HANDLE_INSET * scale, 0.0));
        let x_of =
            |v: f32| track.left() + track.width() * ((v - min) / (max - min)).clamp(0.0, 1.0);
        let value_at = |x: f32| {
            min + ((x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0) * (max - min)
        };
        let pair = |g: Gradient| self.spread.of(g);

        // ---- Interaction ----------------------------------------------------
        let grab_id = response.id.with("spread_grab");
        let near = GRAB_PX / track.width().max(1.0) * (max - min);
        // Reset rather than text entry, the bargain a [`RangeBar`] makes: a bar
        // holding two numbers has no single value to type into it.
        if response.double_clicked() {
            self.spread.set(self.gradient, self.spread.of(self.home.sanitized()));
            response.mark_changed();
        }
        if response.dragged() {
            if let Some(p) = response.interact_pointer_pos() {
                let v = value_at(p.x);
                let aimed = pair(self.gradient.sanitized());
                // Read and write as separate statements: nesting a `data_mut`
                // inside a `data` closure takes the context lock twice.
                let stored = ui.data(|d| d.get_temp::<SpreadGrab>(grab_id));
                let grab = match stored {
                    Some(grab) => grab,
                    None => {
                        let grab = SpreadGrab::at(v, aimed, near);
                        ui.data_mut(|d| d.insert_temp(grab_id, grab));
                        grab
                    }
                };
                let next = self.spread.legal(self.spread.snapped(grab.apply(v, aimed, axis)));
                if next != pair(*self.gradient) {
                    self.spread.set(self.gradient, next);
                    response.mark_changed();
                }
            }
        }
        if response.drag_stopped() {
            ui.data_mut(|d| d.remove_temp::<SpreadGrab>(grab_id));
        }

        // ---- Paint ----------------------------------------------------------
        // The pair read BACK, not the one the gesture was aimed at: a drag has
        // just written it, and painting the earlier value leaves the handles a
        // whole frame behind the pointer. Every other bar here re-reads for the
        // same reason.
        let (centre, spread) = pair(self.gradient.sanitized());
        let (lo, hi) = (centre - spread.abs() * 0.5, centre + spread.abs() * 0.5);
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
        // The stretch of the axis the picture spends, which is what the pair
        // MEANS: a flat ramp fills nothing, and that is the honest drawing of a
        // gradient that spends none of this axis on pitch.
        let (lx, hx) = (x_of(lo), x_of(hi));
        let mut span = rect;
        span.min.x = lx;
        span.max.x = hx;
        painter.rect_filled(span, radius, fill_color);

        // Name and readout exactly as a ValueBar lays them out — the row is one
        // — with the same reserve trick: the width kept clear for the numbers
        // is measured off a string that never changes rather than off the pair
        // currently in the bar, so the name cannot re-elide mid-drag. See
        // [`Spread::widest_readout`] for what that string is.
        let text_color = if response.hovered() || response.dragged() {
            theme::text()
        } else {
            theme::text_dim()
        };
        let mono = TextStyle::Monospace.resolve(ui.style());
        let value = painter.layout_no_wrap(
            self.spread.readout((centre, spread)),
            mono.clone(),
            theme::text(),
        );
        let reserve = painter
            .layout_no_wrap(self.spread.widest_readout(), mono, theme::text())
            .size()
            .x;
        let body = TextStyle::Body.resolve(ui.style());
        let mut job = egui::text::LayoutJob::simple_singleline(
            self.spread.label().to_owned(),
            body,
            text_color,
        );
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

        // The handles on top of the text, a RangeBar's bargain: they are the
        // part you operate, and a digit sliding under one beats a handle
        // disappearing behind a digit. At a flat ramp the two coincide, and one
        // thumb standing on an empty track is the right picture — there is one
        // place the whole range is.
        let handle_w = HANDLE_W * scale;
        for x in [lx, hx] {
            painter.rect_filled(
                egui::Rect::from_center_size(
                    egui::pos2(x, rect.center().y),
                    Vec2::new(handle_w, rect.height() - 3.0 * scale),
                ),
                CornerRadius::same(theme::scaled_points(2, scale)),
                theme::text(),
            );
        }

        // The cursor says which gesture a press would start before committing
        // to it: a handle opens the ramp, the middle picks the whole thing up.
        match response.hover_pos().map(|p| SpreadGrab::at(value_at(p.x), (centre, spread), near)) {
            Some(SpreadGrab::Middle { .. }) => response.on_hover_cursor(egui::CursorIcon::Grab),
            Some(_) => response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal),
            None => response,
        }
    }
}

/// A band of `segments + 1` colored columns across `rect`, each column's color
/// taken from `color` at its position along the band (0 at the left edge, 1 at
/// the right) and interpolated between columns, with the band's ends rounded to
/// `radius`.
///
/// One builder for a [`SpectrumBar`]'s two bands — the track, whose color comes
/// off the hue circle and the pitch ramp either side of the handle, and the
/// pitch-order strip below it, which is the ramp end to end. A quad strip
/// written out twice is two places to get the vertex order or the first-column
/// case wrong, and the second copy is the one that quietly keeps the older
/// answer.
///
/// **The rounding is in the MESH, and that is the whole reason this is not a
/// square band inside a rounded well.** A well showing round an inset mesh is
/// the ordinary way to round colors that a quad strip cannot round itself, and
/// it costs a ring of the well's own color drawn around the content — a border
/// no other control in a settings pane wears, and at the shared
/// [`CONTROL_RADIUS`](theme::CONTROL_RADIUS) of 5 a one-point inset does not
/// even cover the arc, so the band's square corners poke out through it.
/// Pinching the columns to the corner circle instead lets the colors go edge to
/// edge and round exactly like the fill of a [`ValueBar`] beside them.
fn gradient_strip(
    painter: &egui::Painter,
    rect: egui::Rect,
    segments: usize,
    radius: f32,
    color: impl Fn(f32) -> egui::Color32,
) {
    let radius = radius.clamp(0.0, (rect.height() * 0.5).min(rect.width() * 0.5));
    let mut xs: Vec<f32> = (0..=segments)
        .map(|i| rect.left() + rect.width() * i as f32 / segments as f32)
        .collect();
    for k in 1..CORNER_SAMPLES {
        let t = radius * k as f32 / CORNER_SAMPLES as f32;
        xs.push(rect.left() + t);
        xs.push(rect.right() - t);
    }
    xs.sort_by(f32::total_cmp);

    let mut mesh = egui::Mesh::default();
    let mut drawn = f32::NEG_INFINITY;
    for x in xs {
        // Two samples landing on one column would build a triangle of no area,
        // which draws nothing and costs the same as one that does.
        if x - drawn < 0.01 {
            continue;
        }
        drawn = x;
        let inset = corner_inset((x - rect.left()).min(rect.right() - x), radius);
        let p = ((x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
        let c = color(p);
        let v = mesh.vertices.len() as u32;
        mesh.colored_vertex(egui::pos2(x, rect.top() + inset), c);
        mesh.colored_vertex(egui::pos2(x, rect.bottom() - inset), c);
        if v > 0 {
            mesh.add_triangle(v - 2, v - 1, v);
            mesh.add_triangle(v - 1, v + 1, v);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// How far a rounded band's edge is pinched in, top and bottom, at a column
/// `from_end` points from its nearer end: nothing along the straight run, and
/// the corner circle's own profile inside the last `radius`.
fn corner_inset(from_end: f32, radius: f32) -> f32 {
    if from_end >= radius {
        return 0.0;
    }
    let across = radius - from_end.max(0.0);
    radius - (radius * radius - across * across).max(0.0).sqrt()
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

    /// The name the painted bars carry, long enough to elide when the row is
    /// narrow and short enough to draw whole when it is not.
    const NAME: &str = "Pitch range";

    /// Paint one range bar across a `width`-point row and return what it
    /// emitted.
    fn paint_range_bar_wide(width: f32, low: f32, high: f32) -> Vec<egui::Shape> {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(width, 100.0));
        let (mut lo, mut hi) = (low, high);
        let out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                RangeBar::new(&mut lo, &mut hi, AXIS.0..=AXIS.1, NAME).min_span(OCTAVE).show(ui);
            },
        );
        out.shapes.into_iter().map(|s| s.shape).collect()
    }

    /// Paint one range bar across a 300pt row and return what it emitted.
    fn paint_range_bar(low: f32, high: f32) -> Vec<egui::Shape> {
        paint_range_bar_wide(300.0, low, high)
    }

    /// Paint one [`RangeBar::fade_span`] bar across a 300pt row. No `min_span`,
    /// as the bars that ask for this paint have none: their span closes for a
    /// hard edge.
    fn paint_fade_bar(low: f32, high: f32) -> Vec<egui::Shape> {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 100.0));
        let (mut lo, mut hi) = (low, high);
        let out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                RangeBar::new(&mut lo, &mut hi, AXIS.0..=AXIS.1, NAME).fade_span().show(ui);
            },
        );
        out.shapes.into_iter().map(|s| s.shape).collect()
    }

    /// The gradient fill's columns, left to right: where each one is and what
    /// color it carries. A column's two vertices share a color, so reading the
    /// first of each pair is the whole of it.
    fn fill_ramp(shapes: &[egui::Shape]) -> Vec<(f32, egui::Color32)> {
        let mut columns = Vec::new();
        for shape in shapes {
            if let egui::Shape::Mesh(mesh) = shape {
                for pair in mesh.vertices.chunks(2) {
                    columns.push((pair[0].pos.x, pair[0].color));
                }
            }
        }
        columns
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
                    let bar = RangeBar::new(lo, hi, AXIS.0..=AXIS.1, NAME).min_span(OCTAVE);
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

    /// The bar names itself on its own row: three text runs in a 20pt row,
    /// the name where a ValueBar puts one and each number beside the handle it
    /// belongs to. What this is worth is the row it saves: a control with no
    /// name of its own costs a label row above it, which is what a range with
    /// two of these bars a section would spend twice.
    #[test]
    fn a_range_bar_names_itself_on_the_bar() {
        // Mid-axis, so there is empty track either side of the span to sit in.
        let shapes = paint_range_bar(60.0, 72.0);
        let (texts, handles) = (text_boxes(&shapes), handles(&shapes));
        let bar = filled_rects(&shapes)[0].0;
        assert_eq!(texts.len(), 3, "a name and both ends, and nothing else: {texts:?}");
        assert_eq!((texts[0].1.as_str(), texts[1].1.as_str(), texts[2].1.as_str()), (
            NAME, "60.00", "72.00",
        ));
        assert!(texts[0].0.right() <= texts[1].0.left(), "a number ran into the name");
        assert!(texts[1].0.right() <= handles[0].left(), "low value sits outside its handle");
        assert!(texts[2].0.left() >= handles[1].right(), "high value sits outside its handle");
        for (t, _) in &texts {
            assert!(t.left() >= bar.left() && t.right() <= bar.right(), "text left the bar");
        }
    }

    /// At the full range there is no empty track left to write in, so each
    /// readout moves to the inner side of its handle rather than off the bar.
    #[test]
    fn the_readouts_move_inside_when_the_span_leaves_no_room() {
        let shapes = paint_range_bar(AXIS.0, AXIS.1);
        let (texts, handles) = (text_boxes(&shapes), handles(&shapes));
        let bar = filled_rects(&shapes)[0].0;
        assert!(texts[1].0.left() >= handles[0].right(), "low value moved inside the span");
        assert!(texts[2].0.right() <= handles[1].left(), "high value moved inside the span");
        for (t, _) in &texts {
            assert!(t.left() >= bar.left() && t.right() <= bar.right(), "readout left the bar");
        }
    }

    /// The name holds its exact box however the handles move, which is what
    /// lets three runs share the row: the numbers roam, so if the name roamed
    /// too there would be no arrangement of the two that never collides.
    ///
    /// It holds because the width kept clear for the numbers is measured end
    /// by end off the widest string the RANGE can produce rather than off the
    /// pair in the bar. Measured off the pair, the name would re-elide the
    /// moment an end gained a digit — wobbling under the pointer mid-drag,
    /// which is what the monospace face buys the digits themselves.
    ///
    /// Painted NARROW, and that is what gives the test its teeth. In a roomy
    /// row the name is never elided at all, so its galley comes out the same
    /// width whether the reserve was measured off the range's ends or off the
    /// pair in the bar, and the test passes under the very mutation it is
    /// written to catch. At 120pt the name is elided to a width the reserve
    /// decides, so measuring the pair instead moves it.
    #[test]
    fn the_name_holds_its_place_however_the_handles_move() {
        for width in [300.0f32, 120.0] {
            let name_of =
                |low, high| text_boxes(&paint_range_bar_wide(width, low, high))[0].clone();
            let name = name_of(AXIS.0, AXIS.1);
            // Spans of every width, at both ends of the axis and across the
            // middle, and a different number of digits in the numbers beside
            // them — "12.00" against "132.00" is the whole of what a
            // pair-measured reserve would see move.
            for (low, high) in
                [(60.0, 72.0), (AXIS.0, AXIS.0 + OCTAVE), (99.0, AXIS.1), (24.0, 108.0)]
            {
                assert_eq!(name_of(low, high), name, "{width}pt, {low}..{high} re-laid the name");
            }
        }
    }

    /// No number ever reaches the name, at any span and any column width. That
    /// is the arithmetic rather than luck: the name is laid out against a width
    /// with both readouts' worst case already subtracted, so what is left over
    /// always holds the two of them side by side (see [`RangeBar::show`]).
    ///
    /// Swept rather than sampled because the failure is positional — it would
    /// show up at one span placement and nowhere else — and a bar whose name is
    /// half-covered by its own readout says the wrong number as readily as the
    /// wrong name.
    #[test]
    fn the_numbers_never_reach_the_name() {
        for width in [680.0f32, 400.0, 240.0, 120.0] {
            for i in 0..=20 {
                for j in i..=20 {
                    let at = |k: i32| AXIS.0 + (AXIS.1 - AXIS.0) * k as f32 / 20.0;
                    let (low, high) = (at(i), at(j));
                    if high - low < OCTAVE {
                        continue;
                    }
                    let shapes = paint_range_bar_wide(width, low, high);
                    let texts = text_boxes(&shapes);
                    let bar = filled_rects(&shapes)[0].0;
                    assert!(
                        texts[0].0.right() <= texts[1].0.left(),
                        "{width}pt, {low}..{high}: the low number ran into the name",
                    );
                    assert!(
                        texts[1].0.right() <= texts[2].0.left(),
                        "{width}pt, {low}..{high}: the numbers ran into each other",
                    );
                    assert!(
                        texts[2].0.right() <= bar.right(),
                        "{width}pt, {low}..{high}: the high number left the bar",
                    );
                }
            }
        }
    }

    /// A thumb never stands in a number, at any span. That is the whole reason
    /// the two ends are not spelled into one run parked at the right the way
    /// [`SpreadBar`]'s are: the thumb is drawn in the same near-white as the
    /// digits, so a crossing swallows a character whichever of the two paints
    /// last, and "-60 dB" reading "-60 B" is the concrete thing this holds
    /// off. The Level bar's ceiling and the Band bar's outer radius both rest
    /// past four fifths of their axes, which is where a parked run is crossed.
    ///
    /// SWEPT, not sampled at the resting spans, and that is the point of it:
    /// sampled at the three placements the panes open at, this passed while a
    /// narrow span anywhere near either end of the axis put a thumb in a
    /// number at every width including the widest.
    ///
    /// Held down to 300pt. The settings column opens around 423pt in the
    /// reference window, and the real bars are clean well past this — swept
    /// with the pitch range's own `hz_readout`, the widest any pane asks for,
    /// there is not one crossing at 300pt or above. Under about 240 a span
    /// narrower than the two numbers it carries has no run of clear bar left
    /// that holds them, and something has to give.
    #[test]
    fn no_thumb_ever_stands_in_a_number_at_the_widths_the_column_opens_at() {
        for width in [680.0f32, 423.0, 400.0, 300.0] {
            for i in 0..=20 {
                for j in i..=20 {
                    let at = |k: i32| AXIS.0 + (AXIS.1 - AXIS.0) * k as f32 / 20.0;
                    let (low, high) = (at(i), at(j));
                    if high - low < OCTAVE {
                        continue;
                    }
                    let shapes = paint_range_bar_wide(width, low, high);
                    let texts = text_boxes(&shapes);
                    for h in handles(&shapes) {
                        for (t, what) in texts.iter().skip(1) {
                            assert!(
                                t.right() <= h.left() || t.left() >= h.right(),
                                "{width}pt, {low}..{high}: a thumb stands in {what:?}",
                            );
                        }
                    }
                }
            }
        }
    }

    /// No readout ever leaves the bar, at any span and any width — including
    /// the widths where nothing fits and the row is past being readable. Off
    /// the bar is off the pane, where horizontal scrolling is deliberately off
    /// (`panes::Viewer::scroll_bars`) and a number can be neither read nor
    /// dragged to, so this is the one promise that survives a hopeless row.
    ///
    /// Down to 90pt, which is where the promise stops being one that can be
    /// kept: a readout wider than the whole bar has to hang off it somewhere,
    /// and at 40pt these six-character numbers are 36pt against 30pt of room
    /// between the bar's own text insets.
    #[test]
    fn a_readout_never_leaves_the_bar_however_cramped_the_row() {
        for width in [680.0f32, 300.0, 160.0, 120.0, 90.0] {
            for i in 0..=12 {
                for j in i..=12 {
                    let at = |k: i32| AXIS.0 + (AXIS.1 - AXIS.0) * k as f32 / 12.0;
                    let (low, high) = (at(i), at(j));
                    if high - low < OCTAVE {
                        continue;
                    }
                    let shapes = paint_range_bar_wide(width, low, high);
                    let bar = filled_rects(&shapes)[0].0;
                    for (t, what) in text_boxes(&shapes).iter().skip(1) {
                        assert!(
                            t.left() >= bar.left() - 0.01 && t.right() <= bar.right() + 0.01,
                            "{width}pt, {low}..{high}: {what:?} at {t:?} left the bar {bar:?}",
                        );
                    }
                }
            }
        }
    }

    /// Too narrow a row elides the NAME and leaves the numbers whole: the
    /// numbers are what the bar is for, and a name that ran over one, or off
    /// the pane, would cost the reading the control exists to give.
    ///
    /// Read off the laid-out galley's WIDTH rather than its text, because a
    /// galley's text is the job's own string — the whole name, elided or not —
    /// and says nothing about what was drawn.
    ///
    /// 120pt because that is the narrowest column the panes are held to
    /// (`no_settings_pane_overruns_a_narrow_column`), so it is the width at
    /// which the eliding has to work rather than an arbitrary squeeze.
    #[test]
    fn a_narrow_row_elides_the_name_rather_than_the_numbers() {
        let narrow = paint_range_bar_wide(120.0, 60.0, 72.0);
        let roomy = paint_range_bar_wide(300.0, 60.0, 72.0);
        let name_width = |shapes: &[egui::Shape]| text_boxes(shapes)[0].0.width();
        let texts = text_boxes(&narrow);
        assert_eq!((texts[1].1.as_str(), texts[2].1.as_str()), ("60.00", "72.00"));
        assert!(
            name_width(&narrow) < name_width(&roomy),
            "the name did not elide: {:.1}pt of it in a 120pt row against {:.1}pt in a 300pt one",
            name_width(&narrow),
            name_width(&roomy),
        );
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

    /// A `fade_span` bar paints the pair as the edge it describes: one fill
    /// from the axis floor, solid as far as `low`, ramping out to the bare
    /// track by `high`.
    ///
    /// The default paint says the opposite of this — it fills exactly the part
    /// that is FADING and leaves the solid part bare — which on a pair that is
    /// a reach and its fade reads as a bright band floating off the node it is
    /// measured from.
    #[test]
    fn a_fade_span_fills_from_the_floor_and_ramps_out() {
        let (low, high) = (60.0, 100.0);
        let shapes = paint_fade_bar(low, high);
        let track = filled_rects(&shapes)[0].0;
        let x_of = |v: f32| {
            let inset = HANDLE_INSET;
            let inner = track.left() + inset;
            inner + (track.width() - 2.0 * inset) * (v - AXIS.0) / (AXIS.1 - AXIS.0)
        };
        let ramp = fill_ramp(&shapes);
        assert!(!ramp.is_empty(), "a fade span painted no fill");

        let (first, last) = (ramp[0], ramp[ramp.len() - 1]);
        assert!(
            (first.0 - x_of(AXIS.0)).abs() < 0.01,
            "the fill starts at {} rather than at the axis floor {}",
            first.0,
            x_of(AXIS.0),
        );
        assert!(
            (last.0 - x_of(high)).abs() < 0.01,
            "the fill ends at {} rather than at `high` ({})",
            last.0,
            x_of(high),
        );
        assert_eq!(last.1, theme::well(), "the fill does not reach the bare track by `high`");

        // Solid to `low`, and never brightening after it. Read as distance
        // from the well rather than as a color, so this asks the one thing
        // that matters — how much fill is left — of whatever the skin's accent
        // happens to be.
        let well = egui::Rgba::from(theme::well());
        let from_well = |c: egui::Color32| {
            let c = egui::Rgba::from(c);
            ((c.r() - well.r()).powi(2) + (c.g() - well.g()).powi(2) + (c.b() - well.b()).powi(2))
                .sqrt()
        };
        let solid = from_well(ramp[0].1);
        assert!(solid > 0.0, "the fill is the same color as the track it sits on");
        // Halfway along the ramp it is halfway out. This is what pins where
        // the fade STARTS, which is the one thing the paint exists to show and
        // the one thing every assertion around it leaves free: a ramp squeezed
        // into the last quarter is still solid before `low`, still monotone,
        // and still lands on the well at `high`.
        let middle = (x_of(low) + x_of(high)) * 0.5;
        let at_middle = ramp
            .iter()
            .min_by(|a, b| (a.0 - middle).abs().total_cmp(&(b.0 - middle).abs()))
            .expect("the ramp has columns")
            .1;
        assert!(
            (from_well(at_middle) - solid * 0.5).abs() < solid * 0.15,
            "halfway along the fade the fill is {:.0}% of solid, not about half",
            100.0 * from_well(at_middle) / solid,
        );
        let mut previous = f32::INFINITY;
        for (x, color) in ramp {
            if x <= x_of(low) + 0.01 {
                assert!(
                    (from_well(color) - solid).abs() < 0.01,
                    "the fill is already fading at {x}, before `low` ({})",
                    x_of(low),
                );
            }
            assert!(
                from_well(color) <= previous + 0.01,
                "the fill brightens again at {x}",
            );
            previous = from_well(color);
        }
    }

    /// An edge of no reach paints NOTHING. The fill stands on the value axis,
    /// which is the inset track the handles are placed on — not the bar's own
    /// rect, which starts a handle's half-width earlier and would leave a stub
    /// of fill under a control that is switched off, and overstate every reach
    /// above it by the same amount.
    #[test]
    fn an_edge_of_no_reach_paints_no_fill() {
        let shapes = paint_fade_bar(AXIS.0, AXIS.0);
        let width = match (fill_ramp(&shapes).first(), fill_ramp(&shapes).last()) {
            (Some(first), Some(last)) => last.0 - first.0,
            _ => 0.0,
        };
        assert!(width < 0.01, "an edge of no reach paints {width:.2}pt of fill");
    }

    /// A hard edge closes the span, and the fill is then solid the whole way
    /// with no ramp at all — which is the picture of a hard edge, and the one
    /// setting where this bar and the plain fill it replaces look the same.
    ///
    /// Not a NaN guard, though the shape invites reading it as one: a closed
    /// span puts the solid fraction at exactly 1, `gradient_strip` samples no
    /// further than 1, so the `p <= solid` arm takes every column and the
    /// divide is never reached. The `solid >= 1.0` beside it is belt and
    /// braces against float wobble, and deleting it leaves this passing.
    #[test]
    fn a_closed_fade_span_paints_a_solid_fill() {
        let shapes = paint_fade_bar(60.0, 60.0);
        let ramp = fill_ramp(&shapes);
        assert!(!ramp.is_empty(), "a closed fade span painted no fill");
        let first = ramp[0].1;
        for (x, color) in ramp {
            assert_eq!(color, first, "the fill is not solid at {x}");
        }
    }

    /// A CLOSED span is operable from both sides: below it the low end opens
    /// it, at or above it the whole span slides, keeping its width.
    ///
    /// The state is a hard edge on a [`RangeBar::fade_span`] bar — the reach
    /// and the fade that ends it, with no fade — which is an ordinary setting
    /// rather than a degenerate one, and the two gestures are the two a bar
    /// apiece would give: widen without softening (slide), and soften
    /// (the low end). Every measurement is a tie when the ends coincide, so
    /// without the rule the tie-break hands every press to `Low`, which is
    /// pinned against `hi` and moves nothing at all.
    #[test]
    fn a_closed_span_slides_from_above_and_opens_from_below() {
        let closed = (60.0, 60.0);
        // Above it: the span slides, so the edge widens at the same (zero)
        // fade rather than softening.
        let grab = Grab::at(80.0, closed, AXIS, 8.0);
        assert!(matches!(grab, Grab::Span { .. }), "a press above a closed span took {grab:?}");
        assert_eq!(grab.apply(90.0, closed, AXIS, 0.0), (70.0, 70.0));
        // Below it: the low end, which is the only one that can open it.
        let grab = Grab::at(40.0, closed, AXIS, 8.0);
        assert!(matches!(grab, Grab::Low), "a press below a closed span took {grab:?}");
        assert_eq!(grab.apply(40.0, closed, AXIS, 0.0), (40.0, 60.0));
    }

    /// A span that arrives ALREADY closed — or inverted — opens to the
    /// minimum on the first drag rather than sliding along shut.
    ///
    /// `min_span` bounds what the bar PRODUCES, not what it is handed, so
    /// every bar that declares one can still be given a pair that breaks it:
    /// the Nodes tab's colour range is two host params with nothing between
    /// them, and the Band bar's pair reaches `ViewConfig` from a blob
    /// unsanitized. The slide is what a closed span needs and it carries its
    /// width forward, so without the floor below it carries a zero — and the
    /// bar, whose whole job is to repair such a pair by being dragged, would
    /// hold it shut instead. `Grab::apply` promises the opposite in as many
    /// words: the span never closes past `min_span`.
    #[test]
    fn a_span_handed_in_closed_opens_to_the_minimum() {
        let cases: [((f32, f32), &str); 2] =
            [((60.0, 60.0), "a closed pair"), ((100.0, 40.0), "an inverted pair")];
        for (pair, hint) in cases {
            let press = pair.0.max(pair.1) + 5.0;
            let (lo, hi) = Grab::at(press, pair, AXIS, 8.0).apply(press + 10.0, pair, AXIS, OCTAVE);
            assert!(
                hi - lo >= OCTAVE - 1e-3,
                "{hint}: dragging it left a span of {} ({lo}..{hi})",
                hi - lo,
            );
        }
    }

    /// And a closed span at the axis FLOOR — a soft edge switched off
    /// altogether — can still be dragged back out. There is no room below it
    /// for the low end, so the slide is the whole of what the bar has left,
    /// and it is what turns the edge back on hard rather than fully faded.
    #[test]
    fn a_closed_span_at_the_floor_still_opens() {
        let off = (AXIS.0, AXIS.0);
        for v in [AXIS.0, 30.0, 60.0, 120.0] {
            let grab = Grab::at(v, off, AXIS, 8.0);
            let moved = grab.apply(v + 10.0, off, AXIS, 0.0);
            assert!(moved != off, "a press at {v} left the bar dead");
            assert_eq!(moved.1 - moved.0, 0.0, "a press at {v} softened the edge as it widened");
        }
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
        /// What the bar is told to reset to, or `None` to leave the builder
        /// alone — which is a caller naming no home, and a different code path
        /// from one naming the same gradient the default already is.
        home: Option<Gradient>,
    }

    impl Spectrum {
        /// Laid out once before anything is aimed at it: egui resolves the
        /// pointer against the PREVIOUS pass's rects, so a press cannot land on
        /// a bar that has never been drawn.
        fn settled(g: &mut Gradient) -> Spectrum {
            let ctx = egui::Context::default();
            crate::theme::apply_theme(&ctx);
            let mut h = Spectrum {
                ctx,
                screen: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 100.0)),
                rect: egui::Rect::NOTHING,
                live_at: egui::Pos2::ZERO,
                t: 0.0,
                home: None,
            };
            h.frame(g, vec![]);
            h
        }

        /// The same bar, told to reset somewhere other than the lattice's
        /// gradient — the Spectral pane's own case.
        fn settled_with_home(g: &mut Gradient, home: Gradient) -> Spectrum {
            let mut h = Spectrum::settled(g);
            h.home = Some(home);
            // Laid out again under the new builder, for the reason `settled`
            // lays out at all: egui resolves a press against the previous
            // pass's rects.
            h.frame(g, vec![]);
            h
        }

        fn frame(&mut self, g: &mut Gradient, events: Vec<egui::Event>) -> Vec<egui::Shape> {
            self.t += 1.0 / 60.0;
            let rect = std::cell::Cell::new(egui::Rect::NOTHING);
            let out = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(self.screen),
                    time: Some(self.t),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let bar = SpectrumBar::new(g);
                    let bar = match self.home {
                        Some(home) => bar.home(home),
                        None => bar,
                    };
                    rect.set(bar.show(ui).rect)
                },
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

        /// The middle of the flip button, in the gutter before the track's left
        /// edge.
        fn on_flip(&self) -> egui::Pos2 {
            egui::pos2(self.rect.left() - FLIP_W * 0.5, self.rect.center().y)
        }

        /// A point on the pitch-order strip, `across` of the way along it and
        /// `down` of the way through it — 0 at the edge nearest the track,
        /// which is the depth that matters. egui hands a press that hits
        /// nothing to the nearest widget within `interact_radius`, so the top
        /// of the strip is the part a gap alone does not protect, and a probe
        /// at the strip's middle sits clear of the only place the geometry can
        /// fail.
        fn on_strip(&self, across: f32, down: f32) -> egui::Pos2 {
            egui::pos2(
                self.rect.left() + self.rect.width() * across,
                self.rect.bottom() + PIECE_GAP + STRIP_H * down,
            )
        }

        /// Where a span of `span` degrees stands the handle.
        fn at_span(&self, span: f32) -> egui::Pos2 {
            let track = self.track();
            let across = (span / FULL_TURN).abs().clamp(0.0, 1.0);
            egui::pos2(track.left() + track.width() * across, track.center().y)
        }

        /// Press and release at one spot, answering what the frame carrying
        /// the release painted — which is the frame a click lands on.
        fn click(&mut self, g: &mut Gradient, at: egui::Pos2) -> Vec<egui::Shape> {
            self.frame(g, vec![egui::Event::PointerMoved(at)]);
            self.frame(g, vec![press(at, true)]);
            self.frame(g, vec![press(at, false)])
        }

        /// Press at `from` and drag to `to`, answering what the arriving frame
        /// painted. A step clear of egui's drag threshold comes first, since
        /// a gesture that jumps straight to its target settles its grab there.
        fn drag(
            &mut self,
            g: &mut Gradient,
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
        fn double_click(&mut self, g: &mut Gradient, at: egui::Pos2) {
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

    /// The colored bands a spectrum bar paints — the track, then the
    /// pitch-order strip. Everything else it draws is a rect, a line or a
    /// convex polygon, so a mesh is a band and nothing else is.
    fn bands(shapes: &[egui::Shape]) -> Vec<egui::Mesh> {
        shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Mesh(m) => Some((**m).clone()),
                _ => None,
            })
            .collect()
    }

    /// Both bands are rounded by their own mesh, on the corner circle, and
    /// sampled through the arc rather than chamfered across it.
    ///
    /// Nothing else in the suite looks at a mesh, so without this the entire
    /// rounding mechanism — [`corner_inset`], [`CORNER_SAMPLES`], the radius
    /// handed to [`gradient_strip`] — could be deleted and every test would
    /// stay green. What it holds is the reason the bands are drawn edge to edge
    /// at all: a square mesh inside a rounded well needs a ring of well showing
    /// round it to look rounded, and that ring is a border no other bar in a
    /// settings pane wears.
    ///
    /// Three claims, because they fail apart. Pinning the corner vertices to
    /// the arc catches a chamfer and an inset that has stopped following the
    /// circle, but not a corner drawn from its two endpoints alone — the
    /// endpoints are ON the arc. The sample count catches that. And the
    /// straight run catches a radius that has grown to swallow the band.
    #[test]
    fn both_colour_bands_are_rounded_by_their_own_mesh() {
        let mut g = ViewConfig::default().pitch_gradient;
        let mut h = Spectrum::settled(&mut g);
        let shapes = h.frame(&mut g, vec![]);
        let bands = bands(&shapes);
        assert_eq!(bands.len(), 2, "a spectrum bar paints two bands, not {}", bands.len());
        let radius = f32::from(bar_radius(1.0));
        for (which, mesh) in ["track", "strip"].into_iter().zip(&bands) {
            // The strip is 11pt against a radius of 5, so its ends are all but
            // semicircular and its straight run is a point tall. That is a
            // shape, not a limit — what the radius may not do is eat the band's
            // LENGTH, which the straight-run count below is what catches.
            let box_ = mesh.calc_bounds();
            let (mut near, mut far, mut full_height) = (0, 0, 0);
            // Two vertices per column, top then bottom, left to right.
            for column in mesh.vertices.chunks(2) {
                let (top, bottom) = (column[0].pos, column[1].pos);
                assert!((top.x - bottom.x).abs() < 1e-3, "{which}: a column is not vertical");
                let from_end = (top.x - box_.left()).min(box_.right() - top.x);
                if from_end >= radius - 1e-3 {
                    full_height += 1;
                    assert!(
                        (top.y - box_.top()).abs() < 1e-3
                            && (bottom.y - box_.bottom()).abs() < 1e-3,
                        "{which}: a column {from_end} from the end, past the corner, is pinched",
                    );
                    continue;
                }
                let cx = if top.x - box_.left() < radius {
                    near += 1;
                    box_.left() + radius
                } else {
                    far += 1;
                    box_.right() - radius
                };
                for (y, cy) in
                    [(top.y, box_.top() + radius), (bottom.y, box_.bottom() - radius)]
                {
                    let reach = ((top.x - cx).powi(2) + (y - cy).powi(2)).sqrt();
                    assert!(
                        (reach - radius).abs() < 0.05,
                        "{which}: a corner vertex sits {reach} from the arc's centre, not {radius}",
                    );
                }
            }
            // Each corner counted on its own, against a flat four rather than
            // against anything derived from [`CORNER_SAMPLES`]. A floor read
            // off the constant it is meant to pin goes to zero with it and
            // passes on a chamfer; four is the claim itself — fewer than four
            // columns through a quarter turn reads as steps at any radius this
            // control uses.
            for (end, count) in [("near", near), ("far", far)] {
                assert!(
                    count >= 4,
                    "{which}: {count} columns through the {end} corner — a chamfer, not an arc",
                );
            }
            assert!(full_height > 0, "{which}: the radius swallowed the whole band");
        }
    }

    /// A column with no room for both gives the row to the flip button, and
    /// gives it the row rather than a sliver of one.
    ///
    /// The branch is a deliberate choice — `spectrum_track_width` floors at
    /// zero, so past that point the track stops shrinking and the button stops
    /// moving — and no sweep reaches it: `no_settings_pane_overruns_a_narrow_column`
    /// bottoms out at 120pt and this needs 20. Which leaves the arithmetic
    /// under it unexercised, and it is the arithmetic most likely to go
    /// negative: a track laid out from the RIGHT edge inward.
    #[test]
    fn a_column_too_narrow_for_both_gives_the_row_to_the_button() {
        for column in [FLIP_W + PIECE_GAP + 30.0, FLIP_W + PIECE_GAP, FLIP_W, 6.0, 1.0] {
            let ctx = egui::Context::default();
            crate::theme::apply_theme(&ctx);
            let mut g = ViewConfig::default().pitch_gradient;
            let seen = std::cell::Cell::new(egui::Rect::NOTHING);
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(column, 80.0),
                    )),
                    ..Default::default()
                },
                |ui| seen.set(SpectrumBar::new(&mut g).show(ui).rect),
            );
            let track = seen.get();
            let shapes: Vec<egui::Shape> = out.shapes.into_iter().map(|s| s.shape).collect();
            // The button is the one thing painted in the theme's resting widget
            // fill; the well under the track is `well()` and the handle is
            // `text()`. Finding none would mean the paint no longer reads that
            // fill, which is its own failure.
            let buttons: Vec<egui::Rect> = filled_rects(&shapes)
                .into_iter()
                .filter(|(_, fill)| *fill == theme::widget())
                .map(|(r, _)| r)
                .collect();
            assert_eq!(buttons.len(), 1, "at {column}pt the bar drew {buttons:?} buttons");
            let button = buttons[0];

            assert!(track.width() >= 0.0, "at {column}pt the track came out {track:?}");
            assert!(
                button.right() <= track.left() + 0.01,
                "at {column}pt the button {button:?} runs into the track {track:?}",
            );
            // Below the threshold the track is gone and the button holds the
            // row: everything except the gap it would have kept clear.
            if column <= FLIP_W + PIECE_GAP {
                assert_eq!(track.width(), 0.0, "at {column}pt the track kept {}", track.width());
                assert!(
                    button.width() >= track.right() - button.left() - PIECE_GAP - 0.01,
                    "at {column}pt the button shrank to {} of a {}pt row",
                    button.width(),
                    track.right() - button.left(),
                );
            } else {
                assert!(track.width() > 0.0, "at {column}pt the track vanished early");
                assert!(
                    (button.width() - FLIP_W).abs() < 0.01,
                    "at {column}pt the button is {}, not its full {FLIP_W}",
                    button.width(),
                );
            }
        }
    }

    /// Where the bar drew its handle.
    fn spectrum_handle_x(shapes: &[egui::Shape]) -> f32 {
        let hs = handles(shapes);
        assert_eq!(hs.len(), 1, "a spectrum bar draws one handle, not {hs:?}");
        hs[0].center().x
    }

    /// The hue the bar paints at `p`, worked out the way the bar lays a circle
    /// on a track: cut at the arc's own start, one whole turn across.
    fn hue_under(g: Gradient, track: egui::Rect, p: egui::Pos2) -> f32 {
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
            Gradient::MAX_HUE_SPAN,
            "the track draws one turn while the span reaches {}, so the handle \
             parks at the right edge with the value still growing",
            Gradient::MAX_HUE_SPAN,
        );
    }

    /// The handle stands at the fraction of the turn the arc claims, and says
    /// so in the same picture.
    #[test]
    fn the_spectrum_handle_stands_where_its_readout_says() {
        for span in [0.0f32, 45.0, 190.0, 360.0, -190.0, -360.0] {
            let mut g = Gradient { hue_span: span, ..Gradient::default() };
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
        let mut nothing = Gradient { hue_span: 0.0, ..Gradient::default() };
        let mut h = Spectrum::settled(&mut nothing);
        let shapes = h.frame(&mut nothing, vec![]);
        assert!(handles(&shapes)[0].left() >= h.rect.left(), "a zero span hangs the handle off");
        let mut whole = Gradient { hue_span: 360.0, ..Gradient::default() };
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
        let mut g = Gradient { hue_span: 90.0, ..Gradient::default() };
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
            Gradient { hue_start: 260.0, hue_span: 190.0, ..Gradient::default() };
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
    /// They share a row, and what keeps them apart is that each is interacted
    /// with over its OWN rectangle — which works HERE, where the strip below
    /// needs a position check as well, because the button senses clicks and not
    /// drags: a press on it produces no drag hit for the track to inherit, at
    /// any distance. Sensed together, a press that began on the button reaches
    /// the track's rotate branch the moment egui calls it a drag, and the
    /// circle spins under a pointer that pressed a button.
    #[test]
    fn a_drag_begun_on_the_flip_button_turns_nothing() {
        let before =
            Gradient { hue_start: 0.0, hue_span: 90.0, ..Gradient::default() };
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

    /// The pitch-order strip is a picture, and a press anywhere on it —
    /// including hard against the track above — starts nothing.
    ///
    /// Swept from the strip's top edge down, because a gap is not by itself a
    /// barrier: egui's hit test collects every widget within
    /// `interact_radius` of the pointer and, when the press hits nothing
    /// directly, hands it to the nearest one. At the default radius of 5 that
    /// reaches five points past the track's bottom edge — over twice
    /// [`PIECE_GAP`] — so the strip's own top is inside the track's reach and
    /// only the position check in `show` keeps it out. A probe at the strip's
    /// middle alone would sit clear of the one region where this can fail.
    #[test]
    fn the_pitch_strip_is_a_picture_and_not_a_control() {
        // The arc the track's own reset lands on, so the control halves below
        // read the reset rather than a constant that merely used to match it
        // (see `a_double_click_on_the_spectrum_goes_home_to_the_arc_a_fresh_view_opens_on`).
        let home = default_home();
        let dialled = Gradient { hue_start: 12.0, hue_span: 33.0, ..home };

        for down in [0.0f32, 0.05, 0.25, 0.5, 1.0] {
            // A drag begun on the strip and run up into the track.
            let mut g = home;
            let mut h = Spectrum::settled(&mut g);
            let from = h.on_strip(0.2, down);
            let to = egui::pos2(h.on_strip(0.8, down).x, h.track().center().y);
            h.drag(&mut g, from, to);
            assert_eq!(g, home, "a drag begun {down} down the pitch strip turned the circle");

            // The same gesture one row higher does move it, so the harness is
            // delivering something the widget can act on.
            let mut g = home;
            let mut h = Spectrum::settled(&mut g);
            let from = egui::pos2(from.x, h.track().center().y);
            h.drag(&mut g, from, to);
            assert_ne!(g, home, "the same drag on the track moved nothing, so this proves nothing");

            // And a double-click on the strip does not reset the arc. A fresh
            // bar for each pair: run back to back on one, the second lands
            // inside the first's double-click window and arrives as the third
            // click of a sequence, which is a different gesture.
            let mut g = dialled;
            let mut h = Spectrum::settled(&mut g);
            let at = h.on_strip(0.5, down);
            h.double_click(&mut g, at);
            assert_eq!(g, dialled, "a double-click {down} down the pitch strip reset the arc");
        }
    }

    /// The arc a double-click goes home to is the one a fresh view OPENS on,
    /// which is not the gradient type's own default.
    ///
    /// The same argument [`reset_wheel`] is written out for, one control over:
    /// a reset that names its own value drifts the moment the fresh look
    /// moves, and does it silently, because nothing reads out the pair it
    /// resets to. `ViewConfig::default` composes its gradient rather than
    /// taking `Gradient::default()` — a shorter arc over a shallower
    /// brightness ramp — and says at the field that it is free to differ.
    /// A reset that lands on the type's default therefore puts the bar
    /// somewhere the plugin has never opened, and the bar has no text entry
    /// to dial it back with.
    #[test]
    fn a_double_click_on_the_spectrum_goes_home_to_the_arc_a_fresh_view_opens_on() {
        let fresh = ViewConfig::default().pitch_gradient;
        let dialled = Gradient { hue_start: 12.0, hue_span: 33.0, ..fresh };

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

    /// A bar handed a home of its own resets THERE, which is what lets one set
    /// of bars serve two gradients.
    ///
    /// The Spectral pane's heatmap is the second, and its default arc is
    /// nothing like the lattice's — so a reset that ignored the builder would
    /// land a heatmap on the lattice's violet-to-yellow sweep and leave the
    /// shipped ramp unreachable by gesture, which is the same loss
    /// [`default_home`] exists to prevent one pane over.
    ///
    /// Both halves are asserted: the arc that WAS reached, and that it is not
    /// the default one. Without the second, a bar that quietly ignored `home`
    /// would still pass whenever the two happened to agree.
    #[test]
    fn a_bar_over_another_gradient_resets_to_the_one_it_was_handed() {
        let home = crate::SpectrumConfig::default().spectrogram_gradient;
        let lattice = default_home();
        assert_ne!(
            (home.hue_start, home.hue_span),
            (lattice.hue_start, lattice.hue_span),
            "the two homes agree, so this test cannot tell whether `home` was read",
        );

        let mut g = Gradient { hue_start: 12.0, hue_span: 33.0, ..home };
        let mut h = Spectrum::settled_with_home(&mut g, home);
        let at = h.track().center();
        h.double_click(&mut g, at);
        assert_eq!(
            (g.hue_start, g.hue_span),
            (home.hue_start, home.hue_span),
            "the reset ignored the home it was handed",
        );

        // And the pairs the two spread bars carry, which reset the same way.
        for spread in [Spread::Brightness, Spread::Chroma] {
            let dialled = Gradient { hue_start: 12.0, hue_span: 33.0, ..Gradient::default() };
            assert_ne!(
                spread.of(dialled),
                spread.of(home.sanitized()),
                "{spread:?}: the bar already holds the pair it would reset to",
            );
            assert_eq!(
                double_click_spread(spread, dialled, Some(home)),
                spread.of(home.sanitized()),
                "{spread:?}: the reset ignored the home it was handed",
            );
        }
    }

    /// A turn slides the circle under a fixed left edge, and the hue the
    /// gesture took hold of stays under the pointer for the length of it.
    #[test]
    fn turning_the_spectrum_keeps_the_grabbed_hue_under_the_pointer() {
        let mut g = Gradient { hue_start: 0.0, hue_span: 90.0, ..Gradient::default() };
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
                Gradient { hue_start: start, hue_span: span, ..Gradient::default() }
                    .sanitized();
            let after = before.flipped();
            let ends = |g: Gradient| (g.lightness_and_hue(0.0).1, g.lightness_and_hue(1.0).1);
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
        let flipped = Gradient { hue_span: 0.0, ..Gradient::default() }.flipped();
        assert!(
            flipped.hue_span.is_sign_positive(),
            "flipping a span of nothing left it at {}",
            flipped.hue_span,
        );

        let mut g = Gradient { hue_span: -120.0, ..Gradient::default() };
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

    /// A reach in value units wide enough to reach a handle from well away
    /// from it, standing in for the `GRAB_PX` a real bar converts.
    const NEAR: f32 = 4.0;

    /// A dragged end goes where the pointer is and leaves its partner exactly
    /// where it stood. Which is the whole of a two-ended bar, and it is what
    /// makes the readout's two numbers each settable on their own.
    #[test]
    fn a_dragged_end_moves_itself_and_leaves_its_partner() {
        // A 40-point ramp about 50 stands its ends at 30 and 70.
        let ramp = (50.0f32, 40.0);
        assert_eq!(
            SpreadGrab::High.apply(90.0, ramp, L_STAR_AXIS),
            (60.0, 60.0),
            "the high end to 90 leaves 30..90, which is a middle of 60 and a ramp of 60",
        );
        assert_eq!(
            SpreadGrab::Low.apply(10.0, ramp, L_STAR_AXIS),
            (40.0, 60.0),
            "and the low end to 10 leaves 10..70",
        );
    }

    /// Past its PARTNER an end inverts the ramp rather than stopping against
    /// it, which is the whole of how the bright end reaches the bottom of the
    /// pitch range. The gesture keeps hold of the end it grabbed, so the two
    /// trade sides and the sign follows the pointer through zero without a
    /// discontinuity.
    ///
    /// A [`RangeBar`] refuses exactly this — see
    /// `a_dragged_end_stops_at_the_minimum_span` — and is right to: its ends
    /// bound a pitch axis, which inverted maps every pitch on it backwards.
    #[test]
    fn an_end_dragged_past_its_partner_inverts_the_ramp() {
        // The low end walking up through its partner at 70.
        let walk: Vec<(f32, f32)> = [50.0, 70.0, 90.0]
            .into_iter()
            .map(|v| SpreadGrab::Low.apply(v, (50.0, 40.0), L_STAR_AXIS))
            .collect();
        assert_eq!(
            walk,
            vec![(60.0, 20.0), (70.0, 0.0), (80.0, -20.0)],
            "the low end crossing its partner went {walk:?}",
        );
    }

    /// An end stops at the axis, and nowhere short of it: black and white are
    /// both settings, and the ramp that reaches from one to the other is the
    /// widest the bar has.
    #[test]
    fn a_dragged_end_stops_at_the_axis() {
        let ramp = (50.0f32, 40.0);
        assert_eq!(SpreadGrab::High.apply(500.0, ramp, L_STAR_AXIS), (65.0, 70.0), "at white");
        assert_eq!(SpreadGrab::Low.apply(-500.0, ramp, L_STAR_AXIS), (35.0, 70.0), "and at black");
        // Both ends out: the whole axis, which is the steepest ramp there is.
        let full = SpreadGrab::Low.apply(-500.0, (50.0, 100.0), L_STAR_AXIS);
        assert_eq!(full, (50.0, 100.0), "black to white is the widest ramp the axis holds");
    }

    /// Sliding carries the ramp along at the width the gesture began with, so
    /// making the picture brighter is one gesture and does not quietly restyle
    /// how much brightness the pitch range spends.
    #[test]
    fn a_slid_ramp_keeps_its_grabbed_width() {
        let grab = SpreadGrab::Middle { offset: 4.0, spread: 40.0 };
        assert_eq!(grab.apply(64.0, (50.0, 40.0), L_STAR_AXIS), (60.0, 40.0));
        // And a negative ramp stays negative: its SIGN is not the slide's to
        // change, and a slide that flipped the picture would be a surprise
        // nothing on the bar announced.
        let inverted = SpreadGrab::Middle { offset: 4.0, spread: -40.0 };
        assert_eq!(inverted.apply(64.0, (50.0, -40.0), L_STAR_AXIS), (60.0, -40.0));
    }

    /// Slid into an end the ramp squishes rather than the drag jamming: the
    /// leading end pins to the wall and the trailing one carries on with the
    /// pointer, down to nothing. And it springs back out on the way home,
    /// because it reads the width its own gesture began at and never the
    /// squished pair it just wrote — a [`Grab::Span`]'s bargain, both halves.
    #[test]
    fn a_slid_ramp_squishes_against_the_end_it_meets() {
        // Grabbed dead centre of a 40-point ramp at L* 50, so 30..70.
        let grab = SpreadGrab::Middle { offset: 0.0, spread: 40.0 };
        let start = (50.0f32, 40.0);
        assert_eq!(
            grab.apply(90.0, start, L_STAR_AXIS),
            (85.0, 30.0),
            "the bright end pins at white and the dark one carries on to 70",
        );
        assert_eq!(grab.apply(120.0, start, L_STAR_AXIS), (100.0, 0.0), "squishing to nothing");
        // Already squished, same pointer: the answer must not creep further.
        assert_eq!(grab.apply(90.0, (85.0, 30.0), L_STAR_AXIS), (85.0, 30.0));
        // And back down the axis, the ramp the gesture started with returns.
        assert_eq!(grab.apply(50.0, (85.0, 30.0), L_STAR_AXIS), start);
    }

    /// At a FLAT ramp all three grabs stand on the same point, and the middle
    /// is the one that has to win: it is the only thing left to drag, and a
    /// bar whose brightness could not be moved at exactly the isoluminant
    /// setting would strand anyone who dialled their way into it.
    ///
    /// Away from that point the ends take over, and WHICH end is the pointer's
    /// own side — see `a_flat_ramp_opens_the_way_it_is_dragged` for what that
    /// buys, which is a picture the right way round in either direction.
    #[test]
    fn the_middle_stays_grabbable_at_a_flat_ramp() {
        let flat = (50.0f32, 0.0);
        assert!(matches!(SpreadGrab::at(50.0, flat, NEAR), SpreadGrab::Middle { .. }));
        let out = SpreadGrab::at(20.0, flat, NEAR);
        assert!(matches!(out, SpreadGrab::Low), "a press out on the track took {out:?}");
        assert_eq!(
            out.apply(20.0, flat, L_STAR_AXIS),
            (35.0, 30.0),
            "and it opens the ramp dark-at-the-bottom",
        );
    }

    /// Opening a ramp out of a flat one runs the way it is DRAGGED, either
    /// direction: up lifts the top of the pitch range, down darkens the bottom,
    /// and both leave the picture the right way round.
    ///
    /// A flat ramp is the one setting where nothing distinguishes the two ends
    /// — they stand on the same point — so which one a press takes is a rule
    /// rather than a measurement, and taking a FIXED one inverts the picture in
    /// whichever direction it is not. At black or white that direction is the
    /// only one there is: the axis runs one way from either, so a bar that
    /// opened inverted on an up-drag would make an isoluminant black picture
    /// impossible to open the right way round at all.
    #[test]
    fn a_flat_ramp_opens_the_way_it_is_dragged() {
        for (flat, to, want) in [
            ((50.0f32, 0.0f32), 70.0f32, (60.0f32, 20.0f32)),
            ((50.0, 0.0), 30.0, (40.0, 20.0)),
            // Parked on black, where up is the only way out.
            ((0.0, 0.0), 40.0, (20.0, 40.0)),
            // And on white.
            ((100.0, 0.0), 60.0, (80.0, 40.0)),
        ] {
            let grab = SpreadGrab::at(to, flat, NEAR);
            let got = grab.apply(to, flat, L_STAR_AXIS);
            assert_eq!(
                got, want,
                "flat at {} dragged to {to} gave a ramp of {}, and a negative one is the \
                 picture upside down",
                flat.0, got.1,
            );
        }
    }

    /// A wide ramp divides the bar the way a [`RangeBar`] does: a handle's
    /// reach around each end, the whole inside between them, and the empty
    /// track beyond falling to the nearer end. What that buys is that aiming at
    /// a handle cannot land on the slide, which would move both ends instead of
    /// the one aimed at.
    #[test]
    fn a_wide_ramp_leaves_both_the_handles_and_the_slide_reachable() {
        let wide = (50.0f32, 80.0);
        assert!(matches!(SpreadGrab::at(10.0, wide, NEAR), SpreadGrab::Low), "on the low handle");
        assert!(matches!(SpreadGrab::at(90.0, wide, NEAR), SpreadGrab::High), "the high handle");
        assert!(matches!(SpreadGrab::at(50.0, wide, NEAR), SpreadGrab::Middle { .. }), "inside");
        assert!(matches!(SpreadGrab::at(2.0, wide, NEAR), SpreadGrab::Low), "off the end");
        // The reach is NEAR itself here, an 80-point ramp being far too wide
        // for the share to bite: 6 points inside the low end is a slide, 3 is
        // the handle.
        assert!(matches!(SpreadGrab::at(16.0, wide, NEAR), SpreadGrab::Middle { .. }));
        assert!(matches!(SpreadGrab::at(13.0, wide, NEAR), SpreadGrab::Low));
        // And inverted, where the low-pitch end stands on the RIGHT: the same
        // press takes the same pitch end, not the same side of the bar.
        let flipped = (50.0f32, -80.0);
        assert!(matches!(SpreadGrab::at(90.0, flipped, NEAR), SpreadGrab::Low), "still the low");
        assert!(matches!(SpreadGrab::at(10.0, flipped, NEAR), SpreadGrab::High));
    }

    /// Every pair the bar writes puts both ENDS on a whole readout unit inside
    /// the axis, since the ends are what it reads out and a readout is worth
    /// nothing once it is not the number the picture draws.
    #[test]
    fn the_pair_a_bar_writes_puts_both_ends_on_whole_readout_units() {
        let brightness = Spread::Brightness;
        // 43.4..83.8 rounds to 43..84, whose middle is a half.
        assert_eq!(brightness.snapped((63.6, 40.4)), (63.5, 41.0));
        // An odd ramp is exactly what snapping the PAIR could not keep honest:
        // 45 about 64 reaches 41.5 and 86.5, which round to 42 and 87 — the
        // ramp survives at 45 and the middle takes the half instead, which is
        // the right way round, since the middle is not what anyone reads.
        assert_eq!(brightness.snapped((64.0, 45.0)), (64.5, 45.0));
        // Past white, the bright end pins there and the ramp is what is left.
        assert_eq!(brightness.snapped((90.0, 40.0)), (85.0, 30.0));
        // A whole readout unit on the chroma axis is a hundredth of it, which
        // is the same statement about the same picture: the ends are read out
        // as percentages, so those are what land whole. To a tenth of a unit,
        // the resolution the readout itself claims — a hundredth is no binary
        // fraction, and `the_bar_can_only_reach_pairs_sanitize_leaves_alone`
        // covers what that costs the pair.
        for spread in [Spread::Brightness, Spread::Chroma] {
            let unit = spread.per_unit();
            let (min, max) = spread.axis();
            for centre in [0.0f32, 0.135, 0.49, 0.636, 0.896, 1.0].map(|v| min + v * (max - min)) {
                for spread_v in [0.0f32, 0.01, -0.07, 0.45, 0.999, -1.0, 4.0]
                    .map(|v| v * (max - min))
                {
                    let (c, s) = spread.snapped((centre, spread_v));
                    for end in [c - s * 0.5, c + s * 0.5] {
                        let units = end * unit;
                        // A thousandth of a unit, which is a tolerance on the
                        // RECOMPOSITION and not on the snap: the pair is written
                        // as a middle and a ramp, so reading the ends back off
                        // it costs an ulp of the fraction — worst measured at
                        // 7.6e-6 of a percent, over every whole-percent pair of
                        // ends — where a snap that had actually missed the grid
                        // would miss by half a unit, five orders the other side
                        // of this.
                        assert!(
                            (units - units.round()).abs() < 1e-3,
                            "{spread:?}: {centre}/{spread_v} lands an end on {units} units",
                        );
                        assert!(
                            (min..=max).contains(&end),
                            "{spread:?}: {centre}/{spread_v} puts an end off the axis at {end}",
                        );
                    }
                }
            }
        }
    }

    /// Every pair a gesture can settle on, put through the write path the bar
    /// uses and then to `sanitized`.
    ///
    /// The sweep runs in FRACTIONS of the axis so one set of positions means the
    /// same thing on both, since the two axes are two orders of magnitude apart.
    fn pairs_a_bar_can_write(spread: Spread, mut check: impl FnMut((f32, f32))) -> usize {
        let (min, max) = spread.axis();
        let of = |v: f32| min + v * (max - min);
        let mut checked = 0;
        for centre in [0.0f32, 0.01, 0.125, 0.5, 0.636, 0.896, 0.99, 1.0].map(of) {
            for width in [0.0f32, 0.07, -0.33, 1.0, -1.0].map(|v| v * (max - min)) {
                // Every grab the bar can settle on, against pointer positions
                // running the whole axis and a good way off both ends of it.
                // `value_at` clamps, so the widget itself never hands `apply` a
                // value off the axis; the extra range is aimed at `apply`'s OWN
                // clamps, which are what a pointer dragged past the bar's end
                // meets once that stops being true.
                let held = 0.135 * (max - min);
                let grabs = [
                    SpreadGrab::Low,
                    SpreadGrab::High,
                    SpreadGrab::Middle { offset: 0.0, spread: width },
                    SpreadGrab::Middle { offset: held, spread: width },
                    SpreadGrab::Middle { offset: -held, spread: width },
                ];
                for grab in grabs {
                    for step in -20..=120 {
                        check(grab.apply(of(step as f32 / 100.0), (centre, width), (min, max)));
                        checked += 1;
                    }
                }
            }
        }
        checked
    }

    /// What the bar can reach is exactly what `sanitized` leaves alone. The two
    /// say the same thing in different places — the bar because a handle off
    /// the track is not a value it can express, the gradient because a pair out
    /// of a hand-edited file never came through a bar — and nothing but this
    /// stops them drifting into disagreeing about which pairs are legal. A bar
    /// that could write a pair sanitize pulls in would draw one picture and
    /// hold another.
    #[test]
    fn the_bar_can_only_reach_pairs_sanitize_leaves_alone() {
        for spread in [Spread::Brightness, Spread::Chroma] {
            let checked = pairs_a_bar_can_write(spread, |aimed| {
                let (c, s) = spread.legal(spread.snapped(aimed));
                let mut written = Gradient::default();
                spread.set(&mut written, (c, s));
                assert_eq!(
                    written.sanitized(),
                    written,
                    "{spread:?}: the bar wrote a middle of {c} and a ramp of {s}, which \
                     sanitize does not accept as it stands",
                );
            });
            assert!(checked > 10_000, "only {checked} pairs — the sweep stopped covering it");
        }
    }

    /// And [`Spread::legal`] is load-bearing on the chroma axis rather than
    /// belt-and-braces: snapping alone reaches pairs sanitize pulls in.
    ///
    /// Both steps say both ends are on the axis — snapping by clamping the ends
    /// it rounds, the gradient by bounding the ramp against what its middle
    /// leaves — and the two are the same statement only in exact arithmetic. A
    /// hundredth is no binary fraction, so a chroma pair recomposed from whole
    /// percentages can land a ramp one ulp past the bound while whole `L*`
    /// never does. Nothing about the picture turns on 6e-8 of chroma; what turns
    /// on it is whether the number the bar reads out is the number the gradient
    /// holds.
    #[test]
    fn snapping_alone_would_leave_a_chroma_pair_sanitize_pulls_in() {
        let over = |spread: Spread| {
            let mut over = 0;
            pairs_a_bar_can_write(spread, |aimed| {
                let (c, s) = spread.snapped(aimed);
                if spread.legal((c, s)) != (c, s) {
                    over += 1;
                }
            });
            over
        };
        assert_eq!(over(Spread::Brightness), 0, "whole L* recomposes exactly, so this is a no-op");
        assert!(
            over(Spread::Chroma) > 0,
            "no snapped chroma pair needs pulling in, so `legal` is now untested here \
             and the sweep has stopped reaching the ends of the axis",
        );
    }

    /// Where a double-click lands has to BE the pair a fresh view opens with,
    /// for the reason the wheel's reset does: the bar carries no text entry, so
    /// a reset that missed would leave the shipped look unreachable by gesture.
    ///
    /// The bar a caller names NO home for is the one under test, that being the
    /// case a caller gets wrong by omission — a bar handed a home of its own
    /// resets to what it was handed, and
    /// [`a_bar_over_another_gradient_resets_to_the_one_it_was_handed`] is where
    /// that half is held.
    ///
    /// Through the gesture rather than by comparing `default_home()` to the
    /// expression `default_home()` is defined as, which is a tautology that
    /// passes however the widget behaves. What has to be true is that a
    /// double-click on a bar built WITHOUT `.home(..)` lands on the fresh view's
    /// pair — three separate things (the default, the builder, and the reset
    /// branch reading it), only one of which a pure comparison touches.
    #[test]
    fn a_double_click_goes_home_to_the_pair_a_fresh_view_opens_with() {
        let fresh = ViewConfig::default().pitch_gradient;
        for spread in [Spread::Brightness, Spread::Chroma] {
            let dialled = holding(spread, spread.snapped((30.0 / spread.per_unit(), 0.0)));
            assert_ne!(
                spread.of(dialled),
                spread.of(fresh.sanitized()),
                "{spread:?}: the bar already holds the pair it would reset to",
            );
            assert_eq!(
                double_click_spread(spread, dialled, None),
                spread.of(fresh.sanitized()),
                "{spread:?}: the reset landed on a pair no fresh view opens with",
            );
            assert_ne!(
                spread.of(fresh),
                spread.of(Gradient::default()),
                "{spread:?}: the type's own default and the composed one agree today, so \
                 this reset cannot tell whether it is reading the one the plugin actually \
                 opens on",
            );
        }
    }

    /// One gradient carrying this pair on this spread and its own defaults
    /// everywhere else.
    fn holding(spread: Spread, pair: (f32, f32)) -> Gradient {
        let mut g = Gradient::default();
        spread.set(&mut g, pair);
        g
    }

    /// One bar of `spread`, built through the constructor that names it — which
    /// is the only place the two differ to a caller.
    fn spread_bar(spread: Spread, g: &mut Gradient, ui: &mut Ui) -> Response {
        match spread {
            Spread::Brightness => SpreadBar::brightness(g).show(ui),
            Spread::Chroma => SpreadBar::chroma(g).show(ui),
        }
    }

    /// Paint one bar across a 300pt row and return what it emitted.
    fn paint_bar(spread: Spread, pair: (f32, f32)) -> Vec<egui::Shape> {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 100.0));
        let mut g = holding(spread, pair);
        let out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                spread_bar(spread, &mut g, ui);
            },
        );
        out.shapes.into_iter().map(|s| s.shape).collect()
    }

    /// The bar draws the pair it holds: a handle at each end of the ramp, at its
    /// own place along the whole axis. That is the whole claim the control
    /// makes, and handles standing anywhere else would be a picture of some
    /// other pair.
    ///
    /// In FRACTIONS of the axis, which is what makes it one test of two bars:
    /// the geometry is the same picture whether the ends are `L*` 42 and 86 or
    /// 42% and 86% of the color available.
    ///
    /// The inverted case is here because it is the one the picture CANNOT tell
    /// apart: a ramp and its negative put the two handles in exactly the same
    /// places, which is why the readout runs in pitch order instead.
    #[test]
    fn a_bar_stands_its_handles_where_its_numbers_say() {
        for spread in [Spread::Brightness, Spread::Chroma] {
            let (min, max) = spread.axis();
            let of = |v: f32| min + v * (max - min);
            for sign in [1.0f32, -1.0] {
                let ramp = sign * 0.44 * (max - min);
                let shapes = paint_bar(spread, (of(0.64), ramp));
                let bar = filled_rects(&shapes)[0].0;
                let hs = handles(&shapes);
                assert_eq!(hs.len(), 2, "{spread:?} at a ramp of {ramp} drew {} handles", hs.len());
                // The track a handle travels: the bar less the inset at either
                // end.
                let track = bar.shrink2(Vec2::new(HANDLE_INSET, 0.0));
                for (want, h) in [(0.64 - 0.22, hs[0]), (0.64 + 0.22, hs[1])] {
                    let at = track.left() + track.width() * want;
                    assert!(
                        (h.center().x - at).abs() < 0.5,
                        "{spread:?} at a ramp of {ramp} puts an end {want} of the way up the \
                         axis, which is {at} across, and the handle stands at {}",
                        h.center().x,
                    );
                }
            }
            // A flat ramp is one handle's worth of picture in the middle of an
            // empty track: none of the axis is spent on pitch, and there is
            // exactly one place the whole range is.
            let hs = handles(&paint_bar(spread, (of(0.3), 0.0)));
            assert_eq!(hs[0], hs[1], "{spread:?} drew a flat ramp's two handles apart");
        }
    }

    /// Two handles and nothing else standing on the track. A third mark on a
    /// two-ended bar reads as a third handle whatever it is drawn like, and the
    /// middle — the one thing that might have earned one — is not something a
    /// gesture takes hold of.
    #[test]
    fn a_bar_stands_nothing_on_the_track_but_its_two_ends() {
        for spread in [Spread::Brightness, Spread::Chroma] {
            let (min, max) = spread.axis();
            let of = |v: f32| min + v * (max - min);
            let width = 0.44 * (max - min);
            for pair in [(of(0.64), width), (of(0.64), -width), (of(0.3), 0.0)] {
                let hs = handles(&paint_bar(spread, pair));
                assert_eq!(hs.len(), 2, "{spread:?} {pair:?} put {} marks on the track", hs.len());
            }
        }
    }

    /// The numbers one bar reads out, each parsed off the end of the readout
    /// with whatever unit follows it stripped.
    fn readout_ends(spread: Spread, pair: (f32, f32)) -> (String, Vec<f32>) {
        let shown = text_boxes(&paint_bar(spread, pair))
            .into_iter()
            .map(|(_, s)| s)
            .next_back()
            .expect("the bar draws a readout");
        let said = shown
            .split('\u{2192}')
            .map(|s| {
                s.trim()
                    .trim_end_matches(spread.suffix())
                    .parse()
                    .expect("a readout is two numbers")
            })
            .collect();
        (shown, said)
    }

    /// The readout names the `L*` the curve actually draws at both ends of the
    /// pitch range, at every pair the bar can be handed — not only at the ones
    /// a drag leaves behind.
    ///
    /// A drag snaps both ends to whole `L*`, so a bar that has been touched
    /// reads out exactly whatever it does. Everything else arrives unsnapped:
    /// the pair a fresh view opens on, the one a double-click goes home to, and
    /// anything a saved blob or a hand-edited file carries. `ViewConfig`'s own
    /// gradient is 53 over a ramp of 31, whose ends are 37.5 and 68.5 — the
    /// case `snapped` is written to keep a DRAG off, arriving by the one road
    /// that does not pass it.
    ///
    /// A tenth of a point, because that is well under anything a viewer could
    /// see and well over the half-point a whole-number readout costs at these
    /// ends: the failure is a bar reading `38 → 68`, a span of 30, over a
    /// gradient spending 31.
    #[test]
    fn a_brightness_readout_names_the_ends_the_curve_draws() {
        let fresh = ViewConfig::default().pitch_gradient;
        for pair in [
            (fresh.lightness, fresh.lightness_ramp),
            (64.0, 44.0),
            (64.0, -45.0),
            (20.0, 7.0),
        ] {
            let g = holding(Spread::Brightness, pair);
            let (shown, said) = readout_ends(Spread::Brightness, pair);
            // In PITCH order, which is what the readout claims to be in: the
            // curve at t 0 and t 1, not the darker end and the brighter one.
            for (t, said) in [0.0, 1.0].into_iter().zip(said) {
                let drawn = g.lightness_and_hue(t).0 as f32;
                assert!(
                    (said - drawn).abs() < 0.1,
                    "{pair:?} reads out {shown:?}, saying L* {said} where the curve draws {drawn}",
                );
            }
        }
    }

    /// The same claim on the chroma axis, where the readout is a PERCENTAGE of
    /// the curve's own fraction, so the two are a hundred apart and the
    /// arithmetic between them is the thing that can be wrong.
    ///
    /// A tenth of a percent, which is the resolution the readout claims — the
    /// pairs below include the one a fresh view opens with, which arrives
    /// without passing `snapped` and is not whole in percent either.
    #[test]
    fn a_chroma_readout_names_the_ends_the_curve_draws() {
        let fresh = ViewConfig::default().pitch_gradient;
        for pair in [(fresh.chroma, fresh.chroma_ramp), (0.5, 0.6), (0.5, -0.6), (0.2, 0.35)] {
            let g = holding(Spread::Chroma, pair);
            let (shown, said) = readout_ends(Spread::Chroma, pair);
            for (t, said) in [0.0, 1.0].into_iter().zip(said) {
                let drawn = g.chroma_at(t) as f32 * 100.0;
                assert!(
                    (said - drawn).abs() < 0.1,
                    "{pair:?} reads out {shown:?}, saying {said}% where the curve asks for \
                     {drawn}%",
                );
            }
        }
    }

    /// The two ends, in pitch order — the numbers the picture concretely draws,
    /// each standing under its own handle, and each carrying the unit its own
    /// axis is read in. Their ORDER is the sign: neither bar can show which end
    /// of the pitch range carries the most, since the handles stand in the same
    /// two places either way.
    #[test]
    fn a_bar_reads_out_its_two_ends_in_pitch_order() {
        let texts = |spread, pair| -> Vec<String> {
            text_boxes(&paint_bar(spread, pair)).into_iter().map(|(_, s)| s).collect()
        };
        let up = texts(Spread::Brightness, (64.0, 44.0));
        assert_eq!(up.len(), 2, "a name and one readout, not {up:?}");
        assert_eq!(up[0], "Brightness");
        assert_eq!(up[1], "42 \u{2192} 86", "the bottom of the pitch range reads first");
        assert_eq!(
            texts(Spread::Brightness, (64.0, -44.0))[1],
            "86 \u{2192} 42",
            "an inverted ramp draws the same two handles, so the readout is what says so",
        );
        let color = texts(Spread::Chroma, (0.64, 0.44));
        assert_eq!(color[0], "Chroma");
        assert_eq!(color[1], "42% \u{2192} 86%", "a share of the color reads out as one");
        assert_eq!(texts(Spread::Chroma, (0.64, -0.44))[1], "86% \u{2192} 42%");
    }

    /// Double-click one spread bar and answer the pair it wrote. `home` is what
    /// the bar is told to reset to, or `None` to leave the builder alone — which
    /// is a caller naming no home, and a different path from one naming the same
    /// gradient the default already is.
    ///
    /// Through a real context for the reason [`drag_bar`] is: the reset is a
    /// branch on a `Response`, and nothing synthetic reaches it.
    fn double_click_spread(
        spread: Spread,
        start: Gradient,
        home: Option<Gradient>,
    ) -> (f32, f32) {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 100.0));
        let mut g = start;
        let bar = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut t = 0.0;
        let mut frame = |g: &mut Gradient, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let b = match spread {
                        Spread::Brightness => SpreadBar::brightness(g),
                        Spread::Chroma => SpreadBar::chroma(g),
                    };
                    let b = match home {
                        Some(home) => b.home(home),
                        None => b,
                    };
                    bar.set(b.show(ui).rect)
                },
            );
        };
        frame(&mut g, vec![]);
        let at = bar.get().center();
        frame(&mut g, vec![egui::Event::PointerMoved(at)]);
        for _ in 0..2 {
            frame(&mut g, vec![press(at, true)]);
            frame(&mut g, vec![press(at, false)]);
        }
        spread.of(g)
    }

    /// Drag one bar across a 300pt row, from `from` to `to` as fractions of its
    /// width, and answer the pair it wrote. A real gesture through a real
    /// context, for the reason the range bar's is: what a gesture has hold of is
    /// decided on the first frame egui calls the press a drag and then
    /// remembered in context data, so a synthetic call exercises neither the
    /// decision nor the memory.
    fn drag_bar(spread: Spread, pair: (f32, f32), (from, to): (f32, f32)) -> (f32, f32) {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 100.0));
        let mut g = holding(spread, pair);
        let bar = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut t = 0.0;
        let mut frame = |g: &mut Gradient, events: Vec<egui::Event>| {
            t += 1.0 / 60.0;
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(t),
                    events,
                    ..Default::default()
                },
                |ui| bar.set(spread_bar(spread, g, ui).rect),
            );
        };
        // A frame with no input first: egui resolves the pointer against the
        // previous pass's rects, so a press cannot land on a bar that has never
        // been drawn.
        frame(&mut g, vec![]);
        let rect = bar.get();
        let at = |x: f32| egui::pos2(rect.left() + rect.width() * x, rect.center().y);
        frame(&mut g, vec![egui::Event::PointerMoved(at(from))]);
        frame(&mut g, vec![egui::Event::PointerMoved(at(from)), press(at(from), true)]);
        // A step clear of egui's drag threshold first, then the rest of the
        // way: the grab is settled on the first LIVE frame, which a gesture
        // that jumps straight to its target would settle at the target.
        let step = 12.0 / rect.width() * (to - from).signum();
        frame(&mut g, vec![egui::Event::PointerMoved(at(from + step))]);
        frame(&mut g, vec![egui::Event::PointerMoved(at(to))]);
        spread.of(g)
    }

    /// The two ends a pair draws, which is what the bar is really about and
    /// what its readout says.
    fn ends((centre, spread): (f32, f32)) -> (f32, f32) {
        (centre - spread * 0.5, centre + spread * 0.5)
    }

    /// The wiring, once, through a real pointer: a press on a handle moves that
    /// end and leaves its partner standing, and a press between them slides
    /// both without restyling the ramp. Every end lands on a whole `L*`, which
    /// is what the readout claims of it.
    #[test]
    fn a_real_drag_on_a_brightness_bar_keeps_the_gesture_it_started() {
        // The default pair sits at 64 with a 44-point ramp, so its handles are
        // at L* 42 and 86 — a press at 0.86 of the way across is the bright
        // one, dragged out to the top of the axis.
        let dragged = ends(drag_bar(Spread::Brightness, (64.0, 44.0), (0.86, 1.0)));
        assert_eq!(dragged, (42.0, 100.0), "the low end moved, or the high one stopped short");

        // And a slide, from between the handles at 30 and 70: brighter by a
        // quarter of the axis, carrying its ramp.
        let pair = drag_bar(Spread::Brightness, (50.0, 40.0), (0.5, 0.75));
        assert!(pair.0 > 60.0, "the slide barely moved, landing at {}", pair.0);
        assert_eq!(pair.1, 40.0, "the slide restyled the ramp to {}", pair.1);
        let (low, high) = ends(pair);
        assert_eq!((low, high), (low.round(), high.round()), "{low}..{high} is not whole");
    }

    /// The same wiring on the chroma bar, which is where the units the widget
    /// works in are actually at stake: the gesture arrives in pixels, the axis
    /// is a fraction two orders smaller than the `L*` one, and the readout is a
    /// percentage of it. A drag has to land on a whole PERCENT and leave a pair
    /// the gradient accepts unchanged, which is what dragging an end all the way
    /// out to the vivid end of the axis asks for.
    #[test]
    fn a_real_drag_on_a_chroma_bar_lands_on_whole_percentages() {
        // A 44% ramp about 64% stands its ends at 42% and 86%: the same picture
        // as the brightness case above, one axis over.
        let (low, high) = ends(drag_bar(Spread::Chroma, (0.64, 0.44), (0.86, 1.0)));
        for (end, want) in [(low, 42.0f32), (high, 100.0)] {
            assert!(
                (end * 100.0 - want).abs() < 0.05,
                "an end landed on {}% where {want}% is what the bar can say",
                end * 100.0,
            );
        }
        // And what the gradient makes of it: the pair is one it holds as it
        // stands, ends and all — the whole point of `Spread::legal` sitting on
        // the write path.
        let pair = drag_bar(Spread::Chroma, (0.64, 0.44), (0.86, 1.0));
        let written = holding(Spread::Chroma, pair);
        assert_eq!(written.sanitized(), written, "the drag wrote a pair sanitize pulls in");
    }
}
