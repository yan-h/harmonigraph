//! The visual-style settings the view config carries: the enums it selects
//! between, with the shader indices they map to, and the gradient's six knobs.
//! Adding a style means touching this file and the matching branch in
//! `lattice.wgsl`; the gradient reaches the shader as a color table instead,
//! and needs no branch there at all.

/// A low-to-high color gradient, as six things a reader of the picture can
/// name: WHICH colors it walks through (an arc of the OKLAB hue circle), how
/// bright the middle of the range sits, how much brightness separates the ends
/// of it, how much color the middle carries, and how much color separates the
/// ends. The curve those describe is in [`color`](crate::color), which authors
/// it in two spaces at once and says why.
///
/// **What the range IS belongs to whoever holds one**, and two things in the
/// picture do. `ViewConfig::pitch_gradient` spans the lattice's color range, so
/// its low end is the darkest pitch and its high end the brightest — that is
/// the gradient the discs, the octave glyphs, the trail and the piano roll's
/// ribbons all read. The Spectral pane's heatmap holds a second one spanning
/// the analyzer's Level, where low is silence and high is a full-scale bucket.
/// Everything below is stated in "the bottom of the range" and "the top" for
/// that reason: the curve is the same object either way, and nothing in it
/// knows which axis it was handed.
///
/// The six say six independent things on purpose, and that is the point of
/// shaping the setting this way rather than as a list of named palettes:
/// brightness is a far stronger cue than hue, so how much of it a gradient
/// SPENDS on the range is the decision worth having a knob for — and color is
/// worth the same question, which is why the two are shaped alike, a middle and
/// a signed ramp each. Where two of them meet it is a limit rather than a
/// meaning, and it is the same limit twice: a ramp opens either side of its
/// middle, so what that middle leaves on its own axis is all the ramp there is
/// room for, and a middle pinned at either end of that axis leaves none.
///
/// At [`lightness_ramp`](Self::lightness_ramp) 0 the gradient is exactly
/// isoluminant — `L*` is a function of luminance alone, so one `L*` is one
/// screen brightness, and the bottom of the range then reads as bright as the
/// top with hue carrying the difference by itself. Wind the ramp up and
/// brightness takes over; wind it negative and the picture inverts. At
/// [`chroma_ramp`](Self::chroma_ramp) 0 everything carries the same color as
/// everything else, hue and lightness aside, which is the picture a single
/// Chroma knob could draw and the only one it could; wind that ramp up and the
/// top of the range goes vivid against a washed-out bottom, and negative puts
/// the color at the bottom.
///
/// Nothing here can leave the sRGB gamut, whatever the six are set to, which
/// is what makes them safe to expose as free knobs — see
/// [`chroma`](Self::chroma).
///
/// A gradient is a view setting rather than a param: it says what the picture
/// MEANS, and the lattice's reaches the shader as the contents of the pitch LUT
/// (see [`pitch_ramp_lut`](crate::pitch_ramp_lut)), so `lattice.wgsl` never
/// learns any of it exists.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
// Container-level, like every other persisted struct: `impl Default` is the
// one source of a field's fallback, so a blob carrying a PARTIAL gradient
// picks the rest up from there rather than from a second set of values.
#[serde(default)]
pub struct Gradient {
    /// OKLAB hue the BOTTOM of the range takes, in degrees. Wrapped into
    /// 0..360 by [`sanitized`](Self::sanitized), since it names a point on a
    /// circle and every value is therefore a legal one.
    ///
    /// Oklab and not CIELAB: a CIELAB hue angle does not hold its hue as the
    /// other knobs turn, and drifts hardest through the blues this opens on.
    ///
    /// An angle here is Oklab with nothing marking it as such, which is what
    /// makes the choice of space a one-way door: a stored angle cannot say
    /// which space it was written in. Anything ADDED here owes an entry in
    /// `impl Default` below, which the container-level `default` reads.
    pub hue_start: f32,
    /// How far round the hue circle the range walks, in degrees — SIGNED, so
    /// its sign is which way round the circle the range runs and flipping it
    /// reverses the spectrum without touching where it starts.
    ///
    /// Not a second endpoint, which is the same information but the wrong
    /// shape for this: two endpoints on a circle name two arcs and cannot say
    /// which one is meant, and neither can they reach past a full turn. A
    /// signed span says both, and 0 collapses the gradient to a single hue —
    /// a legal setting, where chroma and brightness carry the range alone.
    ///
    /// Oklab degrees, for the reason [`hue_start`](Self::hue_start) gives —
    /// including what that means for a span sitting in an already-saved blob.
    pub hue_span: f32,
    /// `L*` at the CENTRE of the range, 0..100 — the middle of the
    /// gradient rather than either end, so that
    /// [`lightness_ramp`](Self::lightness_ramp) opens symmetrically about it
    /// and this knob keeps meaning "how bright is the picture" at every ramp.
    pub lightness: f32,
    /// Signed `L*` difference from the bottom of the range to the top: how
    /// much of the gradient's separation is spent on brightness. 0 is exactly
    /// isoluminant, negative puts the bright end at the bottom.
    ///
    /// Bounded by what [`lightness`](Self::lightness) leaves on the axis rather
    /// than by a constant of its own — the whole 100 points at the middle,
    /// closing to nothing at either end — so that both ends of the gradient are
    /// `L*` the axis actually holds. See [`sanitized`](Self::sanitized).
    pub lightness_ramp: f32,
    /// How much color the CENTRE of the range carries, as a fraction
    /// 0..1 of what the sRGB gamut holds for EVERY hue at that lightness — NOT
    /// an absolute chroma, and not a share of what this particular hue could
    /// hold, which is a different number at every hue. The middle rather than
    /// either end, so that
    /// [`chroma_ramp`](Self::chroma_ramp) opens symmetrically about it and this
    /// knob keeps meaning "how colorful is the picture" at every ramp.
    ///
    /// The gamut is the reason it is a fraction. At a fixed `L*` sRGB admits a
    /// different maximum Oklab chroma at every hue (0.115 at the tightest point
    /// of the default arc, 0.320 at its widest), and that maximum collapses
    /// toward 0 as `L*` approaches either end of its own range. An absolute
    /// chroma would therefore be a number whose meaning changes under the other
    /// knobs: past what the gamut holds a channel clips, which bends the hue
    /// and MOVES THE LUMINANCE — so the isoluminance promised above would
    /// quietly stop being true — and the top of the control would be dead over
    /// whatever part of the arc had already run out. Denominated in what is
    /// actually available, every setting is reachable, monotone, and in gamut
    /// by construction.
    ///
    /// **What it is a fraction OF is the whole question**, and a fraction of
    /// the per-HUE ceiling is the wrong answer: it draws one setting at 2.4x
    /// the chroma in magenta that it draws in the teal-blues, 3.5x at `L*` 42
    /// and 4.8x at 86, because the sRGB solid is far wider at some hues than
    /// others. That is not a subtlety — it is the picture reading as though the
    /// magentas and oranges had been turned up, and Ottosson names the same
    /// defect in HSLuv, which is built exactly that way.
    /// `chroma_of`, in [`color`](crate::color), resolves it instead against a
    /// floor every hue can hold, opening flat across hue
    /// and reaching each hue's own ceiling only at 1.0. See there for the shape
    /// and what each end of it costs.
    ///
    /// What no denominator can design away is the LIGHTNESS term:
    /// the gamut genuinely closes toward black and white, so a gradient
    /// spending `L*` on its range is less colorful at the bright end whatever
    /// this knob says. On the default arc that is most of the variation left.
    pub chroma: f32,
    /// Signed difference in that fraction from the bottom of the range to the
    /// top: how much of the gradient's separation is spent on COLOR. 0 asks the
    /// same fraction of every sample, negative puts the vivid end at the bottom.
    ///
    /// Bounded by what [`chroma`](Self::chroma) leaves on the 0..1 axis rather
    /// than by a constant of its own — the whole of it at the middle, closing
    /// to nothing at either end — which is
    /// [`lightness_ramp`](Self::lightness_ramp)'s bound on its own axis, for
    /// the reason given there: a ramp steeper than that runs off the axis and
    /// flattens, drawing a PLATEAU over part of the range while the pair
    /// still reads as a straight ramp. See [`sanitized`](Self::sanitized).
    ///
    /// A ramp in the FRACTION and not in absolute chroma, for everything
    /// [`chroma`](Self::chroma) says: the maximum moves along the arc and with
    /// the brightness ramp, so a ramp stated absolutely would be a stretch
    /// whose two ends changed meaning under the other knobs — and one whose
    /// vivid end would sit outside the gamut over whatever part of the arc
    /// could not hold it.
    pub chroma_ramp: f32,
}

/// The hue the bottom of the range opens on: a deep blue-violet.
///
/// Oklab 246.9 is the angle that names the color CIELAB 260 draws at this arc's
/// own bottom (`L*` 42, half chroma) — the two spaces disagree by 13 degrees
/// here, so this is the same opening color under a different name rather than a
/// different one. `the_defaults_are_the_retired_arc_converted` holds it to that.
fn default_hue_start() -> f32 {
    246.9
}

/// Just over half a turn, running blue-violet up through magenta and red to
/// yellow-green. Wide enough that no two octaves of the range share a hue, and
/// short of the full circle so the two ends cannot be confused for each other.
///
/// The SAME ARC that CIELAB names as a span of 190: it ends on Oklab 89.7,
/// which CIELAB calls 90 at `L*` 86, and Oklab simply spends more degrees
/// crossing the blues than CIELAB does. The extra 12.8 degrees are that
/// disagreement between the two axes and not a wider sweep.
fn default_hue_span() -> f32 {
    202.8
}

/// Mid-range brightness, placed with [`default_lightness_ramp`] so the
/// gradient opens at `L*` 42 and closes at 86. That is a 5.4x span in screen
/// luminance from the bottom of the range to the top — a strong cue, and
/// the reading of the picture the defaults are meant to open on. Isoluminant
/// is one bar-drag away.
fn default_lightness() -> f32 {
    64.0
}

/// The 44 points of `L*` that open the span above, split either side of the
/// mid-range brightness.
fn default_lightness_ramp() -> f32 {
    44.0
}

/// Two thirds of the way up a knob whose bottom is flat across hue and whose
/// top is each hue's own ceiling. Enough that the hues read as colors, and
/// short of the boundary, where the ceiling's own kinks between the sRGB
/// primaries come back as bumps in the sweep — which is what the top of the
/// knob is for, not what it should open on.
///
/// The figure holding the default arc's MEAN colorfulness where a fraction of
/// the per-hue ceiling at 0.5 holds it, so the picture opens about as colored as
/// that alternative draws it and spends the color evenly rather than banking it
/// in the magentas. `the_default_opens_at_the_colorfulness_it_used_to` measures
/// it, and measures `ViewConfig`'s own chroma beside it — the two are
/// independent numbers and a retune of this one does not reach that one.
fn default_chroma() -> f32 {
    0.669
}

/// Flat: every note asks for the same fraction, so what separates two notes is
/// their hue and their brightness rather than how colored they are.
///
/// Where [`default_lightness_ramp`] opens wide, and the difference is what the
/// two cues are worth. Brightness is the strongest separation there is, so the
/// picture the defaults open on spends it on the range; color is already
/// carrying the range through the hue arc, and a chroma ramp on top of that arc says the
/// same thing twice while costing one end of the range its color. It is a knob
/// to dial rather than one to open on.
fn default_chroma_ramp() -> f32 {
    0.0
}

impl Default for Gradient {
    fn default() -> Self {
        Gradient {
            hue_start: default_hue_start(),
            hue_span: default_hue_span(),
            lightness: default_lightness(),
            lightness_ramp: default_lightness_ramp(),
            chroma: default_chroma(),
            chroma_ramp: default_chroma_ramp(),
        }
    }
}

impl Gradient {
    /// Widest span either way round the circle: a full turn, which is every
    /// arc there is. Past it the gradient would revisit hues it already used.
    pub const MAX_HUE_SPAN: f32 = 360.0;

    /// Fit the six to what their controls can produce. A bar cannot make a
    /// nonsense value but a hand-edited RON can, and these feed a color
    /// conversion whose output goes straight into the instance buffer: a
    /// non-finite `L*` would ride out as a NaN color rather than announce
    /// itself, and it would also break the LUT memo's key, which compares
    /// gradients for equality.
    ///
    /// Applied at the table (see [`pitch_ramp_lut`](crate::pitch_ramp_lut)) as
    /// well as on the way in from a blob, so the guarantee holds for a
    /// gradient assembled in code too.
    pub fn sanitized(self) -> Gradient {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        let hue_span = finite(self.hue_span, default_hue_span())
            .clamp(-Self::MAX_HUE_SPAN, Self::MAX_HUE_SPAN);
        let lightness = finite(self.lightness, default_lightness()).clamp(0.0, 100.0);
        // The widest ramp that keeps BOTH ends of the gradient on the axis,
        // the ends being `lightness ± ramp/2`: the whole axis at its middle,
        // and nothing at all at either end.
        let widest_ramp = 2.0 * lightness.min(100.0 - lightness);
        // The same statement about the chroma pair, on the 0..1 axis a fraction
        // of the gamut lives on. One rule with two axes rather than two rules:
        // what the middle leaves is all the ramp there is room for.
        let chroma = finite(self.chroma, default_chroma()).clamp(0.0, 1.0);
        let widest_chroma_ramp = 2.0 * chroma.min(1.0 - chroma);
        Gradient {
            // Wrapped rather than clamped: it names a point on a circle.
            hue_start: finite(self.hue_start, default_hue_start()).rem_euclid(360.0),
            // A span of zero has no direction, so it is written with the one
            // sign that reads as none. `-0.0` is the sign that lies: it is not
            // `< 0.0`, so everything that asks which way the arc runs takes it
            // for positive, while `{:+}` prints it as "-0" — a bar that says
            // one direction and behaves as the other. Flipping a zero span and
            // dragging a flipped one down to nothing both produce it.
            hue_span: if hue_span == 0.0 { 0.0 } else { hue_span },
            lightness,
            // Against what the CENTRE leaves rather than against the axis, so
            // the gradient's ends are `L*` and not a pair of numbers the axis
            // has to catch. A steeper ramp runs off the end and flattens there,
            // which draws a PLATEAU over part of the range while the pair
            // still reads as a straight ramp — the picture and the numbers
            // saying different things — and the control that sets the pair, a
            // middle with a handle either side of it, has nowhere to put a
            // handle that has left the bar.
            lightness_ramp: finite(self.lightness_ramp, default_lightness_ramp())
                .clamp(-widest_ramp, widest_ramp),
            chroma,
            // Against what the middle leaves for the same reasons, one axis
            // over: a fraction past 1 is a color the gamut cannot hold — which
            // is the one thing the whole design promises it never asks for —
            // and a fraction under 0 is a chroma with a sign, which draws the
            // hue on the far side of the circle from the one the arc names.
            chroma_ramp: finite(self.chroma_ramp, default_chroma_ramp())
                .clamp(-widest_chroma_ramp, widest_chroma_ramp),
        }
    }

    /// The same arc, run the other way round the circle: the same colors and
    /// the same width, low and high swapped.
    ///
    /// The far end becomes the near one, which is what keeps the arc where it
    /// was. Negating the span alone would hold the START still and swing the
    /// arc off into hues the gradient never had — the same picture the Flip
    /// button promises is emphatically not that, so the two cannot be left as
    /// separate spellings of "flip" in the pane and in a test.
    ///
    /// Sanitized on the way out, which is what stops a flipped zero span coming
    /// back as `-0.0`: a span of nothing has no direction to be flipped.
    pub fn flipped(self) -> Gradient {
        let g = self.sanitized();
        Gradient {
            hue_start: (g.hue_start + g.hue_span).rem_euclid(360.0),
            hue_span: -g.hue_span,
            ..g
        }
        .sanitized()
    }

    /// `L*` and hue at normalized height `t` (0 at the bottom of the range, 1
    /// at the top). The two curves the gradient is made of, kept
    /// together because the chroma available to a sample is a function of
    /// BOTH — which is why the widget that previews a gradient and the table
    /// that draws it must walk it the same way.
    ///
    /// Callers get a sanitized gradient's curve or nothing: `t` outside 0..1
    /// is clamped, so this is defined everywhere.
    pub fn lightness_and_hue(self, t: f64) -> (f64, f64) {
        let g = self.sanitized();
        let t = t.clamp(0.0, 1.0);
        let l = f64::from(g.lightness) + (t - 0.5) * f64::from(g.lightness_ramp);
        let h = (f64::from(g.hue_start) + t * f64::from(g.hue_span)).rem_euclid(360.0);
        // The clamp cannot fire for a sanitized gradient, and it is worth
        // knowing why rather than assuming it might: the widest ramp is
        // `2 * min(l, 100 - l)`, whose every step is exact in f32 — `100 - l`
        // by Sterbenz for the half that needs it, the doubling and the halving
        // by their exponents — so an end lands exactly ON 0 or 100 and the
        // widening to f64 adds nothing. What the clamp keeps is the guarantee
        // itself, for a caller assembling a gradient in code and reaching this
        // through some later path that does not sanitize.
        (l.clamp(0.0, 100.0), h)
    }

    /// The chroma FRACTION at normalized height `t` — what the curve asks for
    /// before `chroma_of` resolves it against the gamut, which is what
    /// [`chroma`](Self::chroma) says at the middle of the range and
    /// [`chroma_ramp`](Self::chroma_ramp) spreads over the rest of it.
    ///
    /// A fraction and not an absolute chroma, so this is only half a
    /// coordinate: what it is a fraction OF depends on the `L*` and hue at the
    /// same `t`, which is why the curve is resolved in `ramp_coords` and
    /// not here.
    ///
    /// The one curve read that does NOT sanitize first, where
    /// [`lightness_and_hue`](Self::lightness_and_hue) does, and the asymmetry
    /// is deliberate: a fraction past 1 is the only ask that can land outside
    /// the gamut, so it is the only failing case `ramp_sample_in_gamut` has to
    /// prove itself against, and a clamp here would answer that check instead
    /// of letting it ask. Nothing loses by it — every drawing path arrives
    /// through `with_lut`, which sanitizes once for a whole table and keys the
    /// memo on the result.
    ///
    /// A sanitized gradient needs no clamp anyway, for exactly the reason
    /// `lightness_and_hue`'s cannot fire: the widest ramp is `2 * min(c, 1 - c)`,
    /// whose every step is exact in f32 — `1 - c` by Sterbenz for the half that
    /// needs it, the doubling and the halving by their exponents — so an end
    /// lands exactly ON 0 or 1 and the widening to f64 adds nothing. Measured
    /// across 100k chromas at the widest ramp both signs: the overshoot is zero,
    /// and `neither_end_of_the_curve_leaves_the_chroma_axis` is what keeps it
    /// there.
    pub fn chroma_at(self, t: f64) -> f64 {
        f64::from(self.chroma) + (t.clamp(0.0, 1.0) - 0.5) * f64::from(self.chroma_ramp)
    }
}

/// Which shimmer the melody/bass rings run: one sheet of soft light laid over
/// the lattice, in the pattern this names, or [`Off`](Pulse::Off) for the
/// steady picture.
///
/// Every live mode is the same animation with a different shape to it, which
/// is what lets one set of knobs size all of them
/// ([`ViewConfig::shimmer_speed`](crate::ViewConfig::shimmer_speed) and the
/// three beside it). They share more than the knobs: the sheet is ONE field
/// spanning the whole lattice rather than a copy per node — every node
/// samples it at its own place on the plane the billboards face — so the
/// light reads as raking over the picture instead of as many small identical
/// animations.
///
/// The sheet also reaches OUT of the ring layer, onto the octave slice each
/// ring points at — a mark being the ring together with the octave it names.
/// That reach is the whole of what it touches outside the rings: the octave
/// glyphs draw steady everywhere no ring points, which is what keeps them
/// readable as which octaves sound. All of it lives in `lattice.wgsl`'s
/// Shimmer section.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Pulse {
    /// Steady — no animation, the look every earlier build drew.
    #[default]
    Off,
    /// Parallel bands laid diagonally, travelling along their own normal: one
    /// grating, and the plainest reading of light passing over the lattice.
    Bands,
    /// Two gratings crossed at right angles and multiplied, which is a
    /// checkerboard with the corners rounded off: cells of light and cells of
    /// dark, swapping as the sheet slides a half cell.
    Checker,
    /// Three gratings sixty degrees apart and summed — the hexagonal answer
    /// to [`Checker`](Pulse::Checker), a honeycomb of bright cells.
    ///
    /// It tessellates where a checkerboard fights the picture: the lattice's
    /// own rows run along three directions, not two, so a hex sheet lands
    /// with them instead of across them, and a hexagon's neighbours are all
    /// edge-to-edge where a square's touch at the corners.
    Hex,
    /// The same two crossed gratings as [`Checker`](Pulse::Checker) with the
    /// BRIGHTER taken instead of the product: a lattice of light lines rather
    /// than of cells, crossing at bright knots.
    Weave,
    /// Concentric rings travelling outward from the lattice's origin — the
    /// one pattern with a center, and so the one that says where the light is
    /// coming from rather than only which way it goes.
    Rings,
}

impl Pulse {
    /// Index the shader reads (`misc6.w` — see `Uniforms` in
    /// harmonigraph-render). 0 is the steady layer and every other value picks
    /// a pattern out of `shimmer_terms`.
    pub fn shader_index(self) -> u32 {
        match self {
            Pulse::Off => 0,
            Pulse::Bands => 1,
            Pulse::Checker => 2,
            Pulse::Hex => 3,
            Pulse::Weave => 4,
            Pulse::Rings => 5,
        }
    }

    /// Whether this mode lays a sheet over its layer at all — everything but
    /// [`Off`](Pulse::Off). What the UI grays the shared Shimmer knobs on,
    /// and what the shader's identity return tests.
    pub fn sweeps(self) -> bool {
        self != Pulse::Off
    }
}

/// What text an OFF-SHEET node's label carries — a node on any sevens sheet
/// but the center one.
///
/// This used to exist because the name was WRONG off the home sheet.
/// [`LatticePos::note_name`](harmonigraph_core::LatticePos::note_name) walks the
/// chain of fifths with `1 + threes + fives*4 - sevens*2`, and nothing added
/// a septimal mark, so every sevens step spelled exactly like two fifths
/// down: `(0,0,1)` and `(-2,0,0)` were both `B♭`, 27 cents apart. A name
/// that appeared three times at three pitches was not merely uninformative;
/// it asserted something false, and the alternatives here were ways of not
/// saying it.
///
/// The name now carries a septimal mark, so it is true on every sheet and
/// [`Name`](SevensLabel::Name) is the default again. What is left is a
/// choice of how much a small off-sheet node should say, which is a look.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SevensLabel {
    /// The note name, which off the home sheet now carries the septimal
    /// mark that tells it from its namesake.
    #[default]
    Name,
    /// The pitch class in cents under the current tuning, alone. Says what
    /// the node is and nothing it isn't, at the cost of saying where it is.
    Cents,
    /// No text at all. The octave band, the marks and the color carry the
    /// node; text is what the home sheet gets and the sevens layer does
    /// without.
    None,
}
