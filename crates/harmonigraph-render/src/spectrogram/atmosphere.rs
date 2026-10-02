//! A scalar image diffuses the heatmap; optional color history follows material
//! coordinates after palette mapping. Per-pane allocations are keyed on sizes.
//! The walk tile carries its bake under [`TileKey`], while color memory carries
//! its values under [`memory_key`]. Source pixels refresh every draw, including
//! paused zooms and palette edits.

use crate::uniforms::{uniform_group, Float2, Float4, Int2};

use super::{create_spectrogram_pipeline, SpectrogramUniforms, SpectrogramVertex};
pub(super) use crate::stars::*;
use crate::{create_vertex_buffer, wgpu};

pub(super) const SOURCE: &str = include_str!("../shaders/spectral_atmosphere.wgsl");
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
/// The tile's own format. Four channels because the wash's walk produces seven
/// numbers over two targets, the fine octave's four filling one; half floats
/// because what is stored is a cell offset of order one, where the eleven-bit
/// mantissa is a thousandth of a cell.
const TILE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The third tile target: per octave, the wash's distance out from the front
/// glob's arc, 0..1 radii, so eight bits are plenty.
const PIGMENT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Unorm;
// Half-float feedback can stall far from the target when alpha is small.
const MEMORY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

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
/// screen direction, unreduced and still in f64. The shader samples
/// `screen + drift`, so the sampling offset moves opposite the texture itself.
fn cloud_offset(settings: harmonigraph_scene::SpectralAtmosphere, now: f64) -> [f64; 2] {
    harmonigraph_scene::MaterialSettings::drift(settings.cloud_speed, settings.cloud_direction, now)
}

/// [`cloud_offset`] as the shader takes it. Watercolor repeats its `tile`, so
/// the offset is reduced by whole repeats before it narrows to f32,
/// as Stars and the lattice reduce theirs: unreduced, a long clock leaves the
/// f32 fewer and fewer bits of the cell it lands in. A screen-axis repeat of
/// the Watercolor tile is five periods, because its basis is the 3-4-5
/// rotation. Reduced about zero rather than onto `[0, repeat)`, so an offset
/// already inside half a repeat — every one in a render's first minutes at the
/// fresh speed — passes bit for bit.
///
/// Scales' warp is nonperiodic, and Stars drift by their own slices, so
/// neither has a tile or a repeat, and their offset passes unreduced.
fn cloud_drift(
    settings: harmonigraph_scene::SpectralAtmosphere,
    offset: [f64; 2],
    tile: Option<TileKey>,
) -> [f32; 2] {
    let repeat = tile.map(|tile| {
        let material = settings.material_settings;
        let (cells, periods) = match settings.cloud_style {
            harmonigraph_scene::CloudStyle::Watercolor => (WASH_CELLS / material.wash_size, 5),
            harmonigraph_scene::CloudStyle::Stars
            | harmonigraph_scene::CloudStyle::VelvetScales => {
                unreachable!("only Watercolor draws out of a tile")
            }
        };
        f64::from(tile.period() * periods) / f64::from(cells)
    });
    offset.map(|v| repeat.map_or(v, |r| v - r * (v / r).round()) as f32)
}

/// The starfield's layout for a pane of `pixels`, or `None` where no starfield
/// is drawn and no atlas is allocated.
pub(super) fn stars(pixels: [u32; 2], atmosphere: SpectrogramAtmosphere) -> Option<StarLayout> {
    let settings = atmosphere.settings.sanitized();
    let drawn =
        settings.effects().cloud && settings.cloud_style == harmonigraph_scene::CloudStyle::Stars;
    drawn.then(|| star_layout(settings.stars, pixels[0] as f32 / pixels[1] as f32))
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

/// Bucket physical history storage without changing its logical texel grid.
/// A divider drag then reallocates only at 64-pixel boundaries; at most 63
/// extra texels per axis are retained, including at the adapter's size limit.
pub(super) fn memory_allocation_size(extent: [u32; 2], limit: u32) -> [u32; 2] {
    #[cfg(test)]
    if color_memory_tests::EXACT_ALLOCATION.get() {
        return extent;
    }
    extent.map(|n| n.div_ceil(64).saturating_mul(64).min(limit))
}

/// The precomposite size, or `None` to work the texture out per pixel in the
/// composite.
///
/// High, Medium and Low Stars composite their far three layers at 75%, 50% and one-third dimensions.
/// Uniform Stars retain a native RGB split on large regions. The caller retains
/// a pane-relative allocation for texel addressing,
/// but scissors this pass to the drawn region on every frame.
///
/// Other styles stay native at sample spacing at or under one DEVICE pixel — the fixed half point on a Retina pane and on a plain one —
/// where a reduced target would be the pane's own resolution or larger and the
/// extra pass would buy nothing. Above that each axis is divided by the same
/// number of pixels per sample, so the per-pixel cost falls with its square.
pub(super) fn tone_size(
    pixels: [u32; 2],
    ppp: f32,
    atmosphere: SpectrogramAtmosphere,
    pixel_points: f32,
) -> Option<[u32; 2]> {
    let settings = atmosphere.settings.sanitized();
    if !settings.effects().cloud {
        return None;
    }
    if settings.cloud_style == harmonigraph_scene::CloudStyle::Stars {
        if crate::stars::star_far_reduced(settings.stars) {
            return Some(star_far_size(pixels, settings.stars));
        }
        let split = u64::from(pixels[0]) * u64::from(pixels[1]) >= STAR_SPLIT_PIXELS;
        #[cfg(test)]
        let split = super::tests::STAR_SPLIT_OVERRIDE.get().unwrap_or(split);
        return split.then_some(pixels);
    }
    if settings.cloud_style == harmonigraph_scene::CloudStyle::VelvetScales {
        // S1's broad bodies need 32 samples per cell, independent of display
        // resolution. Sharper edges increase that density; tiny cells stay
        // native. This bounds the expensive exact body mixture without
        // selecting/truncating contributors or changing their sampled light.
        // Power-of-two spacings keep nearby dial values on one allocation.
        let material = settings.material_settings;
        let cell = pixels[1] as f32 * (24.0 / 405.0) * material.velvet_size;
        let samples = 32.0 * (0.34 / material.velvet_edge).max(1.0);
        let spacing = 2.0f32.powf((cell / samples).max(1.0).log2().floor());
        return Some(pixels.map(|n| (n as f32 / spacing).ceil().max(1.0) as u32));
    }
    let pixel = pixel_points * ppp;
    if pixel <= 1.0 {
        return None;
    }
    Some(std::array::from_fn(|axis| ((pixels[axis] as f32 / pixel).ceil() as u32).max(1)))
}

/// The shader's own `CLOUD_UNITS` and `WASH_CELLS`: how many cloud units cross
/// the pane's height and how many of the wash's cells cross one unit at a size
/// of 1x. Nothing else here needs to know what a cell is —
/// the tile does, because how fine it has to be is how fine the pane draws one.
///
/// Held against the shipped shader text by
/// `the_tile_is_as_fine_as_the_pane_draws_a_cell`.
const CLOUD_UNITS: f32 = 10.0;
const WASH_CELLS: f32 = 5.25;
/// The tile's texel size: a whole number of these, and never fewer or more.
///
/// Quantised because the pane's own pixels feed it: at one texel per pixel a
/// resize drag would rebake a 20 to 40 ms walk on EVERY frame of the drag, where
/// a 256-texel grain crosses a boundary a handful of times across a whole
/// window. The ceiling is memory — two `Rgba16Float` targets and an `Rg8Unorm`
/// one, so 2048 is 75 MB —
/// and past it the tile is simply coarser than the pane, which is the same
/// trade as reducing the tone target.
const TILE_STEP: u32 = 256;
const TILE_MAX: u32 = 2048;

/// What a baked tile holds, and therefore exactly what a rebake has to watch.
///
/// **A key is wrong in two directions and this one is worth writing out.**
/// Anything that feeds the baked channels and is missing here serves a stale
/// picture; anything carried here that decides nothing rebakes a full cell walk
/// at the rate of whatever it should not be watching. So the key is the period,
/// the texel size, the pane orientation, and the dials the WALK reads —
/// `Shape warp` and `Edge feathering`, which are the warp and the feather/bleed
/// widths. Orientation decides the rotated wash basis. Not the style: only the
/// wash bakes a tile, so there is no other walk for a key to tell it from.
///
/// Not the DRIFT and not the clock. The walk's output is a fixed field that the
/// drift slides over — `drift` enters only as a translation of the
/// cell coordinate — so it is a texture coordinate here rather than an input,
/// and a tile is never rebaked because time passed.
///
/// Not the light, the palette, the softness or `Texture mix`: none of them
/// reaches the walk at all. Not `Refraction` or `Fine layer mix`, which are
/// read AFTER the tile, out of channels it already holds. Not `Patch size`,
/// which decides how many cells cross the pane rather than what a
/// cell draws, and not the pane's pixels: both reach this only through
/// [`Self::texels`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TileKey {
    /// The period in cells; production always uses forty.
    period: u32,
    /// One side of the square tile, in texels.
    texels: u32,
    /// Which pane axis is pitch for the wash's rotation.
    pitch_vertical: bool,
    /// The walk's own dials as bits, so this compares by value. Sanitized, so
    /// there is no NaN here to compare unequal to itself.
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
    // The starfield walks its own ring per pixel and reads no tile: what it
    // walks MOVES — every slice at its own speed, every star on its own
    // life — so there is no one fixed field to bake.
    if !settings.effects().cloud
        || matches!(
            settings.cloud_style,
            harmonigraph_scene::CloudStyle::Stars | harmonigraph_scene::CloudStyle::VelvetScales
        )
    {
        return None;
    }
    let harmonigraph_scene::SpectralAtmosphere {
        pitch_softness: _,     // applied after the tile bake
        time_softness: _,      // applied after the tile bake
        spread: _,             // applied after the tile bake
        blur_time_step: _,     // applied after the tile bake
        contour_strength: _,   // applied after the tile bake
        contours: _,           // applied after the tile bake
        contour_softness: _,   // applied after the tile bake
        cloud_depth: _,        // does not change the baked cell walk
        color_pickup: _,       // does not change the baked cell walk
        color_release: _,      // does not change the baked cell walk
        cloud_speed: _,        // does not change the baked cell walk
        cloud_direction: _,    // does not change the baked cell walk
        wash_pool: _,          // the tile holds the distance, the dials shape it after
        wash_pool_width: _,    // the tile holds the distance, the dials shape it after
        wash_pool_softness: _, // the tile holds the distance, the dials shape it after
        cloud_style: _,        // only the wash reaches here; the rest returned above
        stars: _,              // Stars do not use a displacement tile.
        material_settings:
            harmonigraph_scene::MaterialSettings {
                velvet_size: _,
                velvet_variety: _,
                velvet_edge: _,
                velvet_irregularity: _,
                velvet_shape: _,
                velvet_square: _,
                velvet_tilt: _,
                wash_size,
                wash_fuzz,
                wash_lobe,
                wash_refract: _,    // applied after the tile bake
                wash_layers: _,     // applied after the tile bake
                wash_randomness: _, // applied after coloring
            },
    } = settings;
    // The composite reads a cloud out of its tile and nowhere else: its
    // live-walk arm was retired because, never taken, it still cost the
    // full-resolution shader 16 to 21% (#1100).
    assert!(period > 0, "a cloud is drawn only out of a tile, so its period cannot be 0");
    let cells = WASH_CELLS / wash_size;
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
        period,
        texels,
        pitch_vertical: atmosphere.pitch_vertical,
        dials: [wash_lobe, wash_fuzz].map(f32::to_bits),
    })
}

uniform_group! {
struct Uniforms {
    origin: Float2,
    size: Float2,
    step: Float2,
    ppp: f32,
    spread: f32,
    contours: f32,
    contour_softness: f32,
    contour_strength: f32,
    /// 1 when the tone target holds this frame's reduced scalar cloud field
    /// or RGB far Stars layers.
    tone_baked: u32,
    /// Cloud-space offset of the cloud texture, reduced from f64 on the CPU.
    ///
    /// The order below is the WGSL `Cloud` struct's order and has to stay that
    /// way: these are read by OFFSET, not by name, so transposing two `f32`
    /// fields swaps their values silently and nothing in the type system
    /// notices.
    drift: Float2,
    cloud_depth: f32,
    /// 1 for the watercolour wash, 2 for the starfield, 3 for Scales; 0 is
    /// unused. None reads another's own settings; all share what sits above
    /// them.
    cloud_style: u32,
    wash_size: f32,
    wash_fuzz: f32,
    wash_lobe: f32,
    wash_refract: f32,
    wash_layers: f32,
    /// The tile's period in cells, 0 when no tile is baked — no cloud drawn, or
    /// Stars or Scales, which read none — and the shader never reads it then.
    /// See [`TileKey`].
    tile_cells: u32,
    /// 1 when pitch is vertical, 0 when it is horizontal.
    pitch_vertical: u32,
    /// The starfield's brightness variation and life clock ([`star_life`]).
    /// The far, near and slice rows that follow all start on 16-byte boundaries.
    star_randomness: f32,
    star_life: f32,
    /// The starfield's size variation; the rest of its row is padding.
    star_size_variation: f32,
    star_pad0: u32,
    star_pad1: u32,
    /// Exact far target dimensions, optimized-far flag, and padding.
    star_far: Float4,
    /// Exact reduced foreground dimensions; zero means native foreground.
    star_near: Float4,
    star_slices: [StarSlice; STAR_SLICES],
    memory_enabled: u32,
    memory_valid: u32,
    pickup_alpha: f32,
    release_alpha: f32,
    memory_shift: Int2,
    memory_fraction: Float2,
    previous_life: f32,
    wash_randomness: f32,
    memory_extent: Float2,
    previous_slices: [StarSlice; STAR_SLICES],
    /// Actual rounded dimensions and array address for each depth.
    star_halo_samples: [StarHaloSample; STAR_SLICES],
    velvet: Float4,
    /// Scales' `Cell size`, `Squareness` and `Tilt`; w is padding.
    velvet_form: Float4,
    /// The wash's `Edge pooling`, its width, and its softness as an exponent;
    /// w is padding.
    wash_pigment: Float4,
}
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
/// explicit tail. The starfield's scalars now fill the row the tone controls
/// left as padding; adding or dropping a field can move the edge again and this
/// catches it.
const _: () = assert!(
    std::mem::size_of::<Uniforms>().is_multiple_of(16),
    "the cloud uniform is not a whole number of 16-byte rows, so the shader's rounded-up \
     struct is larger than the buffer Rust writes",
);
/// WGSL puts an array in the uniform address space on a 16-byte boundary and a
/// `StarSlice` on a 16-byte stride, where `repr(C)` would pack both to four. So
/// both are held here rather than trusted: a scalar added before the array would
/// otherwise shift every slice by a word on the Rust side only. Likewise the
/// `vec2<i32>` a slice ends with, which WGSL aligns to eight where Rust's
/// `[i32; 2]` aligns to four.
const _: () = assert!(
    std::mem::offset_of!(Uniforms, star_slices).is_multiple_of(16)
        && std::mem::size_of::<StarSlice>().is_multiple_of(16)
        && std::mem::offset_of!(StarSlice, origin).is_multiple_of(8)
        && std::mem::offset_of!(Uniforms, star_halo_samples).is_multiple_of(16)
        && std::mem::size_of::<StarHaloSample>() == 16,
    "the star slices are not where the shader's 16-byte uniform layout reads them",
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
    /// read instead of working it out (a tile tap and its refraction) per pixel.
    pub tone: wgpu::RenderPipeline,
    pub velvet: wgpu::RenderPipeline,
    /// One period of the cell walk into the two tile targets, for both of the
    /// above to read instead of walking the ring at all.
    pub tile: wgpu::RenderPipeline,
    /// Every star on screen into the star atlas, once a frame, for the
    /// composite's walk to read instead of working each star out per pixel.
    pub stars: wgpu::RenderPipeline,
    /// RGB of the three farthest layers at the profile's selected resolution.
    pub star_far: wgpu::RenderPipeline,
    /// Reduced foreground over the far image, still gamma-encoded.
    pub star_near: wgpu::RenderPipeline,
    /// One depth's weighted halo color and coverage at the selected resolution.
    pub star_halo: wgpu::RenderPipeline,
    /// Specialize the final draws so the unsplit shader carries no runtime
    /// split branch (that branch alone regressed intermediate pane sizes).
    pub star_composite: wgpu::RenderPipeline,
    pub star_backdrop: wgpu::RenderPipeline,
    pub memory: wgpu::RenderPipeline,
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
                texture(13),
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 12,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                sampler_entry(7),
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
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
        let spectrogram = super::spectrogram_shader(device);
        Self {
            source: create_spectrogram_pipeline(
                device,
                &spectrogram,
                FORMAT,
                source_layout,
                None,
                "fs_density_source",
                false,
            ),
            bake: create_spectrogram_pipeline(
                device,
                &spectrogram,
                FORMAT,
                source_layout,
                Some(&composite_layout),
                "fs_cloud_light",
                false,
            ),
            velvet: create_spectrogram_pipeline(
                device,
                &spectrogram,
                FORMAT,
                source_layout,
                Some(&composite_layout),
                "fs_velvet_tone",
                false,
            ),
            tone: create_spectrogram_pipeline(
                device,
                &spectrogram,
                FORMAT,
                source_layout,
                Some(&composite_layout),
                "fs_cloud_tone",
                false,
            ),
            tile: tile_pipeline(
                device,
                &spectrogram,
                source_layout,
                &composite_layout,
                "fs_cloud_tile",
                &[Some(TILE_FORMAT), Some(TILE_FORMAT), Some(PIGMENT_FORMAT)],
            ),
            stars: tile_pipeline(
                device,
                &spectrogram,
                source_layout,
                &composite_layout,
                "fs_star_bake",
                &[Some(STAR_FORMAT)],
            ),
            star_halo: tile_pipeline(
                device,
                &spectrogram,
                source_layout,
                &composite_layout,
                "fs_star_halo",
                &[Some(STAR_FAR_FORMAT)],
            ),
            star_far: tile_pipeline(
                device,
                &spectrogram,
                source_layout,
                &composite_layout,
                "fs_star_far",
                &[Some(STAR_FAR_FORMAT)],
            ),
            star_near: tile_pipeline(
                device,
                &spectrogram,
                source_layout,
                &composite_layout,
                "fs_star_near",
                &[Some(STAR_FAR_FORMAT)],
            ),
            memory: tile_pipeline(
                device,
                &spectrogram,
                source_layout,
                &composite_layout,
                "fs_color_memory",
                &[Some(MEMORY_FORMAT)],
            ),
            composite: create_spectrogram_pipeline(
                device,
                &spectrogram,
                format,
                source_layout,
                Some(&composite_layout),
                if format.is_srgb() || format == wgpu::TextureFormat::Rgba16Float {
                    "fs_cloud_linear"
                } else {
                    "fs_cloud_gamma"
                },
                false,
            ),
            backdrop: create_spectrogram_pipeline(
                device,
                &spectrogram,
                format,
                source_layout,
                Some(&composite_layout),
                if format.is_srgb() || format == wgpu::TextureFormat::Rgba16Float {
                    "fs_cloud_backdrop_linear"
                } else {
                    "fs_cloud_backdrop_gamma"
                },
                false,
            ),
            star_composite: create_spectrogram_pipeline(
                device,
                &spectrogram,
                format,
                source_layout,
                Some(&composite_layout),
                if format.is_srgb() || format == wgpu::TextureFormat::Rgba16Float {
                    "fs_cloud_linear"
                } else {
                    "fs_cloud_gamma"
                },
                true,
            ),
            star_backdrop: create_spectrogram_pipeline(
                device,
                &spectrogram,
                format,
                source_layout,
                Some(&composite_layout),
                if format.is_srgb() || format == wgpu::TextureFormat::Rgba16Float {
                    "fs_cloud_backdrop_linear"
                } else {
                    "fs_cloud_backdrop_gamma"
                },
                true,
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

/// A bake: a full-screen triangle over its own targets, with no vertex buffer —
/// the tile's two, or the star atlas.
///
/// The tile declares the source layout at group 0 that it never reads, so the
/// pass can bind the same group every other cloud pass does; what it does read
/// is the cloud uniform at group 1, for the period and the style. The star
/// bake reads the palette there, and the light and the uniform at group 1.
fn tile_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    source_layout: &wgpu::BindGroupLayout,
    composite_layout: &wgpu::BindGroupLayout,
    entry: &str,
    formats: &[Option<wgpu::TextureFormat>],
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("spectral_cloud_tile_pipeline_layout"),
        bind_group_layouts: &[Some(source_layout), Some(composite_layout)],
        ..Default::default()
    });
    let targets = formats
        .iter()
        .map(|format| {
            format.map(|format| wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })
        })
        .collect::<Vec<_>>();
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_cloud_tile"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            targets: &targets,
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
/// rewritten from scratch each frame, where refilling these three is a whole cell
/// walk. So they are carried across a rebuild the light's size forces (see
/// `SpectrogramCallback::prepare`), and [`Self::baked`] is what says a bake is
/// owed rather than a reallocation.
pub(super) struct Tile {
    views: [wgpu::TextureView; 3],
    texels: u32,
    baked: Option<TileKey>,
}

pub(super) struct Memory {
    views: [wgpu::TextureView; 2],
    groups: [wgpu::BindGroup; 2],
    star_groups: [wgpu::BindGroup; 2],
    composite_groups: [wgpu::BindGroup; 2],
    size: [u32; 2],
    extent: [u32; 2],
    index: usize,
    frame: Option<MemoryFrame>,
}

struct MemoryFrame {
    now: f64,
    source_end: i64,
    key: Vec<u32>,
    palette: std::sync::Arc<Vec<[u8; 4]>>,
    origin: [i32; 2],
    slices: [StarSlice; STAR_SLICES],
    life: f32,
}

/// Only coordinate and color interpretation belong to the history key. Sound,
/// time, response times and Texture mix change the response, not its identity.
/// Other styles' dials must not erase the active style's carried color.
/// Stars locate history by absolute cell and life, independent of width and
/// motion. Keep height here because it changes the material sampling scale;
/// the actual atlas cell sizes are appended by `update` below.
fn memory_key(
    s: harmonigraph_scene::SpectralAtmosphere,
    size: [f32; 2],
    pitch_vertical: bool,
    read: &SpectrogramUniforms,
) -> Vec<u32> {
    use harmonigraph_scene::CloudStyle;
    let harmonigraph_scene::SpectralAtmosphere {
        pitch_softness,
        time_softness,
        spread,
        blur_time_step: _, // response/coverage changes do not change material identity
        contour_strength,
        contours,
        contour_softness,
        cloud_depth: _,   // response/coverage changes do not change material identity
        color_pickup: _,  // response/coverage changes do not change material identity
        color_release: _, // response/coverage changes do not change material identity
        cloud_speed,
        cloud_direction,
        wash_pool,
        wash_pool_width,
        wash_pool_softness,
        cloud_style,
        stars:
            harmonigraph_scene::StarSettings {
                star_randomness,
                star_size_variation: _, // core sizes do not change a star's colour
                star_jitter: _,         // each slice's band is appended where the cells are
                star_layers: _,         // read through the plan's drawn slices below
                star_spacing_ratio_far: _, // reaches the key as the cells
                star_spacing_ratio_near: _, // reaches the key as the cells
                star_spacing_ratio_curve: _, // reaches the key as the cells
                star_size_far: _,       // reaches the key as the cells
                star_size_near: _,      // reaches the key as the cells
                star_size_curve: _,     // reaches the key as the cells
                star_speed_far: _,      // carried by absolute cell and per-cell life
                star_speed_near: _,     // carried by absolute cell and per-cell life
                star_speed_curve: _,    // carried by absolute cell and per-cell life
                star_lifetime: _,       // carried by absolute cell and per-cell life
                star_twinkle_far: _,    // only whether a depth holds its stars, below
                star_twinkle_near: _,   // only whether a depth holds its stars, below
                star_halo_resolution: _, // sampling does not change material identity
                star_halo_profile: _,   // sampling does not change material identity
                star_solid_far: _,      // response/coverage changes do not change material identity
                star_solid_near: _,     // response/coverage changes do not change material identity
                star_glow_falloff: _,   // response/coverage changes do not change material identity
                star_solo: _,           // composition only; hidden slices keep their history
            },
        material_settings:
            harmonigraph_scene::MaterialSettings {
                velvet_size,
                velvet_variety,
                velvet_edge,
                velvet_irregularity,
                velvet_shape,
                velvet_square,
                velvet_tilt,
                wash_size,
                wash_fuzz,
                wash_lobe,
                wash_refract,
                wash_layers,
                wash_randomness: _, // display brightness does not change held color
            },
    } = s;
    let mut values = vec![
        size[1],
        u32::from(pitch_vertical) as f32,
        read.min_midi,
        read.span,
        read.level0,
        read.level_per_step,
        read.level_per_midi,
        pitch_softness,
        time_softness,
        spread,
    ];
    match cloud_style {
        CloudStyle::Stars => {
            values.extend([2.0, star_randomness]);
            // Each layer count draws its own set. Solo only hides composition
            // and leaves these slices running. Where the layers sit reaches
            // the key as the cells.
            values.extend(s.stars.plan().depths.map(|depth| {
                u32::from(depth.gather != harmonigraph_scene::star_plan::StarGather::Off) as f32
            }));
            // Whether each depth's stars keep their place across lives: that
            // moves every star and makes a life's colour its last one's. How
            // far a held star dips or blends is response, not identity.
            values.extend(s.stars.plan().depths.map(|depth| u32::from(depth.twinkle < 1.0) as f32));
        }
        CloudStyle::VelvetScales => values.extend([
            3.0,
            size[0],
            cloud_direction,
            cloud_speed,
            velvet_size,
            velvet_variety,
            velvet_edge,
            velvet_irregularity,
            velvet_shape,
            velvet_square,
            velvet_tilt,
            contours,
            contour_softness,
            contour_strength,
        ]),
        CloudStyle::Watercolor => values.extend([
            1.0,
            size[0],
            cloud_direction,
            cloud_speed,
            wash_size,
            wash_fuzz,
            wash_lobe,
            wash_refract,
            wash_layers,
            wash_pool,
            // Width and softness decide nothing while there is no tide line,
            // so they reset the history only once one is drawn.
            if wash_pool != 0.0 { wash_pool_width } else { 0.0 },
            if wash_pool != 0.0 { wash_pool_softness } else { 0.0 },
            contours,
            contour_softness,
            contour_strength,
        ]),
    }
    values.into_iter().map(f32::to_bits).collect()
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
    /// The precomposite and its size: reduced scalar cloud tone or RGB
    /// of the three far Stars layers (75% for High, 50% for Medium, one third for Low, native for Uniform).
    /// None works the texture out per pixel in the composite.
    /// Part of the allocation key beside [`Self::size`] — see
    /// `SpectrogramCallback::prepare`.
    pub tone: Option<(wgpu::TextureView, [u32; 2])>,
    /// Reduced foreground-over-far image; actual size is part of allocation identity.
    pub near: Option<(wgpu::TextureView, [u32; 2])>,
    /// Reads the far target, never the near target attached to its pass.
    pub near_group: Option<wgpu::BindGroup>,
    tile: Option<Tile>,
    /// The star atlas and its size, `None` unless the starfield is drawn. Part
    /// of the allocation key, sized by [`star_atlas_size`].
    stars: Option<(wgpu::TextureView, [u32; 2])>,
    halos: Option<StarHalos>,
    halo_group: Option<wgpu::BindGroup>,
    source_uniform: wgpu::Buffer,
    pub source_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    filter_groups: [wgpu::BindGroup; 3],
    pub bake_group: wgpu::BindGroup,
    /// Reads the baked material and writes the tone target, so the tone target
    /// is the one view this group must NOT carry.
    pub tone_group: Option<wgpu::BindGroup>,
    /// Writes all three tile targets, so those are the views it stands scratch
    /// in for.
    tile_group: Option<wgpu::BindGroup>,
    /// Writes the star atlas, so that is the view it stands a scratch in for.
    star_group: Option<wgpu::BindGroup>,
    pub composite_group: wgpu::BindGroup,
    memory: Option<Memory>,
}

/// Target shapes for the light field, optional tone/tile, star atlas, halos,
/// reduced foreground and retained color. Each follows its own sampling grid, so `prepare` compares
/// every shape before rebuilding. Cached tiles and color history can survive
/// allocation changes in the other fields.
pub(super) struct Allocation {
    pub size: [u32; 2],
    pub tone: Option<[u32; 2]>,
    pub near: Option<[u32; 2]>,
    pub tile: Option<TileKey>,
    pub carried: Option<Tile>,
    pub stars: Option<[u32; 2]>,
    pub halos: Option<StarHaloLayout>,
    pub memory: Option<[u32; 2]>,
    pub carried_memory: Option<Memory>,
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
        let Allocation {
            size,
            tone: tone_size,
            near: near_size,
            tile: tile_key,
            carried,
            stars: star_size,
            halos: halo_layout,
            memory: memory_size,
            carried_memory,
        } = wanted;
        let formatted = |label, size, format| crate::stars::image(device, label, size, format);
        let sized = |label, size: [u32; 2]| formatted(label, size, FORMAT);
        let view = |label| sized(label, size);
        let source_view = view("spectral_cloud_source");
        let views = [
            view("spectral_cloud_scratch"),
            view("spectral_cloud_close"),
            view("spectral_cloud_wide"),
        ];
        // Star presence is already part of the allocation key, so a style
        // change also replaces the tone's format even at identical sizes.
        let tone = tone_size.map(|size| {
            let format = if star_size.is_some() { STAR_FAR_FORMAT } else { FORMAT };
            (formatted("spectral_cloud_tone", size, format), size)
        });
        let near =
            near_size.map(|size| (formatted("spectral_star_near", size, STAR_FAR_FORMAT), size));
        let stars =
            star_size.map(|size| (formatted("spectral_star_atlas", size, STAR_FORMAT), size));
        // What every group that does not read the atlas binds in its place, and
        // what the star pass, which writes it, must.
        let star_scratch = formatted("spectral_star_scratch", [1, 1], STAR_FORMAT);
        let halos = halo_layout.map(|layout| StarHalos::new(device, layout));
        let halo_scratch =
            StarHalos::new(device, StarHaloLayout::from_sizes([Some([1, 1]); STAR_SLICES]));
        let carried_memory = carried_memory.filter(|m| Some(m.size) == memory_size);
        let memory_views = memory_size.map(|size| {
            carried_memory.as_ref().map_or_else(
                || {
                    ["color_memory_a", "color_memory_b"]
                        .map(|label| formatted(label, size, MEMORY_FORMAT))
                },
                |m| m.views.clone(),
            )
        });
        let (memory_index, memory_frame) = carried_memory.map_or((1, None), |m| (m.index, m.frame));
        // Reused whenever it is already the right shape, key and all, so a
        // rebuild the LIGHT's size forced costs no walk at all.
        let tile = tile_key.map(|key| match carried {
            Some(tile) if tile.texels == key.texels => tile,
            _ => Tile {
                views: [
                    ("spectral_cloud_tile_a", TILE_FORMAT),
                    ("spectral_cloud_tile_b", TILE_FORMAT),
                    ("spectral_cloud_tile_c", PIGMENT_FORMAT),
                ]
                .map(|(label, format)| formatted(label, [key.texels; 2], format)),
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
        let cloud_group =
            |front: &wgpu::TextureView,
             tone: &wgpu::TextureView,
             tile: [&wgpu::TextureView; 3],
             stars: &wgpu::TextureView,
             memory: &wgpu::TextureView,
             halos: &[wgpu::TextureView; STAR_HALO_GROUPS]| {
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
                            binding: 13,
                            resource: wgpu::BindingResource::TextureView(tile[2]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 7,
                            resource: wgpu::BindingResource::Sampler(&pipelines.tile_sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 9,
                            resource: wgpu::BindingResource::TextureView(memory),
                        },
                        wgpu::BindGroupEntry {
                            binding: 8,
                            resource: wgpu::BindingResource::TextureView(stars),
                        },
                        wgpu::BindGroupEntry {
                            binding: 10,
                            resource: wgpu::BindingResource::TextureView(&halos[0]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 11,
                            resource: wgpu::BindingResource::TextureView(&halos[1]),
                        },
                        wgpu::BindGroupEntry {
                            binding: 12,
                            resource: wgpu::BindingResource::TextureView(&halos[2]),
                        },
                    ],
                })
            };
        let scratch_tile = [&views[0], &views[0], &views[0]];
        let tile_views = tile.as_ref().map_or(scratch_tile, |tile| tile.views.each_ref());
        let star_view = stars.as_ref().map_or(&star_scratch, |(view, _)| view);
        let halo_view = halos.as_ref().map_or(&halo_scratch.views, |halo| &halo.views);
        let bake_group =
            cloud_group(&views[1], &views[0], tile_views, star_view, &views[0], halo_view);
        let halo_group = halos.as_ref().map(|_| {
            cloud_group(
                &source_view,
                &views[0],
                tile_views,
                star_view,
                &views[0],
                &halo_scratch.views,
            )
        });
        // Reads the baked material the light passes just wrote, and writes the
        // tone target — so that is the one view it stands a scratch in for.
        let tone_group = tone.as_ref().map(|_| {
            cloud_group(&source_view, &views[0], tile_views, star_view, &views[0], halo_view)
        });
        let tile_group = tile.as_ref().map(|_| {
            cloud_group(&source_view, &views[0], scratch_tile, star_view, &views[0], halo_view)
        });
        // Reads the finished light as the stars' level, like the tone pass.
        let star_group = stars.as_ref().map(|_| {
            cloud_group(&source_view, &views[0], tile_views, &star_scratch, &views[0], halo_view)
        });
        let near_group = near.as_ref().map(|_| {
            cloud_group(
                &source_view,
                &tone.as_ref().expect("reduced foreground has a far target").0,
                tile_views,
                star_view,
                &views[0],
                halo_view,
            )
        });
        let final_tone = near.as_ref().or(tone.as_ref()).map_or(&views[0], |(view, _)| view);
        let composite_group =
            cloud_group(&source_view, final_tone, tile_views, star_view, &views[0], halo_view);
        let memory = memory_views.map(|history| Memory {
            groups: std::array::from_fn(|i| {
                cloud_group(
                    &source_view,
                    final_tone,
                    tile_views,
                    &star_scratch,
                    &history[1 - i],
                    halo_view,
                )
            }),
            star_groups: std::array::from_fn(|i| {
                cloud_group(
                    &source_view,
                    &views[0],
                    tile_views,
                    &star_scratch,
                    &history[i],
                    halo_view,
                )
            }),
            composite_groups: std::array::from_fn(|i| {
                cloud_group(&source_view, final_tone, tile_views, star_view, &history[i], halo_view)
            }),
            views: history,
            size: memory_size.unwrap(),
            extent: memory_size.unwrap(),
            index: memory_index,
            frame: memory_frame,
        });
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
            near,
            near_group,
            tile,
            stars,
            halos,
            halo_group,
            source_uniform,
            source_group,
            uniform,
            filter_groups,
            bake_group,
            tone_group,
            tile_group,
            star_group,
            composite_group,
            memory,
        }
    }

    /// The precomposite target's size, for the allocation key to compare
    /// against what this frame's settings ask for.
    pub fn tone_size(&self) -> Option<[u32; 2]> {
        self.tone.as_ref().map(|&(_, size)| size)
    }

    pub fn near_size(&self) -> Option<[u32; 2]> {
        self.near.as_ref().map(|&(_, size)| size)
    }

    /// The star atlas's size, for the allocation key.
    pub fn star_size(&self) -> Option<[u32; 2]> {
        self.stars.as_ref().map(|&(_, size)| size)
    }

    pub fn halo_layout(&self) -> Option<StarHaloLayout> {
        self.halos.as_ref().map(|halo| halo.layout)
    }

    pub fn draw_stars(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &Pipelines,
        pixels: [u32; 2],
        coverage: [u32; 4],
        near_coverage: Option<[u32; 4]>,
    ) {
        let Some((atlas, atlas_group)) = self.star_pass() else { return };
        let halos = self.halos.as_ref().expect("star halos");
        let bake = [&self.source_group, atlas_group];
        let halo = [&self.source_group, self.halo_group.as_ref().expect("halo group")];
        let far = self.tone_group.as_ref().map(|g| [&self.source_group, g]);
        let near = self.near_group.as_ref().map(|g| [&self.source_group, g]);
        #[cfg(test)]
        self.encoded_passes.fetch_add(
            1 + halos.layers.iter().flatten().count() as u32
                + u32::from(self.tone.is_some())
                + u32::from(self.near.is_some()),
            std::sync::atomic::Ordering::Relaxed,
        );
        crate::stars::draw(
            encoder,
            crate::stars::Pass {
                view: atlas,
                pipeline: &pipelines.stars,
                groups: &bake,
                scissor: None,
            },
            halos,
            &pipelines.star_halo,
            &halo,
            self.tone.as_ref().zip(far.as_ref()).map(|((view, size), groups)| crate::stars::Pass {
                view,
                pipeline: &pipelines.star_far,
                groups,
                scissor: Some(star_far_scissor(
                    near_coverage.unwrap_or(coverage),
                    self.near_size().unwrap_or(pixels),
                    *size,
                )),
            }),
            self.near.as_ref().zip(near.as_ref()).map(|((view, _), groups)| crate::stars::Pass {
                view,
                pipeline: &pipelines.star_near,
                groups,
                scissor: near_coverage,
            }),
        );
    }

    /// The star atlas and the group the pass that fills it binds.
    pub fn star_pass(&self) -> Option<(&wgpu::TextureView, &wgpu::BindGroup)> {
        Some((
            &self.stars.as_ref()?.0,
            self.memory.as_ref().map_or(self.star_group.as_ref()?, |m| &m.star_groups[m.index]),
        ))
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

    /// The tile's three targets and the group the pass that writes them binds.
    pub fn tile_pass(&self) -> Option<(&[wgpu::TextureView; 3], &wgpu::BindGroup)> {
        Some((&self.tile.as_ref()?.views, self.tile_group.as_ref()?))
    }

    /// Records the key the tile now holds. Called once the bake is encoded.
    pub fn tile_baked(&mut self, key: TileKey) {
        if let Some(tile) = self.tile.as_mut() {
            tile.baked = Some(key);
        }
    }

    /// The baked tile, for a rebuilt set of targets to carry across.
    pub fn take_memory(&mut self) -> Option<Memory> {
        self.memory.take()
    }

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

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        mut read: SpectrogramUniforms,
        rect: egui::Rect,
        ppp: f32,
        atmosphere: SpectrogramAtmosphere,
        tile: Option<TileKey>,
        stars: Option<StarLayout>,
        memory_extent: Option<[u32; 2]>,
        palette: std::sync::Arc<Vec<[u8; 4]>>,
        source_end: i64,
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
        // the same point the composite would have read under its own pixel.
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
        read.origin_points = Float2(rect.min.into());
        read.viewport_points = Float2(rect.size().into());
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
        let offset = cloud_offset(settings, atmosphere.now);
        let drift = cloud_drift(settings, offset, tile);
        let slices = stars
            .map(|layout| {
                star_slices(settings.stars, settings.cloud_direction, atmosphere.now, &layout)
            })
            .unwrap_or_default();
        let life = star_life(settings.stars, atmosphere.now);
        let mut memory_valid = false;
        let mut alphas = [1.0; 2];
        let mut previous_slices = slices;
        let mut previous_life = life;
        let mut memory_shift = [0; 2];
        let mut memory_fraction = [0.0; 2];
        if let Some(memory) = self.memory.as_mut() {
            memory.extent = memory_extent.expect("allocated history has a logical extent");
            // Off the UNREDUCED offset: the history's lattice has to run on
            // continuously where the shader's drift jumps a whole repeat, or
            // every wrap would read as a seek and reset it.
            let origin = std::array::from_fn(|a| {
                let texels = offset[a]
                    * f64::from(
                        rect.height() / CLOUD_UNITS * (memory.extent[a] - 2) as f32
                            / rect.size()[a],
                    );
                let integer = texels.floor();
                memory_fraction[a] = (texels - integer) as f32;
                integer as i32
            });
            let mut key = memory_key(settings, rect.size().into(), pitch_vertical, &read);
            // A resize changes musical density, hence the light a star samples,
            // but not the absolute cell that owns its carried color.
            if stars.is_none() {
                key.extend([
                    atmosphere.points_per_ms.to_bits(),
                    atmosphere.points_per_cent.to_bits(),
                ]);
            }
            // Different logical grids can now share one allocation. DPI or
            // sampling changes must still reset history even in the same bucket.
            key.extend(memory.extent);
            if let Some(layout) = stars {
                // Stars carry by absolute cell and life across motion edits
                // and width changes. At the atlas budget, a wider pane can
                // coarsen cells: the same integer cell then names a new star.
                // A slice's band moves every centre in it.
                key.extend(layout.cells.map(f32::to_bits));
                key.extend(slices.iter().map(|slice| slice.width.to_bits()));
            }
            if let Some(previous) = &memory.frame {
                let dt = atmosphere.now - previous.now;
                // Six maximum time constants leave under 0.25% residual;
                // ordinary low-rate exports still integrate their actual dt.
                let horizon = 6.0 * f64::from(harmonigraph_scene::atmosphere::COLOR_MEMORY_MAX);
                memory_valid = dt.is_finite()
                    && (0.0..=horizon).contains(&dt)
                    && source_end >= previous.source_end
                    && key == previous.key
                    && (std::sync::Arc::ptr_eq(&palette, &previous.palette)
                        || palette == previous.palette);
                if memory_valid {
                    alphas = [settings.color_pickup, settings.color_release].map(|tau| {
                        if dt == 0.0 {
                            0.0
                        } else if tau <= 0.0 {
                            1.0
                        } else {
                            -(-(dt as f32) / tau).exp_m1()
                        }
                    });
                    previous_slices = previous.slices;
                    previous_life = previous.life;
                    memory_shift =
                        std::array::from_fn(|a| origin[a].saturating_sub(previous.origin[a]));
                }
            }
            memory.index = 1 - memory.index;
            memory.frame = Some(MemoryFrame {
                now: atmosphere.now,
                source_end,
                key,
                palette,
                origin,
                slices,
                life,
            });
        }
        let uniforms = Uniforms {
            velvet: Float4([
                settings.material_settings.velvet_edge,
                settings.material_settings.velvet_irregularity,
                settings.material_settings.velvet_shape,
                settings.material_settings.velvet_variety,
            ]),
            velvet_form: Float4([
                settings.material_settings.velvet_size,
                settings.material_settings.velvet_square,
                settings.material_settings.velvet_tilt,
                0.0,
            ]),
            wash_pigment: Float4([
                settings.wash_pool,
                settings.wash_pool_width,
                harmonigraph_scene::SpectralAtmosphere::pool_exponent(settings.wash_pool_softness),
                0.0,
            ]),
            origin: Float2(rect.min.into()),
            size: Float2(rect.size().into()),
            step: Float2([radius[0] / rect.width(), radius[1] / rect.height()]),
            ppp,
            spread: settings.spread,
            contours: settings.contours,
            contour_softness: settings.contour_softness,
            // At full Stars mix the underlying terraced picture is hidden.
            contour_strength: if settings.cloud_style == harmonigraph_scene::CloudStyle::Stars
                && settings.cloud_depth >= 1.0
            {
                0.0
            } else {
                settings.contour_strength
            },
            tone_baked: u32::from(self.tone.is_some()),
            drift: Float2(drift),
            cloud_depth: if settings.effects().cloud { settings.cloud_depth } else { 0.0 },
            cloud_style: match settings.cloud_style {
                harmonigraph_scene::CloudStyle::Watercolor => 1,
                harmonigraph_scene::CloudStyle::Stars => 2,
                harmonigraph_scene::CloudStyle::VelvetScales => 3,
            },
            wash_size: settings.material_settings.wash_size,
            wash_fuzz: settings.material_settings.wash_fuzz,
            wash_lobe: settings.material_settings.wash_lobe,
            wash_refract: settings.material_settings.wash_refract,
            wash_layers: settings.material_settings.wash_layers,
            // Zero only where no tile was allocated, which is where no cloud is
            // drawn and the shader returns before the tone.
            tile_cells: tile.map_or(0, TileKey::period),
            pitch_vertical: u32::from(pitch_vertical),
            star_randomness: settings.stars.star_randomness,
            star_life: star_life(settings.stars, atmosphere.now),
            star_size_variation: settings.stars.star_size_variation,
            star_pad0: 0,
            star_pad1: 0,
            star_far: {
                let [width, height] = self.tone_size().unwrap_or([1, 1]);
                Float4([
                    width as f32,
                    height as f32,
                    f32::from(crate::stars::star_far_reduced(settings.stars)),
                    0.0,
                ])
            },
            star_near: {
                let [width, height] = self.near_size().unwrap_or([0, 0]);
                Float4([width as f32, height as f32, 0.0, 0.0])
            },
            star_slices: slices,
            memory_enabled: u32::from(self.memory.is_some()),
            memory_valid: u32::from(memory_valid),
            pickup_alpha: alphas[0],
            release_alpha: alphas[1],
            memory_shift: Int2(memory_shift),
            memory_fraction: Float2(memory_fraction),
            previous_life,
            wash_randomness: settings.material_settings.wash_randomness,
            memory_extent: Float2(memory_extent.unwrap_or([0; 2]).map(|n| n as f32)),
            previous_slices,
            star_halo_samples: self
                .halo_layout()
                .unwrap_or_else(|| StarHaloLayout::from_sizes([Some([1, 1]); STAR_SLICES]))
                .samples(),
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniforms));
    }

    pub fn invalidate_memory(&mut self) {
        self.memory = None;
    }

    pub fn memory_size(&self) -> Option<[u32; 2]> {
        self.memory.as_ref().map(|m| m.size)
    }

    pub fn composite_group(&self) -> &wgpu::BindGroup {
        self.memory.as_ref().map_or(&self.composite_group, |m| &m.composite_groups[m.index])
    }

    pub fn remember(&self, encoder: &mut wgpu::CommandEncoder, pipelines: &Pipelines) {
        let Some(memory) = &self.memory else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("spectral_color_memory"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &memory.views[memory.index],
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_viewport(0.0, 0.0, memory.extent[0] as f32, memory.extent[1] as f32, 0.0, 1.0);
        pass.set_scissor_rect(0, 0, memory.extent[0], memory.extent[1]);
        pass.set_pipeline(&pipelines.memory);
        pass.set_bind_group(0, &self.source_group, &[]);
        pass.set_bind_group(1, &memory.groups[memory.index], &[]);
        pass.draw(0..3, 0..1);
    }

    /// The two filter scales. `wide` is false at a `Wide blur mix` of 0, the
    /// fresh one, where the wide pair is skipped: `fs_cloud_light` reads
    /// `mix(close, wide, 0)`, which is `close` exactly for any finite texel,
    /// and the wide target only ever holds zeros or an earlier frame's filter
    /// output.
    pub fn blur(&self, encoder: &mut wgpu::CommandEncoder, pipelines: &Pipelines, wide: bool) {
        // Source -> scratch -> close; close -> scratch -> wide. Feeding the
        // already softened image to the wide kernel closes its sampling gaps.
        // Every pass reads a different texture from the attachment it writes.
        let passes = if wide { 4 } else { 2 };
        for (i, (input, output)) in
            [(0, 0), (1, 1), (2, 0), (1, 2)].into_iter().enumerate().take(passes)
        {
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
        cloud_drift, cloud_offset, retained_size, source_size, star_layout, star_slices, tile_key,
        tone_size, SpectrogramAtmosphere, CLOUD_UNITS, STAR_ATLAS_WIDTH, STAR_HASH_PERIOD,
        STAR_LIFE_PERIOD, STAR_PANE, STAR_SLICES, TILE_MAX, TILE_STEP, WASH_CELLS,
    };

    /// Every slice at `now` over a 16:9 pane.
    fn slices(
        settings: harmonigraph_scene::SpectralAtmosphere,
        now: f64,
    ) -> [super::StarSlice; STAR_SLICES] {
        star_slices(
            settings.stars,
            settings.cloud_direction,
            now,
            &star_layout(settings.stars, 16.0 / 9.0),
        )
    }

    /// One `Star size` value is one star at every depth, whatever the spacing
    /// does across them, and at any size: the spacing is a multiple of it, so
    /// no slice draws a star smaller than asked.
    #[test]
    fn one_star_size_is_one_star_at_every_depth() {
        let mut settings = harmonigraph_scene::SpectralAtmosphere::default();
        for size in [4.0, harmonigraph_scene::STAR_SIZE_MAX] {
            (settings.stars.star_size_far, settings.stars.star_size_near) = (size, size);
            let slices = slices(settings, 0.0);
            assert!(slices[0].cell > slices[STAR_SLICES - 1].cell * 1.4, "spacing must vary");
            for slice in &slices {
                assert_eq!(slice.radius, 0.5 * size);
            }
        }
    }

    fn shader_number(name: &str) -> f64 {
        crate::shadow::tests::shader_const(crate::spectrogram::SPECTROGRAM_SRC, name)
            .trim_end_matches('u')
            .trim()
            .parse()
            .expect("a number")
    }

    #[test]
    fn spectral_uniforms_match_the_bound_shader_layouts() {
        crate::uniforms::layout::check_binding::<super::Uniforms>(
            super::super::SPECTROGRAM_SRC,
            1,
            3,
        );
        crate::uniforms::layout::check_binding_prefix::<super::Uniforms>(super::SOURCE, 0, 2);
    }

    #[test]
    fn halo_resolution_rounds_each_axis_without_losing_tiny_targets() {
        for (resolution, wanted) in
            [(0.25, [1, 2]), (1.0 / 3.0, [1, 2]), (0.5, [2, 3]), (1.0, [3, 5])]
        {
            assert_eq!(super::star_halo_size([3, 5], resolution), wanted);
            assert_eq!(super::star_halo_size([1, 1], resolution), [1, 1]);
        }
    }

    #[test]
    fn halo_profiles_group_actual_sizes_and_keep_depth_addresses() {
        use harmonigraph_scene::{SpectralAtmosphere, StarHaloProfile};
        let medium = SpectralAtmosphere::default();
        assert_eq!(medium.stars.star_halo_profile, StarHaloProfile::Medium);
        let mut settings = medium;
        settings.stars.star_halo_profile = StarHaloProfile::P3;
        let layout = super::star_halo_layout([161, 121], settings.stars);
        assert_eq!(layout.groups.map(|g| g.size), [[161, 121], [97, 73], [1, 1]]);
        assert_eq!(layout.groups.map(|g| g.layers), [1, 1, 0]);
        assert_eq!(layout.layers, [[0, 0], [0, 0], [0, 0], [0, 0], [1, 0]]);
        let layout = super::star_halo_layout([161, 121], medium.stars);
        assert_eq!(layout.groups.map(|g| g.size), [[121, 91], [73, 55], [1, 1]]);
        assert_eq!(layout.groups.map(|g| g.layers), [1, 1, 0]);
        assert_eq!(super::star_far_size([161, 121], medium.stars), [81, 61]);
        assert_eq!(super::star_near_size([161, 121], medium.stars), Some([121, 91]));
        assert_eq!(super::star_near_size([161, 121], settings.stars), None);
        let low = harmonigraph_scene::StarSettings {
            star_halo_profile: StarHaloProfile::Low,
            ..settings.stars
        };
        let layout = super::star_halo_layout([161, 121], low);
        assert_eq!(layout.groups.map(|g| g.size), [[81, 61], [49, 37], [1, 1]]);
        assert_eq!(layout.groups.map(|g| g.layers), [1, 1, 0]);
        assert_eq!(layout.layers, [[0, 0], [0, 0], [0, 0], [0, 0], [1, 0]]);
        assert_eq!(super::star_far_size([161, 121], low), [54, 41]);
        assert_eq!(super::star_near_size([161, 121], low), Some([81, 61]));
        // Uniform gives every 3x3 depth one resolution; at the fresh sizes
        // only the near two need 3x3, so they share one image.
        let uniform =
            harmonigraph_scene::StarSettings { star_halo_profile: StarHaloProfile::Uniform, ..low };
        let layout = super::star_halo_layout([161, 121], uniform);
        assert_eq!(layout.active, [false, false, false, true, true]);
        assert_eq!(
            layout.groups.map(|g| (g.size, g.layers)),
            [([81, 61], 2), ([1, 1], 0), ([1, 1], 0)]
        );
        assert_eq!(layout.layers, [[0, 0], [0, 0], [0, 0], [0, 0], [0, 1]]);
    }

    /// A depth `Star layers` leaves out holds no atlas cells, owns no halo
    /// image and reaches the shader as not drawn; the others are laid out as
    /// with every layer.
    #[test]
    fn a_layer_left_out_is_not_allocated_or_baked() {
        let every = harmonigraph_scene::StarSettings::default();
        let three = harmonigraph_scene::StarSettings { star_layers: 3, ..every };
        let off = [false, true, false, true, false];
        let gathers = three.plan().depths.map(|depth| depth.gather);
        assert_eq!(gathers.map(|g| g == harmonigraph_scene::star_plan::StarGather::Off), off);

        let (layout, full) = (star_layout(three, 16.0 / 9.0), star_layout(every, 16.0 / 9.0));
        let mut left_out = 0u64;
        for (k, &off) in off.iter().enumerate() {
            if off {
                assert_eq!(layout.grids[k], [0, 0], "depth {k}");
                left_out += u64::from(full.grids[k][0] * full.grids[k][1]);
            } else {
                assert_ne!(layout.grids[k], [0, 0], "depth {k}");
            }
        }
        assert_eq!(layout.texels, full.texels - left_out);
        let active = super::star_halo_layout([161, 121], three).active;
        assert!(off.iter().zip(active).all(|(&off, active)| !(off && active)));
        let slices = star_slices(three, 0.0, 0.0, &layout);
        assert!(off.iter().zip(slices).all(|(&off, slice)| off == (slice.gather == 0)));
    }

    /// Solo shows a layer as the whole field draws it. On a pane wide enough
    /// that the atlas floors the finest cells, hiding the other layers
    /// leaves the whole layout and each layer's cells in place.
    #[test]
    fn soloing_keeps_the_cells_the_whole_field_draws() {
        use harmonigraph_scene::star_plan::STAR_DEPTHS;
        let every = harmonigraph_scene::StarSettings::default();
        let full = star_layout(every, 8.0);
        assert_ne!(full.cells, crate::stars::star_cells(every), "the fixture must floor a cell");
        for k in 0..STAR_DEPTHS {
            let mut star_solo = [false; STAR_DEPTHS];
            star_solo[k] = true;
            let solo = harmonigraph_scene::StarSettings { star_solo, ..every };
            assert_eq!(star_layout(solo, 8.0), full, "solo {k}");
        }
    }

    /// Solo changes only composition; hidden slices still bake and keep memory.
    /// A flag on a depth omitted by Star layers solos nothing.
    #[test]
    fn soloing_draws_only_the_soloed_layers() {
        let fresh = harmonigraph_scene::StarSettings::default();
        let solo = [false, true, false, true, false];
        for layers in [3, 5] {
            let full = harmonigraph_scene::StarSettings { star_layers: layers, ..fresh };
            let selected = harmonigraph_scene::StarSettings { star_solo: solo, ..full };
            let layout = star_layout(full, 16.0 / 9.0);
            let slices = star_slices(selected, 37.0, 5.0, &layout);
            let baseline = star_slices(full, 37.0, 5.0, &layout);
            for (k, (got, mut expected)) in slices.into_iter().zip(baseline).enumerate() {
                if layers == 5 && !solo[k] {
                    expected.gather = 0;
                }
                assert_eq!(got, expected, "layers {layers}, depth {k}");
            }
        }
    }

    /// The halo images are allocated for exactly the depths the plan draws
    /// 3x3, at the lattice's scale too, scaled past the range a stored size
    /// can hold: Star size 20 shows as 100 there. A spacing is a multiple of
    /// the size, so the lattice reads every depth as the stored settings do.
    #[test]
    fn halos_follow_the_drawn_plan_at_the_lattice_scale() {
        use harmonigraph_scene::star_plan::StarGather;
        let stored =
            harmonigraph_scene::StarSettings { star_size_near: 20.0, ..Default::default() };
        let lattice = stored.scaled(harmonigraph_scene::LATTICE_STAR_SIZE_SCALE);
        let three = lattice.plan().depths.map(|depth| depth.gather == StarGather::Three);
        assert_eq!(three, stored.plan().depths.map(|d| d.gather == StarGather::Three));
        assert_eq!(super::star_halo_layout([161, 121], lattice).active, three);
    }

    /// Both walks include every star that can reach the pixel: one nominal
    /// cell for a star's inner part, and a 3x3 ring for the whole star. Every
    /// plan holds its stars inside its gather's bound, however big the dials
    /// ask for them.
    #[test]
    fn the_star_ring_holds_every_star_that_reaches_a_pixel() {
        use harmonigraph_scene::star_plan::{star_jitter_width, StarGather};
        use harmonigraph_scene::StarHaloProfile;
        assert_eq!(STAR_SLICES as f64, shader_number("STAR_SLICES"));
        assert_eq!(STAR_HASH_PERIOD, shader_number("STAR_HASH_PERIOD"));
        assert_eq!(STAR_LIFE_PERIOD, shader_number("STAR_LIFE_PERIOD"));
        let fade = shader_number("STAR_INNER_FADE");
        assert!(fade > 0.0 && fade < 1.0);
        for dial in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let jitter = star_jitter_width(dial);
            let core = StarGather::Core.bound(dial);
            for (radius, reach) in [(0, core), (1, StarGather::Three.bound(dial))] {
                let nearest = nearest_outside_the_ring(jitter / 2.0, radius);
                assert!(nearest >= reach - 1e-5,
                    "jitter={dial}, ring={radius}: excluded star at {nearest}, inside reach {reach}");
            }
            for profile in [
                StarHaloProfile::Uniform,
                StarHaloProfile::P3,
                StarHaloProfile::Medium,
                StarHaloProfile::Low,
            ] {
                for size in [harmonigraph_scene::STAR_SIZE_MIN, harmonigraph_scene::STAR_SIZE_MAX] {
                    let settings = harmonigraph_scene::StarSettings {
                        star_jitter: dial,
                        star_halo_profile: profile,
                        star_size_near: size,
                        ..Default::default()
                    };
                    for depth in settings.plan().depths {
                        let bound = depth.gather.bound(dial) * depth.cell;
                        assert!(depth.radius <= bound, "{profile:?} {depth:?}");
                    }
                }
            }
        }
    }

    /// Scan a full cell and both sides of its selection boundaries. A centre
    /// strays `stray` on each axis; the walk is centred on floor(pixel).
    fn nearest_outside_the_ring(stray: f32, radius: i32) -> f32 {
        let mut nearest = f32::INFINITY;
        for step in 0..=64 {
            for other in 0..=64 {
                let pixel = [step as f32 / 64.0, other as f32 / 64.0];
                let centre = pixel.map(|p| p.floor() as i32);
                for cx in -2i32..=3 {
                    for cy in -2i32..=3 {
                        if (centre[0] - radius..=centre[0] + radius).contains(&cx)
                            && (centre[1] - radius..=centre[1] + radius).contains(&cy)
                        {
                            continue;
                        }
                        let toward = |c: i32, p: f32| {
                            (c as f32 + 0.5 - stray).max(p).min(c as f32 + 0.5 + stray)
                        };
                        let star = [toward(cx, pixel[0]), toward(cy, pixel[1])];
                        nearest = nearest.min(
                            ((star[0] - pixel[0]).powi(2) + (star[1] - pixel[1]).powi(2)).sqrt(),
                        );
                    }
                }
            }
        }
        nearest
    }

    /// The shader reads a cell by its index in its slice's grid with no bounds
    /// check, so a grid one cell short would draw a star from another row, or
    /// another slice, silently. This replays the shader's f32 arithmetic at
    /// both edges of the pane on each axis — the walk's extremes — with drifts
    /// up to 4096 f32 steps either side of the ones that put an edge exactly on
    /// a cell boundary, near zero and near the hash period's wrap, on real pixel
    /// sizes at real display scales, the floored fine layout and a strip.
    ///
    /// It holds the grid's own sizing and NOT `STAR_GRID_MARGIN`: it passes at
    /// a margin of 0 too, because a correctly rounded replay cannot floor below
    /// the CPU's exact edge. The margin is for the GPU's division, which this
    /// cannot reach.
    #[test]
    fn the_star_atlas_holds_every_cell_the_walk_reads() {
        assert_eq!(f64::from(STAR_PANE), shader_number("STAR_PANE"));
        assert_eq!(f64::from(STAR_ATLAS_WIDTH), shader_number("STAR_ATLAS_WIDTH"));
        assert_eq!(STAR_ATLAS_WIDTH, 1 << shader_number("STAR_ATLAS_SHIFT") as u32);
        let fresh = harmonigraph_scene::SpectralAtmosphere::default();
        let fine = harmonigraph_scene::SpectralAtmosphere {
            stars: harmonigraph_scene::StarSettings {
                star_spacing_ratio_far: harmonigraph_scene::STAR_SPACING_MIN,
                star_size_far: harmonigraph_scene::STAR_SIZE_MIN,
                ..fresh.stars
            },

            ..fresh
        };
        let panes = [
            (fresh, [3840u32, 2160u32], 2.0f32),
            (fresh, [1531, 877], 1.5),
            (fine, [2559, 1439], 2.0),
            (fresh, [3001, 187], 1.0),
        ];
        for (settings, pixels, ppp) in panes {
            let aspect = pixels[0] as f32 / pixels[1] as f32;
            let layout = star_layout(settings.stars, aspect);
            assert!(layout.fits(), "{layout:?}");
            let size = pixels.map(|side| side as f32 / ppp);
            for (k, slice) in star_slices(settings.stars, settings.cloud_direction, 0.0, &layout)
                .iter()
                .enumerate()
            {
                for axis in 0..2 {
                    let half = f64::from(layout.pane[axis] / 2.0 / slice.cell);
                    // Drifts that put the leading edge, then the trailing one, on
                    // a selection boundary: `-half - offset` and
                    // `half - offset` integers.
                    let exact = [0.0, 1000.0, STAR_HASH_PERIOD - 2.0 * half - 8.0]
                        .into_iter()
                        .flat_map(|m| [half.ceil() + m - half, half - (half.floor() - m)]);
                    for exact in exact {
                        let exact = exact as f32;
                        for ulps in -4096i32..=4096 {
                            let offset = f32::from_bits((exact.to_bits() as i32 + ulps) as u32);
                            let origin = super::star_origin(layout.pane[axis], slice.cell, offset);
                            for pt in [0.0, size[axis]] {
                                let sp = (pt - size[axis] * 0.5) * (STAR_PANE / size[1]);
                                let whole = offset.floor();
                                let cell = (sp / slice.cell - (offset - whole)).floor() as i32
                                    - whole as i32;
                                for step in [-1, 0, 1] {
                                    let local = cell + step - origin;
                                    assert!(
                                        (0..slice.grid.0[axis]).contains(&local),
                                        "{aspect}, slice {k}, axis {axis}, offset {offset}: \
                                         cell {local} of {:?}",
                                        slice.grid
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// The atlas takes the dials' own cells wherever it can hold them — the
    /// coarse starfield on a plain and a very wide pane — and otherwise raises
    /// only the finest cells, to the least that fits.
    #[test]
    fn the_star_atlas_floors_only_the_finest_cells_and_only_past_its_budget() {
        // Exercise the below-budget path independently of the current look defaults.
        let coarse = harmonigraph_scene::SpectralAtmosphere {
            stars: harmonigraph_scene::StarSettings {
                star_spacing_ratio_far: 2.5,
                star_spacing_ratio_near: 1.3,
                star_spacing_ratio_curve: 1.0,
                ..Default::default()
            },

            ..Default::default()
        };
        for aspect in [16.0 / 9.0, 8.0] {
            let layout = star_layout(coarse.stars, aspect);
            assert_eq!(layout.cells, super::star_cells(coarse.stars), "{aspect}");
        }
        let fine = harmonigraph_scene::SpectralAtmosphere {
            stars: harmonigraph_scene::StarSettings {
                star_spacing_ratio_far: harmonigraph_scene::STAR_SPACING_MIN,
                star_size_far: harmonigraph_scene::STAR_SIZE_MIN,
                star_spacing_ratio_near: 0.42,
                ..coarse.stars
            },

            ..coarse
        };
        let wanted = super::star_cells(fine.stars);
        let layout = star_layout(fine.stars, 16.0 / 9.0);
        let floor = layout.cells[0];
        assert!(floor > wanted[0], "{layout:?}");
        assert!(
            !super::StarLayout::at(wanted, [true; STAR_SLICES], floor * 0.99, 16.0 / 9.0).fits()
        );
        for (got, want) in layout.cells.iter().zip(wanted) {
            assert_eq!(*got, want.max(floor));
        }
        assert_eq!(layout.cells[STAR_SLICES - 1], wanted[STAR_SLICES - 1]);
    }

    /// Every depth drifts along `Drift direction` at its own speed between
    /// `Star speed`'s ends, and a depth at 1 at the prototype's pace: about a
    /// ninth of the pane's height a second. Equal ends are no parallax at all,
    /// and the other textures' `Drift speed` moves no star.
    ///
    /// Read as SCREEN travel, `offset * cell`, which is what the eye sees —
    /// the offsets themselves are in each slice's own cells.
    #[test]
    fn stars_drift_with_parallax_at_the_prototypes_pace() {
        // Held at the prototype's parallax, which is what the pace below is
        // stated against, whatever the fresh far speed is.
        let fresh = harmonigraph_scene::SpectralAtmosphere {
            stars: harmonigraph_scene::StarSettings {
                star_speed_far: 0.15,
                star_speed_near: 1.0,
                ..Default::default()
            },
            cloud_direction: 0.0,

            ..Default::default()
        };
        let travelled = |settings, now| {
            slices(settings, now)
                .map(|slice| [slice.offset.0[0] * slice.cell, slice.offset.0[1] * slice.cell])
        };
        let near = super::star_px_per_second() as f32 * 10.0;
        assert!((near / 540.0 / 10.0 - 0.114).abs() < 0.001);
        let moved = travelled(fresh, 10.0);
        assert!((moved[STAR_SLICES - 1][0] - near).abs() < 0.01, "{moved:?}");
        assert!((moved[0][0] - 0.15 * near).abs() < 0.01, "{moved:?}");
        assert!(moved.iter().all(|m| m[1].abs() < 1e-3));
        assert!(moved.windows(2).all(|w| w[0][0] < w[1][0]), "nearer is not faster: {moved:?}");
        let together = travelled(
            harmonigraph_scene::SpectralAtmosphere {
                stars: harmonigraph_scene::StarSettings { star_speed_far: 1.0, ..fresh.stars },
                ..fresh
            },
            10.0,
        );
        assert!(together.iter().all(|m| (m[0] - near).abs() < 0.01), "{together:?}");
        let clouds = harmonigraph_scene::SpectralAtmosphere { cloud_speed: 20.0, ..fresh };
        assert_eq!(travelled(clouds, 10.0), moved);
        // A session left running for days still hands the shader an offset
        // inside one hash period rather than millions of pixels.
        for slice in slices(fresh, 3.0e5) {
            assert!(slice.offset.0.iter().all(|&o| (0.0..=STAR_HASH_PERIOD as f32).contains(&o)));
        }
    }

    #[test]
    fn velvet_tone_density_tracks_size_and_softness_without_a_geometry_tile() {
        let mut a = SpectrogramAtmosphere {
            settings: harmonigraph_scene::SpectralAtmosphere {
                cloud_style: harmonigraph_scene::CloudStyle::VelvetScales,
                cloud_depth: 1.0,
                ..Default::default()
            },
            region: egui::Rect::ZERO,
            pitch_vertical: true,
            points_per_cent: 0.03,
            points_per_ms: 0.01,
            points_per_slab: 0.0,
            now: 0.0,
        };
        let size = [1920, 1080];
        let density = |a| tone_size(size, 2.0, a, 0.5);
        assert_eq!(density(a), Some([960, 540]));
        assert_eq!(tile_key(size, a, 40), None);
        a.settings.material_settings.velvet_size = 1.01;
        assert_eq!(density(a), Some([960, 540]), "small dial steps should not reallocate");
        a.settings.cloud_speed = 15.0;
        a.now = 100_000.0;
        a.settings.material_settings.velvet_irregularity = 1.0;
        a.settings.material_settings.velvet_shape = 0.0;
        a.settings.material_settings.velvet_variety = 1.0;
        a.settings.material_settings.wash_fuzz = 0.0;
        assert_eq!(
            density(a),
            Some([960, 540]),
            "live/unrelated dials changed sampling allocation"
        );
        a.settings.material_settings.velvet_edge = 0.01;
        assert_eq!(density(a), Some(size), "sharp edges must keep native samples");
        a.settings.material_settings.velvet_edge = 0.34;
        a.settings.material_settings.velvet_size = 0.2;
        assert_eq!(density(a), Some(size), "fine scales must keep native samples");
    }

    #[test]
    fn drift_follows_the_dial_at_a_constant_direction() {
        let at = |direction, speed, now| {
            let settings = harmonigraph_scene::SpectralAtmosphere {
                cloud_speed: speed,
                cloud_direction: direction,
                ..Default::default()
            };
            cloud_drift(settings, cloud_offset(settings, now), None)
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
    }

    /// The wash's drift reaches the shader reduced by whole repeats of its
    /// tile, so a long clock lands on the cell the f64 offset names — the 3-4-5
    /// rotated basis included — within a millionth of a cell, where a plain
    /// cast to f32 misses it by 2.4 cells at this clock. A short clock passes
    /// bit for bit.
    #[test]
    fn a_tiled_drift_is_reduced_by_whole_repeats_before_it_narrows() {
        let period = super::CloudSampling::default().tile_cells;
        // Four months at the fresh speed: two million cloud units.
        let long = 1.0e7;
        let settings = harmonigraph_scene::SpectralAtmosphere {
            cloud_style: harmonigraph_scene::CloudStyle::Watercolor,
            cloud_direction: 37.0,
            ..Default::default()
        };
        let atmosphere = SpectrogramAtmosphere {
            settings,
            region: egui::Rect::ZERO,
            pitch_vertical: true,
            points_per_cent: 0.03,
            points_per_ms: 0.01,
            points_per_slab: 0.0,
            now: 0.0,
        };
        let tile = tile_key([1920, 1080], atmosphere, period);
        assert!(tile.is_some(), "the wash drew no tile");
        let short = cloud_offset(settings, 2.0);
        assert_eq!(cloud_drift(settings, short, tile), short.map(|v| v as f32));
        // The tile coordinate the shader samples, in periods: a whole number
        // apart is the same texel.
        let cells = f64::from(WASH_CELLS / settings.material_settings.wash_size);
        let uv = |q: [f64; 2]| {
            let r = q.map(|v| v * cells / f64::from(period));
            [0.8 * r[0] + 0.6 * r[1], -0.6 * r[0] + 0.8 * r[1]]
        };
        let off = |a: [f64; 2], b: [f64; 2]| {
            let (a, b) = (uv(a), uv(b));
            (0..2).map(|i| (a[i] - b[i] - (a[i] - b[i]).round()).abs()).fold(0.0, f64::max)
                * f64::from(period)
        };
        let exact = cloud_offset(settings, long);
        let reduced = cloud_drift(settings, exact, tile).map(f64::from);
        let cast = exact.map(|v| f64::from(v as f32));
        assert!(off(reduced, exact) < 1e-4, "{} cells off", off(reduced, exact));
        assert!(off(cast, exact) > 0.01, "the clock is too short to need reducing");
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
        assert_eq!(WASH_CELLS, number("WASH_CELLS"));

        let at = |cloud_tile, wash_size, height| {
            tile_key(
                [1920, height],
                SpectrogramAtmosphere {
                    settings: harmonigraph_scene::SpectralAtmosphere {
                        material_settings: harmonigraph_scene::MaterialSettings {
                            wash_size,
                            ..Default::default()
                        },
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
        // A 1080-pixel pane draws 20.6 pixels to a glob cell at a Patch size of
        // 1x. Rotation is an isometry, so the production period of forty wants
        // 824 texels and rounds up to 1024.
        let period = super::CloudSampling::default().tile_cells;
        assert_eq!(period, 40);
        assert_eq!(at(period, 1.0, 1080), Some(4 * TILE_STEP));
        // A pane resized by a tenth wants 905 and stays on the same step.
        assert_eq!(at(period, 1.0, 1188), at(period, 1.0, 1080));
        // Fine cells want few texels (51 here), and the floor is one step.
        assert_eq!(at(period, harmonigraph_scene::CLOUD_SIZE_MIN, 1080), Some(TILE_STEP));
        // Coarse cells on a tall pane run past the ceiling, where the tile is
        // simply coarser than the pane.
        assert_eq!(at(period, harmonigraph_scene::CLOUD_SIZE_MAX, 4320), Some(TILE_MAX));
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
                    // A cloud that reduces at all: the starfield never does.
                    settings: harmonigraph_scene::SpectralAtmosphere {
                        cloud_depth,
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

#[cfg(test)]
#[path = "color_memory_tests.rs"]
mod color_memory_tests;
