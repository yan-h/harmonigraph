//! The piano roll's notes, drawn as instanced quads through a wgpu paint
//! callback instead of through egui's tessellator.
//!
//! **Why this exists.** The roll was the frame's dominant cost, and not
//! where it looked. Broken down (the performance overlay's Frame breakdown)
//! tessellation was ~0.5 ms while the vertex UPLOAD was 4-5 ms: 20k vertices
//! idle, 100k+ with notes on screen, in only ~20 primitives. Batching was
//! fine; the volume was the problem. egui is immediate-mode and re-uploads
//! every vertex every frame, so a roll that merely SCROLLS was re-sending
//! six figures of geometry 144 times a second. A note was three stroked,
//! anti-aliased rounded rects, each a couple of hundred vertices once its
//! corners and AA ring were subdivided.
//!
//! **What this does instead.** One quad per note segment, with a box signed
//! distance field in the fragment shader (`shaders/roll.wgsl`). The note's
//! solid body and the outline wrapping it are both read off that distance, so
//! the outline costs a compare rather than a second shape — and its fade, which
//! no stroke can draw at all, costs nothing on top. Four vertices per note
//! against several hundred: the upload stops mattering rather than getting
//! cheaper.
//!
//! **Why the buffer is still rewritten every frame.** The obvious next step
//! is an append-and-evict ring — settled notes never change, so they could
//! be uploaded once. They are not, deliberately. At 100 bytes a piece a busy
//! roll is tens of kilobytes a frame against the megabytes that were the
//! whole problem, so a ring would be optimizing three orders of magnitude
//! below the cost it was built for, and it would have to carry the far-edge
//! trap with it: a note crossing the window's oldest edge is TRUNCATED
//! there, rewriting its geometry every frame while it leaves (see
//! `panes/spectral/roll.rs`), so any cache has to retire chunks before they
//! reach it.
//! Rebuilding per frame keeps the geometry a pure function of `now` — which
//! is also what keeps the offline render deterministic.

use crate::uniforms::{uniform_group, Float2, Float4};

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::pass_aged::PassAged;
use crate::{create_vertex_buffer, wgpu, EGUI_BLEND};

pub(crate) const ROLL_SRC: &str = include_str!("shaders/roll.wgsl");

/// Entry points the roll shader must provide: the two vertex stages (the
/// outline's quad and the bodies'), the outline's mask, and the body layer in
/// each of the two shadings [`create_core_pipeline`] picks between. Its entry
/// point is assembled from a shading's name, so a rename in the WGSL is a panic
/// at pipeline creation and nothing sooner.
#[cfg(any(test, feature = "hot-reload"))]
pub(crate) const ROLL_ENTRY_POINTS: &[&str] = &[
    "vs_note",
    "vs_note_body",
    "fs_outline_mask",
    "fs_core_gamma",
    "fs_core_linear",
    "fs_body_holdout",
    "vs_note_cut",
    "fs_cut_reference",
    "fs_cut_blend",
    "vs_cover",
    "fs_glow_cut_gamma",
    "fs_glow_cut_linear",
];

/// One note segment: a solid box in the pane's (pitch, depth) plane, its
/// color, and the glowing outline standing outside every one of its sides.
///
/// Reading outward: [`core`](Self::core), then [`aura`](Self::aura) fading
/// out over the shadow style's reach, which is the whole roll's and rides in
/// the uniform rather than here.
///
/// Screen geometry, in egui POINTS, already resolved through the pane's
/// `Axes` — this crate never learns which way the pane is turned. Lengths
/// are along the pane's two axes rather than x/y for the same reason.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RollInstance {
    /// Center of the segment, in screen points.
    pub center: [f32; 2],
    /// Half extents of the note's solid body along (pitch, depth). This IS
    /// the note's painted extent: it is filled to its edge and nothing
    /// straddles the boundary.
    ///
    /// No corner radius: a note is a rectangle, always. Rounding was a setting
    /// and is gone — on the notes short enough for it to show at all, it only
    /// ever rounded a tapped key into a bead.
    pub half_extent: [f32; 2],
    /// The center line's pitch drift per point of depth: 0 for a held note,
    /// non-zero for a glide, which makes the box a parallelogram rather than
    /// needing a second shape.
    pub shear: f32,
    /// How much of this segment's LEADING end is a LEAD — a stretch of ribbon
    /// the caller grew the box by, drawn as an extension of the note rather
    /// than as part of it — in points, and 0 for the ordinary segment that is
    /// all note.
    ///
    /// The leading end is the one at the low end of the depth axis, which is the
    /// end nearest whatever the roll is drawn beside — for the spectral pane,
    /// the now-line and the analyzer past it (see
    /// `panes::spectral::roll::lead`, which is also the only thing that sets
    /// this).
    ///
    /// It is what divides the two: past this distance from the tip the segment
    /// is the note itself and is drawn solid, and inside it
    /// [`lead_fade`](Self::lead_fade) and [`lead_alpha`](Self::lead_alpha)
    /// decide what is left. Without it the two would have nothing to measure
    /// against and would take the note's own body with them.
    ///
    /// This crate is told how long the lead is and nothing about why. It does
    /// not grow the geometry to make room for one — the length is the caller's
    /// already — and it invents no lead of its own.
    pub lead: f32,
    /// How much of that lead is spent fading out at the tip, in points: 0 ends
    /// it square, the whole lead fades it from the note's own end outward.
    ///
    /// Both layers ride it: a lead that ends in a fade wears no outline cap
    /// either, or the tip would dissolve inside a hard ring of glow.
    pub lead_fade: f32,
    /// How much of the lead is still standing, 0..1 — the whole of it while the
    /// note is sounding, falling to nothing over whatever the caller counts as
    /// the note's release.
    ///
    /// It multiplies the LEAD alone and never the note, which is the reason
    /// [`lead`](Self::lead) has to be carried here at all. So a lead on the way
    /// out reads as a translucent extension of a solid ribbon, which is what it
    /// is; the two meet at the note's own end, and at full opacity that join is
    /// seamless.
    pub lead_alpha: f32,
    /// How far the outline's cap at the NOTE's own leading end reaches, in
    /// points — the cap standing where the note stops and the
    /// [`lead`](Self::lead) carries on.
    ///
    /// The note's own end is INTERIOR to a box that carries a lead, and a box
    /// has no edge in its middle for an outline to wrap, so without this the
    /// note wears no cap there for as long as its box keeps the lead.
    ///
    /// Drawn in the outline layer, which goes down before ANY body, so a lead
    /// standing over the cap covers it and a spent one (a
    /// [`lead_alpha`](Self::lead_alpha) of 0) leaves it against the note's
    /// end. The spectral pane hands over only those two: a sounding note's lead
    /// stands whole, and a released note keeps a spent lead on its box only
    /// while this reach is being metered, its fading tongue being a separate
    /// instance with no cap of its own.
    ///
    /// A REACH rather than an opacity, and that is the whole of what this field
    /// decides. The cap stands OUTSIDE the note's end, in the stretch the lead
    /// was drawn over, and a caller may have somewhere the ribbon's ink may not
    /// go — the spectral pane's now-line is that somewhere, the clip a lead
    /// opens past it belonging to the lead and not to a cap. Shortening the
    /// reach keeps the cap wholly on the near side of such a place, growing it
    /// out of the note's end as the room opens; dimming it instead would still
    /// put ink across it, just faintly, which is a weaker promise for the same
    /// arithmetic.
    ///
    /// The shadow style's own reach is the ordinary answer and the one a caller
    /// with nowhere to protect should give — more than that draws no more, the
    /// cap being bounded by the outline it is part of, and an outline of no
    /// reach wears no cap at all. Read only where there IS a lead: without one
    /// the box ends at the note and its cap is the box's own outline.
    pub cap_reach: f32,
    /// Premultiplied sRGB bytes, straight out of [`egui::Color32`].
    pub core: [u8; 4],
    /// The outline's glow: its color as straight (not premultiplied) sRGB bytes,
    /// and in alpha how strongly it covers where it is solid; the shadow
    /// style's darkness and the fade take it out from there, and an alpha of 0
    /// draws none. Every note's glow merges into one mask, the strongest
    /// keeping its color (see the head of roll.wgsl).
    ///
    /// Which color and how strong stay the pane's decision — this crate draws
    /// the instance it is handed and invents nothing.
    pub aura: [u8; 4],
    /// The stretch of depth this instance draws, as offsets from
    /// [`center`](Self::center) in points: [`WHOLE`](Self::WHOLE) for the
    /// whole box, or one PIECE of it.
    ///
    /// Pieces are how a note's opacity and thickness follow its playing
    /// along a segment: the caller hands over the segment's box once per
    /// piece, each with the same geometry and a span of its own, and the spans
    /// tile the box. So the outline, the lead and the cap are the segment's,
    /// and no piece has an end of its own for an outline to wrap — exactly,
    /// while the width holds still, and near a cut to within what a piece can
    /// see of its neighbours when it does not (see
    /// [`taper_depth`](Self::taper_depth)). `note_vertex` cuts the quad at the
    /// span, so two pieces meet on one shared edge and every pixel is drawn by
    /// one of them.
    pub span: [f32; 2],
    /// The two depth offsets [`fade`](Self::fade)
    /// are given at; each is a straight line between them, held past either
    /// end.
    pub ramp: [f32; 2],
    /// Opacity at the two [`ramp`](Self::ramp) depths, multiplying the body
    /// and the outline together. `[1.0, 1.0]` is unfaded.
    pub fade: [f32; 2],
    /// Four depth offsets, ascending, that [`taper`](Self::taper) is given at.
    ///
    /// The middle two are this piece's own ends and the outer two its
    /// neighbouring pieces' far ends, or a step's other side where one stands
    /// at a cut, so the outline near a cut is measured against the shape the
    /// piece beside it draws. Beyond them a piece cannot see, so a neighbour
    /// shorter than the outline's reach can leave the outline a step at the
    /// cut. Read only while [`taper`](Self::taper) moves.
    pub taper_depth: [f32; 4],
    /// The ribbon's width at each of the four [`taper_depth`](Self::taper_depth)
    /// depths, as a share of its full [`half_extent`](Self::half_extent)
    /// across pitch: straight lines between them, held past both ends, and
    /// growing about the center line. The body, the outline standing off it,
    /// the shadow and the bloom all follow the tapered shape, and its distance
    /// is still the true one. [`UNTAPERED`](Self::UNTAPERED) for a ribbon
    /// at full width all along.
    ///
    /// At most 1, since the quad, the culls and the shadow's cell are sized
    /// from the box: a caller whose ribbon swells hands over a box grown to
    /// its widest point and the widths as shares of that.
    pub taper: [f32; 4],
    /// Dense-repeat ornament: take-time phase at the center (modulo 6 points),
    /// then composition point scale. A zero scale leaves the body plain.
    /// Only the fill color changes; coverage, edges and shadows stay solid.
    pub tremolo: [f32; 2],
}

impl RollInstance {
    /// A [`span`](Self::span) that draws the whole box.
    pub const WHOLE: [f32; 2] = [f32::MIN, f32::MAX];
    /// A [`taper`](Self::taper) at full width all along.
    pub const UNTAPERED: [f32; 4] = [1.0; 4];
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<RollInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x2, // center
            1 => Float32x2, // half_extent
            2 => Float32,   // shear
            // Several to a slot, since a vertex takes sixteen at most.
            4 => Float32x4, // lead, lead_fade, lead_alpha, cap_reach
            8 => Unorm8x4,  // core
            9 => Unorm8x4,  // aura
            14 => Float32x4, // span, ramp
            15 => Float32x2, // fade
            5 => Float32x4,  // taper_depth
            6 => Float32x4,  // taper
            7 => Float32x2,  // tremolo
        ],
    };
}

/// Which way the pane's two axes run on screen: unit vectors for pitch (the
/// short side) and depth/time (the long side).
///
/// The pane rotates and flips, and rather than baking that into every
/// instance it rides in the uniform — one pair of vectors for the whole
/// roll, the same affine the egui path got from `Axes::at`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollAxes {
    pub pitch_dir: [f32; 2],
    pub depth_dir: [f32; 2],
}

/// The BACKDROP CUT round the roll's notes (#1525): the composite takes the
/// picture under the glow away faster than it lays the glow's color on, so a
/// note's surround holds over bright spectrogram energy where the glow alone,
/// never quite solid, lets it through.
///
/// Per note, `amount × ramp(u)`: whole out to the shadow style's spread, then
/// fading over `reach` σ along a curve between a straight line and a
/// smoothstep. Where the glow's strength is `s` and the notes' cuts combine to
/// `c`, the picture keeps `1 - max(s, c)` of what was there and gains the
/// glow's color at `max(s, fill × max(s, c))`. An `amount` of 0 is the glow
/// exactly, and over black the cut changes nothing.
///
/// Neighbouring notes' cuts combine as a smooth minimum of their distances,
/// softened by `blend` (0 the plain strongest-wins, which folds into a crease
/// along the line between two notes once the cut is wide and soft).
///
/// Every field is a 0..1 share but `reach`; [`RollCut::sanitized`] clamps
/// them, and the pane hands these over as the user dialled them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollCut {
    /// How much of the picture the cut takes at a note's edge.
    pub amount: f32,
    /// How far past the spread the cut fades out, in σ of the glow
    /// ([`ROLL_CUT_REACH_RANGE`]).
    pub reach: f32,
    /// The fade's shape: 0 a straight line, 1 a smoothstep.
    pub curve: f32,
    /// How much of what the cut takes is filled with the glow's own color
    /// rather than left black.
    pub fill: f32,
    /// How softly neighbouring notes' cuts merge: a smooth minimum of their
    /// distances, softened over `blend` × [`ROLL_HALO_BLEND_MAX`] of the cut's
    /// fade, and the plain minimum at 0.
    pub blend: f32,
}

/// The cut's reach, in σ of the glow, that [`RollCut::reach`] is held to.
pub const ROLL_CUT_REACH_RANGE: std::ops::RangeInclusive<f32> = 1.0..=8.0;

/// The halo blend's softness at a [`RollCut::blend`] of 1, as a share of the
/// cut's fade: the smooth minimum's `k` over the fade's length.
///
/// A share of the fade rather than of σ, because the crease the blend takes out
/// widens with the cut: a fade twice as long folds over twice the distance
/// and wants twice the softening. Half of this at the default four σ is
/// look-prototype round R8's pick, a `k` of σ / 2. Held to a quarter so a
/// note's share of the blend, which is lowered by its value at the cut's reach
/// (`fs_cut_blend` in roll.wgsl), keeps that value — and the slope it steps
/// by there — under e⁻⁴ of a share at the edge.
pub const ROLL_HALO_BLEND_MAX: f32 = 0.25;

impl RollCut {
    /// No cut: the glow alone.
    pub const OFF: Self = Self { amount: 0.0, reach: 4.0, curve: 1.0, fill: 0.0, blend: 0.5 };

    /// Every field in its range, a non-finite one taken as [`Self::OFF`]'s.
    pub fn sanitized(self) -> Self {
        let share = |v: f32, off: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { off };
        let off = Self::OFF;
        Self {
            amount: share(self.amount, off.amount),
            reach: if self.reach.is_finite() { self.reach } else { off.reach }
                .clamp(*ROLL_CUT_REACH_RANGE.start(), *ROLL_CUT_REACH_RANGE.end()),
            curve: share(self.curve, off.curve),
            fill: share(self.fill, off.fill),
            blend: share(self.blend, off.blend),
        }
    }
}

/// How far a roll note's surround paints past its box, in points: the glow's
/// reach or the cut's, whichever is longer, and 0 when the shadow is off. What
/// the pane's culls keep a leaving note alive for, and what it meters a cap's
/// room against.
pub fn roll_surround_reach(
    style: harmonigraph_scene::ShadowStyle,
    cut: RollCut,
    point_scale: f32,
) -> f32 {
    let glow = crate::shadow::spectral_shadow_reach(style, point_scale);
    let [_, _, spread, _] = shadow_uniform(style, point_scale);
    let [_, fade, _, _] = cut_uniform(style, cut, point_scale);
    if fade > 0.0 {
        glow.max(spread + fade)
    } else {
        glow
    }
}

/// Draw `instances` into `rect`. See [`crate::PaneIds`] for what identifies
/// the roll's own instance buffer (the pipeline is shared) and how long the
/// buffers live.
///
/// `rect` is the roll's own region rather than the pane's — it is what the
/// bloom's offscreen chain and the outlines' mask cover, so a rect any larger
/// would spend their resolution on somewhere the roll cannot draw. The
/// outlines stop at it; the caller's clip should too.
///
/// `bloom` is the strength the bloom runs at, applied to these notes through
/// the lattice's own chain (see [`RollBloom`]). 0 skips it whole.
///
/// The instances from `clipped_start` onward are clipped to `tail_rect`
/// while keeping one global outline-before-body paint order. This is for a
/// roll whose leading extensions occupy a stricter region than its scrolling
/// history.
#[allow(clippy::too_many_arguments)]
pub fn roll_paint_callback_with_clipped_tail(
    rect: egui::Rect,
    pane_size: egui::Vec2,
    instances: Vec<RollInstance>,
    clipped_start: usize,
    tail_rect: egui::Rect,
    axes: RollAxes,
    bloom: f32,
    shadow: harmonigraph_scene::ShadowStyle,
    cut: RollCut,
    target_format: wgpu::TextureFormat,
    ids: crate::PaneIds,
    point_scale: f32,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        RollCallback {
            point_scale,
            rect,
            pane_size,
            instances,
            clipped_tail: Some((clipped_start as u32, tail_rect)),
            axes,
            bloom,
            shadow,
            cut,
            target_format,
            pane_id: ids.pane,
            pass_nr: ids.pass_nr,
        },
    )
}

/// Per-frame, per-pane draw data, built on the UI thread.
struct RollCallback {
    point_scale: f32,
    /// The roll's region in points. `paint` is handed this as the callback's
    /// viewport; `prepare` is handed nothing, and needs it to size the bloom
    /// chain, so it rides here too.
    rect: egui::Rect,
    pane_size: egui::Vec2,
    instances: Vec<RollInstance>,
    /// A trailing range whose sharp ink is clipped more tightly than `rect`.
    /// Kept in the same callback so every outline still precedes every body.
    clipped_tail: Option<(u32, egui::Rect)>,
    axes: RollAxes,
    bloom: f32,
    shadow: harmonigraph_scene::ShadowStyle,
    cut: RollCut,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    /// Egui's cumulative pass number, shared by every callback prepared before
    /// this frame paints.
    pass_nr: u64,
}

uniform_group! {
struct RollUniforms {
    /// [`shadow_uniform`]'s four.
    shadow: Float4,
    /// [`cut_uniform`]'s four.
    cut: Float4,
    origin_points: Float2,
    viewport_points: Float2,
    feather: f32,
    _feather_pad: f32,
    pitch_dir: Float2,
    depth_dir: Float2,
    /// Where the target drawn into starts, in its own pixels, and how many
    /// points one of them measures, per axis: the fragment stage maps its own
    /// pixel back to the note through the pair (`surface_point` in
    /// roll.wgsl). Set only by [`drawn_into`](Self::drawn_into), with the
    /// viewport above.
    origin_pixels: Float2,
    pixel_points: Float2,
    /// [`RollCut::fill`], 0 while the cut is off.
    cut_fill: f32,
    _tail_pad: f32,
}
}

impl RollUniforms {
    /// These uniforms drawing into a target `target_px` pixels across that
    /// covers the surface's device pixels from `origin_px`, `size_px` across:
    /// the viewport the vertex stage maps onto (`origin_points`,
    /// `viewport_points`), and the pair the fragment stage maps back through
    /// (`origin_pixels`, `pixel_points`). One function for all four, so the
    /// two mappings cannot drift apart.
    ///
    /// A target the size of what it covers — the surface itself, and the body
    /// holdout over the roll's rect — measures a pixel by exactly `1 / ppp`
    /// from an origin in whole device pixels. Its pixel sums are exact in
    /// f32, so every such target lands on the same bits for one surface
    /// pixel at any pixels-per-point, where a scale derived per target
    /// (`viewport_points / size`) rounds differently for each size unless
    /// `ppp` is a power of two. The bloom's half-size target, twice the
    /// chain's rounded-up quarter (`crate::bloom_sizes`), takes the per-axis
    /// scale its own pixels measure.
    fn drawn_into(
        self,
        ppp: f32,
        origin_px: [i32; 2],
        size_px: [u32; 2],
        target_px: [u32; 2],
    ) -> Self {
        // Exactly 1 for a target the size of what it covers.
        let shrink = |axis: usize| target_px[axis].max(1) as f32 / size_px[axis].max(1) as f32;
        Self {
            origin_points: Float2(origin_px.map(|v| v as f32 / ppp)),
            viewport_points: Float2(size_px.map(|v| v as f32 / ppp)),
            origin_pixels: Float2([0, 1].map(|axis| origin_px[axis] as f32 * shrink(axis))),
            pixel_points: Float2([0, 1].map(|axis| 1.0 / (shrink(axis) * ppp))),
            ..self
        }
    }
}

/// The bloom's notes texture for a roll `size` device pixels across: the
/// chain's own half (`crate::bloom_sizes`), which it is read into 1:1.
fn bloom_notes_size(size: [u32; 2]) -> [u32; 2] {
    crate::bloom_sizes(size).0
}

/// The glow's σ, strength, solid spread and whole reach, in points but for the
/// strength — what `outline_coverage` in roll.wgsl draws its profile from.
///
/// The spectral geometry group's own numbers, read as a distance profile: σ is
/// the σ its blur would take and the spread its blur's source expansion, so the
/// glow reaches exactly as far as [`crate::shadow::spectral_shadow_reach`] says,
/// which is what the pane's culls keep a leaving note alive for.
fn shadow_uniform(style: harmonigraph_scene::ShadowStyle, point_scale: f32) -> [f32; 4] {
    let style = style.clamped(harmonigraph_scene::SPECTRAL_SHADOW_MAX);
    if !style.casts() {
        return [0.0; 4];
    }
    let sigma = crate::shadow::spectral_sigma_points(style, point_scale);
    [
        sigma,
        style.depth,
        style.gaussian_spread_points(sigma),
        crate::shadow::spectral_shadow_reach(style, point_scale),
    ]
}

/// The cut's amount, its fade in points past the spread, its curve, and the
/// halo blend's softness as a share of that fade — `locals.cut` in roll.wgsl,
/// read by `cut_coverage` and its kin there. All 0 while the cut is off, which
/// is when the roll casts nothing or the amount is 0.
///
/// The softness is `blend × ROLL_HALO_BLEND_MAX`, in the fade's own units,
/// which is where the shader measures the distances it blends.
fn cut_uniform(style: harmonigraph_scene::ShadowStyle, cut: RollCut, point_scale: f32) -> [f32; 4] {
    let cut = cut.sanitized();
    let [sigma, ..] = shadow_uniform(style, point_scale);
    if sigma <= 0.0 || cut.amount <= 0.0 {
        return [0.0; 4];
    }
    [cut.amount, cut.reach * sigma, cut.curve, cut.blend * ROLL_HALO_BLEND_MAX]
}

/// The bloom's strength, alone in its own buffer (`AddUniforms` in blit.wgsl).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BloomUniforms {
    /// Strength in x; the rest is the 16 bytes a uniform block takes.
    strength: [f32; 4],
}

/// GPU objects cached across frames in egui-wgpu's `CallbackResources`.
struct RollResources {
    /// The note layers, in the order they are drawn: every outline, then
    /// every body over them — see the head of roll.wgsl for why the order is
    /// the whole point, and what the second draw costs. The outlines go into a
    /// [`RollHoldout`]'s mask (`mask_pipeline`), which is laid over the picture
    /// in one draw (`composite_pipeline`, through the mask's `filter_layout`).
    mask_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    /// The backdrop cut's two passes into a [`RollCutTargets`], and the
    /// composite that lays mask and cut over the picture in place of
    /// `composite_pipeline` while the cut is on; with the layouts the second
    /// pass and the composite read those targets through.
    cut_reference_pipeline: wgpu::RenderPipeline,
    cut_blend_pipeline: wgpu::RenderPipeline,
    cut_composite_pipeline: wgpu::RenderPipeline,
    cut_blend_layout: wgpu::BindGroupLayout,
    cut_composite_layout: wgpu::BindGroupLayout,
    core_pipeline: wgpu::RenderPipeline,
    /// The bodies' coverage, unioned into a [`RollHoldout`] before either
    /// layer is drawn, and the layout the outline layer reads it through.
    holdout_pipeline: wgpu::RenderPipeline,
    holdout_layout: wgpu::BindGroupLayout,
    /// The bodies again, into the bloom chain's own [`BLOOM_FORMAT`] rather
    /// than the target's, shaded as the target is and at each note's own
    /// opacity reading, so the halo is the colour and strength the notes are.
    light_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// The bloom chain's four post passes and the one that lays the result
    /// over the notes. The four are the lattice's, out of the same blit.wgsl
    /// and stepped through by the same [`crate::BloomChain::run`], so the
    /// halo the roll grows is the halo the lattice grows: same threshold at
    /// the same resolution, same knee, same kernel, same fractions of the
    /// pane's screen size, in the same half floats ([`BLOOM_FORMAT`]). Only
    /// the threshold's reading of coverage and the composite's target are the
    /// roll's own, which is why the pipelines are its own and the chain is not.
    bright_pipeline: wgpu::RenderPipeline,
    downsample_pipeline: wgpu::RenderPipeline,
    blur_h_pipeline: wgpu::RenderPipeline,
    blur_v_pipeline: wgpu::RenderPipeline,
    bloom_pipeline: wgpu::RenderPipeline,
    /// One sampled texture + the shared sampler (the three chain passes).
    filter_layout: wgpu::BindGroupLayout,
    /// The same, plus the strength (the pass into the egui pass).
    bloom_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    target_format: wgpu::TextureFormat,
    #[cfg(feature = "hot-reload")]
    generation: u64,
    /// What a closed roll would otherwise hold: its buffers and its bloom
    /// chain, three textures the size of the pane it was shown at. That is the
    /// reason there is a sweep here at all rather than a map that only ever
    /// grows; the age it sweeps at is [`crate::pass_aged::TTL_PASSES`].
    panes: PassAged<RollPane>,
}

struct RollPane {
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    capacity: usize,
    count: u32,
    /// The bloom's targets, sized to the roll's rect. `None` until a frame
    /// asks for bloom, and rebuilt when the rect resizes — a roll with the
    /// strength at 0 pays for none of it.
    bloom: Option<RollBloom>,
    /// Built when the roll casts a shadow, and rebuilt when the rect resizes.
    holdout: Option<RollHoldout>,
}

/// The union of every note's body coverage over the roll's rect, one texel
/// per device pixel, which the outline layer takes off every shadow (#1293),
/// and the mask that layer is merged into.
///
/// A shadow drawn under every body still shows through any body drawn at
/// less than full opacity, and blending cannot take it back out afterwards:
/// by the time a body lands, the frame under it no longer says how much of it
/// was shadow. So the bodies' coverage is drawn first, offscreen, and the
/// shadows read it where they are composited. Full resolution because it
/// meets the body edges pixel for pixel; half floats because the outline's
/// own edge takes the larger of this and its note's exact fill, and eight bits
/// of coverage would shift those edges by a rounding step.
struct RollHoldout {
    view: wgpu::TextureView,
    /// Notes mapped into that texture rather than onto the surface.
    notes_uniform: wgpu::Buffer,
    notes_bind_group: wgpu::BindGroup,
    /// The texture, as the outline layer's group 1.
    read_bind_group: wgpu::BindGroup,
    /// Every note's glow over the same rect, kept where it is the strongest of
    /// any note's along with that note's color: drawn in `prepare` after the
    /// holdout it reads, and laid over the picture by `paint`. One mask rather
    /// than an outline blended per note, so where two notes' glows meet the
    /// picture is as covered as the stronger of them and no more.
    mask_view: wgpu::TextureView,
    /// The mask's strength again, as depth, which is what picks the strongest
    /// note's fragment ([`MASK_DEPTH_FORMAT`]).
    mask_depth_view: wgpu::TextureView,
    /// The mask, as the composite's texture and sampler.
    mask_read_bind_group: wgpu::BindGroup,
    /// The backdrop cut's targets over the same rect: kept while the cut is
    /// on, and dropped with it.
    cut: Option<RollCutTargets>,
    /// Where `paint` cuts the clipped tail off, which the passes into these
    /// textures cut it off at too.
    tail: Option<TailClip>,
    /// The roll's size in device pixels this was built for.
    size: [u32; 2],
}

/// The backdrop cut over a [`RollHoldout`]'s rect, one texel per device pixel
/// as the mask beside it.
///
/// Two targets because the halo blend is a smooth minimum over every note,
/// summed under an additive blend, and the sum keeps its precision only
/// relative to the nearest note's cut (see `fs_cut_blend` in roll.wgsl): the
/// first pass finds that nearest cut, the second sums against it. At a blend
/// of 0 the first is the cut, and the second is skipped.
struct RollCutTargets {
    /// Every fragment's cut as a distance under a `Less` test: the nearest
    /// note's, in sixteen bits.
    reference_view: wgpu::TextureView,
    /// The halo blend's sum, in half floats: between 1 and the count of notes
    /// reaching a texel, so their eleven bits are a constant relative error.
    blend_view: wgpu::TextureView,
    /// The holdout (which the cut is held out by, as the glow is) and the
    /// reference, as the blend pass's group 1.
    blend_read_bind_group: wgpu::BindGroup,
    /// The mask, the reference and the sum, as the composite's group 1.
    composite_bind_group: wgpu::BindGroup,
}

/// The cut's nearest distance per texel; Depth16Unorm, as the mask's depth.
const CUT_REFERENCE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;

/// The halo blend's sum: see [`RollCutTargets::blend_view`].
const CUT_BLEND_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

impl RollCutTargets {
    fn new(
        device: &wgpu::Device,
        blend_layout: &wgpu::BindGroupLayout,
        composite_layout: &wgpu::BindGroupLayout,
        holdout_view: &wgpu::TextureView,
        mask_view: &wgpu::TextureView,
        size: [u32; 2],
    ) -> Self {
        let target = |label, format| {
            crate::gpu_memory::texture(
                device,
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
            .create_view(&Default::default())
        };
        let reference_view = target("roll_cut_reference", CUT_REFERENCE_FORMAT);
        let blend_view = target("roll_cut_blend", CUT_BLEND_FORMAT);
        let view = |binding, view| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::TextureView(view),
        };
        RollCutTargets {
            blend_read_bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("roll_cut_blend_read_bind_group"),
                layout: blend_layout,
                entries: &[view(2, holdout_view), view(5, &reference_view)],
            }),
            composite_bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("roll_cut_composite_bind_group"),
                layout: composite_layout,
                entries: &[view(4, mask_view), view(5, &reference_view), view(6, &blend_view)],
            }),
            reference_view,
            blend_view,
        }
    }
}

/// The instances from `start` on, drawn only inside `scissor` (in the roll's
/// own pixels), and not at all when that is `None`: the clipped tail, as a
/// texture over the roll's rect sees it.
#[derive(Clone, Copy)]
struct TailClip {
    start: u32,
    scissor: Option<[u32; 4]>,
}

/// Every instance into a texture over the roll's rect, the clipped tail only
/// as far as `paint` lets it draw.
fn draw_clipped(pass: &mut wgpu::RenderPass<'_>, count: u32, tail: Option<TailClip>) {
    let Some(tail) = tail.filter(|tail| tail.start < count) else {
        pass.draw(0..4, 0..count);
        return;
    };
    pass.draw(0..4, 0..tail.start);
    if let Some([left, top, width, height]) = tail.scissor {
        pass.set_scissor_rect(left, top, width, height);
        pass.draw(0..4, tail.start..count);
    }
}

/// The holdout's format: see [`RollHoldout`] for why half floats.
const HOLDOUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// The glow mask's format: straight gamma-space color, and the strength in
/// alpha. Eight bits, unlike the holdout: what it holds is the final color and
/// alpha of a glow laid over an eight-bit surface, so all it costs is one
/// rounding at the composite.
const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// `1 - strength` per mask texel, under a `Less` test: the strongest note's
/// fragment is the one that lands, its color with it. Sixteen bits resolves
/// strength far finer than the eight the mask's alpha keeps.
const MASK_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;

/// The roll's picture to bloom, and the lattice's own [`crate::BloomChain`] over it:
/// the notes rendered again offscreen, thresholded, blurred separably, and
/// added back over the sharp notes as light.
///
/// The notes are RE-RENDERED rather than read back out of the egui pass, and
/// that is the whole reason this is cheap enough to do at all: the sharp layer
/// stays exactly where it was, drawn straight into the egui pass at full
/// resolution with no resampling between the distance field and the screen.
/// A pass that read the finished picture back would have to reproduce that
/// alignment to the pixel, on a picture that scrolls sub-pixel; a pass that
/// only feeds a blur does not care where its texels landed to half a pixel.
struct RollBloom {
    /// The notes again, at HALF the roll's screen size — the picture the
    /// chain's threshold reads, standing where the lattice's own scene
    /// texture stands.
    ///
    /// Drawing at half skips a resample the lattice pays for. A ribbon at the
    /// roll's two-device-pixel length floor is only one texel here, so its peak
    /// depends on sub-texel phase; the roll's coverage-aware bright pass
    /// thresholds its straight color and applies coverage afterwards, keeping
    /// that phase from becoming a brightness pulse.
    notes_view: wgpu::TextureView,
    /// Notes mapped into that texture rather than onto the surface.
    notes_uniform: wgpu::Buffer,
    notes_bind_group: wgpu::BindGroup,
    bloom: crate::BloomChain,
    /// Quarter A (where the vertical blur lands) plus the strength.
    add_bind_group: wgpu::BindGroup,
    strength_buffer: wgpu::Buffer,
    /// The roll's size in device pixels this was built for.
    size: [u32; 2],
}

/// The bloom chain's working format, from the notes it re-renders to the
/// blurred quarter its composite samples. Half floats preserve faint gradients
/// through the blur passes, as in the lattice's `LATTICE_COLOR_FORMAT`.
const BLOOM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Starting size of a pane's instance buffer; it grows by
/// `next_power_of_two` when a frame overflows it. A roll holds a few
/// hundred notes at the spans this pane is used at.
const INITIAL_NOTE_CAPACITY: usize = 512;

#[cfg(test)]
pub(super) fn timing_pipeline_startup(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> std::time::Duration {
    let started = std::time::Instant::now();
    let _resources = RollResources::new(device, format);
    started.elapsed()
}

impl RollResources {
    fn is_stale(&self, target_format: wgpu::TextureFormat) -> bool {
        #[cfg(feature = "hot-reload")]
        if self.generation != crate::reload::generation() {
            return true;
        }
        self.target_format != target_format
    }

    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let uniform_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("roll_bind_group_layout"),
            entries: &[uniform_entry(0)],
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let filter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("roll_filter_bind_group_layout"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });
        // Binding 2, matching `body_holdout` in roll.wgsl.
        let holdout_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("roll_holdout_bind_group_layout"),
            entries: &[texture_entry(2)],
        });
        // Binding 5, matching `cut_reference` in roll.wgsl, and 4 and 6 its
        // `glow_mask` and `cut_blend`.
        let depth_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let cut_blend_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("roll_cut_blend_bind_group_layout"),
            entries: &[texture_entry(2), depth_entry(5)],
        });
        let cut_composite_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("roll_cut_composite_bind_group_layout"),
                entries: &[texture_entry(4), depth_entry(5), texture_entry(6)],
            });
        // Binding 4 for the strength, matching `AddUniforms` in blit.wgsl —
        // the lattice's own composite holds 3.
        let bloom_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("roll_bloom_bind_group_layout"),
            entries: &[
                texture_entry(0),
                sampler_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("roll_shader"),
            source: wgpu::ShaderSource::Wgsl(crate::roll_source().into()),
        });
        let core_pipeline = |format| {
            create_core_pipeline(device, &shader, format, target_format.is_srgb(), &layout)
        };
        // The chain overwrites its whole target, so those three take no blend;
        // the bloom composite uses ordinary over in the egui pass.
        let blit_shader = crate::blit_module(device);
        let filter = |entry| {
            crate::create_post_pipeline(
                device,
                &blit_shader,
                entry,
                BLOOM_FORMAT,
                &filter_layout,
                None,
            )
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("roll_bloom_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        RollResources {
            mask_pipeline: create_mask_pipeline(device, &shader, &layout, &holdout_layout),
            // The mask holds gamma-space color, so a target that encodes for
            // itself takes the linear composite, as the bodies take theirs.
            composite_pipeline: crate::create_post_pipeline(
                device,
                &blit_shader,
                if target_format.is_srgb() { "fs_shadow_mask_linear" } else { "fs_shadow_mask" },
                target_format,
                &filter_layout,
                Some(EGUI_BLEND),
            ),
            cut_reference_pipeline: create_cut_reference_pipeline(
                device,
                &shader,
                &layout,
                &holdout_layout,
            ),
            cut_blend_pipeline: create_cut_blend_pipeline(
                device,
                &shader,
                &layout,
                &cut_blend_layout,
            ),
            cut_composite_pipeline: create_cut_composite_pipeline(
                device,
                &shader,
                target_format,
                &layout,
                &cut_composite_layout,
            ),
            cut_blend_layout,
            cut_composite_layout,
            core_pipeline: core_pipeline(target_format),
            light_pipeline: core_pipeline(BLOOM_FORMAT),
            holdout_pipeline: create_holdout_pipeline(device, &shader, &layout),
            holdout_layout,
            layout,
            bright_pipeline: filter("fs_bright_coverage"),
            downsample_pipeline: filter("fs_downsample"),
            blur_h_pipeline: filter("fs_blur_h"),
            blur_v_pipeline: filter("fs_blur_v"),
            bloom_pipeline: crate::create_post_pipeline(
                device,
                &blit_shader,
                "fs_bloom_add",
                target_format,
                &bloom_layout,
                Some(EGUI_BLEND),
            ),
            filter_layout,
            bloom_layout,
            // Linear, and the reason is the halo rather than the notes: the
            // chain reads each target at half the size of the one before it,
            // so every hop is a resample.
            sampler,
            target_format,
            #[cfg(feature = "hot-reload")]
            generation: crate::reload::generation(),
            panes: PassAged::new(),
        }
    }
}

/// What a [`RollBloom`] needs from [`RollResources`], as a borrow narrow
/// enough to hold while the pane it belongs to is borrowed mutably — which is
/// why `prepare` takes the struct apart field by field rather than asking it
/// for this.
struct RollBloomShared<'a> {
    notes_layout: &'a wgpu::BindGroupLayout,
    filter_layout: &'a wgpu::BindGroupLayout,
    bloom_layout: &'a wgpu::BindGroupLayout,
    sampler: &'a wgpu::Sampler,
}

impl RollPane {
    /// This pane's buffers, made on first sight of its id and stamped with
    /// `pass_nr` so the sweep can tell a live pane from one whose tab was
    /// closed.
    fn get<'a>(
        panes: &'a mut PassAged<RollPane>,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        pane_id: u64,
        pass_nr: u64,
    ) -> &'a mut RollPane {
        panes.touched_or_insert_with(pane_id, pass_nr, || {
            let uniform_buffer = crate::gpu_memory::buffer(
                device,
                &wgpu::BufferDescriptor {
                    label: Some("roll_uniforms"),
                    size: std::mem::size_of::<RollUniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            );
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("roll_bind_group"),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            });
            RollPane {
                uniform_buffer,
                bind_group,
                instance_buffer: create_vertex_buffer::<RollInstance>(
                    device,
                    "roll_notes",
                    INITIAL_NOTE_CAPACITY,
                ),
                capacity: INITIAL_NOTE_CAPACITY,
                count: 0,
                bloom: None,
                holdout: None,
            }
        })
    }
}

impl RollHoldout {
    fn new(
        device: &wgpu::Device,
        notes_layout: &wgpu::BindGroupLayout,
        read_layout: &wgpu::BindGroupLayout,
        mask_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        size: [u32; 2],
    ) -> Self {
        let target = |label, format| {
            crate::gpu_memory::texture(
                device,
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
            .create_view(&Default::default())
        };
        let view = target("roll_holdout", HOLDOUT_FORMAT);
        let mask_view = target("roll_shadow_mask", MASK_FORMAT);
        let mask_depth_view = crate::gpu_memory::texture(
            device,
            &wgpu::TextureDescriptor {
                label: Some("roll_shadow_mask_depth"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: MASK_DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            },
        )
        .create_view(&Default::default());
        let notes_uniform = crate::gpu_memory::buffer(
            device,
            &wgpu::BufferDescriptor {
                label: Some("roll_holdout_uniforms"),
                size: std::mem::size_of::<RollUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        RollHoldout {
            notes_bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("roll_holdout_notes_bind_group"),
                layout: notes_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: notes_uniform.as_entire_binding(),
                }],
            }),
            read_bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("roll_holdout_read_bind_group"),
                layout: read_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&view),
                }],
            }),
            mask_read_bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("roll_shadow_mask_read_bind_group"),
                layout: mask_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&mask_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            }),
            view,
            mask_view,
            mask_depth_view,
            notes_uniform,
            cut: None,
            tail: None,
            size,
        }
    }
}

impl RollBloom {
    /// Build the chain for a roll `size` device pixels across. Half and
    /// quarter of THAT, so the halo is a constant share of the roll's own
    /// screen size — the same rule the lattice's chain follows.
    fn new(device: &wgpu::Device, shared: &RollBloomShared<'_>, size: [u32; 2]) -> Self {
        let [hw, hh] = bloom_notes_size(size);
        let notes_view = crate::gpu_memory::texture(
            device,
            &wgpu::TextureDescriptor {
                label: Some("roll_bloom_notes"),
                size: wgpu::Extent3d { width: hw, height: hh, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: BLOOM_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        )
        .create_view(&Default::default());
        let bloom = crate::BloomChain::new(
            device,
            "roll",
            BLOOM_FORMAT,
            shared.filter_layout,
            shared.sampler,
            &notes_view,
            size,
        );

        let notes_uniform = crate::gpu_memory::buffer(
            device,
            &wgpu::BufferDescriptor {
                label: Some("roll_bloom_notes_uniforms"),
                size: std::mem::size_of::<RollUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        let strength_buffer = crate::gpu_memory::buffer(
            device,
            &wgpu::BufferDescriptor {
                label: Some("roll_bloom_strength"),
                size: std::mem::size_of::<BloomUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        RollBloom {
            notes_bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("roll_bloom_notes_bind_group"),
                layout: shared.notes_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: notes_uniform.as_entire_binding(),
                }],
            }),
            add_bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("roll_bloom_add_bind_group"),
                layout: shared.bloom_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&bloom.quarter_a_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(shared.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: strength_buffer.as_entire_binding(),
                    },
                ],
            }),
            notes_view,
            bloom,
            notes_uniform,
            strength_buffer,
            size,
        }
    }
}

/// The body layer, in ordinary premultiplied alpha blending. A note's opacity
/// is its instance's `fade`, which takes its body and its outline out together.
/// `srgb` is whether the surface the notes end on encodes for itself, which
/// picks the shading; it is the surface's even where `target_format` is the
/// bloom chain's.
fn create_core_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    srgb: bool,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("roll_pipeline_layout"),
        bind_group_layouts: &[Some(layout)],
        ..Default::default()
    });
    // Same fork egui makes, for the same reason: an sRGB-aware target wants
    // linear values and encodes them itself.
    let shade = if srgb { "linear" } else { "gamma" };
    let entry_point = format!("fs_core_{shade}");
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("roll_core"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            // The body, here and in the bloom's notes, takes a quad one
            // feather wider than itself.
            entry_point: Some("vs_note_body"),
            compilation_options: Default::default(),
            buffers: &[RollInstance::LAYOUT],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(&entry_point),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(EGUI_BLEND),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Every note's glow into a [`RollHoldout`]'s mask, the strongest kept with its
/// color, so the glows merge rather than stack: each fragment writes its
/// strength as depth ([`MASK_DEPTH_FORMAT`]) and lands only where it beats
/// what is there.
fn create_mask_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    holdout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("roll_shadow_mask_pipeline_layout"),
        bind_group_layouts: &[Some(layout), Some(holdout)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("roll_shadow_mask"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            // The outline reaches past its note by the shadow's reach.
            entry_point: Some("vs_note"),
            compilation_options: Default::default(),
            buffers: &[RollInstance::LAYOUT],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_outline_mask"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: MASK_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: MASK_DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Every note's cut into a [`RollCutTargets`]' reference, as a distance under
/// a `Less` test, so the nearest note's lands. Depth alone: the distance is
/// all the pass is for.
fn create_cut_reference_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    holdout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("roll_cut_reference_pipeline_layout"),
        bind_group_layouts: &[Some(layout), Some(holdout)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("roll_cut_reference"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_note_cut"),
            compilation_options: Default::default(),
            buffers: &[RollInstance::LAYOUT],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_cut_reference"),
            compilation_options: Default::default(),
            targets: &[],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: CUT_REFERENCE_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Every note's share of the halo blend into a [`RollCutTargets`]' sum, added
/// as it lands, with no depth test: every note reaching a texel counts.
fn create_cut_blend_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
    read: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("roll_cut_blend_pipeline_layout"),
        bind_group_layouts: &[Some(layout), Some(read)],
        ..Default::default()
    });
    let add = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("roll_cut_blend"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_note_cut"),
            compilation_options: Default::default(),
            buffers: &[RollInstance::LAYOUT],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_cut_blend"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: CUT_BLEND_FORMAT,
                blend: Some(wgpu::BlendState { color: add, alpha: add }),
                write_mask: wgpu::ColorWrites::RED,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// The glow mask and the cut over the roll's rect in one draw, premultiplied
/// over the picture, in the shading the target wants.
fn create_cut_composite_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::BindGroupLayout,
    read: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("roll_cut_composite_pipeline_layout"),
        bind_group_layouts: &[Some(layout), Some(read)],
        ..Default::default()
    });
    let entry_point =
        if target_format.is_srgb() { "fs_glow_cut_linear" } else { "fs_glow_cut_gamma" };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("roll_cut_composite"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_cover"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry_point),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(EGUI_BLEND),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// The bodies' coverage into a [`RollHoldout`], unioned as it lands: each
/// note leaves `1 - coverage` of what is already there uncovered.
fn create_holdout_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("roll_holdout_pipeline_layout"),
        bind_group_layouts: &[Some(layout)],
        ..Default::default()
    });
    let union = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrc,
        operation: wgpu::BlendOperation::Add,
    };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("roll_holdout"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            // Body coverage alone, so the body's quad.
            entry_point: Some("vs_note_body"),
            compilation_options: Default::default(),
            buffers: &[RollInstance::LAYOUT],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_body_holdout"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: HOLDOUT_FORMAT,
                blend: Some(wgpu::BlendState { color: union, alpha: union }),
                write_mask: wgpu::ColorWrites::RED,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl CallbackTrait for RollCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let recreate = callback_resources
            .get::<RollResources>()
            .is_none_or(|r| r.is_stale(self.target_format));
        if recreate {
            callback_resources.insert(RollResources::new(device, self.target_format));
        }
        let resources: &mut RollResources =
            callback_resources.get_mut().expect("inserted above when missing");
        let ppp = screen_descriptor.pixels_per_point.max(f32::EPSILON);
        let shadow = shadow_uniform(self.shadow, self.point_scale);
        let sigma = shadow[0];
        let cut = cut_uniform(self.shadow, self.cut, self.point_scale);
        // The roll's own rect in device pixels, which is what the bloom chain
        // and the body holdout are sized against. Through epaint's own
        // conversion rather than a rounded `width * ppp`, because that
        // conversion is what `paint` stretches the finished halo across: it
        // rounds each EDGE and subtracts, then clamps to the screen, so a rect
        // whose edges round in opposite directions measures a pixel less than
        // its width does, and a rect hanging off the screen measures less
        // again. Sized either way but stretched this way, the halo comes out
        // scaled against the notes it grew from, and slid by whatever the
        // clamp took.
        //
        // A roll thinner than a pixel in either direction has no picture to
        // bloom or hold out.
        let viewport = egui::epaint::ViewportInPixels::from_points(
            &self.rect,
            ppp,
            screen_descriptor.size_in_pixels,
        );
        let roll_size = [viewport.width_px.max(0) as u32, viewport.height_px.max(0) as u32];
        let has_area = !self.instances.is_empty() && roll_size.iter().all(|&d| d > 0);
        let surface = screen_descriptor.size_in_pixels;
        let uniforms = RollUniforms {
            // The whole surface, which is the viewport `paint` draws into
            // (`drawn_into` below).
            origin_points: Float2([0.0; 2]),
            viewport_points: Float2([0.0; 2]),
            origin_pixels: Float2([0.0; 2]),
            pixel_points: Float2([0.0; 2]),
            // One physical pixel, expressed in the points the geometry is
            // in. Derived rather than sampled from the fragment's
            // derivatives so coverage is a pure function of the uniforms,
            // which is what the offline render's byte-for-byte determinism
            // test rests on.
            feather: 1.0 / ppp,
            _feather_pad: 0.0,
            pitch_dir: Float2(self.axes.pitch_dir),
            depth_dir: Float2(self.axes.depth_dir),
            shadow: Float4(shadow),
            cut: Float4(cut),
            cut_fill: if cut[0] > 0.0 { self.cut.sanitized().fill } else { 0.0 },
            _tail_pad: 0.0,
        }
        .drawn_into(ppp, [0, 0], surface, surface);

        // The viewport's own edges: a texture over the roll's rect covers
        // exactly the pixels `paint` will lay it over, so the notes in it
        // stand where the notes under it do.
        let roll_origin = [viewport.left_px, viewport.top_px];
        let roll_uniforms = uniforms.drawn_into(ppp, roll_origin, roll_size, roll_size);
        // Half the roll's size for the bloom's notes, so this is what one
        // pixel of THAT target measures in points — twice the display's, and
        // the ramp has to follow it to conserve a hairline ribbon's weight.
        let half_ppp = ppp * 0.5;
        let bloom_pass = (self.bloom > 0.0 && has_area).then(|| {
            RollUniforms { feather: 1.0 / half_ppp, ..uniforms }.drawn_into(
                ppp,
                roll_origin,
                roll_size,
                bloom_notes_size(roll_size),
            )
        });
        // Only a roll that casts draws the outline layer that reads it.
        let wants_holdout = sigma > 0.0 && has_area;
        // ...and only a casting roll's cut, which `cut_uniform` zeroes along
        // with a cut of no amount.
        let wants_cut = wants_holdout && cut[0] > 0.0;

        // Split apart so the pane can be borrowed mutably while the pipelines
        // and layouts beside it are still readable.
        let RollResources {
            light_pipeline,
            layout,
            mask_pipeline,
            holdout_pipeline,
            holdout_layout,
            cut_reference_pipeline,
            cut_blend_pipeline,
            cut_blend_layout,
            cut_composite_layout,
            bright_pipeline,
            downsample_pipeline,
            blur_h_pipeline,
            blur_v_pipeline,
            filter_layout,
            bloom_layout,
            sampler,
            panes,
            ..
        } = resources;
        let shared = RollBloomShared { notes_layout: layout, filter_layout, bloom_layout, sampler };
        // A roll's id is its surface (the docked pane, the Render preview), and
        // a closed tab simply stops calling back — so the panes still being
        // prepared are the only evidence of which ones exist. Swept from
        // whichever pane IS preparing, so a lone survivor still clears the
        // others.
        panes.evict_unseen(self.pass_nr);
        let pane = RollPane::get(panes, device, layout, self.pane_id, self.pass_nr);
        if self.instances.len() > pane.capacity {
            pane.capacity = self.instances.len().next_power_of_two();
            pane.instance_buffer =
                create_vertex_buffer::<RollInstance>(device, "roll_notes", pane.capacity);
        }
        pane.count = self.instances.len() as u32;
        if !self.instances.is_empty() {
            queue.write_buffer(&pane.instance_buffer, 0, bytemuck::cast_slice(&self.instances));
        }
        queue.write_buffer(&pane.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        // Held whether or not it is wanted this frame, since a strength dialled
        // to 0 and back is one drag: what is skipped is the work, not the
        // textures. They go when the roll's size changes, which is the one
        // thing that invalidates them.
        if pane.bloom.as_ref().is_some_and(|b| b.size != roll_size) {
            pane.bloom = None;
        }
        if let Some(notes_uniforms) = bloom_pass {
            let bloom =
                pane.bloom.get_or_insert_with(|| RollBloom::new(device, &shared, roll_size));
            bloom.bloom.set_owner_size(queue, self.pane_size * screen_descriptor.pixels_per_point);
            queue.write_buffer(&bloom.notes_uniform, 0, bytemuck::bytes_of(&notes_uniforms));
            queue.write_buffer(
                &bloom.strength_buffer,
                0,
                bytemuck::bytes_of(&BloomUniforms { strength: [self.bloom, 0.0, 0.0, 0.0] }),
            );

            // The notes again at half size, and then the lattice's own chain
            // over them, step for step (see [`BloomChain::run`]).
            {
                let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("roll_bloom_notes"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &bloom.notes_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                // The BODIES alone. The glow is a surround standing behind
                // them rather than light they give off, and it is dark at its
                // default, which blooms to nothing anyway.
                pass.set_pipeline(light_pipeline);
                pass.set_bind_group(0, &bloom.notes_bind_group, &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                pass.draw(0..4, 0..pane.count);
            }
            bloom.bloom.run(
                egui_encoder,
                crate::BloomPipelines {
                    bright: bright_pipeline,
                    downsample: downsample_pipeline,
                    blur_h: blur_h_pipeline,
                    blur_v: blur_v_pipeline,
                },
                "roll",
                None,
            );
        }

        // Kept while the roll casts, and dropped with the shadow, since only
        // the outline layer draws into it and reads it. The glow mask beside
        // it is drawn straight after, reading the holdout just finished.
        if !wants_holdout || pane.holdout.as_ref().is_some_and(|h| h.size != roll_size) {
            pane.holdout = None;
        }
        if wants_holdout {
            let holdout = pane.holdout.get_or_insert_with(|| {
                RollHoldout::new(device, layout, holdout_layout, filter_layout, sampler, roll_size)
            });
            queue.write_buffer(&holdout.notes_uniform, 0, bytemuck::bytes_of(&roll_uniforms));
            // A clipped tail holds out, and darkens, only where `paint` lets it
            // draw.
            holdout.tail = self.clipped_tail.map(|(start, tail_rect)| {
                let clip = egui::epaint::ViewportInPixels::from_points(
                    &tail_rect.intersect(self.rect),
                    ppp,
                    screen_descriptor.size_in_pixels,
                );
                let left = (clip.left_px - viewport.left_px).max(0) as u32;
                let top = (clip.top_px - viewport.top_px).max(0) as u32;
                let width = (clip.width_px.max(0) as u32).min(roll_size[0].saturating_sub(left));
                let height = (clip.height_px.max(0) as u32).min(roll_size[1].saturating_sub(top));
                TailClip {
                    start: start.min(pane.count),
                    scissor: (width > 0 && height > 0).then_some([left, top, width, height]),
                }
            });
            let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("roll_holdout"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &holdout.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(holdout_pipeline);
            pass.set_bind_group(0, &holdout.notes_bind_group, &[]);
            pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
            draw_clipped(&mut pass, pane.count, holdout.tail);
            drop(pass);

            let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("roll_shadow_mask"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &holdout.mask_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                // Cleared to 1, the depth of no glow at all, which no fragment
                // of no strength can beat.
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &holdout.mask_depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(mask_pipeline);
            pass.set_bind_group(0, &holdout.notes_bind_group, &[]);
            pass.set_bind_group(1, &holdout.read_bind_group, &[]);
            pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
            draw_clipped(&mut pass, pane.count, holdout.tail);
            drop(pass);

            // The backdrop cut's targets, kept exactly while the cut is on: a
            // cut dialled to nothing pays for neither them nor the passes, and
            // `paint` lays the plain glow down whenever none are held.
            if !wants_cut {
                holdout.cut = None;
            }
            if wants_cut {
                let RollHoldout { cut: targets, view, mask_view, .. } = holdout;
                let targets = targets.get_or_insert_with(|| {
                    RollCutTargets::new(
                        device,
                        cut_blend_layout,
                        cut_composite_layout,
                        view,
                        mask_view,
                        roll_size,
                    )
                });
                // Cleared to 1, the distance of no cut at all.
                let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("roll_cut_reference"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &targets.reference_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(1.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(cut_reference_pipeline);
                pass.set_bind_group(0, &holdout.notes_bind_group, &[]);
                pass.set_bind_group(1, &holdout.read_bind_group, &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                draw_clipped(&mut pass, pane.count, holdout.tail);
                drop(pass);

                // At a halo blend of 0 the reference is the whole cut, and the
                // composite reads nothing else.
                if cut[3] > 0.0 {
                    let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("roll_cut_blend"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &targets.blend_view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_pipeline(cut_blend_pipeline);
                    pass.set_bind_group(0, &holdout.notes_bind_group, &[]);
                    pass.set_bind_group(1, &targets.blend_read_bind_group, &[]);
                    pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                    draw_clipped(&mut pass, pane.count, holdout.tail);
                }
            }
        }

        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<RollResources>() else {
            return;
        };
        let Some(pane) = resources.panes.get(self.pane_id) else {
            return;
        };
        if pane.count == 0 {
            return;
        }
        // Every outline, then every body over them. An outline reaches into its
        // note's surroundings, and those surroundings are other notes; under
        // all of them it can cover the picture and nothing else. See the head
        // of roll.wgsl.
        //
        // The outlines are the mask `prepare` drew, laid over the roll's rect
        // as the halo is below, inside egui's own clip. `prepare` keeps a
        // holdout, and draws its mask, exactly while the roll casts.
        if let Some(holdout) = &pane.holdout {
            let vp = info.viewport_in_pixels();
            render_pass.set_viewport(
                vp.left_px as f32,
                vp.top_px as f32,
                vp.width_px as f32,
                vp.height_px as f32,
                0.0,
                1.0,
            );
            // With the backdrop cut, through its own composite, which reads
            // the cut's amount and fill out of the pane's uniforms.
            if let Some(cut) = &holdout.cut {
                render_pass.set_pipeline(&resources.cut_composite_pipeline);
                render_pass.set_bind_group(0, &pane.bind_group, &[]);
                render_pass.set_bind_group(1, &cut.composite_bind_group, &[]);
            } else {
                render_pass.set_pipeline(&resources.composite_pipeline);
                render_pass.set_bind_group(0, &holdout.mask_read_bind_group, &[]);
            }
            render_pass.draw(0..4, 0..1);
        }

        // Draw against the WHOLE surface rather than the viewport egui-wgpu
        // helpfully set to the callback rect: the geometry is in screen
        // points, so this shader's clip mapping is egui's own and there is
        // no second rounding of the pane rect into pixels to disagree with.
        // egui-wgpu resets the viewport after a callback, and the SCISSOR it
        // set from the clip rect is left alone — that is what keeps the roll
        // inside its pane.
        render_pass.set_viewport(
            0.0,
            0.0,
            info.screen_size_px[0] as f32,
            info.screen_size_px[1] as f32,
            0.0,
            1.0,
        );
        render_pass.set_bind_group(0, &pane.bind_group, &[]);
        render_pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));

        let default_clip = info.clip_rect_in_pixels();
        let set_clip = |pass: &mut wgpu::RenderPass<'static>, rect: egui::Rect| {
            let clip = egui::epaint::ViewportInPixels::from_points(
                &rect.intersect(info.clip_rect),
                info.pixels_per_point,
                info.screen_size_px,
            );
            if clip.width_px > 0 && clip.height_px > 0 {
                pass.set_scissor_rect(
                    clip.left_px as u32,
                    clip.top_px as u32,
                    clip.width_px as u32,
                    clip.height_px as u32,
                );
                true
            } else {
                false
            }
        };
        render_pass.set_pipeline(&resources.core_pipeline);
        if let Some((start, tail_rect)) =
            self.clipped_tail.map(|(start, rect)| (start.min(pane.count), rect))
        {
            if start > 0 && set_clip(render_pass, info.clip_rect) {
                render_pass.draw(0..4, 0..start);
            }
            if start < pane.count && set_clip(render_pass, tail_rect) {
                render_pass.draw(0..4, start..pane.count);
            }
        } else {
            render_pass.draw(0..4, 0..pane.count);
        }

        // The callback owns the scissor while it paints. Put egui's clip back
        // before the full-region bloom composite and before returning; egui's
        // renderer caches its last clip and may not set it again for the next
        // primitive.
        render_pass.set_scissor_rect(
            default_clip.left_px as u32,
            default_clip.top_px as u32,
            default_clip.width_px as u32,
            default_clip.height_px as u32,
        );

        // The halo over them, from the chain `prepare` ran — light only, and
        // last, so a note's own body is brightened by it the way a lattice
        // node's is. Over the roll's rect rather than the surface: quarter A
        // covers exactly that, and stretching it anywhere else would smear the
        // halo across the pane.
        if self.bloom > 0.0 {
            let Some(bloom) = &pane.bloom else {
                return;
            };
            let vp = info.viewport_in_pixels();
            render_pass.set_viewport(
                vp.left_px as f32,
                vp.top_px as f32,
                vp.width_px as f32,
                vp.height_px as f32,
                0.0,
                1.0,
            );
            render_pass.set_pipeline(&resources.bloom_pipeline);
            render_pass.set_bind_group(0, &bloom.add_bind_group, &[]);
            render_pass.draw(0..4, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_harness::{headless_device, readback, render_to_texture};

    /// A 256x256 test surface at one point per pixel, so a distance in
    /// points is a distance in pixels and the band arithmetic below reads
    /// straight off the instance.
    const SIZE: [u32; 2] = [256, 256];
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    /// The four axis pairs the pane hands this shader, named as
    /// `SpectralOrientation` names them: for the side the now-line sits on, so
    /// `depth_dir` points away from it. Pitch reads low-to-high the conventional
    /// way in each pair, which is why only `depth_dir` differs within one.
    ///
    /// [`TOP`] is what [`draw`] uses, so a test that says nothing about
    /// orientation is drawn pitch across (x) and time down (y).
    const TOP: RollAxes = RollAxes { pitch_dir: [1.0, 0.0], depth_dir: [0.0, 1.0] };
    const BOTTOM: RollAxes = RollAxes { pitch_dir: [1.0, 0.0], depth_dir: [0.0, -1.0] };
    /// Pitch climbs the screen (low at the bottom), time runs along it.
    const LEFT: RollAxes = RollAxes { pitch_dir: [0.0, -1.0], depth_dir: [1.0, 0.0] };
    const RIGHT: RollAxes = RollAxes { pitch_dir: [0.0, -1.0], depth_dir: [-1.0, 0.0] };

    /// A background whose bytes are exact, so "nothing was painted here" is
    /// an equality rather than a tolerance.
    const BG: [u8; 4] = [64, 96, 128, 255];

    fn bg_color() -> wgpu::Color {
        wgpu::Color {
            r: f64::from(BG[0]) / 255.0,
            g: f64::from(BG[1]) / 255.0,
            b: f64::from(BG[2]) / 255.0,
            a: 1.0,
        }
    }

    /// Run the callback for real — `prepare` then `paint` — over `clear`,
    /// and read the frame back as RGBA8. No bloom: the notes alone, which is
    /// what every test but [`the_bloom_adds_light_around_a_note`] is about.
    fn draw(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: Vec<RollInstance>,
        clear: wgpu::Color,
    ) -> Vec<u8> {
        draw_turned(device, queue, instances, TOP, clear)
    }

    /// As [`draw`], with the pane turned whichever way `axes` says.
    fn draw_turned(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: Vec<RollInstance>,
        axes: RollAxes,
        clear: wgpu::Color,
    ) -> Vec<u8> {
        draw_bloomed(device, queue, instances, axes, 0.0, clear)
    }

    /// As [`draw_turned`], with the bloom at `bloom` — the whole callback, so
    /// the chain `prepare` encodes and the halo `paint` lays over the notes are
    /// both in the frame that comes back.
    fn draw_bloomed(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: Vec<RollInstance>,
        axes: RollAxes,
        bloom: f32,
        clear: wgpu::Color,
    ) -> Vec<u8> {
        draw_bloomed_resourced(device, queue, instances, axes, bloom, clear).0
    }

    /// As [`draw_bloomed`], handing back the callback's resources as well —
    /// which is where "no bloom ran" is visible, the frame being the one place
    /// it is not.
    fn draw_bloomed_resourced(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: Vec<RollInstance>,
        axes: RollAxes,
        bloom: f32,
        clear: wgpu::Color,
    ) -> (Vec<u8>, CallbackResources) {
        // The roll's rect is the whole test surface, so a point is a pixel
        // here as it is everywhere else in these tests.
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        let cb = RollCallback {
            point_scale: 1.0,
            rect,
            pane_size: rect.size(),
            instances,
            clipped_tail: None,
            axes,
            shadow: harness_shadow(),
            cut: RollCut::OFF,
            bloom,
            target_format: FORMAT,
            pane_id: 0,
            pass_nr: 0,
        };
        draw_callback(device, queue, cb, clear)
    }

    /// The shadow style [`draw`] and its kin draw under: full darkness, at the
    /// width whose glow reaches exactly [`HARNESS_REACH`] (3σ, σ being half a
    /// width of four-point units).
    fn harness_shadow() -> harmonigraph_scene::ShadowStyle {
        let style = harmonigraph_scene::ShadowStyle {
            width: HARNESS_REACH / 6.0,
            depth: 1.0,
            ..Default::default()
        };
        let reach = crate::shadow::spectral_shadow_reach(style, 1.0);
        assert!((reach - HARNESS_REACH).abs() < 1e-5, "the harness outline reaches {reach}");
        style
    }

    /// How far [`harness_shadow`]'s glow reaches off a note, in points — which
    /// on this surface are pixels: 3σ at a σ of one point, where the profile is
    /// lowered to exactly zero.
    ///
    /// Chosen for the probes. In black, a pixel centre half a point off an edge
    /// reads nearly solid (blue 15 against the background's 128) and one a point
    /// and a half off still plainly dark (87), while every pixel wholly past the
    /// reach reads the background exactly.
    const HARNESS_REACH: f32 = 3.0;

    /// Everything egui does to `cb` before it paints, and submitted: its
    /// `prepare`, which draws the outlines' mask with the holdout. Every harness
    /// goes through this, so none can paint a roll whose mask was never drawn
    /// and read "no shadow" as a pass.
    fn prepare_frame(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &mut CallbackResources,
        screen: &ScreenDescriptor,
        cb: &RollCallback,
    ) {
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(device, queue, screen, &mut encoder, resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
    }

    fn draw_callback(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cb: RollCallback,
        clear: wgpu::Color,
    ) -> (Vec<u8>, CallbackResources) {
        let rect = cb.rect;
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        prepare_frame(device, queue, &mut resources, &screen, &cb);

        let texture = render_to_texture(device, queue, SIZE, FORMAT, clear, |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: 1.0,
                    screen_size_px: SIZE,
                },
                pass,
                &resources,
            );
        });
        (readback(device, queue, &texture, SIZE), resources)
    }

    fn draw_with_clipped_tail(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: Vec<RollInstance>,
        clipped_start: u32,
        clear: wgpu::Color,
    ) -> Vec<u8> {
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        draw_callback(
            device,
            queue,
            RollCallback {
                point_scale: 1.0,
                rect,
                pane_size: rect.size(),
                instances,
                clipped_tail: Some((clipped_start, rect)),
                axes: TOP,
                shadow: harness_shadow(),
                cut: RollCut::OFF,
                bloom: 0.0,
                target_format: FORMAT,
                pane_id: 0,
                pass_nr: 0,
            },
            clear,
        )
        .0
    }

    /// One roll frame with an explicitly selected shadow style. Kept separate
    /// from `draw_bloomed_resourced` so the ordinary shape tests keep their
    /// fixed [`harness_shadow`] fixture.
    fn draw_shadowed(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: Vec<RollInstance>,
        shadow: harmonigraph_scene::ShadowStyle,
        clear: wgpu::Color,
        point_scale: f32,
    ) -> Vec<u8> {
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        let cb = RollCallback {
            point_scale,
            rect,
            pane_size: rect.size(),
            instances,
            clipped_tail: None,
            axes: TOP,
            shadow,
            cut: RollCut::OFF,
            bloom: 0.0,
            target_format: FORMAT,
            pane_id: 0,
            pass_nr: 0,
        };
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        prepare_frame(device, queue, &mut resources, &screen, &cb);
        let texture = render_to_texture(device, queue, SIZE, FORMAT, clear, |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: 1.0,
                    screen_size_px: SIZE,
                },
                pass,
                &resources,
            );
        });
        readback(device, queue, &texture, SIZE)
    }

    fn pixel(frame: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * SIZE[0] + x) * 4) as usize;
        [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
    }

    /// 8-bit color comparison with room for the shader's arithmetic.
    fn near(got: [u8; 4], want: [u8; 4]) -> bool {
        got.iter().zip(want).all(|(&a, b)| a.abs_diff(b) <= 3)
    }

    fn shadowed(got: [u8; 4]) -> bool {
        got[2] < BG[2]
    }

    /// A straight note centered in the frame: 24 points thick, 120 long, in
    /// the harness's black outline (its shadow style reaches three points), at
    /// full opacity, and square at both ends. Wide enough that a sample lands
    /// well inside the outline, and hard-edged so where it ends is a place
    /// rather than a slope — the two fades have
    /// [`pieces_tile_their_note_and_a_fade_runs_along_it`] and
    /// [`a_lead_fade_takes_the_ribbon_out_toward_its_tip`] to themselves.
    fn centered_note() -> RollInstance {
        RollInstance {
            center: [128.0, 128.0],
            half_extent: [12.0, 60.0],
            shear: 0.0,
            lead: 0.0,
            lead_fade: 0.0,
            lead_alpha: 0.0,
            // The ordinary answer, and what a caller with nowhere to protect
            // gives: the cap at the note's own end reaches as far as every
            // other edge's outline, and it is the LEAD over it that decides
            // whether it shows.
            cap_reach: HARNESS_REACH,
            core: [255, 0, 0, 255],
            aura: [0, 0, 0, 255],
            span: RollInstance::WHOLE,
            ramp: [0.0, 0.0],
            fade: [1.0, 1.0],
            taper_depth: [0.0; 4],
            taper: RollInstance::UNTAPERED,
            tremolo: [0.0; 2],
        }
    }

    /// [`centered_note`] with a lead of `lead` points at its leading tip, `fade`
    /// of that spent fading, and `alpha` of it still standing. The lead is
    /// carved OUT of the note's own length rather than added to it, so every
    /// coordinate in the tests below stays where [`centered_note`] put it —
    /// which is what the pane does too, in reverse: it grows the box first and
    /// tells this crate where the note inside it ends.
    fn led_note(lead: f32, fade: f32, alpha: f32) -> RollInstance {
        RollInstance { lead, lead_fade: fade, lead_alpha: alpha, ..centered_note() }
    }

    #[test]
    fn uniforms_match_the_bound_shader_layout() {
        crate::uniforms::layout::check_binding::<RollUniforms>(&crate::roll_source(), 0, 0);
    }

    #[test]
    fn tremolo_marks_leave_the_edges_lead_and_coverage_continuous() {
        let Some((device, queue)) = headless_device() else { return };
        for axes in [TOP, BOTTOM, LEFT, RIGHT] {
            for half_width in [3.0, 12.0] {
                let note = RollInstance {
                    half_extent: [half_width, 60.0],
                    core: [128, 64, 96, 192],
                    ..led_note(20.0, 0.0, 1.0)
                };
                let plain =
                    draw_turned(&device, &queue, vec![note], axes, wgpu::Color::TRANSPARENT);
                let marked = draw_turned(
                    &device,
                    &queue,
                    vec![RollInstance { tremolo: [0.0, 1.0], ..note }],
                    axes,
                    wgpu::Color::TRANSPARENT,
                );
                let mut changed = 0;
                for y in 0..SIZE[1] {
                    for x in 0..SIZE[0] {
                        let a = pixel(&plain, x, y);
                        let b = pixel(&marked, x, y);
                        assert_eq!(a[3], b[3], "ornament changed coverage at {x},{y}");
                        if a == b {
                            continue;
                        }
                        changed += 1;
                        let delta = [x as f32 + 0.5 - 128.0, y as f32 + 0.5 - 128.0];
                        let pitch = delta[0] * axes.pitch_dir[0] + delta[1] * axes.pitch_dir[1];
                        let depth = delta[0] * axes.depth_dir[0] + delta[1] * axes.depth_dir[1];
                        assert!(pitch.abs() < half_width - 0.5, "mark crossed the untouched rail");
                        assert!(depth >= -40.0, "mark entered the lead");
                        assert!(b[0] >= 32 && b[0] <= a[0], "mark lost the solid fill: {b:?}");
                    }
                }
                assert!(changed > 30, "fixture must reach visible tremolo marks");
                assert!(changed < 600, "ornament filled too much of the ribbon");
            }
        }
    }

    /// A led note's cap follows the note's own center LINE, so a sheared box
    /// gets its cap where the ribbon actually ends rather than where the box's
    /// center happens to sit.
    ///
    /// This is the one path `box_distance_trimmed` exists for that nothing else
    /// runs: the pane can never reach it, because `RollNote::segments` closes a
    /// note at the pitch of its final bend, so the only segment that can carry
    /// a lead is flat. The shear terms cancel identically — trimming slides the
    /// box's center to `(slope * trim / 2, trim / 2)`, and both halves of that
    /// cancel in its distance — and an implementation that simply forgot to
    /// shear the trimmed box would pass every other test in this file.
    ///
    /// Read across the note's own end, twelve points either side of the box's
    /// center: at a shear of 0.6 the end has drifted that far, so the cap is on
    /// one side and nothing is on the other.
    #[test]
    fn a_sheared_notes_cap_follows_its_center_line() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // The band's CENTER rather than two samples of it. The cap reaches three
        // points and the note is twenty-four wide, so a cap displaced by a wrong
        // shear still covers most of the pixels a correct one does — picking
        // two and asserting ink at one and none at the other passes for a
        // mis-shear as readily as for this one. Where the run is centered is
        // the claim itself.
        let center = |shear: f32| {
            let note = RollInstance { shear, ..led_note(40.0, 0.0, 0.0) };
            let frame = draw(&device, &queue, vec![note], bg_color());
            // Row 106 is a point and a half past the note's own end (y = 108), so the
            // cap is the only thing painting in it: the lead is spent, and the
            // wrap that would wrap the box goes with it.
            // Read on BLUE. The background is `[64, 96, 128]`, so its own red
            // channel is already darker than a threshold picked for black —
            // the one channel the two are far apart on is the one to ask.
            let dark: Vec<u32> =
                (0..SIZE[0]).filter(|&x| shadowed(pixel(&frame, x, 106))).collect();
            assert!(!dark.is_empty(), "the cap painted nothing at all at a shear of {shear}");
            // Between the first and last dark pixels' centres.
            f32::from(*dark.first().unwrap() as u16 + *dark.last().unwrap() as u16) * 0.5 + 0.5
        };
        // The cap is the note's blurred END, so its run is centred on that end
        // rather than on the line carried on past it: on where the line stands
        // at the note's own end, 20 points along depth from the box's own
        // centre, having drifted `shear * 20` along pitch. A trimmed box whose
        // centre is not slid along the shear stands its end elsewhere along
        // pitch and clips the run to its own reach there, which moves that
        // middle five and a half points at a shear of 0.6.
        for shear in [0.0f32, 0.6, -0.6] {
            let want = 128.0 - shear * 20.0;
            let got = center(shear);
            assert!(
                (got - want).abs() < 1.5,
                "at a shear of {shear} the cap is centred on {got}, not {want} — it is \
                 standing where the BOX's centre is rather than where the note's line ends",
            );
        }
    }

    /// The cap is UNIONED with the wrap, not added to it.
    ///
    /// The two shapes share three of their sides: along the note's own flanks
    /// the box the cap measures and the box the wrap measures are the same
    /// distance away, so both are looking at the same ink. Added, that overlap
    /// comes out darker than the outline's own color is.
    ///
    /// Read behind a spent lead, the one the pane keeps under a cap, beside the
    /// note's flank, where the wrap stands at full strength: a note with a cap
    /// there has to draw exactly the outline the same note draws with none.
    ///
    /// A HALF-dark outline is what makes this readable, and the reason is worth
    /// the line: a full one is the degenerate case here. Beside the flank the
    /// wrap is nearly solid, so doubling it doubles a darkness the target
    /// clamps — the sum and the union land on the same byte, and every other
    /// cap test in this file would pass over a `+`. At half the two separate.
    #[test]
    fn the_cap_is_unioned_with_the_wrap_and_not_added_to_it() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // Half a point off the note's flank (x = 140), well inside its length.
        let at = |cap_reach: f32| {
            let note = RollInstance { aura: [0, 0, 0, 128], cap_reach, ..led_note(40.0, 0.0, 0.0) };
            pixel(&draw(&device, &queue, vec![note], bg_color()), 140, 148)
        };
        let (capped, wrap_only) = (at(HARNESS_REACH), at(0.0));
        assert!(!near(wrap_only, BG), "the probe missed the wrap: {wrap_only:?}");
        assert!(
            near(capped, wrap_only),
            "the flank with a cap {capped:?} differs from the wrap alone {wrap_only:?} — the \
             wrap and the cap are being summed rather than unioned",
        );
    }

    /// One note's shadow never darkens under another note's body, however
    /// translucent that body is (#1293): every shadow is held out wherever
    /// any body covers.
    ///
    /// Two half-opaque ribbons overlapping by four points on a bright
    /// background. The probe is inside the upper ribbon a point and a half past
    /// the lower one's edge, well within the harness shadow's three-point reach,
    /// so the lower ribbon's shadow reaches it and only the holdout keeps it
    /// off. It has to read exactly as it does with that shadow switched off.
    #[test]
    fn a_shadow_never_darkens_under_another_ribbons_translucent_body() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let bright = wgpu::Color { r: 0.9, g: 0.9, b: 0.9, a: 1.0 };
        let at = |lower_outline: f32| {
            let lower = RollInstance {
                center: [118.0, 128.0],
                half_extent: [10.0, 60.0],
                aura: [0, 0, 0, (lower_outline * 255.0) as u8],
                fade: [0.5, 0.5],
                ..centered_note()
            };
            let upper = RollInstance {
                center: [136.0, 128.0],
                half_extent: [12.0, 60.0],
                core: [0, 0, 255, 255],
                fade: [0.5, 0.5],
                ..centered_note()
            };
            pixel(&draw(&device, &queue, vec![lower, upper], bright), 129, 128)
        };
        let (shadowed, unshadowed) = (at(1.0), at(0.0));
        assert_eq!(
            shadowed, unshadowed,
            "the lower ribbon's shadow darkened the upper ribbon's body",
        );
    }

    /// Where two notes' shadows meet, the picture is as dark as the darker of
    /// the two and no darker: every outline goes into one mask under `max`,
    /// rather than each darkening what the other already had.
    ///
    /// Two notes three points apart across pitch, the probe in the middle of
    /// the gap, a point and a half off each: where either shadow alone reads
    /// solidly dark.
    #[test]
    fn two_notes_shadows_merge_rather_than_stack() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let left = RollInstance { center: [114.0, 128.0], ..centered_note() };
        let right = RollInstance { center: [141.0, 128.0], ..centered_note() };
        let at = |notes| pixel(&draw(&device, &queue, notes, bg_color()), 127, 128);
        let (left_alone, right_alone) = (at(vec![left]), at(vec![right]));
        assert!(shadowed(left_alone), "the probe is past the left note's shadow: {left_alone:?}");
        assert!(
            shadowed(right_alone),
            "the probe is past the right note's shadow: {right_alone:?}"
        );
        let darker: [u8; 4] = std::array::from_fn(|c| left_alone[c].min(right_alone[c]));
        let both = at(vec![left, right]);
        assert!(
            near(both, darker),
            "between the two notes the picture reads {both:?}, not the darker shadow {darker:?} — \
             the shadows are stacking",
        );
    }

    /// The glow paints in its own color: over a background it can only lift a
    /// channel toward the aura where a black shadow could only lower it, so a
    /// green glow over the harness's blue-grey raises green.
    #[test]
    fn the_glow_paints_its_own_color() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let note = RollInstance { aura: [0, 255, 0, 255], ..centered_note() };
        // Half a point off the note's flank, where the harness profile covers
        // nearly nine tenths: green near 236 against the background's 96.
        let got = pixel(&draw(&device, &queue, vec![note], bg_color()), 140, 128);
        assert!(got[1] > BG[1] + 40, "the glow did not lift green toward its color: {got:?}");
        assert!(got[0] < BG[0] && got[2] < BG[2], "the glow mixed in another color: {got:?}");
    }

    /// Where two glows of different colors meet, the picture is the STRONGER
    /// one's, color and all — not a channel-by-channel blend of the two, which
    /// would draw a hue neither note has.
    ///
    /// A red glow and a blue one standing six points apart at a σ of four, the
    /// probe a point nearer the red note: both reach it strongly (the blue
    /// alone lifts blue far above the background there), and the red is the
    /// stronger, so with both drawn the probe must read as the red glow alone.
    #[test]
    fn where_two_glows_meet_the_stronger_keeps_its_color() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let style =
            harmonigraph_scene::ShadowStyle { width: 2.0, depth: 1.0, ..Default::default() };
        // Edges at x = 126 and x = 132.
        let red = RollInstance { aura: [255, 0, 0, 255], ..centered_note() };
        let red = RollInstance { center: [114.0, 128.0], ..red };
        let blue = RollInstance { center: [144.0, 128.0], aura: [0, 0, 255, 255], ..red };
        let at =
            |notes| pixel(&draw_shadowed(&device, &queue, notes, style, bg_color(), 1.0), 128, 128);
        let (red_alone, blue_alone) = (at(vec![red]), at(vec![blue]));
        assert!(blue_alone[2] > BG[2] + 60, "the blue glow misses the probe: {blue_alone:?}");
        let both = at(vec![red, blue]);
        assert!(
            near(both, red_alone),
            "between a red glow and a weaker blue one the picture reads {both:?}, \
             not the red glow alone {red_alone:?}",
        );
    }

    /// A hairline ribbon is held by as strong a surround as a wide one at the
    /// same distance off its edge. The glow is read off the distance to the
    /// edge, which a ribbon's width does not change; a blur of the ribbon's
    /// silhouette, which is what the roll drew before, covers less the thinner
    /// the ribbon is under its σ — at this σ of four, a two-point ribbon got
    /// under half the surround a wide one did, a point and a half off.
    #[test]
    fn a_hairline_ribbons_glow_is_as_strong_as_a_wide_ones() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let style =
            harmonigraph_scene::ShadowStyle { width: 2.0, depth: 1.0, ..Default::default() };
        let draw = |note| draw_shadowed(&device, &queue, vec![note], style, bg_color(), 1.0);
        // Edges at x = 127 and 129, probed at 130.5.
        let thin = RollInstance { half_extent: [1.0, 60.0], ..centered_note() };
        // Edge at x = 140, probed at 141.5.
        let wide = centered_note();
        let (thin, wide) = (pixel(&draw(thin), 130, 128), pixel(&draw(wide), 141, 128));
        assert!(shadowed(wide), "the probe is past the wide ribbon's glow: {wide:?}");
        assert!(
            near(thin, wide),
            "the hairline's surround {thin:?} is not the wide one's {wide:?}"
        );
    }

    /// A roll whose rect stands off the surface's corner, as the docked pane's
    /// does beside the lattice, darkens the same pixels as one at the corner:
    /// the mask and the holdout are over the roll's rect alone, so a texel read
    /// or a scissor measured from the wrong origin moves the shadows by the
    /// rect's offset. Its clipped tail cuts the tail note's shadow off where it
    /// cuts the note.
    #[test]
    fn shadows_stay_on_their_notes_in_a_rect_off_the_surface_corner() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let left = RollInstance { center: [114.0, 128.0], ..centered_note() };
        let right = RollInstance { center: [141.0, 128.0], ..centered_note() };
        let at_corner = draw(&device, &queue, vec![left, right], bg_color());
        let rect = egui::Rect::from_min_max(egui::pos2(40.0, 24.0), egui::pos2(220.0, 232.0));
        // Ends inside the right note, so its far flank's shadow is past it.
        let tail = egui::Rect::from_min_max(rect.min, egui::pos2(150.0, rect.max.y));
        let (offset, _) = draw_callback(
            &device,
            &queue,
            RollCallback {
                point_scale: 1.0,
                rect,
                pane_size: rect.size(),
                instances: vec![left, right],
                clipped_tail: Some((1, tail)),
                axes: TOP,
                shadow: harness_shadow(),
                cut: RollCut::OFF,
                bloom: 0.0,
                target_format: FORMAT,
                pane_id: 0,
                pass_nr: 0,
            },
            bg_color(),
        );
        // A point and a half off the left note's outer flank, and the gap.
        for (x, y) in [(100, 128), (127, 128)] {
            let want = pixel(&at_corner, x, y);
            assert!(shadowed(want), "the probe at ({x}, {y}) is past every shadow: {want:?}");
            let got = pixel(&offset, x, y);
            assert_eq!(got, want, "at ({x}, {y}) the offset roll reads {got:?}, not {want:?}");
        }
        // A point and a half off the right note's far flank, past the tail.
        assert!(shadowed(pixel(&at_corner, 154, 128)), "the probe is past the right note's shadow");
        let cut = pixel(&offset, 154, 128);
        assert!(near(cut, BG), "the tail's shadow ran past its clip: {cut:?}");
    }

    #[test]
    fn baked_roll_shader_validates() {
        let source = crate::with_common(ROLL_SRC);
        let module = naga::front::wgsl::parse_str(&source)
            .map_err(|e| e.emit_to_string(&source))
            .expect("roll.wgsl must parse");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("roll.wgsl must validate");
        for required in ROLL_ENTRY_POINTS {
            assert!(
                module.entry_points.iter().any(|ep| ep.name == *required),
                "missing entry point `{required}`"
            );
        }
    }

    /// The vertex-layout <-> shader-input contract (attribute locations,
    /// formats, strides), which neither the naga check (shader only) nor the
    /// type system (Rust only) covers — a mismatch otherwise panics at first
    /// paint inside a host.
    #[test]
    fn the_pipeline_builds_against_a_headless_device() {
        let Some((device, _queue)) = headless_device() else {
            return;
        };
        let _resources = RollResources::new(&device, FORMAT);
    }

    /// A shadow style switched off at either end — no width or no darkness —
    /// uploads no glow and drops the mask a casting frame left: the holdout and
    /// the mask beside it are kept only while the roll casts, and `paint` lays
    /// a mask over the picture whenever one is held, so one surviving the
    /// switch would paint the last cast frame's glow on.
    #[test]
    fn switching_the_shadow_off_drops_the_glow_mask() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        let callback = |shadow| RollCallback {
            point_scale: 1.0,
            rect,
            pane_size: rect.size(),
            instances: vec![centered_note()],
            clipped_tail: None,
            axes: TOP,
            shadow,
            cut: RollCut::OFF,
            bloom: 0.0,
            target_format: FORMAT,
            pane_id: 0,
            pass_nr: 0,
        };
        let holds = |resources: &CallbackResources| {
            resources
                .get::<RollResources>()
                .and_then(|r| r.panes.get(0))
                .is_some_and(|pane| pane.holdout.is_some())
        };
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        for off in [
            harmonigraph_scene::ShadowStyle { width: 0.0, depth: 1.0, ..Default::default() },
            harmonigraph_scene::ShadowStyle { width: 1.0, depth: 0.0, ..Default::default() },
        ] {
            assert_eq!(shadow_uniform(off, 1.0), [0.0; 4], "{off:?} uploaded a glow");
            let mut resources = CallbackResources::default();
            prepare_frame(&device, &queue, &mut resources, &screen, &callback(harness_shadow()));
            assert!(holds(&resources), "a casting frame kept no glow mask");
            prepare_frame(&device, &queue, &mut resources, &screen, &callback(off));
            assert!(!holds(&resources), "{off:?} kept the last cast frame's glow mask");
        }
    }

    /// The backdrop cut at the look's own defaults: whole at the edge, fading
    /// over four σ along a smoothstep, cut to black, halo blend at R8's pick.
    const CUT: RollCut = RollCut { amount: 1.0, reach: 4.0, curve: 1.0, fill: 0.0, blend: 0.5 };

    /// A σ of four points (a Shadow width of 2), at full darkness: wide enough
    /// that the cut's fade, sixteen points at [`CUT`]'s reach, spans pixels
    /// enough to read a profile off.
    fn wide_shadow() -> harmonigraph_scene::ShadowStyle {
        harmonigraph_scene::ShadowStyle { width: 2.0, depth: 1.0, ..Default::default() }
    }

    /// One roll frame under `cut`, over `clear`, with its resources.
    fn draw_cut(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: Vec<RollInstance>,
        cut: RollCut,
        clear: wgpu::Color,
    ) -> (Vec<u8>, CallbackResources) {
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        let cb = RollCallback {
            point_scale: 1.0,
            rect,
            pane_size: rect.size(),
            instances,
            clipped_tail: None,
            axes: TOP,
            shadow: wide_shadow(),
            cut,
            bloom: 0.0,
            target_format: FORMAT,
            pane_id: 0,
            pass_nr: 0,
        };
        draw_callback(device, queue, cb, clear)
    }

    fn grey(level: f64) -> wgpu::Color {
        wgpu::Color { r: level, g: level, b: level, a: 1.0 }
    }

    /// A cut of no amount draws exactly the plain glow, whatever its other
    /// four dials say: the reach, curve, fill and blend shape a cut, and with
    /// none to shape they must leave nothing behind — not a filled ring past
    /// the glow's reach, not a crease, not a byte.
    #[test]
    fn a_backdrop_cut_of_nothing_draws_the_plain_glow() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let notes = vec![
            RollInstance { center: [106.5, 128.0], aura: [40, 20, 60, 255], ..centered_note() },
            RollInstance { center: [150.5, 128.0], aura: [20, 60, 40, 255], ..centered_note() },
        ];
        let bright = grey(0.9);
        let plain = draw_cut(&device, &queue, notes.clone(), RollCut::OFF, bright).0;
        let idle = RollCut { amount: 0.0, reach: 8.0, curve: 0.0, fill: 1.0, blend: 1.0 };
        let (drawn, resources) = draw_cut(&device, &queue, notes, idle, bright);
        let pane = resources.get::<RollResources>().and_then(|r| r.panes.get(0));
        assert!(
            pane.is_some_and(|p| p.holdout.as_ref().is_some_and(|h| h.cut.is_none())),
            "a cut of nothing kept the cut's targets"
        );
        assert!(drawn == plain, "a cut of nothing changed the picture");
    }

    /// The cut takes the backdrop away round a note faster than the glow lays
    /// its color on, so over a bright backdrop the surround a σ out is far
    /// darker than the plain glow leaves it — the glow alone is about half
    /// strength there and lets the other half through. Over black there is
    /// nothing to take away, and the picture is the plain glow's.
    #[test]
    fn the_cut_darkens_a_bright_backdrop_and_leaves_black_alone() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let note = RollInstance { aura: [20, 10, 30, 255], ..centered_note() };
        // The note's flank is at x = 140; this pixel's centre is 4.5 points,
        // about a σ, out: the glow covers about half, the cut four fifths.
        let at = |cut, clear| pixel(&draw_cut(&device, &queue, vec![note], cut, clear).0, 144, 128);
        let (glow, cut) = (at(RollCut::OFF, grey(0.9)), at(CUT, grey(0.9)));
        assert!(
            cut[1] + 40 < glow[1],
            "over a bright backdrop the cut surround reads {cut:?}, the plain glow {glow:?}",
        );
        let black = wgpu::Color::BLACK;
        let plain = draw_cut(&device, &queue, vec![note], RollCut::OFF, black).0;
        let drawn = draw_cut(&device, &queue, vec![note], CUT, black).0;
        assert!(drawn == plain, "over black the cut changed the picture");
    }

    /// Where two notes' cuts meet, the strongest-wins cut FOLDS along the line
    /// between them: the backdrop brightens at full slope right up to the
    /// bisector and turns there, a crease. The halo blend rounds it, so the
    /// brightening eases off into the bisector instead.
    ///
    /// Two notes twenty points apart, the bisector on a pixel centre (x =
    /// 128) ten points from each, well inside each cut's sixteen and past
    /// the glow's reach but for a trace. Read as how much the last pixel into
    /// the bisector rises against the pixel two further out: at a fold all
    /// of it (the smoothstep's own easing takes a few percent), under the
    /// blend at the default about a third.
    #[test]
    fn the_halo_blend_takes_the_crease_out_between_two_notes() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let notes = vec![
            RollInstance { center: [106.5, 128.0], ..centered_note() },
            RollInstance { center: [150.5, 128.0], ..centered_note() },
        ];
        let profile = |blend| {
            let frame =
                draw_cut(&device, &queue, notes.clone(), RollCut { blend, ..CUT }, grey(1.0)).0;
            let at = |x| f32::from(pixel(&frame, x, 128)[1]);
            ((at(128) - at(129)) / (at(130) - at(131)), [128, 129, 130, 131].map(at))
        };
        let (fold, folded) = profile(0.0);
        assert!(fold > 0.8, "with no blend the cut does not fold at the bisector: {folded:?}");
        let (eased, blended) = profile(CUT.blend);
        assert!(eased < 0.5, "the halo blend left the fold standing: {blended:?}");
    }

    /// The cut's targets are kept exactly while it can paint: a cut dialled to
    /// nothing, or a shadow switched off, drops them, as the glow's mask is
    /// dropped with the shadow ([`switching_the_shadow_off_drops_the_glow_mask`]).
    #[test]
    fn switching_the_cut_or_the_shadow_off_drops_the_cut_targets() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        let callback = |shadow, cut| RollCallback {
            point_scale: 1.0,
            rect,
            pane_size: rect.size(),
            instances: vec![centered_note()],
            clipped_tail: None,
            axes: TOP,
            shadow,
            cut,
            bloom: 0.0,
            target_format: FORMAT,
            pane_id: 0,
            pass_nr: 0,
        };
        let holds = |resources: &CallbackResources| {
            resources
                .get::<RollResources>()
                .and_then(|r| r.panes.get(0))
                .and_then(|pane| pane.holdout.as_ref())
                .is_some_and(|holdout| holdout.cut.is_some())
        };
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let dark = harmonigraph_scene::ShadowStyle { depth: 0.0, ..wide_shadow() };
        for (shadow, cut) in [(wide_shadow(), RollCut { amount: 0.0, ..CUT }), (dark, CUT)] {
            let mut resources = CallbackResources::default();
            prepare_frame(&device, &queue, &mut resources, &screen, &callback(wide_shadow(), CUT));
            assert!(holds(&resources), "a cutting frame kept no cut targets");
            prepare_frame(&device, &queue, &mut resources, &screen, &callback(shadow, cut));
            assert!(
                !holds(&resources),
                "{shadow:?} with {cut:?} kept the last frame's cut targets"
            );
        }
    }

    #[test]
    fn enlarged_preview_roll_shadows_extend_beyond_maximum_dial_reach() {
        use harmonigraph_scene::{ShadowStyle, SPECTRAL_SHADOW_MAX};
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let style = ShadowStyle { width: SPECTRAL_SHADOW_MAX, spread: 0.2, depth: 1.0 };
        let frames = [1.0, 2.0].map(|point_scale| {
            let mut note = centered_note();
            note.center = [32.0, 32.0];
            note.half_extent = [5.0, 12.0];
            note.cap_reach = crate::spectral_shadow_reach(style, point_scale);
            draw_shadowed(&device, &queue, vec![note], style, wgpu::Color::WHITE, point_scale)
        });
        // The caster starts at x=27. This sample is beyond the dial's
        // maximum support and spread, but the 2x preview must reach it.
        let at = ((32 * SIZE[0] + 3) * 4) as usize;
        assert_eq!(&frames[0][at..at + 3], [255; 3]);
        assert!(frames[1][at] < 253, "enlarged shadow was capped");
    }

    #[test]
    fn roll_geometry_shadows_hold_at_editor_and_export_scales() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        for (ppp, width) in [1.0f32, 1.5, 2.0, 4.0].into_iter().flat_map(|ppp| {
            [0.5, harmonigraph_scene::SPECTRAL_SHADOW_MAX].map(|width| (ppp, width))
        }) {
            // Probe inside the support. The wide reading still sits beyond
            // the old eight-point limit at every pixel scale.
            let shadow_x = if width > 1.0 { 17.0 } else { 25.0 };
            let clear = if width > 1.0 { [0.0, 0.0] } else { [12.0, 12.0] };
            let physical = (64.0 * ppp).round() as u32;
            let size = [physical.div_ceil(64) * 64, physical];
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(64.0, 64.0));
            let shadow =
                harmonigraph_scene::ShadowStyle { width, depth: 1.0, ..Default::default() };
            let reach = crate::shadow::spectral_shadow_reach(shadow, 1.0);
            let note = RollInstance {
                center: [32.0, 32.0],
                half_extent: [5.0, 12.0],
                shear: 0.0,
                lead: 0.0,
                lead_fade: 0.0,
                lead_alpha: 0.0,
                cap_reach: reach,
                core: [255, 0, 0, 255],
                aura: [0, 0, 0, 255],
                span: RollInstance::WHOLE,
                ramp: [0.0, 0.0],
                fade: [1.0, 1.0],
                taper_depth: [0.0; 4],
                taper: RollInstance::UNTAPERED,
                tremolo: [0.0; 2],
            };
            let cb = RollCallback {
                point_scale: 1.0,
                rect,
                pane_size: rect.size(),
                instances: vec![note],
                clipped_tail: None,
                axes: TOP,
                shadow,
                cut: RollCut::OFF,
                bloom: 0.0,
                target_format: FORMAT,
                pane_id: 0,
                pass_nr: 0,
            };
            let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: ppp };
            let mut resources = CallbackResources::default();
            prepare_frame(&device, &queue, &mut resources, &screen, &cb);
            let texture = render_to_texture(
                &device,
                &queue,
                size,
                FORMAT,
                wgpu::Color { r: 0.5, g: 0.5, b: 0.5, a: 1.0 },
                |pass| {
                    cb.paint(
                        egui::PaintCallbackInfo {
                            viewport: rect,
                            clip_rect: rect,
                            pixels_per_point: ppp,
                            screen_size_px: size,
                        },
                        pass,
                        &resources,
                    );
                },
            );
            let frame = readback(&device, &queue, &texture, size);
            let pixel = |x: f32, y: f32| {
                let i = ((((y * ppp).floor() as u32) * size[0] + (x * ppp).floor() as u32) * 4)
                    as usize;
                [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
            };
            // The widest fixture probes beyond the old 8-point support.
            let shadow_pixel = pixel(shadow_x, 32.0);
            assert!(
                shadow_pixel[0] < 118 && shadow_pixel[1] == shadow_pixel[0],
                "at {ppp} ppp left no black under-body shadow: {shadow_pixel:?}",
            );
            assert_eq!(pixel(32.0, 32.0), [255, 0, 0, 255], "the body covers its shadow");
            assert_eq!(
                pixel(clear[0], clear[1]),
                [128, 128, 128, 255],
                "the shadow reached beyond its reach"
            );
        }
    }

    /// A note faded by its Opacity reading keeps its own color over any
    /// spectral energy, white included, and its surround fades with it in the
    /// same proportion.
    ///
    /// The fade is the only way the pane draws a note at less than full
    /// opacity: the body's color is opaque, and the instance's `fade` takes
    /// the body and its outline out together, so a note faded to nothing
    /// leaves no dark silhouette behind.
    ///
    /// The outline standing outside is the flood invariant, and the reason it
    /// is read off a distance rather than drawn as a stroke of the note's path:
    /// a centered stroke grows inward exactly as much as outward, and on a
    /// ribbon a few points thick the two long edges meet in the middle and
    /// paint the interior over. Coverage taken at a positive distance cannot
    /// reach inside a box whose interior is at negative distance.
    #[test]
    fn a_faded_note_keeps_its_color_over_bright_heatmaps_and_its_surround_fades_with_it() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // An opaque purple at an Opacity reading of 0.8.
        let note = RollInstance { core: [128, 64, 192, 255], fade: [0.8, 0.8], ..centered_note() };
        for (background, expected) in [
            (wgpu::Color::BLACK, [102, 51, 154, 255]),
            (bg_color(), [115, 70, 180, 255]),
            (wgpu::Color::WHITE, [153, 102, 205, 255]),
        ] {
            let frame = draw(&device, &queue, vec![note], background);
            for x in [128, 138] {
                assert!(
                    near(pixel(&frame, x, 128), expected),
                    "faded flat fill at x={x}: {:?}, expected {expected:?}",
                    pixel(&frame, x, 128)
                );
            }
        }
        let frame = draw(&device, &queue, vec![note], bg_color());
        // The note's edge is at x = 140, and the harness shadow reaches three
        // points past it.
        let at = |x: u32| pixel(&frame, x, 128);
        let opaque = draw(&device, &queue, vec![centered_note()], bg_color());
        for x in [140, 141] {
            // Read on blue, the channel the background and black are far apart
            // on: the faded surround takes 0.8 of the darkening the opaque
            // note's does.
            let full = f32::from(BG[2]) - f32::from(pixel(&opaque, x, 128)[2]);
            let faded = f32::from(BG[2]) - f32::from(at(x)[2]);
            assert!(full > 8.0, "no outline standing against the opaque note at {x}");
            assert!(
                (faded - 0.8 * full).abs() <= 2.0,
                "the faded note's surround darkens by {faded} at {x}, not 0.8 of {full}",
            );
        }
        assert!(near(at(143), BG), "the outline reaches further than it should: {:?}", at(143));
    }

    /// The outline wraps EVERY side of the note — its ends as much as its
    /// flanks — and turns the corner between them. The note's own body is
    /// untouched either way: it is the shape, and the outline is what is drawn
    /// around it.
    ///
    /// What the corner's falloff looks like is the blur's own, the product of
    /// its two edges' profiles; nothing here chooses it, so only its presence
    /// is asked.
    #[test]
    fn the_outline_wraps_every_side_of_a_note() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // The note spans +-12 across pitch (x) and +-60 along time (y), so its
        // corner is at (140, 188) and the harness blur reaches 3 past it.
        let frame = draw(&device, &queue, vec![centered_note()], bg_color());
        let at = |x: u32, y: u32| pixel(&frame, x, y);
        const RED: [u8; 4] = [255, 0, 0, 255];
        assert!(near(at(138, 128), RED), "the note's body went missing: {:?}", at(138, 128));
        assert!(shadowed(at(141, 128)), "no outline along the flank: {:?}", at(141, 128));
        assert!(near(at(128, 186), RED), "the note's body was cut at its end: {:?}", at(128, 186));
        assert!(shadowed(at(128, 189)), "no outline across the end: {:?}", at(128, 189));
        assert!(near(at(128, 191), BG), "the outline runs past its reach: {:?}", at(128, 191));
        // The pixel centre diagonally off the corner, 0.7 points from it.
        assert!(shadowed(at(140, 188)), "the corner is missing: {:?}", at(140, 188));
    }

    /// The outline runs the FULL length of the note it wraps, corner to corner,
    /// at its outer edge as much as against the note.
    ///
    /// It is coverage of the box distance, so past the note's end it curves
    /// with the field: the flank's outer edge holds its distance right up to
    /// where the note's own ink stops and only then turns. An outline that
    /// pulled away early would read as the note tapering off before it ends.
    #[test]
    fn the_outline_runs_the_whole_length_of_its_note() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let frame = draw(&device, &queue, vec![centered_note()], bg_color());
        let dark = |x: u32, y: u32| shadowed(pixel(&frame, x, y));
        // The two columns the harness blur darkens solidly off the flank, half
        // a point and a point and a half out from the note's own edge (x =
        // 140) — the outer one being where the falloff round the corner bites
        // first.
        for x in [140, 141] {
            assert!(dark(x, 128), "the outline is missing at the note's middle (x = {x})");
            // Row 187 is the note's last full row along time; the outline is
            // still at its full stand-off there.
            assert!(dark(x, 187), "the outline pulls away before the note ends (x = {x})");
        }
        // And it keeps going past the end, around the corner, rather than being
        // cut there: the flank and the cap are one shape.
        assert!(dark(140, 189), "the outline is cut at the note's end");
    }

    /// A leading fade takes the ribbon out gradually toward its tip, and takes
    /// the outline's cap with it: the tip dissolves rather than ending, and
    /// nothing is left standing around where it went.
    ///
    /// Both layers is the half that a body-only fade would miss, and it is not
    /// a refinement — the outline wraps the ENDS as much as the flanks, so a
    /// body faded on its own leaves an opaque black cap hanging in front of a
    /// ribbon that has already gone. That is the shape the spectral pane's lead
    /// is drawn with (`panes::spectral::roll::lead`), where the tip lands in the
    /// middle of the spectrum and a ring of black there is the loudest thing on
    /// the pane.
    ///
    /// The OTHER end is untouched, which is what makes this a leading fade
    /// rather than a softening of the whole segment: the leading end is the low
    /// end of the depth axis, and the trailing end is where the ribbon carries
    /// on into the rest of its own note.
    #[test]
    fn a_lead_fade_takes_the_ribbon_out_toward_its_tip() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // White on black with no outline: every painted byte is the body's own
        // coverage, read straight off the frame. Under `TOP` the depth axis
        // runs down the screen, so the leading tip of this note is at y = 68
        // and its trailing end at y = 188. A lead of 40 points reaches to
        // y = 108, and the fade is measured back from the tip.
        let bare = |lead: f32, fade: f32| RollInstance {
            core: [255; 4],
            aura: [0; 4],
            ..led_note(lead, fade, 1.0)
        };
        let cov = |fade: f32, y: u32| {
            let frame = draw(&device, &queue, vec![bare(40.0, fade)], wgpu::Color::BLACK);
            f32::from(pixel(&frame, 128, y)[0]) / 255.0
        };

        // A fade over 20 points of the tip: coverage climbs linearly from
        // nothing at y = 68 to solid by y = 88. Sampled at pixel centres, which
        // is the half point in each expectation.
        for (y, want) in [(73u32, 0.275f32), (78, 0.525), (83, 0.775), (88, 1.0)] {
            let got = cov(20.0, y);
            assert!(
                (got - want).abs() < 0.06,
                "the leading fade covers {got:.3} at y = {y}, not {want:.3}",
            );
        }
        // Square-ended, the same ribbon is solid from its tip.
        assert!(cov(0.0, 73) > 0.97, "a fade of 0 is not a square end");
        // And a fade at one end is not a fade at the other: the trailing end
        // keeps its own edge whatever the leading one is doing.
        assert!(cov(20.0, 185) > 0.97, "the leading fade reached the trailing end too");

        // The outline's cap goes with it. Row 66 is a point and a half past the
        // tip, inside the harness outline's reach, and that is exactly where a
        // body-only fade
        // leaves a black ring standing in front of nothing.
        let capped = |fade: f32| {
            let frame = draw(&device, &queue, vec![led_note(40.0, fade, 1.0)], bg_color());
            pixel(&frame, 128, 66)
        };
        assert!(
            shadowed(capped(0.0)),
            "a square-ended ribbon lost its outline cap: {:?}",
            capped(0.0),
        );
        assert!(
            near(capped(20.0), BG),
            "the outline still caps a tip that has faded out: {:?}",
            capped(20.0),
        );

        // And the surround along the fading stretch goes with the ribbon
        // beside it. Half a point off the flank at y = 73, where the ribbon
        // covers about a quarter, the faded tip's surround darkens no more than
        // that share of what the square-ended one's does there. The blurred
        // cell does not see to this on its own: depth is spent in stops, so a
        // ribbon a quarter covered still casts a shadow all but black, and an
        // outline that took no share of the lead's coverage would ring the
        // fading tip in it.
        let flank = |fade: f32| {
            let frame = draw(&device, &queue, vec![led_note(40.0, fade, 1.0)], bg_color());
            f32::from(BG[2] - pixel(&frame, 140, 73)[2])
        };
        let (square, faded, share) = (flank(0.0), flank(20.0), cov(20.0, 73));
        assert!(square > 100.0, "the probe missed the square end's surround: {square}");
        assert!(
            faded <= share * square + 3.0,
            "beside the faded tip the surround darkens by {faded}, more than the ribbon's \
             {share:.3} of {square}",
        );
    }

    /// A lead's opacity dims the LEAD and never the note it hangs off.
    ///
    /// That division is the whole reason `lead` is carried per instance rather
    /// than the fade being measured against the box. The spectral pane takes a
    /// released note's lead out over its release time
    /// (`panes::spectral::roll::lead_alpha`) while the ribbon itself is still
    /// sounding-loud and still scrolling; an opacity applied across the box
    /// would fade the note along with the thing hanging off it, and a roll that
    /// dims every note you let go of is telling you something the music did not.
    ///
    /// Read on both sides of the note's own end: inside the lead the ink
    /// follows the opacity, and past it the ribbon is solid at every one.
    #[test]
    fn a_leads_opacity_dims_the_lead_and_not_the_note() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // White on black, no outline, a square-ended 40-point lead — so the
        // only thing between the tip (y = 68) and the note's own end (y = 108)
        // is the opacity, with no ramp on top of it to unpick.
        let cov = |alpha: f32, y: u32| {
            let note = RollInstance { core: [255; 4], aura: [0; 4], ..led_note(40.0, 0.0, alpha) };
            let frame = draw(&device, &queue, vec![note], wgpu::Color::BLACK);
            f32::from(pixel(&frame, 128, y)[0]) / 255.0
        };
        for alpha in [0.25f32, 0.5, 0.75, 1.0] {
            // Well inside the lead, and well inside the note.
            let (lead, body) = (cov(alpha, 88), cov(alpha, 148));
            assert!(
                (lead - alpha).abs() < 0.03,
                "the lead covers {lead:.3} at an opacity of {alpha}",
            );
            assert!(body > 0.97, "the note itself came out at {body:.3} with its lead at {alpha}",);
        }
        // Gone entirely, and the note is still every bit of itself.
        assert!(cov(0.0, 88) < 0.03, "a lead at no opacity still painted");
        assert!(cov(0.0, 148) > 0.97, "a lead at no opacity took its note with it");
    }

    /// The cap reaches exactly as far as `cap_reach`, which is the caller's
    /// answer and not the outline's.
    ///
    /// The two are the same number wherever the cap has the room for it. Where
    /// it does not — the spectral pane's now-line being the case that exists,
    /// with the clip a lead opens past it belonging to the lead — the cap is
    /// SHORTENED rather than dimmed, so it grows out of the note's end and is
    /// wholly on the near side of whatever the caller is protecting at every
    /// width it passes through. Dimmed, it would put ink across that line at
    /// every width but the last.
    #[test]
    fn a_caps_reach_is_the_callers_and_not_the_outlines() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // The lead spent, as the pane leaves it under a cap, so the cap is
        // uncovered and the frame reads it directly. The note's own end is at
        // y = 108, and the cap reaches back from it toward the tip.
        let at = |cap_reach: f32, y: u32| {
            let note = RollInstance { cap_reach, ..led_note(40.0, 0.0, 0.0) };
            pixel(&draw(&device, &queue, vec![note], bg_color()), 128, y)
        };
        // At the outline's own reach the cap paints a point and a half inside
        // the end (y = 106)...
        let full = at(HARNESS_REACH, 106);
        assert!(shadowed(full), "the full cap is absent: {full:?}");
        // ...and told to cap over a single point, it paints half a point inside
        // it (y = 107) and stops short of the row the outline's reach still
        // covers.
        assert!(shadowed(at(1.0, 107)), "the shortened cap is absent: {:?}", at(1.0, 107));
        assert!(
            near(at(1.0, 106), BG),
            "the cap reached past the point it was given: {:?}",
            at(1.0, 106),
        );
        // None at all is none drawn: a cap of no reach leaves the note's end
        // bare, which is what a caller with no room to give it is asking for.
        let none = RollInstance { cap_reach: 0.0, ..led_note(40.0, 0.0, 0.0) };
        let bare = draw(&device, &queue, vec![none], bg_color());
        assert!(
            near(pixel(&bare, 128, 107), BG),
            "a cap of no reach still painted: {:?}",
            pixel(&bare, 128, 107),
        );
    }

    /// A note is a rectangle: its corners are square, right out to them.
    ///
    /// Nothing rounds a note, and this samples the corner a radius would
    /// take off. On the notes short enough for rounding to show at all it
    /// only hurts — a tap is a few points long, the radius clamps to its own
    /// half-length, and the note comes out a bead. A run of them comes out as
    /// a string of beads.
    #[test]
    fn a_notes_corners_are_square() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // 40 points across pitch, 6 along time — the shape of a tapped key on
        // a thick ribbon. No outline, so the sample reads the shape alone.
        let tap = RollInstance { half_extent: [20.0, 3.0], aura: [0; 4], ..centered_note() };
        let frame = draw(&device, &queue, vec![tap], bg_color());
        // 19.5 points out along pitch and 2.5 along time: inside the square
        // note, and outside any rounding of it (a radius clamped to the note's
        // half-length would arc from 17 out, missing this by half a point).
        let corner = pixel(&frame, 147, 130);
        assert!(
            near(corner, [255, 0, 0, 255]),
            "the tap's corner is missing ({corner:?}) — something is rounding it off",
        );
    }

    /// The outline must still stand OUTSIDE a note floored to its minimum
    /// thickness: the note's own color at the middle, the outline beyond it.
    /// This is the same invariant at the width where it actually bites — a
    /// hairline is all edge, so an outline that grew inward would simply paint
    /// over the note.
    ///
    /// A DARK outline is what makes it the sharp test: black leaking inward
    /// does not merely tint a hairline, it erases the one thing the ribbon is
    /// there to say, which is its color.
    #[test]
    fn the_outline_does_not_paint_over_a_hairline_note() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // 1.5 points across pitch: what `panes::spectral::roll` floors a ribbon too thin
        // to see to (MIN_RIBBON_PX). Narrower than a pixel is wide, so no
        // sample here is purely one thing — what matters is that the middle
        // still reads as the NOTE rather than as the outline having flooded it.
        // Its own coverage there is 0.75, so a red channel anywhere near that
        // is the note; near 0 is the outline standing where the note should be.
        let note =
            RollInstance { half_extent: [0.75, 60.0], core: [204, 0, 0, 204], ..centered_note() };
        let frame = draw(&device, &queue, vec![note], bg_color());
        let at = |x: u32| pixel(&frame, x, 128);
        let middle = at(128);
        assert!(middle[0] > 150, "the outline flooded the note's own color: {middle:?}");
        assert!(middle[1] < BG[1], "the shadow did not darken the backdrop: {middle:?}");
        assert!(shadowed(at(130)), "no outline beside it: {:?}", at(130));
        assert!(near(at(134), BG), "the outline reaches further than it should: {:?}", at(134));
    }

    /// A note two pixels along the depth axis holds its brightness as it
    /// scrolls sub-pixel; one pixel does not. That threshold is what
    /// `panes::spectral::roll::MIN_LENGTH_DEVICE_PX` floors a brief note's length to, and
    /// this is the measurement it is quoted from.
    ///
    /// The `inside` box filter is one pixel wide, so a shape's coverage
    /// profile is a trapezoid with one-pixel ramps: its flat top is
    /// `length - 1` pixels across, and every sub-pixel offset lands a sample on
    /// the top only once that top is a pixel wide. Under it, some offsets catch
    /// only the ramps and the note reads dimmer — which, on something scrolling,
    /// is a flicker at the scroll rate rather than a static difference.
    ///
    /// Total ink is not the thing to measure: the filter conserves it to a
    /// fraction of a percent well past the point where the peak has collapsed,
    /// so a test on ink alone passes while the artifact is at its worst.
    #[test]
    fn a_two_pixel_note_holds_its_brightness_as_it_scrolls() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // White, no outline: every painted byte is the fill's own coverage.
        let bare = RollInstance { core: [255, 255, 255, 255], aura: [0; 4], ..centered_note() };
        // The brightest pixel anywhere, over a sweep of sub-pixel scroll
        // offsets along depth — which is y under [`TOP`]. One point is one
        // pixel on this surface.
        let peak_spread = |half_depth: f32| {
            let peaks: Vec<u8> = (0..8)
                .map(|step| {
                    let note = RollInstance {
                        center: [128.0, 128.0 + step as f32 / 8.0],
                        half_extent: [bare.half_extent[0], half_depth],
                        ..bare
                    };
                    let frame = draw(&device, &queue, vec![note], wgpu::Color::BLACK);
                    (0..SIZE[1])
                        .flat_map(|y| (0..SIZE[0]).map(move |x| (x, y)))
                        .map(|(x, y)| pixel(&frame, x, y)[0])
                        .max()
                        .unwrap_or(0)
                })
                .collect();
            let (lo, hi) = (*peaks.iter().min().unwrap(), *peaks.iter().max().unwrap());
            f32::from(hi - lo) / f32::from(hi.max(1))
        };

        // Two pixels long: the floor. Nothing may move.
        let floored = peak_spread(1.0);
        assert!(
            floored < 0.02,
            "a two-pixel note pulsed by {:.0}% as it scrolled",
            floored * 100.0,
        );

        // One pixel: what an unfloored brief note gets, and what the floor is
        // for. Asserted so the check above cannot pass by measuring nothing.
        let hairline = peak_spread(0.5);
        assert!(
            hairline > 0.3,
            "a one-pixel note only pulsed by {:.0}%, so the floor above is guarding nothing",
            hairline * 100.0,
        );
    }

    /// The sharp pass is the control for the two low-resolution effects around
    /// it. A two-pixel note must move by fractional coverage, not sit for seven
    /// frames and jump on the eighth. Total ink alone cannot say that: a
    /// snapped shape conserves just as much, so read its coverage-weighted
    /// centre as well.
    #[test]
    fn a_two_pixel_notes_coverage_centroid_moves_sub_pixel() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let bare = RollInstance {
            half_extent: [12.0, 1.0],
            core: [255; 4],
            aura: [0; 4],
            ..centered_note()
        };
        let mut centres = Vec::new();
        let mut ink = Vec::new();
        for step in 0..8 {
            let note = RollInstance { center: [128.0, 128.0 + step as f32 / 8.0], ..bare };
            let frame = draw(&device, &queue, vec![note], wgpu::Color::BLACK);
            let mut mass = 0.0f64;
            let mut moment = 0.0f64;
            for y in 0..SIZE[1] {
                for x in 0..SIZE[0] {
                    let coverage = f64::from(pixel(&frame, x, y)[0]) / 255.0;
                    mass += coverage;
                    moment += coverage * (f64::from(y) + 0.5);
                }
            }
            centres.push(moment / mass);
            ink.push(mass);
        }
        for pair in centres.windows(2) {
            let moved = pair[1] - pair[0];
            assert!(
                (moved - 0.125).abs() < 0.01,
                "the sharp note moved {moved:.4} px in one eighth-pixel step: {centres:?}",
            );
        }
        let spread = ink.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            - ink.iter().copied().fold(f64::INFINITY, f64::min);
        assert!(spread < 0.1, "the sharp note changed {spread:.4} px of ink: {ink:?}");
    }

    /// Bloom thresholds the note before blurring it. Its source therefore has
    /// to preserve the sharp pass's sub-pixel coverage: at half resolution the
    /// roll's two-device-pixel minimum note was only one source texel, whose
    /// peak pulsed as it crossed that grid and made the halo pulse with it.
    #[test]
    fn a_two_pixel_notes_bloom_holds_its_energy_as_it_scrolls() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let bare = RollInstance {
            half_extent: [12.0, 1.0],
            core: [160, 96, 48, 204],
            aura: [0; 4],
            ..centered_note()
        };
        let energy: Vec<f64> = (0..8)
            .map(|step| {
                let note = RollInstance { center: [128.0, 128.0 + step as f32 / 8.0], ..bare };
                let plain = draw_bloomed(&device, &queue, vec![note], TOP, 0.0, wgpu::Color::BLACK);
                let bloomed =
                    draw_bloomed(&device, &queue, vec![note], TOP, 1.5, wgpu::Color::BLACK);
                bloomed
                    .chunks_exact(4)
                    .zip(plain.chunks_exact(4))
                    .map(|(lit, base)| f64::from(lit[0].saturating_sub(base[0])))
                    .sum()
            })
            .collect();
        let lo = energy.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = energy.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!(
            (hi - lo) / hi.max(1.0) < 0.04,
            "the two-pixel note's bloom energy pulsed across sub-pixel phases: {energy:?}",
        );
    }

    /// The halo keeps its shape as a note crosses the quarter-size grid it is
    /// blurred on, read in the note's own frame rather than as a total — a
    /// halo can hold its energy while it swells and shrinks around the note.
    ///
    /// The chain shrinks the picture twice and stretches the blur back up, and
    /// with a plain two-texel average down and a bilinear read back up, a two-
    /// pixel note's halo changed shape by 18% of its peak every four pixels it
    /// travelled — at a long Span, a pulse a few times a second at the end of
    /// every note. Measured on a 1080 px pane, the size the blur's profile is
    /// defined at, which after `BloomChain::scales` gives back the filters'
    /// spread runs the blur's sub-texel arm (scale 0.95); the integrating arm
    /// above about 1130 px blurs wider and swings no more. The same reading
    /// sweeps to 4.1% at 600 px and 10% at 400, where the halo is barely
    /// wider than a quarter texel.
    #[test]
    fn a_two_pixel_notes_halo_keeps_its_shape_as_it_scrolls() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let note = RollInstance {
            half_extent: [12.0, 1.0],
            core: [255, 255, 255, 255],
            aura: [0; 4],
            ..centered_note()
        };
        // A roll 2 mod 4 pixels tall as well as an aligned one: the half target
        // was rounded up from the screen and the quarter from the screen too,
        // so at 254 px the quarter was not half the half, the tent slid off its
        // texels across the pane, and the halo pulsed by 66%.
        for height in [256.0f32, 254.0] {
            let rect =
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, height));
            let column = |bloom: f32, note: RollInstance| -> Vec<f64> {
                let cb = RollCallback {
                    point_scale: 1.0,
                    rect,
                    pane_size: egui::vec2(1080.0, 1080.0),
                    instances: vec![note],
                    clipped_tail: None,
                    axes: TOP,
                    shadow: harmonigraph_scene::ShadowStyle { depth: 0.0, ..harness_shadow() },
                    cut: RollCut::OFF,
                    bloom,
                    target_format: FORMAT,
                    pane_id: 0,
                    pass_nr: 0,
                };
                let frame = draw_callback(&device, &queue, cb, wgpu::Color::BLACK).0;
                (0..SIZE[1]).map(|y| f64::from(frame[((y * SIZE[0] + 128) * 4) as usize])).collect()
            };
            // Glow alone, from 4 to 30 px either side of the note — its own pixels
            // are white and clip — at each eighth of a pixel across two
            // quarter-size texels.
            let profiles: Vec<Vec<f64>> = (0..64)
                .map(|step| {
                    let centre = 120.0 + step as f32 / 8.0;
                    let note = RollInstance { center: [128.0, centre], ..note };
                    let (plain, lit) = (column(0.0, note), column(4.0, note));
                    let glow: Vec<f64> = lit.iter().zip(&plain).map(|(l, p)| l - p).collect();
                    (-30..=30)
                        .filter(|d: &i32| d.abs() >= 4)
                        .map(|d| {
                            let y = f64::from(centre) + f64::from(d) - 0.5;
                            let (i, t) = (y.floor() as usize, y - y.floor());
                            glow[i] * (1.0 - t) + glow[i + 1] * t
                        })
                        .collect()
                })
                .collect();
            let peak = profiles.iter().flatten().copied().fold(0.0, f64::max);
            let swing = (0..profiles[0].len())
                .map(|d| {
                    let at = profiles.iter().map(|p| p[d]);
                    at.clone().fold(0.0, f64::max) - at.fold(f64::MAX, f64::min)
                })
                .fold(0.0, f64::max);
            assert!(peak > 50.0, "{height} px roll: too little halo to measure: peak {peak}");
            assert!(
                swing / peak < 0.05,
                "{height} px roll: the halo changed shape by {:.1}% of its {peak} peak as the note slid",
                100.0 * swing / peak,
            );
        }
    }

    /// A spread glow behind a spent lead stands off the note's end exactly as
    /// it stands off the same note with no lead at all.
    ///
    /// Behind the lead the surround is the CAP (`cap_coverage`), measured to
    /// the note inside the box rather than to the box, and the spread's solid
    /// band has to start from that same end. Measured from anywhere else, the
    /// surround there would come out thinner or thicker than at every other
    /// edge of the note. The pane keeps a spent lead on every released note
    /// until its end clears the now-line, so this is the surround at that end.
    ///
    /// Only a spread tells the two apart from a plain profile, which is why the
    /// fixture has one.
    #[test]
    fn a_spread_glow_behind_a_spent_lead_matches_the_note_alone() {
        let Some((device, queue)) = headless_device() else { return };
        let style = harmonigraph_scene::ShadowStyle { width: 1.0, depth: 1.0, spread: 1.0 };
        assert!(
            style.gaussian_spread_points(crate::shadow::spectral_sigma_points(style, 1.0)) > 1.0
        );
        let reach = crate::shadow::spectral_shadow_reach(style, 1.0);
        // The note's own stretch is y 108..188 either way; the led box carries
        // a spent 40-point lead in front of it, out to y = 68.
        let led = RollInstance { cap_reach: reach, ..led_note(40.0, 0.0, 0.0) };
        let alone =
            RollInstance { center: [128.0, 148.0], half_extent: [12.0, 40.0], ..centered_note() };
        // All the darkening in front of the note's own end, on blue.
        let darkness = |note: RollInstance| {
            let frame = draw_shadowed(&device, &queue, vec![note], style, bg_color(), 1.0);
            (40..108)
                .flat_map(|y| (0..SIZE[0]).map(move |x| (x, y)))
                .map(|(x, y)| f32::from(BG[2].saturating_sub(pixel(&frame, x, y)[2])))
                .sum::<f32>()
        };
        let (behind_lead, by_itself) = (darkness(led), darkness(alone));
        assert!(by_itself > 1000.0, "the note alone cast no shadow past its end: {by_itself}");
        let ratio = behind_lead / by_itself;
        assert!(
            (ratio - 1.0).abs() < 0.05,
            "behind a spent lead the note's end casts {ratio:.3} of its shadow alone \
             ({behind_lead} against {by_itself})",
        );
    }

    #[test]
    fn spread_expands_roll_glow() {
        let Some((device, queue)) = headless_device() else { return };
        let style =
            harmonigraph_scene::ShadowStyle { width: 2.0, depth: 1.0, ..Default::default() };
        let note = RollInstance { core: [255; 4], ..centered_note() };
        let draw = |style| draw_shadowed(&device, &queue, vec![note], style, bg_color(), 1.0);
        let base = draw(style);
        let grown = draw(harmonigraph_scene::ShadowStyle { spread: 0.5, ..style });
        assert_eq!(pixel(&base, 128, 128), pixel(&grown, 128, 128), "visible core moved");
        // x=140 is the body's edge; the unspread profile ends at x=152.
        assert!(
            pixel(&grown, 153, 128)[2] < pixel(&base, 153, 128)[2],
            "expanded footprint was clipped"
        );
    }

    fn bloom_frame(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        ppp: f32,
        rect: egui::Rect,
        bloom: f32,
    ) -> Vec<u8> {
        let size = [(256.0 * ppp) as u32; 2];
        let cb = RollCallback {
            point_scale: 1.0,
            rect,
            pane_size: egui::vec2(1080.0, 1080.0),
            instances: vec![RollInstance {
                half_extent: [8.0, 32.0],
                core: [128, 128, 128, 255],
                aura: [0; 4],
                ..centered_note()
            }],
            clipped_tail: None,
            axes: TOP,
            shadow: harmonigraph_scene::ShadowStyle { width: 0.0, ..Default::default() },
            cut: RollCut::OFF,
            bloom,
            target_format: FORMAT,
            pane_id: 0,
            pass_nr: 0,
        };
        let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: ppp };
        let mut resources = CallbackResources::default();
        let mut encoder = device.create_command_encoder(&Default::default());
        let buffers = cb.prepare(device, queue, &screen, &mut encoder, &mut resources);
        queue.submit(buffers.into_iter().chain([encoder.finish()]));
        let texture = render_to_texture(device, queue, size, FORMAT, wgpu::Color::BLACK, |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: ppp,
                    screen_size_px: size,
                },
                pass,
                &resources,
            );
        });
        readback(device, queue, &texture, size)
    }

    #[test]
    fn bloom_width_follows_resolution() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(256.0, 256.0));
        let sigma = |ppp: f32| {
            let plain = bloom_frame(&device, &queue, ppp, rect, 0.0);
            let lit = bloom_frame(&device, &queue, ppp, rect, 1.0);
            let width = (256.0 * ppp) as usize;
            let y = (128.0 * ppp) as usize;
            let (mass, moment) = (0..width).fold((0.0, 0.0), |(mass, moment), x| {
                let i = (y * width + x) * 4;
                let weight = f64::from(lit[i].saturating_sub(plain[i]));
                let offset = x as f64 + 0.5 - 128.0 * f64::from(ppp);
                (mass + weight, moment + weight * offset * offset)
            });
            assert!(mass > 100.0, "fixture needs measurable bloom: {mass}");
            (moment / mass).sqrt() / f64::from(ppp)
        };
        let reference = sigma(1.0);
        assert!(reference > 4.0, "fixture must reach the blur: {reference}");
        for ppp in [0.5, 1.5, 2.0] {
            let measured = sigma(ppp);
            assert!(
                (measured / reference - 1.0).abs() < 0.15,
                "{ppp}ppp bloom widths: {reference}, {measured}"
            );
        }
    }

    #[test]
    fn bloom_width_uses_the_owning_pane() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(256.0, 256.0));
        let cropped = egui::Rect::from_min_max(egui::pos2(0.0, 64.0), egui::pos2(256.0, 192.0));
        let a = bloom_frame(&device, &queue, 1.0, whole, 1.0);
        let b = bloom_frame(&device, &queue, 1.0, cropped, 1.0);
        let mut measurable = false;
        for y in 112..144 {
            for x in 104..152 {
                let i = (y * 256 + x) * 4;
                measurable |= x > 138 && a[i] > 4;
                assert!(a[i].abs_diff(b[i]) <= 1, "({x},{y}): {} vs {}", a[i], b[i]);
            }
        }
        assert!(measurable, "comparison must contain halo outside the ribbon");
    }

    /// The bloom adds light around a note and to the note itself, and adds it
    /// as LIGHT: nothing it touches is occluded, and a strength of 0 leaves the
    /// frame byte for byte the frame with no bloom in it at all.
    ///
    /// The halo is the lattice's, off the same chain — `fs_bright`'s threshold
    /// and knee, the same pane-scaled profile on the quarter-size texture — so
    /// what this owes is that the roll runs it, not what a Gaussian does. Both
    /// halves are the point: light that did not reach past the note would be a
    /// tint, and light that did not brighten the note's own body would be a
    /// halo drawn around it rather than the note glowing.
    #[test]
    fn the_bloom_adds_light_around_a_note() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // Bright enough to clear the threshold, dim enough that brightening it
        // has somewhere to go. No outline: black is the one thing that cannot
        // bloom, and its coverage would only dilute what does.
        let note = RollInstance {
            core: [160, 96, 48, 204], // 80% of the warm note color.
            aura: [0; 4],
            ..centered_note()
        };
        let at = |frame: &[u8], x: u32, y: u32| f32::from(pixel(frame, x, y)[0]);
        let plain = draw_bloomed(&device, &queue, vec![note], TOP, 0.0, wgpu::Color::BLACK);
        let lit = draw_bloomed(&device, &queue, vec![note], TOP, 1.5, wgpu::Color::BLACK);

        // The note's edge is at x = 140; 2 points past it is inside the
        // halo's reach and well outside the note.
        assert_eq!(at(&plain, 142, 128), 0.0, "the unbloomed frame is not black beside the note");
        assert!(at(&lit, 142, 128) > 8.0, "no light beside the note: {}", at(&lit, 142, 128),);
        assert!(
            at(&lit, 128, 128) > at(&plain, 128, 128) + 8.0,
            "the note's own body was not brightened: {} against {}",
            at(&lit, 128, 128),
            at(&plain, 128, 128),
        );
        // Light, not a shape: the halo may never take alpha away from what is
        // under it, and over an opaque frame that means the alpha channel is
        // untouched everywhere.
        for (x, y) in [(128u32, 128u32), (142, 128), (200, 40)] {
            assert_eq!(
                pixel(&lit, x, y)[3],
                255,
                "the halo punched a hole in the alpha at ({x}, {y})",
            );
        }

        // And at 0 it is not merely faint: no chain is built and no pass runs.
        //
        // Asked of the RESOURCES rather than of the frame. Two frames drawn at
        // strength 0 are the same bytes however much work went into them, so a
        // `paint` that started laying an all-black halo over every note, or a
        // `prepare` that started running the chain and multiplying by zero,
        // would leave the picture untouched and cost the whole thing — which
        // is what "0 skips it whole" is a claim about.
        let black = wgpu::Color::BLACK;
        let bloom_of = |strength: f32| {
            let (_, resources) =
                draw_bloomed_resourced(&device, &queue, vec![note], TOP, strength, black);
            let roll: &RollResources = resources.get().expect("the callback inserts its resources");
            roll.panes.get(0).expect("the roll prepared a pane").bloom.is_some()
        };
        assert!(!bloom_of(0.0), "a strength of 0 built the bloom chain anyway");
        assert!(bloom_of(1.5), "no chain was built at a strength that asks for one");
    }

    /// A note's bloom follows the opacity it is drawn at (#1289).
    ///
    /// The Opacity mapping reaches this crate as the instance's `fade` — the
    /// pane's `read_through` writes it, and
    /// `a_notes_fade_follows_its_pressure_in_pieces_of_one_box` in
    /// `panes::spectral::roll` pins a routed mapping to it. So a quarter
    /// reading here is a note the mapping drew at a quarter, and its halo has
    /// to be a quarter of the full one's.
    ///
    /// Quarter rather than merely less, because the threshold reads the
    /// STRAIGHT color and spends coverage after it (`fs_bright_coverage`):
    /// opacity scales the light linearly, so anything off the ratio is the
    /// bloom pass reading some other opacity than the one on screen.
    #[test]
    fn a_notes_bloom_follows_its_opacity_reading() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let note = |fade: f32| RollInstance {
            core: [160, 96, 48, 255],
            aura: [0; 4],
            fade: [fade; 2],
            ..centered_note()
        };
        // All the light the halo adds around the note, on red: the bloomed
        // frame less the same note drawn without bloom. Off the body and a
        // pixel round it, where the unbloomed frame is black at either
        // opacity — over the full note's body the sum clips at 255 and reads
        // the halo short.
        let light = |fade: f32| {
            let plain =
                draw_bloomed(&device, &queue, vec![note(fade)], TOP, 0.0, wgpu::Color::BLACK);
            let lit = draw_bloomed(&device, &queue, vec![note(fade)], TOP, 1.5, wgpu::Color::BLACK);
            (0..SIZE[1])
                .flat_map(|y| (0..SIZE[0]).map(move |x| (x, y)))
                .filter(|&(x, y)| !(115..141).contains(&x) || !(67..189).contains(&y))
                .map(|(x, y)| f32::from(pixel(&lit, x, y)[0]) - f32::from(pixel(&plain, x, y)[0]))
                .sum::<f32>()
        };
        let (full, quarter) = (light(1.0), light(0.25));
        assert!(full > 1000.0, "the unfaded note grew no halo to compare against: {full}");
        let ratio = quarter / full;
        assert!(
            (ratio - 0.25).abs() < 0.03,
            "a note drawn at a quarter opacity blooms at {ratio:.3} of the full note's light \
             ({quarter} against {full})",
        );
    }

    /// One `prepare` of `cb` against `resources`, submitted — the unit both
    /// tests below count in, since a pane's age is measured in these.
    fn prepare_once(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &mut CallbackResources,
        ppp: f32,
        cb: &RollCallback,
    ) {
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: ppp };
        prepare_frame(device, queue, resources, &screen, cb);
    }

    /// A roll callback over `rect`, drawing one note with the bloom on.
    fn bloomed_callback(rect: egui::Rect, pane_id: u64) -> RollCallback {
        RollCallback {
            point_scale: 1.0,
            rect,
            pane_size: rect.size(),
            instances: vec![centered_note()],
            clipped_tail: None,
            axes: TOP,
            shadow: harmonigraph_scene::ShadowStyle { width: 0.0, ..Default::default() },
            cut: RollCut::OFF,
            bloom: 1.5,
            target_format: FORMAT,
            pane_id,
            pass_nr: 0,
        }
    }

    /// The bloom chain covers exactly the pixels the halo is stretched across.
    ///
    /// `paint` lays quarter A over `viewport_in_pixels()`, which rounds each
    /// EDGE of the roll's rect and subtracts, then clamps to the screen — not
    /// the width, rounded. On a rect whose two edges round in opposite
    /// directions those differ by a pixel, and a texture sized one way and
    /// stretched the other puts the halo at a slightly different scale from
    /// the notes it grew out of, which on a picture that scrolls sub-pixel is
    /// a halo sliding against its own note.
    #[test]
    fn the_bloom_covers_the_pixels_the_halo_is_stretched_across() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let ppp = 2.0;
        // 5.25 -> 10.5 -> 11 (up) and 15.2 -> 30.4 -> 30 (down), so the
        // viewport is 19 px across where the rounded width is 20.
        let rect = egui::Rect::from_min_max(egui::pos2(5.25, 4.0), egui::pos2(15.2, 60.0));
        let cb = bloomed_callback(rect, 0);
        let mut resources = CallbackResources::default();
        prepare_once(&device, &queue, &mut resources, ppp, &cb);

        let vp = egui::epaint::ViewportInPixels::from_points(&rect, ppp, SIZE);
        let roll: &RollResources = resources.get().expect("prepare inserts its resources");
        let bloom = roll
            .panes
            .get(0)
            .expect("the roll prepared a pane")
            .bloom
            .as_ref()
            .expect("a strength of 1.5 asks for a chain");
        assert_eq!(
            bloom.size,
            [vp.width_px as u32, vp.height_px as u32],
            "the chain covers {:?} where paint stretches it across {}x{}",
            bloom.size,
            vp.width_px,
            vp.height_px,
        );
        // And the two roundings genuinely disagree on this rect, or the
        // assertion above holds for a reason that is not the one it is about.
        assert_ne!(
            (rect.width() * ppp).round() as i32,
            vp.width_px,
            "this rect no longer tells the two roundings apart; the test is vacuous",
        );
    }

    /// A pane that stops being drawn gives its buffers and its bloom chain
    /// back, rather than holding three textures the size of the pane it was
    /// last shown at for as long as the plugin is loaded.
    ///
    /// There is no teardown to hang this on — a closed tab simply stops
    /// calling back — so the only evidence is the panes that ARE still
    /// preparing, and the sweep runs from whichever one that is.
    #[test]
    fn a_pane_that_stops_drawing_gives_its_chain_back() {
        // Held across every prepare: under `hot-reload` a shader published by
        // another test between two of them makes `is_stale` rebuild the
        // resources, pane map included, which reads here as an eviction (#675).
        let _guard = crate::reload::test_lock();
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        let (mut docked, preview) = (bloomed_callback(rect, 0), bloomed_callback(rect, 1));
        let mut resources = CallbackResources::default();
        prepare_once(&device, &queue, &mut resources, 1.0, &docked);
        prepare_once(&device, &queue, &mut resources, 1.0, &preview);
        let live = |resources: &CallbackResources| {
            let roll: &RollResources = resources.get().expect("prepare inserts its resources");
            let mut ids: Vec<u64> = roll.panes.keys().collect();
            ids.sort_unstable();
            ids
        };
        assert_eq!(live(&resources), vec![0, 1], "both rolls should be holding buffers");

        // The preview's tab closes: the docked roll goes on drawing alone.
        for pass_nr in 1..=crate::pass_aged::TTL_PASSES {
            docked.pass_nr = pass_nr;
            prepare_once(&device, &queue, &mut resources, 1.0, &docked);
        }
        assert_eq!(live(&resources), vec![0], "the closed pane is still holding its chain");
    }

    /// Egui prepares every callback in a frame before it paints any of them,
    /// so the cache lifetime must not advance as each live pane prepares.
    #[test]
    fn every_pane_prepared_in_one_frame_survives_until_paint() {
        // Held across every prepare, for the reason given in the test above.
        let _guard = crate::reload::test_lock();
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        let mut resources = CallbackResources::default();

        for pane_id in 0..=crate::pass_aged::TTL_PASSES {
            let pane = bloomed_callback(rect, pane_id);
            prepare_once(&device, &queue, &mut resources, 1.0, &pane);
        }

        let roll: &RollResources = resources.get().expect("prepare inserts its resources");
        assert!(
            roll.panes.contains_key(0),
            "the first live pane was evicted before egui reached its paint callback",
        );
    }

    /// A thin ribbon glows in proportion to the ink it has, not a fraction of
    /// it — which is a claim about WHERE the threshold sits in the chain.
    ///
    /// `fs_bright` is a soft knee, so what reaches it decides everything. The
    /// chain thresholds the half-res picture and downsamples afterwards; a
    /// chain that thresholds after the downsample instead measures a color
    /// already box-averaged a second time, and on a ribbon near the width of
    /// the pixel doing the averaging that is a fraction of the color it is
    /// actually painted in. The knee then gates it hard, so a note goes dark
    /// in the halo while the lattice node it lit up — same color, same
    /// setting, but several pixels across — glows normally.
    ///
    /// Measured as the total light the halo adds, against a ribbon eight times
    /// wider. The blur conserves light, so the ratio is the THRESHOLD's answer
    /// and nothing else: 13x with the threshold above the downsample, 79x with
    /// it below, against 8x of ink. The wide ribbon is the control and comes
    /// out at the same 431588 either way — a shape comfortably wider than the
    /// pixel cannot tell the two chains apart, which is why this needs a thin
    /// one to say anything at all.
    #[test]
    fn a_thin_ribbon_glows_in_proportion_to_the_ink_it_has() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // Mid grey: luminance ~0.47, which the knee (0.35 +/- 0.25) is still
        // climbing through, so the halo is sensitive to what the threshold is
        // handed rather than saturated whatever it is.
        //
        // Three points across, which is what the spectral pane's own width
        // floor (`MIN_RIBBON_PX`, 1.5 points) comes to on the 2x display this
        // is looked at on — the notes are rendered at half the roll's device
        // size, so what decides this is the ribbon's width in DEVICE pixels.
        let thin = RollInstance {
            half_extent: [1.5, 60.0],
            core: [120, 120, 120, 255],
            aura: [0; 4],
            ..centered_note()
        };
        let wide = RollInstance { half_extent: [12.0, 60.0], ..thin };
        // All the light the halo adds, over the whole frame: the blur moves
        // light around and conserves it, so this is what the THRESHOLD let
        // through and nothing else. Red channel only — the note is grey.
        let light = |note: RollInstance| {
            let plain = draw_bloomed(&device, &queue, vec![note], TOP, 0.0, wgpu::Color::BLACK);
            let lit = draw_bloomed(&device, &queue, vec![note], TOP, 1.5, wgpu::Color::BLACK);
            lit.iter()
                .zip(&plain)
                .step_by(4)
                .map(|(a, b)| f32::from(*a) - f32::from(*b))
                .sum::<f32>()
        };
        let (thin_light, wide_light) = (light(thin), light(wide));
        assert!(thin_light > 0.0, "a thin ribbon grew no halo at all");
        let ratio = wide_light / thin_light;
        assert!(
            ratio < 25.0,
            "the wide ribbon out-glows the thin one {ratio:.0}x on 8x the ink — the \
             threshold is reading a picture that has been downsampled past it \
             ({thin_light} against {wide_light})",
        );
    }

    /// The pane's orientation lives entirely in the uniform: turning the axes
    /// turns the picture, and nothing in the instances or the shader names a
    /// screen side.
    ///
    /// [`TOP`] against [`LEFT`], which is a rotation AND a flip. The same note
    /// drawn through each must come out as the same picture, turned:
    /// `left(x, y)` is `top(255 - y, x)`.
    #[test]
    fn turning_the_axes_turns_the_picture() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let top = draw(&device, &queue, vec![centered_note()], bg_color());
        let left = draw_turned(&device, &queue, vec![centered_note()], LEFT, bg_color());

        // A window around the note, wide enough to hold its full length.
        let mut painted = 0;
        for y in 60..200u32 {
            for x in 60..200u32 {
                let (a, b) = (pixel(&left, x, y), pixel(&top, 255 - y, x));
                assert!(near(a, b), "the turned pane differs at ({x}, {y}): {a:?} vs {b:?}");
                if !near(a, BG) {
                    painted += 1;
                }
            }
        }
        assert!(painted > 500, "the note barely drew ({painted} pixels); the comparison is thin");
    }

    /// A REVERSED depth direction mirrors the picture and changes nothing else.
    ///
    /// The pane has four orientations, and the two whose now-line is on the
    /// right or the bottom hand this shader a negated `depth_dir` — `[-1, 0]`
    /// and `[0, -1]`, which the other two can never produce. Nothing here reads
    /// a screen side, so the claim is that a negated direction is just a mirror:
    /// a note drawn through `RIGHT` is the `LEFT` picture reflected in x, to the
    /// byte. A sign dropped anywhere between the uniform and the box distance
    /// would show as the mirror failing, most likely by the glide's shear
    /// landing on the wrong side.
    #[test]
    fn a_reversed_depth_direction_only_mirrors_the_picture() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // A note with a glide, so the picture is not its own mirror image and
        // the comparison can actually fail.
        let bent = RollInstance { shear: 0.6, ..centered_note() };
        let left = draw_turned(&device, &queue, vec![bent], LEFT, bg_color());
        let right = draw_turned(&device, &queue, vec![bent], RIGHT, bg_color());

        // `LEFT` and `RIGHT` share a centre column, so the reflection is about
        // x = 255 - x with the note's own centre fixed.
        let mut painted = 0;
        for y in 0..SIZE[1] {
            for x in 0..SIZE[0] {
                let (a, b) = (pixel(&right, x, y), pixel(&left, SIZE[0] - 1 - x, y));
                assert!(near(a, b), "the mirrored pane differs at ({x}, {y}): {a:?} vs {b:?}");
                if !near(a, BG) {
                    painted += 1;
                }
            }
        }
        assert!(painted > 500, "the note barely drew ({painted} pixels); the comparison is thin");

        // And the vertical pair, so a flip that only reached the x axis is
        // caught too.
        let top = draw_turned(&device, &queue, vec![bent], TOP, bg_color());
        let bottom = draw_turned(&device, &queue, vec![bent], BOTTOM, bg_color());
        for y in 0..SIZE[1] {
            for x in 0..SIZE[0] {
                let (a, b) = (pixel(&bottom, x, y), pixel(&top, x, SIZE[1] - 1 - y));
                assert!(near(a, b), "the mirrored upright pane differs at ({x}, {y})");
            }
        }
    }

    /// An outline drawn first cannot erase a later opaque body.
    ///
    /// The outline is at its darkest where it meets its own note. Composited
    /// with its own note it therefore lands, at that strength, on whatever it
    /// reaches into; and along time what it reaches into is the next note,
    /// since repeats of one key butt together there. The later note blanked the
    /// tail of the earlier.
    ///
    /// Every outline is drawn before every body. These full-opacity fixtures
    /// must cover the neighboring shadow. Faded bodies may take some of that
    /// shadow through, but retain their own premultiplied color.
    #[test]
    fn an_outline_never_covers_another_notes_body() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // Depth runs down y under `TOP`. The two boxes meet exactly at y=128,
        // and the harness outline reaches three points into each other's.
        let butted = |center, core| RollInstance {
            center: [128.0, center],
            half_extent: [12.0, 30.0],
            core,
            aura: [0, 0, 0, 255],
            ..centered_note()
        };
        const RED: [u8; 4] = [255, 0, 0, 255];
        const GREEN: [u8; 4] = [0, 255, 0, 255];
        let early = butted(98.0, RED); // y 68..128
        let late = butted(158.0, GREEN); // y 128..188
        let frame = draw(&device, &queue, vec![early, late], bg_color());
        let grouped = draw_with_clipped_tail(&device, &queue, vec![early, late], 1, bg_color());
        assert_eq!(
            grouped, frame,
            "putting the later note in the clipped tail changed global layer ordering"
        );

        // A point and a half inside the earlier note's tail, which is a point
        // and a half inside the later note's outline. This is the pixel that
        // went black.
        assert!(
            shadowed(pixel(&draw(&device, &queue, vec![late], bg_color()), 128, 126)),
            "the probe is not under the later note's outline",
        );
        assert!(
            near(pixel(&frame, 128, 126), RED),
            "the later note's outline blanked the earlier note's tail: {:?}",
            pixel(&frame, 128, 126),
        );
        // And the same join from the other side: paint order is not what is
        // deciding it.
        assert!(
            shadowed(pixel(&draw(&device, &queue, vec![early], bg_color()), 128, 129)),
            "the probe is not under the earlier note's outline",
        );
        assert!(
            near(pixel(&frame, 128, 129), GREEN),
            "the earlier note's outline blanked the later note's head: {:?}",
            pixel(&frame, 128, 129),
        );
        // The outline is still drawn — over the pane, where no note is.
        let outside = pixel(&frame, 128, 66);
        assert!(
            shadowed(outside),
            "the outline stopped painting at all outside the notes: {outside:?}",
        );
    }

    /// How far off the pixel grid the tiling tests set their note, along
    /// depth, so that a cut a whole number of points into it is off the grid
    /// by the same fraction.
    ///
    /// A shadow cell is laid on a texel grid anchored at its caster's own
    /// first row, and a piece's caster starts at its cut. A cut off the whole
    /// note's registration blurs the note's far end on a grid that much off,
    /// and at the harness's σ of one texel that end's last pixels come out
    /// differently though the geometry is the same. Cut at 10.3, a note on the
    /// grid differs from its pieces by up to 27 levels on blue in the rows 1.5
    /// and 2.5 points past its far end, and by nothing along the seam (31 cut
    /// at 10.5; 11 at a σ of two points, 6 at three, 2 under the default
    /// spectral shadow). A separate note over just the piece's stretch draws
    /// the piece's far end to the byte, so that is every caster's sub-texel
    /// registration and not tiling, and these tests hold it equal.
    const PIECE_REGISTRATION: f32 = 0.3;

    /// A note cut into pieces draws exactly the note it was cut from, and a
    /// fade takes its body and outline out together along depth.
    ///
    /// The cut sits off the pixel grid on purpose, where two quads meeting on
    /// a shared edge is what decides which of them draws the row — and the
    /// note sits off it by the same fraction, so every piece's shadow cell is
    /// registered as the whole note's is (see [`PIECE_REGISTRATION`]).
    #[test]
    fn pieces_tile_their_note_and_a_fade_runs_along_it() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let whole = RollInstance { center: [128.0, 128.0 + PIECE_REGISTRATION], ..centered_note() };
        // At y = 138.3.
        let cut = 10.0;
        let pieces = vec![
            RollInstance { span: [f32::MIN, cut], ..whole },
            RollInstance { span: [cut, f32::MAX], ..whole },
        ];
        assert!(
            draw(&device, &queue, pieces, bg_color())
                == draw(&device, &queue, vec![whole], bg_color()),
            "two pieces of a note drew something other than the note",
        );

        // Depth runs down y under `TOP`: transparent at the note's top end
        // (y 68), opaque at its bottom (y 188).
        let whole = centered_note();
        let faded = RollInstance { ramp: [-60.0, 60.0], fade: [0.0, 1.0], ..whole };
        let frame = draw(&device, &queue, vec![faded], bg_color());
        let red = |y| pixel(&frame, 128, y)[0];
        assert!(near(pixel(&frame, 128, 69), BG), "top: {:?}", pixel(&frame, 128, 69));
        assert!(near(pixel(&frame, 128, 187), [255, 0, 0, 255]), "{:?}", pixel(&frame, 128, 187));
        assert!(red(100) < red(128) && red(128) < red(160), "the fade is not a ramp");
        assert!(!shadowed(pixel(&frame, 128, 66)), "a faded-out end kept its outline");
        let reversed = RollInstance { ramp: [-60.0, 60.0], fade: [1.0, 0.0], ..whole };
        let frame = draw(&device, &queue, vec![reversed], bg_color());
        assert!(shadowed(pixel(&frame, 128, 66)), "an opaque end lost its outline");
    }

    /// [`centered_note`] tapering from half its width at its top end (y 68)
    #[test]
    fn zero_thickness_has_no_body_shadow_or_bloom() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let absent = RollInstance { taper: [0.0; 4], ..centered_note() };
        let style =
            harmonigraph_scene::ShadowStyle { width: 8.0, depth: 4.0, ..Default::default() };
        let baseline = draw_shadowed(&device, &queue, vec![], style, bg_color(), 1.0);
        let empty = draw_shadowed(&device, &queue, vec![absent], style, bg_color(), 1.0);
        assert_eq!(empty, baseline, "invisible width left ink");
        assert_ne!(
            draw_shadowed(&device, &queue, vec![centered_note()], style, bg_color(), 1.0),
            baseline
        );
        assert_eq!(
            draw_bloomed(&device, &queue, vec![absent], TOP, 2.0, bg_color()),
            draw_bloomed(&device, &queue, vec![], TOP, 2.0, bg_color())
        );
    }

    /// to all of it at its bottom (y 188), in a straight line.
    fn tapered_note() -> RollInstance {
        RollInstance {
            taper_depth: [-60.0, -60.0, 60.0, 60.0],
            taper: [0.5, 0.5, 1.0, 1.0],
            ..centered_note()
        }
    }

    /// A tapered note narrows about its center line, and its outline stands
    /// its own reach off the narrowed flank rather than off the full box.
    #[test]
    fn a_tapered_note_narrows_about_its_center_and_its_outline_follows() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let frame = draw(&device, &queue, vec![tapered_note()], bg_color());
        // Row 70 is 2.5 points into the note, where it is 12.2 points wide:
        // 121.9..134.1 across pitch.
        let at = |x| pixel(&frame, x, 70);
        assert!(near(at(128), [255, 0, 0, 255]), "the center: {:?}", at(128));
        for x in [120, 135] {
            assert!(shadowed(at(x)), "no outline 1.4 points off the flank at {x}: {:?}", at(x));
        }
        // Inside the full box, but past the outline's reach from the taper.
        for x in [117, 138] {
            assert!(near(at(x), BG), "the full box drew at {x}: {:?}", at(x));
        }
        // Near the bottom it is all but the full width again.
        assert!(near(pixel(&frame, 117, 186), [255, 0, 0, 255]), "{:?}", pixel(&frame, 117, 186));
    }

    /// Pieces cut from a tapered note, each told its neighbour's far end,
    /// draw the note they were cut from, with the cut off the pixel grid and
    /// the note off it by the same fraction (see [`PIECE_REGISTRATION`]).
    #[test]
    fn pieces_of_a_tapered_note_tile_it() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let whole = RollInstance { center: [128.0, 128.0 + PIECE_REGISTRATION], ..tapered_note() };
        // At y = 138.3.
        let cut = 10.0;
        let at_cut = 0.5 + 0.5 * (cut + 60.0) / 120.0;
        let pieces = vec![
            RollInstance {
                span: [f32::MIN, cut],
                taper_depth: [-60.0, -60.0, cut, 60.0],
                taper: [0.5, 0.5, at_cut, 1.0],
                ..whole
            },
            RollInstance {
                span: [cut, f32::MAX],
                taper_depth: [-60.0, cut, 60.0, 60.0],
                taper: [0.5, at_cut, 1.0, 1.0],
                ..whole
            },
        ];
        let (cut_up, reference) = (
            draw(&device, &queue, pieces, bg_color()),
            draw(&device, &queue, vec![whole], bg_color()),
        );
        let worst = cut_up.iter().zip(&reference).map(|(a, b)| a.abs_diff(*b)).max();
        assert!(worst <= Some(1), "two pieces of a tapered note differ from it by {worst:?}");
    }

    /// A width that STEPS at a cut keeps the outline on the step's shoulder
    /// and round its corner, though the piece drawing that stretch is the
    /// narrow one: each piece measures its neighbour's side of the step.
    #[test]
    fn a_stepped_width_keeps_its_shoulder_outlined() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // Half width above y 128 (122..134 across pitch), whole below
        // (116..140).
        let pieces = vec![
            RollInstance {
                span: [f32::MIN, 0.0],
                taper_depth: [-60.0, -60.0, 0.0, 0.0],
                taper: [0.5, 0.5, 0.5, 1.0],
                ..centered_note()
            },
            RollInstance {
                span: [0.0, f32::MAX],
                taper_depth: [0.0, 0.0, 60.0, 60.0],
                taper: [0.5, 1.0, 1.0, 1.0],
                ..centered_note()
            },
        ];
        let frame = draw(&device, &queue, pieces, bg_color());
        let red = [255, 0, 0, 255];
        assert!(near(pixel(&frame, 128, 126), red) && near(pixel(&frame, 118, 130), red));
        // 1.5 points above the shoulder, 3.5 off the narrow flank.
        assert!(shadowed(pixel(&frame, 118, 126)), "no shoulder: {:?}", pixel(&frame, 118, 126));
        // 0.7 points from the wide part's corner, diagonally, and 6.5 off the
        // narrow flank.
        assert!(shadowed(pixel(&frame, 115, 127)), "no corner: {:?}", pixel(&frame, 115, 127));
        // 4.9 points from it: past the reach.
        assert!(near(pixel(&frame, 112, 124), BG), "{:?}", pixel(&frame, 112, 124));
    }

    /// A steep glide's outline wraps the note's ENDS as well as its flanks,
    /// out round the far corner, where a sheared note's ink reaches furthest
    /// along pitch.
    ///
    /// The quad the vertex stage grows has to hold that corner's surround, or
    /// the fragment stage never runs there and the outline is cut off along a
    /// straight line at nothing in particular. The box distance has to be the
    /// true one too: read in the sheared box's own coordinates, it calls the
    /// corner's surround further off than the reach that masks it, and cuts
    /// it off the same way.
    ///
    /// How far past the corner the outline runs is the blur's own reach, which
    /// nothing here decides, so that is not asked.
    #[test]
    fn a_steep_glides_outline_wraps_round_its_far_corner() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // 8 points across pitch, 40 along time, bending 3 points of pitch per
        // point of depth, in the harness's three-point outline.
        let steep = RollInstance { half_extent: [4.0, 20.0], shear: 3.0, ..centered_note() };
        let frame = draw(&device, &queue, vec![steep], bg_color());
        // Row 147 samples `local.y = 19.5` — inside the note's box, half a
        // point short of its end. The far flank's ribbon ends at 190.5 there,
        // and the box's far corner stands at (192, 20).
        let at = |x: u32| pixel(&frame, x, 147);
        assert!(
            shadowed(at(193)),
            "the outline is cut off at the note's end ({:?}) — the quad is short of its corner",
            at(193),
        );
    }

    /// A glide a hundredth of a point long paints nothing beyond its own reach
    /// along pitch.
    ///
    /// Per-note tuning makes these: the tuning lands samples after the note-on,
    /// so the note opens on a segment far shorter than a pixel that carries it
    /// part of a semitone — a slope in the thousands. Distance measured in the
    /// sheared box's coordinates called a strip `slope` times the outline's
    /// reach tall "near" it, which drew its outline, and a flickering
    /// antialiased hairline of body color, through the whole pane.
    #[test]
    fn a_sliver_glide_paints_nothing_along_pitch_past_its_reach() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // The center line drifts 5 points each way, so the box spans pitch
        // 119..137 and its outline reaches three points past that.
        let sliver = RollInstance { half_extent: [4.0, 0.005], shear: 1000.0, ..centered_note() };
        // ...and the same sliver tapering, which measures its distance the
        // other way (`tapered_distance`) and owes the same bound.
        let tapered = RollInstance {
            taper_depth: [-0.005, -0.005, 0.005, 0.005],
            taper: [0.5, 0.5, 1.0, 1.0],
            ..sliver
        };
        for note in [sliver, tapered] {
            let frame = draw(&device, &queue, vec![note], bg_color());
            let x_far = (0..112).chain(145..SIZE[0]);
            for (x, y) in x_far.flat_map(|x| (118..138).map(move |y| (x, y))) {
                let got = pixel(&frame, x, y);
                assert!(near(got, BG), "the sliver painted {got:?} at ({x}, {y}), far along pitch");
            }
        }
    }
}

#[cfg(all(test, target_os = "macos", feature = "shader-assets-tools"))]
pub(super) fn asset_catalog(device: &wgpu::Device, format: wgpu::TextureFormat) {
    drop(RollResources::new(device, format));
}

#[cfg(test)]
#[path = "roll_timing.rs"]
mod timing;
