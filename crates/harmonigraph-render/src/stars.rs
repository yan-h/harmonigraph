//! Shared star geometry, allocation and frame transport for colored light fields.
use crate::uniforms::{uniform_group, Float2, Int2};
use crate::wgpu;
#[cfg(doc)]
use harmonigraph_scene::star_plan::star_profile;
use harmonigraph_scene::star_plan::{star_falloff_bend, star_jitter_width, StarGather};

/// How many depth slices the starfield walks, from the farthest (0) to the
/// nearest. The shader's `STAR_SLICES`, held to this by
/// `the_star_ring_holds_every_star_that_reaches_a_pixel`.
pub(crate) const STAR_SLICES: usize = harmonigraph_scene::star_plan::STAR_DEPTHS;
/// The period the star hash wraps at, in cells of each slice, and the modulus
/// each slice's drift is reduced by here in f64 before it is narrowed to f32.
///
/// Without the reduction a session left running for hours would carry a drift
/// of millions of star pixels into an f32, and the stars would start stepping
/// by fractions of a pixel. It is wider than any pane is in the finest cells,
/// so the repeat never shows: those are `STAR_SPACING_MIN` times
/// `STAR_SIZE_MIN`, 5/24 of a star pixel, which puts a 16:9 pane 4608 cells
/// wide and a 16:1 pane 41,472.
///
/// The price is the offset's own precision: an f32 near 65536 resolves a 256th
/// of a cell, which is 0.03 star pixels in the fresh nearest cells. A bigger
/// cell resolves coarser in proportion, and takes as much longer to drift
/// that far.
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
    /// The cell one star is hashed into: the plan's, its star size times its
    /// `Star spacing`, unless the atlas raised it ([`star_layout`]).
    cell: f32,
    /// The stars' outer radius in star pixels before each star's own size
    /// draw shrinks it: the plan's, half the depth's `Star size`.
    radius: f32,
    /// The star's shape ([`star_profile`]): its solid share of the radius,
    /// `1 / (1 - solid)`, and the glow's [`star_falloff_bend`].
    solid: f32,
    ramp: f32,
    bend: f32,
    /// Where this slice sits in the star atlas: the texel its first cell
    /// takes, counted along the rows, the cell that first one is, and how
    /// many cells it holds across and down. See [`StarLayout`]. An undrawn
    /// slice omitted by the layer count holds no cells; a solo-hidden slice keeps them.
    base: i32,
    origin: Int2,
    grid: Int2,
    /// The band a star's centre is drawn from, in cells: the slice's
    /// `Position variation`.
    width: f32,
    /// The inverse of the narrowest radius a star is drawn at, in star
    /// pixels: one texel of the star image, or as much of one as the slice's
    /// read holds whole ([`star_slices`]).
    inverse_floor: f32,
    /// How the slice is gathered: [`star_gather_code`].
    gather: u32,
    /// How far its stars fade between lives: the plan's. Below 1 a star
    /// keeps its place across its lives.
    twinkle: f32,
}
}

/// The shader's code for a gather, which `star_layers` and the bake read.
fn star_gather_code(gather: StarGather) -> u32 {
    match gather {
        StarGather::Off => 0,
        StarGather::Core => 1,
        StarGather::Two => 2,
        StarGather::Three => 3,
    }
}

/// The shader's `STAR_PANE`: star pixels across the pane's height.
pub(crate) const STAR_PANE: f32 = 540.0;
/// The star atlas's texel, one cell's star as the shader's `star_bake` packs it.
pub(crate) const STAR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
/// The star image's texel: every slice composited, gamma-coded, before the
/// consumer filters it up into the pane.
pub(crate) const STAR_IMAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The most texels the atlas may take, 64 MB at sixteen bytes each. At the
/// fresh dials a 16:9 pane takes about 1.2 million and an 8:1 strip about 5.4
/// million; a pane wider than about 6.3:1, or a finer `Star spacing`, asks
/// for more (see [`star_layout`]).
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

/// Each slice's cell as the dials ask for it, in star pixels: its star size
/// times its `Star spacing`.
pub(crate) fn star_cells(settings: harmonigraph_scene::StarSettings) -> [f32; STAR_SLICES] {
    settings.plan().depths.map(|depth| depth.cell)
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
    /// Cells across and down in each slice, none for an undrawn one, and the
    /// atlas texel its first cell is, counted along the rows.
    pub(crate) grids: [[u32; 2]; STAR_SLICES],
    pub(crate) bases: [u32; STAR_SLICES],
    /// The pane in star pixels, across and down.
    pub(crate) pane: [f32; 2],
    /// The texels all of it takes.
    pub(crate) texels: u64,
}

impl StarLayout {
    pub(crate) fn at(
        cells: [f32; STAR_SLICES],
        drawn: [bool; STAR_SLICES],
        floor: f32,
        aspect: f32,
    ) -> Self {
        let cells = cells.map(|cell| cell.max(floor));
        let pane = [STAR_PANE * aspect, STAR_PANE];
        // Retain the padded atlas bounds needed by the 3x3 walk around each
        // pixel's nominal cell. An undrawn slice bakes nothing.
        let grids = std::array::from_fn(|k| {
            pane.map(|span| {
                let cells =
                    ((span / cells[k]).ceil() as u32).saturating_add(4 + 2 * STAR_GRID_MARGIN);
                if drawn[k] {
                    cells
                } else {
                    0
                }
            })
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
/// the finest `Star spacing`, where a far cell is a fraction of a pixel on any
/// real pane — the finest cells are raised to the
/// smallest floor that fits, so those slices hold fewer, sparser stars and
/// every other slice is untouched. Solo only changes composition: hidden
/// layers keep their cells and colour history running at the full field's cost.
pub(crate) fn star_layout(settings: harmonigraph_scene::StarSettings, aspect: f32) -> StarLayout {
    let wanted = star_cells(settings);
    let drawn = settings.plan().depths.map(|depth| depth.gather != StarGather::Off);
    let at = |floor| StarLayout::at(wanted, drawn, floor, aspect);
    if at(0.0).fits() {
        return at(0.0);
    }
    let mut high = wanted.iter().copied().fold(f32::INFINITY, f32::min).max(1e-3);
    while !at(high).fits() {
        high *= 2.0;
    }
    let mut low = 0.0;
    for _ in 0..24 {
        let mid = (low + high) / 2.0;
        if at(mid).fits() {
            high = mid;
        } else {
            low = mid;
        }
    }
    at(high)
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

/// Every slice's numbers for this frame, for a star image `image` texels
/// across and down. Each depth moves as one sheet at its own speed.
///
/// The star image is drawn by evaluating each star at its texel centres, so
/// a star narrower than a texel would show only where a centre happened to
/// fall inside it, and blink as it drifted past them (#1446). Every star is
/// therefore drawn at least a texel wide, dimmed by the ratio of the areas
/// so it keeps its light, and each slice takes the cheapest read that holds
/// a star that wide whole. Where even the 3x3 read cannot, the floor stops
/// at what it holds: those cells are finer than a texel, so their stars were
/// never told apart at this resolution.
pub(crate) fn star_slices(
    settings: harmonigraph_scene::StarSettings,
    direction: f32,
    now: f64,
    layout: &StarLayout,
    image: [u32; 2],
) -> [StarSlice; STAR_SLICES] {
    let texel = STAR_PANE / image[1].max(1) as f32;
    // Star pixels travelled at a speed of one.
    let travel = now * star_px_per_second();
    let (sin, cos) = f64::from(direction).to_radians().sin_cos();
    let plan = settings.plan();
    let solo = plan
        .depths
        .iter()
        .enumerate()
        .any(|(k, depth)| depth.gather != StarGather::Off && settings.star_solo[k]);
    let (jitter, bend) = (settings.star_jitter, star_falloff_bend(settings.star_glow_falloff));
    std::array::from_fn(|k| {
        let depth = plan.depths[k];
        let cell = layout.cells[k];
        let speed = f64::from(depth.speed);
        let shift = |axis: f64| {
            (axis * travel * speed / f64::from(cell)).rem_euclid(STAR_HASH_PERIOD) as f32
        };
        let offset = [shift(cos), shift(sin)];
        let grid = layout.grids[k];
        // The cell a pixel at the pane's top left edge is in, as the shader
        // works it out, with the original conservative neighbor and margin.
        let origin: [i32; 2] =
            std::array::from_fn(|axis| star_origin(layout.pane[axis], cell, offset[axis]));
        let holds = |gather: StarGather| gather.bound(jitter) * cell;
        let widest = depth.radius.max(texel);
        let gather = match depth.gather {
            StarGather::Off => StarGather::Off,
            planned => StarGather::DRAWN
                .into_iter()
                .skip_while(|&gather| gather != planned)
                .find(|&gather| widest <= holds(gather))
                .unwrap_or(StarGather::Three),
        };
        StarSlice {
            offset: Float2(offset),
            cell,
            radius: depth.radius,
            solid: depth.solid,
            ramp: 1.0 / (1.0 - depth.solid),
            bend,
            base: layout.bases[k] as i32,
            origin: Int2(origin),
            grid: Int2(grid.map(|side| side as i32)),
            width: star_jitter_width(jitter),
            inverse_floor: 1.0
                / if gather == StarGather::Off { texel } else { texel.min(holds(gather)) },
            gather: if solo && !settings.star_solo[k] { 0 } else { star_gather_code(gather) },
            twinkle: depth.twinkle,
        }
    })
}

/// The cell at the start of a slice's grid on one axis: the one a pixel at
/// the pane's leading edge is in, less one conservative neighbor and the
/// margin. This bounds the three-cell walk.
/// `span` is the pane along the axis in star pixels.
pub(crate) fn star_origin(span: f32, cell: f32, offset: f32) -> i32 {
    let edge = -f64::from(span / 2.0 / cell) - f64::from(offset);
    edge.floor() as i32 - 1 - STAR_GRID_MARGIN as i32
}
/// The star image every slice is drawn into: the pane's device pixels at
/// `Stars resolution`, rounded up. The rounded shape is the whole allocation
/// key, so dial edits that round to the same image refill the same target.
pub(crate) fn star_image_size(
    pixels: [u32; 2],
    settings: harmonigraph_scene::StarSettings,
) -> [u32; 2] {
    pixels.map(|n| (n as f32 * settings.star_resolution).ceil().max(1.0) as u32)
}

/// Include every bilinear tap at the boundary of a partially covered pane.
pub(crate) fn star_image_scissor(
    coverage: [u32; 4],
    pixels: [u32; 2],
    target: [u32; 2],
) -> [u32; 4] {
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

uniform_group! {
    struct StarUniforms {
        size: Float2,
        star_image: Float2,
        star_randomness: f32,
        star_life: f32,
        star_size_variation: f32,
        pad: f32,
        star_slices: [StarSlice; STAR_SLICES],
    }
}
impl StarUniforms {
    pub(crate) fn new(
        settings: harmonigraph_scene::StarSettings,
        direction: f32,
        now: f64,
        pixels: [u32; 2],
        layout: &StarLayout,
        image: [u32; 2],
    ) -> Self {
        Self {
            size: Float2(pixels.map(|n| n as f32)),
            star_image: Float2(image.map(|n| n as f32)),
            star_randomness: settings.star_randomness,
            star_life: star_life(settings, now),
            star_size_variation: settings.star_size_variation,
            pad: 0.0,
            star_slices: star_slices(settings, direction, now, layout, image),
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
    pub(crate) fn draw(&self, encoder: &mut wgpu::CommandEncoder) {
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
        pass.draw(0..3, 0..1);
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
