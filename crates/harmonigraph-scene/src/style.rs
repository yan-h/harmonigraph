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

/// Which shimmer a layer — the octave glyphs, or the melody/bass rings —
/// runs: one sheet of soft white light laid over the whole lattice, in the
/// pattern this names, or [`Off`](Pulse::Off) for the steady picture.
///
/// Every live mode is the same animation with a different shape to it, which
/// is what lets one set of knobs size all of them
/// ([`ViewConfig::shimmer_speed`](crate::ViewConfig::shimmer_speed) and the
/// three beside it). They share more than the knobs: the sheet is ONE field
/// spanning the whole lattice rather than a copy per node — every node
/// samples it at its own place on the plane the billboards face — so the
/// light reads as raking over the picture instead of as many small identical
/// animations. A mode works on a node with no mark on it at all.
///
/// The two layers run their sheets a quarter turn apart and half a period
/// offset, so a node wearing both crosses two of them and the brighter wins
/// the pixel. The mark rings' sheet also reaches OUT of its own layer, onto
/// the octave slice each ring points at — a mark being the ring together
/// with the octave it names. All of that lives in `lattice.wgsl`'s Shimmer
/// section.
///
/// One enum for both layers: the states mean the same thing wherever they're
/// read, and reading them off one shared clock keeps a node whose octaves
/// and marks are both animating in step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Pulse {
    /// Steady — no animation, the look every earlier build drew.
    #[default]
    Off,
    /// Parallel bands laid diagonally, travelling along their own normal: one
    /// grating, and the plainest reading of light passing over the lattice.
    ///
    /// Persisted blobs carry three older tokens that land here. `Shimmer` is
    /// this exact pattern under its former name, from before there was more
    /// than one. `Together` and `Alternating` were a different animation
    /// altogether — a slow breathe playing the octave a melody or bass ring
    /// pointed at against the rest of the layer — and they are gone; a view
    /// that asked for one of them asked for the layer to move, so it loads as
    /// the sweep rather than as [`Off`](Pulse::Off).
    #[serde(alias = "Shimmer", alias = "Together", alias = "Alternating")]
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
    /// Index the shader reads (`misc6.w` for the mark rings, `misc7.z` for
    /// the octave glyphs — see `Uniforms` in harmonigraph-render). 0 is the
    /// steady layer and every other value picks a pattern out of
    /// `shimmer_terms`.
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
