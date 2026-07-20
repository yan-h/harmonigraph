//! The visual-style enums the view config selects between, and the
//! shader indices they map to. Adding a style means touching this file and
//! the matching branch in `lattice.wgsl`.

/// The OUTER layer: whether a node shows which octaves its pitch class is
/// sounding in. The glyphs draw inside the radial band between the view's
/// `outer_inner` and `outer_outer` radii, from the per-node octave bitmask.
///
/// Only ONE glyph shape remains. This started as a set of switchable looks
/// (dots, rings, and several trimmed earlier) for live comparison; slices
/// won, so the choice is now just whether the layer draws at all. The dead
/// names are kept as serde aliases, not variants — an old blob naming one
/// loads as slices rather than dropping the whole persist.
///
/// Fully independent of the CORE layer ([`CoreStyle`]): the glyphs draw
/// the same whatever the core does. [`ViewConfig::outer_backdrop`] — its
/// own outer-layer setting, not the core's business — ghosts the silent
/// slots in the note color, completing the circle so a single sounding
/// octave still reads as one whole note.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum OuterStyle {
    /// No octave indication.
    Off,
    /// Annular pizza-slice sectors spanning the band, with
    /// constant-thickness gaps between neighbors. Each sector's angle
    /// tracks absolute pitch: middle C straight up, 45deg clockwise per
    /// octave, pitch class within the octave included.
    #[default]
    #[serde(alias = "Dots", alias = "Rings", alias = "Petals", alias = "Flares", alias = "Bumps")]
    Slices,
}

/// How the core orb is painted while notes sound (inert when there is no
/// orb, i.e. [`ViewConfig::core_radius`](crate::ViewConfig::core_radius)
/// is 0). All styles share the same instance data
/// (activation + per-note phase); the fragment/vertex shader switches
/// on a uniform. Kept as switchable candidates for live comparison — idle
/// nodes look identical in every style.
///
/// The aliases on Steady absorb node styles that used to exist (Breathe,
/// Sparks, and the Wire/Corona/Plasma/Aurora/Marble/Lava/Filament/Stripes/
/// Rings/Tiles set trimmed later) so persisted view blobs that still name
/// them keep loading.
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
        alias = "Tiles"
    )]
    Steady,
    /// Gas ball: octave colors sheared into rotating spiral streaks, like
    /// stirred paint.
    Vortex,
    /// Pattern: soft checkerboard on the globe graticule.
    Checker,
    /// Pattern: two-armed spiral of color waves hugging the sphere.
    Spiral,
    /// Pattern: beach-ball sectors around a tilted pole, slowly turning.
    Pinwheel,
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
            NodeStyle::Pinwheel => 11,
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

impl OuterStyle {
    /// Index used by the shader (uniform `misc.z`). Now only "off" or
    /// "on" — the shader has one glyph shape left — but kept as the
    /// original index so the slices branch stays byte-for-byte unchanged.
    pub fn shader_index(self) -> u32 {
        match self {
            OuterStyle::Off => 0,
            OuterStyle::Slices => 5,
        }
    }
}

/// Legacy load-only core mode, from before the core became a plain radius
/// (`core_radius`, with 0 = off) plus a `core_solidity` slider. Persisted
/// blobs still carry a `core_style` token; [`ViewConfig::migrate_legacy`]
/// folds each value into today's radius/solidity and then this is ignored
/// (the field is `skip_serializing`). Kept as a distinct type only so those
/// tokens keep deserializing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum CoreStyle {
    /// The pre-radius-off "nothing" mode (serialized `"Empty"`). Folds to
    /// `core_radius = 0` (off). The bare `"None"` token is the older
    /// glow-only mode instead, aliased onto [`Glow`](CoreStyle::Glow).
    #[serde(rename = "Empty")]
    None,
    /// The core is present with radius + solidity already in their own
    /// fields; nothing to fold. The default (and what recent blobs wrote).
    #[default]
    On,
    /// The pre-solidity solid orb. Folds to solidity 1.
    Orb,
    /// The pre-solidity glow-only mode (also the bare `"None"` token
    /// pre-split blobs wrote). Folds to solidity 0.
    #[serde(alias = "None")]
    Glow,
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

/// Which extreme held notes get marked, so a chord's melody and/or bass
/// line is identifiable at a glance. "Extreme" is by sounding pitch
/// (`Voice::pitch`, which includes MPE/tuning bends), over HELD voices
/// only: a released note is on its way out and shouldn't keep the mark
/// from the note that replaced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum HighlightExtremes {
    Off,
    /// The highest held note — the melody.
    Melody,
    /// The lowest held note — the bass.
    Bass,
    /// Both. Each ring takes its own note's color, and they are told apart
    /// by radius (bass inside the octave band, melody outside) rather than
    /// by hue. The default: the marks are subtle
    /// enough to live with always-on, and a chord's outer voices are
    /// worth seeing without having to go turn something on first. Blobs
    /// saved before this setting existed pick it up too, which is
    /// deliberate.
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

/// What separates the melody/bass stripe from the note under it.
///
/// The stripe is white. White is the right color for it — it reads as a
/// mark rather than as more note — but on its own it fails at the top of
/// the pitch ramp, which runs to near-white, and that is exactly where the
/// MELODY mark lands. So the separation comes from the boundary rather than
/// from the fill: something between the white and the note that neither end
/// of the ramp can swallow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum MarkContrast {
    /// A gap knocked through the sector where the white meets the note —
    /// the same device as the gaps between sectors, and thinner: a band of
    /// constant thickness about a ray from the node's centre. Nothing is
    /// painted, so nothing can be the wrong color against the note.
    #[default]
    #[serde(alias = "Keyline")]
    Gap,
    /// The white ramps to dark across the stripe instead, so it ends on the
    /// same boundary without a visible seam.
    Gradient,
    /// Nothing: plain white, which is legible on every note but the palest.
    Off,
}

/// How the melody and bass are marked.
///
/// The ring already encodes absolute pitch as ANGLE, so a reader can see
/// which of two sectors is the higher one. The mark therefore does not have
/// to distinguish melody from bass — only to say WHICH TWO they are. That
/// is a much weaker requirement than the earlier designs assumed, and the
/// last two options here spend nothing to meet it: no ink, no reserved
/// space, and nothing that has to hold contrast against a note that could
/// be any color on the ramp.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum MarkStyle {
    /// A white stripe down one side of the marked sector.
    #[default]
    Stripe,
    /// Nothing is added: the INNER voices are dimmed instead, leaving the
    /// outer two at full strength. Contrast is relative, so it cannot wash
    /// out on any note; and it costs nothing when there is nothing to say,
    /// because with one or two notes held every note IS an extreme and so
    /// nothing dims.
    Emphasis,
    /// The marked sector spans a wider angle, growing toward its own side —
    /// the melody clockwise, the bass counter-clockwise, a lone note both
    /// ways. It grows into the gap it already has, so nothing is reserved
    /// for it, and size reads on any color.
    Widen,
    /// The marked sector is lifted toward white until it crosses the bloom
    /// pass's luminance threshold, so it — and only it — halos. The emphasis
    /// is light rather than color or shape: nothing is added to the layer,
    /// and the halo lands outside the node where there is nothing to crowd.
    /// Needs Bloom above 0 to show its halo; the lift alone still reads.
    Glow,
    /// The marked sector breathes: a slow rise and fall in brightness, on
    /// the same global clock the field styles use, so notes never restart
    /// it. Motion is the one channel nothing else on the node is using, and
    /// it survives any color the note happens to be.
    Pulse,
    /// The marked sector stays crisp while the rest soften, as though the
    /// outer voices were in focus and the inner ones behind them. Costs no
    /// ink, no space and no color — only sharpness.
    Focus,
    /// A bright band travels along the marked sector, outward for the
    /// melody and inward for the bass — so the motion says which end it is
    /// as well as that it is one. A lone note, being both, sweeps both ways
    /// at once.
    ///
    /// It lifts what it passes over toward white, so it reads strongly on a
    /// dark or saturated note and hardly at all on a pale one — and the top
    /// of the pitch ramp is nearly white, which is exactly where the MELODY
    /// mark tends to sit. [`Pulse`](MarkStyle::Pulse) and
    /// [`Throb`](MarkStyle::Throb) have no such blind spot.
    Sweep,
    /// The marked sector breathes in SIZE rather than in brightness,
    /// widening and narrowing on the beat. Motion with nothing taken from
    /// the note's color or its light.
    Throb,
}

impl MarkStyle {
    /// Index used by the shader (uniform `misc7.x`).
    pub fn shader_index(self) -> u32 {
        match self {
            MarkStyle::Stripe => 0,
            MarkStyle::Emphasis => 1,
            MarkStyle::Widen => 2,
            MarkStyle::Glow => 3,
            MarkStyle::Pulse => 4,
            MarkStyle::Focus => 5,
            MarkStyle::Sweep => 6,
            MarkStyle::Throb => 7,
        }
    }
}

/// How the inner voices recede under [`MarkStyle::Emphasis`].
///
/// All three act on the sector's COLOR, never on its coverage. Dimming
/// coverage — what this did first — makes a sector translucent rather than
/// dark: its edges soften, the glow behind shows through, and the crisp
/// gaps between octaves go mushy. That is a real loss of contrast across
/// the whole layer, and it is not what was being asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum MarkRecede {
    /// Drain the color, keeping the brightness. The inner voices stay
    /// exactly as visible as they were — same luminance, same edges — and
    /// give up only their hue, so the outer two are the only sectors still
    /// carrying color. Costs the least contrast of the three.
    #[default]
    Grey,
    /// Darken, keeping the hue. Every voice keeps its pitch color; the
    /// inner ones simply sit further back.
    Dim,
    /// Both at once, for when one alone is not separation enough.
    Both,
}

impl MarkRecede {
    /// Index used by the shader (uniform `misc7.z`).
    pub fn shader_index(self) -> u32 {
        match self {
            MarkRecede::Grey => 0,
            MarkRecede::Dim => 1,
            MarkRecede::Both => 2,
        }
    }
}

/// Which side of the marked sector's edge the white sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum MarkPlace {
    /// Carved out of the sector itself, along its edge.
    #[default]
    Inside,
    /// Laid alongside the sector instead, just past its edge, leaving the
    /// note's own wedge whole. Drawn UNDER the octave sectors, so a lit
    /// neighbour still wins the pixels it wants.
    Outside,
}

impl MarkPlace {
    /// Index used by the shader (uniform `misc6.z`).
    pub fn shader_index(self) -> u32 {
        match self {
            MarkPlace::Inside => 0,
            MarkPlace::Outside => 1,
        }
    }
}

impl MarkContrast {
    /// Index used by the shader (uniform `misc6.x`).
    pub fn shader_index(self) -> u32 {
        match self {
            MarkContrast::Gap => 0,
            MarkContrast::Gradient => 1,
            MarkContrast::Off => 2,
        }
    }
}

/// The short-lived NodeBody experiment's variants (one working-tree
/// build, 2026-07-18, octave-only note bodies): parsed load-only via
/// `ViewConfig::node_body` and folded into the core/outer split by
/// [`ViewConfig::migrate_legacy`], so blobs saved by that build keep
/// loading instead of dropping the whole persist.
/// (Not an Option: the legacy blobs wrote the variant bare, which RON
/// would refuse to parse into an Option's `Some`; the `Disc` default
/// doubles as the "nothing to migrate" state.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum LegacyNodeBody {
    #[default]
    Disc,
    #[serde(alias = "Pie")]
    Slices,
    Rings,
    Beads,
}
