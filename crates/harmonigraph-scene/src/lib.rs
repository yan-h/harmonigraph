//! The scene layer: turns core state (note tracker + tuning) into a
//! render-friendly description, once per frame. The renderer consumes a
//! [`Scene`] and knows nothing about MIDI; the core knows nothing about
//! cameras or colors. Animation/envelope *policy* lives here.
//!
//! What lives where:
//! - `lib.rs` (this file) — the render-facing types: [`Scene`],
//!   [`NodeInstance`], [`PlusInstance`], and the constants they share.
//! - [`derive`](mod@derive) — the per-frame derivation ([`derive_scene`]): note tracker
//!   + tuning -> node/marker lists and initial voice envelopes.
//! - [`motion`] — carried lattice animation and melody/bass marks ([`NodeMotion`]).
//! - [`view`] — [`ViewConfig`] (persisted visual settings and their serde
//!   defaults).
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
//! - [`intensity`] — how loud each note is drawn, from its velocity and
//!   expressions.
//!
//! Every public item of the modules globbed below is re-exported at the crate
//! root, so downstream code keeps using `harmonigraph_scene::Camera` rather
//! than module paths, and a new item needs no list edited to get there.
//! [`framing`], [`skin`], [`star_plan`] and [`trail`] are reached by their
//! paths instead: their names (`skin::STEP`, `framing::frame`) read as their
//! module's rather than the crate's.

pub mod atmosphere;
pub mod camera;
pub mod color;
pub mod derive;
pub mod framing;
pub mod intensity;
mod light_owners;
pub mod motion;
pub mod octaves;
pub mod skin;
pub mod spectral;
pub mod star_plan;
pub mod style;
pub mod trail;
pub mod view;

pub use atmosphere::*;
pub use camera::*;
pub use color::*;
pub use derive::*;
pub use intensity::*;
pub use light_owners::light_owners;
pub use motion::*;
pub use octaves::*;
pub use spectral::*;
pub use style::*;
pub use view::*;

use glam::{Vec3, Vec4};
use harmonigraph_core::LatticePos;
use std::ops::{Bound, RangeBounds, RangeInclusive};

/// A stored angle in `[0, 360)`. `rem_euclid` alone rounds an angle a hair
/// below zero up to exactly 360, which a second wrap turns into 0: a saved
/// value and its sanitized copy then disagree though they draw the same.
/// Finite input only; sanitizers repair a non-finite angle first.
pub fn wrap_degrees(degrees: f32) -> f32 {
    let wrapped = degrees.rem_euclid(360.0);
    if wrapped < 360.0 {
        wrapped
    } else {
        0.0
    }
}

/// A number held to a range, with a fallback for a non-finite one: `value`
/// held to `range`, and `fallback` in its place when it is no number, a NaN or
/// an infinity, which no clamp catches since NaN answers no to every
/// comparison a clamp makes.
///
/// The one spelling of the load repair every look setting's `sanitize` uses
/// (#1956), and of a few guards on what the picture reads at run time (a
/// shadow's spread, a note's expressions, the camera distance a font size is
/// scaled by). The fallback is the caller's, because what a broken number should
/// become varies: the fresh value almost everywhere, the pane's own fresh value
/// for Stars, 1.0 for an intensity weight, the type's own default inside a
/// gradient, the off position for the ring gate and hysteresis. It is held to
/// `range` as well, so what comes back is always inside it.
///
/// `range` is inclusive at each end it bounds. `..` asks for a number alone,
/// for a value with no bound, such as a periodic angle. An inverted range,
/// which no caller hands it, is a `debug_assert`, where `f32::clamp` panicked.
pub fn repair<T: Setting>(value: T, fallback: T, range: impl RangeBounds<T>) -> T {
    if let (Bound::Included(low), Bound::Included(high)) = (range.start_bound(), range.end_bound())
    {
        debug_assert!(low <= high, "an inverted range");
    }
    let value = if value.is_number() { value } else { fallback };
    let value = match range.start_bound() {
        Bound::Included(&low) if value < low => low,
        Bound::Excluded(_) => unreachable!("a setting's range includes its ends"),
        _ => value,
    };
    match range.end_bound() {
        Bound::Included(&high) if value > high => high,
        Bound::Excluded(_) => unreachable!("a setting's range includes its ends"),
        _ => value,
    }
}

/// What [`repair`] holds to a range: a float, which can be no number, and the
/// integers, which are always one.
pub trait Setting: Copy + PartialOrd {
    /// Whether this is a number a range can hold.
    fn is_number(self) -> bool;
}

impl Setting for f32 {
    fn is_number(self) -> bool {
        self.is_finite()
    }
}

impl Setting for f64 {
    fn is_number(self) -> bool {
        self.is_finite()
    }
}

impl Setting for u32 {
    fn is_number(self) -> bool {
        true
    }
}

impl Setting for i32 {
    fn is_number(self) -> bool {
        true
    }
}

/// The unit fraction: the range of the unit-fraction settings that have no
/// range of their own, a share, a mix or a depth, at its `sanitize` and at its
/// bar alike.
pub const UNIT: RangeInclusive<f32> = 0.0..=1.0;

/// Axis mapping, matching v1's orientation: major thirds run horizontally
/// (x), fifths vertically (y), and harmonic sevenths in depth (z).
/// Each lattice step is one world unit.
fn lattice_to_world(pos: LatticePos) -> Vec3 {
    Vec3::new(pos.fives as f32, pos.threes as f32, pos.sevens as f32)
}

/// Node radius in world units; adjacent lattice positions are one unit apart.
const NODE_RADIUS_FACTOR: f32 = 0.25;

/// One quad uv in node radii: uv 1, the octave band's outer edge at the
/// default stack, is this many of a node's own radii out. The one conversion
/// between the size bars' unit and the world; lattice.wgsl declares the same
/// constant, and harmonigraph-render's contract test pins the two together.
pub const RADII_PER_UV: f32 = 1.8;

/// The floor lattice.wgsl puts under a node's per-sheet scale
/// ([`NodeInstance::scale`]) wherever it sizes that node's quads, so a CPU
/// bound on one of those quads applies the same. Declared in the shader too,
/// and pinned to it like [`RADII_PER_UV`].
pub const MIN_NODE_SCALE: f32 = 0.05;

/// How far a node's halo reaches, in the node's quad uv: the view's widest rim
/// — the rings' outer edge, or the mark strip's where marks draw at all — plus
/// the Background glow reach, floored at 0.1 so a node drawn down to almost
/// nothing still has a footprint to fall off over.
///
/// One rim for every halo, whether or not the node wears a mark now, so a mark
/// arriving or leaving does not resize the light. The one spelling of the
/// footprint: the renderer ships it to the glow splat (`GlowParams::span` in
/// harmonigraph-render), and [`ViewConfig::light_owner_radius`] bounds the
/// off-pane light owners on it. #1252 widened a second spelling and missed
/// the first.
///
/// It takes the rim's inputs rather than a [`ViewConfig`] because the renderer
/// has to size the halo on the rings it packs for the node's own geometry,
/// which are the [`Scene`]'s stored `rings_outer` and `mark_inner`. Out of
/// [`derive_scene`] those are `view.rings()` of the scene's own view, but a
/// scene built by hand need not agree (the lattice tests set rings from a
/// probe view, or move `mark_inner` alone), and a halo sized off the view
/// there would no longer fit the node it is drawn around.
pub fn glow_span(rings_outer: f32, mark_inner: f32, mark_thickness: f32, reach: f32) -> f32 {
    let mut rim = rings_outer.max(0.0);
    if mark_thickness > 0.0 {
        rim = rim.max(mark_inner + mark_thickness);
    }
    (rim + reach.max(0.0)).max(0.1)
}

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
/// assumed — past about 3.3:1 at that depth cabinet does reach this, and
/// there is a pane that gets there without hand-editing anything, since
/// `Layout::split` will give the lattice a fifth of a 21:9 frame. What the
/// trim costs there is 5634 nodes at 4:1 and 15912 at 6:1, off all four
/// edges. A cap that covered it would have to be twice this one, for a band
/// of lattice eight steps tall.
///
/// Nine sheets is full depth, and this cap is why: #896 briefly took
/// [`SEVENS_LAYER_LIMIT`] to six for the layer strip's cell width, and the
/// thirteen sheets that allowed moved the crossing from 3.3:1 down to 1.8:1 —
/// putting a plain 16:9 pane over the cap at 21021 nodes. It went unseen
/// because the test named for this bound read a count the trim had already
/// held down; both halves are #916.
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

/// A range's pair of ends held to its axis `min..=max` with the high end at
/// least `min_span` above the low: the low end under `max - min_span`, then the
/// high end between `lo + min_span` and `max`.
///
/// The one owner of that rule, for every pair a `sanitize` repairs and every
/// control that writes one (the range bars, the analyzer's drags), so the pair
/// a control writes is the pair the next `sanitize` keeps, f32 rounding
/// included (#1839). Both ends must be finite; callers repair a non-finite
/// one first.
///
/// An axis too short for its span crosses the clamps' bounds, which no caller
/// hands it: a `debug_assert`, as the stakes record asks of a case no
/// producer writes (#1631), and in release `f32::clamp`'s own panic.
pub fn clamp_span(lo: f32, hi: f32, (min, max): (f32, f32), min_span: f32) -> (f32, f32) {
    debug_assert!(
        min <= max - min_span && (max - min_span) + min_span <= max,
        "an axis {min}..={max} too short for a span of {min_span}"
    );
    let lo = lo.clamp(min, max - min_span);
    (lo, hi.clamp(lo + min_span, max))
}

/// Samples in the pitch->color lookup EVERYTHING pitch-colored reads: the
/// melody and bass marks and the piano roll on the CPU, the octave glyphs and
/// their glow in the shader. The shader mirrors this length, and `harmonigraph-render`
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
/// What that costs is measured (#1111), on UNBENT gradients swept in `t` steps
/// of 2.5e-4. The default LEVEL gradient's chroma fraction reaches exactly 1.0
/// at its top, where the boundary near `L*` 94 at hues about 120-130 has a
/// sharp corner: green saturates while blue turns steep within 0.2 `L*`. The
/// table misses it by 5.8/255 on the default, and by 30.9/255 at worst with
/// hue start swept over 290-310 and lightness over 45-55 (worst at 296 / 48,
/// at `t` 0.996). A brightness bend puts the corner where the table samples
/// more coarsely, and on #1110's branch that read 36.7/255. Accepted: the miss
/// sits in the loudest half-percent of the ramp at non-default dials. What
/// would remove it is a table entry pinned on the corner — a second warp beside
/// the bend's in [`LutSpacing`] and in every shader's `lut_position`, catching
/// only the one corner the arc crosses — or a chroma fraction held under 1.0,
/// which moves the look.
///
/// Do NOT read that error as a mismatch between shapes. It is the difference
/// between the table and an ideal nothing draws.
pub const PITCH_LUT_N: usize = 64;

/// The most nodes that can be lit at once, on one surface.
///
/// A ceiling on the light's TEXTURE rather than on the picture: the renderer
/// reads every lit node's colour into one row of a strip, and a texture has a
/// maximum dimension (the adapter's own limit, 16,384 on Apple silicon), where
/// [`MAX_DRAWN_NODES`] is 20480, so a fully zoomed-out lattice can light more
/// nodes than a strip holds. Past this a node simply gets no light: its layers
/// draw exactly as they do with the glow off, which is the graceful end of a
/// halo nobody can pick out of four thousand others anyway. The shell's glow
/// pass lights no more than this (`GlowFade` in harmonigraph-ui), and the
/// renderer asserts it in debug builds rather than capping again.
pub const MAX_LIGHTS: usize = 4096;

/// What a node's light is made of: how much light each of its layers puts on
/// it, slot by slot, which the renderer reads round the node into the halo's
/// colour (`ink_at` in lattice.wgsl).
///
/// **Weights, and only weights.** No colour is kept here: a layer's colour is
/// its slot's pitch through the gradient, which the shader looks up in the
/// uniforms, so a gradient, range or tuning change reaches a halo in the frame
/// it reaches the notes (#1759, Q1). What a light CARRIES between frames is how
/// much of each layer it holds, and the halo's colour is a sum of the slot
/// colours weighted by these numbers — linear in each of them, save the one
/// place `ink_at` names (the mark walk's per-angle max) — so stepping every
/// weight toward its target eases the colour exactly
/// as mixing the two readings would. That is the guarantee, and the renderer's
/// `a_stepped_light_reads_as_its_two_readings_mixed` is what holds it.
///
/// A per-node input that is NOT a weight over the slot colours snaps rather
/// than eases, because nothing steps it. The node's pitch class (`cents`) is
/// the one such input, declared beside these rather than among them
/// (`LightRow` in harmonigraph-render, whose size assertion names the rule).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GlowLight {
    /// Each slot's light: its colour after the Opacity mapping
    /// ([`NodeInstance::octaves`]), clamped to 0..=1.
    pub l: [f32; OCTAVE_SLOTS],
    /// The light of the mark standing on each slot: the melody's and the
    /// bass's levels on their strongest slots, clamped, and the larger where
    /// the two share one.
    pub m: [f32; OCTAVE_SLOTS],
}

impl GlowLight {
    /// How lit the node is, as far as its light is concerned: its most opaque
    /// octave slot or mark as drawn, each after the Opacity mapping, in 0..=1 —
    /// so a soft note's halo is as faint as its ink and one faded to nothing
    /// has none. Not [`NodeInstance::presence_with_opacity`], the presence that
    /// holds through a release. Read off the same weights the colour is
    /// (#1304, #1339), though the colour takes only those the ring draws: a
    /// light whose weights all sit off the ring has a level and no colour, and
    /// draws nothing.
    pub fn level(&self) -> f32 {
        self.l.iter().chain(&self.m).copied().fold(0.0, f32::max).clamp(0.0, 1.0)
    }

    /// Move every weight toward `target`'s, each in its own direction: `up` of
    /// the way while it is below its goal, `down` while it is above, as a
    /// per-channel envelope follower does (#1784 G2). ONE walk over every
    /// field, so a field added to the struct is stepped by adding it to the
    /// pattern below, which the compiler insists on.
    pub fn step(&mut self, target: &Self, up: f32, down: f32) {
        let Self { l, m } = self;
        for (weights, goal) in [l, m].into_iter().zip([&target.l, &target.m]) {
            for (weight, goal) in weights.iter_mut().zip(goal) {
                let mix = if *goal > *weight { up } else { down };
                *weight += (goal - *weight) * mix;
            }
        }
    }
}

/// One node's state in the glow's own slow filter: how lit its light is, and
/// what that light is made of.
///
/// Out of [`NodeMotion::step`] it is the node's own reading as it stands, which
/// is what a stateless snapshot draws. The shell's glow pass (`GlowFade` in
/// harmonigraph-ui) carries both on the Glow attack and release, each value in
/// its own direction: the level and every weight rise on the attack and fall on
/// the release (#1784 G2), on one clock, so a halo's brightness and its colour
/// are stepped together (#1626). Nothing about a light is kept anywhere else:
/// the renderer reads it afresh every frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GlowStep {
    /// How lit this node's light is drawn. Out of [`NodeMotion::step`] it is
    /// the [`level`](GlowLight::level) of the node's own reading; the
    /// shell's pass replaces it with where it has got to on the Glow attack and
    /// release toward its goal — the reading's level, or 0 where no drawn layer
    /// holds the light — so it can be above zero on a node whose every layer
    /// has gone silent, which is the whole of what makes a halo linger.
    pub level: f32,
    /// What the light is made of. Out of [`NodeMotion::step`] it is the node's
    /// own [`glow_light`](NodeInstance::glow_light); the shell's pass replaces
    /// it with where that reading has got to.
    pub light: GlowLight,
}

/// One lattice node, ready for instanced rendering.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodeInstance {
    pub lattice_pos: LatticePos,
    pub world_pos: Vec3,
    /// 0 = idle, 1 = fully lit: the node's presence, its loudest octave slot
    /// Opacity included. A held note sits at its Opacity reading; a released
    /// one holds while its release travels the slices, then decays. Never
    /// below any slot's colour in [`octaves`](Self::octaves), which leaves at
    /// the release.
    ///
    /// A change to what this level means renames the field, so every reader is
    /// visited.
    pub presence_with_opacity: f32,
    /// [`presence_with_opacity`](Self::presence_with_opacity) before Opacity:
    /// the loudest octave slot's presence alone, so a held note is 1 however
    /// softly it was played.
    ///
    /// What reads it is the NAME ([`name_level`](Self::name_level)), and the
    /// cross that is its complement. A name says which position this is, not
    /// how loud it sounded, and a name held at the note's Opacity would jump to
    /// the departing reserve at key-up (#1316). The name's grey-to-white ink
    /// follows neither: it reads the drawn colour, the loudest of
    /// [`octaves`](Self::octaves), which leaves at the release while
    /// `presence_with_opacity` waits through a Fade-out delay.
    ///
    /// A change to what this level means renames the field, so every reader is
    /// visited.
    pub presence_without_opacity: f32,
    /// Whether the voice this node is lit by is on its way OUT — its key is
    /// up and its departure has begun.
    ///
    /// [`presence_with_opacity`](Self::presence_with_opacity) alone cannot
    /// answer this: it is the arrival times what is left of the departure, so
    /// a level part-way up and the same level part-way down are one number.
    /// The two ends run on one dial and never overlap, which is what makes a
    /// single flag enough to tell them apart — a voice is arriving, or full,
    /// or departing.
    ///
    /// What needs it is anything that reads a low presence, with Opacity or
    /// without, as "nearly gone" and acts on it, which is only true on the way
    /// out. The kept note names are the case: their level is reserved ahead of the trail
    /// record that takes over when the release finishes, and reserving that
    /// on the way IN draws a name ahead of the note it names.
    pub departing: bool,
    /// Continuous geometric pose by drawn slice, lowest first (slot
    /// `ring.base + i`), not by octave slot; one means settled.
    pub slice_progress: [f32; OCTAVE_SLOTS],
    /// Per-octave activation (slot = MIDI octave + 1, clamped into the span
    /// the view shows — see [`octaves`]): each octave's indicator fades on
    /// its own voice's envelope, independent of the node's overall
    /// presence. Slots outside the shown span stay 0, so a note beyond the
    /// range lights the outermost indicator on its side rather than
    /// disappearing.
    pub octaves: [f32; OCTAVE_SLOTS],
    /// How thick each octave's LIT slice is drawn, as a multiple of the band's
    /// width, by the same slots as [`octaves`](Self::octaves): the slice runs
    /// from the band's inner edge out this far. Thinner leaves a notch in the
    /// ring that the ghost fills back in only as the note releases; thicker
    /// swells past the band toward the marks, and a slot's mark stands off its
    /// slice rather than off the band. 1 is the full slice, which is what every
    /// slot reads that is not lit or that nothing drives (see
    /// [`crate::intensity`]).
    pub thickness: [f32; OCTAVE_SLOTS],
    pub hovered: bool,
    /// On the home (center sevens) sheet. An idle node draws nothing
    /// wherever it sits; what marks a home position is the MARKER standing
    /// under where the node itself would draw, and off-sheet positions have
    /// not even that (see [`derive_pluses`]).
    pub on_home: bool,
    /// Billboard size, as a factor of the scene's `node_radius` (see
    /// [`ViewConfig::sevens_size`]): 1 on the home sheet, smaller with every
    /// step off it. Scales the whole node uniformly — the quad and its uv
    /// together — so every layer inside keeps its proportions and only the
    /// node's size on screen changes.
    pub scale: f32,
    /// The node's pitch class in cents under the current tuning, for the
    /// in-lattice cents readout.
    pub cents: f32,
    /// Octave slots (bit i = slot i) where this node carries the melody —
    /// the highest held note. 0 when it doesn't. One voice lights every node
    /// its pitch class matches under the tuning tolerance, and the mark follows
    /// the same rule.
    ///
    /// One bit at a time: the node carries one melody mark at one level, so
    /// the mask names the strongest carried slot ([`NodeMotion`]).
    /// A MASK rather than a slot index because 0 then says
    /// "unmarked" on its own, which a `0` index could not — and because it is
    /// what the shader tests a slot against.
    ///
    /// The mark wears the colour of the sector it extends, which the shader
    /// looks up from this slot's pitch as it does the slice's: the slot's
    /// pitch rather than the marking voice's, since a note past either end of
    /// the ring folds onto the outermost slot.
    pub melody_slots: u32,
    /// The same for the bass — the lowest held note. A slot set in BOTH
    /// masks is a note that is at once the melody and the bass (a lone held
    /// note, or the two ends of a chord voiced inside one octave). The two
    /// marks are then one slice extended once, in the one colour they both
    /// carry — the mark says that slice is an end of the chord, and it is
    /// both.
    pub bass_slots: u32,
    /// How far each mark has eased in, 0..1: a mark grows on over the Fade
    /// duration ([`ViewConfig::fade_time`]) from the moment its note TOOK
    /// that end (plus whatever wait [`ViewConfig::mark_delay`] asks for
    /// first), rather than appearing at full the frame it is claimed.
    /// Separate from `presence_with_opacity` because a mark can be arriving while the
    /// node it sits on has been fully lit for a while — the mark has to
    /// follow its own note, not the node's.
    ///
    /// On target loss the carried level reverses immediately, and any pending
    /// wait is canceled. Short notes therefore leave from their current level;
    /// a delayed mark that never arrived has nothing to fade out.
    ///
    /// Per node rather than per slot because one node carries at most one
    /// mark of each kind; the slots above say which sector it extends.
    pub melody_level: f32,
    pub bass_level: f32,
    /// How much of the audio ring this node wears, 0..=1 — the gate's answer
    /// for its wedges ([`RingGate`]) carried on the note Fade ([`RingFade`]),
    /// and never less than the node's own
    /// [`presence_with_opacity`](Self::presence_with_opacity).
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
    /// ring is gone keeps its octave band and its marks exactly as
    /// the keys drew them, and loses only the annulus between the core and the
    /// band.
    ///
    /// `1.0` out of [`derive_scene`], which is not a decision but the absence
    /// of one — nothing in this crate reads audio, so a scene derived without
    /// [`NodeMotion::step`] behind it is a scene where nothing has been
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
    /// is, and what it is made of (see [`GlowStep`]).
    ///
    /// [`NodeMotion::step`] writes the node's reading as it stands, its level
    /// and its light ([`glow_light`](Self::glow_light)). The shell's pass
    /// (`panes::glow_fade` in harmonigraph-ui) is what replaces both with where
    /// they have got to on the Glow attack and release.
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
    /// `presence_with_opacity` are the ones it would carry having never been
    /// played.
    pub trail: f32,
}

impl NodeInstance {
    /// Place an unlit node before spectral folding, motion, glow and trail
    /// supply their channels.
    pub fn at(
        lattice_pos: LatticePos,
        world_pos: Vec3,
        scale: f32,
        on_home: bool,
        cents: f32,
        hovered: bool,
    ) -> Self {
        Self {
            lattice_pos,
            world_pos,
            scale,
            on_home,
            cents,
            hovered,
            presence_with_opacity: 0.0,
            presence_without_opacity: 0.0,
            departing: false,
            slice_progress: [1.0; OCTAVE_SLOTS],
            thickness: [1.0; OCTAVE_SLOTS],
            octaves: [0.0; OCTAVE_SLOTS],
            melody_slots: 0,
            bass_slots: 0,
            melody_level: 0.0,
            bass_level: 0.0,
            // No audio has been measured, so nothing can be held back yet.
            audio_ring: 1.0,
            glow: GlowStep::default(),
            trail: 0.0,
        }
    }

    /// Whether this node is somewhere the picture accounts for, and so can
    /// carry pitch info (hover label, tuning readout). A sounding node always
    /// counts, on its presence WITHOUT Opacity as its name reads it: a note
    /// Opacity draws at nothing is still held there, and off the home sheet it
    /// would otherwise be the one held note with no name and no hover (#1677
    /// area 6 Q1). An idle node draws nothing at all, but a home-sheet position
    /// is still a place the marker field says is there — a marker stands
    /// exactly where a pointer goes looking.
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
        self.presence_without_opacity > 0.0 || self.on_home || self.trail > 0.0
    }

    /// How much NOTE NAME stands over this node, 0 to 1.
    ///
    /// The label pass's own question (`draw_node_labels` in
    /// `harmonigraph-ui`), answered HERE because two things turn on it and a
    /// second spelling of it is a picture that contradicts itself: the pass
    /// draws the name, and [`derive_pluses`] clears the
    /// resting marker out from under it. A marker behind a name is the one
    /// place in the lattice where two marks claim the same position, and the
    /// name is the better of the two — it says WHICH position, where the
    /// marker only says that there is one.
    ///
    /// So the label layer is the one that decides, and this is the whole of
    /// its rule: the node is sounding, hovered, or one Label visibility keeps
    /// at rest.
    ///
    /// A LEVEL rather than a yes or no, and that is the whole of what keeps
    /// the handoff continuous. Under [`NoteNames::Played`] a name is drawn at
    /// exactly the node's
    /// [`presence_without_opacity`](Self::presence_without_opacity)
    /// (`draw_node_labels` in `harmonigraph-ui`), so a released note spends
    /// the end of its fade with a name too faint to see. Answered as a
    /// predicate, the marker under it stays away for all of that and then
    /// arrives at full ground opacity the frame that presence reaches 0 — a
    /// hole in the field, and then a cross popping into it, once per note. As
    /// a level the two cross-fade: what the name gives up, the marker takes,
    /// and the position carries the same ink throughout.
    ///
    /// The presence without Opacity rather than with it: a soft
    /// note held at its Opacity reading and then reserved whole at key-up
    /// would step its name up and its cross out in one frame (#1316).
    ///
    /// The `is_visible` term re-checks what [`Scene::pick`] already enforces,
    /// and `hovered` is picking's alone, so it is a second lock on one door.
    /// It stays because the field is public shared state rather than picking's
    /// private output, and what it costs to be wrong is a name floating in the
    /// sevens dimension on a node nothing is sounding on.
    ///
    /// What it does NOT answer is whether the name lands on the PANE — that
    /// needs a rect and a projector, which is the label pass's business and
    /// not the scene's. It costs nothing: a name off the pane is a name over a
    /// marker that is off the pane too, and neither is drawn.
    pub fn name_level(&self, view: &ViewConfig) -> f32 {
        if !self.is_visible() {
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
        // written until the voice's Note fade ends, so until then there is
        // nothing to settle onto and the level has to be reserved ahead of the
        // record. Reserved on the way OUT alone, where a record
        // can actually land, and under the one mode that keeps one.
        //
        // Here rather than in the label pass alone, because the marker under
        // the name is this level's complement (`derive_pluses`): a reserve one
        // side only is a full name standing over a nearly-full marker for the
        // whole of a release, and then the marker going out in a single frame.
        let reserved = if view.note_names == NoteNames::Past
            && self.on_home
            && self.departing
            && self.presence_without_opacity > 0.0
        {
            1.0
        } else {
            0.0
        };
        self.presence_without_opacity.max(resting).max(reserved).clamp(0.0, 1.0)
    }

    /// Whether a note name is drawn over this node AT ALL — the gate the label
    /// pass asks before it lays a glyph out, where the marker under it wants
    /// [`name_level`](Self::name_level)'s ramp instead.
    ///
    /// One spelling still: this IS the level, asked for any at all.
    pub fn is_named(&self, view: &ViewConfig) -> bool {
        self.name_level(view) > 0.0
    }

    /// How lit the node is DRAWN: its most opaque octave slot's colour, Opacity
    /// included, which leaves at the release rather than waiting out a Fade-out
    /// delay as [`presence_with_opacity`](Self::presence_with_opacity) does.
    ///
    /// What the name's ink follows (`label_ink` in harmonigraph-ui); a node's
    /// light reads the same slots, marks added ([`glow_light`](Self::glow_light)),
    /// so a name and a halo cannot disagree about whether the node is going
    /// dark.
    pub fn drawn_level(&self) -> f32 {
        self.octaves.iter().copied().fold(0.0, f32::max)
    }

    /// What this node's light is made of as it stands: every slot's light and
    /// its mark (see [`GlowLight`]). A slice's Thickness does not weigh in: it
    /// changes how the slice is drawn, not its share of the halo's colour
    /// (#1784 G3). The one constructor of that
    /// reading — [`NodeMotion::step`] writes it, and the shell's glow pass
    /// steps toward it.
    pub fn glow_light(&self) -> GlowLight {
        let l = self.octaves.map(|level| level.clamp(0.0, 1.0));
        let mut m = [0.0_f32; OCTAVE_SLOTS];
        for (slots, level) in
            [(self.melody_slots, self.melody_level), (self.bass_slots, self.bass_level)]
        {
            for (slot, mark) in m.iter_mut().enumerate() {
                if slots & (1 << slot) != 0 {
                    *mark = mark.max(level.clamp(0.0, 1.0));
                }
            }
        }
        GlowLight { l, m }
    }
}

/// One marker standing at a lattice position: a small cross, drawn under the
/// nodes, and the whole of what an unplayed lattice draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlusInstance {
    /// Index of the home node this marker stands under in [`Scene::nodes`].
    /// Each node has at most one marker. Reordering nodes must update these
    /// frame-local indices; hand-built scenes must supply the same association.
    pub node: usize,
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
    /// The marker's opacity — its ink and the share of the shadow its cross
    /// casts (`plus_paint` in lattice.wgsl), both at once.
    ///
    /// ONE number for both, which is the rule about a cross and its shadow:
    /// they arrive and leave together, so a position handing itself over to a
    /// name never shows a cross with no shadow under it nor a shadow under no
    /// cross. The shadow a cross casts is worth exactly the cross, which is
    /// also what lets [`derive_pluses`] drop a
    /// fully-claimed marker outright rather than ship one that still shades.
    pub strength: f32,
}

/// Everything the renderer needs for one frame.
#[derive(Debug, PartialEq)]
pub struct Scene {
    /// The normalized look used to derive this snapshot. Geometry and colors
    /// below are derived values; renderers read the look's dials here.
    pub view: ViewConfig,
    /// Artistic edge half-band in logical points; the renderer keeps a one-pixel AA floor.
    pub edge_softness_points: f32,
    pub nodes: Vec<NodeInstance>,
    pub camera: Camera,
    /// Base node radius in world units.
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
    /// where its picture actually ends. It is also the circle a node's caster
    /// box is projected from (`LatticeCallback::from_scene`), that box being a
    /// bound on the ink rather than its shape.
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
    /// The angular use of a node's shared padding (see
    /// [`ViewConfig::ring_gap`]): between one octave sector and the next, on the
    /// band, on the audio ring's wedges and down a mark's own sides. Already
    /// clamped.
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
    /// [`derive_pluses`]): one cross per visible
    /// HOME-sheet position, each carrying
    /// [`ViewConfig::marker_ink`](crate::ViewConfig::marker_ink)'s grey as its
    /// colour — so a marker IS that grey rather than a brightness of it. A bar
    /// of its own and not [`lattice_ground`](Self::lattice_ground): the field
    /// says where the positions are, which is worth keeping legible at any
    /// brightness a node's empty rings are dialled to. The light standing at one
    /// washes over that grey whole, on the unlit side of
    /// [`ViewConfig::glow_wash`](crate::ViewConfig::glow_wash), so a marker
    /// inside a halo reads as standing in the light rather than as a hole
    /// punched in it.
    ///
    /// Off-sheet positions get none, and that is the whole of what says which
    /// sheet is the ground: a note there floats over the marker field instead
    /// of standing in it.
    pub pluses: Vec<PlusInstance>,
    /// Half an arm's thickness, as a SHARE of the arm's length — the shape's
    /// one proportion, and what the shader folds a fragment's distance against
    /// (see [`PLUS_WIDTH_PER_LABEL_SCALE`], which is the WHOLE thickness and in
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
    /// composited over, which is `skin::PICTURE` — the black the lattice
    /// pane paints its own rect with, as every other picture pane does (see
    /// [`skin::picture_color`]).
    ///
    /// No shadow reads it: a shadow is a multiply on what the frame already
    /// holds rather than a hole cut through to the pane. The final composite
    /// does (`fs_composite` in blit.wgsl), counting the pane fill this ground
    /// will add into the cap on displayed light.
    pub background: Vec4,
    /// Pitch->color lookup for the octave glyphs, the same table the marks
    /// are colored from; the renderer hands it to the shader (see [`pitch_ramp_lut`]).
    pub pitch_lut: [Vec4; PITCH_LUT_N],
    /// Where `pitch_lut`'s entries stand along the range, which the shader maps
    /// a `t` through before indexing (see [`LutSpacing`]).
    pub pitch_lut_spacing: LutSpacing,
    /// One quad-uv length of the home sheet, as a world length
    /// (`marker_world`): what converts the marker field between the units its
    /// own draws are in and the units every glow bar is dialled in.
    ///
    /// A marker's own draw needs both. Its billboard is sized in WORLD and the
    /// arm it is sized around arrives per instance as a world length, while the
    /// Shadow that quad has to hold is quad uv, being a node's bar — so one of
    /// the two has to be converted, and the conversion is a property of the
    /// scene rather than of any marker. Handing it over once is what keeps the
    /// shader from carrying a second copy of the uv rule (see
    /// [`Scene::node_radius`], which is the same rule for a node).
    pub marker_unit: f32,
    /// The glow's clock, in seconds, where the shell's glow pass has stepped
    /// this scene's lights (`panes::glow_fade` in harmonigraph-ui): what the
    /// glow's material, texture and breathing travel by in the renderer.
    /// `None` out of [`derive_scene`], a snapshot with no time in it, which
    /// holds them where they are.
    pub glow_clock: Option<f64>,
    /// Which Stars depths the editor solos: while any drawn depth is, only the
    /// soloed ones are composited. A viewing aid of the editor's window rather
    /// than part of the look (#1828): none out of [`derive_scene`], and only
    /// the editor's working lattice pane is handed one, so the Video preview
    /// and an export draw none.
    pub star_solo: [bool; star_plan::STAR_DEPTHS],
}

#[cfg(test)]
mod tests;
