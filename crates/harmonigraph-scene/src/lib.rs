//! The scene layer: turns core state (note tracker + tuning) into a
//! render-friendly description, once per frame. The renderer consumes a
//! [`Scene`] and knows nothing about MIDI; the core knows nothing about
//! cameras or colors. Animation/envelope *policy* lives here.
//!
//! What lives where:
//! - `lib.rs` (this file) — the render-facing types: [`Scene`],
//!   [`NodeInstance`], [`EdgeInstance`], and the constants they share.
//! - [`derive`](mod@derive) — the per-frame derivation ([`derive_scene`]): note tracker
//!   + tuning -> node/edge lists. Envelope and animation policy.
//! - [`view`] — [`ViewConfig`] (persisted visual settings and their serde
//!   defaults) and [`FrameParams`].
//! - [`style`] — the visual-style enums and their shader indices, and the
//!   pitch gradient's knobs.
//! - [`octaves`] — where the octave indicators sit around a node: how many,
//!   how wide, and the boundary angles the shader draws them between.
//! - [`camera`] — [`Camera`], [`Projection`], and the [`Projector`] used
//!   for label placement and picking.
//! - [`color`] — the pitch ramp every note is colored off, and the idle color.
//! - [`spectral`] — the lattice's audio channel, and the second of the two
//!   colour schemes the plugin has (the analyzer's, by loudness).
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
pub mod spectral;
pub mod style;
pub mod trail;
pub mod view;

pub use camera::{Camera, Projection, Projector, VisibleSheet};
pub use color::{gradient_color, hue_circle, pitch_lut_color, pitch_ramp_lut, HUE_CIRCLE_N};
pub use derive::derive_scene;
pub use octaves::{
    clamp_center, clamp_wheel, octave_layout, OctaveLayout, Ring, DEFAULT_CENTER, DEFAULT_COUNT,
    DEFAULT_EXTRA_BLEND, DEFAULT_EXTRA_SIZE, MAX_EXTRAS, MAX_SPAN, MIDDLE_C_SLOT, MIN_COUNT,
    MIN_EXTRA_SIZE, MIN_SPAN, OCTAVE_SLOTS, PITCH_CEIL, PITCH_FLOOR,
};
pub use spectral::{
    bucket_pitch, ring_gradient, SpectralLevels, SpectralPaint, SpectralReading, SPECTRAL_AXIS,
    SPECTRAL_BUCKETS, SPECTRAL_BUCKETS_PER_SEMITONE, SPECTRAL_RANGE_MAX, SPECTRAL_RANGE_MIN,
};
pub use style::{Gradient, Pulse, SevensLabel};
pub use view::{DrawnWindow, FrameParams, RingStack, ViewConfig};

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

/// The most nodes one drawn window may hold, and the backstop on a lattice
/// with no size of its own: the window is derived from the viewport
/// ([`ViewConfig::scrolled`]), so what decides how much work a frame is is the
/// camera — and there are cameras with no bounded answer at all.
///
/// Which is worth being exact about, because it is the whole reason this
/// exists. Tilt a camera toward the sheet's own plane and the sheet goes
/// edge-on: the far half of an unbounded plane is genuinely on screen, in a
/// band a few pixels tall. At the pitch limit, fully zoomed out, on a 16:9
/// pane with the sheets on, orthographic asks for 261837 nodes that way and
/// perspective for 1294821; swept over the whole camera space a flat
/// perspective window saturates `view`'s `MAX_DRAWN_EXTENT` clamp on both
/// sheet axes at every aspect, which is 67125249 nodes. There is nothing to
/// draw there and no window that would be right.
///
/// Which is also why a pane whose view of the sheets runs off to the horizon
/// is drawn about its CENTER rather than out to a far edge: past about 45° of
/// pitch there is no such edge, and the rectangle
/// [`Camera::visible_world_bounds`] hands back has one only because a corner's
/// line was followed backwards through the eye to find it. `scrolled` mirrors
/// that case and lets this cap ration it — see [`VisibleSheet::bounded`].
///
/// Cabinet has no such camera — it faces the sheet by construction, whatever
/// the orbit says — so its window is bounded at every setting, and this sits
/// above the largest one a pane of ordinary shape reaches: 19251 nodes, at
/// 3:1, fully zoomed out, nine sheets deep, at cavalier scale. Bounded is not
/// the same as under the cap, and the difference is measured rather than
/// assumed — past about 3.3:1 with full depth cabinet does reach this, and
/// there is a pane that gets there without hand-editing anything, since
/// `Layout::split` will give the lattice a fifth of a 21:9 frame. What the
/// trim costs there is 5634 nodes at 4:1 and 15912 at 6:1, off all four
/// edges. A cap that covered it would have to be twice this one, for a band
/// of lattice eight steps tall.
///
/// Cabinet's figures are the same under a window with bounds of its own as
/// under the mirrored one it replaced — measured equal at every aspect, depth
/// and shear, because a cabinet camera's view of the sheet is symmetric about
/// the origin to begin with. It is perspective that the bounds save, and they
/// save it 2.7x at a middling tilt — at the tilts where the pane's view of the
/// sheets has a far edge at all, which is every one a lattice is read at.
///
/// The cost it is holding: `derive_scene` with ten voices held takes 0.02ms
/// over 273 nodes, 0.06ms over 875 (a 16:9 pane fully zoomed out, flat), and
/// 1.2ms over 14877.
pub const MAX_DRAWN_NODES: usize = 20480;

/// The longest wait the Delay bar offers before a melody/bass mark starts
/// easing in ([`ViewConfig::mark_delay`]), and the clamp `derive_scene` holds
/// a hand-edited view to. ONE constant for the two so the bar's end and the
/// picture's cannot drift apart — which is not the same as saying a view out
/// of range reads correctly: the bar fills to its end and reads out the value
/// it actually holds, so a blob carrying five seconds says "5.00 s" over a
/// full bar while the marks behave as one. Dragging it writes a value in
/// range and the two agree again.
///
/// A second, because that is where the setting stops being about flicker and
/// starts being about tempo: at 120bpm it marks only what is held for a whole
/// beat, and the useful settings — a passing sixteenth is 125ms — sit in the
/// first quarter of the bar. Past a second a chord would have to be held
/// deliberately still before its ends read at all, which is a different
/// instrument rather than more of this one.
pub const MARK_DELAY_MAX: f32 = 1.0;

/// The narrowest and widest the spectral kernel can be set to
/// ([`ViewConfig::spectral_width`]), in cents — the bar's two ends, and what
/// `sanitize` holds a hand-edited view to.
///
/// The floor is a third of an analyzer bucket (3.125¢), which is as narrow as
/// asking is worth: at 1¢ the kernel already sits inside one bucket, reading a
/// single 3.125¢ column of the spectrum — precisely the exposure the whole
/// design avoids — and narrower buys nothing a column does not already give.
/// The ceiling is a comfortable quarter-tone,
/// wide enough to take in a tempered seventh's 31¢ miss with room over — past
/// that the kernel starts admitting the NEXT lattice node's partials as well as
/// this one's, and the constellation smears into a glow.
pub const SPECTRAL_WIDTH_MIN: f32 = 1.0;
/// See [`SPECTRAL_WIDTH_MIN`].
pub const SPECTRAL_WIDTH_MAX: f32 = 50.0;

/// The ends of the four bars that size a node's layers, in quad UV units — the
/// stack [`ViewConfig::rings`] reads outward from the center, and what it holds
/// a hand-edited view to.
///
/// Every one of them has 0 for its low end, and 0 is that layer's off position
/// rather than a hairline of it: one way to turn a layer off, in the same place
/// on every layer. What the high ends buy is a bar whose useful settings are
/// spread over its whole travel — the quad is 1 across, so a ring bar reaching
/// 1 would spend most of itself on stacks that are already off the node's edge.
///
/// [`RING_WIDTH_MAX`] is the same for the audio ring and the octave band,
/// because they are read the same way: a wedge whose radial extent is a
/// hairline says nothing about how loud it is, whichever ring it is on, and
/// neither is worth more of the quad than the other. The core keeps the widest
/// bar of the four, being the one layer that starts at the center and so the
/// only one that can fill the node by itself.
pub const CORE_RADIUS_MAX: f32 = 0.9;
/// See [`CORE_RADIUS_MAX`].
pub const RING_WIDTH_MAX: f32 = 0.6;
/// See [`CORE_RADIUS_MAX`].
pub const MARK_THICKNESS_MAX: f32 = 0.3;
/// See [`CORE_RADIUS_MAX`]. The one of the four that is a padding rather than a
/// layer, and it is spent up to three times over on one node (between the core
/// and the audio ring, that ring and the band, and the band and the marks), so
/// its bar is short: at the top of it the gaps alone are more than the quad.
pub const GAP_MAX: f32 = 0.4;

/// How far from a node's center anything on it is still drawn, in the same
/// quad units as the four sizes above: the billboard's own reach, past which
/// the melody/bass strip has been eased to nothing and every other layer
/// finished long before.
///
/// **A mirror of the lattice shader's `QUAD_MARGIN`**, which is where the
/// number is used — the billboard is scaled by it and the mark strip is faded
/// out over its last few hundredths. Kept here because it is the top of the
/// Layers bar's axis, and that axis is the node's whole drawn reach: the quad
/// edge at 1.0 is where a RING stops fitting, and the room past it is the
/// margin the marks alone are allowed into. A bar that stopped at the quad
/// would have no room to put the outer half of that strip.
///
/// The one number here the shader can change out from under: the two are
/// separate declarations of one constant. Change either and change both — what
/// drifting costs is the bar's last stretch describing room the marks cannot
/// reach, or stopping short of room they can.
pub const QUAD_MARGIN: f32 = 1.6;

/// Samples in the pitch->color lookup EVERYTHING pitch-colored reads: the
/// disc, the trail and the piano roll on the CPU, the octave glyphs and their
/// glow in the shader. The shader mirrors this length, and `harmonigraph-render`
/// asserts that it does.
///
/// Because all of them read this one table, its size is not what makes two
/// shapes agree — that is structural (see `color::pitch_lut_color`). It buys
/// only the table's own fidelity to the designed curve, and it buys that
/// unevenly, because the curve is not smooth. Its chroma follows the sRGB
/// gamut's own boundary (see [`Gradient::chroma`]), and that boundary is
/// the surface of a CUBE: where the widest chroma at a lightness passes from
/// one face of it to another, the maximum has a corner, and so does the curve
/// riding at a fixed fraction of it. Linear interpolation across a corner
/// converges linearly at best, and a little erratically, since what dominates
/// is whether a sample happens to land near one rather than how many samples
/// there are.
///
/// Sweeping the default gradient's whole range, worst channel error is 8.0/255
/// at 16 entries, 7.5 at 32, 5.6 at 48, 3.4 at 64, 1.3 at 96 — but 2.4 at 128,
/// and 1.4 at 192 before 0.3 at 256, four kilobytes in. The sequence is not
/// monotone and that is the point: the worst case is set by where a sample
/// lands relative to a CORNER of the gamut's own boundary, not by how many
/// samples there are, so more entries can measure worse. Past 64 the spend
/// buys a fraction of a level that no display step can show, and the curve is
/// already tracked far closer than a viewer can see.
///
/// 3.4/255 is what 64 entries buy on THIS curve, and a curve authored in
/// CIELAB rather than Oklab would put the same table at 1.8: the arc is long
/// in Oklab degrees and its chroma ceiling turns fast along it, so what a
/// table of any size is chasing simply bends more. 3.4/255 is 1.3% of a
/// channel, on a gradient whose neighbouring entries differ by more than that
/// — and it is a distance from an IDEAL, not a disagreement between two
/// shapes, which stays exact at every table size.
///
/// A gradient can be dialled to a harsher curve than the default — chroma at
/// 1.0 rides the boundary itself, corners and all, rather than half way in —
/// which is the case for leaving headroom here rather than trimming to what
/// the default alone needs.
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
    /// Whether the voice this node is lit by is on its way OUT — its key is
    /// up and its departure has begun.
    ///
    /// [`activation`](Self::activation) alone cannot answer this: it is the
    /// arrival times what is left of the departure, so a level part-way up
    /// and the same level part-way down are one number. The two ends run on
    /// one dial and never overlap, which is what makes a single flag enough
    /// to tell them apart — a voice is arriving, or full, or departing.
    ///
    /// What needs it is anything that reads a low activation as "nearly
    /// gone" and acts on it, which is only true on the way out. The kept
    /// note names are the case: their level is reserved ahead of the trail
    /// record that takes over when the release finishes, and reserving that
    /// on the way IN draws a name ahead of the note it names.
    pub departing: bool,
    /// Per-octave activation (slot = MIDI octave + 1, clamped into the span
    /// the view shows — see [`octaves`]): each octave's indicator fades on
    /// its own voice's envelope, independent of the node's overall
    /// activation. Slots outside the shown span stay 0, so a note beyond the
    /// range lights the outermost indicator on its side rather than
    /// disappearing.
    pub octaves: [f32; OCTAVE_SLOTS],
    pub hovered: bool,
    /// On the home (center sevens) sheet. An idle node draws nothing
    /// wherever it sits; what marks a home position is the GRID, whose
    /// lines stop short of it on every side, and off-sheet positions have
    /// not even that (see [`derive_grid`](derive::derive_grid)).
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
    /// One bit at a time: the node carries one melody mark at one level, so
    /// the mask names the one sector that mark extends (see
    /// `derive::Mark`). A MASK rather than a slot index because 0 then says
    /// "unmarked" on its own, which a `0` index could not — and because it is
    /// what the shader tests a slot against.
    pub melody_slots: u32,
    /// The same for the bass — the lowest held note. A slot set in BOTH
    /// masks is a note that is at once the melody and the bass (a lone held
    /// note, or the two ends of a chord voiced inside one octave). The two
    /// marks are then one slice extended once, in the one colour they both
    /// carry — the mark says that slice is an end of the chord, and it is
    /// both. See [`ViewConfig::mark_melody`].
    pub bass_slots: u32,
    /// How far each mark has eased in, 0..1: a mark grows on over the Fade
    /// duration ([`FrameParams::fade_time`]) from the moment its note TOOK
    /// that end (plus whatever wait [`ViewConfig::mark_delay`] asks for
    /// first), rather than appearing at full the frame it is claimed.
    /// Separate from `activation` because a mark can be arriving while the
    /// node it sits on has been fully lit for a while — the mark has to
    /// follow its own note, not the disc's.
    ///
    /// Both directions: the ease in above, times what is left of the note's
    /// own release, so a mark leaves with its note rather than snapping off
    /// at the key (see [`derive`](mod@derive)). [`ViewConfig::mark_delay`] is
    /// answered as a threshold AT the key-up — a mark that had not earned its
    /// way past the wait must not climb into one while the note is already
    /// fading — and the ramp itself then runs on at the current frame, like
    /// the sector's, so the two halves of one arrival never disagree about
    /// how fast it happened.
    ///
    /// Per node rather than per slot because one node carries at most one
    /// mark of each kind; the slots above say which sector it extends.
    pub melody_level: f32,
    pub bass_level: f32,
    /// Each mark's color: the color of the SECTOR it extends — the pitch
    /// of that slot on this node, through [`color::pitch_lut_color`] — so a
    /// mark reads as that indicator continued rather than as a fixed livery.
    /// Taken from the strongest marking voice (they can differ
    /// mid-crossfade). No lift on top of the ramp: the disc, the roll and the
    /// glyphs all wear it as the table hands it over, whatever the gradient's
    /// brightness is dialled to, so a mark that lightened its own copy would
    /// sit a shade whiter than the slice it continues.
    ///
    /// The slot's pitch rather than the marking VOICE's: a note past either end
    /// of the ring folds onto the outermost slot, and a mark carrying the
    /// unfolded pitch would then sit a register off the sector it extends.
    pub melody_color: Vec4,
    pub bass_color: Vec4,
    /// How strongly the music is remembered here (see [`trail`]): 0 where
    /// it has never been, up to 1 where it has.
    ///
    /// Read by the LABEL layer alone, which is what makes a memory
    /// unmistakable for a sounding note — the two are not the same kind of
    /// thing on screen. No drawn layer looks at it, and this node's other
    /// fields are untouched by the trail: a remembered node's `color` and
    /// `activation` are the ones it would carry having never been played.
    pub trail: f32,
}

impl NodeInstance {
    /// Whether this node is somewhere the picture accounts for, and so can
    /// carry pitch info (hover label, tuning readout). Sounding nodes always
    /// draw; an idle one draws nothing at all, but a home-sheet position is
    /// still a place the grid lines say is there — they stop short of it on
    /// every side, which is exactly the gap a pointer goes looking in.
    ///
    /// So this is deliberately NOT "does this node paint a pixel": an empty
    /// home sheet would then be uninspectable, and it is the thing most worth
    /// inspecting. Off-sheet idle positions have no lines around them and are
    /// correspondingly not hoverable — a pitch revealed there would be
    /// information from nowhere.
    ///
    /// The `trail` term decides nothing today and is a guard rather than a
    /// case: [`trail::TrailField::apply`] writes the field only where
    /// `on_home` holds, so a trailed node is a home node and the middle term
    /// has already answered. It stands because the restriction is the trail's
    /// and not this predicate's — the day a memory is shown where it was
    /// actually played, the node carrying it is one to reveal the pitch of,
    /// and that is the reasoning here rather than in the caller.
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
    /// Seconds for global shader animation, and NOT wrapped: the shimmer is
    /// the only thing that clocks on it, and it reaches the shader already
    /// reduced against its own period (see
    /// [`shimmer_slide`](Self::shimmer_slide)). Its sheet is one field
    /// spanning the whole lattice, so every node must read the same clock.
    ///
    /// f64 because a transport position is a song position: an hour in, an
    /// f32 second is quantized to 0.0005 s, and the reduction below is what
    /// the picture is built from. Wrapping this instead — an hourly `now %
    /// 3600`, which is what a shader-side clock would need — puts a seam in
    /// the sheet at every setting whose period does not divide the wrap,
    /// which is most of them.
    pub now: f64,
    /// Base node radius in world units (scales with lattice spacing).
    pub node_radius: f32,
    /// The core's radius in quad UV units; `0` turns the core off (nothing
    /// at all). Sizes both the disc and its glow; the shader reads solidity
    /// separately (`core_solidity`).
    pub core_radius: f32,
    /// The core's solidity 0..1 (see [`ViewConfig::core_solidity`]): 0 a
    /// soft glow, 1 the solid orb. Ignored when the core is off.
    pub core_solidity: f32,
    /// The outer octave layer's radial band (quad UV units), already
    /// sanitized: outer is always ahead of inner on a band that draws, and
    /// both are 0 when the layer is off (see [`ViewConfig::band_width`]).
    pub outer_inner: f32,
    pub outer_outer: f32,
    /// The outer edge of the outermost RING the node draws — the band's, save
    /// where the band is off and something inside it is the last layer on (see
    /// [`RingStack::outer`]).
    ///
    /// What the melody/bass marks stand off and what the node's gutter is
    /// measured from, so a node whose band is dialled away still wears both
    /// where its picture actually ends.
    pub rings_outer: f32,
    /// Where the melody/bass mark strip starts (see
    /// [`RingStack::mark_inner`]) — a padding out from
    /// [`rings_outer`](Self::rings_outer), or the node's center on a node with
    /// no rings at all, where there is nothing for it to stand off.
    ///
    /// Handed over rather than re-derived from `rings_outer + ring_gap`,
    /// because that sum is only right while some ring is there to owe the
    /// padding to.
    pub mark_inner: f32,
    /// The one padding on a node (see [`ViewConfig::ring_gap`]): between two
    /// stacked rings, between one octave sector and the next, and between the
    /// outermost ring and the mark strip alike. Already clamped.
    pub ring_gap: f32,
    /// The lattice's AUDIO channel: what the analyzer measured, where the
    /// ring that draws it sits, and the ramp every audio-lit element on the
    /// node is painted from (see [`spectral`]).
    ///
    /// [`SpectralPaint::silent`] from [`derive_scene`], because nothing in
    /// this crate reads audio; `harmonigraph-ui`'s `panes::spectral_fold` is
    /// the one pass that fills it, and a scene drawn without that pass is the
    /// MIDI picture alone.
    ///
    /// By value, with the analyzer's GRID boxed inside it: the grid is the
    /// only part big enough to be worth an allocation, and holding the whole
    /// struct behind a pointer would make replacing it an allocation a frame
    /// per pane for the sake of the kilobyte of ramp beside it.
    pub spectral: SpectralPaint,
    /// The pitch axis the octave indicators are drawn on (see [`octaves`]):
    /// how many octaves one turn of a node covers, which pitch sits at the top
    /// of it, and how the circle is shared out between them. ONE set of widths
    /// for the whole frame — every node draws the same slices, turned by where
    /// its own octaves fall against the center pitch, which is what makes an
    /// indicator's ANGLE mean an absolute pitch.
    pub octave_layout: OctaveLayout,
    /// The faint background grid (see [`derive_grid`](derive::derive_grid)): one segment per
    /// adjacent pair of visible positions, inset so every node position
    /// keeps a circular gap where its disc draws while sounding. Reuses
    /// [`EdgeInstance`]; `strength` carries the line opacity.
    pub grid: Vec<EdgeInstance>,
    /// Grid line thickness as a multiple of the shader's built-in grid
    /// width (see [`ViewConfig::grid_thickness`]), already clamped.
    pub grid_thickness: f32,
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
    /// How deep the melody/bass mark strip is, in quad UV units; 0 = off (see
    /// [`ViewConfig::mark_thickness`]). It starts one [`ring_gap`](Self::ring_gap)
    /// past [`rings_outer`](Self::rings_outer). Already clamped.
    pub mark_thickness: f32,
    /// Which shimmer sweeps the melody/bass marks (see [`Pulse`] and
    /// [`ViewConfig::pulse_marks`]).
    ///
    /// Folded to [`Pulse::Off`] when the marks are off — which is
    /// [`ViewConfig::marks_draw`], a thickness of 0 OR neither end
    /// switched on, not the thickness alone — where there is no mark
    /// to animate and the mark's own octave slice must not go on shimmering
    /// under a control the pane has grayed out.
    pub pulse_marks: Pulse,
    /// How fast the shimmer travels (world units per second), how wide its
    /// period is (world units), how deep the light it carries is (0 none, 1
    /// the tuned depth) and how gradually that light arrives across the
    /// period (0 a crest, 1 a cosine) — see [`ViewConfig::shimmer_speed`].
    /// Already clamped, the width to strictly positive.
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

/// How far the shimmer's sheet has travelled, in world units, reduced onto
/// one cycle of its own pattern.
///
/// The shader wants `now * speed`, and every pattern it builds is periodic in
/// that quantity, so it can have it modulo a cycle instead — the same picture,
/// out of a number that stays small. Which is the whole point of doing it
/// here:
///
/// - **f64, from the unwrapped clock.** The reduction is exact against a
///   period the Spacing bar can set as low as 0.02, where the f32 product an
///   hour into a song would have quantized the phase into about two dozen
///   steps per band and stair-stepped visibly.
/// - **No seam.** A clock wrapped for the shader's sake — hourly, say — lands
///   mid-band unless the settings happen to divide the wrap, and 3600 being
///   highly composite that is true of a lot of round pairs and none of the
///   rest, so the sheet would jump at some settings and not others. Reduced
///   against the pattern's OWN period there is nothing to land mid-band.
///
/// TWO periods, not one, and the factor is load-bearing: Hex crosses three
/// gratings sixty degrees apart, and the outer two take the travel through a
/// `cos 60°` — so they run at half the sheet's own frequency along their axes
/// and only close a cycle over two of its periods. Reduce by one and Hex flips
/// sign at every wrap. The other two patterns take the travel whole and
/// repeat over either.
impl Scene {
    pub fn shimmer_slide(&self) -> f32 {
        // The same floor the shader puts under the period, so a hand-built
        // Scene reduces against the width the pattern is actually drawn at.
        let cycle = 2.0 * (self.shimmer_width as f64).max(0.01);
        let slide = (self.now * self.shimmer_speed as f64).rem_euclid(cycle);
        // A clock or a speed that is not finite reaches here as a NaN, and a
        // NaN slide is a lattice of NaN colors rather than a wrong sheet.
        if slide.is_finite() { slide as f32 } else { 0.0 }
    }
}

#[cfg(test)]
mod tests;
