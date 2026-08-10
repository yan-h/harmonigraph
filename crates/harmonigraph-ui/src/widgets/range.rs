//! [`RangeBar`]: one control for the pair of values that bound a span, and the
//! [`Grab`] that decides which part of it a drag has hold of.

use std::ops::RangeInclusive;

use egui::{CornerRadius, Response, Sense, TextStyle, Ui, Vec2};

use super::bar::{
    aimed_at, bar_radius, bar_width, grip_over_text, BAR_LABEL_GAP, BAR_TEXT_PAD, GRAB_PX,
    HANDLE_INSET, HANDLE_REACH_SHARE, HANDLE_W, TEXT_GAP,
};
use super::mesh::gradient_strip;
use crate::theme;

/// Segments the ramp half of a [`fade_span`](RangeBar::fade_span) fill is drawn
/// in. The ramp is a part of a bar that is a couple of hundred points wide at
/// most, so this is already finer than the pixels it lands on — and it is a
/// whole-strip count rather than a per-point one because the strip is sampled
/// over its own width, which the span does not fix.
const FADE_SEGMENTS: usize = 64;

/// Which part of a [`RangeBar`] a drag took hold of. Decided once, at
/// drag-start, and remembered for the gesture — otherwise dragging one end
/// past the other would hand the drag to whichever handle is nearest now.
/// (`Default` is derived only to satisfy egui's `remove_temp` bound; the
/// value is always written by drag-start before anything reads it.)
#[derive(Clone, Copy, Debug, Default)]
pub(super) enum Grab {
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
    ///
    /// **The mirror of that has no fallback, and the cost is a band that is
    /// inert one way.** An end held against `min_span` cannot move inward, so a
    /// press inside its reach dragged that way writes nothing — the whole
    /// `near` band, either side of both handles, on a pair already at its
    /// minimum. The pitch range parks exactly there, since that is where
    /// zooming the analyzer all the way in leaves it, and `HANDLE_REACH_SHARE`
    /// is what keeps the middle grabbable beside it (at the 24-semitone minimum
    /// on a 423pt bar: 14 points claimed at each end out of 83).
    ///
    /// No fallback because the reach is claimed before the DIRECTION is known,
    /// and the end is only pinned one way: the same press dragged outward opens
    /// the span, which is the gesture nothing else offers. Handing the band to
    /// the span instead would cost that to buy the other, and reading `v`'s
    /// direction here is reading a position, not a travel. What is left is a
    /// handle that answers in one direction and holds in the other, which is
    /// what a handle against a wall should do — and what the cursor says it is,
    /// since the band draws `ResizeHorizontal`.
    ///
    /// The alternative is [`apply`](Self::apply) letting a pinned end carry its
    /// partner, the way the span branch squishes against a wall. That is a
    /// change to what `Low` and `High` MEAN rather than to this rule, and it is
    /// not made here.
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
/// bar's outer radius both sit at rest. A number goes in a run of CLEAR bar
/// instead, which is what keeps a thumb's own width between it and every thumb;
/// swept with the pitch range's `hz_readout`, the widest readout any pane asks
/// for, no thumb stands in a number at 300pt or above, and the settings column
/// opens around 423.
///
/// A crossed run CAN be made readable — [`grip_over_text`] knocks one out
/// through the thumb, which is what a [`SpreadBar`]'s parked readout leans on —
/// but this bar's two numbers do not get that treatment and a thumb standing in
/// one still swallows a digit. Only the name is knocked out here. Placement is
/// the better answer where it is available: a digit on flat track is a plainer
/// thing to read than one inverted inside a 6pt grip, and the sweep that keeps
/// the numbers off the thumbs would lose its teeth if they had a knockout to
/// fail into. Below the width that placement holds to, a crossed digit is what
/// this bar ships — see the paragraph below.
///
/// Under about 240pt that stops being reachable — a span narrower than the two
/// numbers it carries has no run of clear bar left that holds them — and what
/// the placement spends the remaining room on is reading ORDER, low then high
/// and both still on the bar. Order is what makes them a range rather than two
/// numbers.
///
/// **The NAME is crossed by the low handle** where the numbers are not, and the
/// difference is that the name cannot be placed: it is pinned to the left of
/// the bar so the row reads as a row, while a number is free to take whichever
/// run of clear track is going. A thumb roams the whole track, so the one fixed
/// run is the one it eventually stands in. The name's own share of the bar is
/// about a sixth of the axis at the width the settings column opens at, a
/// tenth on a bar twice that wide.
///
/// Most bars only reach it while the low end is DRAGGED there: the two that
/// open at the full axis stand their low handle a point clear of the name, and
/// the Level and Band bars open at 40% and 66% of theirs. The two
/// [`fade_span`](RangeBar::fade_span) bars rest inside it, and the Gutter does
/// so at a fresh install — its low end is where the gutter stops being solid,
/// which on a nearly-fully-soft default is 1.4% of the axis, so the thumb
/// stands on the "G".
///
/// **That costs no letter**, and it is why the fresh look does not have to be
/// chosen around it. The name is painted a second time clipped to the thumb, in
/// the panel colour, so its letters cross the grip in reverse rather than
/// disappearing under it ([`grip_over_text`]). Nothing moves and the thumb
/// keeps its full width; the "G" changes colour for as long as the handle
/// stands on it. A look picked to keep a handle off a letter would be the
/// picture paying for the panel, and this is what buys it back.
///
/// Letting the name slide out of the way instead was measured and dropped: it
/// has to snap back the moment the handle passes it, and a name jumping the
/// width of itself mid-drag reads worse than a letter that merely inverts.
///
/// [`ValueBar`]: super::value::ValueBar
/// [`SpreadBar`]: super::gradient::SpreadBar
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
    ///
    /// [`ValueBar::integer`]: super::value::ValueBar::integer
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
        let (rect, mut response) = ui.allocate_exact_size(
            Vec2::new(width, theme::row_height(scale)),
            Sense::click_and_drag(),
        );
        let (min, max) = (*self.range.start(), *self.range.end());
        // A [`fade_span`](RangeBar::fade_span) bar's values run the whole bar,
        // the way the octave strip's wheel does: its low end IS the bar's left
        // end — a gutter of no softness at all, a note standing off nothing —
        // and a handle stopping a point clear of it reads as a control that
        // cannot reach its own floor. The handles are still drawn whole, by
        // clamping where they are PLACED rather than where they mean, which is
        // the same bargain the strip makes.
        //
        // Every other range bar keeps the inset. They open at the FULL axis,
        // where flush handles at both ends sit under the corner rounding on a
        // bare track and read as the bar's own border — the affordance
        // HANDLE_INSET exists for, and which a bar opening on a fill does not
        // need.
        let inset = if self.fade_span { 0.0 } else { HANDLE_INSET * scale };
        let track = rect.shrink2(Vec2::new(inset, 0.0));
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
                        // From where the press LANDED (see `aimed_at`), snapped
                        // the same way the live value is: the span grab reads
                        // its own offset off this, so an unsnapped one would
                        // leave a fraction of a value inside a gesture whose
                        // whole point is whole ones.
                        let aim = value_at(aimed_at(ui, p).x);
                        let aim = if self.integer { aim.round() } else { aim };
                        let grab = Grab::at(aim, (*self.low, *self.high), (min, max), near);
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
        let r = bar_radius(scale);
        let corner = f32::from(r);
        let radius = CornerRadius::same(r);
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
            // One fill from the bar's left edge out to `high`, solid as far as
            // `low` and ramping to the well over the rest — the picture of the
            // edge this pair sets. Mixed toward the WELL rather than painted
            // with alpha, so the ramp ends on exactly the color the bare track
            // beyond it already is and the fill has no seam at its own end.
            //
            // Its floor is the bar's own left edge, which is where this bar's
            // axis floors too, so it is the same fill as the ValueBars it sits
            // among. An inset axis — what every other range bar puts its values
            // on — leaves a sliver of bare well at the far left, HANDLE_INSET
            // of it, and that sliver reads as a notch cut out of the control
            // rather than as reach the edge has not got to, because no other
            // bar in the pane has one.
            //
            // A reach of zero is then the one thing the fill cannot say for
            // itself, and the guard below is what says it: the solid head is
            // widened past its own end so that its corner survives (see below)
            // and the clip is what takes it back, so at a reach of zero there
            // is nothing left to cut it back TO and a control switched off
            // keeps a hair of its own feathered edge.
            //
            // **The two parts are drawn by different machinery, and the split
            // is what puts the rounded corner in the right hands.** Everything
            // left of `low` is one flat color, so it is a `rect_filled` like
            // any other bar's fill, and its corner comes out of the same call
            // the well's own does — one shape, one radius, nothing to keep in
            // step. The ramp cannot be a rect: a rect is a single color, which
            // is what makes a mesh the only way to carry a gradient at all, and
            // a mesh rounding this corner instead would put two roundings on
            // the one arc of the one fill.
            //
            // So the mesh is handed the stretch that leaves out the bar's own
            // left corner — the one end of this fill that stands in full color
            // against the panel. Its far end rounds itself, on the well's own
            // radius, and both of its ends land in the color they meet: the
            // head's at `low` and the well's at `high`, so neither is a visible
            // edge at all.
            if *self.high > min {
                let mut fill = rect;
                fill.max.x = hx;
                // Where the solid head gives way to the ramp — at `low`, held
                // off both ends of the fill.
                //
                // It never begins INSIDE the corner, because the corner can
                // only be drawn by a rounded rect and a rounded rect is one
                // color. A fade dragged fully open puts `low` at the bar's own
                // end, so without that the mesh would take the corner back —
                // square-ended, poking a whole radius out through the well's
                // arc, or stepped if it rounded itself. What it costs is the
                // first few points of the fade painted solid: five points of a
                // ramp a hundred long is a few percent of one, and a ramp short
                // enough for it to be more than that is a reach of a few
                // points, under two handles standing on each other.
                //
                // And it never begins past the fill's own end, which is what a
                // pair arriving crossed (`low` above `high`, which a host param
                // or a blob can still say) would otherwise ask for.
                let ramp_start = lx.max(fill.left() + corner).min(hx);
                // Held to the reach, so the head below can be widened past it
                // without ever drawing reach the edge does not have. Only the
                // far end is cut — the sides are pushed out of the way so the
                // fill's own antialiasing is not shaved with them.
                let mut clip = rect.expand(2.0);
                clip.max.x = hx;
                let painter = painter.with_clip_rect(clip);
                let mut head = fill;
                // Never narrow enough for its own corner to be clamped. epaint
                // holds a corner radius to half the rect's shortest side, so a
                // head cut to fit — one radius, five points, is where a fade
                // dragged fully open leaves it — would round at two and a half
                // points where the well rounds at five, and poke out through
                // the well's arc. That is the bar changing shape as a handle
                // reaches the end of its travel.
                //
                // Twice the radius is what leaves the corner alone, and it
                // costs nothing to ask for: the ramp is drawn over the excess,
                // and past the reach the clip takes it.
                head.max.x = ramp_start.max(fill.left() + 2.0 * corner);
                // Square where the ramp continues it, round where the bar is:
                // a rounded right end here would cut a notch out of a fill that
                // does not stop there.
                painter.rect_filled(
                    head,
                    CornerRadius { nw: r, sw: r, ne: 0, se: 0 },
                    fill_color,
                );
                let mut ramp = fill;
                ramp.min.x = ramp_start;
                // A hard edge closes the span, which leaves the head the whole
                // fill and the ramp nothing — an ordinary setting, and the one
                // case that has no gradient to draw.
                if ramp.width() > 0.0 {
                    let from = egui::Rgba::from(fill_color);
                    let to = egui::Rgba::from(theme::well());
                    gradient_strip(&painter, ramp, FADE_SEGMENTS, (0.0, corner), |p| {
                        egui::lerp(from..=to, p).into()
                    });
                }
            }
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
        let label_pos = centered(&label, rect.left() + text_pad);
        painter.galley(label_pos, label.clone(), text_color);

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
        // Where the thumbs are DRAWN, as against what they mean. A fade bar's
        // axis runs to the bar's own ends, so a value at either limit puts a
        // thumb's center on the edge with half its width out over the pane:
        // held in by half a handle it stands flush instead, whole and inside,
        // which is how the octave strip places its own. A no-op on a bar whose
        // track is already inset, where no value reaches this far.
        //
        // The readouts step around these rather than around `lx`/`hx`, since
        // what a number must not be crossed by is the thumb that is there.
        //
        // `max`/`min` rather than `clamp`, for the reason `contain` below names:
        // a row with less width than one handle inverts the pair, and `clamp`
        // asserts `min <= max` and takes the editor down with it. `bar_width`
        // floors at zero, so a column squeezed past its own controls reaches
        // that.
        let held = |x: f32| {
            let floor = rect.left() + half_handle;
            x.max(floor).min((rect.right() - half_handle).max(floor))
        };
        let (lgx, hgx) = (held(lx), held(hx));
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
            clipped((region_left, lgx - reach)),
            clipped((lgx + reach, hgx - reach)),
            clipped((hgx + reach, region_right)),
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
                && [lgx, hgx]
                    .iter()
                    .all(|&x| x + half_handle <= left || x - half_handle >= left + w)
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
        //
        // A thumb that can stand in the bar's own corner is rounded like the
        // bar rather than like a grip, so it follows the arc it is standing in
        // as closely as six points of width can — epaint holds a corner to half
        // that, which is near enough to keep the thumb inside the well. A bar
        // whose track is inset never gets there, and keeps the grip's own.
        //
        // Only the NAME is knocked out through the thumb. The two numbers are
        // PLACED clear of every thumb instead, which is the better answer where
        // it is available — a number on flat track beats an inverted one on a
        // grip — and `no_thumb_ever_stands_in_a_number_at_the_widths_the_column_
        // opens_at` is the sweep that holds that placement honest. Knocking the
        // numbers out too would give the placement somewhere soft to fail into
        // and cost that check its teeth. The name has no such option: it is
        // pinned to the left of the bar and a thumb comes to rest on it — which
        // for the two `fade_span` bars and a fresh Gutter is where they OPEN.
        let grip_radius =
            CornerRadius::same(if self.fade_span { r } else { theme::scaled_points(2, scale) });
        for x in [lgx, hgx] {
            grip_over_text(
                painter,
                egui::Rect::from_center_size(
                    egui::pos2(x, rect.center().y),
                    Vec2::new(handle_w, rect.height() - 3.0 * scale),
                ),
                grip_radius,
                &[(label_pos, label.clone())],
            );
        }

        // The cursor says which of the two gestures a press would start, so the
        // difference is visible BEFORE committing to a drag: an end resizes,
        // the middle picks the whole range up and slides it.
        let would_start = response
            .hover_pos()
            .map(|p| Grab::at(value_at(p.x), (*self.low, *self.high), (min, max), near));
        match would_start {
            Some(Grab::Span { .. }) => response.on_hover_cursor(egui::CursorIcon::Grab),
            Some(_) => response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal),
            None => response,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::mesh::{band_columns, bands, fades_out_at_its_edges};
    use crate::widgets::probe::{filled_rects, handles, knockouts, text_boxes};

    /// The analyzer's axis, the range bar's real caller.
    const AXIS: (f32, f32) = (12.0, 132.0);

    const OCTAVE: f32 = 12.0;

    /// The name the painted bars carry, long enough to elide when the row is
    /// narrow and short enough to draw whole when it is not.
    const NAME: &str = "Pitch range";

    /// Paint one range bar across a `width`-point row and return what it
    /// emitted, each shape still carrying the clip rect it was painted through.
    ///
    /// The clip is what almost every test here can throw away and the knockout
    /// pass cannot: that pass repeats the name's own galley at the name's own
    /// origin, so string, box and position are all identical to the run it
    /// doubles, and the clip rect is the only thing that says it is confined to
    /// a thumb rather than painted over the whole name.
    fn paint_range_bar_clipped(
        width: f32,
        low: f32,
        high: f32,
    ) -> Vec<egui::epaint::ClippedShape> {
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
        out.shapes
    }

    /// Paint one range bar across a `width`-point row and return what it
    /// emitted.
    fn paint_range_bar_wide(width: f32, low: f32, high: f32) -> Vec<egui::Shape> {
        paint_range_bar_clipped(width, low, high).into_iter().map(|s| s.shape).collect()
    }

    /// Paint one range bar across a 300pt row and return what it emitted.
    fn paint_range_bar(low: f32, high: f32) -> Vec<egui::Shape> {
        paint_range_bar_wide(300.0, low, high)
    }

    /// Paint one [`RangeBar::fade_span`] bar across a 300pt row, each shape
    /// still carrying the rect it is clipped to. No `min_span`, as the bars
    /// that ask for this paint have none: their span closes for a hard edge.
    fn paint_fade_bar_clipped(low: f32, high: f32) -> Vec<egui::epaint::ClippedShape> {
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
        out.shapes
    }

    /// The same, as bare shapes — what every reading but the clip wants.
    fn paint_fade_bar(low: f32, high: f32) -> Vec<egui::Shape> {
        paint_fade_bar_clipped(low, high).into_iter().map(|s| s.shape).collect()
    }

    /// Where a value stands on a [`RangeBar::fade_span`] bar's axis, which runs
    /// the whole bar rather than the inset track every other range bar puts its
    /// values on — so its floor is the bar's own left edge.
    fn fade_x_of(bar: egui::Rect, v: f32) -> f32 {
        bar.left() + bar.width() * (v - AXIS.0) / (AXIS.1 - AXIS.0)
    }

    /// The SOLID head of a [`RangeBar::fade_span`] fill — the stretch left of
    /// `low`, which is one flat color and so a rounded rect rather than part of
    /// the ramp mesh. Picked out by its color: the well beneath it and the two
    /// handles over it are the only other rects a bar fills, and neither wears
    /// the accent.
    fn fade_head(shapes: &[egui::Shape]) -> Option<(egui::Rect, egui::Color32)> {
        filled_rects(shapes).into_iter().find(|(_, fill)| *fill == theme::accent_fill())
    }

    /// The gradient fill's columns, left to right: where each one stands and
    /// what color it carries.
    fn fill_ramp(shapes: &[egui::Shape]) -> Vec<(f32, egui::Color32)> {
        let mut columns = Vec::new();
        for shape in shapes {
            if let egui::Shape::Mesh(mesh) = shape {
                for (top, _, color) in band_columns(mesh) {
                    columns.push((top.x, color));
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
        // way, because that is what a real hand delivers: egui calls the press
        // a drag only once the pointer has left the threshold, so the first
        // frame the bar sees is never at `from`. A harness that jumped straight
        // to its target would hand the bar a first frame at the destination and
        // never put a gap between where the press landed and where the gesture
        // is read — which is the gap [`aimed_at`] exists for.
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

    /// The name is drawn THROUGH a thumb standing in it rather than under one:
    /// the same galley, at the same origin, painted a second time in the panel
    /// colour and clipped to the grip, so its letters cross the thumb in
    /// reverse instead of vanishing into a block of the near-white they are
    /// already drawn in. That is what buys the name the right to stay PUT under
    /// a handle, where sliding it clear was built and dropped for snapping back
    /// mid-drag.
    ///
    /// Only the NAME: this bar's two numbers are placed into runs of clear
    /// track instead, and `no_thumb_ever_stands_in_a_number_at_the_widths_the_
    /// column_opens_at` is what holds that placement honest. A [`SpreadBar`]
    /// has no such placement and knocks out both of its runs.
    ///
    /// [`SpreadBar`]: super::gradient::SpreadBar
    #[test]
    fn the_name_is_knocked_out_where_a_thumb_stands_in_it() {
        // Two minimum spans down at the bottom of the axis, chosen to reach
        // both arms of the loop that paints this. The 680pt row crosses the
        // name with its HIGH thumb only — at the very floor the low one stops a
        // point short of the name's first letter, which is the clearance the
        // type's docs claim for the bars that open at the full axis. The 300pt
        // row has the same twelve semitones spanning a quarter of the points,
        // so both thumbs fit inside the name at once.
        let mut seen = Vec::new();
        for (width, low) in [(680.0, AXIS.0), (300.0, AXIS.0 + 6.0)] {
            let shapes = paint_range_bar_clipped(width, low, low + OCTAVE);
            let flat: Vec<_> = shapes.iter().map(|s| s.shape.clone()).collect();
            let grips = handles(&flat);
            let name = text_boxes(&flat)[0].0;
            let knocked = knockouts(&shapes);

            // Derived from the geometry rather than hard-coded, so the count
            // tracks the fixture instead of pinning a number a font change
            // could move — with a floor under it, or a fixture that stopped
            // standing a thumb in the name would pass by asserting nothing.
            let crossing: Vec<_> = grips.iter().copied().filter(|g| g.intersects(name)).collect();
            assert!(!crossing.is_empty(), "{width}pt: no thumb stands in the name any more");
            assert_eq!(
                knocked.len(),
                crossing.len(),
                "{width}pt: one knockout per thumb in the name, {knocked:?} vs {grips:?}",
            );
            for ((clip, at, what, colour), grip) in knocked.iter().zip(&crossing) {
                assert_eq!(what, NAME, "{width}pt: only the name is knocked out, not {what:?}");
                assert_eq!(*at, name, "{width}pt: a knockout is the same galley, same place");
                assert_eq!(
                    *colour,
                    Some(theme::panel()),
                    "{width}pt: a knockout is drawn in the panel colour",
                );
                assert_eq!(*clip, *grip, "{width}pt: a knockout is confined to its thumb");
            }
            seen.push(crossing.len());
        }
        // The point of the second fixture: one thumb in the name is the easy
        // case, and the loop that paints both is only exercised by the other.
        assert_eq!(seen, vec![1, 2], "the pair stopped covering one thumb and then two");
    }

    /// And emits nothing where no thumb reaches the name, which is where the
    /// bars rest: one pass and no clipped second one. The saving is a shape
    /// rather than a tessellation (epaint would cull the row anyway — see
    /// [`grip_over_text`]); what it really buys is a paint list in which a
    /// knockout shape means a knockout happened, which is what lets the test
    /// above count them. This holds the guard that keeps it true.
    #[test]
    fn the_name_is_painted_once_where_no_thumb_reaches_it() {
        for (low, high) in [(60.0, 72.0), (AXIS.0, AXIS.1)] {
            let shapes = paint_range_bar_clipped(300.0, low, high);
            let knocked = knockouts(&shapes);
            assert!(
                knocked.is_empty(),
                "{low}..{high}: paid for a knockout it cannot see: {knocked:?}",
            );
        }
    }

    /// A [`RangeBar::fade_span`] bar is knocked out like any other, and it is
    /// the one that needs it most: the two of them rest with a thumb inside the
    /// name's own share of the bar, and a fresh Gutter stands its low end on
    /// the "G". Every other bar has to be dragged there.
    ///
    /// It is also the bar that stretches the square clip furthest. Its thumb
    /// takes the BAR's corner rather than a grip's, which on a 6pt width epaint
    /// holds to a 3pt pill, so the corner notches the clip cannot follow are
    /// the whole of the top and bottom 3pt rather than a sliver — see
    /// [`grip_over_text`] for why that is a bound worth stating and not a bug
    /// worth code. What is asserted here is what the shape list can answer: the
    /// knockout happens, on the thumb, in the panel colour.
    #[test]
    fn a_fade_span_bars_name_is_knocked_out_where_its_thumb_rests_on_it() {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 100.0));
        // A low end just inside the name, the way a fresh Gutter opens.
        let (mut lo, mut hi) = (AXIS.0 + 3.0, AXIS.0 + 20.0);
        let out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                RangeBar::new(&mut lo, &mut hi, AXIS.0..=AXIS.1, NAME)
                    .min_span(OCTAVE)
                    .fade_span()
                    .show(ui);
            },
        );
        let flat: Vec<_> = out.shapes.iter().map(|s| s.shape.clone()).collect();
        let (grips, name) = (handles(&flat), text_boxes(&flat)[0].0);
        let knocked = knockouts(&out.shapes);
        let crossing: Vec<_> = grips.iter().copied().filter(|g| g.intersects(name)).collect();
        assert!(!crossing.is_empty(), "this fixture no longer rests a thumb on the name");
        assert_eq!(
            knocked.len(),
            crossing.len(),
            "a fade_span bar skipped a knockout: {knocked:?} against {grips:?}",
        );
        for ((clip, at, what, colour), grip) in knocked.iter().zip(&crossing) {
            assert_eq!(what, NAME, "a fade_span bar knocked out {what:?}");
            assert_eq!(*at, name, "a knockout moved off the name");
            assert_eq!(*colour, Some(theme::panel()), "a knockout is in the panel colour");
            assert_eq!(*clip, *grip, "a knockout escaped its thumb");
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
    ///
    /// [`SpreadBar`]: super::gradient::SpreadBar
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

    /// A row with no width at all still PAINTS. Nothing about it is legible,
    /// which is fine — what is not fine is a panic, and a bar is one `clamp`
    /// away from one at every step that holds a thumb or a number inside the
    /// bar's own ends: below one handle's width those bounds cross, `clamp`
    /// asserts `min <= max`, and the panic takes the whole editor down with it.
    ///
    /// Zero rather than merely narrow, because [`bar_width`] floors there: a
    /// column squeezed past its own controls hands the bar a negative width and
    /// gets the floor, so this is the row the dock can really produce and not a
    /// hypothetical one. Both paints, since the fade bar is the one whose
    /// values reach the ends this arithmetic is about.
    #[test]
    fn a_row_of_no_width_paints_rather_than_panicking() {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(0.0, 100.0));
        for fade in [false, true] {
            let (mut lo, mut hi) = (AXIS.0, AXIS.1);
            let out = ctx.run_ui(
                egui::RawInput { screen_rect: Some(screen), ..Default::default() },
                |ui| {
                    let bar = RangeBar::new(&mut lo, &mut hi, AXIS.0..=AXIS.1, NAME);
                    if fade { bar.fade_span().show(ui) } else { bar.show(ui) };
                },
            );
            let shapes: Vec<egui::Shape> = out.shapes.into_iter().map(|s| s.shape).collect();
            assert!(!filled_rects(&shapes).is_empty(), "fade={fade}: the well was not painted");
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

    /// And through a real pointer: a press within an end's reach takes THAT
    /// end, whichever way the drag then runs.
    ///
    /// [`Grab::at`] is asked on the first frame egui calls the press a drag,
    /// which is already six points along — see [`aimed_at`]. Asked at the live
    /// position, a press in the outer half of the reach that then runs INWARD
    /// is past the reach by the time the question reaches this bar, so it reads
    /// as a middle grab and slides both ends. That is exactly the mistake
    /// [`GRAB_PX`] is generous to prevent, made by the bar itself, and the
    /// `Grab::at` cases above cannot see it: they hand the function the value
    /// the gesture never had.
    #[test]
    fn a_press_within_an_ends_reach_takes_that_end_whichever_way_it_runs() {
        let held = (48.0f32, 96.0f32);
        let bar = filled_rects(&paint_range_bar(held.0, held.1))[0].0;
        let track = bar.shrink2(Vec2::new(HANDLE_INSET, 0.0));
        let x_of = |v: f32| track.left() + track.width() * (v - AXIS.0) / (AXIS.1 - AXIS.0);
        let frac = |x: f32| (x - bar.left()) / bar.width();
        // Inside the reach on either side of the handle, which the cursor
        // spells out as a `ResizeHorizontal` before the press.
        for reach in [-GRAB_PX * 0.8, GRAB_PX * 0.8] {
            for run in [-50.0f32, 50.0] {
                let from = x_of(held.0) + reach;
                let (lo, hi) = drag_range_bar(held, (frac(from), frac(from + run)), false);
                let aimed = format!("pressed {reach} from the low end and dragged {run}");
                assert_eq!(hi, held.1, "{aimed}: the high end came along");
                assert_ne!(lo, held.0, "{aimed}: the low end did not move");
            }
        }
    }

    /// A pair already at `min_span` answers a press in an end's reach one way
    /// and holds the other — the inert band [`Grab::at`]'s doc names, pinned
    /// here so it cannot drift back into being an accident.
    ///
    /// Both halves matter. Inward the end is against `min_span` and writes
    /// nothing, which is a handle against a wall; outward the SAME press opens
    /// the span, which is the gesture the band exists for and the reason it is
    /// not handed to the slide. A change to `Grab::apply` that let a pinned end
    /// carry its partner would fail the first half, and should — that is a
    /// decision about what `Low` means, not a tidy-up.
    #[test]
    fn an_end_against_the_minimum_span_holds_inward_and_opens_outward() {
        // Exactly at OCTAVE, the min_span these bars declare.
        let held = (60.0f32, 72.0f32);
        let bar = filled_rects(&paint_range_bar(held.0, held.1))[0].0;
        let track = bar.shrink2(Vec2::new(HANDLE_INSET, 0.0));
        let x_of = |v: f32| track.left() + track.width() * (v - AXIS.0) / (AXIS.1 - AXIS.0);
        let frac = |x: f32| (x - bar.left()) / bar.width();
        // Inside the reach, which a 12-unit span caps at 0.35 of itself.
        let from = x_of(held.0) + 8.0;
        let inward = drag_range_bar(held, (frac(from), frac(from + 40.0)), false);
        assert_eq!(inward, held, "the low end moved into a span already at its minimum");
        let outward = drag_range_bar(held, (frac(from), frac(from - 40.0)), false);
        assert!(outward.0 < held.0, "the same press outward did not open the span");
        assert_eq!(outward.1, held.1, "opening the span carried the high end with it");
    }

    /// The reach still cannot swallow a narrow span whole, or a zoomed-in
    /// range would have no middle left to slide along the axis.
    #[test]
    fn the_handle_reach_leaves_a_narrow_span_pannable() {
        let narrow = (60.0, 60.0 + OCTAVE);
        let middle = 60.0 + OCTAVE / 2.0;
        assert!(matches!(Grab::at(middle, narrow, AXIS, 1_000.0), Grab::Span { .. }));
    }

    /// A `fade_span` bar paints the pair as the edge it describes: solid from
    /// the bar's left edge as far as `low`, then ramping out to the bare track
    /// by `high`.
    ///
    /// The default paint says the opposite of this — it fills exactly the part
    /// that is FADING and leaves the solid part bare — which on a pair that is
    /// a reach and its fade reads as a bright band floating off the node it is
    /// measured from.
    ///
    /// The left edge is the BAR's, not the inset track's: a fill starting where
    /// the values do leaves a sliver of bare well no other bar in the pane has,
    /// and it reads as a notch in the control.
    ///
    /// Two shapes, and the split is the point rather than an implementation
    /// detail: the solid part is a rounded RECT, which is what leaves the bar's
    /// own corner with the machinery that antialiases it, and the mesh takes
    /// only the stretch that needs a gradient. So this reads them in turn and
    /// pins the join — the ramp starts where the head stops, in the head's own
    /// color, or the fill has a seam in the middle of itself.
    #[test]
    fn a_fade_span_fills_from_the_floor_and_ramps_out() {
        let (low, high) = (60.0, 100.0);
        let shapes = paint_fade_bar(low, high);
        let bar = filled_rects(&shapes)[0].0;
        let x_of = |v: f32| fade_x_of(bar, v);
        let (head, head_color) = fade_head(&shapes).expect("a fade span painted no solid head");
        assert!(
            (head.left() - bar.left()).abs() < 0.01,
            "the fill starts at {} rather than at the bar's left edge {}",
            head.left(),
            bar.left(),
        );
        assert!(
            (head.right() - x_of(low)).abs() < 0.01,
            "the solid head stops at {} rather than at `low` ({})",
            head.right(),
            x_of(low),
        );

        let ramp = fill_ramp(&shapes);
        assert!(!ramp.is_empty(), "a fade span painted no ramp");

        let (first, last) = (ramp[0], ramp[ramp.len() - 1]);
        assert!(
            (first.0 - x_of(low)).abs() < 0.01,
            "the ramp starts at {} rather than where the head stops ({})",
            first.0,
            x_of(low),
        );
        assert_eq!(first.1, head_color, "the ramp starts a different color from the head");
        assert!(
            (last.0 - x_of(high)).abs() < 0.01,
            "the fill ends at {} rather than at `high` ({})",
            last.0,
            x_of(high),
        );
        assert_eq!(last.1, theme::well(), "the fill does not reach the bare track by `high`");

        // Never brightening after `low`. Read as distance from the well rather
        // than as a color, so this asks the one thing that matters — how much
        // fill is left — of whatever the skin's accent happens to be.
        let well = egui::Rgba::from(theme::well());
        let from_well = |c: egui::Color32| {
            let c = egui::Rgba::from(c);
            ((c.r() - well.r()).powi(2) + (c.g() - well.g()).powi(2) + (c.b() - well.b()).powi(2))
                .sqrt()
        };
        let solid = from_well(head_color);
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
            assert!(
                from_well(color) <= previous + 0.01,
                "the fill brightens again at {x}",
            );
            previous = from_well(color);
        }
    }

    /// A fade dragged fully OPEN keeps the bar's corner, which takes widening
    /// the solid head past where it is painted.
    ///
    /// The low end at the axis floor puts the head at one radius, five points,
    /// and epaint holds a corner radius to half the rect's shortest side — so
    /// the head would round at two and a half points where the well rounds at
    /// five and poke out through the well's own arc. The bar changing shape as
    /// a handle reaches the end of its travel is the thing to keep out, and it
    /// is invisible in the shapes themselves: the radius handed to
    /// `rect_filled` is the same either way, and only the tessellator clamps
    /// it. So the width is what this asks about.
    ///
    /// The ramp is what makes the widening free, and it must clear the CORNER
    /// for that: a fade this open has its low end on the bar's own edge, so a
    /// ramp starting where the value says would take the corner back from the
    /// rect that can draw it.
    #[test]
    fn a_fully_soft_edge_keeps_the_fills_corner_round() {
        let shapes = paint_fade_bar(AXIS.0, 100.0);
        let bar = filled_rects(&shapes)[0].0;
        let (head, _) = fade_head(&shapes).expect("a fully soft edge painted no solid head");
        let radius = f32::from(bar_radius(1.0));
        assert!(
            head.width() >= 2.0 * radius,
            "the head is {:.1}pt wide, under the {:.1}pt its own corner needs",
            head.width(),
            2.0 * radius,
        );
        let ramp = fill_ramp(&shapes);
        let first = ramp.first().expect("a fully soft edge painted no ramp").0;
        assert!(
            (first - (bar.left() + radius)).abs() < 0.01,
            "the ramp starts at {first}, not clear of the corner ({})",
            bar.left() + radius,
        );
    }

    /// A fade bar's thumb reaches the end of the bar. Its axis runs the whole
    /// bar for that — a low end at the axis floor is a gutter of no softness at
    /// all, an ordinary setting, and a thumb stopping a point clear of the edge
    /// says the control cannot reach it.
    ///
    /// Flush, not hanging off: the value puts the thumb's CENTER on the edge,
    /// and what is drawn is held in by half a handle, the same way the octave
    /// strip holds its own boundary marks.
    #[test]
    fn a_fade_bars_thumb_reaches_the_end_of_the_bar() {
        let shapes = paint_fade_bar(AXIS.0, 100.0);
        let bar = filled_rects(&shapes)[0].0;
        let thumb = *handles(&shapes).first().expect("a fade bar painted no thumb");
        assert!(
            (thumb.left() - bar.left()).abs() < 0.01,
            "the thumb's edge is at {} rather than at the bar's own ({})",
            thumb.left(),
            bar.left(),
        );
        assert!(
            thumb.width() >= HANDLE_W - 0.01,
            "the thumb was cut down to {:.1}pt to fit",
            thumb.width(),
        );
    }

    /// The inset stays where a bar opens at the full axis: both handles flush
    /// on a bare track is the state a plain range bar starts life in, and there
    /// they read as the bar's own border rather than as something to grab.
    #[test]
    fn a_plain_range_bars_thumb_keeps_its_clearance() {
        let shapes = paint_range_bar(AXIS.0, AXIS.1);
        let bar = filled_rects(&shapes)[0].0;
        let thumb = *handles(&shapes).first().expect("a range bar painted no thumb");
        assert!(
            thumb.left() > bar.left() + 0.5,
            "the low thumb sits at {}, flush with the bar's edge ({})",
            thumb.left(),
            bar.left(),
        );
    }

    /// The widening is cut back at the reach, and the CLIP is what cuts it.
    ///
    /// A reach under twice the corner radius leaves the head running past
    /// `high` — which is the whole point of widening it, since the alternative
    /// is a corner clamped to half of whatever room is left. Nothing in the
    /// rect itself then says where the fill stops, so the one thing standing
    /// between a two-point gutter and ten points of solid bar is the clip. Read
    /// it directly: a shape's own geometry cannot show it.
    #[test]
    fn a_widened_head_is_still_cut_back_at_the_reach() {
        // A hundredth of the axis: a couple of points of reach on this row.
        let high = AXIS.0 + (AXIS.1 - AXIS.0) * 0.01;
        let painted = paint_fade_bar_clipped(AXIS.0, high);
        let shapes: Vec<egui::Shape> = painted.iter().map(|s| s.shape.clone()).collect();
        let bar = filled_rects(&shapes)[0].0;
        let hx = fade_x_of(bar, high);
        let (clip, head) = painted
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.fill == theme::accent_fill() => {
                    Some((s.clip_rect, r.rect))
                }
                _ => None,
            })
            .expect("a fade span painted no solid head");
        assert!(
            head.right() > hx + 0.01,
            "the head stops at {} without reaching past `high` ({hx}), so this pins nothing",
            head.right(),
        );
        assert!(
            (clip.right() - hx).abs() < 0.01,
            "the fill is cut at {} rather than at the reach ({hx})",
            clip.right(),
        );
    }

    /// An edge of no reach paints NOTHING — neither part of the fill — and it
    /// takes a check of its own to say so. The solid head is widened past its
    /// own end so that its corner is never clamped, and the CLIP is what takes
    /// it back at the reach; with the reach at the floor there is nothing left
    /// to cut it back to, and a control switched off keeps a hair of the head's
    /// own feathered edge.
    #[test]
    fn an_edge_of_no_reach_paints_no_fill() {
        let shapes = paint_fade_bar(AXIS.0, AXIS.0);
        let width = match (fill_ramp(&shapes).first(), fill_ramp(&shapes).last()) {
            (Some(first), Some(last)) => last.0 - first.0,
            _ => 0.0,
        };
        assert!(width < 0.01, "an edge of no reach paints {width:.2}pt of ramp");
        assert!(
            fade_head(&shapes).is_none(),
            "an edge of no reach paints a solid head of fill",
        );
    }

    /// A hard edge closes the span, and the fill is then solid the whole way
    /// with no ramp at all — which is the picture of a hard edge, and the one
    /// setting where this bar and the plain fill it replaces look the same.
    ///
    /// Solid the whole way means the head takes ALL of it and the mesh is never
    /// built, so this asks for the absence of a ramp rather than for a flat one.
    /// The zero-width strip that would otherwise be handed to `gradient_strip`
    /// is the case the guard beside it exists for.
    #[test]
    fn a_closed_fade_span_paints_a_solid_fill() {
        let shapes = paint_fade_bar(60.0, 60.0);
        let (head, _) = fade_head(&shapes).expect("a closed fade span painted no fill");
        let bar = filled_rects(&shapes)[0].0;
        let x_of = |v: f32| fade_x_of(bar, v);
        assert!(
            (head.left() - bar.left()).abs() < 0.01 && (head.right() - x_of(60.0)).abs() < 0.01,
            "a closed span's solid fill runs {}..{} rather than {}..{}",
            head.left(),
            head.right(),
            bar.left(),
            x_of(60.0),
        );
        assert!(fill_ramp(&shapes).is_empty(), "a closed fade span painted a ramp");
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
    /// Color & light's Color range is two host params with nothing between
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

    /// A fade bar's ramp fades out at its edges too, and it is the band that
    /// asks the question differently: it is SQUARE at the end it hands to the
    /// solid head, where the outline turns a right angle and the mitre is the
    /// one offset in the file longer than the feather it is made of.
    ///
    /// The two bands above cannot stand in for it. Their ends are rounded, so
    /// the last chord before the end already faces all but straight out along
    /// the axis, and an end that took its facing from that chord instead of
    /// from its own vertical run misses `fades_out_at_its_edges` by a
    /// thousandth of a point — it passes, on a ring drawn very nearly right.
    /// On a square end the same mistake points the offset straight up and
    /// leaves that end with no ring at all, which is the failure this suite is
    /// meant to be able to see.
    #[test]
    fn a_fade_ramp_fades_out_at_its_square_end() {
        // Wide enough apart to leave the ramp a stretch of its own, and clear
        // of the corner the head keeps (see `a_fully_soft_edge_...`).
        let shapes = paint_fade_bar(60.0, 100.0);
        let ramp = bands(&shapes);
        let ramp = match ramp.as_slice() {
            [one] => one,
            other => panic!("a fade bar paints one ramp, not {}", other.len()),
        };
        fades_out_at_its_edges("fade ramp", ramp);
    }
}
