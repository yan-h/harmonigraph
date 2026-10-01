//! Shared star geometry, allocation and frame transport for colored light fields.
use crate::uniforms::{uniform_group, Float2, Float4, Int2};
use crate::wgpu;

/// How many depth slices the starfield walks, from the farthest (0) to the
/// nearest. The shader's `STAR_SLICES`, held to this by
/// `the_star_ring_holds_every_star_that_reaches_a_pixel`.
pub(crate) const STAR_SLICES: usize = 5;
/// The period the star hash wraps at, in cells of each slice, and the modulus
/// each slice's drift is reduced by here in f64 before it is narrowed to f32.
///
/// Without the reduction a session left running for hours would carry a drift
/// of millions of star pixels into an f32, and the stars would start stepping
/// by fractions of a pixel. It is wider than any pane is in the finest cells,
/// so the repeat never shows: those are `STAR_SIZE_MIN / sqrt(STAR_DENSITY_MAX
/// / 2)`, 0.224 star pixels, which puts a 16:9 pane about 4300 cells wide (the
/// 4096 this was once repeated inside it) and a 16:1 pane about 38,600.
///
/// The price is the offset's own precision: an f32 near 65536 resolves a 256th
/// of a cell, which is 0.03 star pixels in the fresh nearest cells and half a
/// star pixel only in the biggest cell `Star size` and `Star density` allow.
pub(crate) const STAR_HASH_PERIOD: f64 = 65536.0;
/// The period the life clock is reduced by, in lives: a power of two, so the
/// shader's mask on the life index wraps with it and a star's life runs
/// straight across the wrap. The shader's own constant, checked against it.
pub(crate) const STAR_LIFE_PERIOD: f64 = 4096.0;
/// How fast a depth at `Star speed` 1 travels, in star pixels (a
/// 540th of the pane's height) per second: the prototype's `(-60, -14)` px/s
/// over its 540-pixel pane, which is about a ninth of the pane's height a
/// second, while the music scrolled at 192 px/s under it.
///
/// Its OWN speed, not the other textures' `UNITS_PER_SECOND` times something:
/// those cross a pane in minutes, and at that pace a starfield reads as a still
/// sprite over the sound — which is exactly what Yan rejected in the first
/// motion video.
pub(crate) fn star_px_per_second() -> f64 {
    (60.0f64 * 60.0 + 14.0 * 14.0).sqrt()
}

uniform_group! {
/// One depth slice of the starfield, in the shader's `StarSlice` order: read by
/// OFFSET, so a reordering here swaps values silently.
///
/// Worked out here rather than in the shader because it is a function of the
/// dials alone, per slice, and because the drift has to be reduced in f64. The
/// formulas are the prototype's `drift.slice_params`, with `d` running 0 (far)
/// to 1 (near); every length is in STAR PIXELS, a 540th of the pane's height,
/// which was the prototype's pane.
#[derive(Debug, Default, PartialEq)]
struct StarSlice {
    /// This slice's drift, in its own cells, reduced modulo
    /// [`STAR_HASH_PERIOD`]: the stars sit at `cell + offset`.
    offset: Float2,
    /// The cell one star is hashed into: `Star size`'s low end at the far end
    /// over the square root of half the density, times the ratio of its ends
    /// raised to `d^Size curve` — at the fresh 2 to 32 and 2, 32 at the near
    /// end and most of the depth fine dust.
    cell: f32,
    /// The core's base sigma, before the per-star size draw: a quarter of
    /// this depth's value on the `Star size` curve, the same at every depth
    /// for the same value.
    sigma: f32,
    /// The ceiling on a core, before defocus: a third of a cell. It is what
    /// keeps the dust pinpoint — dropping it made the prototype's field foamy.
    cap: f32,
    /// How much the core is widened after the cap, equally at every depth.
    defocus: f32,
    /// The same-colour fringe's coverage at the star's centre, falling off as
    /// `exp(-d / 2.5 sigma)`. Uniform and the nearest two layers fade to zero
    /// at 1.2 cells; the optimized far-three response ends at
    /// `1.0 - 0.3 * star_jitter` cells.
    fringe: f32,
    /// Where this slice sits in the star atlas: the texel its first cell
    /// takes, counted along the rows, the cell that first one is, and how
    /// many cells it holds across and down. See [`StarLayout`].
    base: i32,
    origin: Int2,
    grid: Int2,
}
}

/// The slice's depth, 0 for the farthest and 1 for the nearest.
fn star_depth(k: usize) -> f32 {
    k as f32 / (STAR_SLICES - 1) as f32
}

/// The shader's `STAR_PANE`: star pixels across the pane's height.
pub(crate) const STAR_PANE: f32 = 540.0;
/// The star atlas's texel, one cell's star as the shader's `star_bake` packs it.
pub(crate) const STAR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
pub(crate) const STAR_FAR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Below this area the extra pass does not consistently pay for itself.
/// Apply it to drawn device pixels, independent of display scale and star count.
pub(crate) const STAR_SPLIT_PIXELS: u64 = 2560 * 1440;
/// The most texels the atlas may take, 64 MB at sixteen bytes each. At the
/// fresh dials a 16:9 pane takes about 470 thousand and an 8:1 strip about 2
/// million; only the finest `Star size` at a high `Star density`, or a
/// still wider pane, asks for more (see [`star_layout`]).
const STAR_ATLAS_TEXELS: u64 = 1 << 22;
/// The atlas's width, the shader's `STAR_ATLAS_WIDTH`: a power of two, so a
/// cell's index splits into a texel with a mask and a shift.
pub(crate) const STAR_ATLAS_WIDTH: u32 = 2048;
/// Cells a slice's grid holds past the ones the walk can reach from the pane
/// on each side. On correctly rounded arithmetic it is not needed: the CPU
/// takes the pane's leading edge exactly in f64 from the same f32 inputs, and
/// the shader's rounded f32 can only floor at or above that. It is for the GPU,
/// whose `sp / cell` is not correctly rounded and which no CPU replay
/// reproduces — see `the_star_atlas_holds_every_cell_the_walk_reads`.
const STAR_GRID_MARGIN: u32 = 1;
/// The atlas is allocated in whole multiples of this many rows, so a drag of a
/// dial that sizes cells reallocates at steps rather than every frame.
const STAR_ATLAS_STEP: u32 = 64;

/// Each slice's cell as the dials ask for it, in star pixels: `Star size`'s
/// low end at the far end over the square root of half the density, times the
/// ratio of its ends raised to `d^Size curve`.
pub(crate) fn star_cells(settings: harmonigraph_scene::StarSettings) -> [f32; STAR_SLICES] {
    let packing = (settings.star_density / 2.0).sqrt();
    let (small, big) = (settings.star_size_min, settings.star_size_max);
    std::array::from_fn(|k| {
        small * (big / small).powf(star_depth(k).powf(settings.star_size_curve)) / packing
    })
}

/// Where every slice's cells sit in the star atlas the shader's
/// `fs_star_bake` fills each frame: a texel a cell, each slice's grid — the
/// pane's cells and a margin — laid row after row along the atlas's rows, one
/// slice straight after the other.
///
/// A function of the dials and the pane's SHAPE alone — a pane has the same
/// number of cells at any resolution, because a star pixel is a fraction of
/// its height — so an export draws the same stars at every size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct StarLayout {
    /// Each slice's cell in star pixels: the dials' own, unless the atlas
    /// could not hold the finest of them.
    pub(crate) cells: [f32; STAR_SLICES],
    /// Cells across and down in each slice, and the atlas texel its first
    /// cell is, counted along the rows.
    pub(crate) grids: [[u32; 2]; STAR_SLICES],
    pub(crate) bases: [u32; STAR_SLICES],
    /// The pane in star pixels, across and down.
    pub(crate) pane: [f32; 2],
    /// The texels all of it takes.
    pub(crate) texels: u64,
}

impl StarLayout {
    pub(crate) fn at(cells: [f32; STAR_SLICES], floor: f32, aspect: f32) -> Self {
        let cells = cells.map(|cell| cell.max(floor));
        let pane = [STAR_PANE * aspect, STAR_PANE];
        // Retain the padded atlas bounds needed by the halo
        // pass's 3x3 walk around each pixel's nominal cell.
        let grids = cells.map(|cell| {
            pane.map(|span| ((span / cell).ceil() as u32).saturating_add(4 + 2 * STAR_GRID_MARGIN))
        });
        // Each slice's cells row after row, straight after the last slice's,
        // so no slice pays for another's width.
        let mut bases = [0; STAR_SLICES];
        let mut texels = 0u64;
        for (base, grid) in bases.iter_mut().zip(&grids) {
            *base = texels.min(u64::from(u32::MAX)) as u32;
            texels += u64::from(grid[0]) * u64::from(grid[1]);
        }
        Self { cells, grids, bases, pane, texels }
    }

    pub(crate) fn fits(&self) -> bool {
        self.texels <= STAR_ATLAS_TEXELS
    }

    /// The atlas this needs: [`STAR_ATLAS_WIDTH`] across and as many rows as
    /// the cells fill.
    pub fn size(&self) -> [u32; 2] {
        [STAR_ATLAS_WIDTH, self.texels.div_ceil(u64::from(STAR_ATLAS_WIDTH)).max(1) as u32]
    }
}

/// The starfield's layout for a pane `aspect` wide per unit of height.
///
/// Where the dials' cells would take more atlas than [`STAR_ATLAS_TEXELS`] —
/// the finest `Star size` at a high `Star density`, where a far cell is a
/// fraction of a pixel on any real pane — the finest cells are raised to the
/// smallest floor that fits, so those slices hold fewer, sparser stars and
/// every other slice is untouched.
pub(crate) fn star_layout(settings: harmonigraph_scene::StarSettings, aspect: f32) -> StarLayout {
    let wanted = star_cells(settings);
    let whole = StarLayout::at(wanted, 0.0, aspect);
    if whole.fits() {
        return whole;
    }
    let mut high = wanted[0].max(1e-3);
    while !StarLayout::at(wanted, high, aspect).fits() {
        high *= 2.0;
    }
    let mut low = 0.0;
    for _ in 0..24 {
        let mid = (low + high) / 2.0;
        if StarLayout::at(wanted, mid, aspect).fits() {
            high = mid;
        } else {
            low = mid;
        }
    }
    StarLayout::at(wanted, high, aspect)
}

/// The atlas to allocate for `needed` texels, keeping the one `held` while it
/// still holds them and has not more than twice the rows, so a dial drag steps
/// through a handful of sizes rather than one per frame.
pub(crate) fn star_atlas_size(needed: [u32; 2], held: Option<[u32; 2]>) -> [u32; 2] {
    let rows = needed[1].next_multiple_of(STAR_ATLAS_STEP);
    held.filter(|held| held[0] == needed[0] && needed[1] <= held[1] && held[1] <= rows * 2)
        .unwrap_or([needed[0], rows])
}

/// The life clock every slice shares: seconds over `Star lifetime`, reduced
/// modulo [`STAR_LIFE_PERIOD`] in f64. A cell's lives start at this plus its
/// hashed stagger, so it moves with nothing but the clock and that one dial.
pub(crate) fn star_life(settings: harmonigraph_scene::StarSettings, now: f64) -> f32 {
    (now / f64::from(settings.star_lifetime)).rem_euclid(STAR_LIFE_PERIOD) as f32
}

/// A slice's speed, as a multiple of [`star_px_per_second`]:
/// `min + (max - min) d^curve` over `Star speed`'s two ends.
pub(crate) fn star_speed(settings: harmonigraph_scene::StarSettings, k: usize) -> f32 {
    let (far, near) = (settings.star_speed_min, settings.star_speed_max);
    far + (near - far) * star_depth(k).powf(settings.star_speed_curve)
}

/// Every slice's numbers for this frame. Each depth moves as one sheet at its
/// own speed.
pub(crate) fn star_slices(
    settings: harmonigraph_scene::StarSettings,
    direction: f32,
    now: f64,
    layout: &StarLayout,
) -> [StarSlice; STAR_SLICES] {
    // Star pixels travelled at a speed of one.
    let travel = now * star_px_per_second();
    let (sin, cos) = f64::from(direction).to_radians().sin_cos();
    let small = settings.star_size_min;
    let big = settings.star_size_max;
    std::array::from_fn(|k| {
        let d = star_depth(k);
        let along = d.powf(settings.star_size_curve);
        let cell = layout.cells[k];
        // The core follows this depth's point on the `Star size` curve alone,
        // so one value on the control is one star size at every depth: a
        // quarter of that spacing (the fresh far end's 0.5 at 2), capped at a
        // third of the depth's actual spacing, which density and the atlas
        // floor set.
        let sigma = 0.25 * small * (big / small).powf(along);
        let cap = 0.33 * cell;
        let defocus = 1.0 + settings.star_defocus;
        let speed = f64::from(star_speed(settings, k));
        let shift = |axis: f64| {
            (axis * travel * speed / f64::from(cell)).rem_euclid(STAR_HASH_PERIOD) as f32
        };
        let offset = [shift(cos), shift(sin)];
        let grid = layout.grids[k];
        // The cell a pixel at the pane's top left edge is in, as the shader
        // works it out, with the original conservative neighbor and margin.
        let origin: [i32; 2] =
            std::array::from_fn(|axis| star_origin(layout.pane[axis], cell, offset[axis]));
        StarSlice {
            offset: Float2(offset),
            cell,
            sigma,
            cap,
            defocus,
            fringe: settings.star_fringe,
            base: layout.bases[k] as i32,
            origin: Int2(origin),
            grid: Int2(grid.map(|side| side as i32)),
        }
    })
}

/// The cell at the start of a slice's grid on one axis: the one a pixel at
/// the pane's leading edge is in, less one conservative neighbor and the
/// margin. This bounds the three-cell halo walk and its one-cell core.
/// `span` is the pane along the axis in star pixels.
pub(crate) fn star_origin(span: f32, cell: f32, offset: f32) -> i32 {
    let edge = -f64::from(span / 2.0 / cell) - f64::from(offset);
    edge.floor() as i32 - 1 - STAR_GRID_MARGIN as i32
}
pub(crate) fn star_geometry(jitter: f32, far_fill: f32) -> Float4 {
    let width = 0.6 * f64::from(jitter);
    Float4([width as f32, (0.5 - width / 2.0) as f32, 0.7, far_fill])
}

/// Per-depth halo sampling follows pane pixels and the sanitized resolution
/// dial. The rounded texture shape is the whole allocation key: jitter, halo
/// width, drift and color edits refill the same targets.
pub(crate) fn star_halo_size(pixels: [u32; 2], resolution: f32) -> [u32; 2] {
    pixels.map(|n| (n as f32 * resolution).ceil().max(1.0) as u32)
}

/// Uniform keeps exact native texel addressing; High, Medium and Low draw the
/// complete far-three response at 75%, 50% and one-third dimensions.
pub(crate) fn star_far_size(
    pixels: [u32; 2],
    settings: harmonigraph_scene::StarSettings,
) -> [u32; 2] {
    use harmonigraph_scene::StarHaloProfile;
    match settings.star_halo_profile {
        StarHaloProfile::Uniform => pixels,
        StarHaloProfile::P3 => star_halo_size(pixels, 0.75),
        StarHaloProfile::Medium => star_halo_size(pixels, 0.5),
        StarHaloProfile::Low => star_halo_size(pixels, 1.0 / 3.0),
    }
}

/// Medium and Low shade the foreground over the far image at 75% and 50% dimensions.
/// Exact rounded dimensions belong to allocation identity, not the preset name.
pub(crate) fn star_near_size(
    pixels: [u32; 2],
    settings: harmonigraph_scene::StarSettings,
) -> Option<[u32; 2]> {
    use harmonigraph_scene::StarHaloProfile;
    match settings.star_halo_profile {
        StarHaloProfile::Medium => Some(star_halo_size(pixels, 0.75)),
        StarHaloProfile::Low => Some(star_halo_size(pixels, 0.5)),
        StarHaloProfile::P3 | StarHaloProfile::Uniform => None,
    }
}

/// Include every bilinear tap at the boundary of a partially covered pane.
pub(crate) fn star_far_scissor(coverage: [u32; 4], pixels: [u32; 2], target: [u32; 2]) -> [u32; 4] {
    if pixels == target {
        return coverage;
    }
    let start: [u32; 2] = std::array::from_fn(|axis| {
        ((f64::from(coverage[axis]) * f64::from(target[axis]) / f64::from(pixels[axis])).floor()
            as u32)
            .saturating_sub(1)
    });
    let end: [u32; 2] = std::array::from_fn(|axis| {
        ((f64::from(coverage[axis] + coverage[axis + 2]) * f64::from(target[axis])
            / f64::from(pixels[axis]))
        .ceil() as u32)
            .saturating_add(1)
            .min(target[axis])
    });
    [start[0], start[1], end[0] - start[0], end[1] - start[1]]
}

pub(crate) const STAR_HALO_GROUPS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HaloGroup {
    pub(crate) size: [u32; 2],
    pub(crate) layers: u32,
}

/// Allocation identity contains only the actual images and their depth mapping.
/// Different controls that round to this same layout reuse the same targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StarHaloLayout {
    pub(crate) groups: [HaloGroup; STAR_HALO_GROUPS],
    /// [group, array layer] for each far-to-near depth.
    pub(crate) layers: [[u32; 2]; STAR_SLICES],
    /// High, Medium and Low far depths are drawn directly and own no halo images.
    pub(crate) first_active_layer: usize,
}

impl StarHaloLayout {
    pub(crate) fn from_sizes(sizes: [[u32; 2]; STAR_SLICES], first_active_layer: usize) -> Self {
        let mut layout = Self {
            groups: [HaloGroup { size: [1, 1], layers: 0 }; STAR_HALO_GROUPS],
            layers: [[0, 0]; STAR_SLICES],
            first_active_layer,
        };
        for (depth, size) in sizes.into_iter().enumerate().skip(first_active_layer) {
            let group = layout
                .groups
                .iter()
                .position(|g| g.layers > 0 && g.size == size)
                .or_else(|| layout.groups.iter().position(|g| g.layers == 0))
                .expect("halo profiles use at most three different target sizes");
            layout.layers[depth] = [group as u32, layout.groups[group].layers];
            layout.groups[group].size = size;
            layout.groups[group].layers += 1;
        }
        layout
    }

    pub(crate) fn samples(self) -> [StarHaloSample; STAR_SLICES] {
        self.layers.map(|[group, layer]| StarHaloSample {
            size: Float2(self.groups[group as usize].size.map(|n| n as f32)),
            group,
            layer,
        })
    }
}

/// High, Medium and Low need halos only for the nearest two depths. Material history
/// keeps the same identity across profiles, independent of their sampling.
pub(crate) fn star_halo_layout(
    pixels: [u32; 2],
    settings: harmonigraph_scene::StarSettings,
) -> StarHaloLayout {
    use harmonigraph_scene::StarHaloProfile;
    let settings = settings.sanitized();
    let (factors, first_active_layer) = match settings.star_halo_profile {
        StarHaloProfile::Uniform => ([settings.star_halo_resolution; STAR_SLICES], 0),
        StarHaloProfile::P3 => ([0.0, 0.0, 0.0, 1.0, 0.6], 3),
        StarHaloProfile::Medium => ([0.0, 0.0, 0.0, 0.75, 0.45], 3),
        StarHaloProfile::Low => ([0.0, 0.0, 0.0, 0.5, 0.3], 3),
    };
    StarHaloLayout::from_sizes(
        factors.map(|factor| star_halo_size(pixels, factor)),
        first_active_layer,
    )
}

// One 16-byte uniform row: the renderer supplies the allocated size and
// array address directly, without reproducing float rounding in the shader.
uniform_group! {
    #[derive(Debug)]
    struct StarHaloSample {
        size: Float2,
        group: u32,
        layer: u32,
    }
}

/// One premultiplied halo image per depth, sampled together only after each
/// slice's native core has been added. A flattened RGB image would lose the
/// coverage normalization and depth order.
pub(crate) struct StarHalos {
    pub(crate) views: [wgpu::TextureView; STAR_HALO_GROUPS],
    pub(crate) layers: [Option<wgpu::TextureView>; STAR_SLICES],
    pub(crate) layout: StarHaloLayout,
}

impl StarHalos {
    pub(crate) fn new(device: &wgpu::Device, layout: StarHaloLayout) -> Self {
        let textures = layout.groups.map(|group| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("spectral_star_halos"),
                size: wgpu::Extent3d {
                    width: group.size[0],
                    height: group.size[1],
                    // Unused fixed bindings receive a harmless one-texel array.
                    depth_or_array_layers: group.layers.max(1),
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: STAR_FAR_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        });
        Self {
            views: std::array::from_fn(|group| {
                textures[group].create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    ..Default::default()
                })
            }),
            layers: std::array::from_fn(|depth| {
                if depth < layout.first_active_layer {
                    return None;
                }
                let [group, layer] = layout.layers[depth];
                Some(textures[group as usize].create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                }))
            }),
            layout,
        }
    }
}

uniform_group! {
    struct StarUniforms {
        origin: Float2,
        size: Float2,
        ppp: f32,
        star_randomness: f32,
        star_life: f32,
        star_size_variation: f32,
        star_far: Float4,
        star_near: Float4,
        star_geometry: Float4,
        star_slices: [StarSlice; STAR_SLICES],
        star_halo_samples: [StarHaloSample; STAR_SLICES],
    }
}
impl StarUniforms {
    pub(crate) fn new(
        settings: harmonigraph_scene::StarSettings,
        direction: f32,
        now: f64,
        pixels: [u32; 2],
        layout: &StarLayout,
        halos: StarHaloLayout,
    ) -> Self {
        let far = star_far_size(pixels, settings);
        let near = star_near_size(pixels, settings).unwrap_or([0; 2]);
        Self {
            origin: Float2([0.0; 2]),
            size: Float2(pixels.map(|n| n as f32)),
            ppp: 1.0,
            star_randomness: settings.star_randomness,
            star_life: star_life(settings, now),
            star_size_variation: settings.star_size_variation,
            star_far: Float4([
                far[0] as f32,
                far[1] as f32,
                f32::from(
                    settings.star_halo_profile != harmonigraph_scene::StarHaloProfile::Uniform,
                ),
                0.0,
            ]),
            star_near: Float4([near[0] as f32, near[1] as f32, 0.0, 0.0]),
            star_geometry: star_geometry(settings.star_jitter, settings.star_far_fill),
            star_slices: star_slices(settings, direction, now, layout),
            star_halo_samples: halos.samples(),
        }
    }
}

/// A material draw; resource bindings remain owned by the light-field consumer.
pub(crate) struct Pass<'a> {
    pub view: &'a wgpu::TextureView,
    pub pipeline: &'a wgpu::RenderPipeline,
    pub groups: &'a [&'a wgpu::BindGroup],
    pub scissor: Option<[u32; 4]>,
}
impl Pass<'_> {
    pub(crate) fn draw(&self, encoder: &mut wgpu::CommandEncoder, layer: u32) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("star_material"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: self.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(self.pipeline);
        for (index, group) in self.groups.iter().enumerate() {
            pass.set_bind_group(index as u32, *group, &[]);
        }
        if let Some([x, y, w, h]) = self.scissor {
            pass.set_scissor_rect(x, y, w, h);
        }
        pass.draw(0..3, layer..layer + 1);
    }
}
/// Every profile follows this sequence; only allocated halo layers and optional
/// far/near images differ. Neither consumer can silently omit a profile's pass.
pub(crate) fn draw(
    encoder: &mut wgpu::CommandEncoder,
    bake: Pass<'_>,
    halos: &StarHalos,
    halo_pipeline: &wgpu::RenderPipeline,
    halo_groups: &[&wgpu::BindGroup],
    far: Option<Pass<'_>>,
    near: Option<Pass<'_>>,
) {
    bake.draw(encoder, 0);
    for (layer, view) in halos.layers.iter().enumerate() {
        if let Some(view) = view {
            Pass { view, pipeline: halo_pipeline, groups: halo_groups, scissor: None }
                .draw(encoder, layer as u32);
        }
    }
    if let Some(pass) = far {
        pass.draw(encoder, 0);
    }
    if let Some(pass) = near {
        pass.draw(encoder, 0);
    }
}

pub(crate) fn image(
    device: &wgpu::Device,
    label: &str,
    size: [u32; 2],
    format: wgpu::TextureFormat,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | if cfg!(test) {
                    wgpu::TextureUsages::COPY_SRC
                } else {
                    wgpu::TextureUsages::empty()
                },
            view_formats: &[],
        })
        .create_view(&Default::default())
}
