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

/// WHICH nodes carry a note-name label, once labels are on at all
/// ([`ViewConfig::show_labels`](crate::ViewConfig::show_labels)).
///
/// One axis, and it is how far a name reaches past the note that put it
/// there. Nothing here changes what a label says or how big it draws — those
/// are the rest of the Labels section — only which nodes get one.
///
/// [`Past`](NoteNames::Past) is the trail, and the only mode with a history
/// behind it: it is the one that reads
/// [`NodeInstance::trail`](crate::NodeInstance::trail), so it is also the
/// only one Clear note names means anything under. See the
/// [`trail`](crate::trail) module for why a memory is drawn in TYPE and
/// nothing else.
///
/// A HOVERED node is named under all three. Pointing at a node to ask what it
/// is belongs to the pointer rather than to this setting, and a mode that
/// refused it would leave the lattice with no way to answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum NoteNames {
    /// Every node on screen, played or not — the lattice read as a map.
    /// Every name is equally readable, unplayed and sounding alike; what
    /// says which note is playing is the NODE under the name, not the name.
    All,
    /// Every node the music has visited, and it never forgets: a whole
    /// piece's territory accumulates as it plays.
    #[default]
    Past,
    /// Only what is sounding now. The lattice at rest carries no text.
    Played,
}

/// How many σ out a blurred cell is padded, and how far its kernel reaches —
/// `REACH` in shadow.wgsl and `REACH_SIGMAS` in `harmonigraph-render`'s
/// `shadow.rs`, pinned to both by
/// `the_blurs_reach_and_loop_bound_are_the_packers_own`.
///
/// Here as well as there because a kernel's reach is what every QUAD is grown
/// by, and which kernel a caster is drawn by is the scene's to say.
pub const REACH_SIGMAS: f32 = 3.0;

/// The FLOOR on how many Shadow widths out the standoff's curve is windowed to
/// nothing, and so on how far a [`ShadowKernel::Distance`] cell is padded —
/// [`shadow_stop`] is what solves for the radius a given
/// [`ShadowStyle::falloff`] actually needs, and this is what it never goes
/// below.
///
/// A decay has no radius at which it stops and a quad does, so this is where
/// that difference is settled rather than left to the billboard: the coverage
/// is exactly 0 at the radius the quad is sized to, and there is no fragment
/// where the two disagree. Left to the quad, the tail is cut at whatever value
/// it happens to hold there — a hard step at a SCREEN-ALIGNED square, which is
/// the worst closed contour a smooth field can carry.
///
/// TWO, where the plain exponential — the falloff at 1, and every falloff
/// above 0.64 sooner — is under half a code value ([`SHADOW_INVISIBLE`]) of
/// the deepest shadow before the window closes it. `SHADOW_STOP` in
/// common.wgsl is the shader's copy, pinned by
/// `the_shaders_distance_kind_and_window_are_the_packers`.
pub const SHADOW_STOP: f32 = 2.0;

/// How far down the standoff's decay is carried by one Shadow width, as the
/// exponent of the decay there: four e-folds, so a distance shadow stands at a
/// fiftieth of its own depth at the width the bar names and spends the rest
/// inside it.
///
/// Four rather than the `ln(255)` that would put the width's edge under one
/// code value. A decay steep enough to be invisible there is steep enough to
/// spend the whole shadow in the first fifth of it, which is the abrupt shadow
/// the family is here to not draw; what stands at the edge instead is a
/// fiftieth, on a curve still falling. `SHADOW_TAIL` in common.wgsl is the
/// shader's copy.
pub const SHADOW_TAIL: f32 = 4.0;

/// What counts as gone: the share of the deepest shadow under which a decay is
/// half a code value on an 8-bit target, and so the value a cell may be cut off
/// at without the cut being visible.
///
/// The threshold [`SHADOW_STOP`] was chosen against, named here because
/// [`shadow_stop`] now solves for a radius rather than checking a fixed one.
pub const SHADOW_INVISIBLE: f32 = 0.5 / 255.0;

/// The falloff at and above which a group's shadow costs exactly what it always
/// did: where [`shadow_stop`]'s solve meets the [`SHADOW_STOP`] floor, so the
/// cell is padded and the window shuts precisely as they did before the bar
/// existed.
///
/// `ln(SHADOW_INVISIBLE_FOLDS) / ln(SHADOW_STOP)`, which is the falloff whose
/// decay reaches [`SHADOW_INVISIBLE`] at exactly [`SHADOW_STOP`] widths. A
/// literal because `ln` is not available in a `const fn`;
/// `every_falloff_shuts_its_window_on_nothing` holds it to the algebra.
///
/// Named because the whole cost story turns on it — the bar is free above it
/// and buys atlas below it — and because it is the branch both spellings of
/// `shadow_stop` take, rather than computing a `pow` whose answer the floor
/// then discards.
pub const SHADOW_FALLOFF_FREE: f32 = 0.640_253_25;

/// The bottom of [`ShadowStyle::falloff`], and what the shader floors a
/// caster's own at (`SHADOW_FALLOFF_FLOOR` in common.wgsl).
///
/// Set by DIMINISHING RETURNS against the padding, which is the honest reason
/// once [`shadow_stop`] makes the cell follow the exponent. A falloff under
/// 0.64 carries the decay past the fixed [`SHADOW_STOP`], so the cell grows to
/// hold it: 1.5× the pad at 0.5, 1.8× at 0.35, and 3.0× by 0.25. What that buys
/// stops growing long before the cost does — the bar's whole point is the near
/// field, and a tenth of a width out the decay moves 0.45 → 0.20 over
/// 0.7..=0.4 and only 0.20 → 0.11 over the whole of 0.4..=0.25, for four times
/// the atlas.
///
/// 0.35, which is past the useful travel rather than short of it, at 1.8× the
/// pad and 3.2× the cell AREA for a group dialled all the way down. Nothing
/// above 0.64 pays anything at all.
///
/// This exponent is not new. `glow_shadow_shape` was the same `pow(u, ·)`,
/// removed in #563 as redundant with the Shadow curve — which #582 then removed
/// in turn on finding it was the identity at its own fresh value, so what the
/// pair left behind was no falloff control at all. That bar also ran down to
/// 0.25, and its own [`SHADOW_STOP`] doc conceded what it cost: the cell did
/// NOT follow, so the window cut a tail still standing at 0.9% and drew the
/// closed contour the family exists to avoid. The difference here is
/// [`shadow_stop`], not a braver floor.
pub const SHADOW_FALLOFF_MIN: f32 = 0.35;

/// The top of [`ShadowStyle::falloff`]: the shadow at its most plateaued
/// before the fall becomes a step.
///
/// Three, where half a width out still holds 61% of the depth and the whole
/// fall is spent in the outer half. Past that the profile stops being a
/// gradient at all — a plateau ending in a cliff is the closed contour again,
/// arrived at from the other side.
pub const SHADOW_FALLOFF_MAX: f32 = 3.0;

/// How many Shadow widths out a distance cell is padded, and where its window
/// shuts, at a given [`ShadowStyle::falloff`] — the radius at which that
/// falloff's decay has reached [`SHADOW_INVISIBLE`].
///
/// [`SHADOW_STOP`] is the FLOOR and not the answer. A falloff of 1 reaches the
/// threshold at 1.56 widths and every falloff above it sooner, so the fixed
/// stop is already generous there and holding it fixed is what keeps a fresh
/// picture the picture it was. Under 0.64 the decay reaches the threshold later
/// than the fixed stop, and the cell follows rather than cutting the tail off:
/// that is the whole of what lets the bar go below the 0.7 a fixed pad allowed.
///
/// The padding is the cost of the setting and lands only where it is dialled.
/// Both sides compute it — `shadow_stop` in common.wgsl is the shader's copy,
/// pinned by `the_shaders_falloff_stop_is_the_packers` — because a cell padded
/// to one radius and windowed at another is the straight-line cut that the pin
/// on [`SHADOW_STOP`] has always been there to prevent.
pub fn shadow_stop(falloff: f32) -> f32 {
    let falloff = if falloff.is_finite() {
        falloff.clamp(SHADOW_FALLOFF_MIN, SHADOW_FALLOFF_MAX)
    } else {
        1.0
    };
    // Above the crossover the solve lands under the floor and the floor is the
    // answer, so the `powf` is skipped rather than computed and discarded —
    // which is what makes "free above SHADOW_FALLOFF_FREE" true of a fragment
    // and not only of the atlas. The `max` still guards the other branch: the
    // constant is a rounded literal, and the floor is the thing that must hold.
    if falloff >= SHADOW_FALLOFF_FREE {
        return SHADOW_STOP;
    }
    SHADOW_STOP.max(((1.0 / SHADOW_INVISIBLE).ln() / SHADOW_TAIL).powf(1.0 / falloff))
}

/// How much of a distance shadow stands `u` Shadow widths out from the ink,
/// 0..=1, at a given [`ShadowStyle::falloff`] — `standoff_coverage` in
/// common.wgsl, in Rust.
///
/// The bar's own preview runs on this, so what the Lighting pane draws is the
/// profile and not a drawing of one. A second spelling of arithmetic that also
/// lives in a shader is a thing to keep honest rather than to avoid: WGSL is a
/// string here and Rust cannot call into it, and the alternative — a preview
/// with a formula of its own — is the copy that drifts silently.
/// `the_falloff_preview_is_the_shaders_profile` is what pins the two together.
///
/// `u` beyond [`shadow_stop`] is past the window and reads 0, which is also
/// where the cell stops being padded.
pub fn standoff_level(falloff: f32, u: f32) -> f32 {
    let falloff = if falloff.is_finite() {
        falloff.clamp(SHADOW_FALLOFF_MIN, SHADOW_FALLOFF_MAX)
    } else {
        1.0
    };
    let u = if u.is_finite() { u.max(0.0) } else { 0.0 };
    // The shader's own branch at the neutral value, kept here so the two are
    // one function rather than two that agree to a rounding.
    let t = if falloff == 1.0 { u } else { u.powf(falloff) };
    // The window, as a smoothstep between one width and this falloff's own
    // stop — spelled out rather than reached for because `f32` has no
    // smoothstep of its own.
    let w = ((u - 1.0) / (shadow_stop(falloff) - 1.0)).clamp(0.0, 1.0);
    (-SHADOW_TAIL * t).exp() * (1.0 - w * w * (3.0 - 2.0 * w))
}

/// What a shadow is MADE of: which of the two renderers turns a caster's ink
/// into the number its draw multiplies the frame by.
///
/// Two renderers and nothing between them. A caster is drawn by one or the
/// other — there is no mixture, no weight and no second term — and a frame
/// whose casters disagree schedules both paths rather than a third.
///
/// What parts them in the picture is FORM at a wide Shadow. A Gaussian of a
/// hairline peaks at a fraction of 1 and its profile carries the stroke's
/// WIDTH, which is why a blur needs a gain to reach the depth at all; a
/// distance is the same at a hairline as at a slab, so a letterform's
/// counters, a cross's arms and a corner's diagonal all survive the widest
/// Shadow the bar reaches. What a distance gives up is the pocket a blur
/// builds where two strokes stand close: the distance to the nearer of two
/// strokes is the distance to one of them, so a crease reads no deeper than a
/// lone edge — #490's crease, which is the model rather than a defect in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ShadowKernel {
    /// The DISTANCE, in the pane's points, from a fragment to the caster's
    /// nearest ink, supplied by that caster's exact field and read back through
    /// the standoff's own decay ([`SHADOW_TAIL`], [`SHADOW_STOP`]).
    #[default]
    Distance,
    /// The caster's coverage, convolved with one Gaussian at half the Shadow's
    /// width — the conventional accumulation, kept beside the distance as the
    /// alternative a group may be switched to rather than as a correction term
    /// mixed into it.
    ///
    /// One fixed kernel. A straight edge keeps `erfc(d / (σ√2)) / 2` of the
    /// light `d` out from its edge, which at one Shadow width is 2.3%, so both
    /// renderers answer one Shadow bar with one reach.
    Gaussian,
}

impl ShadowKernel {
    /// How far this kernel reaches past a caster's ink, in the picture's own σ.
    ///
    /// The two renderers end at different multiples of their own width, and
    /// both end EXACTLY: a Gaussian is lowered onto zero at [`REACH_SIGMAS`] σ
    /// (`PEDESTAL` in shadow.wgsl) and the standoff's curve is windowed to zero
    /// at [`shadow_stop`] Shadow widths, which is `2 · shadow_stop(falloff)` σ
    /// because a width is 2σ.
    ///
    /// One expression for both, so a quad and its cell's padding are sized off
    /// one number — a cell padded short of what its quad reaches is a shadow
    /// cut off in a straight line at the box.
    ///
    /// In σ and not in Shadow widths because that is the unit `sigma_points`
    /// hands out — one conversion, at one site.
    ///
    /// The `falloff` is the group's ([`ShadowStyle::falloff`]) and is what a
    /// distance's stop is solved from ([`shadow_stop`]), so a group dialled
    /// under 0.64 pads its cells and grows its quads to hold the longer tail it
    /// asked for. A Gaussian ignores it: its own reach is a number of σ and the
    /// exponent never enters that renderer.
    pub fn reach_sigmas(self, falloff: f32) -> f32 {
        match self {
            ShadowKernel::Gaussian => REACH_SIGMAS,
            ShadowKernel::Distance => 2.0 * shadow_stop(falloff),
        }
    }

    /// Whether a caster drawn by this kernel puts a DISTANCE in its cell rather
    /// than blurred ink — what the fill, the blur chain and the sampler each
    /// branch on, and what `ShadowCaster::shade` carries to the shader.
    pub fn is_distance(self) -> bool {
        matches!(self, ShadowKernel::Distance)
    }
}

/// One GROUP of casters' shadow: which renderer draws it, how far it reaches,
/// how dark it lands and where inside that reach the darkness sits.
///
/// Four values and no more. Which kernel a group is drawn by is a look; the
/// other three are what a person dials against that look. Anything that only
/// calibrates one renderer against the other — the Gaussian's gain, the
/// standoff's window — is a renderer constant and lives at the consumer, so
/// switching a group's kernel does not move its bars.
///
/// The groups are explicit ([`ShadowSettings`]) rather than one style with
/// per-group overrides: an override needs a sentinel for "not set", and a
/// sentinel is a fourth value with no place on a bar. Four numbers written out
/// twice cost less than one inheritance rule.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ShadowStyle {
    /// Which of the two renderers turns this group's ink into its shadow.
    pub kernel: ShadowKernel,
    /// How wide the shadow is, 0..=[`GLOW_SHADOW_MAX`](crate::GLOW_SHADOW_MAX),
    /// as a share of the caster group's reference size. Lattice groups use a
    /// node radius; spectral groups use the renderer's fixed four-point edge
    /// unit so their shadows stay screen-constant across pitch zoom and pane
    /// size. Half of that resolved width is the σ a Gaussian blurs at and the
    /// resolved width is what a distance's decay is measured in.
    ///
    /// 0 is the picture with no shadow in it for this group, pixel for pixel:
    /// no cell is packed and every draw multiplies by 1.
    pub width: f32,
    /// How dark the shadow lands where it is whole, 0..=1 — the factor the
    /// frame is left with under this group's solid ink, spent in STOPS across
    /// the width above (`shadow_transmittance` in common.wgsl).
    ///
    /// A FLOOR rather than a scale: ink wide against σ lands exactly here and a
    /// hairline lands short of it. 1 takes the frame under wide ink to black; 0
    /// is this group's second off switch.
    pub depth: f32,
    /// Where inside the width the shadow spends its darkness:
    /// [`SHADOW_FALLOFF_MIN`]..=[`SHADOW_FALLOFF_MAX`], the exponent the
    /// standoff's decay takes on the DISTANCE rather than on the coverage —
    /// `exp(-SHADOW_TAIL · u^falloff)` at `u` widths out.
    ///
    /// On the distance because the exponential family is closed under powers:
    /// an exponent on the finished coverage is `exp(-TAIL · f · u)`, which is
    /// the same curve at `width / f` and so a second spelling of the bar above
    /// — the shape #533's Shadow curve turned out to have, and half of why
    /// #582 could drop it without moving a pixel. An exponent on `u` bends the
    /// profile instead of rescaling it.
    ///
    /// The width keeps its exact meaning at every value, because `1^f` is 1
    /// whatever `f` is: one Shadow width out the decay stands at
    /// `exp(-SHADOW_TAIL)` — the fiftieth [`SHADOW_TAIL`] names — whatever this
    /// is dialled to. So the bar redistributes the near field rather than
    /// respelling the width, and the Shadow bar never has to be re-dialled
    /// after it.
    ///
    /// What it does move is where the decay ENDS, and so what the cell costs.
    /// From 0.64 up the window still shuts at [`SHADOW_STOP`] widths on the
    /// cell that was already padded for it, and the bar is free; below that
    /// [`shadow_stop`] solves for a longer radius and the cell grows to hold
    /// it — 1.8× the pad and 3.2× the cell AREA at [`SHADOW_FALLOFF_MIN`].
    /// That is the price of not cutting the tail off at a fixed radius, and
    /// [`SHADOW_FALLOFF_MIN`] is where it stops being worth paying.
    ///
    /// 1 is the plain exponential, and below it a sharp edge against the ink
    /// paid for with a long low tail; above it the shadow holds most of its
    /// depth out to a plateau and spends the fall late.
    ///
    /// A DISTANCE value. A Gaussian's coverage is not an exponential in the
    /// distance and has a saturation plateau of its own, so this is floored
    /// out of that renderer's arithmetic entirely rather than meaning
    /// something different there ([`ShadowKernel::Gaussian`], and the bar is
    /// disabled on a group dialled to it).
    pub falloff: f32,
}

impl Default for ShadowStyle {
    /// The style a bare `ShadowStyle` opens on: a fixture, or the renderer's
    /// fallback for a caster handed no style. Not what a fresh VIEW draws —
    /// its four groups are [`ShadowSettings::default`], and each of them
    /// differs from this.
    fn default() -> ShadowStyle {
        ShadowStyle {
            // Distance keeps a caster's form at this broad shadow width, where
            // a blur would inherit too much of the caster's own thickness.
            kernel: ShadowKernel::Distance,
            // A broad shadow preserves the form of the ring and marker across
            // the wide light field.
            width: 0.418_517_17,
            // Just under half depth leaves the shadow legible without cutting
            // the shared field back to the ground.
            depth: 0.477_784_4,
            // The plain exponential the standoff has always decayed on, so a
            // bare style is the picture from before the bar existed to the
            // last bit (`pow` is skipped outright at 1, see
            // `standoff_coverage`).
            falloff: 1.0,
        }
    }
}

impl ShadowStyle {
    /// Whether this group casts at all.
    ///
    /// Either bar at its bottom is the whole of this group's shadow gone — no
    /// cell packed, no atlas area, no taps, and every draw multiplying by
    /// exactly 1. The two are separate switches because they answer separate
    /// questions, and a picture is entitled to reach the off state from either.
    pub fn casts(self) -> bool {
        self.width > 0.0 && self.depth > 0.0
    }

    /// This style held to the ranges its bars name — the PICTURE's door, where
    /// [`ViewConfig::sanitize`](crate::ViewConfig::sanitize) is the blob's.
    ///
    /// Every caster's quad is grown by the width, so a number from outside the
    /// bar is a quad nothing can fill. The falloff's own bottom is the window's
    /// ([`SHADOW_FALLOFF_MIN`]) rather than a taste, so a number under it is a
    /// visible edge at a fixed radius. The kernel takes no clamp: an enum is in
    /// range or the blob did not parse.
    pub fn clamped(self) -> ShadowStyle {
        ShadowStyle {
            kernel: self.kernel,
            width: self.width.clamp(0.0, crate::GLOW_SHADOW_MAX),
            depth: self.depth.clamp(0.0, 1.0),
            falloff: self.falloff.clamp(SHADOW_FALLOFF_MIN, SHADOW_FALLOFF_MAX),
        }
    }
}

/// Every group of casters the picture dials its shadow separately.
///
/// One implementation with a value per group, not a shadow per group: both
/// renderers read one geometric field per caster and one profile, and a frame
/// whose groups disagree schedules two paths rather than four. What a group
/// buys is the pair of numbers, because the ink in it is a different KIND of
/// ink. A node's layered geometry and the notation that names its positions
/// want independent shadows, and the width that preserves a ring need not be
/// the width that keeps a letterform or resting marker legible.
///
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ShadowSettings {
    /// The lattice's node geometry: audio rings, octave bands and marks.
    pub lattice_geometry: ShadowStyle,
    /// The lattice's notation: note names, drawn marks and resting markers.
    pub lattice_text: ShadowStyle,
    /// The spectral roll's ribbons and the spiral's sounding-note dots.
    pub spectral_geometry: ShadowStyle,
    /// The spectral pane's note names and axis labels, and the spiral's names.
    pub spectral_text: ShadowStyle,
}

impl Default for ShadowSettings {
    /// The four groups a fresh view opens on, and the fallback for any one of
    /// them missing from a blob (the container-level `serde(default)` above).
    ///
    /// Four styles rather than one, last captured from the DAW on 2026-09-08: the
    /// picture as dialled, group by group. Every group is a distance shadow
    /// (see [`ShadowStyle::default`]); what differs is how far each kind of
    /// ink casts, how dark, and how the decay is bent.
    fn default() -> ShadowSettings {
        ShadowSettings {
            // As wide as the bar goes at about a quarter of the depth, with a
            // long falloff: a node's rings and marks stand in a broad, shallow
            // shadow without a hard edge.
            lattice_geometry: ShadowStyle {
                kernel: ShadowKernel::Distance,
                width: 1.0,
                depth: 0.248_531_57,
                falloff: 1.361_747_5,
            },
            // Tight and shallower than the geometry beside it, with a slightly
            // longer than exponential falloff: a letterform or a resting
            // marker keeps a crisp edge and sits close to what it is written
            // on rather than standing off it.
            lattice_text: ShadowStyle {
                kernel: ShadowKernel::Distance,
                width: 0.242_857_14,
                depth: 0.297_857_14,
                falloff: 1.083_516_8,
            },
            // Full depth under the roll's ribbons and the spiral's dots, wide
            // enough to lift them off the heatmap.
            spectral_geometry: ShadowStyle {
                kernel: ShadowKernel::Distance,
                width: 0.739_761_9,
                depth: 1.0,
                falloff: 0.693_238_14,
            },
            // Full depth under the spectral pane's names and axis labels too.
            spectral_text: ShadowStyle {
                kernel: ShadowKernel::Distance,
                width: 0.642_857_13,
                depth: 1.0,
                falloff: 0.797_345_2,
            },
        }
    }
}

impl ShadowSettings {
    /// Every group, in the order the settings pane lists them.
    ///
    /// The one place the groups are enumerated, so a group added at step 7 is
    /// added to the struct and to this and to nothing else: the clamp, the
    /// sanitize and the tests that sweep the groups all read it.
    pub fn groups(&self) -> [ShadowStyle; 4] {
        [self.lattice_geometry, self.lattice_text, self.spectral_geometry, self.spectral_text]
    }

    /// The same, to write through — [`groups`](Self::groups)'s pair.
    pub fn groups_mut(&mut self) -> [&mut ShadowStyle; 4] {
        [
            &mut self.lattice_geometry,
            &mut self.lattice_text,
            &mut self.spectral_geometry,
            &mut self.spectral_text,
        ]
    }

    /// Every group held to its bars' ranges; see [`ShadowStyle::clamped`].
    pub fn clamped(self) -> ShadowSettings {
        ShadowSettings {
            lattice_geometry: self.lattice_geometry.clamped(),
            lattice_text: self.lattice_text.clamped(),
            spectral_geometry: self.spectral_geometry.clamped(),
            spectral_text: self.spectral_text.clamped(),
        }
    }
}
