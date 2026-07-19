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
    /// Both, in their own colors. The default: the marks are subtle
    /// enough to live with always-on, and a chord's outer voices are
    /// worth seeing without having to go turn something on first. Blobs
    /// saved before this setting existed pick it up too, which is
    /// deliberate.
    #[default]
    Both,
}

/// How a melody/bass ring points back at the octave sector it came from.
///
/// The marks are full rings — the bass just inside the octave band, the
/// melody just outside — which is what lets both draw at full weight even
/// when one note is both ends of the chord. The cost of a full ring is that
/// it says the NODE carries the melody without saying which of its octaves
/// does, and on a chord voiced inside one pitch class that is the whole
/// question. These are the candidates for putting that back; they are
/// switchable so they can be compared live, the same way the octave glyph
/// shapes were before one of them won.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum MarkLink {
    /// Nothing: a plain unbroken ring. The quietest, and honest about
    /// marking the node rather than one octave of it.
    Off,
    /// The ring swells where the responsible sector is behind it. Keeps an
    /// unbroken circle — the shape reads first, the swell second.
    #[default]
    Bulge,
    /// The ring is dim all the way round and comes up to full only over the
    /// responsible sector, so the eye lands there first.
    Glow,
    /// A radial stub bridges the gap between the ring and its sector,
    /// physically joining the two. The most literal, and the loudest.
    Spur,
}

impl MarkLink {
    /// Index used by the shader (uniform `misc5.y`).
    pub fn shader_index(self) -> u32 {
        match self {
            MarkLink::Off => 0,
            MarkLink::Bulge => 1,
            MarkLink::Glow => 2,
            MarkLink::Spur => 3,
        }
    }
}

impl HighlightExtremes {
    pub fn marks_melody(self) -> bool {
        matches!(self, HighlightExtremes::Melody | HighlightExtremes::Both)
    }

    pub fn marks_bass(self) -> bool {
        matches!(self, HighlightExtremes::Bass | HighlightExtremes::Both)
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
