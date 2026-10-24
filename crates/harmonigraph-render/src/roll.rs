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
//! solid body and the surround standing off it are both read off that distance,
//! so the surround costs a compare rather than a second shape — and its fade, which
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

use crate::uniforms::{uniform_group, Float2};

use std::collections::HashMap;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::{create_vertex_buffer, wgpu, EGUI_BLEND};

pub(crate) const ROLL_SRC: &str = include_str!("shaders/roll.wgsl");

/// Entry points the roll shader must provide: the two vertex stages (the
/// surround's quad and the bodies'), the surround's pass, the bodies'
/// holdout, and the body layer in each of the two shadings
/// [`create_core_pipeline`] picks between. Its entry point is assembled from a
/// shading's name, so a rename in the WGSL is a panic at pipeline creation and
/// nothing sooner.
#[cfg(any(test, feature = "hot-reload"))]
pub(crate) const ROLL_ENTRY_POINTS: &[&str] = &[
    "vs_note_cut",
    "vs_note_body",
    "fs_note_surround",
    "fs_core_gamma",
    "fs_core_linear",
    "fs_body_holdout",
];

/// One note segment: a solid box in the pane's (pitch, depth) plane, its
/// color, and the backdrop cut standing outside every one of its sides.
///
/// Reading outward: [`core`](Self::core), then the cut fading out over its
/// reach, which is the whole roll's and rides in the uniform rather than here,
/// filled with [`aura`](Self::aura)'s color.
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
    /// Both layers ride it: a lead that ends in a fade wears no surround cap
    /// either, or the tip would dissolve inside a hard ring of cut.
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
    /// How far the surround's cap at the NOTE's own leading end reaches, in
    /// points — the cap standing where the note stops and the
    /// [`lead`](Self::lead) carries on.
    ///
    /// The note's own end is INTERIOR to a box that carries a lead, and a box
    /// has no edge in its middle for a surround to wrap, so without this the
    /// note wears no cap there for as long as its box keeps the lead.
    ///
    /// Drawn in the surround layer, which goes down before ANY body, so a lead
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
    /// The cut's own reach is the ordinary answer and the one a caller with
    /// nowhere to protect should give — more than that draws no more, the cap
    /// being bounded by the surround it is part of, and a surround of no reach
    /// wears no cap at all. Read only where there IS a lead: without one
    /// the box ends at the note and its cap is the box's own surround.
    pub cap_reach: f32,
    /// Premultiplied sRGB bytes, straight out of [`egui::Color32`].
    pub core: [u8; 4],
    /// The surround: the color its cut fills with, as straight (not
    /// premultiplied) sRGB bytes, and in alpha how strongly the note cuts where
    /// its cut is whole; the cut's fade and the note's take it out from there,
    /// and an alpha of 0 cuts nothing. Every note's cut merges into one target,
    /// and the strongest keeps its color (see the head of roll.wgsl).
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
    /// tile the box. So the surround, the lead and the cap are the segment's,
    /// and no piece has an end of its own for a surround to wrap — exactly,
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
    /// and the surround together. `[1.0, 1.0]` is unfaded.
    pub fade: [f32; 2],
    /// Four depth offsets, ascending, that [`taper`](Self::taper) is given at.
    ///
    /// The middle two are this piece's own ends and the outer two its
    /// neighbouring pieces' far ends, or a step's other side where one stands
    /// at a cut, so the surround near a cut is measured against the shape the
    /// piece beside it draws. Beyond them a piece cannot see, so a neighbour
    /// shorter than the surround's reach can leave the surround a step at the
    /// cut. Read only while [`taper`](Self::taper) moves.
    pub taper_depth: [f32; 4],
    /// The ribbon's width at each of the four [`taper_depth`](Self::taper_depth)
    /// depths, as a share of its full [`half_extent`](Self::half_extent)
    /// across pitch: straight lines between them, held past both ends, and
    /// growing about the center line. The body, the surround standing off it
    /// and the bloom all follow the tapered shape, and its distance
    /// is still the true one. [`UNTAPERED`](Self::UNTAPERED) for a ribbon
    /// at full width all along.
    ///
    /// At most 1, since the quad and the culls are sized from the box: a caller whose ribbon swells hands over a box grown to
    /// its widest point and the widths as shares of that.
    pub taper: [f32; 4],
    /// Dense-repeat ornament: take-time phase at the center (modulo 6 points),
    /// then composition point scale. A zero scale leaves the body plain.
    /// Only the fill color changes; coverage, edges and the surround stay solid.
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

/// Draw `instances` into `rect`. `pane_id` is unique per roll drawing in the
/// same frame and the same across frames for one live copy: each id keeps its
/// own instance buffer, bloom chain and cut targets (the pipeline is shared),
/// for as long as the renderer does — see [`RollResources::panes`].
///
/// `rect` is the roll's own region rather than the pane's — it is what the
/// bloom's offscreen chain and the surround's targets cover, so a rect any
/// larger would spend their resolution on somewhere the roll cannot draw. The
/// surrounds stop at it; the caller's clip should too.
///
/// `bloom` is the strength the bloom runs at, applied to these notes through
/// the lattice's own chain (see [`RollBloom`]). 0 skips it whole.
///
/// The instances from `clipped_start` onward are clipped to `tail_rect`
/// while keeping one global surround-before-body paint order. This is for a
/// roll whose leading extensions occupy a stricter region than its scrolling
/// history.
#[allow(clippy::too_many_arguments)]
pub fn roll_paint_callback(
    rect: egui::Rect,
    pane_size: egui::Vec2,
    instances: Vec<RollInstance>,
    clipped_start: usize,
    tail_rect: egui::Rect,
    axes: RollAxes,
    bloom: f32,
    cut: crate::SurroundCut,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    point_scale: f32,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        RollCallback {
            point_scale,
            rect,
            pane_size,
            instances,
            clipped_tail: (clipped_start as u32, tail_rect),
            axes,
            bloom,
            cut,
            target_format,
            pane_id,
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
    /// Kept in the same callback so every surround still precedes every body.
    /// A start at or past the instance count means no tail.
    clipped_tail: (u32, egui::Rect),
    axes: RollAxes,
    bloom: f32,
    cut: crate::SurroundCut,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
}

uniform_group! {
struct RollUniforms {
    /// The backdrop cut, as [`crate::SurroundCut::uniform`] hands it over.
    surround: crate::surround::SurroundUniform,
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
    _tail_pad: Float2,
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

/// The bloom's strength, alone in its own buffer (`AddUniforms` in blit.wgsl).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BloomUniforms {
    /// Strength in x; the rest is the 16 bytes a uniform block takes.
    strength: [f32; 4],
}

/// GPU objects cached across frames in egui-wgpu's `CallbackResources`.
struct RollResources {
    /// The note layers, in the order they are drawn: every surround, then
    /// every body over them — see the head of roll.wgsl for why the order is
    /// the whole point, and what the second draw costs. The surrounds go into a
    /// [`RollHoldout`]'s targets (`surround`), which are laid over the picture
    /// in one draw (`surround_composite`).
    surround: wgpu::RenderPipeline,
    surround_composite: crate::surround::SurroundComposite,
    core_pipeline: wgpu::RenderPipeline,
    /// The bodies' coverage, unioned into a [`RollHoldout`] before either
    /// layer is drawn, and the layout the surround reads it through.
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
    /// Every roll this renderer has drawn, by pane id, and never swept, like
    /// the lattice's panes. An id is a surface — the docked pane, the
    /// Render preview, one placement of an export — so the map holds as many
    /// entries as there are surfaces, and it goes with the `CallbackResources`
    /// it lives in when the editor window closes or the export ends. Until
    /// then a hidden tab or a closed preview keeps its buffers, bloom chain
    /// and cut targets, so switching back to it rebuilds nothing, and a rest
    /// that adds no callback loses nothing either.
    panes: HashMap<u64, RollPane>,
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
    /// Built while the cut draws, and rebuilt when the rect resizes.
    holdout: Option<RollHoldout>,
}

/// The union of every note's body coverage over the roll's rect, one texel
/// per device pixel, which the surround takes off every cut (#1293), and the
/// surround's own targets over the same rect.
///
/// A cut drawn under every body still shows through any body drawn at less
/// than full opacity, and blending cannot take it back out afterwards: by the
/// time a body lands, the frame under it no longer says how much of it was
/// cut. So the bodies' coverage is drawn first, offscreen, and the cuts read it
/// where they are drawn. Full resolution because it meets the body edges pixel
/// for pixel; half floats because the surround's own edge takes the larger of
/// this and its note's exact fill, and eight bits of coverage would shift
/// those edges by a rounding step.
struct RollHoldout {
    view: wgpu::TextureView,
    /// Notes mapped into that texture rather than onto the surface.
    notes_uniform: wgpu::Buffer,
    notes_bind_group: wgpu::BindGroup,
    /// The texture, as the surround pass's group 1.
    read_bind_group: wgpu::BindGroup,
    /// Every note's cut and fill color over the same rect: drawn in `prepare`
    /// after the holdout it reads, and laid over the picture by `paint`. One
    /// target rather than a cut blended per note, so where two notes' cuts meet
    /// the picture is cut as the two together say and not twice over.
    surround: crate::surround::SurroundTargets,
    /// The roll's size in device pixels this was built for.
    size: [u32; 2],
}

/// The instances from `start` on, drawn only inside `scissor` (in the roll's
/// own pixels), and not at all when that is `None`: the clipped tail, as a
/// texture over the roll's rect sees it. A `start` at or past the instance
/// count means no tail.
#[derive(Clone, Copy)]
struct TailClip {
    start: u32,
    scissor: Option<[u32; 4]>,
}

/// Every instance into a texture over the roll's rect, the clipped tail only
/// as far as `paint` lets it draw. A tail starting at or past `count` is no
/// tail, and everything draws.
fn draw_clipped(pass: &mut wgpu::RenderPass<'_>, count: u32, tail: TailClip) {
    if tail.start >= count {
        pass.draw(0..4, 0..count);
        return;
    }
    pass.draw(0..4, 0..tail.start);
    if let Some([left, top, width, height]) = tail.scissor {
        pass.set_scissor_rect(left, top, width, height);
        pass.draw(0..4, tail.start..count);
    }
}

/// The holdout's format: see [`RollHoldout`] for why half floats.
const HOLDOUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

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
            // The surround reaches past its note by the cut's reach, and reads
            // the holdout.
            surround: crate::surround::surround_pipeline(
                device,
                &shader,
                &[Some(&layout), Some(&holdout_layout)],
                crate::surround::SurroundStages {
                    label: "roll_surround",
                    vertex: "vs_note_cut",
                    fragment: "fs_note_surround",
                    buffers: &[RollInstance::LAYOUT],
                },
            ),
            surround_composite: crate::surround::SurroundComposite::new(device, target_format),
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
            panes: HashMap::new(),
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
    /// This pane's buffers, made on first sight of its id.
    fn get<'a>(
        panes: &'a mut HashMap<u64, RollPane>,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        pane_id: u64,
    ) -> &'a mut RollPane {
        panes.entry(pane_id).or_insert_with(|| {
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
        composite: &crate::surround::SurroundComposite,
        size: [u32; 2],
    ) -> Self {
        let view = crate::gpu_memory::texture(
            device,
            &wgpu::TextureDescriptor {
                label: Some("roll_holdout"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HOLDOUT_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
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
            surround: crate::surround::SurroundTargets::new(device, composite, size),
            view,
            notes_uniform,
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
/// is its instance's `fade`, which takes its body and its surround out together.
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
        let surround = self.cut.uniform(self.point_scale);
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
            surround,
            _tail_pad: Float2([0.0; 2]),
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
        // Only a roll whose cut draws draws the surround that reads it.
        let wants_holdout = surround.draws() && has_area;

        // Split apart so the pane can be borrowed mutably while the pipelines
        // and layouts beside it are still readable.
        let RollResources {
            light_pipeline,
            layout,
            surround: surround_pipeline,
            surround_composite,
            holdout_pipeline,
            holdout_layout,
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
        let pane = RollPane::get(panes, device, layout, self.pane_id);
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
                // The BODIES alone. The surround stands behind them rather
                // than being light they give off, and it is dark at its
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

        // Kept while the cut is on, and dropped with it, since only the
        // surround draws into it and reads it. Kept through a frame with no
        // notes, as the bloom is: the pane calls back through every rest, and
        // `paint` declines a frame with nothing in it before it reaches what
        // these still hold. The surround's own targets are drawn straight
        // after, reading the holdout just finished.
        if !surround.draws() || pane.holdout.as_ref().is_some_and(|h| h.size != roll_size) {
            pane.holdout = None;
        }
        if wants_holdout {
            let holdout = pane.holdout.get_or_insert_with(|| {
                RollHoldout::new(device, layout, holdout_layout, surround_composite, roll_size)
            });
            queue.write_buffer(&holdout.notes_uniform, 0, bytemuck::bytes_of(&roll_uniforms));
            // A clipped tail holds out, and cuts, only where `paint` lets it
            // draw.
            let tail = {
                let (start, tail_rect) = self.clipped_tail;
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
            };
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
            draw_clipped(&mut pass, pane.count, tail);
            drop(pass);

            holdout.surround.draw(egui_encoder, surround_pipeline, |pass| {
                pass.set_bind_group(0, &holdout.notes_bind_group, &[]);
                pass.set_bind_group(1, &holdout.read_bind_group, &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                draw_clipped(pass, pane.count, tail);
            });
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
        let Some(pane) = resources.panes.get(&self.pane_id) else {
            return;
        };
        if pane.count == 0 {
            return;
        }
        // Every surround, then every body over them. A surround reaches into
        // its note's surroundings, and those surroundings are other notes;
        // under all of them it can cut the picture and nothing else. See the
        // head of roll.wgsl.
        //
        // The surrounds are the targets `prepare` drew, laid over the roll's
        // rect as the halo is below, inside egui's own clip. `prepare` keeps a
        // holdout while the cut is on and draws its targets on every frame
        // with notes in it, which past the `count` check above is this one.
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
            holdout.surround.paint(render_pass, &resources.surround_composite);
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
        let (start, tail_rect) = self.clipped_tail;
        let start = start.min(pane.count);
        if start > 0 && set_clip(render_pass, info.clip_rect) {
            render_pass.draw(0..4, 0..start);
        }
        if start < pane.count && set_clip(render_pass, tail_rect) {
            render_pass.draw(0..4, start..pane.count);
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
mod tests;

#[cfg(all(test, target_os = "macos", feature = "shader-assets-tools"))]
pub(super) fn asset_catalog(device: &wgpu::Device, format: wgpu::TextureFormat) {
    drop(RollResources::new(device, format));
}

#[cfg(test)]
#[path = "roll_timing.rs"]
mod timing;
