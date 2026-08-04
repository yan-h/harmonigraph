//! The scene layer: turns core state (note tracker + tuning) into a
//! render-friendly description, once per frame. The renderer consumes a
//! [`Scene`] and knows nothing about MIDI; the core knows nothing about
//! cameras or colors. Animation/envelope *policy* lives here.
//!
//! What lives where:
//! - `lib.rs` (this file) — the render-facing types: [`Scene`],
//!   [`NodeInstance`], [`EdgeInstance`], and the constants they share.
//! - [`derive`] — the per-frame derivation ([`derive_scene`]): note tracker
//!   + tuning -> node/edge lists. Envelope and animation policy.
//! - [`view`] — [`ViewConfig`] (persisted visual settings, serde defaults,
//!   legacy-blob migration) and [`FrameParams`].
//! - [`style`] — the visual-style enums and their shader indices.
//! - [`octaves`] — where the octave indicators sit around a node: how many,
//!   how wide, and the boundary angles the shader draws them between.
//! - [`camera`] — [`Camera`], [`Projection`], and the [`Projector`] used
//!   for label placement and picking.
//! - [`color`] — the LCh pitch ramp and channel/idle colors.
//! - [`skin`] — the static palette the UI and renderer share.
//! - [`trail`] — a quiet mark on the nodes the music has already been to.
//!
//! Every public item is re-exported at the crate root, so downstream code
//! keeps using `harmonigraph_scene::Camera` rather than module paths.

pub mod camera;
pub mod color;
pub mod derive;
pub mod octaves;
pub mod skin;
pub mod style;
pub mod trail;
pub mod view;

pub use camera::{Camera, Projection, Projector};
pub use color::{channel_color, pitch_lut_color, pitch_ramp_lut};
pub use derive::derive_scene;
pub use octaves::{
    clamp_center, clamp_wheel, octave_layout, OctaveLayout, Ring, DEFAULT_CENTER, DEFAULT_COUNT,
    DEFAULT_EXTRA_BLEND, DEFAULT_EXTRA_SIZE, MAX_EXTRAS, MAX_SPAN, MIDDLE_C_SLOT, MIN_COUNT,
    MIN_EXTRA_SIZE, MIN_SPAN, OCTAVE_SLOTS, PITCH_CEIL, PITCH_FLOOR,
};
pub use style::{
    HighlightExtremes, IdleMarker, NodeStyle, Pulse, SevensLabel,
};
pub use trail::TrailMark;
pub use view::{FrameParams, ViewConfig};

use glam::{Vec3, Vec4};
use harmonigraph_core::LatticePos;

/// Axis mapping, matching v1's orientation: major thirds run horizontally
/// (x), fifths vertically (y), and harmonic sevenths in depth (z).
fn lattice_to_world(pos: LatticePos, spacing: f32) -> Vec3 {
    Vec3::new(
        pos.fives as f32 * spacing,
        pos.threes as f32 * spacing,
        pos.sevens as f32 * spacing,
    )
}

/// Node radius as a fraction of the lattice spacing.
const NODE_RADIUS_FACTOR: f32 = 0.25;

/// Seconds an indicator on the outer layer eases in — the octave sectors
/// and the melody/bass rings alike. Keeps a fresh octave's color GROWING
/// into the gas swirl instead of instantly repainting its share of the disc
/// (and softens glyph pop-in); short enough to still feel immediate.
///
/// ONE time for both, because a ring and the sector it links back to belong
/// to the same note: easing them in at different rates would have the two
/// halves of one arrival disagree about when it happened.
///
/// Note that this is an attack on the *appearance*, not on the note — a
/// staccato note still reaches full brightness, since its release fades the
/// envelope over `fade_time` while this ramp is still climbing, and the
/// product peaks shortly after the key comes up.
const ATTACK_TIME: f64 = 0.15;

/// Samples in the pitch->color lookup EVERYTHING pitch-colored reads: the
/// disc, the trail and the piano roll on the CPU, the octave glyphs and their
/// glow in the shader. The shader mirrors this length, and `harmonigraph-render`
/// asserts that it does.
///
/// Because all of them read this one table, its size is not what makes two
/// shapes agree — that is structural (see `color::pitch_lut_color`). It buys
/// only the table's own fidelity to the designed curve, and it buys that
/// badly: the ramp's dark end rides the sRGB gamut boundary, where the LCh the
/// curve asks for is unrepresentable and the red channel sits pinned at 0
/// until t is about 0.2205 and then leaves it with a jump in slope. Linear
/// interpolation across a corner like that converges linearly at best, and
/// erratically in practice, since what dominates is whether a sample happens
/// to land near the corner rather than how many samples there are. Sweeping
/// the whole gradient range in 0.01-semitone steps, worst channel error is
/// 14.9/255 at 16 entries, 9.6 at 32, 3.6 at 64, 1.5 at 128 — but 4.9 at 130,
/// and still 2.4 at 256, four kilobytes in. So 64 is the knee: past it the
/// spend stops buying anything reliable, and the curve is already tracked far
/// closer than a viewer can see.
///
/// Do NOT read that error as a mismatch between shapes. It is the difference
/// between the table and an ideal nothing draws.
pub const PITCH_LUT_N: usize = 64;

/// One lattice node, ready for instanced rendering.
#[derive(Clone, Copy, Debug)]
pub struct NodeInstance {
    pub lattice_pos: LatticePos,
    pub world_pos: Vec3,
    /// RGBA base color (alpha unused for now).
    pub color: Vec4,
    /// 0 = idle, 1 = fully lit. Held notes are 1; released notes decay.
    pub activation: f32,
    /// Per-octave activation (slot = MIDI octave + 1, clamped into the span
    /// the view shows — see [`octaves`]): each octave's indicator fades on
    /// its own voice's envelope, independent of the node's overall
    /// activation. Slots outside the shown span stay 0, so a note beyond the
    /// range lights the outermost indicator on its side rather than
    /// disappearing.
    pub octaves: [f32; OCTAVE_SLOTS],
    /// Small constant seeding animation variety. NOT a timestamp — only
    /// ever used as a seed. A stable per-node hash for the field styles
    /// (see [`NodeStyle::is_field_style`]); Steady ignores it.
    pub seed: f32,
    /// Render as an outline instead of a filled disc (channel 14, v1's
    /// "channel 15" in MIDI convention).
    pub outlined: bool,
    pub hovered: bool,
    /// On the home (center sevens) sheet. Home nodes keep a blank
    /// placeholder ring while idle; off-sheet nodes draw nothing.
    pub on_home: bool,
    /// Billboard size, as a factor of the scene's `node_radius` (see
    /// [`ViewConfig::sevens_size`]): 1 on the home sheet, smaller with every
    /// step off it. Scales the whole node uniformly — the quad and its uv
    /// together — so every layer inside keeps its proportions and only the
    /// node's size on screen changes.
    pub scale: f32,
    /// Width of the knockout gutter this node clears around itself, in quad
    /// UV units (see [`ViewConfig::sevens_gutter`]). Every sounding node
    /// clears, the home sheet included and whatever depth the window holds;
    /// 0 on a silent node, and whenever the gutter is off.
    pub gutter: f32,
    /// Signed cents from the home-sheet node this one shares a LETTER and an
    /// accidental with: `(threes - 2*(sevens - center), fives, center)`,
    /// which is the position the letter walk lands on. Not the same NAME —
    /// the septimal mark is what tells the two apart, and this is the
    /// distance that mark stands for. The septimal comma — ±27¢ per step at
    /// just intonation, but it moves with the tuning. 0 on the home sheet.
    ///
    /// Derived here rather than at the label because the namesake can be
    /// outside the displayed window entirely, so the UI has no node to read
    /// it off; the tuning is right here and answers for any position.
    ///
    /// **Nothing draws this.** Its only reader was the retired
    /// `SevensLabel::Comma`, which put the number under the name; the name
    /// now carries the mark instead. Kept because the quantity is still the
    /// one the mark means, and the scene is where it can be answered for any
    /// position — but it is computed per off-sheet node per frame for tests
    /// alone, so anything that makes that cost matter should delete it.
    pub comma: f32,
    /// The node's pitch class in cents under the current tuning, for the
    /// in-lattice cents readout.
    pub cents: f32,
    /// Octave slots (bit i = slot i) where this node carries the melody —
    /// the highest held note. 0 when it doesn't, or when the melody isn't
    /// being marked. One voice lights every node its pitch class matches
    /// under the tuning tolerance, and the mark follows the same rule.
    ///
    /// A mask rather than a single slot because one node can hold both
    /// outer notes at once, in different octaves (a chord voiced inside a
    /// single pitch class), and the two must stay tellable apart.
    pub melody_slots: u32,
    /// The same for the bass — the lowest held note. A slot set in BOTH
    /// masks is a note that is at once the melody and the bass (a lone held
    /// note, or the two ends of a chord voiced inside one octave). The two
    /// marks are drawn as rings at different radii, so that costs nothing:
    /// they simply both draw. See [`ViewConfig::mark_melody`].
    pub bass_slots: u32,
    /// How far each mark has eased in, 0..1: a ring grows on over
    /// [`ATTACK_TIME`] from the moment its note TOOK that end, rather than
    /// appearing at full the frame it is claimed. Separate from
    /// `activation` because a mark can be arriving while the node it sits
    /// on has been fully lit for a while — the ring has to follow its own
    /// note, not the disc's.
    ///
    /// Only the fade IN: a mark is held-only (see [`derive`]), so it still
    /// comes off with the key rather than trailing the disc's release. The
    /// voice's envelope rides along all the same, for the day a released
    /// voice is allowed to keep an end.
    ///
    /// Per node rather than per slot because the mark is a ring around the
    /// whole node; the slots above only say which sector it links back to.
    pub melody_level: f32,
    pub bass_level: f32,
    /// Each mark's color: the color of the SECTOR it links back to — the pitch
    /// of that slot on this node, through [`color::pitch_lut_color`] — so a
    /// ring reads as belonging to the indicator it points at rather than as a
    /// fixed livery. Taken from the strongest marking voice (they can differ
    /// mid-crossfade). The ramp already bakes in the lift the disc/roll/glyphs
    /// carry (see `color::NOTE_LIGHTEN`), so the ring inherits it and adds
    /// nothing; a second one would leave the ring a shade whiter than the band.
    ///
    /// The slot's pitch rather than the marking VOICE's: a note past either end
    /// of the ring folds onto the outermost slot, and a ring carrying the
    /// unfolded pitch would then sit a register off the sector it brackets.
    pub melody_color: Vec4,
    pub bass_color: Vec4,
    /// How strongly the music is remembered here (see [`trail`]): 0 where
    /// it has never been, up to 1 where it has. Drives ONLY the idle
    /// marker, so a memory can never be mistaken for a sounding note; the
    /// label layer reads it too, to caption a visited node.
    ///
    /// While the mark is [`TrailMark::Tint`] this node's `color` carries
    /// the remembered note's color instead of the idle grey — but only when
    /// nothing sounds here, since a sounding note owns that field.
    pub trail: f32,
}

impl NodeInstance {
    /// Whether this node puts anything on screen, and so can carry pitch
    /// info (hover label, tuning readout). Sounding nodes always draw;
    /// idle ones only on the home sheet, where they keep a placeholder
    /// marker. Off-sheet idle nodes draw literally nothing, so revealing
    /// their pitch on mouse-over would be information from nowhere.
    ///
    /// Deliberately ignores [`Scene::idle_marker`] being `None`: that
    /// setting hides the idle markers but shouldn't make the home sheet
    /// unhoverable, which would leave an empty lattice uninspectable.
    /// A visited off-sheet node counts: it draws a trail marker where an
    /// unvisited one draws nothing, and the music having gone there is
    /// exactly what makes its pitch worth revealing.
    pub fn is_visible(&self) -> bool {
        self.activation > 0.0 || self.on_home || self.trail > 0.0
    }
}

/// One line segment of the lattice grid, between two adjacent positions
/// (one unit step along exactly one prime axis = one interval).
#[derive(Clone, Copy, Debug)]
pub struct EdgeInstance {
    pub a: Vec3,
    pub b: Vec3,
    pub color: Vec4,
    /// Line opacity.
    pub strength: f32,
    /// Render as short dashes (the sevens-axis links between sheets).
    pub dashed: bool,
}

/// Everything the renderer needs for one frame.
pub struct Scene {
    pub nodes: Vec<NodeInstance>,
    pub camera: Camera,
    /// Seconds for global shader animation, wrapped hourly so f32
    /// precision holds in long sessions. The field styles clock on this so
    /// their fields keep flowing across note events (at worst the pattern
    /// jumps once an hour at the wrap).
    pub time: f32,
    /// Base node radius in world units (scales with lattice spacing).
    pub node_radius: f32,
    pub node_style: NodeStyle,
    /// The core's radius in quad UV units; `0` turns the core off (nothing
    /// at all). Sizes both the disc and its glow; the shader reads solidity
    /// separately (`core_solidity`).
    pub core_radius: f32,
    /// The core's solidity 0..1 (see [`ViewConfig::core_solidity`]): 0 a
    /// soft glow, 1 the solid orb. Ignored when the core is off.
    pub core_solidity: f32,
    /// The outer octave layer's radial band (quad UV units), already
    /// sanitized: outer is always ahead of inner.
    pub outer_inner: f32,
    pub outer_outer: f32,
    /// Padding inside the octave layer (see [`ViewConfig::outer_gap`]):
    /// sector-to-sector and band-to-mark-ring alike. Already clamped.
    pub outer_gap: f32,
    /// The pitch axis the octave indicators are drawn on (see [`octaves`]):
    /// how many octaves one turn of a node covers, which pitch sits at the top
    /// of it, and how the circle is shared out between them. ONE set of widths
    /// for the whole frame — every node draws the same slices, turned by where
    /// its own octaves fall against the center pitch, which is what makes an
    /// indicator's ANGLE mean an absolute pitch.
    pub octave_layout: OctaveLayout,
    /// Which shimmer sweeps the octave glyphs (see [`Pulse`] and
    /// [`ViewConfig::pulse_octaves`]).
    pub pulse_octaves: Pulse,
    /// The idle (unlit home-sheet node) marker, independent of the active
    /// appearance and of the playing state; drawn in the idle grey and
    /// composited under any active note. See [`ViewConfig::idle_marker`].
    pub idle_marker: IdleMarker,
    pub idle_radius: f32,
    /// The faint background grid (see [`derive_grid`]): one segment per
    /// adjacent pair of visible positions, inset so every node position
    /// keeps a circular gap where its disc draws while sounding. Reuses
    /// [`EdgeInstance`]; `strength` carries the line opacity.
    pub grid: Vec<EdgeInstance>,
    /// Grid line thickness as a multiple of the shader's built-in grid
    /// width (see [`ViewConfig::grid_thickness`]), already clamped.
    pub grid_thickness: f32,
    /// How a node the music has already visited is marked (see
    /// [`TrailMark`]), and how strongly, 0..1. The renderer hands both to
    /// the shader's idle-marker branch; which NODES are marked rides on
    /// each node's own `trail`.
    pub trail_mark: TrailMark,
    pub trail_strength: f32,
    /// How wide the sevens knockout's fade is, in the uv of a full-size
    /// node (see [`ViewConfig::sevens_gutter_soft`]). View-wide, unlike the
    /// per-node reach, which the envelope and the node's own rim both bear
    /// on. Already clamped.
    pub sevens_soft: f32,
    /// The ground the lattice is drawn onto: the pane fill this pass gets
    /// composited over, which is the skin's `panel` (what `egui_dock`'s
    /// `tab_body.bg_fill` paints under every pane).
    ///
    /// Only the sevens knockout reads it, and it is the difference between
    /// a hole and a blob. The pass blends premultiplied, so a gutter with no
    /// color of its own knocks out to BLACK — and black is several shades
    /// darker than this skin's panel, so the cleared disc sat on the picture
    /// as an obviously darker plate instead of disappearing into the ground
    /// wherever it crossed nothing. Handing the ground in means the gutter
    /// is invisible over empty lattice and only shows as a clearing where it
    /// actually crosses something.
    pub background: Vec4,
    /// Color of the idle node markers (see [`ViewConfig::grid_color`]):
    /// the grid color's RGB at full alpha, so the idle structure reads as
    /// one layer. The renderer hands this to the shader.
    pub node_idle: Vec4,
    /// Melody/bass ring thickness in quad UV units, 0 = off (see
    /// [`ViewConfig::mark_thickness`]). Already clamped.
    pub mark_thickness: f32,
    /// Which shimmer sweeps the melody/bass rings (see [`Pulse`] and
    /// [`ViewConfig::pulse_marks`]).
    ///
    /// Folded to [`Pulse::Off`] when the rings are off
    /// ([`mark_thickness`](Self::mark_thickness) 0), where there is no ring
    /// to animate and the mark's own octave slice must not go on shimmering
    /// under a control the pane has grayed out.
    pub pulse_marks: Pulse,
    /// How fast the shimmer travels (world units per second), how wide its
    /// period is (world units), how deep the light it carries is (0 none, 1
    /// the tuned depth) and how gradually that light arrives across the
    /// period (0 a crest, 1 a cosine) — see [`ViewConfig::shimmer_speed`].
    /// ONE set for both layers that can run the sweep, it being a single
    /// sheet crossing the lattice. Already clamped, the width to strictly
    /// positive.
    pub shimmer_speed: f32,
    pub shimmer_width: f32,
    pub shimmer_intensity: f32,
    pub shimmer_softness: f32,
    /// Pitch->color lookup for the octave glyphs, matching the disc
    /// gradient; the renderer hands it to the shader (see [`pitch_ramp_lut`]).
    pub pitch_lut: [Vec4; PITCH_LUT_N],
    /// Gradient endpoints (MIDI notes) the shader maps a dot's pitch through
    /// to index `pitch_lut`; mirrors the disc coloring's `FrameParams`.
    pub darkest_pitch: f32,
    pub brightest_pitch: f32,
    /// Offscreen render resolution multiplier (see [`ViewConfig`]); the
    /// renderer sizes its offscreen color+depth target by this.
    pub render_scale: f32,
    /// Bloom intensity; 0 disables the whole post-process chain.
    pub bloom_strength: f32,
}

#[cfg(test)]
mod tests;
