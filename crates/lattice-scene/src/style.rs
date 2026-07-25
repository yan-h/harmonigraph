//! The visual-style enums the view config selects between, and the
//! shader indices they map to. Adding a style means touching this file and
//! the matching branch in `lattice.wgsl`.

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

/// Legacy load-only spelling of the melody/bass marks, from before they
/// became the two independent flags they always were:
/// [`ViewConfig::mark_melody`](crate::ViewConfig::mark_melody) and
/// [`mark_bass`](crate::ViewConfig::mark_bass). Four variants for two bits
/// meant the UI offered a row of alternatives to a question with two
/// answers, and "Both" had to be argued for as a default rather than
/// falling out of two boxes both being ticked.
///
/// Persisted blobs still carry a `highlight_extremes` token;
/// [`ViewConfig::migrate_legacy`](crate::ViewConfig::migrate_legacy) folds
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

/// What text an OFF-SHEET node's label carries — a node on any sevens sheet
/// but the center one.
///
/// Their names are the reason this exists.
/// [`LatticePos::note_name`](lattice_core::LatticePos::note_name) walks the
/// chain of fifths with `1 + threes + fives*4 - sevens*2`, and nothing
/// anywhere adds a septimal mark, so **every sevens step spells exactly like
/// two fifths down**: `(0,0,1)` and `(-2,0,0)` are both `B♭`, 27 cents apart.
/// On a three-sheet view each name appears three times at three different
/// pitches — in the biggest glyph on the node. Off the home sheet the name is
/// not merely uninformative; it asserts something false.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SevensLabel {
    /// The same name the home sheet gets, ambiguity and all. What a view
    /// saved before this setting existed keeps.
    #[default]
    Name,
    /// The pitch class in cents under the current tuning, alone. Says what
    /// the node is and nothing it isn't, at the cost of saying where it is.
    Cents,
    /// The shared name, plus the signed cents to the home-sheet node it
    /// shares that name with — the septimal comma, ±27¢ at just intonation.
    /// Keeps the letter for orientation and adds precisely the information
    /// the name refuses to carry. The fifth, third and seventh are all
    /// tunable, so the number moves: it reads out how far the sevens axis
    /// sits from the name it inherits.
    Comma,
    /// No text at all. The octave band, the marks and the color carry the
    /// node; text is what the home sheet gets and the sevens layer does
    /// without.
    None,
}

/// Which shape the septimal mark is DRAWN as.
///
/// It is drawn rather than typeset because no character in the bundled
/// Iosevka subset can carry it: the subset holds exactly four arrow glyphs
/// (`←↑→↓`) and no triangles, and an arrow puts its direction in the head --
/// a 1px detail at the size a mark actually renders, which is `MARK_SIZE`
/// (8.25pt) times the off-sheet floor. Geometry lets the weight be chosen in
/// pixels instead of inherited from a typeface designed for body text.
///
/// This enum exists to be DELETED. Four designs are hard to rank by
/// reasoning about them and easy to rank by looking, so they ship together
/// and the winner gets hardcoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum SeptimalGlyph {
    /// Filled triangle. The most ink and the most robust silhouette: the
    /// direction is the whole shape, so nothing about it is a small detail
    /// that can blur away.
    #[default]
    Triangle,
    /// The triangle's outline only. Lighter beside the letter, at the cost
    /// of an interior that closes up as it shrinks.
    Hollow,
    /// Stem plus a solid head -- an arrow, but with the head sized for this
    /// use rather than for a text face.
    Arrow,
    /// The head alone, as a stroked V. The lightest of the four, and the
    /// only one whose weight reads like the `+`/`-` beside it.
    Chevron,
}
