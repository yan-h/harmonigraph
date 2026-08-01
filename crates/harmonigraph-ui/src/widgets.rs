//! Custom controls. `ValueBar` is the workhorse: a flat, DAW-style
//! parameter bar (drag anywhere to set, double-click to type a value)
//! that replaces egui's rail-and-knob `Slider` + separate `DragValue`.
//! `RangeBar` is its two-handle sibling, for a pair of values that bound a
//! span rather than one value on a scale.

use std::ops::RangeInclusive;

use egui::{CornerRadius, Key, Response, Sense, TextEdit, TextStyle, Ui, Vec2};

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
    /// ahead of the bar's name. Used by the major-third bar while meantone
    /// mode derives the third from the fifth: the number in the bar is then
    /// not the one the param holds, and the badge is what says so.
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
    display: fn(f32) -> String,
}

impl<'a> RangeBar<'a> {
    pub fn new(low: &'a mut f32, high: &'a mut f32, range: RangeInclusive<f32>) -> Self {
        RangeBar { low, high, range, min_span: 0.0, display: |v| format!("{v:.2}") }
    }

    pub fn min_span(mut self, span: f32) -> Self {
        self.min_span = span;
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
pub fn choice_row<T: Copy + PartialEq>(
    ui: &mut Ui,
    name: &str,
    value: &mut T,
    options: &[(T, &str, &str)],
) {
    button_row(ui, |ui| {
        ui.label(name);
        for (option, label, hint) in options {
            let response = ui.selectable_value(value, *option, option_label(label));
            if !hint.is_empty() {
                response.on_hover_text(*hint);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
