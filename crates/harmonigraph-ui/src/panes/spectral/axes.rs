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

/// How far off the ruling it names a frequency label sits, ACROSS the pitch
/// axis, in points — measured to the INK, which is why the pane corrects the
/// anchor by [`ink_inset`](crate::text::ink_inset) before drawing.
///
/// Two points, which is the reach of the halo every label on this pane
/// carries: as close as the number can be set without its own rim eating the
/// line it is pointing at, and no closer. That the halo is what decides it is
/// the argument for measuring to the ink at all — a gap to the layout BOX
/// carries the font's descent as well, which scales with the type, so the same
/// constant would hold the number a hair off its line on a small pane and
/// adrift from it on a large one. Adrift is the wrong way to fail: an axis
/// logarithmic in frequency puts the ruling above a label a third as far off
/// as the one below, so which line a number belongs to is precisely the
/// reading the labels exist to settle.
pub(super) const LABEL_GAP_PT: f32 = 2.0;

/// How far in from the end of its ruling a frequency label starts, ALONG the
/// depth axis, in points — enough that the label clears the pane edge it is
/// anchored against.
pub(super) const LABEL_INSET_PT: f32 = 2.0;

/// Where on the depth axis the frequency labels ride, and how far they offset
/// back into the spectrum from there — the pair [`Axes::text_anchor`] takes as
/// its depth and its `into`.
///
/// The spectrum's CEILING end, which is the edge its peaks reach, rather than
/// the baseline they stand on. The baseline is the busiest line the pane has:
/// joined, it is the now-line, with the spectrogram's newest column arriving on
/// one side of it and every sounding note's ribbon reaching through it from the
/// other, so a number written there is read against three pictures at once and
/// competes with the one line on the pane that divides two of them. The
/// ceiling end is the quiet one — only a peak at full scale reaches it — and
/// it is the end of the ruling nearest what the number is being looked up
/// for.
///
/// Which screen edge that is flips with the layout, because joining a roll
/// mirrors the curve: hanging off the now-line the peaks point out to depth 0,
/// and standing alone on the outer edge they reach the far end instead. The
/// offset follows, so in both the label sits inside the pane rather than off
/// it.
///
/// Whole-song mode draws no spectrum at all and hands the whole axis to the
/// roll (`split` is 0), so it takes the first arm and its labels ride the near
/// edge — the only edge it has to put them on.
pub(super) fn label_anchor(split: f32) -> (f32, f32) {
    if split < 1.0 {
        (0.0, LABEL_INSET_PT)
    } else {
        (1.0, -LABEL_INSET_PT)
    }
}

/// Point size of an axis marking's label — a dozen standing marks that
/// should stay quiet.
///
/// Quoted at the size the pane is actually read at, so the Marking size bar
/// reads 1 there: the bar is where the size gets dialled, and a built-in nobody
/// opens on turns every view into a correction of it. When the bar settles
/// somewhere other than 1, this number is what moves.
pub(super) const MARKING_PT: f32 = 16.0;

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
/// The names carry a second number that is not a text size at all — the air
/// they keep in front of them, which takes the first factor alone, so that the
/// spacing stays put while the type it stands next to grows.
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
    /// The name written on each ribbon: its type, and separately the air it
    /// keeps in front of it.
    ///
    /// The pair is nested rather than laid out as two fields here so that it
    /// has exactly ONE construction site, in [`text_scales`], with each half
    /// beside the expression that computes it. They are both `f32` and they are
    /// equal in any picture that is not zoomed, so a second site assembling
    /// them would be a swap that draws correctly at the dialled view and
    /// wrongly everywhere else — see [`NameScale`](super::names::NameScale).
    pub(super) names: super::names::NameScale,
}

pub(super) fn text_scales(
    cfg: &crate::SpectrumConfig,
    axes: &Axes,
    span: f32,
    ppp: f32,
) -> TextScales {
    let pane = axes.pitch_len() / REFERENCE_PITCH_LEN;
    TextScales {
        markings: crate::text::snap_scale(pane * cfg.marking_scale, MARKING_PT, ppp),
        names: super::names::NameScale {
            // NOT snapped, unlike the markings above: this is the one size here
            // that follows a zoom, and a zoom is continuous. It is snapped for
            // RASTERIZING where it is drawn (`names::draw`), which is what
            // keeps the atlas bounded without quantizing the picture -- see
            // `crate::text::TextBatch::magnified`.
            label: pane * cfg.note_name_scale * name_zoom(span),
            // The pane alone. Neither the zoom nor the user's bar reaches it:
            // how big a name is drawn is a question about the picture it is
            // written over, and the picture grows with the zoom; how far it
            // stands off the end of the ribbon it names is a question about the
            // reader's eye, which does not. The pane is the one factor common
            // to both, and has to be, or the Render preview and the video it
            // previews would set their names different distances off their
            // notes -- see REFERENCE_PITCH_LEN.
            air: pane,
        },
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
    let (floor, ceiling) = level_window(cfg);
    (db - floor) / (ceiling - floor)
}

/// The dB window the depth axis spans: the Level range bar's floor, and a ceiling
/// held at least [`LEVEL_RANGE_MIN_SPAN`](crate::LEVEL_RANGE_MIN_SPAN) above
/// it.
///
/// Never trust the pair to be ordered or apart, exactly as the pitch range is
/// not trusted: the bar can't produce a collapsed one, a hand-edited state blob
/// can, and dividing by its zero span paints NaN geometry that takes the editor
/// — and with it the host — down.
///
/// One function rather than the pair read twice, because the curve's height and
/// the [`level_grid`] ruled across it are the same axis measured two ways. A
/// grid settling its own idea of where the ceiling is would rule -20 dB
/// somewhere the curve does not put it, which is the one failure a grid cannot
/// survive: it is drawn to be trusted over what it is drawn on.
pub(crate) fn level_window(cfg: &crate::SpectrumConfig) -> (f32, f32) {
    (cfg.floor_db, cfg.ceiling_db.max(cfg.floor_db + crate::LEVEL_RANGE_MIN_SPAN))
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
pub(crate) struct Axes {
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
        if self.time_vertical {
            self.rect.width()
        } else {
            self.rect.height()
        }
    }

    /// Points spanned by the full depth/time axis (the long side).
    ///
    /// Points rather than device pixels, and the distinction is load-bearing:
    /// this is the divisor that turns a size in points ([`PLOT_HEADROOM_PT`])
    /// into a fraction of the axis, so reading it as physical pixels halves
    /// that size on a 2x display. [`roll`](super::roll)'s `MIN_LENGTH_DEVICE_PX`
    /// is what a floor in the other unit looks like, and says so in its name.
    pub fn depth_len(&self) -> f32 {
        if self.time_vertical {
            self.rect.height()
        } else {
            self.rect.width()
        }
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
        if self.time_reversed {
            1.0 - d
        } else {
            d
        }
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

    /// The cursor a drag along the depth axis shows while resizing something
    /// — vertical or horizontal, whichever way the depth axis currently runs.
    pub(super) fn resize_cursor(&self) -> egui::CursorIcon {
        if self.time_vertical {
            egui::CursorIcon::ResizeVertical
        } else {
            egui::CursorIcon::ResizeHorizontal
        }
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
pub(crate) struct PitchScale {
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
/// ladder wears down to the numbered marks alone — a line under every number
/// and nothing else, which is a legible axis rather than a broken one.
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
/// Only the steps `scale` shows come back, and of the UNNUMBERED ones only
/// those with room for themselves ([`MIN_RULING_GAP_PT`]) — a numbered step
/// draws at any spacing, since a label with no line under it is worse than two
/// lines close together. Which of the rest have room turns on the length of a
/// DECADE alone, not on where the decade sits, so it is settled once and the
/// surviving ladder is identical in every decade on the axis.
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

/// The dB steps the level grid may be ruled at, closest first — an analyzer's
/// 1-2-5 series carried across decades, the same discipline
/// [`frequency_grid`] rules the other axis by.
///
/// A ladder rather than the one step, because the level axis is the one axis
/// on this pane whose SPAN is dialled: the Level range bar and the drag across the
/// spectrum move the window between 12 dB and 100 dB while the pane it is drawn
/// on stays the size it is. At the wide end a 10 dB step on a short analyzer
/// closes to a wash, and at the narrow end it rules a single line across a whole
/// picture, which is a grid that has stopped measuring anything.
pub(super) const LEVEL_STEPS_DB: [f32; 7] = [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0];

/// Which rung of [`LEVEL_STEPS_DB`] the grid is ruled at wherever it fits, and
/// the step it names: **10 dB**.
///
/// This is the ladder's home rather than a starting guess. The search below
/// leaves it only when the pane forces the move — coarser when 10 dB lines
/// would crowd, finer when they would stand too far apart to measure the
/// picture between them — so an analyzer at any ordinary size is ruled in tens.
///
/// Only the RULINGS start here. The numbers start from whatever the rulings
/// settled on and thin from there (see [`level_grid`]), because the question
/// they answer is a different one: a line needs room to be a line, and a number
/// needs room to be read.
pub(super) const LEVEL_STEP: usize = 3;

/// The closest two level rulings may be drawn, in points — the ABSOLUTE floor,
/// under the type-relative one below.
///
/// Wider air than the frequency ladder's [`MIN_RULING_GAP_PT`], and the reason
/// is what the two grids are FOR. Frequency rulings crowd at the top of every
/// decade because the ladder is even in frequency and the axis is not, so a
/// crowded stretch is information — it says where the ladder restarts. The
/// level grid is even in the unit its axis is linear in, so its lines are
/// equally spaced by construction and a crowded one says nothing at all: it is
/// simply a mesh laid across the picture, closing into texture. Coarsening to
/// the next rung keeps it a set of lines.
pub(super) const MIN_LEVEL_GAP_PT: f32 = 12.0;

/// What share of a NUMBER's own room two rulings are also held apart by, on top
/// of [`MIN_LEVEL_GAP_PT`].
///
/// This is what makes the grid coarsen as the type grows and not only as the
/// pane shrinks, and the two are the same problem: what makes a number hard to
/// attribute to a line is its size against the line SEPARATION, so the
/// separation has to answer to the size. A grid that only watched the pane
/// would rule tens under numbers set large enough to span three of them.
///
/// A half, so wherever the ladder has a rung at twice the step — which is every
/// rung of a 1-2-5 ladder except the 2 — the numbers land on every ruling or
/// every second one, and at most one unnumbered line stands between two
/// numbers. That is the ambiguity the stronger ink then settles outright (see
/// `RULING_FADE`); past it the grid coarsens instead.
pub(super) const NUMBERED_LINE_SHARE: f32 = 0.5;

/// The furthest apart two level rulings may stand before the grid is
/// subdivided, in points.
///
/// The bound the ZOOM needs. Closing the Level window to its 12 dB minimum
/// leaves a 10 dB step with one line to rule across the whole analyzer, which
/// brackets the picture rather than measuring it — the reading a grid exists
/// for is the one BETWEEN its lines, and there is no between with one line. Set
/// where a 10 dB step stops being a measure of what is drawn across it; below
/// this the ladder subdivides to 5 dB and then 2, and the tens keep the
/// numbers.
pub(super) const MAX_LEVEL_GAP_PT: f32 = 160.0;

/// Most rulings the level grid will return.
///
/// Not a look — it is what keeps a hand-edited state blob from turning a draw
/// into a hundred-million-shape loop. The Level range bar and `sanitize` hold the
/// window inside 100 dB, where the ladder's finest rung yields a hundred lines;
/// a blob that names a floor of -1e9 gets a bounded, meaningless grid instead of
/// a hung editor.
const MAX_LEVEL_RULINGS: i32 = 128;

/// One level ruled across the pitch axis — a rung of the volume grid, running
/// perpendicular to [`frequency_grid`]'s rulings.
#[derive(Clone, Copy, Debug)]
pub(super) struct LevelRuling {
    /// Where it falls on the level axis, `0..1` from the window's floor to its
    /// ceiling. The pane scales this by its depth budget, which is what makes a
    /// ruling land exactly where the curve reaching that level lands.
    pub(super) level: f32,
    /// The level itself, in the dB the Level range bar is quoted in.
    pub(super) db: f32,
    /// Carries a number — and, because it does, the stronger of the pane's two
    /// ruling inks.
    ///
    /// Counted UP FROM THE LOWEST ruling rather than off the round dB values,
    /// so the bottom of the scale is always named: it is the one level a reader
    /// cannot infer from the others, every number above it being measured from
    /// somewhere. Numbering the round multiples instead leaves the lowest line
    /// bare exactly when the numbers thin, which is when the axis most needs its
    /// bottom stated.
    pub(super) numbered: bool,
}

/// The level ladder the analyzer rules and numbers, floor to ceiling: every
/// 10 dB where that fits ([`LEVEL_STEP`]), coarser where the rulings would
/// crowd and finer where they would stand too far apart to measure the picture
/// between them (see [`LEVEL_STEPS_DB`]).
///
/// `level_len` is what the LEVEL axis spans in points — the spectrum's depth
/// budget, not the whole depth axis — and `label_room` the closest two numbers
/// may be set, which the pane measures off the type it is about to draw them in.
/// BOTH densities are decided from those two lengths alone, so the grid answers
/// a pane resize, a Level zoom and a Marking-size change by one rule. The
/// rulings answer to the type as well as to the pane (see
/// [`NUMBERED_LINE_SHARE`]): a number is hard to attribute when it is large
/// against the line separation, and coarsening the lines is the half of that the
/// numbers cannot fix for themselves.
///
/// The FLOOR is ruled where it lands on the ladder, and being the lowest rung
/// it is then always numbered — so a window of -60 to -20 states its own bottom
/// rather than trailing off after -50 and leaving the reader to work out where
/// the scale ends. Where the floor does not land on a rung there is nothing
/// round to write, and the lowest ruling above it is what carries the number
/// instead.
///
/// Joined, that rung falls exactly on the now-line, which draws over it at full
/// strength — so what it costs is one covered shape and no ink, against a number
/// that would otherwise point at nothing. Standing alone the spectrum has no
/// now-line and the rung is its baseline, which is the line it should have had.
///
/// The CEILING is not ruled at either. It is the edge the picture stops at,
/// already stated by the pane ending there, and it is the end the frequency
/// numbers ride along (see [`label_anchor`]) — so a number on it is written into
/// the one corner of this pane that is already spoken for.
///
/// What the numbers mean is the window's own dB, which is the reading the Level
/// bar is quoted in and the one the curve is drawn against. With a Tilt dialled
/// in that is not the raw dB of the bucket under it — the tilt is a slope the
/// display SUBTRACTS per octave (see [`loudness_raw`]), so a ruling is one level
/// of the tilted picture and the untilted dB it stands for slides with pitch.
/// That is the axis the pane actually has: the alternative is a grid that bends
/// across the pitch axis, drawn against a curve that does not bend with it.
pub(super) fn level_grid(
    cfg: &crate::SpectrumConfig,
    level_len: f32,
    label_room: f32,
) -> Vec<LevelRuling> {
    let (floor, ceiling) = level_window(cfg);
    let span = ceiling - floor;
    // The same guard `frequency_grid` opens with, against the same NaN: `span`
    // is what a level's position divides by, and egui panics on NaN geometry.
    // A zero-length axis is not a crash so much as nothing to rule.
    if !span.is_finite() || span <= 0.0 || !level_len.is_finite() || level_len <= 0.0 {
        return Vec::new();
    }
    let gap = |step: f32| step / span * level_len;

    // Down the ladder while the rulings crowd, then up it while they stand too
    // far apart.
    //
    // Ordinarily one of the two runs and the other finds nothing to do. They can
    // BOTH run, and the order is what settles it when they do: type large enough
    // that `closest` passes `MAX_LEVEL_GAP_PT` asks for a rung the sparse bound
    // will not have, and the second loop walks back off it. The sparse bound
    // wins because losing it costs a rung so coarse it rules nothing across the
    // picture, where losing the other costs a crowded grid under numbers that
    // thin hard — and a grid that rules nothing is not a grid.
    // `the_sparse_bound_wins_where_the_type_wants_more_room_than_the_axis_has`
    // holds the case, which is off the end of what any real Marking size
    // reaches.
    // Multiples of a step across the window, taken as whole counts so the ladder
    // is the same set of levels wherever the window has been dragged to, rather
    // than a phase of it. The floor is IN when it lands on one and the ceiling
    // is always out — see the ends above.
    let ends = |step: f32| {
        let rungs = floor / step;
        let first = if (rungs - rungs.round()).abs() < 1e-4 {
            rungs.round() as i32
        } else {
            rungs.floor() as i32 + 1
        };
        let last = ((ceiling / step).ceil() as i32 - 1).min(first + MAX_LEVEL_RULINGS - 1);
        (first, last)
    };

    let closest = MIN_LEVEL_GAP_PT.max(label_room * NUMBERED_LINE_SHARE);
    let mut rung = LEVEL_STEP;
    while rung + 1 < LEVEL_STEPS_DB.len() && gap(LEVEL_STEPS_DB[rung]) < closest {
        rung += 1;
    }
    // The sparse bound is TWO bounds, and the second is the one the first
    // cannot state. `gap` is a step's nominal spacing and says nothing about
    // how many multiples of it land inside the window, so a step near or past
    // the span itself passes `MAX_LEVEL_GAP_PT` comfortably while the window
    // holds a single rung — the exact picture that bound exists to forbid, a
    // line bracketing the analyzer rather than measuring it. The count is what
    // the reading actually needs, so the walk-back tests both.
    //
    // It always terminates with a grid: `level_window` holds the span at
    // `LEVEL_RANGE_MIN_SPAN` (12 dB) or wider, and the ladder's finest rung is
    // 1 dB, so rung 0 has eleven rulings in the narrowest window there is.
    // `a_ruled_level_axis_always_has_a_between` sweeps it.
    let sparse = |step: f32| {
        let (first, last) = ends(step);
        gap(step) > MAX_LEVEL_GAP_PT || last - first < 1
    };
    while rung > 0 && sparse(LEVEL_STEPS_DB[rung]) {
        rung -= 1;
    }
    let step = LEVEL_STEPS_DB[rung];

    // The numbers go on a MULTIPLE of that step, never a rung beside it: a
    // label wants more room than a line and gets it by numbering one line in
    // two or five, which is the one way of thinning that cannot leave a number
    // written where no line was drawn. The ladder is 1-2-5, so the multiples of
    // a rung are themselves rungs and every step stays a round dB number.
    //
    // The FINEST multiple with room, so the numbers are as dense as the type
    // allows rather than pinned to tens. Both directions of the zoom need that:
    // the Level window closed to 12 dB has one ten in it, which is a number and
    // no axis, and a tall analyzer ruled in fives has the room to name them.
    let divides = |s: f32| ((s / step) - (s / step).round()).abs() < 1e-3;
    let mut labels = step;
    for &candidate in LEVEL_STEPS_DB.iter().filter(|&&s| s >= step && divides(s)) {
        labels = candidate;
        if gap(candidate) >= label_room {
            break;
        }
    }

    let (first, last) = ends(step);
    // ...and the numbers every `stride` rulings COUNTED UP FROM THE LOWEST, so
    // the bottom of the scale is named whatever the window has been dragged to.
    // Numbering the round multiples of `labels` instead reads the same on a
    // window whose floor happens to sit on the ladder and leaves the lowest line
    // bare on every other one — the -60..-20 default among them, where numbers
    // every 20 dB would name -40 and nothing else.
    let stride = (labels / step).round().max(1.0) as i32;
    (first..=last)
        .map(|k| {
            let db = k as f32 * step;
            let level = (db - floor) / span;
            // Every number is set on the CEILING side of its ruling, so the one
            // end that can run out of room is that one — and a ruling can sit
            // hard against it, the ceiling being wherever the Level range bar was
            // dragged rather than a rung. A number with no room there is not
            // written rather than set somewhere else: which side they sit on is
            // what makes a column of them readable at a glance, and one number
            // in the picture answering to a different rule costs more than the
            // number is worth.
            //
            // The lowest rung is exempt, because the bottom of the scale is
            // stated whatever it costs — and it is the ruling FURTHEST from the
            // ceiling, so the exemption only ever fires on an analyzer too small
            // to hold a single number.
            let numbered = (k - first) % stride == 0
                && (k == first || (1.0 - level) * level_len >= label_room);
            LevelRuling { level, db, numbered }
        })
        .collect()
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
                // The take's own, floored there rather than here: the trim
                // that feeds the heatmap reads the same method, and a second
                // copy of this floor is what let the two disagree.
                window: ws.window(),
                origin: ws.start,
                now,
                whole_song: true,
            },
            None => TimeAxis {
                split,
                depth_span,
                // The same floor on the live arm's own input, for the same
                // reason — a window of nothing maps every time to one depth.
                window: (state.spectrum_config.roll_seconds as f64)
                    .max(crate::WholeSong::MIN_WINDOW),
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

    /// This axis' own share of the depth axis, in points: `depth_span` (the
    /// far region's fraction of it, live or whole-song alike) of what the
    /// full axis spans.
    pub(super) fn region_depth_len(&self, axes: &Axes) -> f32 {
        axes.depth_len() * self.depth_span
    }

    /// Seconds per point along the depth axis, in the region this axis owns
    /// — the rate a screen-space length converts through to reach take time.
    /// Floored at one point, so a region squeezed to nothing returns a large
    /// but finite rate rather than dividing by zero.
    pub(super) fn seconds_per_point(&self, axes: &Axes) -> f64 {
        self.window / f64::from(self.region_depth_len(axes).max(1.0))
    }
}
