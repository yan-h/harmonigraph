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
//! anti-aliased rounded rects — keyline, black outline, core — each a
//! couple of hundred vertices once its corners and AA ring were subdivided.
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
//! be uploaded once. They are not, deliberately. At 36 bytes per note a busy
//! roll is tens of kilobytes a frame against the megabytes that were the
//! whole problem, so a ring would be optimizing three orders of magnitude
//! below the cost it was built for, and it would have to carry the far-edge
//! trap with it: a note crossing the window's oldest edge is TRUNCATED
//! (as is, at the other end, one whose tail the Gap setting is shaving)
//! there, rewriting its geometry every frame while it leaves (see
//! `panes/spectral/roll.rs`), so any cache has to retire chunks before they
//! reach it.
//! Rebuilding per frame keeps the geometry a pure function of `now` — which
//! is also what keeps the offline render deterministic.

use std::collections::HashMap;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::{create_vertex_buffer, wgpu, EGUI_BLEND};

pub(crate) const ROLL_SRC: &str = include_str!("shaders/roll.wgsl");

/// Entry points the roll shader must provide: the vertex stage, and each of
/// the two layers in each of the two shadings [`create_roll_pipeline`] picks
/// between. Its entry point is assembled from those two words, so a rename in
/// the WGSL is a panic at pipeline creation and nothing sooner.
#[cfg(any(test, feature = "hot-reload"))]
pub(crate) const ROLL_ENTRY_POINTS: &[&str] = &[
    "vs_note",
    "fs_outline_gamma",
    "fs_outline_linear",
    "fs_core_gamma",
    "fs_core_linear",
    "vs_shadow_cell",
    "fs_shadow_coverage",
];

/// One note segment: a solid box in the pane's (pitch, depth) plane, its
/// color, and the outline standing outside every one of its sides.
///
/// Reading outward: [`core`](Self::core), then [`outline`](Self::outline)
/// fading out over [`outline_reach`](Self::outline_reach).
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
    /// How far the outline reaches past the note's edge, in points, and 0 when
    /// it is turned off. It wraps the note: every side and, rounded, every
    /// corner.
    ///
    /// A flat reach, not a distance the shader scales: on a sheared note it is
    /// measured perpendicular to the edge it stands against, so this is its
    /// true thickness at any angle, and it is `vs_note`'s job to grow the quad
    /// by however far along pitch that reaches.
    pub outline_reach: f32,
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
    /// either, or the tip would dissolve inside a hard black ring.
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
    /// note wears no cap there until the lead is dropped and the box shrinks
    /// back to it — at which point the cap arrives whole, in one frame, on a
    /// ribbon that has spent the whole release dissolving.
    ///
    /// Drawn under the lead instead it needs no ramp of its own. The outline
    /// layer goes down before ANY body, so a lead at full opacity covers the
    /// cap and a lead on its way out uncovers it at exactly the rate it goes:
    /// the crossfade is the compositing.
    ///
    /// A REACH rather than an opacity, and that is the whole of what this field
    /// decides. The cap stands OUTSIDE the note's end, in the stretch the lead
    /// was drawn over, and a caller may have somewhere the ribbon's ink may not
    /// go — the spectral pane's now-line is that somewhere, the clip a lead
    /// opens past it belonging to the lead and not to a cap. Shortening the
    /// reach keeps the cap wholly on the near side of such a place; dimming it
    /// instead would still put ink across it, just faintly, which is a weaker
    /// promise for the same arithmetic.
    ///
    /// [`outline_reach`](Self::outline_reach) is the ordinary answer and the
    /// one a caller with nowhere to protect should give — more than that draws
    /// no more, the cap being bounded by the outline it is part of, and an
    /// outline of no reach wears no cap at all. Read only where there IS a
    /// lead: without one the box ends at the note and its cap is the box's own
    /// outline.
    pub cap_reach: f32,
    /// Premultiplied sRGB bytes, straight out of [`egui::Color32`].
    pub core: [u8; 4],
    /// The outline's color where it is solid; the fade takes it out from there.
    ///
    /// What color that is stays the pane's decision — this crate draws the
    /// instance it is handed and invents nothing.
    pub outline: [u8; 4],
}

impl RollInstance {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<RollInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x2, // center
            1 => Float32x2, // half_extent
            2 => Float32,   // shear
            3 => Float32,   // outline_reach
            4 => Float32,  // lead
            5 => Float32,  // lead_fade
            6 => Float32,  // lead_alpha
            7 => Float32,  // cap_reach
            8 => Unorm8x4, // core
            9 => Unorm8x4, // outline
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

/// Draw `instances` into `rect`. `pane_id` must be unique per roll shown in
/// the same frame (each gets its own instance buffer; the pipeline is
/// shared).
///
/// `rect` is the roll's own region rather than the pane's — it is what the
/// bloom's offscreen chain covers, so a rect any larger would spend the halo's
/// resolution on somewhere the roll cannot draw.
///
/// `bloom` is the lattice's own bloom strength, applied to these notes through
/// the lattice's own chain (see [`RollBloom`]). 0 skips it whole.
#[allow(clippy::too_many_arguments)]
pub fn roll_paint_callback(
    rect: egui::Rect,
    instances: Vec<RollInstance>,
    axes: RollAxes,
    bloom: f32,
    shadow: harmonigraph_scene::ShadowStyle,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    shadow_surface_id: u64,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        RollCallback {
            rect,
            instances,
            axes,
            bloom,
            shadow,
            target_format,
            pane_id,
            shadow_surface_id,
        },
    )
}

/// Per-frame, per-pane draw data, built on the UI thread.
struct RollCallback {
    /// The roll's region in points. `paint` is handed this as the callback's
    /// viewport; `prepare` is handed nothing, and needs it to size the bloom
    /// chain, so it rides here too.
    rect: egui::Rect,
    instances: Vec<RollInstance>,
    axes: RollAxes,
    bloom: f32,
    shadow: harmonigraph_scene::ShadowStyle,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    shadow_surface_id: u64,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RollUniforms {
    origin_points: [f32; 2],
    viewport_points: [f32; 2],
    feather: f32,
    _pad: f32,
    pitch_dir: [f32; 2],
    depth_dir: [f32; 2],
    _axis_pad: [f32; 2],
    shadow: [f32; 4],
    shadow_atlas_size: [f32; 2],
    _shadow_pad: [f32; 2],
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
    /// The note pipelines, in the order they are drawn: every outline, then
    /// every body over them. Two passes over one instance buffer — see the
    /// head of roll.wgsl for why the order is the whole point, and what the
    /// second draw costs.
    outline_pipeline: wgpu::RenderPipeline,
    core_pipeline: wgpu::RenderPipeline,
    shadow_cell_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// The bloom chain's four post passes and the one that lays the result
    /// over the notes. The four are the lattice's, out of the same blit.wgsl
    /// and stepped through by the same [`crate::BloomChain::run`], so the
    /// halo the roll grows is the halo the lattice grows: same threshold at
    /// the same resolution, same knee, same kernel, same fractions of the
    /// pane's screen size. Only the target FORMAT is the roll's own, which is
    /// why the pipelines are its own and the chain is not.
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
    panes: HashMap<u64, RollPane>,
    /// Counts `prepare` calls, which is what [`RollPane::last_seen`] is
    /// stamped with — a clock the callback already has, where a frame count
    /// would need one to be plumbed in.
    prepares: u64,
}

/// How many `prepare` calls a pane may go unseen before its buffers and its
/// bloom chain are dropped.
///
/// A pane is prepared once per frame while it is on screen, so with the two
/// rolls that can be live at once this is about a second at 60 fps. Long
/// enough that a pane hidden for a frame keeps everything, short enough that
/// a closed one is not still holding a bloom chain a minute later: three
/// textures the size of the pane it was shown at, which is the reason there is
/// a sweep here at all rather than a map that only ever grows.
const PANE_TTL_PREPARES: u64 = 120;

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
    /// The value of [`RollResources::prepares`] when this pane was last drawn.
    last_seen: u64,
}

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
    /// Half rather than the roll's full size is the one place the two
    /// pictures are not the same thing, so it is worth saying what the
    /// difference is. `fs_bright` only ever SAMPLES at half, so drawing there
    /// skips a resample the lattice pays for; what it costs is that a ribbon
    /// narrower than one of these pixels is measured by the note shader's own
    /// box filter rather than by a bilinear tap over a sharper raster, and on
    /// a shape that lands between two of these pixels the two readings of its
    /// peak differ by up to a factor of two. At the width the spectral pane
    /// floors a ribbon to — 1.5 points, which is 3 device pixels and so one
    /// and a half of these — they are close, and the alternative is a
    /// full-resolution copy of the roll, four times this, for a picture
    /// nothing downstream reads at full resolution.
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

/// Starting size of a pane's instance buffer; it grows by
/// `next_power_of_two` when a frame overflows it. A roll holds a few
/// hundred notes at the spans this pane is used at.
const INITIAL_NOTE_CAPACITY: usize = 512;

impl RollResources {
    fn is_stale(&self, target_format: wgpu::TextureFormat) -> bool {
        #[cfg(feature = "hot-reload")]
        if self.generation != crate::reload::generation() {
            return true;
        }
        self.target_format != target_format
    }

    fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        shadow_layouts: &crate::spectral_shadow::Layouts,
    ) -> Self {
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
        let note_pipeline = |layer| {
            create_roll_pipeline(
                device,
                target_format,
                &layout,
                &shadow_layouts.atlas,
                &shadow_layouts.casters,
                layer,
            )
        };
        // The chain overwrites its whole target, so those three take no blend;
        // the one that lands in the egui pass blends the way every other thing
        // the roll draws does.
        let filter =
            |entry| crate::create_post_pipeline(device, entry, target_format, &filter_layout, None);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("roll_bloom_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        RollResources {
            outline_pipeline: note_pipeline("outline"),
            core_pipeline: note_pipeline("core"),
            shadow_cell_pipeline: create_shadow_cell_pipeline(device, &layout),
            layout,
            bright_pipeline: filter("fs_bright"),
            downsample_pipeline: filter("fs_blit"),
            blur_h_pipeline: filter("fs_blur_h"),
            blur_v_pipeline: filter("fs_blur_v"),
            bloom_pipeline: crate::create_post_pipeline(
                device,
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
            prepares: 0,
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
    format: wgpu::TextureFormat,
}

impl RollPane {
    /// This pane's buffers, made on first sight of its id and stamped with
    /// `prepares` so [`RollPane::evict_unseen`] can tell a live pane from one
    /// whose tab was closed.
    fn get<'a>(
        panes: &'a mut HashMap<u64, RollPane>,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        pane_id: u64,
        prepares: u64,
    ) -> &'a mut RollPane {
        let pane = panes.entry(pane_id).or_insert_with(|| {
            let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("roll_uniforms"),
                size: std::mem::size_of::<RollUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
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
                last_seen: prepares,
            }
        });
        pane.last_seen = prepares;
        pane
    }

    /// Drop every pane that has not been drawn for [`PANE_TTL_PREPARES`].
    ///
    /// A roll's id is its surface (the docked pane, the Render preview), and a
    /// closed tab simply stops calling back — there is no teardown to hang
    /// this on, so the panes still being prepared are the only evidence of
    /// which ones exist. Run from whichever pane IS preparing, so a lone
    /// survivor still clears the others.
    fn evict_unseen(panes: &mut HashMap<u64, RollPane>, prepares: u64) {
        panes.retain(|_, pane| prepares.saturating_sub(pane.last_seen) < PANE_TTL_PREPARES);
    }
}

impl RollBloom {
    /// Build the chain for a roll `size` device pixels across. Half and
    /// quarter of THAT, so the halo is a constant share of the roll's own
    /// screen size — the same rule the lattice's chain follows, which is what
    /// makes one bloom strength mean the same thing in every picture that
    /// grows one.
    fn new(device: &wgpu::Device, shared: &RollBloomShared<'_>, size: [u32; 2]) -> Self {
        let (hw, hh) = (size[0].div_ceil(2).max(1), size[1].div_ceil(2).max(1));
        let notes_view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("roll_bloom_notes"),
                size: wgpu::Extent3d { width: hw, height: hh, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: shared.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        let bloom = crate::BloomChain::new(
            device,
            "roll",
            shared.format,
            shared.filter_layout,
            shared.sampler,
            &notes_view,
            size,
        );

        let notes_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("roll_bloom_notes_uniforms"),
            size: std::mem::size_of::<RollUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let strength_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("roll_bloom_strength"),
            size: std::mem::size_of::<BloomUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
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

/// The note pipeline: instanced quads, blended exactly the way egui blends
/// its own shapes so a note composites over the spectrogram identically to
/// the tessellated version it replaces.
fn create_roll_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::BindGroupLayout,
    shadow: &wgpu::BindGroupLayout,
    casters: &wgpu::BindGroupLayout,
    layer: &str,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("roll_shader"),
        source: wgpu::ShaderSource::Wgsl(crate::roll_source().into()),
    });
    let bind_group_layouts = if layer == "outline" {
        vec![Some(layout), None, Some(shadow), Some(casters)]
    } else {
        vec![Some(layout)]
    };
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("roll_pipeline_layout"),
        bind_group_layouts: &bind_group_layouts,
        ..Default::default()
    });
    // Same fork egui makes, for the same reason: an sRGB-aware target wants
    // linear values and encodes them itself.
    let shade = if target_format.is_srgb() { "linear" } else { "gamma" };
    let entry_point = format!("fs_{layer}_{shade}");
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&format!("roll_{layer}")),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_note"),
            compilation_options: Default::default(),
            buffers: &[RollInstance::LAYOUT],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
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

fn create_shadow_cell_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("roll_shadow_cell_shader"),
        source: wgpu::ShaderSource::Wgsl(crate::roll_source().into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("roll_shadow_cell_pipeline_layout"),
        bind_group_layouts: &[Some(layout)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("roll_shadow_cell"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_shadow_cell"),
            compilation_options: Default::default(),
            buffers: &[RollInstance::LAYOUT, crate::shadow::ShadowBox::BESIDE_ROLL],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_shadow_coverage"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: crate::shadow::ATLAS_FORMAT,
                blend: None,
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

impl CallbackTrait for RollCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let shadow_layouts = crate::spectral_shadow::layouts(device, callback_resources);
        let recreate = callback_resources
            .get::<RollResources>()
            .is_none_or(|r| r.is_stale(self.target_format));
        if recreate {
            callback_resources.insert(RollResources::new(
                device,
                self.target_format,
                &shadow_layouts,
            ));
        }
        let resources: &mut RollResources =
            callback_resources.get_mut().expect("inserted above when missing");
        resources.prepares = resources.prepares.wrapping_add(1);
        let prepares = resources.prepares;

        let ppp = screen_descriptor.pixels_per_point.max(f32::EPSILON);
        let style = self.shadow.clamped();
        let sigma = if style.casts() { crate::shadow::spectral_sigma_points(style) } else { 0.0 };
        let casters: Vec<crate::shadow::Caster> = self
            .instances
            .iter()
            .map(|note| {
                let half_pitch = note.half_extent[0] + note.shear.abs() * note.half_extent[1];
                let half_depth = note.half_extent[1];
                let screen_half = [
                    self.axes.pitch_dir[0].abs() * half_pitch
                        + self.axes.depth_dir[0].abs() * half_depth,
                    self.axes.pitch_dir[1].abs() * half_pitch
                        + self.axes.depth_dir[1].abs() * half_depth,
                ];
                crate::shadow::Caster {
                    rect: [
                        note.center[0] - screen_half[0],
                        note.center[1] - screen_half[1],
                        2.0 * screen_half[0],
                        2.0 * screen_half[1],
                    ],
                    level: 1.0,
                    sigma_points: sigma,
                    kernel: style.kernel,
                    direct_distance: true,
                }
            })
            .collect();
        let uniforms = RollUniforms {
            // The whole surface, which is the viewport `paint` draws into.
            origin_points: [0.0, 0.0],
            viewport_points: [
                screen_descriptor.size_in_pixels[0] as f32 / ppp,
                screen_descriptor.size_in_pixels[1] as f32 / ppp,
            ],
            // One physical pixel, expressed in the points the geometry is
            // in. Derived rather than sampled from the fragment's
            // derivatives so coverage is a pure function of the uniforms,
            // which is what the offline render's byte-for-byte determinism
            // test rests on.
            feather: 1.0 / ppp,
            _pad: 0.0,
            pitch_dir: self.axes.pitch_dir,
            depth_dir: self.axes.depth_dir,
            _axis_pad: [0.0; 2],
            shadow: [
                sigma,
                style.depth,
                if style.kernel.is_distance() { crate::shadow::DISTANCE_KIND } else { 0.0 },
                crate::shadow::spectral_shadow_reach(style),
            ],
            // Patched with the shared target's actual retained allocation by
            // the surface finalizer after every spectral group has arrived.
            shadow_atlas_size: [1.0; 2],
            _shadow_pad: [0.0; 2],
        };

        // The roll's own rect in device pixels, which is what the bloom chain
        // is sized against. Through epaint's own conversion rather than a
        // rounded `width * ppp`, because that conversion is what `paint`
        // stretches the finished halo across: it rounds each EDGE and
        // subtracts, then clamps to the screen, so a rect whose edges round
        // in opposite directions measures a pixel less than its width does,
        // and a rect hanging off the screen measures less again. Sized either
        // way but stretched this way, the halo comes out scaled against the
        // notes it grew from, and slid by whatever the clamp took.
        //
        // A roll thinner than a pixel in either direction has no picture to
        // bloom.
        let viewport = egui::epaint::ViewportInPixels::from_points(
            &self.rect,
            ppp,
            screen_descriptor.size_in_pixels,
        );
        let bloom_size = [viewport.width_px.max(0) as u32, viewport.height_px.max(0) as u32];
        let wants_bloom =
            self.bloom > 0.0 && !self.instances.is_empty() && bloom_size.iter().all(|&d| d > 0);

        let bloom_pass = wants_bloom.then(|| {
            // Half the roll's size for the notes, so this is what one pixel of
            // THAT target measures in points — twice the display's, and the
            // ramp has to follow it or a hairline ribbon comes out at the wrong
            // weight in the halo.
            let half_ppp = ppp * 0.5;
            // The viewport's own edges, back in points: the texture covers
            // exactly the pixels `paint` will lay it over, so the notes in it
            // stand where the notes under it do.
            RollUniforms {
                origin_points: [viewport.left_px as f32 / ppp, viewport.top_px as f32 / ppp],
                viewport_points: [bloom_size[0] as f32 / ppp, bloom_size[1] as f32 / ppp],
                feather: 1.0 / half_ppp,
                ..uniforms
            }
        });

        // Split apart so the pane can be borrowed mutably while the pipelines
        // and layouts beside it are still readable.
        let RollResources {
            core_pipeline,
            layout,
            shadow_cell_pipeline,
            bright_pipeline,
            downsample_pipeline,
            blur_h_pipeline,
            blur_v_pipeline,
            filter_layout,
            bloom_layout,
            sampler,
            target_format,
            panes,
            ..
        } = resources;
        let shared = RollBloomShared {
            notes_layout: layout,
            filter_layout,
            bloom_layout,
            sampler,
            format: *target_format,
        };
        RollPane::evict_unseen(panes, prepares);
        let pane = RollPane::get(panes, device, layout, self.pane_id, prepares);
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
        if pane.bloom.as_ref().is_some_and(|b| b.size != bloom_size) {
            pane.bloom = None;
        }
        if let Some(notes_uniforms) = bloom_pass {
            let bloom =
                pane.bloom.get_or_insert_with(|| RollBloom::new(device, &shared, bloom_size));
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
                // The BODIES alone. The outline is black, and black is the
                // one thing that cannot bloom.
                pass.set_pipeline(core_pipeline);
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
            );
        }

        let submission = crate::spectral_shadow::Submission {
            key: crate::spectral_shadow::ProducerKey::Roll(self.pane_id),
            casters,
            draw: crate::spectral_shadow::CellDraw::Roll {
                pipeline: shadow_cell_pipeline.clone(),
                locals: pane.bind_group.clone(),
                instances: pane.instance_buffer.clone(),
                count: pane.count,
            },
            atlas_uniform: pane.uniform_buffer.clone(),
            atlas_size_offset: std::mem::offset_of!(RollUniforms, shadow_atlas_size) as u64,
        };
        crate::spectral_shadow::register(
            device,
            callback_resources,
            self.shadow_surface_id,
            submission,
        );

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
        // Every outline, then every body over them — one instance buffer, two
        // draws. An outline reaches into its note's surroundings, and those
        // surroundings are other notes; under all of them it can darken the
        // picture and nothing else. See the head of roll.wgsl.
        render_pass.set_bind_group(0, &pane.bind_group, &[]);
        let shadow = crate::spectral_shadow::binding(
            callback_resources,
            self.shadow_surface_id,
            crate::spectral_shadow::ProducerKey::Roll(self.pane_id),
        );
        if let Some(shadow) = &shadow {
            render_pass.set_bind_group(2, shadow.atlas, &[]);
            render_pass.set_bind_group(3, shadow.casters, &[]);
        }
        render_pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
        if shadow.as_ref().is_some_and(|binding| binding.active) {
            render_pass.set_pipeline(&resources.outline_pipeline);
            render_pass.draw(0..4, 0..pane.count);
        }
        render_pass.set_pipeline(&resources.core_pipeline);
        render_pass.draw(0..4, 0..pane.count);

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
            rect,
            instances,
            axes,
            shadow: harmonigraph_scene::ShadowStyle {
                width: 0.5,
                depth: 1.0,
                kernel: harmonigraph_scene::ShadowKernel::Distance,
            },
            bloom,
            target_format: FORMAT,
            pane_id: 0,
            shadow_surface_id: 0,
        };
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(device, queue, &screen, &mut encoder, &mut resources);
        crate::spectral_shadow::finish(device, queue, &screen, &mut encoder, &mut resources, 0);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));

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

    /// A straight note centered in the frame: 24 points thick, 120 long, in a
    /// 4-point black outline with no fade, and square at both ends. Wide enough
    /// that a sample lands well inside the outline, and hard-edged so where it
    /// ends is a place rather than a slope — the two fades have
    /// [`a_fade_takes_the_outline_out_gradually`] and
    /// [`a_lead_fade_takes_the_ribbon_out_toward_its_tip`] to themselves.
    fn centered_note() -> RollInstance {
        RollInstance {
            center: [128.0, 128.0],
            half_extent: [12.0, 60.0],
            shear: 0.0,
            outline_reach: 4.0,
            lead: 0.0,
            lead_fade: 0.0,
            lead_alpha: 0.0,
            // The ordinary answer, and what a caller with nowhere to protect
            // gives: the cap at the note's own end reaches as far as every
            // other edge's outline, and it is the LEAD over it that decides
            // how much of it shows.
            cap_reach: 4.0,
            core: [255, 0, 0, 255],
            outline: [0, 0, 0, 255],
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

    /// The cap belongs to the outline, so it is never wider than one and never
    /// draws where there is no outline at all.
    ///
    /// Both are the same bound read twice. `cap_reach` is the caller's answer
    /// to how much ROOM the cap has, and room is the only thing it decides —
    /// asked for more than the outline itself stands off, the cap would band
    /// the note wider than every other edge of it, and `vs_note` sizes the quad
    /// from `outline_reach` alone, so the surplus is CLIPPED across pitch
    /// rather than merely drawn: a hard vertical edge where a rounded corner
    /// belongs. With the outline off there is no band to be part of, and the
    /// same clamp is what stops a note whose outline is turned off from wearing
    /// one at its own end.
    ///
    /// This is a bound the crate holds rather than a precondition it states,
    /// because the fixtures make the mistake easy: [`centered_note`] carries a
    /// `cap_reach`, so any `..centered_note()` that turns the outline off
    /// inherits one.
    #[test]
    fn a_cap_is_never_wider_than_the_outline_it_belongs_to() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let at = |note: RollInstance, y: u32| {
            let frame = draw(&device, &queue, vec![note], bg_color());
            pixel(&frame, 128, y)
        };
        // Seven and a half points inside the note's own end (y = 108) — outside
        // a 4-point outline's reach, and well inside the 12 this asks for. The
        // lead is spent, so nothing else paints here: the body is gone and the
        // wrap is masked out of the box's interior.
        let greedy = RollInstance { cap_reach: 12.0, ..led_note(40.0, 0.0, 0.0) };
        assert!(
            near(at(greedy, 100), BG),
            "a cap asked for 12 points painted past the 4 its outline stands off: {:?}",
            at(greedy, 100),
        );
        // And it still draws the reach it does have.
        assert!(
            shadowed(at(greedy, 106)),
            "clamping the cap to the outline took the cap with it: {:?}",
            at(greedy, 106),
        );
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
    /// drop out of `across` — and an implementation that simply forgot to shear
    /// the trimmed box would pass every other test in this file.
    ///
    /// Read across the note's own end, twelve points either side of the box's
    /// center: at a shear of 0.6 the end has drifted that far, so the cap is on
    /// one side and nothing is on the other.
    #[test]
    fn a_sheared_notes_cap_follows_its_center_line() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // The band's CENTER rather than two samples of it. The cap is four
        // points wide and the note twenty-four, so a cap displaced by a wrong
        // shear still covers most of the pixels a correct one does — picking
        // two and asserting ink at one and none at the other passes for a
        // mis-shear as readily as for this one. Where the run is centered is
        // the claim itself.
        let center = |shear: f32| {
            let note = RollInstance { shear, ..led_note(40.0, 0.0, 0.0) };
            let frame = draw(&device, &queue, vec![note], bg_color());
            // Row 106 is two points past the note's own end (y = 108), so the
            // cap is the only thing painting in it: the lead is spent, and the
            // wrap that would wrap the box goes with it.
            // Read on BLUE. The background is `[64, 96, 128]`, so its own red
            // channel is already darker than a threshold picked for black —
            // the one channel the two are far apart on is the one to ask.
            let dark: Vec<u32> =
                (0..SIZE[0]).filter(|&x| shadowed(pixel(&frame, x, 106))).collect();
            assert!(!dark.is_empty(), "the cap painted nothing at all at a shear of {shear}");
            f32::from(*dark.first().unwrap() as u16 + *dark.last().unwrap() as u16) * 0.5
        };
        // Pixel centres sit at y = 106.5, which is 21.5 points along depth from
        // the box's own centre, so the note's line has drifted `shear * 21.5`
        // along pitch by the time it gets there.
        for shear in [0.0f32, 0.6, -0.6] {
            let want = 128.0 - shear * 21.5;
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
    /// The two shapes share three of their sides, and beside the note's leading
    /// corners both are looking at the same ink — the wrap dimmed by the lead
    /// it is standing in, the cap not. Added, that overlap comes out darker
    /// than the outline's own color is.
    ///
    /// A GREY outline is what makes this readable, and the reason is worth the
    /// line: black is the degenerate case here. Premultiplied black is
    /// `(0, 0, 0, 1)`, so doubling it doubles an alpha that the target clamps
    /// and leaves a color that was already zero — the sum and the union land on
    /// the same byte, and every other cap test in this file would pass over a
    /// `+`. At half grey the two separate.
    #[test]
    fn the_cap_is_unioned_with_the_wrap_and_not_added_to_it() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // A lead at half opacity, read just outside the note's leading corner:
        // inside the cap's reach of the trimmed box, and inside the wrap's
        // reach of the full one, which the lead has taken to half.
        let note = RollInstance { outline: [128, 128, 128, 255], ..led_note(40.0, 0.0, 0.5) };
        let frame = draw(&device, &queue, vec![note], bg_color());
        let corner = pixel(&frame, 141, 106);
        let cap_only = RollInstance { lead_alpha: 0.0, ..note };
        let cap_frame = draw(&device, &queue, vec![cap_only], bg_color());
        let expected = pixel(&cap_frame, 141, 106);
        assert!(
            near(corner, expected),
            "the overlap {corner:?} differs from the cap alone {expected:?} — the wrap and \
             the cap are being summed rather than unioned",
        );
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
        let mut callbacks = CallbackResources::default();
        let layouts = crate::spectral_shadow::layouts(&device, &mut callbacks);
        let _resources = RollResources::new(&device, FORMAT, &layouts);
    }

    #[test]
    fn either_spectral_geometry_endpoint_allocates_no_roll_shadow_atlas() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        for shadow in [
            harmonigraph_scene::ShadowStyle {
                width: 0.0,
                depth: 1.0,
                kernel: harmonigraph_scene::ShadowKernel::Gaussian,
            },
            harmonigraph_scene::ShadowStyle {
                width: 1.0,
                depth: 0.0,
                kernel: harmonigraph_scene::ShadowKernel::Gaussian,
            },
        ] {
            let cb = RollCallback {
                rect,
                instances: vec![centered_note()],
                axes: TOP,
                shadow,
                bloom: 0.0,
                target_format: FORMAT,
                pane_id: 0,
                shadow_surface_id: 0,
            };
            let mut resources = CallbackResources::default();
            let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
            let mut encoder = device.create_command_encoder(&Default::default());
            cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
            crate::spectral_shadow::finish(
                &device,
                &queue,
                &screen,
                &mut encoder,
                &mut resources,
                0,
            );
            assert!(
                !crate::spectral_shadow::target_allocated(&resources, 0),
                "{shadow:?} allocated a shadow atlas"
            );
        }
    }

    #[test]
    fn both_roll_geometry_kernels_hold_at_editor_and_export_scales() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        for kernel in
            [harmonigraph_scene::ShadowKernel::Distance, harmonigraph_scene::ShadowKernel::Gaussian]
        {
            for ppp in [1.0f32, 1.5, 2.0, 4.0] {
                let physical = (64.0 * ppp).round() as u32;
                let size = [physical.div_ceil(64) * 64, physical];
                let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(64.0, 64.0));
                let shadow = harmonigraph_scene::ShadowStyle { width: 0.5, depth: 1.0, kernel };
                let reach = crate::shadow::spectral_shadow_reach(shadow);
                let note = RollInstance {
                    center: [32.0, 32.0],
                    half_extent: [5.0, 12.0],
                    shear: 0.0,
                    outline_reach: reach,
                    lead: 0.0,
                    lead_fade: 0.0,
                    lead_alpha: 0.0,
                    cap_reach: reach,
                    core: [255, 0, 0, 255],
                    outline: [0, 0, 0, 255],
                };
                let cb = RollCallback {
                    rect,
                    instances: vec![note],
                    axes: TOP,
                    shadow,
                    bloom: 0.0,
                    target_format: FORMAT,
                    pane_id: 0,
                    shadow_surface_id: 0,
                };
                let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: ppp };
                let mut resources = CallbackResources::default();
                let mut encoder = device.create_command_encoder(&Default::default());
                let buffers = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
                crate::spectral_shadow::finish(
                    &device,
                    &queue,
                    &screen,
                    &mut encoder,
                    &mut resources,
                    0,
                );
                queue.submit(buffers.into_iter().chain([encoder.finish()]));
                let texture =
                    render_to_texture(&device, &queue, size, FORMAT, wgpu::Color::WHITE, |pass| {
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
                let frame = readback(&device, &queue, &texture, size);
                let pixel = |x: f32, y: f32| {
                    let i = ((((y * ppp).floor() as u32) * size[0] + (x * ppp).floor() as u32) * 4)
                        as usize;
                    [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
                };
                let shadow_pixel = pixel(25.0, 32.0);
                assert!(
                    shadow_pixel[0] < 245 && shadow_pixel[1] == shadow_pixel[0],
                    "{kernel:?} at {ppp} ppp left no black under-body shadow: {shadow_pixel:?}",
                );
                assert_eq!(pixel(32.0, 32.0), [255, 0, 0, 255], "the body covers {kernel:?}");
                assert_eq!(pixel(12.0, 12.0), [255; 4], "{kernel:?} reached beyond its atlas");
            }
        }
    }

    /// A note is a SOLID rectangle of its own color, with the outline standing
    /// entirely outside it: reading outward from the middle — the note's color
    /// right to its edge, the outline, nothing.
    ///
    /// The outline standing outside is the flood invariant, and the reason it
    /// is read off a distance rather than drawn as a stroke of the note's path:
    /// a centered stroke grows inward exactly as much as outward, and on a
    /// ribbon a few points thick the two long edges meet in the middle and
    /// paint the interior over. Coverage taken at distance 0..4 cannot reach
    /// inside a box whose interior is at negative distance.
    #[test]
    fn a_note_is_solid_and_its_outline_stands_outside_it() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let frame = draw(&device, &queue, vec![centered_note()], bg_color());
        // The note's edge is at x = 140 and the outline runs to 144.
        let at = |x: u32| pixel(&frame, x, 128);
        const RED: [u8; 4] = [255, 0, 0, 255];
        assert!(near(at(128), RED), "the note's middle is not painted: {:?}", at(128));
        assert!(near(at(138), RED), "the fill stops short of the note's edge: {:?}", at(138));
        assert!(shadowed(at(141)), "no outline standing against the note's edge: {:?}", at(141),);
        // Solid nearly all the way out — the last half pixel of the reach is
        // the antialiasing ramp a hard edge still gets — and gone past it.
        assert!(shadowed(at(142)), "the outline is short of its reach: {:?}", at(142));
        assert!(near(at(145), BG), "the outline reaches further than it should: {:?}", at(145));
    }

    /// The outline wraps EVERY side of the note — its ends as much as its
    /// flanks — and turns the corner between them on the radius the box
    /// distance gives it. The note's own body is untouched either way: it is
    /// the shape, and the outline is what is drawn around it.
    ///
    /// The corner is the half of this that a per-edge implementation would
    /// miss. A note is a rectangle, so its outline is the set of points within
    /// the reach of one — which is a rounded rectangle, not four bands butted
    /// together. Sampled on the diagonal past the corner's radius, where four
    /// bands would still be painting.
    #[test]
    fn the_outline_wraps_every_side_of_a_note() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // The note spans +-12 across pitch (x) and +-60 along time (y), so its
        // corner is at (140, 188) and the outline reaches 4 past it.
        let frame = draw(&device, &queue, vec![centered_note()], bg_color());
        let at = |x: u32, y: u32| pixel(&frame, x, y);
        const RED: [u8; 4] = [255, 0, 0, 255];
        assert!(near(at(138, 128), RED), "the note's body went missing: {:?}", at(138, 128));
        assert!(shadowed(at(142, 128)), "no outline along the flank: {:?}", at(142, 128));
        assert!(near(at(128, 186), RED), "the note's body was cut at its end: {:?}", at(128, 186));
        assert!(shadowed(at(128, 190)), "no outline across the end: {:?}", at(128, 190));
        assert!(near(at(128, 193), BG), "the outline runs past its reach: {:?}", at(128, 193));
        // Diagonally off the corner: 2.8 points out is inside the radius, 5.0
        // is outside it, and four bands butted together would paint both.
        assert!(shadowed(at(141, 189)), "the corner is missing: {:?}", at(141, 189));
        assert!(
            near(at(143, 191), BG),
            "the corner is square ({:?}) — the outline is being drawn as bands rather \
             than as a distance",
            at(143, 191),
        );
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
        // Every column of the flank's two-point Distance profile, from the
        // note's own edge (x = 140) to its last covered pixel — the outer one
        // being where a distance field's corner rounding bites first.
        for x in [140, 141] {
            assert!(dark(x, 128), "the outline is missing at the note's middle (x = {x})");
            // Row 187 is the note's last full row along time; the outline is
            // still at its full stand-off there.
            assert!(dark(x, 187), "the outline pulls away before the note ends (x = {x})");
        }
        // And it keeps going past the end, around the corner, rather than being
        // cut there: the flank and the cap are one shape.
        assert!(dark(140, 190), "the outline is cut at the note's end");
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
        // Black on white with no outline: every painted byte is the body's own
        // coverage, read straight off the frame. Under `TOP` the depth axis
        // runs down the screen, so the leading tip of this note is at y = 68
        // and its trailing end at y = 188. A lead of 40 points reaches to
        // y = 108, and the fade is measured back from the tip.
        let bare = |lead: f32, fade: f32| RollInstance {
            outline_reach: 0.0,
            core: [0, 0, 0, 255],
            ..led_note(lead, fade, 1.0)
        };
        let cov = |fade: f32, y: u32| {
            let frame = draw(&device, &queue, vec![bare(40.0, fade)], wgpu::Color::WHITE);
            1.0 - f32::from(pixel(&frame, 128, y)[0]) / 255.0
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

        // The outline's cap goes with it. Two points past the tip is inside a
        // 4-point outline's reach, and that is exactly where a body-only fade
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
        // Black on white, no outline, a square-ended 40-point lead — so the
        // only thing between the tip (y = 68) and the note's own end (y = 108)
        // is the opacity, with no ramp on top of it to unpick.
        let cov = |alpha: f32, y: u32| {
            let note = RollInstance {
                outline_reach: 0.0,
                core: [0, 0, 0, 255],
                ..led_note(40.0, 0.0, alpha)
            };
            let frame = draw(&device, &queue, vec![note], wgpu::Color::WHITE);
            1.0 - f32::from(pixel(&frame, 128, y)[0]) / 255.0
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

    /// The cap at a led note's OWN end is drawn UNDER the lead, so a lead on
    /// its way out uncovers it — it does not arrive once the lead is gone.
    ///
    /// The end of a note carrying a lead is in the middle of its box, where the
    /// outline that wraps the box has nothing to stand against; without a cap
    /// of its own the note wears no dark edge there until the caller drops the
    /// spent lead and the box shrinks back, and then wears the whole of one in
    /// a single frame. On a ribbon that has spent a quarter second dissolving
    /// that reads as the edge popping in at the exact moment the tongue
    /// finishes, which is the one moment nothing should happen.
    ///
    /// Under the lead it needs no ramp: the outline layer is drawn before ANY
    /// body, so what shows through is what the lead has stopped covering, and
    /// the crossfade is the compositing. Read two points inside the note's own
    /// end, where the cap is solid and the lead is over it: the red is the
    /// lead's, the black underneath is the cap's, and the lead's opacity is the
    /// only thing dividing them.
    #[test]
    fn a_fading_lead_uncovers_the_cap_at_the_notes_own_end() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // A square-ended 40-point lead, so nothing between the tip (y = 68) and
        // the note's own end (y = 108) but the opacity. The cap stands OUTSIDE
        // that end, reaching back toward the tip: y = 104 to 108, and y = 106
        // is well inside it at every pixel centre.
        let at = |alpha: f32| {
            let frame = draw(&device, &queue, vec![led_note(40.0, 0.0, alpha)], bg_color());
            pixel(&frame, 128, 106)
        };
        // The lead standing: its own red, and no sign of what is under it.
        let full = at(1.0);
        assert!(near(full, [255, 0, 0, 255]), "the lead is not opaque over its cap: {full:?}");
        // Gone: the cap in full, black against a background that is not.
        assert!(
            shadowed(at(0.0)),
            "the note's own end has no cap under a spent lead: {:?}",
            at(0.0),
        );
        // And in between the two are mixed in the lead's own proportion — the
        // red at half opacity over black, rather than over the background.
        let under = at(0.0);
        let half = at(0.5);
        let expected = [
            ((255u16 + u16::from(under[0])) / 2) as u8,
            (u16::from(under[1]) / 2) as u8,
            (u16::from(under[2]) / 2) as u8,
            255,
        ];
        assert!(
            near(half, expected),
            "a half-gone lead does not sit half over its cap: {half:?}, expected {expected:?}",
        );
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
        // The lead spent, so the cap is uncovered and the frame reads it
        // directly. A 4-point outline told to cap over 2: solid a point inside
        // the note's own end (y = 107), and nothing 3 points inside it
        // (y = 105), where the outline's own reach would still be painting.
        let short = RollInstance { cap_reach: 2.0, ..led_note(40.0, 0.0, 0.0) };
        let frame = draw(&device, &queue, vec![short], bg_color());
        let at = |y: u32| pixel(&frame, 128, y);
        assert!(shadowed(at(107)), "the shortened cap is absent: {:?}", at(107));
        assert!(near(at(105), BG), "the cap reached past the 2 points it was given: {:?}", at(105));
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
        let tap = RollInstance { half_extent: [20.0, 3.0], outline_reach: 0.0, ..centered_note() };
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
        let note = RollInstance { half_extent: [0.75, 60.0], ..centered_note() };
        let frame = draw(&device, &queue, vec![note], bg_color());
        let at = |x: u32| pixel(&frame, x, 128);
        let middle = at(128);
        assert!(middle[0] > 150, "the outline flooded the note's own color: {middle:?}");
        assert!(middle[1] < 32, "something light flooded the note's middle: {middle:?}");
        assert!(shadowed(at(130)), "no outline beside it: {:?}", at(130));
        assert!(near(at(134), BG), "the outline reaches further than it should: {:?}", at(134));
    }

    /// A note two pixels along the depth axis holds its brightness as it
    /// scrolls sub-pixel; one pixel does not. That threshold is what
    /// `panes::spectral::roll::MIN_LENGTH_DEVICE_PX` floors a brief note's length to, and
    /// this is the measurement it is quoted from.
    ///
    /// The `band`/`inside` box filter is one pixel wide, so a shape's coverage
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
        let bare = RollInstance {
            outline_reach: 0.0,
            core: [255, 255, 255, 255],
            outline: [0, 0, 0, 0],
            ..centered_note()
        };
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

    /// The bloom adds light around a note and to the note itself, and adds it
    /// as LIGHT: nothing it touches is occluded, and a strength of 0 leaves the
    /// frame byte for byte the frame with no bloom in it at all.
    ///
    /// The halo is the lattice's, off the same chain — `fs_bright`'s threshold
    /// and knee, the same 9-tap kernel at a quarter of the picture's size — so
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
            outline_reach: 0.0,
            core: [200, 120, 60, 255],
            outline: [0, 0, 0, 0],
            ..centered_note()
        };
        let at = |frame: &[u8], x: u32, y: u32| f32::from(pixel(frame, x, y)[0]);
        let plain = draw_bloomed(&device, &queue, vec![note], TOP, 0.0, wgpu::Color::BLACK);
        let lit = draw_bloomed(&device, &queue, vec![note], TOP, 1.5, wgpu::Color::BLACK);

        // The note's edge is at x = 140; 10 points past it is inside the
        // halo's reach and well outside the note.
        assert_eq!(at(&plain, 150, 128), 0.0, "the unbloomed frame is not black beside the note");
        assert!(at(&lit, 150, 128) > 8.0, "no light beside the note: {}", at(&lit, 150, 128),);
        assert!(
            at(&lit, 128, 128) > at(&plain, 128, 128) + 8.0,
            "the note's own body was not brightened: {} against {}",
            at(&lit, 128, 128),
            at(&plain, 128, 128),
        );
        // Light, not a shape: the halo may never take alpha away from what is
        // under it, and over an opaque frame that means the alpha channel is
        // untouched everywhere.
        for (x, y) in [(128u32, 128u32), (150, 128), (200, 40)] {
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
            roll.panes[&0].bloom.is_some()
        };
        assert!(!bloom_of(0.0), "a strength of 0 built the bloom chain anyway");
        assert!(bloom_of(1.5), "no chain was built at a strength that asks for one");
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
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(device, queue, &screen, &mut encoder, resources);
        crate::spectral_shadow::finish(device, queue, &screen, &mut encoder, resources, 0);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
    }

    /// A roll callback over `rect`, drawing one note with the bloom on.
    fn bloomed_callback(rect: egui::Rect, pane_id: u64) -> RollCallback {
        RollCallback {
            rect,
            instances: vec![centered_note()],
            axes: TOP,
            shadow: harmonigraph_scene::ShadowStyle { width: 0.0, ..Default::default() },
            bloom: 1.5,
            target_format: FORMAT,
            pane_id,
            shadow_surface_id: 0,
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
        let bloom = roll.panes[&0].bloom.as_ref().expect("a strength of 1.5 asks for a chain");
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
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        let (docked, preview) = (bloomed_callback(rect, 0), bloomed_callback(rect, 1));
        let mut resources = CallbackResources::default();
        prepare_once(&device, &queue, &mut resources, 1.0, &docked);
        prepare_once(&device, &queue, &mut resources, 1.0, &preview);
        let live = |resources: &CallbackResources| {
            let roll: &RollResources = resources.get().expect("prepare inserts its resources");
            let mut ids: Vec<u64> = roll.panes.keys().copied().collect();
            ids.sort_unstable();
            ids
        };
        assert_eq!(live(&resources), vec![0, 1], "both rolls should be holding buffers");

        // The preview's tab closes: the docked roll goes on drawing alone.
        for _ in 0..PANE_TTL_PREPARES {
            prepare_once(&device, &queue, &mut resources, 1.0, &docked);
        }
        assert_eq!(live(&resources), vec![0], "the closed pane is still holding its chain");
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
            outline_reach: 0.0,
            core: [120, 120, 120, 255],
            outline: [0, 0, 0, 0],
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
    /// byte. A sign dropped anywhere between the uniform and `across` would
    /// show as the mirror failing, most likely by the keyline landing on the
    /// wrong flank.
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

    /// An outline never covers another note's BODY.
    ///
    /// The outline is opaque where it meets its own note — it has to be, or it
    /// takes its color from the spectrogram cell behind it and washes out
    /// against the bright end of a palette. Composited with its own note it
    /// therefore lands, at full strength, on whatever it reaches into; and
    /// along time what it reaches into is the next note, since repeats of one
    /// key butt together there. The later note blanked the tail of the earlier.
    ///
    /// Every outline is drawn before every body, so an outline can darken the
    /// picture and never another note. Two notes butted exactly, in colors
    /// that can be told apart, and read on both sides of the join: whichever
    /// note is drawn first, the other's outline is behind it.
    #[test]
    fn an_outline_never_covers_another_notes_body() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // Depth runs down y under `TOP`. The two boxes meet exactly at y=128,
        // and a 4-point outline reaches 4 points into each other's.
        let butted = |center, core| RollInstance {
            center: [128.0, center],
            half_extent: [12.0, 30.0],
            outline_reach: 4.0,
            core,
            outline: [0, 0, 0, 255],
            ..centered_note()
        };
        const RED: [u8; 4] = [255, 0, 0, 255];
        const GREEN: [u8; 4] = [0, 255, 0, 255];
        let early = butted(98.0, RED); // y 68..128
        let late = butted(158.0, GREEN); // y 128..188
        let frame = draw(&device, &queue, vec![early, late], bg_color());

        // Two points inside the earlier note's tail, which is two points inside
        // the later note's outline. This is the pixel that went black.
        assert!(
            near(pixel(&frame, 128, 126), RED),
            "the later note's outline blanked the earlier note's tail: {:?}",
            pixel(&frame, 128, 126),
        );
        // And the same join from the other side: paint order is not what is
        // deciding it.
        assert!(
            near(pixel(&frame, 128, 130), GREEN),
            "the earlier note's outline blanked the later note's head: {:?}",
            pixel(&frame, 128, 130),
        );
        // The outline is still drawn — over the pane, where no note is.
        let outside = pixel(&frame, 128, 66);
        assert!(
            shadowed(outside),
            "the outline stopped painting at all outside the notes: {outside:?}",
        );
    }

    /// A glide's outline keeps its thickness instead of thinning with the
    /// angle.
    ///
    /// The shear that turns the box into the parallelogram a bent note
    /// follows also stretches distances along the pitch axis, so the outline
    /// has to be measured perpendicular to the edge it stands against — that is
    /// the division by the shear's length. Without it a 45-degree glide's
    /// outline comes out 1/sqrt(2) as thick as the same note held.
    ///
    /// Measured as total ink across one scanline, which for a slanted band is
    /// `sqrt(1 + slope^2)` times its true thickness. A hard-edged 3-point
    /// outline lays down 2.5 points of ink per flank — the reach less the half
    /// pixel its antialiasing ramp spends ending — so 5.0 points held and 7.07
    /// at 45 degrees. An unnormalized distance would read 5.0 for both.
    #[test]
    fn a_glides_outline_keeps_its_thickness_instead_of_thinning_with_the_angle() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // Only the outline paints, and in black: over a white background its
        // coverage is then exactly `1 - r/255` in every pixel it touched.
        let bare = RollInstance {
            outline_reach: 3.0,
            core: [0, 0, 0, 0],
            outline: [0, 0, 0, 255],
            ..centered_note()
        };
        let white = wgpu::Color::WHITE;
        let ink = |note: RollInstance| {
            let frame = draw(&device, &queue, vec![note], white);
            (0..SIZE[0]).map(|x| 1.0 - f32::from(pixel(&frame, x, 128)[0]) / 255.0).sum::<f32>()
        };

        let held = ink(bare);
        assert!(held > 1.0, "a held note's two flanks painted no measurable shadow");

        let glide = ink(RollInstance { shear: 1.0, ..bare });
        let expected = held * f32::sqrt(2.0);
        assert!(
            (glide - expected).abs() < 0.5,
            "a 45-degree glide's outline measured {glide} across the scanline, not \
             {expected} — it thins with the angle instead of keeping its thickness",
        );
    }

    /// A steep glide's outline stands its full width off the note at the note's
    /// ENDS, where a sheared note's ink reaches furthest along pitch.
    ///
    /// Two things have to be right for the ink to be there, and either alone
    /// would leave it black at the flank and short of where it belongs. The
    /// outline is measured perpendicular to the edge it stands against (the
    /// test above), so along PITCH it covers `sqrt(1+slope^2)` times its own
    /// reach; and the quad the vertex stage grows has to hold all of that, or
    /// the fragment stage never runs where the ink was owed and the outline is
    /// cut off along a straight line at nothing in particular.
    ///
    /// Sampled at the note's own last row, since both shortfalls grow with
    /// `|local.y|` and are zero at its middle — which is where a scanline
    /// measurement like the one above would look.
    #[test]
    fn a_steep_glides_outline_still_stands_off_at_the_notes_ends() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // 8 points across pitch, 40 along time, bending 3 points of pitch per
        // point of depth: steep enough that `skew` is 3.16, so the outline
        // stands 12.6 points out along pitch rather than 4.
        let steep = RollInstance { half_extent: [4.0, 20.0], shear: 3.0, ..centered_note() };
        let frame = draw(&device, &queue, vec![steep], bg_color());
        // Row 147 samples `local.y = 19.5` — inside the note's box, half a
        // point short of its end. There the note's center line has drifted 58.5
        // points, so the far flank's ribbon ends at 190.5 and its outline runs
        // out to 203.2 — where an outline that kept its width along pitch
        // rather than perpendicular to the edge would stop at 194.5.
        let at = |x: u32| pixel(&frame, x, 147);
        assert!(shadowed(at(193)), "the outline is missing at the note's end: {:?}", at(193),);
        assert!(
            shadowed(at(199)),
            "the outline is cut off at the note's end ({:?}) — it thins with the angle, \
             or the quad was grown by its flat reach rather than its sheared one",
            at(199),
        );
        assert!(near(at(206), BG), "the outline reaches further than it should: {:?}", at(206));
    }
}
