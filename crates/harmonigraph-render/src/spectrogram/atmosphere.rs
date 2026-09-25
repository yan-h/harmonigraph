//! A small scalar image diffuses the heatmap before its single palette lookup.
//! Targets belong to one pane and are keyed only on their sizes — the light
//! field's, the cloud tone's and the walk tile's ([`Allocation`]); source pixels
//! and uniforms are refreshed every draw, including paused zooms and palette
//! edits. The TILE is the one thing here not refilled every draw, and
//! [`TileKey`] is what decides when it is.

use super::{create_spectrogram_pipeline, SpectrogramUniforms, SpectrogramVertex};
use crate::{create_vertex_buffer, wgpu};

pub(super) const SOURCE: &str = include_str!("../shaders/spectral_atmosphere.wgsl");
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
/// The tile's own format. Four channels because the mosaic's walk produces two
/// vectors and the wash's produces five numbers over two targets; half floats
/// because what is stored is a cell offset of order one, where the eleven-bit
/// mantissa is a thousandth of a cell.
const TILE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Fixed cloud sampling: native on 1x/2x displays, with a 40-cell repeat.
/// Forty closes every hashed lattice and makes repetition less frequent than
/// twenty at the same steady-frame cost. Tests and the timing probe override
/// it through a callback resource, never persisted settings; the period is
/// never 0, because the composite has no live walk to fall back to (#1100).
#[derive(Clone, Copy)]
pub(super) struct CloudSampling {
    pub tile_cells: u32,
    pub pixel_points: f32,
}

impl Default for CloudSampling {
    fn default() -> Self {
        Self { tile_cells: 40, pixel_points: 0.5 }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SpectrogramAtmosphere {
    pub settings: harmonigraph_scene::SpectralAtmosphere,
    /// The whole spectrogram region, including history that has no data yet.
    pub region: egui::Rect,
    /// Axis used to preserve the reduced source image's pitch footprint.
    pub pitch_vertical: bool,
    /// Physical display points per cent and per millisecond, before clipping.
    pub points_per_cent: f32,
    pub points_per_ms: f32,
    /// Display points one SLAB of the run being drawn spans along the time
    /// axis — the resolution the DATA has there, which is what
    /// `blur_time_step` bounds the light field against.
    ///
    /// The slab width of the run actually on screen, held rung and all, rather
    /// than one re-derived from the window: a held rung draws wider slabs than
    /// the window alone would ask for, and bounding the field against the
    /// narrower number would leave it finer than the picture it is drawn from.
    ///
    /// 0 where the caller has no run to say it from, which reads as no bound —
    /// the same answer the dial's own zero gives, since the two are multiplied.
    pub points_per_slab: f32,
    /// The pane's clock, which drives the cloud drift. Offline it is the
    /// frame's time, so a render is deterministic.
    pub now: f64,
}

/// Cloud-space sampling offset for a texture travelling at a constant visible
/// screen direction. The shader samples `screen + drift`, so the sampling
/// offset moves opposite the texture itself.
fn cloud_drift(settings: harmonigraph_scene::SpectralAtmosphere, now: f64) -> [f32; 2] {
    const UNITS_PER_SECOND: f64 = 0.047_169_905_660_283_02;
    const INITIAL_PHASE: [f64; 2] = [0.0, 0.6];
    let distance = now * f64::from(settings.cloud_speed) * UNITS_PER_SECOND;
    let direction = f64::from(settings.cloud_direction).to_radians();
    let (sin, cos) = direction.sin_cos();
    [(INITIAL_PHASE[0] - distance * cos) as f32, (INITIAL_PHASE[1] - distance * sin) as f32]
}

/// Bound filter work by reducing each axis only as its musical radius grows,
/// and — where `Blur time step` asks — the time axis by the DATA's own
/// resolution as well. The scalar source averages its whole footprint before
/// these Gaussian passes. The allocation key is this size alone; no measurement
/// cache is invalidated.
///
/// **What decides this size, both directions.** The pane's pixels and `ppp`,
/// the two softnesses through `points_per_cent`/`points_per_ms`, whether the
/// field is drawn at all, and now
/// [`harmonigraph_scene::SpectralAtmosphere::blur_time_step`] with
/// [`SpectrogramAtmosphere::points_per_slab`]. Nothing else reaches the
/// picture's needed resolution, so nothing else may serve a stale one. The
/// input that CHURNS is the slab width: `points_per_slab` moves continuously
/// through a Span drag, since the window moves while the rung holds. That does
/// not reallocate per frame, because a capped axis is a REDUCED axis and
/// [`retained_size`] gives those a 10% band — a drag refreshes pixels until it
/// has moved the requested size a tenth, and a rung crossing (the slab width
/// doubling) costs exactly one reallocation.
pub(super) fn source_size(
    pixels: [u32; 2],
    ppp: f32,
    atmosphere: SpectrogramAtmosphere,
) -> [u32; 2] {
    let settings = atmosphere.settings.sanitized();
    // Terraces alone read the level under the pixel, so nothing is ever drawn
    // into this target and it costs one texel. A cloud at zero softness is the
    // other case and falls through: both radii are zero, so both axes come out
    // at full resolution and the filter's sub-texel arm copies the measured
    // picture through for the cloud to read.
    if !settings.effects().light() {
        return [1, 1];
    }
    let pitch = settings.pitch_softness * atmosphere.points_per_cent * ppp;
    let time = settings.time_softness * atmosphere.points_per_ms * ppp;
    let sigma = if atmosphere.pitch_vertical { [time, pitch] } else { [pitch, time] };
    // The same axis order `sigma` is built in: time leads when pitch is the
    // pane's Y. The dial reaches this axis and no other.
    let time_axis = usize::from(!atmosphere.pitch_vertical);
    // Device pixels the dial allows per source texel on it. Not finite or not
    // positive is no bound at all, which covers the dial's own zero, a caller
    // with no run to measure, and any nonsense either could carry.
    let per_texel = settings.blur_time_step * atmosphere.points_per_slab * ppp;
    let bounded = per_texel.is_finite() && per_texel > 0.0;
    std::array::from_fn(|axis| {
        let base = pixels[axis];
        let mut texels = pixels[axis] as f32 / (sigma[axis] * 0.5).max(1.0);
        if axis == time_axis && bounded {
            texels = texels.min(pixels[axis] as f32 / per_texel);
        }
        (texels.ceil() as u32).max(8).min(base)
    })
}

/// Small zoom and Span changes refresh pixels, not GPU allocations. Keep the
/// retained resolution within 10% of the requested one to bound the change
/// in kernel sampling density and truncation as the musical radius moves.
/// Full-resolution axes (including zero softness) must match the viewport.
pub(super) fn retained_size(
    requested: [u32; 2],
    pixels: [u32; 2],
    retained: Option<[u32; 2]>,
) -> [u32; 2] {
    retained
        .filter(|size| {
            (0..2).all(|axis| {
                let held = u64::from(size[axis]);
                let wanted = u64::from(requested[axis]);
                held <= u64::from(pixels[axis])
                    && (requested[axis] != pixels[axis] || held == wanted)
                    && held * 10 >= wanted * 9
                    && held * 10 <= wanted * 11
            })
        })
        .unwrap_or(requested)
}

/// How big the cloud's scalar tone target is, or `None` where the layer is
/// drawn natively under every pixel of the composite.
///
/// Native is both the no-cloud case and sample spacing at or under one
/// DEVICE pixel — the fixed half point on a Retina pane and on a plain one —
/// where a reduced target would be the pane's own resolution or larger and the
/// extra pass would buy nothing. Above that each axis is divided by the same
/// number of pixels per sample, so the walk's cost falls with its square.
pub(super) fn tone_size(
    pixels: [u32; 2],
    ppp: f32,
    atmosphere: SpectrogramAtmosphere,
    pixel_points: f32,
) -> Option<[u32; 2]> {
    let settings = atmosphere.settings.sanitized();
    let pixel = pixel_points * ppp;
    if !settings.effects().cloud || pixel <= 1.0 {
        return None;
    }
    Some(std::array::from_fn(|axis| ((pixels[axis] as f32 / pixel).ceil() as u32).max(1)))
}

/// The shader's own `CLOUD_UNITS`, `SCALE_CELLS` and `WASH_CELLS`: how many
/// cloud units cross the pane's height and how many cells of each texture cross
/// one unit at a size of 1x. Nothing else here needs to know what a cell is —
/// the tile does, because how fine it has to be is how fine the pane draws one.
///
/// Held against the shipped shader text by
/// `the_tile_is_as_fine_as_the_pane_draws_a_cell`.
const CLOUD_UNITS: f32 = 10.0;
const SCALE_CELLS: f32 = 6.0 / 2.2;
const WASH_CELLS: f32 = 5.25;
/// The tile's texel size: a whole number of these, and never fewer or more.
///
/// Quantised because the pane's own pixels feed it: at one texel per pixel a
/// resize drag would rebake a 20 to 40 ms walk on EVERY frame of the drag, where
/// a 256-texel grain crosses a boundary a handful of times across a whole
/// window. The ceiling is memory — two `Rgba16Float` targets, so 2048 is 67 MB —
/// and past it the tile is simply coarser than the pane, which is the same
/// trade as reducing the tone target.
const TILE_STEP: u32 = 256;
const TILE_MAX: u32 = 2048;

/// What a baked tile holds, and therefore exactly what a rebake has to watch.
///
/// **A key is wrong in two directions and this one is worth writing out.**
/// Anything that feeds the baked channels and is missing here serves a stale
/// picture; anything carried here that decides nothing rebakes a full cell walk
/// at the rate of whatever it should not be watching. So the key is the STYLE,
/// the period, the texel size, the wash's pane orientation, and the dials the
/// WALK reads — `Variety` for the mosaic; `Lobe shape` and `Fuzz` for the
/// wash, which are the warp and the feather/bleed widths. Orientation decides
/// the rotated wash basis; the unrotated mosaic neither bakes nor reads it.
///
/// Not the DRIFT and not the clock. The walk's output is a fixed field that the
/// drift slides over — `drift` enters both styles only as a translation of the
/// cell coordinate — so it is a texture coordinate here rather than an input,
/// and a tile is never rebaked because time passed.
///
/// Not the light, the palette, the softness or `Cloud depth`: none of them
/// reaches the walk at all. Not `Refraction` or `Layers`, which are
/// read AFTER the tile, out of channels it already holds. Not `Scale size` or
/// `Glob size`, which decide how many cells cross the pane rather than what a
/// cell draws, and not the pane's pixels: both reach this only through
/// [`Self::texels`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TileKey {
    /// 0 for the mosaic, 1 for the wash — the same word the uniform carries.
    style: u32,
    /// The period in cells; production always uses forty.
    period: u32,
    /// One side of the square tile, in texels.
    texels: u32,
    /// Which pane axis is pitch for the wash's rotation. Always false for the
    /// mosaic, whose square bake stays in physical pane coordinates.
    wash_pitch_vertical: bool,
    /// The walk's own dials as bits, so this compares by value. Sanitized, so
    /// there is no NaN here to compare unequal to itself. The mosaic reads one
    /// and leaves the rest at zero.
    dials: [u32; 2],
}

impl TileKey {
    /// The period, for the uniform the shader divides a cell coordinate by.
    pub fn period(self) -> u32 {
        self.period
    }

    /// One side of the square tile, for the pass that fills it.
    pub fn texels(self) -> u32 {
        self.texels
    }
}

/// The tile this frame wants, or `None` where no cloud is drawn.
///
/// See [`TileKey`] for what is in it and what deliberately is not.
pub(super) fn tile_key(
    pixels: [u32; 2],
    atmosphere: SpectrogramAtmosphere,
    period: u32,
) -> Option<TileKey> {
    let settings = atmosphere.settings.sanitized();
    if !settings.effects().cloud {
        return None;
    }
    // The composite reads a cloud out of its tile and nowhere else: its
    // live-walk arm was retired because, never taken, it still cost the
    // full-resolution shader 16 to 21% (#1100).
    assert!(period > 0, "a cloud is drawn only out of a tile, so its period cannot be 0");
    let (style, cells, dials) = match settings.cloud_style {
        harmonigraph_scene::CloudStyle::Mosaic => {
            (0, SCALE_CELLS / settings.scale_size, [settings.scale_variety, 0.0])
        }
        harmonigraph_scene::CloudStyle::Watercolor => {
            (1, WASH_CELLS / settings.wash_size, [settings.wash_lobe, settings.wash_fuzz])
        }
    };
    // As fine as the pane itself draws a cell, so a tiled picture is the walk
    // resampled rather than a coarser one — and then rounded UP to a whole
    // [`TILE_STEP`], which is what keeps a resize off the bake. The 3-4-5
    // transform is a pure rotation, so unlike the former shear it has singular
    // value one and asks for no extra texels in any direction.
    let wanted = period as f32 * (pixels[1] as f32 / CLOUD_UNITS / cells);
    let texels = ((wanted / TILE_STEP as f32).ceil().max(1.0) as u32)
        .saturating_mul(TILE_STEP)
        .min(TILE_MAX);
    Some(TileKey {
        style,
        period,
        texels,
        wash_pitch_vertical: style == 1 && atmosphere.pitch_vertical,
        dials: dials.map(f32::to_bits),
    })
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    origin: [f32; 2],
    size: [f32; 2],
    step: [f32; 2],
    ppp: f32,
    spread: f32,
    contours: f32,
    contour_softness: f32,
    contour_strength: f32,
    /// 1 when the tone target exists and holds this frame's cloud, so the
    /// composite reads it instead of walking the cells under every pixel.
    tone_baked: u32,
    /// Cloud-space offset of the cloud texture, reduced from f64 on the CPU.
    ///
    /// The order below is the WGSL `Cloud` struct's order and has to stay that
    /// way: these are read by OFFSET, not by name, so transposing two `f32`
    /// fields swaps their values silently and nothing in the type system
    /// notices.
    drift: [f32; 2],
    cloud_depth: f32,
    scale_size: f32,
    scale_variety: f32,
    scale_refract: f32,
    /// 0 for the refracting scales, 1 for the watercolour wash. The wash reads
    /// none of the `scale_` settings and the scales read none of the `wash_`
    /// ones; both share what sits above them.
    cloud_style: u32,
    wash_size: f32,
    wash_fuzz: f32,
    wash_lobe: f32,
    wash_refract: f32,
    wash_layers: f32,
    /// The tile's period in cells, 0 only when no cloud is drawn and the shader
    /// never reads it. See [`TileKey`].
    tile_cells: u32,
    /// 1 when pitch is vertical, 0 when it is horizontal.
    pitch_vertical: u32,
    _pad: [u32; 2],
}

/// The `Cloud` struct's size in the uniform address space, which WGSL rounds up
/// to a multiple of 16 whatever the members add to.
///
/// A Rust mirror SHORTER than that binds a buffer the shader is entitled to read
/// past, and wgpu refuses the bind group rather than the draw — so this is a
/// compile-time check on a runtime failure that would otherwise arrive as a
/// validation error on the first clouded frame.
///
/// Retiring `Ragged` once left the members four bytes short of 112 and needed an
/// explicit tail. Retiring the tone controls leaves two tail words now;
/// adding or dropping a field can move the edge again and this catches it.
const _: () = assert!(
    std::mem::size_of::<Uniforms>().is_multiple_of(16),
    "the cloud uniform is not a whole number of 16-byte rows, so the shader's rounded-up \
     struct is larger than the buffer Rust writes",
);

/// The production uniform fields read by `wash_tile_field`, for its direct
/// shader probe. Returned as bytes so the parent test need not widen
/// [`Uniforms`]' visibility just to bind the same layout production uses.
#[cfg(test)]
pub(super) fn watercolor_tile_probe_uniform(pitch_vertical: bool) -> Vec<u8> {
    let mut uniforms: Uniforms = bytemuck::Zeroable::zeroed();
    uniforms.wash_layers = 1.0;
    uniforms.tile_cells = 40;
    uniforms.pitch_vertical = u32::from(pitch_vertical);
    bytemuck::bytes_of(&uniforms).to_vec()
}

pub(super) struct Pipelines {
    pub source: wgpu::RenderPipeline,
    pub bake: wgpu::RenderPipeline,
    /// The cloud's scalar tone into its own reduced target, for the composite to
    /// read instead of walking the cells per pixel.
    pub tone: wgpu::RenderPipeline,
    /// One period of the cell walk into the two tile targets, for both of the
    /// above to read instead of walking the ring at all.
    pub tile: wgpu::RenderPipeline,
    pub composite: wgpu::RenderPipeline,
    pub backdrop: wgpu::RenderPipeline,
    filter_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    filters: [wgpu::RenderPipeline; 4],
    sampler: wgpu::Sampler,
    /// The tile's repeating sampler — see the shader's `tile_sampler`, where
    /// the reason the other reads must keep clamping is spelled out.
    tile_sampler: wgpu::Sampler,
}

impl Pipelines {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        source_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let texture = |binding| wgpu::BindGroupLayoutEntry {
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
        let uniform = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let filter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spectral_cloud_filter_layout"),
            entries: &[texture(0), sampler_entry(1), uniform(2)],
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spectral_cloud_composite_layout"),
            entries: &[
                texture(0),
                texture(1),
                sampler_entry(2),
                uniform(3),
                texture(4),
                texture(5),
                texture(6),
                sampler_entry(7),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spectral_cloud_filter"),
            source: wgpu::ShaderSource::Wgsl(SOURCE.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spectral_cloud_filter_pipeline_layout"),
            bind_group_layouts: &[Some(&filter_layout)],
            ..Default::default()
        });
        let filters = ["fs_close_h", "fs_close_v", "fs_wide_h", "fs_wide_v"].map(|entry| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        });
        Self {
            source: create_spectrogram_pipeline(
                device,
                FORMAT,
                source_layout,
                None,
                "fs_density_source",
            ),
            bake: create_spectrogram_pipeline(
                device,
                FORMAT,
                source_layout,
                Some(&composite_layout),
                "fs_cloud_light",
            ),
            tone: create_spectrogram_pipeline(
                device,
                FORMAT,
                source_layout,
                Some(&composite_layout),
                "fs_cloud_tone",
            ),
            tile: tile_pipeline(device, source_layout, &composite_layout),
            composite: create_spectrogram_pipeline(
                device,
                format,
                source_layout,
                Some(&composite_layout),
                if format.is_srgb() || format == wgpu::TextureFormat::Rgba16Float {
                    "fs_cloud_linear"
                } else {
                    "fs_cloud_gamma"
                },
            ),
            backdrop: create_spectrogram_pipeline(
                device,
                format,
                source_layout,
                Some(&composite_layout),
                if format.is_srgb() || format == wgpu::TextureFormat::Rgba16Float {
                    "fs_cloud_backdrop_linear"
                } else {
                    "fs_cloud_backdrop_gamma"
                },
            ),
            filter_layout,
            composite_layout,
            filters,
            sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("spectral_cloud_sampler"),
                min_filter: wgpu::FilterMode::Linear,
                mag_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            tile_sampler: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("spectral_cloud_tile_sampler"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                min_filter: wgpu::FilterMode::Linear,
                mag_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
        }
    }
}

/// The tile bake: a full-screen triangle over the tile's own target, with two
/// colour attachments and no vertex buffer.
///
/// It declares the source layout at group 0 that it never reads, so the pass can
/// bind the same group every other cloud pass does; what it does read is the
/// cloud uniform at group 1, for the period and the style.
fn tile_pipeline(
    device: &wgpu::Device,
    source_layout: &wgpu::BindGroupLayout,
    composite_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("spectrogram_shader"),
        source: wgpu::ShaderSource::Wgsl(super::SPECTROGRAM_SRC.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("spectral_cloud_tile_pipeline_layout"),
        bind_group_layouts: &[Some(source_layout), Some(composite_layout)],
        ..Default::default()
    });
    let target = Some(wgpu::ColorTargetState {
        format: TILE_FORMAT,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("spectral_cloud_tile"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_cloud_tile"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_cloud_tile"),
            compilation_options: Default::default(),
            targets: &[target.clone(), target],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// One period of the cell walk, and what it was filled for.
///
/// The one thing here that does NOT follow the pane: every other target is
/// rewritten from scratch each frame, where refilling these two is a whole cell
/// walk. So they are carried across a rebuild the light's size forces (see
/// `SpectrogramCallback::prepare`), and [`Self::baked`] is what says a bake is
/// owed rather than a reallocation.
pub(super) struct Tile {
    views: [wgpu::TextureView; 2],
    texels: u32,
    baked: Option<TileKey>,
}

pub(super) struct Targets {
    #[cfg(test)]
    pub encoded_passes: std::sync::atomic::AtomicU32,
    pub size: [u32; 2],
    pub source_view: wgpu::TextureView,
    pub coverage_vertices: wgpu::Buffer,
    /// The whole-pane quad for the material and reduced-tone passes.
    /// Final painting uses `coverage_vertices`, the atmosphere's region.
    /// Both sampled fields must be filled beyond that region: a displaced or
    /// bilinear read at the divider otherwise blends with a cleared texel and
    /// draws a dark seam. The original data mesh still bounds measured sound.
    pub tone_vertices: wgpu::Buffer,
    views: [wgpu::TextureView; 3],
    /// The reduced tone target and its size, `None` where the cloud is drawn
    /// natively. Part of the allocation key beside [`Self::size`] — see
    /// `SpectrogramCallback::prepare`.
    pub tone: Option<(wgpu::TextureView, [u32; 2])>,
    tile: Option<Tile>,
    source_uniform: wgpu::Buffer,
    pub source_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    filter_groups: [wgpu::BindGroup; 3],
    pub bake_group: wgpu::BindGroup,
    /// Reads the baked material and writes the tone target, so the tone target
    /// is the one view this group must NOT carry.
    pub tone_group: Option<wgpu::BindGroup>,
    /// Writes both tile targets, so those are the two views it stands scratch
    /// in for.
    tile_group: Option<wgpu::BindGroup>,
    pub composite_group: wgpu::BindGroup,
}

/// What one set of targets is allocated FOR: the light field's size, the reduced
/// tone's where there is one, and the tile this frame wants beside whatever tile
/// the previous set held.
///
/// The three move independently — the light's size follows the musical radius,
/// the tone's follows pane pixels and display scale, and the tile's follows how
/// many cloud cells cross the pane — which is why `SpectrogramCallback::prepare` compares all
/// three before rebuilding, and why the tile alone is handed back in.
pub(super) struct Allocation {
    pub size: [u32; 2],
    pub tone: Option<[u32; 2]>,
    pub tile: Option<TileKey>,
    pub carried: Option<Tile>,
}

impl Targets {
    pub fn new(
        device: &wgpu::Device,
        pipelines: &Pipelines,
        wanted: Allocation,
        source_layout: &wgpu::BindGroupLayout,
        grid: &wgpu::Buffer,
        lut: &wgpu::TextureView,
    ) -> Self {
        let Allocation { size, tone: tone_size, tile: tile_key, carried } = wanted;
        let formatted = |label, size: [u32; 2], format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
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
                })
                .create_view(&Default::default())
        };
        let sized = |label, size: [u32; 2]| formatted(label, size, FORMAT);
        let view = |label| sized(label, size);
        let source_view = view("spectral_cloud_source");
        let views = [
            view("spectral_cloud_scratch"),
            view("spectral_cloud_close"),
            view("spectral_cloud_wide"),
        ];
        let tone = tone_size.map(|size| (sized("spectral_cloud_tone", size), size));
        // Reused whenever it is already the right shape, key and all, so a
        // rebuild the LIGHT's size forced costs no walk at all.
        let tile = tile_key.map(|key| match carried {
            Some(tile) if tile.texels == key.texels => tile,
            _ => Tile {
                views: ["spectral_cloud_tile_a", "spectral_cloud_tile_b"]
                    .map(|label| formatted(label, [key.texels; 2], TILE_FORMAT)),
                texels: key.texels,
                baked: None,
            },
        });
        let buffer = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let source_uniform = buffer(
            "spectral_cloud_source_uniform",
            std::mem::size_of::<SpectrogramUniforms>() as u64,
        );
        let uniform = buffer("spectral_cloud_uniform", std::mem::size_of::<Uniforms>() as u64);
        let filter_groups = [&source_view, &views[0], &views[1]].map(|view| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("spectral_cloud_filter_group"),
                layout: &pipelines.filter_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&pipelines.sampler),
                    },
                    wgpu::BindGroupEntry { binding: 2, resource: uniform.as_entire_binding() },
                ],
            })
        });
        // wgpu validates every resource a bound group carries against the pass's
        // attachments whether the shader reads it or not, so the views a pass
        // WRITES are PARAMETERS here: the tone pass binds a scratch where the
        // tone target would be, and the tile pass binds one at each tile. The
        // scratch is the harmless choice — neither `fs_cloud_tone` nor
        // `fs_cloud_tile` reads any of those three bindings.
        let cloud_group = |front, tone, tile: [&wgpu::TextureView; 2]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("spectral_cloud_composite_group"),
                layout: &pipelines.composite_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(front),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&views[2]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&pipelines.sampler),
                    },
                    wgpu::BindGroupEntry { binding: 3, resource: uniform.as_entire_binding() },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(tone),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(tile[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(tile[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::Sampler(&pipelines.tile_sampler),
                    },
                ],
            })
        };
        let scratch_tile = [&views[0], &views[0]];
        let tile_views =
            tile.as_ref().map_or(scratch_tile, |tile| [&tile.views[0], &tile.views[1]]);
        let bake_group = cloud_group(&views[1], &views[0], tile_views);
        // Reads the baked material the light passes just wrote, and writes the
        // tone target — so that is the one view it stands a scratch in for.
        let tone_group = tone.as_ref().map(|_| cloud_group(&source_view, &views[0], tile_views));
        let tile_group = tile.as_ref().map(|_| cloud_group(&source_view, &views[0], scratch_tile));
        let composite_group = cloud_group(
            &source_view,
            tone.as_ref().map_or(&views[0], |(view, _)| view),
            tile_views,
        );
        let source_group = source_group(device, source_layout, &source_uniform, grid, lut);
        Self {
            #[cfg(test)]
            encoded_passes: std::sync::atomic::AtomicU32::new(0),
            size,
            source_view,
            coverage_vertices: create_vertex_buffer::<SpectrogramVertex>(
                device,
                "spectral_cloud_coverage",
                6,
            ),
            tone_vertices: create_vertex_buffer::<SpectrogramVertex>(
                device,
                "spectral_cloud_tone_quad",
                6,
            ),
            views,
            tone,
            tile,
            source_uniform,
            source_group,
            uniform,
            filter_groups,
            bake_group,
            tone_group,
            tile_group,
            composite_group,
        }
    }

    /// The reduced tone target's size, for the allocation key to compare
    /// against what this frame's settings ask for.
    pub fn tone_size(&self) -> Option<[u32; 2]> {
        self.tone.as_ref().map(|&(_, size)| size)
    }

    /// The held tile's texel size, which is the whole of what it was ALLOCATED
    /// for — both targets exist whatever the style, so a style change is a
    /// rebake and never a reallocation.
    pub fn tile_texels(&self) -> Option<u32> {
        self.tile.as_ref().map(|tile| tile.texels)
    }

    /// Whether the tile holds something other than `key` and so owes a walk.
    /// The WHOLE of the rebake decision — see [`TileKey`] for what is in one.
    pub fn tile_owes(&self, key: TileKey) -> bool {
        self.tile.as_ref().is_some_and(|tile| tile.baked != Some(key))
    }

    /// The tile's two targets and the group the pass that writes them binds.
    pub fn tile_pass(&self) -> Option<(&[wgpu::TextureView; 2], &wgpu::BindGroup)> {
        Some((&self.tile.as_ref()?.views, self.tile_group.as_ref()?))
    }

    /// Records the key the tile now holds. Called once the bake is encoded.
    pub fn tile_baked(&mut self, key: TileKey) {
        if let Some(tile) = self.tile.as_mut() {
            tile.baked = Some(key);
        }
    }

    /// The baked tile, for a rebuilt set of targets to carry across.
    pub fn into_tile(self) -> Option<Tile> {
        self.tile
    }

    pub fn rebind(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        grid: &wgpu::Buffer,
        lut: &wgpu::TextureView,
    ) {
        self.source_group = source_group(device, layout, &self.source_uniform, grid, lut);
    }

    pub fn update(
        &self,
        queue: &wgpu::Queue,
        mut read: SpectrogramUniforms,
        rect: egui::Rect,
        ppp: f32,
        atmosphere: SpectrogramAtmosphere,
        tile: Option<TileKey>,
    ) {
        // The source mesh records only measured history. Its already-blurred
        // light can occupy the whole spectrogram region, without crossing the
        // analyzer divider or widening the exact heatmap's sample footprint.
        let region = atmosphere.region.intersect(rect);
        let region = if region.is_positive() {
            region
        } else {
            egui::Rect::from_min_max(rect.min, rect.min)
        };
        let corners =
            [region.left_top(), region.right_top(), region.right_bottom(), region.left_bottom()];
        // `slab` and `t` carry the corner's PANE-RELATIVE fraction here rather
        // than a run position, which is what `fs_cloud_tone` reads to recover
        // the same point the composite would have walked under its own pixel.
        // The region is a sub-rect of the pane, so these need not reach 0 and 1.
        // Nothing else reads these two on this quad: the bake and the backdrop
        // both work off `in.position` alone.
        let vertices = [0, 1, 2, 0, 2, 3].map(|i| {
            let fraction = (corners[i] - rect.min) / rect.size();
            SpectrogramVertex { pos: corners[i].into(), slab: fraction.x, t: fraction.y }
        });
        queue.write_buffer(&self.coverage_vertices, 0, bytemuck::cast_slice(&vertices));
        // The same quad over the whole pane, for the material and tone passes — see
        // `tone_vertices`. Built here rather than once at allocation because
        // `rect` is what moves, and this is where it arrives.
        let pane = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
        let tone_quad = [0, 1, 2, 0, 2, 3].map(|i| {
            let fraction = (pane[i] - rect.min) / rect.size();
            SpectrogramVertex { pos: pane[i].into(), slab: fraction.x, t: fraction.y }
        });
        queue.write_buffer(&self.tone_vertices, 0, bytemuck::cast_slice(&tone_quad));
        read.origin_points = rect.min.into();
        read.viewport_points = rect.size().into();
        let pitch_vertical = atmosphere.pitch_vertical;
        let axis = usize::from(pitch_vertical);
        // A clipped pane can cover only part of the full pitch axis. Keep
        // that axis's bucket footprint per reduced pixel, not one full-range
        // footprint per texel of the smaller visible rectangle.
        let visible_pixels = if pitch_vertical { rect.height() } else { rect.width() } * ppp;
        read.rows =
            (read.rows as f32 * self.size[axis] as f32 / visible_pixels).round().max(1.0) as u32;
        queue.write_buffer(&self.source_uniform, 0, bytemuck::bytes_of(&read));
        let settings = atmosphere.settings.sanitized();
        let pitch = settings.pitch_softness * atmosphere.points_per_cent;
        let time = settings.time_softness * atmosphere.points_per_ms;
        let radius = if pitch_vertical { [time, pitch] } else { [pitch, time] };
        // A constant crossing in the direction the setting names, in cloud
        // units — ten across the pane's height, so 1x travels about one pane
        // height every four minutes.
        let drift = cloud_drift(settings, atmosphere.now);
        let uniforms = Uniforms {
            origin: rect.min.into(),
            size: rect.size().into(),
            step: [radius[0] / rect.width(), radius[1] / rect.height()],
            ppp,
            spread: settings.spread,
            contours: settings.contours,
            contour_softness: settings.contour_softness,
            contour_strength: settings.contour_strength,
            tone_baked: u32::from(self.tone.is_some()),
            drift,
            cloud_depth: if settings.effects().cloud { settings.cloud_depth } else { 0.0 },
            scale_size: settings.scale_size,
            scale_variety: settings.scale_variety,
            scale_refract: settings.scale_refract,
            cloud_style: match settings.cloud_style {
                harmonigraph_scene::CloudStyle::Mosaic => 0,
                harmonigraph_scene::CloudStyle::Watercolor => 1,
            },
            wash_size: settings.wash_size,
            wash_fuzz: settings.wash_fuzz,
            wash_lobe: settings.wash_lobe,
            wash_refract: settings.wash_refract,
            wash_layers: settings.wash_layers,
            // Zero only where no tile was allocated, which is where no cloud is
            // drawn and the shader returns before the tone.
            tile_cells: tile.map_or(0, TileKey::period),
            pitch_vertical: u32::from(pitch_vertical),
            _pad: [0; 2],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniforms));
    }

    pub fn blur(&self, encoder: &mut wgpu::CommandEncoder, pipelines: &Pipelines) {
        // Source -> scratch -> close; close -> scratch -> wide. Feeding the
        // already softened image to the wide kernel closes its sampling gaps.
        // Every pass reads a different texture from the attachment it writes.
        for (i, (input, output)) in [(0, 0), (1, 1), (2, 0), (1, 2)].into_iter().enumerate() {
            #[cfg(test)]
            self.encoded_passes.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("spectral_cloud_blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.views[output],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipelines.filters[i]);
            pass.set_bind_group(0, &self.filter_groups[input], &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

fn source_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    grid: &wgpu::Buffer,
    lut: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("spectral_cloud_source_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: grid.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(lut) },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::{
        cloud_drift, retained_size, source_size, tile_key, tone_size, SpectrogramAtmosphere,
        CLOUD_UNITS, SCALE_CELLS, TILE_MAX, TILE_STEP, WASH_CELLS,
    };

    #[test]
    fn drift_follows_the_dial_at_a_constant_direction() {
        let at = |direction, speed, now| {
            cloud_drift(
                harmonigraph_scene::SpectralAtmosphere {
                    cloud_speed: speed,
                    cloud_direction: direction,
                    ..Default::default()
                },
                now,
            )
        };
        let phase = at(0.0, 1.0, 0.0);
        let travelled = |direction| {
            let later = at(direction, 1.0, 25.0);
            [later[0] - phase[0], later[1] - phase[1]]
        };
        let close = |got: [f32; 2], want: [f32; 2]| {
            assert!((got[0] - want[0]).abs() < 1e-5, "x: {got:?} != {want:?}");
            assert!((got[1] - want[1]).abs() < 1e-5, "y: {got:?} != {want:?}");
        };
        let step = 25.0 * 0.047_169_905;
        // Sampling moves opposite the visible texture direction.
        close(travelled(0.0), [-step, 0.0]);
        close(travelled(90.0), [0.0, -step]);
        close(travelled(180.0), [step, 0.0]);
        close(travelled(270.0), [0.0, step]);
        let first = travelled(37.0);
        let second = {
            let a = at(37.0, 1.0, 25.0);
            let b = at(37.0, 1.0, 50.0);
            [b[0] - a[0], b[1] - a[1]]
        };
        close(second, first);
        close(at(220.0, 0.0, 10_000.0), phase);
        close(
            travelled(harmonigraph_scene::SpectralAtmosphere::default().cloud_direction),
            [1.0, -0.625],
        );
    }

    /// The tile is as fine as the pane draws a cell, in whole [`TILE_STEP`]s —
    /// and the cell counts it divides by are the SHADER's own.
    ///
    /// Both halves matter. Finer than the pane buys nothing and coarser is a
    /// blur the dial did not ask for, so the size follows the pane; but at one
    /// texel per pixel a resize drag would rebake a whole cell walk on every
    /// frame of the drag, which is the too-wide key this repo ships. The step
    /// is what makes a drag cross a boundary a handful of times.
    ///
    /// The constants are a mirror of the shader's, since only this side needs
    /// to know what a cell is. Read off the shipped text, because a mirror that
    /// drifted would size every tile wrong with nothing saying so.
    #[test]
    fn the_tile_is_as_fine_as_the_pane_draws_a_cell() {
        let number = |name: &str| -> f32 {
            crate::shadow::tests::shader_const(crate::spectrogram::SPECTROGRAM_SRC, name)
                .split('/')
                .map(|part| part.trim().parse::<f32>().expect("a number"))
                .reduce(|a, b| a / b)
                .expect("a constant has a value")
        };
        assert_eq!(CLOUD_UNITS, number("CLOUD_UNITS"));
        assert_eq!(SCALE_CELLS, number("SCALE_CELLS"));
        assert_eq!(WASH_CELLS, number("WASH_CELLS"));

        let at = |cloud_tile, wash_size, height| {
            tile_key(
                [1920, height],
                SpectrogramAtmosphere {
                    settings: harmonigraph_scene::SpectralAtmosphere {
                        wash_size,
                        cloud_style: harmonigraph_scene::CloudStyle::Watercolor,
                        ..Default::default()
                    },
                    region: egui::Rect::ZERO,
                    pitch_vertical: true,
                    points_per_cent: 0.03,
                    points_per_ms: 0.01,
                    points_per_slab: 0.0,
                    now: 0.0,
                },
                cloud_tile,
            )
            .map(|key| key.texels())
        };
        // A 1080-pixel pane draws 20.6 pixels to a glob cell at the fresh size.
        // Rotation is an isometry, so P20 wants 412 texels and rounds to 512.
        assert_eq!(at(20, 1.0, 1080), Some(2 * TILE_STEP));
        let period = super::CloudSampling::default().tile_cells;
        assert_eq!(period, 40);
        assert_eq!(at(period, 1.0, 1080), Some(4 * TILE_STEP));
        // A pane resized by a tenth stays on the same step.
        assert_eq!(at(20, 1.0, 1188), at(20, 1.0, 1080));
        // Fine cells want few texels, and the floor is one step.
        assert_eq!(at(20, harmonigraph_scene::CLOUD_SIZE_MIN, 1080), Some(TILE_STEP));
        // Coarse cells on a tall pane run past the ceiling, where the tile is
        // simply coarser than the pane.
        assert_eq!(at(40, harmonigraph_scene::CLOUD_SIZE_MAX, 4320), Some(TILE_MAX));
    }

    /// A reduced tone target exists only where it would be SMALLER than the
    /// pane, and it is the pane's pixels divided by device pixels per sample.
    #[test]
    fn the_tone_target_appears_only_where_it_is_coarser_than_the_pane() {
        let at = |cloud_pixel, cloud_depth, ppp| {
            tone_size(
                [1920, 1081],
                ppp,
                SpectrogramAtmosphere {
                    settings: harmonigraph_scene::SpectralAtmosphere {
                        cloud_depth,
                        ..Default::default()
                    },
                    region: egui::Rect::ZERO,
                    pitch_vertical: true,
                    points_per_cent: 0.03,
                    points_per_ms: 0.01,
                    points_per_slab: 0.0,
                    now: 0.0,
                },
                cloud_pixel,
            )
        };
        // The fresh half point is one device pixel on a Retina pane and half of
        // one on a plain pane: native on both, which is what keeps the default
        // picture the full-resolution one.
        let pixel = super::CloudSampling::default().pixel_points;
        assert_eq!(pixel, 0.5);
        assert_eq!(at(pixel, 1.0, 2.0), None);
        assert_eq!(at(pixel, 1.0, 1.0), None);
        assert_eq!(at(pixel, 1.0, 4.0), Some([960, 541]));
        // One point is native at 1x and halves each axis at 2x — a quarter of
        // the walk — and the odd axis rounds UP so the target still covers the
        // pane.
        assert_eq!(at(1.0, 1.0, 1.0), None);
        assert_eq!(at(1.0, 1.0, 2.0), Some([960, 541]));
        assert_eq!(at(4.0, 1.0, 2.0), Some([240, 136]));
        // No cloud, nothing to reduce, whatever the sample spacing.
        assert_eq!(at(4.0, 0.0, 2.0), None);
    }

    /// `Blur time step` bounds the light field's TIME axis by the run's slab
    /// count, and reaches nothing else.
    ///
    /// The fixture is the zoomed-out pane the dial exists for, and its
    /// precondition is asserted rather than assumed: at this span the time
    /// sigma is a tenth of a pixel, so the musical reduction alone leaves that
    /// axis at FULL resolution and every texel the dial removes is one only it
    /// could remove. The pitch axis is reduced by its own sigma in the same
    /// fixture, which is what makes "the dial left it alone" a claim about the
    /// dial rather than about an axis nothing was touching.
    ///
    /// Powers of two throughout so the bound divides exactly and the assertion
    /// is the rule rather than a rounding.
    #[test]
    fn the_time_step_bounds_the_light_field_by_the_runs_slabs_alone() {
        // A 1024 x 1024 pane at 2 px/pt showing 600 s of a 119.59-semitone
        // spectrum in 256 slabs: four points to a slab.
        let pixels = [1024, 1024];
        let points = [512.0, 512.0];
        let sized = |blur_time_step, points_per_slab| {
            source_size(
                pixels,
                2.0,
                SpectrogramAtmosphere {
                    settings: harmonigraph_scene::SpectralAtmosphere {
                        blur_time_step,
                        pitch_softness: 35.0,
                        time_softness: 57.23,
                        ..Default::default()
                    },
                    region: egui::Rect::ZERO,
                    pitch_vertical: true,
                    points_per_cent: points[1] / (119.59 * 100.0),
                    points_per_ms: points[0] / (600.0 * 1000.0),
                    points_per_slab,
                    now: 0.0,
                },
            )
        };
        let slab = points[0] / 256.0;
        let at = |blur_time_step| sized(blur_time_step, slab);
        let off = at(0.0);
        assert_eq!(off[0], pixels[0], "the fixture's time axis was reduced without the dial");
        assert!(off[1] < pixels[1], "the fixture's pitch axis was not reduced by its own sigma");
        // One texel per slab, per two slabs, and per half a slab.
        assert_eq!(at(1.0), [256, off[1]]);
        assert_eq!(at(2.0), [128, off[1]]);
        assert_eq!(at(0.5), [512, off[1]]);
        // A caller with no run to measure is the dial's own zero: no bound.
        assert_eq!(sized(1.0, 0.0), off);
    }

    #[test]
    fn retained_targets_bound_resolution_and_preserve_full_axes() {
        let choose = |held| retained_size([100, 100], [128, 128], Some(held));
        assert_eq!(choose([90, 110]), [90, 110]);
        assert_eq!(choose([89, 100]), [100, 100]);
        assert_eq!(choose([100, 111]), [100, 100]);
        assert_eq!(retained_size([100, 100], [105, 128], Some([110, 100])), [100, 100]);
        for axis in 0..2 {
            let mut full = [100, 100];
            full[axis] = 128;
            let mut held = full;
            held[axis] = 120;
            assert_eq!(retained_size(full, [128, 128], Some(held)), full);
        }
        assert_eq!(retained_size([1, 1], [128, 128], Some([8, 8])), [1, 1]);
        // After crossing a resize boundary, reversing over that boundary
        // must retain the new allocation, not oscillate between two sizes.
        let mut held = [100, 100];
        for desired in [112, 111, 112, 111, 112, 111] {
            held = retained_size([desired, 100], [128, 128], Some(held));
            assert_eq!(held, [112, 100]);
        }
    }
}
