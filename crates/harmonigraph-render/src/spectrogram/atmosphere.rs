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

/// How many depth slices the starfield walks, from the farthest (0) to the
/// nearest. The shader's `STAR_SLICES`, held to this by
/// `the_star_ring_holds_every_star_that_reaches_a_pixel`.
const STAR_SLICES: usize = 5;
/// How far a star's hashed centre may sit from its cell's middle, as a whole
/// width: `STAR_JITTER / 2` either way. The shader's own constant, mirrored for
/// the ring's reach and checked against the shipped text.
const STAR_JITTER: f32 = 0.6;
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
const STAR_HASH_PERIOD: f64 = 65536.0;
/// The period the life clock is reduced by, in lives: a power of two, so the
/// shader's mask on the life index wraps with it and a star's life runs
/// straight across the wrap. The shader's own constant, checked against it.
const STAR_LIFE_PERIOD: f64 = 4096.0;
/// The farthest a star's own speed carries it from where its slice's drift
/// puts it, in cells either way, over one life: the ring's budget for the
/// spread. A slice whose cells cannot hold a star at its widest spread speed
/// this close for a whole `Star lifetime` gets a narrower spread instead.
///
/// As much as the ring can give while it still reaches half a cell past every
/// star (the floor `the_star_ring_holds_every_star_that_reaches_a_pixel`
/// holds): [`STAR_REACH_CELLS`] less this is 0.5. More would take a 4x4 walk,
/// which would cost every pixel seven more cells in every slice.
const STAR_SPREAD_REACH: f32 = 0.7;
/// How fast a depth at `Star speed` 1 travels, in star pixels (a
/// 540th of the pane's height) per second: the prototype's `(-60, -14)` px/s
/// over its 540-pixel pane, which is about a ninth of the pane's height a
/// second, while the music scrolled at 192 px/s under it.
///
/// Its OWN speed, not the other textures' `UNITS_PER_SECOND` times something:
/// those cross a pane in minutes, and at that pace a starfield reads as a still
/// sprite over the sound — which is exactly what Yan rejected in the first
/// motion video.
pub(super) fn star_px_per_second() -> f64 {
    (60.0f64 * 60.0 + 14.0 * 14.0).sqrt()
}

/// One depth slice of the starfield, in the shader's `StarSlice` order: read by
/// OFFSET, so a reordering here swaps values silently.
///
/// Worked out here rather than in the shader because it is a function of the
/// dials alone, per slice, and because the drift has to be reduced in f64. The
/// formulas are the prototype's `drift.slice_params`, with `d` running 0 (far)
/// to 1 (near); every length is in STAR PIXELS, a 540th of the pane's height,
/// which was the prototype's pane.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct StarSlice {
    /// This slice's drift, in its own cells, reduced modulo
    /// [`STAR_HASH_PERIOD`]: the stars sit at `cell + offset`.
    offset: [f32; 2],
    /// How far, in cells along the drift, a star whose speed is the widest
    /// `Speed spread` allows moves from its slice's drift over one life. A
    /// star's own is this times `2h - 1` for its hashed `h`, and it sits at
    /// that times `age - 0.5` through its life, so it passes where the drift
    /// puts it at mid-life and strays at most half this either side.
    spread: [f32; 2],
    /// The cell one star is hashed into: `Star size`'s low end at the far end
    /// over the square root of half the density, times the ratio of its ends
    /// raised to `d^Size curve` — at the fresh 2 to 32 and 2, 32 at the near
    /// end and most of the depth fine dust.
    cell: f32,
    /// The core's base sigma, before the per-star size draw.
    sigma: f32,
    /// The ceiling on a core, before defocus: 1.8 star pixels or a third of a
    /// cell. The cell half is what keeps the dust pinpoint — dropping it made
    /// the prototype's field foamy.
    cap: f32,
    /// How much the core is widened after the cap: 1 at the far end.
    defocus: f32,
    /// The same-colour fringe's coverage at the star's centre, falling off as
    /// `exp(-d / 2.5 sigma)` and bounded only by the ring's fade to zero at
    /// `reach`: `Fringe`, alike at every depth.
    fringe: f32,
    /// The ring's reach in star pixels — the nearest a star outside the 3x3
    /// walk can be to the pixel — where every star's coverage is windowed to
    /// zero.
    reach: f32,
    /// Where this slice sits in the star atlas: the texel its first cell
    /// takes, counted along the rows, the cell that first one is, and how
    /// many cells it holds across and down. See [`StarLayout`].
    base: i32,
    /// Puts `origin` on the eight-byte boundary WGSL gives a `vec2<i32>`.
    _pad: i32,
    origin: [i32; 2],
    grid: [i32; 2],
}

/// The slice's depth, 0 for the farthest and 1 for the nearest.
fn star_depth(k: usize) -> f32 {
    k as f32 / (STAR_SLICES - 1) as f32
}

/// The shader's `STAR_PANE`: star pixels across the pane's height.
const STAR_PANE: f32 = 540.0;
/// The star atlas's texel, one cell's star as the shader's `star_bake` packs it.
const STAR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Uint;
/// The most texels the atlas may take, 64 MB at sixteen bytes each. At the
/// fresh dials a 16:9 pane takes about 470 thousand and an 8:1 strip about 2
/// million; only the finest `Star size` at a high `Star density`, or a
/// still wider pane, asks for more (see [`star_layout`]).
const STAR_ATLAS_TEXELS: u64 = 1 << 22;
/// The atlas's width, the shader's `STAR_ATLAS_WIDTH`: a power of two, so a
/// cell's index splits into a texel with a mask and a shift.
const STAR_ATLAS_WIDTH: u32 = 2048;
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
fn star_cells(settings: harmonigraph_scene::SpectralAtmosphere) -> [f32; STAR_SLICES] {
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
pub(super) struct StarLayout {
    /// Each slice's cell in star pixels: the dials' own, unless the atlas
    /// could not hold the finest of them.
    cells: [f32; STAR_SLICES],
    /// Cells across and down in each slice, and the atlas texel its first
    /// cell is, counted along the rows.
    grids: [[u32; 2]; STAR_SLICES],
    bases: [u32; STAR_SLICES],
    /// The pane in star pixels, across and down.
    pane: [f32; 2],
    /// The texels all of it takes.
    texels: u64,
}

impl StarLayout {
    fn at(cells: [f32; STAR_SLICES], floor: f32, aspect: f32) -> Self {
        let cells = cells.map(|cell| cell.max(floor));
        let pane = [STAR_PANE * aspect, STAR_PANE];
        // The walk reaches from `floor(r) - 1` at the pane's one edge to
        // `floor(r) + 1` at the other, which is at most `ceil(span) + 3` cells.
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

    fn fits(&self) -> bool {
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
pub(super) fn star_layout(
    settings: harmonigraph_scene::SpectralAtmosphere,
    aspect: f32,
) -> StarLayout {
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

/// The starfield's layout for a pane of `pixels`, or `None` where no starfield
/// is drawn and no atlas is allocated.
pub(super) fn stars(pixels: [u32; 2], atmosphere: SpectrogramAtmosphere) -> Option<StarLayout> {
    let settings = atmosphere.settings.sanitized();
    let drawn =
        settings.effects().cloud && settings.cloud_style == harmonigraph_scene::CloudStyle::Stars;
    drawn.then(|| star_layout(settings, pixels[0] as f32 / pixels[1] as f32))
}

/// The atlas to allocate for `needed` texels, keeping the one `held` while it
/// still holds them and has not more than twice the rows, so a dial drag steps
/// through a handful of sizes rather than one per frame.
pub(super) fn star_atlas_size(needed: [u32; 2], held: Option<[u32; 2]>) -> [u32; 2] {
    let rows = needed[1].next_multiple_of(STAR_ATLAS_STEP);
    held.filter(|held| held[0] == needed[0] && needed[1] <= held[1] && held[1] <= rows * 2)
        .unwrap_or([needed[0], rows])
}

/// How far the ring's reach is from a pixel, in cells, before the spread: a
/// centre strays `STAR_JITTER / 2` from its own cell's middle, so the nearest
/// a star from a cell outside the 3x3 walk can come is this. Each slice's
/// `reach` takes its spread's excursion, at most [`STAR_SPREAD_REACH`], off it
/// again. The Mosaic's `DOME_RADIUS` proof in one line.
const STAR_REACH_CELLS: f32 = 1.5 - STAR_JITTER / 2.0;

/// The life clock every slice shares: seconds over `Star lifetime`, reduced
/// modulo [`STAR_LIFE_PERIOD`] in f64. A cell's lives start at this plus its
/// hashed stagger, so it moves with nothing but the clock and that one dial.
fn star_life(settings: harmonigraph_scene::SpectralAtmosphere, now: f64) -> f32 {
    (now / f64::from(settings.star_lifetime)).rem_euclid(STAR_LIFE_PERIOD) as f32
}

/// A slice's speed, as a multiple of [`star_px_per_second`]:
/// `min + (max - min) d^curve` over `Star speed`'s two ends.
fn star_speed(settings: harmonigraph_scene::SpectralAtmosphere, k: usize) -> f32 {
    let (far, near) = (settings.star_speed_min, settings.star_speed_max);
    far + (near - far) * star_depth(k).powf(settings.star_speed_curve)
}

/// Every slice's numbers for this frame. Every star lives `Star lifetime`, and
/// `Speed spread` is a share of the widest spread speed the slice can hold:
/// half the gap to its neighbours' speeds, or whatever carries a star exactly
/// [`STAR_SPREAD_REACH`] from its drift in one life, if that is less. The ring
/// stays 3x3 and small cells stray less, rather than living shorter — which
/// tied every life to seven dials, so dragging any of them reshuffled the
/// whole field. A share of the most that fits, rather than the gap clamped to
/// it, because the clamp binds at every depth at the fresh 6 s and left the
/// dial dead above a few percent.
fn star_slices(
    settings: harmonigraph_scene::SpectralAtmosphere,
    now: f64,
    layout: &StarLayout,
) -> [StarSlice; STAR_SLICES] {
    // Star pixels a second at a speed of one.
    let rate = star_px_per_second();
    let travel = now * rate;
    let (sin, cos) = f64::from(settings.cloud_direction).to_radians().sin_cos();
    let small = settings.star_size_min;
    let big = settings.star_size_max;
    std::array::from_fn(|k| {
        let d = star_depth(k);
        let along = d.powf(settings.star_size_curve);
        let cell = layout.cells[k];
        // The core and its cap grow with this depth's spacing over the fresh
        // 2-to-32 one at the same depth, so a bigger `Star size` is bigger
        // stars and not only sparser ones, and the fresh ends are 1 here.
        let scale = small / 2.0 * (big / small / 16.0).powf(along);
        let sigma = (0.5 + 0.8 * d) * scale;
        let cap = (0.33 * cell).min(1.8 * scale);
        let defocus = 1.0 + settings.star_defocus * d * d;
        let speed = f64::from(star_speed(settings, k));
        let shift = |axis: f64| {
            (axis * travel * speed / f64::from(cell)).rem_euclid(STAR_HASH_PERIOD) as f32
        };
        // The gap to the neighbouring depths' speeds, per depth step — the
        // mean of the two either side, or the one at an end — and the widest
        // a star's own speed may stray from this depth's, in star pixels a
        // second: half that gap, so the spreads meet, but no more than keeps
        // a whole life's excursion inside the budget. `Speed spread` is a
        // share of that.
        let (below, above) = (k.saturating_sub(1), (k + 1).min(STAR_SLICES - 1));
        let gap =
            (star_speed(settings, above) - star_speed(settings, below)) / (above - below) as f32;
        let lifetime = settings.star_lifetime;
        let widest = settings.star_speed_spread
            * (gap.abs() / 2.0 * rate as f32).min(2.0 * STAR_SPREAD_REACH * cell / lifetime);
        // The whole swing over a life, in cells: at most twice the budget,
        // clamped so rounding cannot put it a hair past.
        let swing = (widest * lifetime / cell).min(2.0 * STAR_SPREAD_REACH);
        let offset = [shift(cos), shift(sin)];
        let grid = layout.grids[k];
        // The cell a pixel at the pane's top left edge is in, as the shader
        // works it out, less the one the walk steps back and the margin.
        let origin: [i32; 2] =
            std::array::from_fn(|axis| star_origin(layout.pane[axis], cell, offset[axis]));
        StarSlice {
            offset,
            spread: [swing * cos as f32, swing * sin as f32],
            cell,
            sigma,
            cap,
            defocus,
            fringe: settings.star_fringe,
            reach: (STAR_REACH_CELLS - swing / 2.0) * cell,
            base: layout.bases[k] as i32,
            _pad: 0,
            origin,
            grid: grid.map(|side| side as i32),
        }
    })
}

/// The cell at the start of a slice's grid on one axis: the one a pixel at
/// the pane's leading edge is in, as the shader works it out, less the one the
/// walk steps back and the margin. `span` is the pane along the axis in star
/// pixels.
fn star_origin(span: f32, cell: f32, offset: f32) -> i32 {
    let edge = -f64::from(span / 2.0 / cell) - f64::from(offset);
    edge.floor() as i32 - 1 - STAR_GRID_MARGIN as i32
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
    // The starfield is always native: its target would have to hold colour
    // rather than one scalar, and a reduced star is a blurred one.
    let stars = settings.cloud_style == harmonigraph_scene::CloudStyle::Stars;
    if !settings.effects().cloud || pixel <= 1.0 || stars {
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
    // The starfield walks its own ring per pixel and reads no tile: what it
    // walks MOVES — every slice at its own speed, every star at its own and
    // on its own life — so there is no one fixed field to bake.
    if !settings.effects().cloud || settings.cloud_style == harmonigraph_scene::CloudStyle::Stars {
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
        harmonigraph_scene::CloudStyle::Stars => unreachable!("returned above"),
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
    /// 0 for the refracting scales, 1 for the watercolour wash, 2 for the
    /// starfield. None reads another's own settings; all share what sits above
    /// them.
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
    /// The starfield's one dial the shader reads directly, sanitized, and its
    /// life clock ([`star_life`]). `star_slices` lands on a 16-byte boundary
    /// right after them, as the shader's array must.
    star_randomness: f32,
    star_life: f32,
    star_slices: [StarSlice; STAR_SLICES],
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
/// otherwise shift every slice by a word on the Rust side only.
const _: () = assert!(
    std::mem::offset_of!(Uniforms, star_slices).is_multiple_of(16)
        && std::mem::size_of::<StarSlice>().is_multiple_of(16),
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
    /// read instead of walking the cells per pixel.
    pub tone: wgpu::RenderPipeline,
    /// One period of the cell walk into the two tile targets, for both of the
    /// above to read instead of walking the ring at all.
    pub tile: wgpu::RenderPipeline,
    /// Every star on screen into the star atlas, once a frame, for the
    /// composite's walk to read instead of working each star out per pixel.
    pub stars: wgpu::RenderPipeline,
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
            tile: tile_pipeline(
                device,
                source_layout,
                &composite_layout,
                "fs_cloud_tile",
                &[Some(TILE_FORMAT), Some(TILE_FORMAT)],
            ),
            stars: tile_pipeline(
                device,
                source_layout,
                &composite_layout,
                "fs_star_bake",
                &[Some(STAR_FORMAT)],
            ),
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

/// A bake: a full-screen triangle over its own targets, with no vertex buffer —
/// the tile's two, or the star atlas.
///
/// The tile declares the source layout at group 0 that it never reads, so the
/// pass can bind the same group every other cloud pass does; what it does read
/// is the cloud uniform at group 1, for the period and the style. The star
/// bake reads the palette there, and the light and the uniform at group 1.
fn tile_pipeline(
    device: &wgpu::Device,
    source_layout: &wgpu::BindGroupLayout,
    composite_layout: &wgpu::BindGroupLayout,
    entry: &str,
    formats: &[Option<wgpu::TextureFormat>],
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
            module: &shader,
            entry_point: Some("vs_cloud_tile"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
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
    /// The star atlas and its size, `None` unless the starfield is drawn. Part
    /// of the allocation key, sized by [`star_atlas_size`].
    stars: Option<(wgpu::TextureView, [u32; 2])>,
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
    /// Writes the star atlas, so that is the view it stands a scratch in for.
    star_group: Option<wgpu::BindGroup>,
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
    pub stars: Option<[u32; 2]>,
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
        let Allocation { size, tone: tone_size, tile: tile_key, carried, stars: star_size } =
            wanted;
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
        let stars =
            star_size.map(|size| (formatted("spectral_star_atlas", size, STAR_FORMAT), size));
        // What every group that does not read the atlas binds in its place, and
        // what the star pass, which writes it, must.
        let star_scratch = formatted("spectral_star_scratch", [1, 1], STAR_FORMAT);
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
        let cloud_group = |front, tone, tile: [&wgpu::TextureView; 2], stars| {
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
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: wgpu::BindingResource::TextureView(stars),
                    },
                ],
            })
        };
        let scratch_tile = [&views[0], &views[0]];
        let tile_views =
            tile.as_ref().map_or(scratch_tile, |tile| [&tile.views[0], &tile.views[1]]);
        let star_view = stars.as_ref().map_or(&star_scratch, |(view, _)| view);
        let bake_group = cloud_group(&views[1], &views[0], tile_views, star_view);
        // Reads the baked material the light passes just wrote, and writes the
        // tone target — so that is the one view it stands a scratch in for.
        let tone_group =
            tone.as_ref().map(|_| cloud_group(&source_view, &views[0], tile_views, star_view));
        let tile_group =
            tile.as_ref().map(|_| cloud_group(&source_view, &views[0], scratch_tile, star_view));
        // Reads the finished light as the stars' level, like the tone pass.
        let star_group =
            stars.as_ref().map(|_| cloud_group(&source_view, &views[0], tile_views, &star_scratch));
        let composite_group = cloud_group(
            &source_view,
            tone.as_ref().map_or(&views[0], |(view, _)| view),
            tile_views,
            star_view,
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
            stars,
            source_uniform,
            source_group,
            uniform,
            filter_groups,
            bake_group,
            tone_group,
            tile_group,
            star_group,
            composite_group,
        }
    }

    /// The reduced tone target's size, for the allocation key to compare
    /// against what this frame's settings ask for.
    pub fn tone_size(&self) -> Option<[u32; 2]> {
        self.tone.as_ref().map(|&(_, size)| size)
    }

    /// The star atlas's size, for the allocation key.
    pub fn star_size(&self) -> Option<[u32; 2]> {
        self.stars.as_ref().map(|&(_, size)| size)
    }

    /// The star atlas and the group the pass that fills it binds.
    pub fn star_pass(&self) -> Option<(&wgpu::TextureView, &wgpu::BindGroup)> {
        Some((&self.stars.as_ref()?.0, self.star_group.as_ref()?))
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

    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &self,
        queue: &wgpu::Queue,
        mut read: SpectrogramUniforms,
        rect: egui::Rect,
        ppp: f32,
        atmosphere: SpectrogramAtmosphere,
        tile: Option<TileKey>,
        stars: Option<StarLayout>,
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
                harmonigraph_scene::CloudStyle::Stars => 2,
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
            star_randomness: settings.star_randomness,
            star_life: star_life(settings, atmosphere.now),
            // Zeroes where the starfield is not drawn, which the shader never
            // reads then.
            star_slices: stars
                .map(|layout| star_slices(settings, atmosphere.now, &layout))
                .unwrap_or_default(),
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
        cloud_drift, retained_size, source_size, star_layout, star_slices, tile_key, tone_size,
        SpectrogramAtmosphere, CLOUD_UNITS, SCALE_CELLS, STAR_ATLAS_WIDTH, STAR_HASH_PERIOD,
        STAR_JITTER, STAR_LIFE_PERIOD, STAR_PANE, STAR_REACH_CELLS, STAR_SLICES, STAR_SPREAD_REACH,
        TILE_MAX, TILE_STEP, WASH_CELLS,
    };

    /// Every slice at `now` over a 16:9 pane.
    fn slices(
        settings: harmonigraph_scene::SpectralAtmosphere,
        now: f64,
    ) -> [super::StarSlice; STAR_SLICES] {
        star_slices(settings, now, &star_layout(settings, 16.0 / 9.0))
    }

    fn shader_number(name: &str) -> f64 {
        crate::shadow::tests::shader_const(crate::spectrogram::SPECTROGRAM_SRC, name)
            .trim_end_matches('u')
            .trim()
            .parse()
            .expect("a number")
    }

    /// The 3x3 walk each star slice takes sees every star whose light reaches
    /// the pixel.
    ///
    /// A centre strays `STAR_JITTER / 2` from its cell's middle, so the nearest
    /// a star from a cell OUTSIDE the ring can come to a pixel is
    /// [`STAR_REACH_CELLS`] — and the shader fades every star to zero by then. That makes the walk exact rather than
    /// "close enough": the prototype's reach was 0.85 of a cell at its V3, and
    /// its own defocus multiplies past the cell cap, so at the fourth depth the
    /// biggest cores are 0.39 of a cell wide and would have left a tenth of
    /// their peak on the far side of a cell edge without the fade.
    ///
    /// The spread's excursion is on top of both, along the drift, and each
    /// slice takes it off its own reach — so the scan runs per slice, at the
    /// widest spread over the longest life the fastest drift allows, where
    /// only the clamp on the spread keeps a small cell's stars in the ring.
    ///
    /// Measured by scanning the geometry rather than trusting the one-line
    /// formula, at the extremes of every dial that moves a star's extent, and
    /// with the jitter, slice count and periods read off the shipped shader.
    #[test]
    fn the_star_ring_holds_every_star_that_reaches_a_pixel() {
        assert_eq!(STAR_JITTER, shader_number("STAR_JITTER") as f32);
        assert_eq!(STAR_SLICES as f64, shader_number("STAR_SLICES"));
        assert_eq!(STAR_HASH_PERIOD, shader_number("STAR_HASH_PERIOD"));
        assert_eq!(STAR_LIFE_PERIOD, shader_number("STAR_LIFE_PERIOD"));
        // The whole budget still leaves the ring the half cell it holds.
        assert!((STAR_REACH_CELLS - STAR_SPREAD_REACH - 0.5).abs() < 1e-6);
        let fresh = harmonigraph_scene::SpectralAtmosphere::default();
        // The spread at its widest, lived as long as the dial allows, at the
        // fastest drift: every slice's spread is cut to fit here.
        let spread = harmonigraph_scene::SpectralAtmosphere {
            star_speed_spread: 1.0,
            star_lifetime: harmonigraph_scene::STAR_LIFETIME_MAX,
            cloud_direction: 30.0,
            star_speed_min: harmonigraph_scene::STAR_SPEED_MIN,
            star_speed_max: harmonigraph_scene::STAR_SPEED_MAX,
            ..fresh
        };
        let extremes = [
            fresh,
            harmonigraph_scene::SpectralAtmosphere {
                star_defocus: harmonigraph_scene::STAR_DEFOCUS_MAX,
                star_fringe: harmonigraph_scene::STAR_FRINGE_MAX,
                star_density: harmonigraph_scene::STAR_DENSITY_MAX,
                star_size_min: harmonigraph_scene::STAR_SIZE_MIN,
                star_size_max: harmonigraph_scene::STAR_SIZE_MAX,
                ..spread
            },
            harmonigraph_scene::SpectralAtmosphere {
                star_density: harmonigraph_scene::STAR_DENSITY_MIN,
                star_size_min: harmonigraph_scene::STAR_SIZE_MAX,
                star_size_max: harmonigraph_scene::STAR_SIZE_MAX,
                ..spread
            },
            // The steepest speed gaps sit at the far end at the low curve and
            // at the near end at the high one.
            harmonigraph_scene::SpectralAtmosphere {
                star_speed_curve: harmonigraph_scene::STAR_SPEED_CURVE_MIN,
                star_size_curve: harmonigraph_scene::STAR_SIZE_CURVE_MIN,
                ..spread
            },
            harmonigraph_scene::SpectralAtmosphere {
                star_speed_curve: harmonigraph_scene::STAR_SPEED_CURVE_MAX,
                star_size_curve: harmonigraph_scene::STAR_SIZE_CURVE_MAX,
                ..spread
            },
        ];
        for settings in extremes {
            for slice in slices(settings, 0.0) {
                let excursion = slice.spread[0].hypot(slice.spread[1]) / 2.0;
                assert!(excursion <= STAR_SPREAD_REACH + 1e-6, "{settings:?}: {slice:?}");
                // The scan: a pixel anywhere in cell (0, 0), a star in every
                // cell one ring out pushed as far toward it as the jitter and
                // the spread allow.
                let stray = STAR_JITTER / 2.0 + excursion;
                let reach = slice.reach / slice.cell;
                assert!(
                    reach >= 0.5 - 1e-6,
                    "{settings:?}: the ring holds almost nothing at {reach} cells"
                );
                assert!((reach - (STAR_REACH_CELLS - excursion)).abs() < 1e-5, "{slice:?}");
                let nearest = nearest_outside_the_ring(stray);
                assert!(
                    nearest >= reach - 1e-5,
                    "a star outside the ring comes {nearest} cells from the pixel, inside the \
                     {reach} it is windowed to zero at"
                );
            }
        }
        // ...and the excursion is real: at the widest spread the small cells
        // take the whole budget, where with none they take nothing.
        let widest = slices(extremes[2], 0.0);
        let swing = widest[0].spread[0].hypot(widest[0].spread[1]);
        assert!((swing - 2.0 * STAR_SPREAD_REACH).abs() < 1e-5, "{swing}");
        let still = slices(
            harmonigraph_scene::SpectralAtmosphere { star_speed_spread: 0.0, ..spread },
            0.0,
        );
        assert!(still.iter().all(|slice| slice.spread == [0.0; 2]));
    }

    /// The nearest a star from a cell outside the 3x3 walk round cell (0, 0) can
    /// come to a pixel inside it, when a centre strays `stray` either way on
    /// each axis from its cell's middle.
    fn nearest_outside_the_ring(stray: f32) -> f32 {
        let mut nearest = f32::INFINITY;
        for step in 0..=64 {
            for other in 0..=64 {
                let pixel = [step as f32 / 64.0, other as f32 / 64.0];
                for cx in -2i32..=2 {
                    for cy in -2i32..=2 {
                        if cx.abs() < 2 && cy.abs() < 2 {
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
            star_density: harmonigraph_scene::STAR_DENSITY_MAX,
            star_size_min: harmonigraph_scene::STAR_SIZE_MIN,
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
            let layout = star_layout(settings, aspect);
            assert!(layout.fits(), "{layout:?}");
            let size = pixels.map(|side| side as f32 / ppp);
            for (k, slice) in star_slices(settings, 0.0, &layout).iter().enumerate() {
                for axis in 0..2 {
                    let half = f64::from(layout.pane[axis] / 2.0 / slice.cell);
                    // Drifts that put the leading edge, then the trailing one, on
                    // a whole cell: `-half - offset` and `half - offset` integers.
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
                                let cell = (sp / slice.cell - offset).floor() as i32;
                                for step in [-1, 1] {
                                    let local = cell + step - origin;
                                    assert!(
                                        (0..slice.grid[axis]).contains(&local),
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
    /// fresh starfield on a plain and a very wide pane — and otherwise raises
    /// only the finest cells, to the least that fits.
    #[test]
    fn the_star_atlas_floors_only_the_finest_cells_and_only_past_its_budget() {
        let fresh = harmonigraph_scene::SpectralAtmosphere::default();
        for aspect in [16.0 / 9.0, 8.0] {
            let layout = star_layout(fresh, aspect);
            assert_eq!(layout.cells, super::star_cells(fresh), "{aspect}");
        }
        let fine = harmonigraph_scene::SpectralAtmosphere {
            star_density: harmonigraph_scene::STAR_DENSITY_MAX,
            star_size_min: harmonigraph_scene::STAR_SIZE_MIN,
            ..fresh
        };
        let wanted = super::star_cells(fine);
        let layout = star_layout(fine, 16.0 / 9.0);
        let floor = layout.cells[0];
        assert!(floor > wanted[0], "{layout:?}");
        assert!(!super::StarLayout::at(wanted, floor * 0.99, 16.0 / 9.0).fits());
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
            cloud_direction: 0.0,
            star_speed_min: 0.15,
            star_speed_max: 1.0,
            ..Default::default()
        };
        let travelled = |settings, now| {
            slices(settings, now)
                .map(|slice| [slice.offset[0] * slice.cell, slice.offset[1] * slice.cell])
        };
        let near = super::star_px_per_second() as f32 * 10.0;
        assert!((near / 540.0 / 10.0 - 0.114).abs() < 0.001);
        let moved = travelled(fresh, 10.0);
        assert!((moved[STAR_SLICES - 1][0] - near).abs() < 0.01, "{moved:?}");
        assert!((moved[0][0] - 0.15 * near).abs() < 0.01, "{moved:?}");
        assert!(moved.iter().all(|m| m[1].abs() < 1e-3));
        assert!(moved.windows(2).all(|w| w[0][0] < w[1][0]), "nearer is not faster: {moved:?}");
        let together = travelled(
            harmonigraph_scene::SpectralAtmosphere { star_speed_min: 1.0, ..fresh },
            10.0,
        );
        assert!(together.iter().all(|m| (m[0] - near).abs() < 0.01), "{together:?}");
        let clouds = harmonigraph_scene::SpectralAtmosphere { cloud_speed: 20.0, ..fresh };
        assert_eq!(travelled(clouds, 10.0), moved);
        // A session left running for days still hands the shader an offset
        // inside one hash period rather than millions of pixels.
        for slice in slices(fresh, 3.0e5) {
            assert!(slice.offset.iter().all(|&o| (0.0..=STAR_HASH_PERIOD as f32).contains(&o)));
        }
    }

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
