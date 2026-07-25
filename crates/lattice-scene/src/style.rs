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
    /// by radius (melody inside the octave band, bass outside) rather than
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
    /// The same name the home sheet gets, ambiguity and all. What every
    /// build before this drew, and what a view saved by one keeps.
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

/// How a label is lifted off whatever it lands on.
///
/// Both panes draw text over a picture — note names over lit nodes, pitch
/// labels over the spectrogram — so text with no separation of its own
/// disappears into whatever is behind it. The separation is drawn by
/// stamping the text around a ring in the panel's dark color, which is also
/// the labels' whole cost: it is the same glyphs again, once per sample.
///
/// Hence a choice rather than a constant. The ring is 32 stamps at [`Halo`],
/// 8 at [`Outline`], and none at [`Off`] — and since a busy lattice's labels
/// are most of the geometry it hands the tessellator, this is the largest
/// single lever the look has over what a frame costs.
///
/// [`Halo`]: LabelRim::Halo
/// [`Outline`]: LabelRim::Outline
/// [`Off`]: LabelRim::Off
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum LabelRim {
    /// A tight opaque ring inside a wider faint one: the text sits in a soft
    /// dark pool that fades out. Reads over anything, including a node at
    /// full brightness, and is what every build before the setting drew.
    #[default]
    Halo,
    /// The tight ring alone, at a third of the samples — a crisp dark
    /// outline hugging the glyphs, with no pool around it. Keeps the
    /// separation and drops the softness, for about a quarter of the cost.
    Outline,
    /// Nothing: the glyphs alone. The cheapest label there is, and it reads
    /// fine wherever the picture behind it stays dark.
    Off,
}
