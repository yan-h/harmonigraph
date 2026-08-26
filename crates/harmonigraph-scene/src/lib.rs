//! The scene layer: turns core state (note tracker + tuning) into a
//! render-friendly description, once per frame. The renderer consumes a
//! [`Scene`] and knows nothing about MIDI; the core knows nothing about
//! cameras or colors. Animation/envelope *policy* lives here.
//!
//! What lives where:
//! - `lib.rs` (this file) — the render-facing types: [`Scene`],
//!   [`NodeInstance`], [`PlusInstance`], and the constants they share.
//! - [`derive`](mod@derive) — the per-frame derivation ([`derive_scene`]): note tracker
//!   + tuning -> node/marker lists. Envelope and animation policy.
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
pub use color::{
    gradient_color, grey_of_lightness, hue_circle, pitch_lut_color, pitch_ramp_lut, HUE_CIRCLE_N,
};
pub use derive::derive_scene;
pub use octaves::{
    clamp_center, clamp_wheel, octave_layout, OctaveLayout, Ring, DEFAULT_CENTER, DEFAULT_COUNT,
    DEFAULT_EXTRA_BLEND, DEFAULT_EXTRA_SIZE, MAX_EXTRAS, MAX_SPAN, MIDDLE_C_SLOT, MIN_COUNT,
    MIN_EXTRA_SIZE, MIN_SPAN, OCTAVE_SLOTS, PITCH_CEIL, PITCH_FLOOR,
};
pub use spectral::{
    bucket_pitch, ring_gradient, RingFade, RingGate, SpectralLevels, SpectralPaint,
    SpectralReading, SPECTRAL_AXIS, SPECTRAL_BALLISTICS_MAX, SPECTRAL_BUCKETS,
    SPECTRAL_BUCKETS_PER_SEMITONE, SPECTRAL_GATE_MAX, SPECTRAL_GATE_MIN, SPECTRAL_HYSTERESIS_MAX,
    SPECTRAL_RANGE_MAX, SPECTRAL_RANGE_MIN,
};
pub use style::{Gradient, NoteNames, Pulse, SevensLabel};
pub use view::{DrawnWindow, FrameParams, RingStack, ViewConfig};

use glam::{Vec3, Vec4};
use harmonigraph_core::{Envelope, LatticePos};

/// Axis mapping, matching v1's orientation: major thirds run horizontally
/// (x), fifths vertically (y), and harmonic sevenths in depth (z).
fn lattice_to_world(pos: LatticePos, spacing: f32) -> Vec3 {
    Vec3::new(pos.fives as f32 * spacing, pos.threes as f32 * spacing, pos.sevens as f32 * spacing)
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

/// The ends of the bars that size a node's layers, in quad UV units — the
/// stack [`ViewConfig::rings`] reads outward from the center, and what it holds
/// a hand-edited view to.
///
/// Every one of them has 0 for its low end, and on a WIDTH 0 is that layer's
/// off position rather than a hairline of it: one way to turn a layer off, in
/// the same place on every layer. What the high ends buy is a bar whose useful
/// settings are spread over its whole travel — the quad is 1 across, so a ring
/// bar reaching 1 would spend most of itself on stacks that are already off the
/// node's edge.
///
/// [`RING_WIDTH_MAX`] is the same for the audio ring and the octave band,
/// because they are read the same way: a wedge whose radial extent is a
/// hairline says nothing about how loud it is, whichever ring it is on, and
/// neither is worth more of the quad than the other.
pub const RING_WIDTH_MAX: f32 = 0.6;
/// See [`RING_WIDTH_MAX`].
pub const MARK_THICKNESS_MAX: f32 = 0.3;
/// See [`RING_WIDTH_MAX`]. The one bar of the stack that is a POSITION rather
/// than a width — how far out the innermost layer begins
/// ([`ViewConfig::ring_inner`]) — which is why its low end is not an off
/// position: nothing is switched off there, the stack simply seats on the
/// node's own center.
///
/// Short of the quad edge by a tenth, so the top of the bar is a node whose
/// rings are being refused one at a time rather than one with nowhere at all to
/// put them.
pub const RING_INNER_MAX: f32 = 0.9;
/// See [`RING_WIDTH_MAX`]. The ceiling on BOTH of a node's paddings, which are
/// a padding rather than a layer and so share one.
///
/// It is the RADIAL one ([`ViewConfig::ring_gap`]) the number is sized for,
/// that being the one spent out of the quad, and spent twice over on one node
/// (between the audio ring and the band, and between the band and the marks):
/// at the top of the bar the gaps alone are two fifths of it. The ANGULAR one
/// ([`ViewConfig::octave_gap`]) costs the stack
/// nothing and wants a ceiling for a different reason — a gap of a whole
/// sector's arc is every indicator erased — and lands near enough the same
/// place that a second constant would be two numbers saying one thing.
pub const GAP_MAX: f32 = 0.2;

/// How far a resting marker may be asked to reach on EITHER of its two axes —
/// the length of an arm ([`ViewConfig::plus_arm`]) and the thickness across
/// one ([`ViewConfig::plus_width`]) — in the same quad UV units the layer
/// sizes above are in, so a marker and a ring radius are two readings on one
/// axis and can be compared by their numbers.
///
/// ONE constant under both bars rather than two: the two numbers are lengths
/// on the same axis, and a second ceiling would be two numbers saying one
/// thing (as [`GAP_MAX`] says of the pair above it). It does not make the two
/// bars read alike — a length is measured from the crossing OUT and a width
/// ACROSS an arm, so a plus has filled its own square once the width reaches
/// twice the length, and the rest of the width bar is that same square.
///
/// Sized against [`RING_INNER_MAX`] rather than under it: a marker is not part
/// of the ring stack and owes it no room, so at the top of the arm bar it
/// reaches past the middle a node's rings stand around, and the lattice at
/// rest is a field of crosses rather than of points. That is the far end being
/// a different picture, which is what a bar's far end is for; the small marker
/// the fresh view draws is a fifth of the way along it.
pub const PLUS_SIZE_MAX: f32 = 0.9;

/// How far past a node's outermost drawn edge its glow may be asked to reach
/// (see [`ViewConfig::glow_reach`]), in the same quad UV units the layer sizes
/// above are in.
///
/// Sized for the light that stops being an ACCENT. A uv of 1 is 0.45 of the
/// step between two neighbouring nodes — the node's own radius is
/// `NODE_RADIUS_FACTOR` of that step and its quad is 1.8 radii across at uv 1
/// (`node_vertex`) — so a reach of about one covers the gap to a neighbour and
/// no further, which is a halo on each node. Eight crosses three or four
/// lattice steps: every node's light overlaps every node's light for a
/// neighbourhood around it, so nothing in that layer is a node any more, and
/// what the pane draws is a coloured field with the lattice sitting in it.
/// That is a different picture rather than more of the same one, and it is the
/// one the far end of this bar is for.
///
/// What it costs is fill rate, and the bar is where it is spent: the glow's
/// draws size their quad to hold the whole halo (`quad_margin` in
/// lattice.wgsl), so a node's billboard is as wide as its rim plus this and the
/// fragments in it go as the square — about twenty times as many at the top of
/// the bar as at a reach of one. Cheap fragments, the ink strip having already
/// answered the colour, so what that comes to depends on how many nodes are
/// lit at once: measured off `the_node_glow_draws_a_picture` at 1200x1000, a
/// chord's worth of light costs the same at 8 as at the fresh 0.35 (5.4 ms a
/// frame either way), and a lattice with thirty-odd nodes lit goes from 6.6 ms
/// to 12.2. A bar to turn up while watching the frame rate, in other words,
/// rather than a number the renderer defends.
pub const GLOW_REACH_MAX: f32 = 8.0;

/// What the node glow scales its own skirt by
/// (see [`ViewConfig::glow_strength`]).
///
/// Its own ceiling rather than [`ViewConfig::bloom_strength`]'s, because it is
/// a different light: the bloom's chain thresholds, so its strength acts on the
/// bright end alone, where this one scales one node's own skirt. Two is where
/// the travel stops being about the light and starts being about the clamp —
/// the base (`GLOW_BASE` in lattice.wgsl) is picked so that 1 reads plainly,
/// which puts the middle of a node at saturation somewhere short of this.
pub const GLOW_STRENGTH_MAX: f32 = 2.0;

/// How far past each ring a node's clearing may be asked to stand the light
/// off (see [`ViewConfig::glow_shadow`]), in the quad UV units the layer sizes
/// above are in — the far end of the Glow section's Shadow bar, and with it the
/// widest fade that bar can spend, the fade being held inside the shadow.
///
/// A whole node radius, where the two paddings above stop at [`GAP_MAX`]: a
/// padding is room between two layers of one node and has no business being
/// wider than a layer, but the standoff is a dip in the LIGHT, and what stops
/// it reading as a black ring is a dip broad enough to come off at the rate
/// the skirt does. One radius past every ring is a band wider than the whole
/// ring stack it stands off, past which there is no more shape to soften.
pub const GLOW_SHADOW_MAX: f32 = 1.0;

/// The two ends of the Glow section's Feather bar, as the rate the light's
/// exponential comes off at across its own span. Mirrored in lattice.wgsl
/// (`GLOW_FALLOFF_TIGHT`/`FLAT`), which holds the rationale for the numbers
/// and is the copy the picture is drawn from; `harmonigraph-render` asserts the
/// two agree. This copy is what the bar's own preview is drawn from
/// ([`glow_skirt`]).
pub const GLOW_FALLOFF_TIGHT: f32 = 3.0;
pub const GLOW_FALLOFF_FLAT: f32 = 0.25;

/// The two ends of the Glow section's Shadow curve bar, as the exponent the
/// standoff's decay is taken over across its own width. Mirrored in
/// lattice.wgsl (`SHADOW_SHAPE_RIND`/`PLAIN`) on the same terms as the pair above
/// — the shader holds the rationale, including why the plain exponential is
/// the bar's ceiling rather than a point inside it — and the render crate
/// asserts the copies agree. What this copy draws is the bar's preview
/// ([`standoff_recovery`]).
pub const SHADOW_SHAPE_RIND: f32 = 0.25;
pub const SHADOW_SHAPE_PLAIN: f32 = 1.0;

/// How many e-folds of the standoff the Shadow's fade spends, mirrored from
/// lattice.wgsl (`SHADOW_TAIL`), which holds the rationale.
pub const SHADOW_TAIL: f32 = 4.0;

/// `smoothstep(0, 1, x)`, as WGSL spells it.
fn smoothstep(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of a node's light is left `p` of the way from its centre to where
/// the light stops, 0..=1 of the peak, at `feather`
/// ([`ViewConfig::glow_feather`]): the Feather bar's preview.
///
/// A COPY of `glow_layer` in lattice.wgsl and not the function itself, which
/// is the one thing the Fade curve bar's preview did not have to be: that one
/// is handed the envelope the notes run on, and the light's falloff runs on
/// the GPU with nothing on the CPU to hand over. So it is a copy, kept to the
/// shader's own two lines — the exponential at the rate the bar mixes, under
/// the window that shuts it over the far half of the span — and the render
/// crate holds both lines to the shader's text
/// (`the_feather_bars_preview_is_the_skirt_the_shader_draws`). The base the
/// shader scales by is left out, the preview being a shape rather than an
/// amount; the Strength bar is that.
pub fn glow_skirt(feather: f32, p: f32) -> f32 {
    let rate = GLOW_FALLOFF_TIGHT + (GLOW_FALLOFF_FLAT - GLOW_FALLOFF_TIGHT) * feather;
    let window = 1.0 - smoothstep((p - 0.5) / 0.5);
    (-rate * p).exp() * window
}

/// How much of its light a ring has given back `p` of the way across the
/// standoff's fade, 0..=1, at `shape` ([`ViewConfig::glow_shadow_shape`]): the Shadow
/// curve bar's preview, drawn as the light coming BACK because that is the
/// direction the fade runs in — the ring's edge at the left, the halo at the
/// right.
///
/// The low end of the bar draws a curve that is nearly a step at the left,
/// which is the shape rather than a clipped preview: an exponent under one
/// gives the light back inside the ink and leaves the rest of the width to a
/// haze.
///
/// A copy of `standoff_coverage`'s decay and the exponent `glow_shadow_shape`
/// takes it over, on the terms [`glow_skirt`] states, and held to the shader's
/// text the same way (`the_shadow_curve_bars_preview_is_the_ramp_the_shader_runs`).
///
/// It does not reach 1 at the right-hand end, and that is the shape rather than
/// a clipped preview: nowhere in the bar's own width does the fade land on
/// nothing (see `SHADOW_TAIL`), so it draws a curve still climbing where a ramp
/// would have arrived. What ends the tail is a window a couple of Shadow widths further
/// out again (`SHADOW_STOP` in lattice.wgsl), outside every point drawn here.
pub fn standoff_recovery(shape: f32, p: f32) -> f32 {
    let exponent =
        SHADOW_SHAPE_RIND * (SHADOW_SHAPE_PLAIN / SHADOW_SHAPE_RIND).powf(shape.clamp(0.0, 1.0));
    1.0 - (-SHADOW_TAIL * p.clamp(0.0, 1.0).powf(exponent)).exp()
}

/// The longest attack or release the node glow offers, in seconds (see
/// [`ViewConfig::glow_attack`]).
///
/// Longer than the audio ring's own pair ([`SPECTRAL_BALLISTICS_MAX`]) because
/// it is a different quantity: that one is how fast a measurement moves, and a
/// third of a second is already a smear, where this is how long a halo hangs
/// around after the note that lit it. Six seconds is a light that is plainly
/// still leaving a bar later, which is the far end of the look rather than a
/// guard rail on it.
pub const GLOW_BALLISTICS_MAX: f32 = 6.0;

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

/// One node's state in the glow's own slow filter: what the light is doing at
/// this node, and where its colour is being kept.
///
/// Several numbers rather than one because the light is carried in two places
/// at once. The LEVEL is stepped on the CPU, where the node's identity lives;
/// the COLOUR is stepped on the GPU, where the node's ink is read (the ink
/// strip in harmonigraph-render). What ties them is [`mix`](Self::mix): the
/// same coefficient carries both, so the two halves of one light can never be
/// running at different speeds — and [`marked`](Self::marked), the light's own
/// memory of how big the node is, rides that same coefficient for the same
/// reason.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlowStep {
    /// How lit this node is for the purpose of the light it gives off, carried
    /// on the Glow attack and release. Its TARGET is the largest level that
    /// puts ink on the node; this is where that target has got to, so it can be
    /// above zero on a node whose every layer has gone silent — which is the
    /// whole of what makes a halo linger.
    pub level: f32,
    /// Which row of this frame's ink strip holds this node's colour.
    ///
    /// It has to hold STILL while the node keeps glowing: the row is where last
    /// frame's colour is read back from, so a node handed a different row each
    /// frame would read a stranger's ink and morph toward that instead. The
    /// instance list is sorted by depth and culled, so its own order is exactly
    /// what cannot be used.
    pub row: u32,
    /// How much of this frame's reading the two of them take, `1 - exp(-dt/tau)`
    /// on the attack or the release.
    ///
    /// 1 means SETTLE rather than carry, and it is the same statement in both
    /// halves: on the CPU the level lands on its target outright, and on the
    /// GPU the row takes the new ink whole. So the first frame of all, a row
    /// just handed to a node, and a strip that has just been rebuilt all say
    /// the one thing, and none of them needs a flag of its own.
    pub mix: f32,
    /// How much of a MARK the light still has this node wearing, carried on the
    /// same [`mix`](Self::mix) as everything else about it.
    ///
    /// The light's span is the node's outermost drawn edge plus the Reach, and
    /// a mark is the one layer that moves that edge per node: a marked node
    /// reaches its strip past the outermost ring, an unmarked one stops at the
    /// ring. Read straight off `melody_slots | bass_slots` that edge is a STEP
    /// — the bit is set while the marking voice exists and clear the frame it
    /// is pruned — so the whole halo snapped a size smaller one Fade after the
    /// key came up, while its own level was still near full and had seconds of
    /// release left to run. Carried instead, the light's size comes off on the
    /// light's own clock, exactly as its brightness and its colour do.
    ///
    /// A share rather than a bit, and read as one: the shader interpolates the
    /// node's rim between the two the mark chooses between (`glow_rim` in
    /// lattice.wgsl), so what the light draws against is the edge as the light
    /// remembers it and not the edge the node has this frame.
    pub marked: f32,
}

impl Default for GlowStep {
    /// Unlit, on row 0, settling. A node nothing has stepped is a node with no
    /// light, and the mix is the value that makes the next step a settle rather
    /// than a fade up from a colour nobody drew.
    fn default() -> GlowStep {
        GlowStep { level: 0.0, row: 0, mix: 1.0, marked: 0.0 }
    }
}

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
    /// wherever it sits; what marks a home position is the MARKER standing
    /// under where the node itself would draw, and off-sheet positions have
    /// not even that (see [`derive_pluses`](derive::derive_pluses)).
    pub on_home: bool,
    /// Billboard size, as a factor of the scene's `node_radius` (see
    /// [`ViewConfig::sevens_size`]): 1 on the home sheet, smaller with every
    /// step off it. Scales the whole node uniformly — the quad and its uv
    /// together — so every layer inside keeps its proportions and only the
    /// node's size on screen changes.
    pub scale: f32,
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
    /// How much of the audio ring this node wears, 0..=1 — the gate's answer
    /// for its wedges ([`RingGate`]) carried on the note Fade ([`RingFade`]),
    /// and never less than the node's own [`activation`](Self::activation).
    ///
    /// A LEVEL and not the gate's own yes or no, because a ring is a layer of a
    /// node and every other layer of a node arrives and leaves on the Fade: a
    /// ring that switched off at the instant the spectrum crossed the bar would
    /// flicker on a breathing partial, and would leave part way through the
    /// release the rest of the node is still drawing.
    ///
    /// The floor under it is the whole of what the KEYS have to say here: a
    /// node the player is holding wears its ring for as long as the note lasts
    /// and fades out with it, whatever the analyzer reads there. The gate is
    /// then a question about the nodes nobody is playing — where a partial is
    /// sounding on a lattice the keys have not lit — which is where a
    /// reading-per-node was worth holding back.
    ///
    /// Beyond that floor it says nothing about the MIDI picture: a node whose
    /// ring is gone keeps its disc, its octave band and its marks exactly as
    /// the keys drew them, and loses only the annulus between the core and the
    /// band.
    ///
    /// `1.0` out of [`derive_scene`], which is not a decision but the absence
    /// of one — nothing in this crate reads audio, so a scene derived without
    /// [`Scene::wear_audio_rings`] behind it is a scene where nothing has been
    /// measured and nothing can be held back.
    ///
    /// It says nothing about a shell that forgets the pass, which is the
    /// tempting reading and the wrong one: such a shell keeps
    /// [`SpectralPaint::silent`]'s empty annulus, so the ring layer is off and
    /// no node draws one whatever this holds. Where the value is load-bearing
    /// is a scene assembled BY HAND with the annulus filled in — a test, a
    /// fixture — and there the ungated picture is the one that cannot be
    /// mistaken for a bug.
    pub audio_ring: f32,
    /// The node's own light, as far as the light is concerned: how bright it
    /// is, which row of the frame's ink strip is this node's, and how much of
    /// this frame's reading the pair of them take (see [`GlowStep`]).
    ///
    /// Out of [`derive_scene`] this is the UNCARRIED picture — the level the
    /// MIDI layers are at, a row per node in the list's own order, and the
    /// whole of the new reading — for the same reason
    /// [`audio_ring`](Self::audio_ring) arrives at 1: nothing in this crate
    /// keeps state between frames, so a scene derived without a pass behind it
    /// is one where nothing has been carried. The shell's pass
    /// (`panes::glow_fade` in harmonigraph-ui) is what replaces it with a
    /// level carried on the Glow attack and release, a row that holds still
    /// while the node keeps glowing, and the coefficient that carried it.
    pub glow: GlowStep,
    /// Whether the music is remembered here (see [`trail`]): 0 where it has
    /// never been, 1 where it has. A memory never fades, so those are the
    /// only two values a node carries; the field is an `f32` because the
    /// label layer scales its own strength by it.
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
    /// still a place the marker field says is there — a marker stands exactly
    /// where a pointer goes looking.
    ///
    /// So this is deliberately NOT "does this node paint a pixel": an empty
    /// home sheet would then be uninspectable, and it is the thing most worth
    /// inspecting. Off-sheet idle positions carry no marker and are
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

    /// How much NOTE NAME stands over this node, 0 to 1.
    ///
    /// The label pass's own question (`draw_node_labels` in
    /// `harmonigraph-ui`), answered HERE because two things turn on it and a
    /// second spelling of it is a picture that contradicts itself: the pass
    /// draws the name, and [`derive_pluses`](derive::derive_pluses) clears the
    /// resting marker out from under it. A marker behind a name is the one
    /// place in the lattice where two marks claim the same position, and the
    /// name is the better of the two — it says WHICH position, where the
    /// marker only says that there is one.
    ///
    /// So the label layer is the one that decides, and this is the whole of
    /// its rule: names on at all, then the node is sounding, hovered, or one
    /// the Show row keeps at rest.
    ///
    /// A LEVEL rather than a yes or no, and that is the whole of what keeps
    /// the handoff continuous. Under [`NoteNames::Played`] a name is drawn at
    /// exactly the node's `activation` (`label_strength` in `harmonigraph-ui`),
    /// so a released note spends the end of its fade with a name too faint to
    /// see. Answered as a predicate, the marker under it stays away for all of
    /// that and then arrives at full ground opacity the frame activation
    /// reaches 0 — a hole in the field, and then a cross popping into it, once
    /// per note. As a level the two cross-fade: what the name gives up, the
    /// marker takes, and the position carries the same ink throughout.
    ///
    /// The `is_visible` term re-checks what [`Scene::pick`] already enforces,
    /// and `hovered` is picking's alone, so it is a second lock on one door.
    /// It stays because the field is public shared state rather than picking's
    /// private output, and what it costs to be wrong is a name floating in the
    /// sevens dimension on a node that draws nothing.
    ///
    /// What it does NOT answer is whether the name lands on the PANE — that
    /// needs a rect and a projector, which is the label pass's business and
    /// not the scene's. It costs nothing: a name off the pane is a name over a
    /// marker that is off the pane too, and neither is drawn.
    pub fn name_level(&self, view: &ViewConfig) -> f32 {
        if !view.show_labels || !self.is_visible() {
            return 0.0;
        }
        if self.hovered {
            return 1.0;
        }
        // Named while nothing is happening on it: every node under All, a
        // visited one under Past, and nothing at all under Played. A trail is
        // written whole or not at all (`TrailField::apply`), so Past is a level
        // only in the sense that 0 and 1 are levels — what actually ramps here
        // is a sounding note's own fade.
        let resting = match view.note_names {
            NoteNames::All => 1.0,
            NoteNames::Past => self.trail,
            NoteNames::Played => 0.0,
        };
        // A name on its way to being KEPT settles on the level its record takes
        // over at rather than easing out and being popped back: `trail` is not
        // written until the frame the release finishes, so through the release
        // there is nothing to settle onto and the level has to be reserved
        // ahead of the record. Reserved on the way OUT alone, where a record
        // can actually land, and under the one mode that keeps one.
        //
        // Here rather than in the label pass alone, because the marker under
        // the name is this level's complement (`derive_pluses`): a reserve one
        // side only is a full name standing over a nearly-full marker for the
        // whole of a release, and then the marker going out in a single frame.
        let reserved = if view.note_names == NoteNames::Past
            && self.on_home
            && self.departing
            && self.activation > 0.0
        {
            1.0
        } else {
            0.0
        };
        self.activation.max(resting).max(reserved).clamp(0.0, 1.0)
    }

    /// Whether a note name is drawn over this node AT ALL — the gate the label
    /// pass asks before it lays a glyph out, where the marker under it wants
    /// [`name_level`](Self::name_level)'s ramp instead.
    ///
    /// One spelling still: this IS the level, asked for any at all.
    pub fn is_named(&self, view: &ViewConfig) -> bool {
        self.name_level(view) > 0.0
    }
}

/// One marker standing at a lattice position: a small cross, drawn under the
/// nodes, and the whole of what an unplayed lattice draws.
#[derive(Clone, Copy, Debug)]
pub struct PlusInstance {
    /// Which lattice position this marker stands at. Never uploaded —
    /// [`pos`](Self::pos) is what the GPU is given.
    pub lattice_pos: LatticePos,
    /// The position's own world center.
    pub pos: Vec3,
    /// How far the marker reaches, in WORLD units: the length of one arm,
    /// crossing to tip. Both of the other two numbers a plus has — its
    /// thickness and where its ends start to fade — are shares of THIS, so
    /// this alone is what sizes the billboard.
    ///
    /// Already resolved from [`ViewConfig::plus_arm`]'s quad UV against the
    /// scene's node radius, so nothing downstream carries a second copy of
    /// that convention.
    pub radius: f32,
    pub color: Vec4,
    /// The marker's opacity — its ink, its own pool of light, and the standoff
    /// its cross writes into the light around it (`plus_standoff` in
    /// lattice.wgsl), all three at once.
    ///
    /// ONE number for all of them, which is the rule about a cross and its
    /// shadow: they arrive and leave together, so a position handing itself
    /// over to a name never shows a cross with no shadow under it nor a shadow
    /// under no cross. The shadow a cross casts is worth exactly the cross,
    /// which is also what lets [`derive_pluses`](derive::derive_pluses) drop a
    /// fully-claimed marker outright rather than ship one that still shades.
    pub strength: f32,
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
    /// The outer octave layer's radial band (quad UV units), already
    /// sanitized: outer is always ahead of inner on a band that draws, and
    /// both are 0 when the layer is off (see [`ViewConfig::band_width`]).
    pub outer_inner: f32,
    pub outer_outer: f32,
    /// The outer edge of the outermost RING the node draws — the band's, save
    /// where the band is off and something inside it is the last layer on (see
    /// [`RingStack::outer`]).
    ///
    /// What the melody/bass marks stand off and what the node's billboard is
    /// sized on, so a node whose band is dialled away still wears its marks
    /// where its picture actually ends. The clearing is bounded by this and
    /// measured per layer instead (`node_clearing` in lattice.wgsl).
    pub rings_outer: f32,
    /// Where the melody/bass mark strip starts (see
    /// [`RingStack::mark_inner`]) — a padding out from
    /// [`rings_outer`](Self::rings_outer), or the stack's own start
    /// ([`ViewConfig::ring_inner`]) on a node with no rings at all, where there
    /// is nothing for it to stand off.
    ///
    /// Handed over rather than re-derived from `rings_outer + ring_gap`,
    /// because that sum is only right while some ring is there to owe the
    /// padding to.
    pub mark_inner: f32,
    /// The ANGULAR padding on a node (see [`ViewConfig::octave_gap`]): between
    /// one octave sector and the next, on the band, on the audio ring's wedges
    /// and down a mark's own sides. Already clamped.
    ///
    /// The node's other padding, the RADIAL one, reaches the picture as the
    /// radii themselves — every stand-off it buys is already spent in
    /// [`rings_outer`](Self::rings_outer), [`mark_inner`](Self::mark_inner) and
    /// the band's two edges — so it needs no field of its own here. This one
    /// has one because nothing upstream can spend it: a gap cut across an
    /// annulus is a per-fragment test, and the shader is where the fragments
    /// are.
    pub octave_gap: f32,
    /// A NODE at rest — both of its rings where nothing is lit — already
    /// resolved from [`ViewConfig::lattice_ground`]'s `L*` to the neutral grey
    /// it names.
    ///
    /// A COLOUR here and an `L*` in the view, because the two ends want
    /// different things: the bar is a brightness a person reads and drags, and
    /// what the shader needs is three channels it can lay down without solving
    /// for a luminance per fragment. Resolved once per frame in
    /// [`derive_scene`] — the audio ring's own copy is
    /// the `t` = 0 end of [`spectral`](Self::spectral)'s table, baked from the
    /// same number, so the two agree by construction rather than by both being
    /// aimed at the skin.
    ///
    /// Read by the OCTAVE band, which is what a silent slice draws and what a
    /// sounding one's pitch is painted over as the note fades. The audio ring
    /// carries the same grey without reading this field, because it does not
    /// reach the shader as a uniform: it is the `t` = 0 end of its table. Two
    /// copies of one resolve, not two answers.
    ///
    /// The markers are NOT on this — see [`pluses`](Self::pluses), which
    /// carries its own resolve of [`ViewConfig::marker_ink`].
    pub lattice_ground: Vec4,
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
    /// The lattice's resting markers (see
    /// [`derive_pluses`](derive::derive_pluses)): one cross per visible
    /// HOME-sheet position, each carrying
    /// [`ViewConfig::marker_ink`](crate::ViewConfig::marker_ink)'s grey as its
    /// colour — so a marker IS that grey rather than a brightness of it. A bar
    /// of its own and not [`lattice_ground`](Self::lattice_ground): the field
    /// says where the positions are, which is worth keeping legible at any
    /// brightness a node's empty rings are dialled to. The light standing at
    /// one washes over that grey on the terms it takes a node's ink
    /// (`wash_over` in lattice.wgsl), so a marker inside a halo reads as
    /// standing in the light rather than as a hole punched in it.
    ///
    /// Off-sheet positions get none, and that is the whole of what says which
    /// sheet is the ground: a note there floats over the marker field instead
    /// of standing in it.
    pub pluses: Vec<PlusInstance>,
    /// Half an arm's thickness, as a SHARE of the arm's length — the shape's
    /// one proportion, and what the shader folds a fragment's distance against
    /// (see [`ViewConfig::plus_width`], which is the WHOLE thickness and in
    /// quad UV).
    ///
    /// View-wide, as the length beside it is not: a length reaches the renderer
    /// per instance because it is a world distance, while a proportion is one
    /// answer the whole field shares and the shader reads out of a uniform.
    /// Already clamped, and to 1 at the top — at half an arm's thickness the
    /// cross has filled its own square and there is nothing past it to draw.
    pub plus_half_width: f32,
    /// Where a plus's arms stop being solid and start fading out, as a SHARE
    /// of one arm's length: 1 is a square end, 0 an arm that fades the whole
    /// way from the crossing to its tip (see [`ViewConfig::plus_taper`], which
    /// is the same edge measured from the other side and in quad UV).
    ///
    /// A share here, where the view keeps a width, because the shader works in
    /// the arm's own units — uv 1 IS the tip — so it needs the point on that
    /// axis rather than a length it would have to divide out per fragment.
    /// Already clamped, and held short of 1 so the fade never collapses to a
    /// zero-width `smoothstep`.
    pub plus_taper_start: f32,
    /// The ground the lattice is drawn onto: the pane fill this pass gets
    /// composited over, which is the skin's `well` — the recessed grey the
    /// lattice pane paints its own rect with, as every other picture pane
    /// does (see [`skin::well_color`]).
    ///
    /// Only the sevens knockout reads it, and it is the difference between
    /// a hole and a blob. The pass blends premultiplied, so a gutter with no
    /// color of its own knocks out to BLACK — still darker than the well, so
    /// a cleared disc sits on the picture as a darker plate instead of
    /// disappearing into the ground wherever it crosses nothing. Handing the
    /// ground in means the gutter is invisible over empty lattice and only
    /// shows as a clearing where it actually crosses something.
    pub background: Vec4,
    /// How deep the melody/bass mark strip is, in quad UV units; 0 = off (see
    /// [`ViewConfig::mark_thickness`]). It starts one
    /// [`ViewConfig::ring_gap`] past [`rings_outer`](Self::rings_outer) — a
    /// sum already spent, this struct carrying [`mark_inner`](Self::mark_inner)
    /// itself. Already clamped.
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
    /// How far past a node's outermost drawn edge its own glow is shown, in
    /// quad UV units; 0 turns the whole glow off (see
    /// [`ViewConfig::glow_reach`]). Already clamped to [`GLOW_REACH_MAX`].
    pub glow_reach: f32,
    /// How much of that glow is added back as light; already clamped to
    /// [`GLOW_STRENGTH_MAX`]. Inert while [`glow_reach`](Self::glow_reach) is
    /// 0, which is the pair's one off switch.
    pub glow_strength: f32,
    /// How flat the light's falloff is across that reach, 0 the exponential
    /// heaped on the node and 1 an even field lit right out to where the window
    /// shuts it (see [`ViewConfig::glow_feather`]); already clamped to 0..=1.
    /// Inert while [`glow_reach`](Self::glow_reach) is 0.
    pub glow_feather: f32,
    /// How much two nodes' overlapping light adds up, 1 screening the halos
    /// together and 0 leaving an overlap as bright as the brighter node alone
    /// (see [`ViewConfig::glow_meld`]); already clamped to 0..=1. Inert while
    /// [`glow_reach`](Self::glow_reach) is 0.
    pub glow_meld: f32,
    /// The standoff: how far past every ring a node dims the light standing
    /// there, in the same quad UV units (see [`ViewConfig::glow_shadow`]);
    /// already clamped to [`GLOW_SHADOW_MAX`]. Inert while
    /// [`glow_reach`](Self::glow_reach) is 0.
    pub glow_shadow: f32,
    /// How much of that shadow is spent fading the light back in, measured back
    /// from its end in the same units (see [`ViewConfig::glow_shadow_soft`]);
    /// already clamped to the shadow.
    pub glow_shadow_soft: f32,
    /// How the standoff's decay is shaped across that width (see
    /// [`ViewConfig::glow_shadow_shape`]), 1 a plain exponential off the ring and
    /// 0 handing the light back inside the ink; already clamped to 0..=1.
    /// Inert while [`glow_reach`](Self::glow_reach) is 0.
    pub glow_shadow_shape: f32,
    /// How much of the light the standoff takes away where it stands (see
    /// [`ViewConfig::glow_shadow_depth`]); already clamped to 0..=1.
    pub glow_shadow_depth: f32,
    /// One quad-uv length of the home sheet, as a world length
    /// (`marker_world`): what converts the marker field between the units its
    /// own draws are in and the units every glow bar is dialled in.
    ///
    /// The markers' standoff draw needs both. Its billboard is sized in WORLD
    /// and the arm it is sized around arrives per instance as a world length,
    /// while the Shadow that same draw holds the light off by is quad uv, being a
    /// node's bar — so one of the two has to be converted, and the conversion
    /// is a property of the scene rather than of any marker. Handing it over
    /// once is what keeps the shader from carrying a second copy of the uv rule
    /// (see [`Scene::node_radius`], which is the same rule for a node).
    pub marker_unit: f32,
    /// How widely a node's own ink is averaged into the colour of its light
    /// (see [`ViewConfig::glow_blend`]); already clamped to 0..=1.
    pub glow_blend: f32,
    /// How many rows the frame's ink strip has to hold — the ceiling on every
    /// [`GlowStep::row`] in `nodes`, plus one.
    ///
    /// A CAPACITY and not this frame's node count, which is the difference a
    /// carried light makes: rows are handed out per node and held while that
    /// node's light lasts, so the tallest row in use has nothing to do with how
    /// many nodes are drawn. The renderer sizes the strip textures to it.
    ///
    /// `nodes.len()` out of [`derive_scene`], where every node has its own row
    /// in the list's own order (see [`NodeInstance::glow`]).
    pub glow_rows: u32,
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
        if slide.is_finite() {
            slide as f32
        } else {
            0.0
        }
    }

    /// Decide how much of the audio ring each node wears — [`SpectralPaint::gate`]
    /// against what its wedges reach, carried on `env` by `fade`, and floored by
    /// the node's own envelope — and write it into
    /// [`NodeInstance::audio_ring`].
    ///
    /// Run after the levels are measured in, which is the whole reason it is a
    /// pass of its own rather than part of [`derive_scene`]: nothing in this
    /// crate reads audio, so the question has no answer until the shell's fold
    /// has filled [`Scene::spectral`] (`panes::spectral_fold::apply` is the one
    /// caller, and it calls this last).
    ///
    /// A method on the scene and not a free function over the three parts,
    /// because the parts are only right together: the levels, the wheel the
    /// wedges are laid on and the nodes being gated all have to come from ONE
    /// frame, and a caller assembling them by hand is a caller who can pair
    /// last frame's grid with this frame's wheel. The `fade` is the one thing
    /// that must OUTLIVE the frame, which is why it is passed in rather than
    /// held here: a scene is built afresh every frame and a transition is
    /// exactly what cannot be.
    ///
    /// The gate at its FLOOR runs the reduction like any other setting, though
    /// the floor admits every node and the answer is a foregone yes: the fade
    /// is what needs it. A lattice arriving at the floor has rings still on
    /// their way in, and skipping the pass would leave them standing where the
    /// bar's last position put them.
    pub fn wear_audio_rings(&mut self, fade: &mut RingFade, env: &Envelope, now: f64) {
        // Nothing to hold back on a ring dialled to no width, and nothing to
        // carry either: the layer is off, so the fade keeps whatever it last
        // held and picks the reading up again when the width bar brings a ring
        // back.
        if !self.spectral.ring_draws() {
            return;
        }
        let gate = RingGate::new(&self.spectral);
        fade.advance(&gate, env, now);
        let layout = &self.octave_layout;
        for node in &mut self.nodes {
            // The keys' own floor. `activation` and not the octave word beside
            // it, though the two carry the same envelopes: this is the level
            // the node's disc and its clearing are drawn at, so the ring leaves
            // exactly with the rest of the node rather than a slot at a time.
            node.audio_ring = fade.level(layout, node.cents).max(node.activation);
        }
    }
}

#[cfg(test)]
mod tests;
