//! The Spectral plane's coordinate system: pitch across it, depth into it,
//! and what a level means once the spectrum curve is drawn against depth.
//!
//! Split out because it is not the pane's — [`roll`](super::roll),
//! [`spectrogram`](super::spectrogram) and [`names`](super::names) all draw
//! into the same plane and all read it. Everything here is expressed in the
//! abstract *(pitch, depth)* pair; [`Axes`] is the one place that knows which
//! screen direction either of them runs in, so an orientation change turns
//! every layer together and no layer has to know which way is up.

use crate::SharedState;

/// The profile line along the spectrum curve's top, in points — the light edge
/// that gives the fill a boundary to be seen by (see
/// [`keyline`](super::roll::keyline) for what colors it).
pub(super) const PROFILE_PT: f32 = 1.0;

/// What the spectrum curve stops short of the pane's outer edge by, in POINTS:
/// half the profile line, which is centred on the curve's top and would
/// otherwise be half-clipped by the edge.
///
/// That is the WHOLE clearance, and deliberately so — at the ceiling the ink
/// reaches the pane edge and the pane carries no empty band at all. Where the
/// analyzer ends is already drawn, by the dock separator between it and the
/// pane beside it; a second boundary inside the picture is one border too
/// many.
///
/// In points rather than as a fraction of the spectrum's share. A fraction is
/// an empty margin that grows with the pane, so the same picture carries a
/// thicker border the larger it is drawn, and on a tall analyzer that band is
/// the loudest empty thing on it. What the room is for is one line, and a line
/// is the same width at every pane size.
pub(super) const PLOT_HEADROOM_PT: f32 = PROFILE_PT * 0.5;

/// How far into the depth axis the spectrum curve may reach: the spectrum's
/// whole share of it, less [`PLOT_HEADROOM_PT`] expressed in that axis' own
/// fraction — which is what `depth_len`, the axis in points, is for.
///
/// Floors at zero, and that floor is the whole guard. A pane too short to hold
/// even the headroom draws a flat curve rather than one that reaches back
/// through the now-line and paints the spectrum into the roll's half, and a
/// zero-length axis divides to an infinity that the same floor takes to zero —
/// so nothing here can hand egui's tessellator a NaN. Holding `depth_len` into
/// range instead would defeat both: a sub-point axis would come back with a
/// budget worth half of itself.
pub(super) fn plot_budget(split: f32, depth_len: f32) -> f32 {
    (split - PLOT_HEADROOM_PT / depth_len).max(0.0)
}

/// The 1 kHz pivot of the tilt slope, as a MIDI pitch.
pub(super) const TILT_PIVOT_MIDI: f32 = 83.213_1;

/// Point size of an axis marking's label — a dozen standing marks that
/// should stay quiet.
///
/// Doubled from the 10 it was drawn at before the Label size bar existed. The
/// bar went to 2 the first time it was tried and stayed there — so the number
/// was wrong rather than the bar wanted, and rebasing it leaves the bar
/// reading 1 at the size the pane is actually read at.
pub(super) const MARKING_PT: f32 = 20.0;

/// The whole pitch axis, in semitones — the widest the range opens, and the
/// zoom the note names' built-in size is dialled for.
pub(super) const FULL_PITCH_SPAN: f32 =
    harmonigraph_core::spectrum::SPECTRUM_MAX_MIDI - harmonigraph_core::spectrum::SPECTRUM_MIN_MIDI;

/// How much bigger a note name draws with the pitch range zoomed to `span`
/// semitones: 1 across the whole axis, growing in PROPORTION as it narrows.
///
/// The names are written ON the ribbons, and a ribbon's width is set in
/// SEMITONES — so zooming the pitch range draws every ribbon thicker by
/// exactly the zoom factor while leaving the name that names it the size it
/// was. That is the mismatch this answers, and proportional answers it
/// squarely: a name keeps a constant share of the pitch axis, so it holds the
/// same relation to its ribbon at every zoom and the picture simply gets
/// bigger, type and all.
///
/// Which turns "how large can a name get" into "how far may the range be
/// CLOSED", and that is answered where it belongs — see
/// [`PITCH_RANGE_MIN_SPAN`](crate::PITCH_RANGE_MIN_SPAN), whose two octaves
/// against this ten-octave axis are what cap a name at five times its dialled
/// size. One law with a limit at the end beats a law that bends in the middle:
/// a softened curve holds the far end down by making a name disagree with its
/// ribbon at every zoom in between, which is where the reading actually
/// happens.
///
/// Thinning follows for free: [`plan`](super::names::plan) measures the room a
/// name demands from the size it is drawn at, so names that have grown are
/// spaced further apart and the ones that no longer fit are dropped, exactly
/// as they are at any other size.
///
/// Never SMALLER than the built-in size: the reference is the widest range the
/// axis offers, so the only direction left is up.
///
/// Says nothing about which way the pane is turned. The span is in semitones,
/// not in points, so a Top or Bottom pane (whose pitch axis is horizontal) scales
/// exactly as a wide one does — [`Axes`] is the only thing here that knows a
/// screen side, and this is not it.
pub(super) fn name_zoom(span: f32) -> f32 {
    FULL_PITCH_SPAN / span.clamp(crate::PITCH_RANGE_MIN_SPAN, FULL_PITCH_SPAN)
}

/// The pane the sizes here are quoted against: a pitch axis 860 points long,
/// which is what the Spectral pane gets in the 1512x886 window they were
/// dialled in. Halve the pane and the type halves with it, along with every
/// ribbon and every band it is written over.
///
/// The PITCH axis, of the two: a ribbon's width is set in semitones, so that
/// is the axis whose length decides how big the picture under a name is. It is
/// also the short side, and so the one a window resize tends to take first.
pub(super) const REFERENCE_PITCH_LEN: f32 = 860.0;

/// The sizes this pane sets text at for one frame, as multiples of each
/// piece's built-in point size.
///
/// Three factors go into each: the pane's own size against
/// [`REFERENCE_PITCH_LEN`], the user's bar for that kind of text, and — for
/// the note names alone — the pitch zoom.
///
/// The markings come out snapped onto the size ladder, since they change only
/// with the pane and a snapped size is one fewer entry in egui's font atlas.
/// The names do not: they follow the pitch zoom, and quantizing a size that
/// follows a zoom is what makes type step against the ribbons it is written
/// over. They are snapped for rasterizing alone, where they are drawn.
#[derive(Clone, Copy, Debug)]
pub(super) struct TextScales {
    /// The axis marking labels.
    pub(super) markings: f32,
    /// The name written on each ribbon.
    pub(super) names: f32,
}

pub(super) fn text_scales(cfg: &crate::SpectrumConfig, axes: &Axes, span: f32, ppp: f32) -> TextScales {
    let pane = axes.pitch_len() / REFERENCE_PITCH_LEN;
    TextScales {
        markings: crate::text::snap_scale(pane * cfg.marking_scale, MARKING_PT, ppp),
        // NOT snapped, unlike the markings above: this is the one size here
        // that follows a zoom, and a zoom is continuous. It is snapped for
        // RASTERIZING where it is drawn (`names::draw`), which is what keeps
        // the atlas bounded without quantizing the picture -- see
        // `crate::text::TextBatch::magnified`.
        names: pane * cfg.note_name_scale * name_zoom(span),
    }
}

/// How loud `power` reads at pitch `midi`, on a 0..1 scale: the configured
/// floor is 0, the configured ceiling is 1, and the tilt lifts treble by
/// its slope above the 1 kHz pivot. The spectrum curve's height and the
/// spectrogram's cell intensity both read from this, so the two always agree
/// on what "loud" means for a given bucket.
pub(crate) fn loudness(cfg: &crate::SpectrumConfig, power: f32, midi: f32) -> f32 {
    loudness_db(cfg, power_db(power), midi)
}

/// A bucket's power as dB — the form the spectrogram's history already stores,
/// and the only thing [`loudness`] does with power before mapping it.
pub(crate) fn power_db(power: f32) -> f32 {
    10.0 * power.max(1e-12).log10()
}

/// [`loudness`] from a bucket already in dB, so the heatmap (whose columns are
/// stored that way) never takes a `log10` per pixel.
///
/// The heatmap reads exactly this and nothing of its own. Giving it a private dB
/// window and a contrast curve is tempting — a curve is read as a shape against
/// a baseline and a picture is read as a picture, so the ranges that suit them
/// need not coincide. What that argument misses is that one range IS what makes
/// "loud" the same claim in both halves of one pane, and a second one is a
/// second thing to keep in step.
pub(crate) fn loudness_db(cfg: &crate::SpectrumConfig, power_db: f32, midi: f32) -> f32 {
    loudness_raw(cfg, power_db, midi).clamp(0.0, 1.0)
}

/// [`loudness_db`] before its 0..1 clamp — AFFINE in `power_db`, with a slope
/// that does not depend on `midi`.
///
/// That shape is why this is exposed rather than folded into the one caller
/// that wants it. The heatmap's cells are stored dB bytes, so a row's whole
/// mapping is `level = row0 + step * byte` for two constants it can lift out of
/// the pixel loop, and the ramp behind it can then be a table
/// (`Shades` in the spectrogram) instead of a per-pixel evaluation — measured at
/// 58% of a full repaint's arithmetic.
///
/// The clamp is what makes it a separate function rather than a note on the
/// existing one: at a stored `0` the level is far below the floor for any
/// ordinary dB window, so reading the constants out of the CLAMPED mapping
/// returns 0 for both and flattens the row. Deriving them here keeps one copy of
/// the formula — `the_shade_table_matches_the_mapping_it_replaces` holds the
/// table to it byte for byte.
pub(crate) fn loudness_raw(cfg: &crate::SpectrumConfig, power_db: f32, midi: f32) -> f32 {
    let db = power_db - cfg.tilt * (midi - TILT_PIVOT_MIDI) / 12.0;
    // Never trust the pair to be ordered or apart, exactly as the pitch range
    // is not trusted: the bar can't produce a collapsed one, a hand-edited
    // state blob can, and dividing by its zero span paints NaN geometry that
    // takes the editor — and with it the host — down.
    let ceiling = cfg.ceiling_db.max(cfg.floor_db + crate::LEVEL_RANGE_MIN_SPAN);
    (db - cfg.floor_db) / (ceiling - cfg.floor_db)
}

/// The pane's abstract drawing plane, and how it lands on screen.
///
/// Two axes, both running `0..1`:
///
/// - **pitch** runs across the pane's SHORT side; 0 is the low end of the
///   pitch range.
/// - **depth** is the time axis, running along the LONG side; 0 is the
///   spectrum's outer edge, `split` the now-line (where the spectrum joins
///   the spectrogram), and 1 the far edge (the roll's oldest notes).
///
/// Orientation is handled inside [`at`](Self::at); nothing else in the pane
/// names a screen side. Four layouts, named for the side the now-line is on —
/// **Left** (time left->right, pitch bottom->top), **Right** (the same flipped
/// along time), **Top** (time top->bottom, pitch left->right) and **Bottom**.
/// In all four the spectrum sits at the now-line end.
#[derive(Clone, Copy)]
pub(super) struct Axes {
    pub rect: egui::Rect,
    /// Time (the depth axis) runs down the pane rather than along it, with
    /// pitch across. See [`SpectralOrientation`](crate::SpectralOrientation).
    pub(super) time_vertical: bool,
    /// Time runs against its screen axis — leftward, or upward.
    pub(super) time_reversed: bool,
}

impl Axes {
    pub(super) fn new(rect: egui::Rect, cfg: &crate::SpectrumConfig) -> Axes {
        Axes {
            rect,
            time_vertical: cfg.orientation.is_time_vertical(),
            time_reversed: cfg.orientation.is_time_reversed(),
        }
    }

    /// The screen point at pitch fraction `p` and depth (time) fraction `d`.
    pub fn at(&self, p: f32, d: f32) -> egui::Pos2 {
        // Depth measured from whichever edge the now-line is on. Reversed, the
        // past runs back toward the origin of the screen axis; pitch is not
        // touched by it, so it keeps reading low-to-high the conventional way.
        let d = if self.time_reversed { 1.0 - d } else { d };
        if self.time_vertical {
            // Time runs down the pane (Top) or up it (Bottom); pitch runs
            // across, low left to high right.
            egui::pos2(
                self.rect.left() + self.rect.width() * p,
                self.rect.top() + self.rect.height() * d,
            )
        } else {
            // Time runs along the pane, rightward (Left) or leftward (Right);
            // pitch climbs, low bottom to high top.
            egui::pos2(
                self.rect.left() + self.rect.width() * d,
                self.rect.bottom() - self.rect.height() * p,
            )
        }
    }

    /// Points spanned by the full pitch axis (the short side).
    pub fn pitch_len(&self) -> f32 {
        if self.time_vertical { self.rect.width() } else { self.rect.height() }
    }

    /// Points spanned by the full depth/time axis (the long side).
    ///
    /// Points rather than device pixels, and the distinction is load-bearing:
    /// this is the divisor that turns a size in points ([`PLOT_HEADROOM_PT`])
    /// into a fraction of the axis, so reading it as physical pixels halves
    /// that size on a 2x display. [`roll`](super::roll)'s `MIN_LENGTH_DEVICE_PX`
    /// is what a floor in the other unit looks like, and says so in its name.
    pub fn depth_len(&self) -> f32 {
        if self.time_vertical { self.rect.height() } else { self.rect.width() }
    }

    /// Which way the pitch axis points on screen (unit vector).
    pub(super) fn dir_pitch(&self) -> egui::Vec2 {
        (self.at(1.0, 0.0) - self.at(0.0, 0.0)).normalized()
    }

    /// Which way the depth axis points on screen (unit vector).
    pub(super) fn dir_depth(&self) -> egui::Vec2 {
        (self.at(0.0, 1.0) - self.at(0.0, 0.0)).normalized()
    }

    /// A line clean across the pitch axis at depth `d` — the shape of the
    /// roll's "now" line and the divider.
    pub fn across_pitch(&self, d: f32) -> [egui::Pos2; 2] {
        [self.at(0.0, d), self.at(1.0, d)]
    }

    /// The pitch fraction under a screen position — the inverse of the
    /// pitch half of [`at`](Self::at). Unclamped.
    pub(super) fn pitch_at(&self, pos: egui::Pos2) -> f32 {
        if self.time_vertical {
            (pos.x - self.rect.left()) / self.rect.width().max(1.0)
        } else {
            (self.rect.bottom() - pos.y) / self.rect.height().max(1.0)
        }
    }

    /// The depth fraction under a screen position — the inverse of the
    /// depth half of [`at`](Self::at). Unclamped.
    pub(super) fn depth_at(&self, pos: egui::Pos2) -> f32 {
        let d = if self.time_vertical {
            (pos.y - self.rect.top()) / self.rect.height().max(1.0)
        } else {
            (pos.x - self.rect.left()) / self.rect.width().max(1.0)
        };
        if self.time_reversed { 1.0 - d } else { d }
    }

    /// A grab band `half` pixels either side of depth `d`, spanning the
    /// pitch axis — the splitter's hit area. Kept inside the pane, so the
    /// handle stays grabbable when the split is pushed all the way to an
    /// edge.
    pub(super) fn depth_band(&self, d: f32, half: f32) -> egui::Rect {
        let band = egui::Rect::from_two_pos(self.at(0.0, d), self.at(1.0, d))
            .expand2(self.dir_depth().abs() * half);
        band.intersect(self.rect)
    }

    /// Anchor and alignment for a text label at `(p, d)`, offset `along`
    /// pixels up the pitch axis and `into` pixels up the depth axis, and
    /// growing in those same directions. One helper covers both
    /// orientations: the growth direction is read off the axes rather
    /// than case-matched per side.
    pub(super) fn text_anchor(
        &self,
        p: f32,
        d: f32,
        along: f32,
        into: f32,
    ) -> (egui::Pos2, egui::Align2) {
        let (pu, du) = (self.dir_pitch(), self.dir_depth());
        let pos = self.at(p, d) + pu * along + du * into;
        let grow = pu * along.signum() + du * into.signum();
        let axis = |v: f32| {
            if v > 0.5 {
                egui::Align::Min
            } else if v < -0.5 {
                egui::Align::Max
            } else {
                egui::Align::Center
            }
        };
        (pos, egui::Align2([axis(grow.x), axis(grow.y)]))
    }
}

/// How the depth axis is shared out: the spectrum owns `0..split`, the
/// roll (and/or spectrogram) `split..1`. With that far region off the
/// spectrum owns all of it, which reproduces the pre-roll layout exactly —
/// that equality is what keeps the voice bars and the curve calibrated
/// against each other. The spectrogram shares the roll's region and time
/// axis, so it carves out the same share.
pub(super) fn spectrum_share(cfg: &crate::SpectrumConfig) -> f32 {
    if cfg.show_roll || cfg.show_spectrogram {
        (1.0 - cfg.roll_fraction).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// The whole axis the analyzer covers, as a span — the widest the pitch range
/// can be zoomed out to.
pub(super) fn widest_span() -> f32 {
    use harmonigraph_core::spectrum::{SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI};
    SPECTRUM_MAX_MIDI - SPECTRUM_MIN_MIDI
}

/// Where a pitch sits on the pane's axis: the pitch range, as a mapping.
#[derive(Clone, Copy)]
pub(super) struct PitchScale {
    pub min_midi: f32,
    pub max_midi: f32,
    pub span: f32,
}

impl PitchScale {
    /// MIDI pitch to axis fraction. Outside the zoom this runs past
    /// `0..1`; callers clip or skip.
    pub fn t_of(&self, midi: f32) -> f32 {
        (midi - self.min_midi) / self.span
    }

    pub fn contains(&self, midi: f32) -> bool {
        (self.min_midi..=self.max_midi).contains(&midi)
    }
}

/// One decade of frequency, in semitones. Ten times the frequency is
/// log2(10) octaves up and the axis is linear in semitones, so a decade is
/// the same length wherever on the axis it falls — which is what lets
/// [`frequency_grid`] settle its ladder once for all of them.
pub(super) const DECADE_SEMITONES: f32 = 12.0 * std::f32::consts::LOG2_10;

/// The closest two rulings may be drawn, in points.
///
/// The ladder is even in FREQUENCY and the axis is logarithmic, so the steps
/// inside a decade crowd toward its top: 9 kHz to 10 kHz gets a sixth of the
/// room 1 kHz to 2 kHz does. Left alone the last few of every decade close into
/// a smear that reads as one thick line, and the smear lands in the same place
/// in every decade, so it looks like a feature of the picture rather than of the
/// drawing. Dropping the steps that no longer fit is what a log axis does
/// anywhere, and it degrades in the right direction: squeezed hard enough the
/// ladder wears down to the numbered marks alone, which is the axis the pane had
/// before it was ruled at all.
pub(super) const MIN_RULING_GAP_PT: f32 = 8.0;

/// One frequency ruled across the pitch axis.
///
/// The two flags are separate because they answer separate questions, and the
/// same line often answers only one of them: 200 Hz carries a number and is not
/// a decade, 10 kHz is both, 300 Hz is neither.
#[derive(Clone, Copy, Debug)]
pub(super) struct Ruling {
    /// Where it falls on the pitch axis, `0..1`.
    pub(super) t: f32,
    pub(super) hz: f32,
    /// A decade boundary — 100 Hz, 1 kHz, 10 kHz — drawn a shade stronger.
    ///
    /// This is where the ladder RESTARTS: the step below one of these is a
    /// tenth of the step above it, which is the one thing about a grid even in
    /// frequency that the eye cannot get from the spacing alone, since a log
    /// axis draws every decade at the same length. Marking the boundary is what
    /// turns the picture from lines-that-bunch-up into three copies of one
    /// ruler.
    pub(super) decade: bool,
    /// One of the 1-2-5 series — the analyzer-standard marks, which are the
    /// ones the axis writes a number beside, and the ones thinning never drops.
    pub(super) numbered: bool,
}

/// The frequency ladder the pane rules and labels, low to high: every 10 Hz
/// below 100, every 100 Hz below 1 kHz, every 1 kHz below 10 kHz, and so on —
/// one step per decade, so the grid makes the same claim about every part of
/// the axis instead of a finer one down where the numbers happen to be smaller.
///
/// Only the steps `scale` shows come back, and only those with room for
/// themselves ([`MIN_RULING_GAP_PT`]). Which steps have room turns on the length
/// of a DECADE alone — not on where the decade sits — so it is settled once and
/// the surviving ladder is identical in every decade on the axis.
pub(super) fn frequency_grid(scale: &PitchScale, pitch_len: f32) -> Vec<Ruling> {
    // A collapsed or inverted range has no axis to rule: `t_of` divides by its
    // span, so every ruling would be placed at a NaN, and egui panics on NaN
    // geometry. The pane's own scale cannot hold one (it opens the range to
    // `PITCH_RANGE_MIN_SPAN` first) and a hand-edited state blob can. Past this,
    // `contains` is what makes a ruling's `t` finite and inside `0..1`.
    if !scale.span.is_finite() || scale.span <= 0.0 {
        return Vec::new();
    }
    let numbered = |step: i32| matches!(step, 1 | 2 | 5);
    let decade_pt = DECADE_SEMITONES / scale.span * pitch_len;
    let at = |step: i32| (step as f32).log10() * decade_pt;

    let mut keep = [false; 9];
    keep[0] = true; // Step 1 opens a decade and is numbered; it always draws.
    let mut last = at(1);
    for step in 2..=9 {
        // A numbered step draws whatever else is on the axis — it is the mark a
        // label is written beside — so the rest have to clear the ruling below
        // AND leave the next numbered step its room, or they crowd a line they
        // cannot displace.
        let next_numbered = if step < 5 { 5 } else { 10 };
        let room = at(step) - last >= MIN_RULING_GAP_PT
            && at(next_numbered) - at(step) >= MIN_RULING_GAP_PT;
        if numbered(step) || room {
            keep[step as usize - 1] = true;
            last = at(step);
        }
    }

    let mut rulings = Vec::new();
    // 1 Hz to 9 MHz. The analyzer's axis is 20 Hz to 20 kHz, so this is slack
    // around it rather than a second range to keep in step with the first —
    // what bounds the ladder is the visible range, tested per step below.
    for decade in 0..=6 {
        for step in 1..=9 {
            if !keep[step as usize - 1] {
                continue;
            }
            let hz = step as f32 * 10f32.powi(decade);
            let midi = harmonigraph_core::spectrum::hz_to_midi(hz);
            if scale.contains(midi) {
                let t = scale.t_of(midi);
                rulings.push(Ruling { t, hz, decade: step == 1, numbered: numbered(step) });
            }
        }
    }
    rulings
}

/// Maps take time to a depth fraction on the shared time axis (the region the
/// roll and spectrogram split between them), and back. One mapping for two
/// modes, so the live and offline paths go through the same place instead of
/// each carrying a copy:
///
/// - **Live**: a `now`-anchored window. `now` sits on the near edge (`split`)
///   and the past scrolls out to the far edge (1), spanning `roll_seconds`.
/// - **Whole-song** (offline playhead): the entire take laid out statically —
///   the near edge is the render's start, the far edge its end — and only the
///   playhead ([`depth_of`](Self::depth_of) of `now`) moves.
#[derive(Clone, Copy)]
pub(super) struct TimeAxis {
    pub(super) split: f32,
    pub(super) depth_span: f32,
    /// Live: `roll_seconds`. Whole-song: the take span.
    pub(super) window: f64,
    /// Take time at the near edge. Live: `now`. Whole-song: the render start.
    pub(super) origin: f64,
    pub(super) now: f64,
    pub(super) whole_song: bool,
}

impl TimeAxis {
    pub(super) fn new(state: &SharedState, split: f32, now: f64) -> Self {
        let depth_span = 1.0 - split;
        match state.whole_song.as_ref() {
            Some(ws) => TimeAxis {
                split,
                depth_span,
                window: ws.span.max(0.05),
                origin: ws.start,
                now,
                whole_song: true,
            },
            None => TimeAxis {
                split,
                depth_span,
                window: state.spectrum_config.roll_seconds.max(0.05) as f64,
                origin: now,
                now,
                whole_song: false,
            },
        }
    }

    /// Fraction from the near edge (0) to the far edge (1) for take time `t`,
    /// unclamped.
    pub(super) fn frac(&self, t: f64) -> f64 {
        if self.whole_song {
            (t - self.origin) / self.window
        } else {
            (self.origin - t) / self.window
        }
    }

    /// Depth for take time `t`, WITHOUT clamping it into the region.
    ///
    /// For geometry that is meant to overhang the region's edge and be cut off
    /// by the pane's scissor rather than squashed against it — a note ribbon
    /// leaving the far end still owes its outline and rim, and clamping paints
    /// those as a blob sitting on the edge instead of sliding out under it.
    /// Only safe for times the caller has already bounded: an unbounded one
    /// gives an unbounded depth, and a note that started ten minutes ago would
    /// become a quad ten minutes long.
    pub(super) fn depth_of_unclamped(&self, t: f64) -> f32 {
        self.split + self.frac(t) as f32 * self.depth_span
    }

    /// Depth for take time `t`, clamped into the region.
    pub(super) fn depth_of(&self, t: f64) -> f32 {
        self.split + self.frac(t).clamp(0.0, 1.0) as f32 * self.depth_span
    }

    /// Take time at a screen depth — the unclamped inverse of
    /// [`depth_of`](Self::depth_of).
    pub(super) fn time_at(&self, d: f32) -> f64 {
        let f = ((d - self.split) / self.depth_span) as f64;
        if self.whole_song {
            self.origin + f * self.window
        } else {
            self.origin - f * self.window
        }
    }

    /// The oldest take time the region shows — its far-edge cull point.
    pub(super) fn oldest(&self) -> f64 {
        if self.whole_song {
            self.origin
        } else {
            self.now - self.window
        }
    }

    /// Seconds spanned across the region.
    pub(super) fn window(&self) -> f64 {
        self.window
    }

    /// Whether this is the offline whole-song layout.
    pub(super) fn whole_song(&self) -> bool {
        self.whole_song
    }

    /// Depth of the playhead (the present moment).
    pub(super) fn playhead_depth(&self) -> f32 {
        self.depth_of(self.now)
    }
}
