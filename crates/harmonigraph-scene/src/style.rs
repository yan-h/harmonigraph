//! The visual-style settings the view config carries: the enums it selects
//! between, with the shader indices they map to, and the pitch gradient's four
//! knobs. Adding a style means touching this file and the matching branch in
//! `lattice.wgsl`; the gradient reaches the shader as a color table instead,
//! and needs no branch there at all.

/// How the core orb is painted while notes sound (inert when there is no
/// orb, i.e. [`ViewConfig::core_radius`](crate::ViewConfig::core_radius)
/// is 0). All styles share the same instance data
/// (activation + per-note phase); the fragment/vertex shader switches
/// on a uniform. Kept as switchable candidates for live comparison — idle
/// nodes look identical in every style.
///
/// The aliases on Steady absorb node styles that used to exist (Breathe,
/// Sparks, the Wire/Corona/Plasma/Aurora/Marble/Lava/Filament/Stripes/
/// Rings/Tiles set trimmed later, and Pinwheel after them) so persisted
/// view blobs that still name them keep loading.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum NodeStyle {
    /// The original look: steady disc + glow.
    #[default]
    #[serde(
        alias = "Breathe",
        alias = "Sparks",
        alias = "Wire",
        alias = "Corona",
        alias = "Plasma",
        alias = "Aurora",
        alias = "Marble",
        alias = "Lava",
        alias = "Filament",
        alias = "Stripes",
        alias = "Rings",
        alias = "Tiles",
        alias = "Pinwheel"
    )]
    Steady,
    /// Gas ball: octave colors sheared into rotating spiral streaks, like
    /// stirred paint.
    Vortex,
    /// Pattern: soft checkerboard on the globe graticule.
    Checker,
    /// Pattern: two-armed spiral of color waves hugging the sphere.
    Spiral,
}

impl NodeStyle {
    /// Index used by the shader (uniform `misc.w`). Indices are preserved
    /// from the original 15-style set so each kept style's shader branch in
    /// lattice.wgsl stays byte-for-byte unchanged; the gaps are the removed
    /// styles.
    pub fn shader_index(self) -> u32 {
        match self {
            NodeStyle::Steady => 0,
            NodeStyle::Vortex => 3,
            NodeStyle::Spiral => 12,
            NodeStyle::Checker => 13,
        }
    }

    /// The field family — everything except Steady: styles whose active
    /// discs paint the swirled octave-color field (noise-driven gas or
    /// deterministic patterns). These animate on global time with a stable
    /// per-node seed (see [`derive_scene`]), so note events never restart
    /// the pattern. Mirrors `is_field_style` in lattice.wgsl; keep in sync.
    pub fn is_field_style(self) -> bool {
        !matches!(self, NodeStyle::Steady)
    }
}


/// The low-to-high pitch gradient, as four things a reader of the picture can
/// name: WHICH colors it walks through (an arc of the LCh hue circle), how
/// bright the middle of the range sits, how much brightness separates the ends
/// of it, and how much color the whole thing carries. The curve those describe
/// is in [`color::pitch_ramp_lch`](crate::color).
///
/// The four are independent on purpose, and that is the point of shaping the
/// setting this way rather than as a list of named palettes: brightness is a
/// far stronger cue than hue, so how much of it a gradient SPENDS on pitch is
/// the decision worth having a knob for. At
/// [`lightness_ramp`](Self::lightness_ramp) 0 the gradient is exactly
/// isoluminant — in CIELab `L*` is a function of luminance alone, so one `L*`
/// is one screen brightness, and a bass note then reads as loud as a treble
/// one with hue carrying the pitch by itself. Wind the ramp up and brightness
/// takes over; wind it negative and the picture inverts.
///
/// Nothing here can leave the sRGB gamut, whatever the four are set to, which
/// is what makes them safe to expose as free knobs — see
/// [`chroma`](Self::chroma).
///
/// A gradient is a view setting rather than a param: it says what the picture
/// MEANS, and it reaches the shader as the contents of the pitch LUT (see
/// [`pitch_ramp_lut`](crate::pitch_ramp_lut)), so `lattice.wgsl` never learns
/// any of it exists.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PitchGradient {
    /// LCh hue the BOTTOM of the pitch range takes, in degrees. Wrapped into
    /// 0..360 by [`sanitized`](Self::sanitized), since it names a point on a
    /// circle and every value is therefore a legal one.
    #[serde(default = "default_hue_start")]
    pub hue_start: f32,
    /// How far round the hue circle the range walks, in degrees — SIGNED, so
    /// its sign is which way round the circle the pitch runs and flipping it
    /// reverses the spectrum without touching where it starts.
    ///
    /// Not a second endpoint, which is the same information but the wrong
    /// shape for this: two endpoints on a circle name two arcs and cannot say
    /// which one is meant, and neither can they reach past a full turn. A
    /// signed span says both, and 0 collapses the gradient to a single hue —
    /// a legal setting, where chroma and brightness carry the pitch alone.
    #[serde(default = "default_hue_span")]
    pub hue_span: f32,
    /// `L*` at the CENTRE of the pitch range, 0..100 — the middle of the
    /// gradient rather than either end, so that
    /// [`lightness_ramp`](Self::lightness_ramp) opens symmetrically about it
    /// and this knob keeps meaning "how bright is the picture" at every ramp.
    #[serde(default = "default_lightness")]
    pub lightness: f32,
    /// Signed `L*` difference from the bottom of the range to the top: how
    /// much of the gradient's separation is spent on brightness. 0 is exactly
    /// isoluminant, negative puts the bright end at the bottom.
    #[serde(default = "default_lightness_ramp")]
    pub lightness_ramp: f32,
    /// How much color, as a fraction 0..1 of the most the sRGB gamut holds at
    /// each point of the curve — NOT an absolute chroma.
    ///
    /// The gamut is the reason. At a fixed `L*` sRGB admits a different
    /// maximum chroma at every hue (49.9 at the tightest point of the default
    /// arc, 89.8 at its widest), and that maximum collapses toward 0 as `L*`
    /// approaches either end of its own range. An absolute chroma would
    /// therefore be a number whose meaning changes under the other three
    /// knobs: past what the gamut holds a channel clips, which bends the hue
    /// and MOVES THE LUMINANCE — so the isoluminance promised above would
    /// quietly stop being true — and the top of the control would be dead over
    /// whatever part of the arc had already run out. Denominated in what is
    /// actually available, every setting is reachable, monotone, and in gamut
    /// by construction; the cost is that absolute chroma varies along the arc,
    /// since the hues that can hold more are given more.
    #[serde(default = "default_chroma")]
    pub chroma: f32,
}

/// The hue the bottom of the range opens on: a deep blue-violet.
fn default_hue_start() -> f32 {
    260.0
}

/// Just over half a turn, running blue-violet up through magenta and red to
/// yellow-green. Wide enough that no two octaves of the range share a hue, and
/// short of the full circle so the two ends cannot be confused for each other.
fn default_hue_span() -> f32 {
    190.0
}

/// Mid-range brightness, placed with [`default_lightness_ramp`] so the
/// gradient opens at `L*` 42 and closes at 86. That is a 5.4x span in screen
/// luminance from the bottom of the pitch range to the top — a strong cue, and
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

/// Half the available chroma. Enough that the hues read as colors, and well
/// short of the gamut boundary, where the maximum's own kinks between the sRGB
/// primaries start to show up as bumps in the sweep — which is what the top of
/// the knob is for, not what it should open on.
fn default_chroma() -> f32 {
    0.5
}

impl Default for PitchGradient {
    fn default() -> Self {
        PitchGradient {
            hue_start: default_hue_start(),
            hue_span: default_hue_span(),
            lightness: default_lightness(),
            lightness_ramp: default_lightness_ramp(),
            chroma: default_chroma(),
        }
    }
}

impl PitchGradient {
    /// Widest span either way round the circle: a full turn, which is every
    /// arc there is. Past it the gradient would revisit hues it already used.
    pub const MAX_HUE_SPAN: f32 = 360.0;

    /// Fit the four to what their controls can produce. A bar cannot make a
    /// nonsense value but a hand-edited RON can, and these feed a color
    /// conversion whose output goes straight into the instance buffer: a
    /// non-finite `L*` would ride out as a NaN color rather than announce
    /// itself, and it would also break the LUT memo's key, which compares
    /// gradients for equality.
    ///
    /// Applied at the table (see [`pitch_ramp_lut`](crate::pitch_ramp_lut)) as
    /// well as on the way in from a blob, so the guarantee holds for a
    /// gradient assembled in code too.
    pub fn sanitized(self) -> PitchGradient {
        let finite = |v: f32, fallback: f32| if v.is_finite() { v } else { fallback };
        let hue_span = finite(self.hue_span, default_hue_span())
            .clamp(-Self::MAX_HUE_SPAN, Self::MAX_HUE_SPAN);
        PitchGradient {
            // Wrapped rather than clamped: it names a point on a circle.
            hue_start: finite(self.hue_start, default_hue_start()).rem_euclid(360.0),
            // A span of zero has no direction, so it is written with the one
            // sign that reads as none. `-0.0` is the sign that lies: it is not
            // `< 0.0`, so everything that asks which way the arc runs takes it
            // for positive, while `{:+}` prints it as "-0" — a bar that says
            // one direction and behaves as the other. Flipping a zero span and
            // dragging a flipped one down to nothing both produce it.
            hue_span: if hue_span == 0.0 { 0.0 } else { hue_span },
            lightness: finite(self.lightness, default_lightness()).clamp(0.0, 100.0),
            // The ends of the ramp are clamped into 0..100 where they are
            // computed, so a ramp steeper than the `L*` axis is a legal
            // setting that simply flattens against black or white.
            lightness_ramp: finite(self.lightness_ramp, default_lightness_ramp())
                .clamp(-100.0, 100.0),
            chroma: finite(self.chroma, default_chroma()).clamp(0.0, 1.0),
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
    pub fn flipped(self) -> PitchGradient {
        let g = self.sanitized();
        PitchGradient {
            hue_start: (g.hue_start + g.hue_span).rem_euclid(360.0),
            hue_span: -g.hue_span,
            ..g
        }
        .sanitized()
    }

    /// `L*` and hue at normalized height `t` (0 at the bottom of the pitch
    /// range, 1 at the top). The two curves the gradient is made of, kept
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
        (l.clamp(0.0, 100.0), h)
    }
}

/// The idle-node marker: a minimal grey mark shown at each home-sheet node
/// at all times, independent of the active appearance and of whether a note
/// is playing. Sized by [`ViewConfig::idle_radius`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum IdleMarker {
    /// Nothing.
    None,
    /// A filled grey dot.
    Dot,
    /// A thin grey outline circle (the classic placeholder look).
    #[default]
    Circle,
}

impl IdleMarker {
    /// Index the shader reads (uniform `misc4.w`): 0 none, 1 dot, 2 circle.
    pub fn shader_index(self) -> u32 {
        match self {
            IdleMarker::None => 0,
            IdleMarker::Dot => 1,
            IdleMarker::Circle => 2,
        }
    }
}

/// How the octave glyphs or the melody/bass rings animate. Two families,
/// and they animate different things:
///
/// [`Together`](Pulse::Together) and [`Alternating`](Pulse::Alternating) are
/// a slow breathe on a split the layer already has — a "near the melody/bass
/// slice" part and a "rest of it" part: the octave glyph FOR the slot a
/// melody or bass ring is pointing at, against every other glyph (sounding
/// or ghost alike), or a mark ring's own arc over that slot against the rest
/// of the ring (see the octave-glyph loop and [`mark_ring_alpha`] in
/// lattice.wgsl — both key off the same `melody_slots`/`bass_slots`
/// bitmasks). Deliberately not "every sounding octave": a chord tone that is
/// neither the highest nor the lowest held note isn't an indicator those
/// modes are about — which is also why they have nothing to say with both
/// marks switched off.
///
/// [`Shimmer`](Pulse::Shimmer) leaves that split alone and sweeps a sheet of
/// soft white bands across the layer instead, so it works on a node with no
/// mark at all. It is also the one mode that reaches OUT of its own layer:
/// the mark rings' sweep takes the octave slice each ring points at, a mark
/// being the ring together with the octave it names. How fast, how wide and
/// how deep that sweep is are settings shared by both layers
/// ([`ViewConfig::shimmer_speed`](crate::ViewConfig::shimmer_speed)).
///
/// One enum for both layers: the states mean the same thing wherever they're
/// read, and reading them off one shared clock keeps a node whose octaves
/// and marks are both animating in step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Pulse {
    /// Steady — no animation, the look every earlier build drew.
    #[default]
    Off,
    /// Both parts breathe together, in phase.
    Together,
    /// The two parts breathe a half-cycle apart, so one brightens as the
    /// other dims.
    Alternating,
    /// Soft white bands, laid diagonally and scrolling: not a breathe but a
    /// travelling highlight, brightening each part of the layer as it
    /// crosses.
    ///
    /// The bands are ONE field spanning the whole lattice rather than a copy
    /// per node — every node samples the same sheet at its own place on the
    /// plane the billboards face — so a band reads as light raking over the
    /// picture. The octave glyphs' bands and the mark rings' run a quarter
    /// turn apart, so a node wearing both crosses two textures at right
    /// angles, and where they cross the brighter of the two wins the pixel.
    /// The mark rings' sweep covers the octave slice each ring points at as
    /// well as the ring. All of that lives in `lattice.wgsl`'s Shimmer
    /// section.
    Shimmer,
}

impl Pulse {
    /// Index the shader reads (`misc6.w` for the mark rings, `misc7.z` for
    /// the octave glyphs — see `Uniforms` in harmonigraph-render).
    pub fn shader_index(self) -> u32 {
        match self {
            Pulse::Off => 0,
            Pulse::Together => 1,
            Pulse::Alternating => 2,
            Pulse::Shimmer => 3,
        }
    }

    /// Whether this mode DRIVES the layer's "near the melody/bass slice"
    /// against "the rest" split apart, and so needs a marked slot to have two
    /// parts to drive — with neither mark on, the split collapses and the
    /// mode animates something other than what it says. The UI grays exactly
    /// these out, and `derive_scene` folds them to [`Off`](Pulse::Off) so the
    /// picture agrees with the grayed control.
    ///
    /// [`Alternating`](Pulse::Alternating) is the only one. The test is what
    /// the shader's `pulse_pair` does, NOT which modes are named for the
    /// split: [`Together`](Pulse::Together) is the mode that deliberately
    /// does not drive it apart — its two phases are the same wave, so it
    /// breathes the layer as one whether or not anything is marked, and has
    /// nothing to collapse. [`Shimmer`](Pulse::Shimmer) crosses the whole
    /// layer and never touches the split either.
    pub fn needs_a_marked_slot(self) -> bool {
        matches!(self, Pulse::Alternating)
    }
}

/// Legacy load-only spelling of the melody/bass marks, from before they
/// became the two independent flags they always were:
/// [`ViewConfig::mark_melody`](crate::ViewConfig::mark_melody) and
/// [`mark_bass`](crate::ViewConfig::mark_bass). Four variants for two bits
/// meant the UI offered a row of alternatives to a question with two
/// answers, and "Both" had to be argued for as a default rather than
/// falling out of two boxes both being ticked.
///
/// Persisted blobs still carry a `highlight_extremes` token;
/// [`ViewConfig::sanitize`](crate::ViewConfig::sanitize) folds
/// it into the pair and it is never written back. Kept as a distinct type
/// only so those tokens keep deserializing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum HighlightExtremes {
    Off,
    /// The highest held note — the melody.
    Melody,
    /// The lowest held note — the bass.
    Bass,
    /// Both, which is what a blob predating the setting entirely picks up.
    #[default]
    Both,
}

impl HighlightExtremes {
    pub fn marks_melody(self) -> bool {
        matches!(self, HighlightExtremes::Melody | HighlightExtremes::Both)
    }

    pub fn marks_bass(self) -> bool {
        matches!(self, HighlightExtremes::Bass | HighlightExtremes::Both)
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
    ///
    /// Aliases the retired `Comma` mode — the name plus the signed cents to
    /// that namesake — which existed to supply exactly the information the
    /// mark now carries in the name itself.
    #[default]
    #[serde(alias = "Comma")]
    Name,
    /// The pitch class in cents under the current tuning, alone. Says what
    /// the node is and nothing it isn't, at the cost of saying where it is.
    Cents,
    /// No text at all. The octave band, the marks and the color carry the
    /// node; text is what the home sheet gets and the sevens layer does
    /// without.
    None,
}
