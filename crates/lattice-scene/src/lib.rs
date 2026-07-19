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
//! - [`camera`] — [`Camera`], [`Projection`], and the [`Projector`] used
//!   for label placement and picking.
//! - [`color`] — the LCh pitch ramp and channel/idle colors.
//! - [`skin`] — the static palette the UI and renderer share.
//!
//! Every public item is re-exported at the crate root, so downstream code
//! keeps using `lattice_scene::Camera` rather than module paths.

pub mod camera;
pub mod color;
pub mod derive;
pub mod skin;
pub mod style;
pub mod view;

pub use camera::{Camera, Projection, Projector};
pub use color::{channel_color, pitch_ramp_lut};
pub use derive::derive_scene;
pub use style::{
    CoreStyle, HighlightExtremes, IdleMarker, LegacyNodeBody, MarkLink, NodeStyle, OuterStyle,
};
pub use view::{FrameParams, ViewConfig};

use glam::{Vec3, Vec4};
use lattice_core::LatticePos;

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

/// Octave indicator slots (MIDI octaves 0..=9).
pub const OCTAVE_SLOTS: usize = 10;

/// Seconds an octave indicator eases in after note-on. Keeps a fresh
/// octave's color GROWING into the gas swirl instead of instantly
/// repainting its share of the disc (and softens glyph pop-in);
/// short enough to still feel immediate.
const OCTAVE_ATTACK_TIME: f64 = 0.15;

/// Samples in the pitch->color lookup the octave glyphs use to tint each
/// slot by its own octave's pitch. The shader mirrors this length.
pub const PITCH_LUT_N: usize = 16;

/// One lattice node, ready for instanced rendering.
#[derive(Clone, Copy, Debug)]
pub struct NodeInstance {
    pub lattice_pos: LatticePos,
    pub world_pos: Vec3,
    /// RGBA base color (alpha unused for now).
    pub color: Vec4,
    /// 0 = idle, 1 = fully lit. Held notes are 1; released notes decay.
    pub activation: f32,
    /// Per-octave activation (slot = MIDI octave 0..=9, clamped): each
    /// octave's indicator fades on its own voice's envelope, independent of
    /// the node's overall activation.
    pub octaves: [f32; OCTAVE_SLOTS],
    /// Small constant seeding animation variety. NOT a timestamp — only
    /// ever used as a seed. A stable per-node hash for the field styles
    /// (see [`NodeStyle::is_field_style`]); Steady ignores it.
    pub seed: f32,
    /// Render as an outline instead of a filled disc (channel 14, v1's
    /// "channel 15" in MIDI convention).
    pub outlined: bool,
    pub hovered: bool,
    /// Depth-cue size multiplier (see [`depth_scale`]): nodes nearer the
    /// eye than the camera's focus distance grow, farther ones shrink,
    /// exaggerating the perspective so depth reads at a glance.
    pub scale: f32,
    /// On the home (center sevens) sheet. Home nodes keep a blank
    /// placeholder ring while idle; off-sheet nodes draw nothing.
    pub on_home: bool,
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
    /// they simply both draw. See [`HighlightExtremes`].
    pub bass_slots: u32,
    /// Envelope of each mark, 0..1, so a mark fades out with its note
    /// instead of snapping off at release. Separate from `activation`
    /// because the marked voice can be fading while a different, still-held
    /// voice keeps the NODE at full — the mark has to follow its own note.
    ///
    /// Per node rather than per slot because the mark is a ring around the
    /// whole node; the slots above only say which sector it links back to.
    pub melody_level: f32,
    pub bass_level: f32,
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
    pub fn is_visible(&self) -> bool {
        self.activation > 0.0 || self.on_home
    }
}

/// A glowing beam between two simultaneously sounding, lattice-adjacent
/// nodes (one unit step along exactly one prime axis = one interval).
#[derive(Clone, Copy, Debug)]
pub struct EdgeInstance {
    pub a: Vec3,
    pub b: Vec3,
    pub color: Vec4,
    /// min of the two nodes' activations: the beam fades with whichever
    /// endpoint fades first. (Grid segments: line opacity.)
    pub strength: f32,
    /// Grid segments only: render as short dashes (the sevens-axis links
    /// between sheets). Never set on chord beams.
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
    pub outer_style: OuterStyle,
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
    /// Outer-layer cohesion device (see [`ViewConfig::outer_backdrop`]):
    /// ghost the silent octaves faintly to complete the ring. Independent
    /// of the core. Already clamped to 0..1; 0 draws no backdrop at all.
    pub outer_backdrop: f32,
    /// The outer glyphs' solidity 0..1 (see [`ViewConfig::outer_solidity`]):
    /// 1 crisp, toward 0 the glyph edges spread into soft glows.
    pub outer_solidity: f32,
    /// The idle (unlit home-sheet node) marker, independent of the active
    /// appearance and of the playing state; drawn in the idle grey and
    /// composited under any active note. See [`ViewConfig::idle_marker`].
    pub idle_marker: IdleMarker,
    pub idle_radius: f32,
    /// Chord edges (empty when the toggle is off).
    pub edges: Vec<EdgeInstance>,
    /// The faint background grid (see [`derive_grid`]): one segment per
    /// adjacent pair of visible positions, inset so every node position
    /// keeps a circular gap where its disc draws while sounding. Segments
    /// between two sounding notes light up with their blended color.
    /// Reuses [`EdgeInstance`]; `strength` carries the line opacity.
    pub grid: Vec<EdgeInstance>,
    /// Grid line thickness as a multiple of the shader's built-in grid
    /// width (see [`ViewConfig::grid_thickness`]), already clamped.
    pub grid_thickness: f32,
    /// Color of the idle node markers (see [`ViewConfig::grid_color`]):
    /// the grid color's RGB at full alpha, so the idle structure reads as
    /// one layer. The renderer hands this to the shader.
    pub node_idle: Vec4,
    /// How the melody/bass rings point back at the sector they came from
    /// (see [`MarkLink`]). Which NOTES are marked is baked into each node's
    /// `melody_slots`/`bass_slots`.
    pub mark_link: MarkLink,
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
