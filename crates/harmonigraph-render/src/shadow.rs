//! The shadow atlas: every caster's ink blurred into a cell of its own, which
//! is what that caster multiplies the frame by in its own draw
//! (`fs_shadow_box` in text.wgsl).
//!
//! ONE cell per caster, at a scale that keeps the blur's cost flat: the cell is
//! the caster's box grown by the kernel's reach, drawn at `min(1, 3 / σ)` of
//! the target's pixels, so σ is at most `SIGMA_CELL_MAX` texels in every cell
//! and the kernel at most nineteen taps whatever the Shadow bar says. The atlas
//! is about the names' own area at the fresh Shadow, and shrinks as the bar
//! widens.
//!
//! What lives here is the arithmetic that runs without a GPU — the packer and
//! σ — beside the textures and the blur passes over them (shadow.wgsl). The
//! rasterizer that fills a cell is the caster's own: a name's is text.wgsl's
//! `fs_glyph_ink`, drawn at the cell's transform by `vs_glyph_cell`.

use crate::wgpu;

const SHADOW_SRC: &str = include_str!("shaders/shadow.wgsl");

/// Blurred coverage (red) and its local source-opacity ceiling (green).
/// The second half-float lets lattice nodes retain Gaussian gain without
/// clipping away a slice's fade. Fixed casters only read red.
///
/// Half floats rather than a byte because the blur's tail is MULTIPLIED into
/// the frame: a tail quantized to 1/255 steps across a wide soft shadow, where
/// the light under it has no steps of its own to hide them in.
pub(crate) const ATLAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Float;

/// The most texels a cell's σ may be. What bounds the kernel, at
/// `2 * ceil(REACH_SIGMAS * this) + 1` taps, and so the cost of a shadow at
/// any width: past this a cell is drawn SMALLER rather than blurred wider.
/// `MAX_RADIUS` in shadow.wgsl is the loop bound this implies.
pub(crate) const SIGMA_CELL_MAX: f32 = 3.0;

/// How many σ out a blurred cell is padded, and how far its kernel reaches —
/// `REACH` in shadow.wgsl, pinned by
/// `the_blurs_reach_and_loop_bound_are_the_packers_own`.
pub(crate) const REACH_SIGMAS: f32 = 3.0;

/// How many σ a style's shadow reaches past its caster's ink: the blur's own
/// [`REACH_SIGMAS`] plus the source's spread, which is measured in σ too. What
/// every quad and every cell's pad is grown by.
pub(crate) fn reach_sigmas(style: harmonigraph_scene::ShadowStyle) -> f32 {
    REACH_SIGMAS + style.gaussian_spread_points(1.0)
}

/// Screen-point reference behind the spectral groups' dimensionless Shadow
/// width. One unit is four points; the bar's upper limit lives in the scene.
/// Changing pane size or pitch zoom does not change it.
pub const SPECTRAL_WIDTH_POINTS: f32 = 4.0;

/// A spectral style's σ in screen points.
pub(crate) fn spectral_sigma_points(
    style: harmonigraph_scene::ShadowStyle,
    point_scale: f32,
) -> f32 {
    sigma_points(style.width, SPECTRAL_WIDTH_POINTS) * point_scale.max(0.0)
}

/// How far a spectral style's shadow can paint past its caster.
/// `point_scale` scales the drawing after clamping the dial, so an enlarged
/// composed preview can exceed the dial's maximum point reach.
pub fn spectral_shadow_reach(style: harmonigraph_scene::ShadowStyle, point_scale: f32) -> f32 {
    let style = style.clamped(harmonigraph_scene::SPECTRAL_SHADOW_MAX);
    if style.casts() {
        spectral_sigma_points(style, point_scale) * reach_sigmas(style)
    } else {
        0.0
    }
}

/// σ of a caster's shadow in the pane's POINTS, for a group whose Shadow is
/// `width` node radii over a node of `node_points` points.
///
/// HALF the bar's width. A half-plane blurred at σ keeps `erfc(d / (σ√2)) / 2`
/// of the light `d` out from its edge, which at `d = 2σ` is 2.3% — so one
/// Shadow width is where a wide caster's shadow has all but run out, which is
/// what the bar says it is.
///
/// PER CASTER and not per frame: a caster carries the σ its style group asked
/// for ([`Caster::sigma_points`]), so a frame whose groups disagree packs each
/// at its own width with no second conversion between them.
///
/// In points and not in the target's pixels, so the target's own scale enters
/// once, where the cell is SIZED (`pack`'s `px_per_point`, the device's scale
/// times the render scale, which is the term #496 found missing from the
/// field's reach). Written as the POSITIVE test so a NaN out of a corrupt blob
/// is no shadow rather than a kernel of NaNs.
pub(crate) fn sigma_points(width: f32, node_points: f32) -> f32 {
    let sigma = 0.5 * width * node_points;
    if sigma > 0.0 {
        sigma
    } else {
        0.0
    }
}

/// What a caster hands the packer: its ink's bounding box in the pane's points
/// (min, then size), how much of its shadow lands, 0..=1, and the style its
/// group is dialled to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Caster {
    pub rect: [f32; 4],
    pub level: f32,
    /// This caster's own σ in the pane's points ([`sigma_points`]) — half its
    /// group's Shadow width over a node's radius.
    ///
    /// Per caster because node geometry and lattice notation are separate
    /// groups (`ShadowStyle`). Zero is a caster that darkens nothing — either
    /// of its group's bars at the bottom — and takes no cell at all.
    ///
    /// The cell's scale, pad and σ in texels all come off this, so a caster at
    /// three times the width is a cell drawn a third the size rather than a
    /// kernel three times as wide: the blur's tap count is flat in this exactly
    /// as it is in the bar.
    pub sigma_points: f32,
    /// Source expansion in pane points before the blur
    /// ([`harmonigraph_scene::ShadowStyle::gaussian_spread_points`]). Zero for
    /// text, whose Spread is [`Self::stops`] instead.
    pub spread_points: f32,
    /// How many stops the blurred cell is lifted by where it is read, before
    /// the clamp ([`harmonigraph_scene::ShadowStyle::text_spread_stops`]).
    /// Text's Spread; zero for every other caster.
    pub stops: f32,
}

/// The caster a name's glyphs make, on the lattice and the spectral panes
/// alike: one per name, its glyphs drawn into its one cell under MAX. The box
/// round every glyph's rect, the strength the name's rim colour carries, and
/// the style its text group is dialled to: its σ, and its Spread as
/// [`Caster::stops`], which lifts the blur of the word's whole ink.
///
/// The strength is the one number a lattice name's `rim` holds (`LABEL_SHADOW`
/// in harmonigraph_ui), so a name easing in as the marker under it eases out
/// grows its shadow on the clock its ink arrives on. A spectral name's rim is
/// the knockout colour its shadow is drawn in (`fs_spectral_shadow`), which
/// spends the strength itself, so there the level only decides whether the name
/// casts at all.
///
/// A run with no ink in it — every rect empty — is a caster of nothing, with
/// its level zeroed rather than a box of infinities for the packer to size.
pub(crate) fn caster_of(glyphs: &[crate::GlyphInstance], sigma_points: f32, stops: f32) -> Caster {
    let (mut min, mut max) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for g in glyphs {
        for axis in 0..2 {
            min[axis] = min[axis].min(g.rect[axis]);
            max[axis] = max[axis].max(g.rect[axis] + g.rect[axis + 2]);
        }
    }
    let empty = Caster { rect: [0.0; 4], level: 0.0, sigma_points, spread_points: 0.0, stops };
    if !(max[0] > min[0] && max[1] > min[1]) {
        return empty;
    }
    let level = glyphs.iter().map(|g| f32::from(g.rim[3]) / 255.0).fold(0.0, f32::max);
    Caster { rect: [min[0], min[1], max[0] - min[0], max[1] - min[1]], level, ..empty }
}

/// One caster's cell, as every draw that touches it takes it: the cell's
/// rasterizer (`vs_glyph_cell` in text.wgsl), the blur passes (`vs_cell` in
/// shadow.wgsl) and the multiply in the scene pass (`vs_shadow_box`).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ShadowBox {
    /// The caster's box grown by the blur's reach, in the pane's points: min,
    /// then size. The quad the scene pass multiplies over, and the region the
    /// cell is a picture of.
    pub rect: [f32; 4],
    /// The cell in atlas texels: origin, then size, all whole numbers.
    pub cell: [f32; 4],
    /// x: the scale from points to cell texels; y: σ in cell texels; z: the
    /// caster's level, 0..=1; w: the cell's share of the TARGET's pixels,
    /// `min(1, SIGMA_CELL_MAX / σ)`.
    ///
    /// The last is what a cell's own rasterizer antialiases against
    /// (`aa_width` in lattice.wgsl): a fragment of the cell is one pane pixel
    /// divided by it, and a soft band taken off that fragment alone is the
    /// Shadow bar times a constant rather than a screen width.
    pub cell_map: [f32; 4],
    /// x: which caster this box belongs to, as an index into
    /// [`Packed::casters`]; y: 1 on the first glyph of a spectral name in the
    /// text's per-glyph copy (`spectral_shadow::finish_for_pass`), the one
    /// instance that draws the name's shadow, and zero everywhere else, which
    /// nothing reads; z: how far past the caster's ink
    /// this cell reaches, in the pane's points — the pad the rect was grown
    /// by; w: the caster's source expansion before blur, in points.
    ///
    /// x is read by the node's SCENE draw, which needs the caster's whole entry
    /// and so reaches the array rather than the box; z and w by the passes that
    /// fill the CELLS, which take one at a time. Carried on the box rather than in a buffer of its
    /// own because the node's two draws — the one that fills a cell and the one
    /// that reads the atlas — bind the same stream, and one row is cheaper than
    /// a second binding.
    pub who: [f32; 4],
}

impl ShadowBox {
    pub(crate) const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ShadowBox>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4
        ],
    };

    /// The same rows at the locations after a glyph's five, for the draw that
    /// rasterizes a glyph into its cell alongside `GlyphInstance::LAYOUT`.
    pub(crate) const BESIDE_GLYPHS: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ShadowBox>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4
        ],
    };

    /// The same rows again, at the four locations a node's own instance rows
    /// leave free (`Instance` in lattice.wgsl, `GpuInstance::LAYOUT`) — the
    /// second instance-step buffer the node draw and the cell draw both bind.
    ///
    /// Scattered rather than consecutive because a vertex attribute's location
    /// has to be under sixteen and a node's rows already reach fifteen; which
    /// four are free is what picks them. Together the two fill all sixteen, so
    /// a node attribute added now has to share a slot with one already there.
    pub(crate) const BESIDE_NODES: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ShadowBox>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            5 => Float32x4, 9 => Float32x4, 14 => Float32x4, 13 => Float32x4
        ],
    };

    /// After a roll instance's eleven attributes, for rasterizing its box SDF
    /// into its cell.
    pub(crate) const BESIDE_ROLL: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ShadowBox>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            10 => Float32x4, 11 => Float32x4, 12 => Float32x4, 13 => Float32x4
        ],
    };

    /// After a spiral dot's centre, radius and color, for rasterizing its
    /// circle field into its cell.
    pub(crate) const BESIDE_DOTS: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ShadowBox>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4
        ],
    };
}

/// A caster with no cell at all: what a draw carries when the frame packed
/// nothing, and what every reader of a box answers 1 to (`shadow_through` in
/// lattice.wgsl, `fs_shadow_box` in text.wgsl) — the frame left exactly whole,
/// with nothing sampled.
pub(crate) const NO_CELL: ShadowBox =
    ShadowBox { rect: [0.0; 4], cell: [0.0; 4], cell_map: [0.0; 4], who: [0.0; 4] };

/// A frame's cells, packed: one box per caster in the order they arrived, the
/// same casters gathered as the scene pass reads them, and the atlas size that
/// holds it all.
///
/// The two views of one packing. `boxes` is what FILLS and BLURS a cell — one
/// instance per cell, carrying its own σ and scale — and `casters` is what
/// SAMPLES them, one entry per caster carrying the quad it is drawn over.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Packed {
    pub boxes: Vec<ShadowBox>,
    pub casters: Vec<ShadowCaster>,
    pub size: [u32; 2],
}

/// One caster's shadow, as the scene pass reads it: the quad to draw over, the
/// cell to sample, and what to spend what it holds as.
///
/// In a STORAGE BUFFER rather than beside the instance. A node's own rows reach
/// location 15 and leave five free, and a caster is four of them; the cell is
/// read by a node, a marker and a name alike, so one array they all index is
/// also one place the shape is written down. Indexed by the caster's own index
/// — the order `pack` was handed — so nothing carries a second copy.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ShadowCaster {
    /// The caster's padded box, in the pane's points: min, then size. What its
    /// shadow is drawn OVER, so the reach finishes inside the quad rather than
    /// being cut off in a straight line at it.
    pub rect: [f32; 4],
    /// The cell in atlas texels: origin, then size. Zeroed whole where the
    /// caster casts nothing.
    pub cell: [f32; 4],
    /// The map from a point of the pane to a texel of that cell: x/y the
    /// origin, z the scale, so a texel is `xy + points * z`. w is unused and
    /// zero: which node casters can cover a point is [`node_occluders`]'.
    ///
    /// Pre-composed on the CPU rather than sent as (cell origin, box origin,
    /// scale) for the shader to combine: a fragment would otherwise repeat the
    /// subtraction for a number that never changes within a frame.
    pub map: [f32; 4],
    /// What this caster SPENDS, none of it a coordinate.
    ///
    /// x: how much of its shadow lands, 0..=1 — zero where the atlas could not
    /// hold its cell, which is a caster that darkens nothing. y: the stops its
    /// blur is lifted by ([`Caster::stops`]). z and w are zero: the row is a
    /// vec4 so the storage array's stride needs no padding rule of its own.
    pub shade: [f32; 4],
}

/// A caster the frame packed nothing for: what every draw carries when the
/// Shadow is shut, and what a reader answers 1 to with nothing sampled.
pub(crate) const NO_CASTER: ShadowCaster =
    ShadowCaster { rect: [0.0; 4], cell: [0.0; 4], map: [0.0; 4], shade: [0.0; 4] };

/// Every caster's cell, shelf-packed in the order the casters arrive.
///
/// `px_per_point` is the target's pixels per pane point — the device's scale
/// times the render scale — and `max_side` the device's texture limit.
///
/// A PURE function of this frame, which is what the offline renderer's
/// determinism rests on: the layout depends on the casters — their boxes, their
/// σ and their spread — the target scale and the device limit, and nothing a
/// previous frame left behind. The texture that holds it may be larger than
/// `size` (it grows to demand and never shrinks, see [`ShadowTarget`]); the
/// cells' texel coordinates are absolute, so that changes nothing sampled.
///
/// A cell the atlas cannot hold — past `max_side` in either direction — is
/// packed as no cell at all, its level zeroed so the box draws nothing. At the
/// scales here that is over a hundred pane-fuls of names; the fallback
/// criterion in #498 is what a frame that reaches it calls for.
///
/// Every property that differs BETWEEN groups rides on the caster, so this walk
/// never learns what a group is: a mixed frame is one shelf of cells whose
/// widths differ, packed in one pass.
pub(crate) fn pack(casters: &[Caster], px_per_point: f32, max_side: u32) -> Packed {
    let mut packed = Packed::default();
    pack_into(casters, px_per_point, max_side, &mut packed, &mut PackScratch::default());
    packed
}

/// What [`pack_into`] works in, kept between frames with the packing it fills.
#[derive(Default)]
pub(crate) struct PackScratch {
    rects: Vec<[f32; 4]>,
    sizes: Vec<[u32; 2]>,
    placed: Vec<[u32; 2]>,
}

/// [`pack`], into last frame's buffers rather than new ones: at the node cap
/// they come to a couple of megabytes a frame. Every one is emptied first, so
/// the packing is still this frame's alone.
pub(crate) fn pack_into(
    casters: &[Caster],
    px_per_point: f32,
    max_side: u32,
    packed: &mut Packed,
    scratch: &mut PackScratch,
) {
    packed.boxes.clear();
    packed.casters.clear();
    packed.size = Packed::default().size;
    // A finite positive number, which a NaN or an infinity out of a corrupt
    // blob is not: either is no shadow rather than a kernel of nothing.
    let positive = |x: f32| x.is_finite() && x > 0.0;
    if casters.is_empty() || !positive(px_per_point) {
        return;
    }
    let spread_of = |c: &Caster| c.spread_points.max(0.0);
    // One caster's σ in the target's pixels, which is where its cell is drawn.
    //
    // Floored at zero because `shape` runs for every caster, including the
    // ones `casts` goes on to reject: a NaN σ out of a corrupt blob would
    // otherwise reach `min`, which returns its OTHER operand for a NaN, and
    // come out as a NaN pad on a box the vertex stage reads.
    let sigma_of = |c: &Caster| (c.sigma_points * px_per_point).max(0.0);
    // A cell, sized off ITS OWN σ: drawn at `min(1, SIGMA_CELL_MAX / σ)` of the
    // target's pixels so σ is at most `SIGMA_CELL_MAX` texels whatever the
    // caster asked for, and padded by the kernel's reach in those same texels,
    // plus one so the scene pass's bilinear tap at the box's own edge still
    // lands inside the cell. That cap is what holds the blur's cost flat across
    // the whole bar: the chain reads σ off the cell and clamps its taps to that
    // cell's rect, so a wider Shadow is a smaller cell rather than a wider
    // kernel.
    let shape = |c: &Caster| {
        let sigma = sigma_of(c);
        let scale = (SIGMA_CELL_MAX / sigma).min(1.0);
        let k = scale * px_per_point;
        let texels = sigma * scale;
        let pad = ((REACH_SIGMAS * texels).ceil() + 1.0) / k + spread_of(c);
        (scale, k, texels, pad)
    };
    // What a caster comes to where it is spent. A level at zero, a NaN out of a
    // corrupt blob, or a group with either of its bars at the bottom — the
    // width arriving as a σ of nothing — all darken nothing.
    let casts = |c: &Caster| c.level.clamp(0.0, 1.0) > 0.0 && positive(c.sigma_points);

    // One entry per caster, in the order they arrived — the order every index
    // below is in.
    let PackScratch { rects, sizes, placed } = scratch;
    rects.clear();
    sizes.clear();
    placed.clear();
    for caster in casters {
        // A caster that darkens nothing takes NO cell: a reader hands the
        // frame back whole at level 0, so the cell it would fill is one
        // nothing ever samples. Nodes clipped off the pane and nodes
        // projected behind the eye arrive here at level 0 in numbers
        // (`node_caster`), and a cell each would widen the atlas every blur
        // pass sweeps and be rasterized by the whole node shader for no
        // picture at all.
        if !casts(caster) {
            rects.push([0.0; 4]);
            sizes.push([0, 0]);
            continue;
        }
        let (_, k, _, pad) = shape(caster);
        let r = caster.rect;
        let rect = [r[0] - pad, r[1] - pad, r[2] + 2.0 * pad, r[3] + 2.0 * pad];
        let texels = |points: f32| ((points * k).ceil() as u32).max(1);
        rects.push(rect);
        sizes.push([texels(rect[2]), texels(rect[3])]);
    }
    // Wide enough for the widest cell and about square over the total area,
    // so a pane's worth of names packs into a few shelves rather than one.
    let widest = sizes.iter().map(|[w, _]| *w).max().unwrap_or(1);
    let area: u64 = sizes.iter().map(|[w, h]| u64::from(*w) * u64::from(*h)).sum();
    let square = ((area as f64 * 4.0 / 3.0).sqrt().ceil() as u32).max(1);
    let width = widest.max(square).next_power_of_two().min(max_side);
    let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
    for &[w, h] in sizes.iter() {
        // A caster with no cell is not shelved, and holds its index with a
        // placement `fits` reads as no cell below.
        if w == 0 || h == 0 {
            placed.push([0, 0]);
            continue;
        }
        if x + w > width && x > 0 {
            y += shelf;
            x = 0;
            shelf = 0;
        }
        placed.push([x, y]);
        x += w;
        shelf = shelf.max(h);
    }
    let height = (y + shelf).next_power_of_two().min(max_side);

    // A cell the atlas could not hold is a caster that darkens NOTHING rather
    // than one drawn at part of its shadow: the quad is still billboarded on
    // the reach the bar names, so half a shadow inside it would be a hard edge
    // at the box.
    let fits = |i: usize| {
        let [w, h] = sizes[i];
        let [x, y] = placed[i];
        w > 0 && h > 0 && x + w <= width && y + h <= height
    };
    packed.boxes.reserve(rects.len());
    packed.casters.reserve(casters.len());
    for (c, caster) in casters.iter().enumerate() {
        let whole = casts(caster) && fits(c);
        let level = if whole { caster.level.clamp(0.0, 1.0) } else { 0.0 };
        let mut entry = NO_CASTER;
        let (scale, k, sigma, pad) = shape(caster);
        let rect = rects[c];
        let [w, h] = sizes[c];
        let [x, y] = placed[c];
        let cell = if whole { [x as f32, y as f32, w as f32, h as f32] } else { [0.0; 4] };
        packed.boxes.push(ShadowBox {
            rect,
            cell,
            cell_map: [k, sigma, level, scale],
            who: [c as f32, 0.0, pad, spread_of(caster)],
        });
        if whole {
            entry.shade = [level, caster.stops.max(0.0), 0.0, 0.0];
            entry.cell = cell;
            entry.map = [cell[0] - rect[0] * k, cell[1] - rect[1] * k, k, 0.0];
            entry.rect = rect;
        }
        packed.casters.push(entry);
    }
    packed.size = [width, height];
}

/// One pane's atlas: the plane every cell is drawn into, and the blur's
/// intermediate for the frames that run one.
///
/// [`atlas`](Self::atlas) holds the casters' ink after the pre-pass and the
/// finished blur after [`blur`](Self::blur). Grown on demand and never shrunk
/// (`Offscreen::ensure_shadow`), on the pane's own
/// [`Offscreen`](crate::Offscreen) so two panes never share one.
pub(crate) struct ShadowTarget {
    pub(crate) size: [u32; 2],
    atlas: Plane,
    /// The half-blurred middle: a second plane the atlas's whole size, made
    /// with it, since a frame holds an atlas only while it packs a cell and
    /// every cell is blurred. [`blur`](Self::blur) reads one at the other's
    /// coordinates.
    half: Plane,
}

/// One atlas-sized Rg16Float target and the bind group that reads it, which are
/// made together because neither is any use without the other.
struct Plane {
    /// Kept only so a test can put ink in and read the blur back out
    /// (`a_cells_blur_stays_inside_its_own_cell_and_keeps_its_mass`). The
    /// `COPY_*` usages that needs are granted whatever the build, being a
    /// property of the texture rather than of the test.
    #[cfg(test)]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Reading `view`, as every consumer of the atlas takes it
    /// ([`read_layout`]).
    read: wgpu::BindGroup,
}

impl Plane {
    fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        size: [u32; 2],
        label: &str,
    ) -> Plane {
        let texture = crate::gpu_memory::texture(
            device,
            &wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: ATLAS_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            },
        );
        let view = texture.create_view(&Default::default());
        let read = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lattice_shadow_atlas_read"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        Plane {
            #[cfg(test)]
            texture,
            view,
            read,
        }
    }
}

impl ShadowTarget {
    pub(crate) fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        size: [u32; 2],
    ) -> Self {
        let size = [size[0].max(1), size[1].max(1)];
        let atlas = Plane::new(device, layout, sampler, size, "lattice_shadow_atlas");
        let half = Plane::new(device, layout, sampler, size, "lattice_shadow_atlas_half");
        ShadowTarget { size, atlas, half }
    }

    /// Make `slot` hold an atlas a layout of `size` fits in: grown to demand,
    /// never shrunk, and the old atlas dropped BEFORE its replacement is built,
    /// so a growth never holds both (the resize test, #1508, found both callers
    /// building first). Nothing carries across a rebuild: the ink pass clears
    /// the atlas every frame it is used.
    pub(crate) fn ensure(
        slot: &mut Option<Self>,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        size: [u32; 2],
    ) {
        if slot.as_ref().is_some_and(|target| target.holds(size)) {
            return;
        }
        let held = slot.take().map_or([0, 0], |target| target.size);
        *slot =
            Some(Self::new(device, layout, sampler, [size[0].max(held[0]), size[1].max(held[1])]));
    }

    /// Whether this atlas can hold a layout of `size`.
    fn holds(&self, size: [u32; 2]) -> bool {
        self.size[0] >= size[0] && self.size[1] >= size[1]
    }

    /// The finished atlas as every scene draw binds it ([`read_layout`]).
    pub(crate) fn read(&self) -> &wgpu::BindGroup {
        &self.atlas.read
    }

    /// The atlas's own texture, for the tests that write ink into it and read
    /// the finished cells back out.
    #[cfg(test)]
    pub(crate) fn texture(&self) -> &wgpu::Texture {
        &self.atlas.texture
    }

    /// The pass that fills the atlas with the casters' ink: cleared, then the
    /// caller's draws.
    pub(crate) fn ink_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
    ) -> wgpu::RenderPass<'a> {
        Self::pass(
            encoder,
            "lattice_shadow_ink_pass",
            &self.atlas.view,
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
        )
    }

    /// The two blur passes over `count` cells of `boxes`, leaving the finished
    /// atlas in [`atlas`](Self::atlas).
    ///
    /// The intermediate is cleared and the atlas is loaded: every cell has its
    /// own rect redrawn in full on the way back, so a clear would buy nothing.
    pub(crate) fn blur(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: (&wgpu::RenderPipeline, &wgpu::RenderPipeline),
        boxes: &wgpu::Buffer,
        count: u32,
    ) {
        let half = &self.half;
        let (blur_x, blur_y) = pipelines;
        {
            let mut pass = Self::pass(
                encoder,
                "lattice_shadow_blur_pass",
                &half.view,
                wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            );
            pass.set_pipeline(blur_x);
            pass.set_bind_group(0, &self.atlas.read, &[]);
            pass.set_vertex_buffer(0, boxes.slice(..));
            pass.draw(0..4, 0..count);
        }
        let mut pass =
            Self::pass(encoder, "lattice_shadow_blur_pass", &self.atlas.view, wgpu::LoadOp::Load);
        pass.set_pipeline(blur_y);
        pass.set_bind_group(0, &half.read, &[]);
        pass.set_vertex_buffer(0, boxes.slice(..));
        pass.draw(0..4, 0..count);
    }

    fn pass<'a>(
        encoder: &'a mut wgpu::CommandEncoder,
        label: &'static str,
        target: &'a wgpu::TextureView,
        load: wgpu::LoadOp<wgpu::Color>,
    ) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        })
    }
}

/// How the atlas is read, by the blur and by the scene pass alike: the
/// texture, filterable, and a sampler. The blur takes texels by `textureLoad`
/// and leaves the sampler alone; the scene pass's one tap is bilinear, which
/// is what lets a cell drawn at a fraction of the target's pixels come back
/// smooth.
pub(crate) fn read_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("lattice_shadow_atlas_layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// What binds the casters to the scene pipelines, at group 3: the casters'
/// kernels at binding 0 and the [`node_occluders`] grid at binding 1, both
/// read-only storage.
///
/// A group of its own rather than more bindings beside the atlas, because
/// the two have different lifetimes and different readers. The atlas layout is
/// shared with the BLUR pipelines, which sweep the cells and have no use for
/// the array; and the atlas's own bind groups are made once with its textures,
/// where these buffers are rewritten and regrown every frame. One layout
/// carrying both would rebuild the atlas's bind groups whenever a name arrived.
pub(crate) fn caster_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let storage = |binding, visibility| wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("lattice_shadow_casters_layout"),
        entries: &[
            // The VERTEX stage as well: a caster's quad is its widest term's
            // box, and that box is in here (`vs_shadow_box` in text.wgsl).
            storage(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
            // Read by `node_visibility` alone, which only fragments call.
            storage(1, wgpu::ShaderStages::FRAGMENT),
        ],
    })
}

/// A buffer for `capacity` casters' kernels and the bind group naming it,
/// for a surface with no node casters: its occluder grid is one zero word,
/// which `node_visibility` reads as an empty grid.
pub(crate) fn caster_buffer(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    capacity: usize,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let (casters, _, bind_group) = caster_buffers(device, layout, capacity, 1);
    (casters, bind_group)
}

/// Buffers for `casters` kernels and `occluders` words of [`node_occluders`],
/// and the bind group naming both.
///
/// The three together because they cannot come apart: a storage buffer's bind
/// group names the buffer, so a pane that outgrows either rebuilds all three.
/// Each is floored at one entry, an empty storage binding being a validation
/// error and a frame with nothing in the list still having to bind SOMETHING
/// for the pipeline's layout. A fresh buffer is zeroed, which is an empty grid.
pub(crate) fn caster_buffers(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    casters: usize,
    occluders: usize,
) -> (wgpu::Buffer, wgpu::Buffer, wgpu::BindGroup) {
    let buffer = |label, bytes: usize| {
        crate::gpu_memory::buffer(
            device,
            &wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        )
    };
    let kernels =
        buffer("lattice_shadow_casters", std::mem::size_of::<ShadowCaster>() * casters.max(1));
    let grid = buffer("lattice_node_occluders", std::mem::size_of::<u32>() * occluders.max(1));
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("lattice_shadow_casters_bind_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: kernels.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: grid.as_entire_binding() },
        ],
    });
    (kernels, grid, bind_group)
}

/// Words ahead of the offsets in [`node_occluders`]: columns, rows, the grid's
/// origin in points (x then y, as f32 bits) and bins per point (f32 bits).
pub(crate) const OCCLUDER_HEADER: usize = 5;

/// Bins along the longer side of the node casters' union box.
///
/// Coarse on purpose: a bin only has to be small against the spread of the
/// casters for its list to be a handful rather than all of them. At 1,025
/// nodes on a 1536-point pane that is about 48 points a bin.
const OCCLUDER_BINS: f32 = 32.0;

/// Which node casters can cover each part of the pane, for `node_visibility`
/// (common.wgsl): a coarse grid over the union of their boxes, each bin
/// listing, in painter order, every caster whose box reaches it.
///
/// The fragment reads its own bin's list and keeps the casters LATER than its
/// receiver, where it used to follow a linked list through every later node
/// caster on the pane — about half of them per fragment, 500 steps at 1,025
/// nodes (#1098). The picture is the same bit for bit: the shader still
/// applies its exact box test, the list only has to hold every caster whose
/// box contains the point, and it is walked in the same ascending order, so
/// the product multiplies the same factors in the same sequence.
///
/// Holding every containing caster is what the bin ranges below are built
/// for. Each box is widened by a hundredth of a bin before it is binned, so a
/// GPU rounding the point's bin differently from this CPU still lands inside
/// the range, and a one-bin margin surrounds the union so no point in any box
/// falls off the grid's near edge. A point outside the grid is covered by no
/// caster, and the shader answers 1 there.
///
/// `node_cells` are the node casters' indices; only those whose shadow lands
/// (`shade[0] > 0`) occlude, as they did in the list this replaces. Layout,
/// in `u32` words: [`OCCLUDER_HEADER`], then `cols * rows + 1` absolute
/// offsets, then the lists. A frame with no occluder is the header alone, a
/// zero-column grid.
///
/// Written into `out`, last frame's grid, emptied first.
pub(crate) fn node_occluders(casters: &[ShadowCaster], node_cells: &[u32], out: &mut Vec<u32>) {
    let mut occluders: Vec<u32> = node_cells
        .iter()
        .copied()
        .filter(|&i| casters.get(i as usize).is_some_and(|c| c.shade[0] > 0.0))
        .collect();
    occluders.sort_unstable();
    occluders.dedup();
    let rect = |i: u32| {
        let r = casters[i as usize].rect;
        ([r[0], r[1]], [r[0] + r[2], r[1] + r[3]])
    };
    // A box that is not finite contains nothing the shader's test accepts,
    // except an infinite one, which `pack` never makes.
    occluders.retain(|&i| {
        let (min, max) = rect(i);
        min.iter().chain(&max).all(|v| v.is_finite())
    });
    out.clear();
    out.resize(OCCLUDER_HEADER, 0);
    let Some((min, max)) = occluders.iter().map(|&i| rect(i)).reduce(|(a0, a1), (b0, b1)| {
        ([a0[0].min(b0[0]), a0[1].min(b0[1])], [a1[0].max(b1[0]), a1[1].max(b1[1])])
    }) else {
        return;
    };
    let extent = [max[0] - min[0], max[1] - min[1]];
    let per_point = OCCLUDER_BINS / extent[0].max(extent[1]).max(1.0);
    let bin_points = 1.0 / per_point;
    let origin = [min[0] - bin_points, min[1] - bin_points];
    // One margin bin before the union, one after, and one for the far edge.
    let dims = extent.map(|e| (e * per_point).floor() as usize + 3);
    let pad = 0.01 * bin_points;
    let span = |i: u32| {
        let (lo, hi) = rect(i);
        let bin = |v: f32, axis: usize| {
            (((v - origin[axis]) * per_point).floor().max(0.0) as usize).min(dims[axis] - 1)
        };
        [0, 1].map(|axis| bin(lo[axis] - pad, axis)..=bin(hi[axis] + pad, axis))
    };
    // A counting sort: sizes, then offsets, then each list filled in the
    // occluders' own ascending order, which is the order the shader needs.
    let bins = dims[0] * dims[1];
    let mut count = vec![0u32; bins + 1];
    for &i in &occluders {
        let [xs, ys] = span(i);
        for y in ys {
            for x in xs.clone() {
                count[y * dims[0] + x + 1] += 1;
            }
        }
    }
    // `count[k + 1]` holds bin k's size, so an inclusive running sum turns
    // `count[k]` into where bin k's list starts, and `count[bins]` the end.
    let mut offset = (OCCLUDER_HEADER + bins + 1) as u32;
    for c in &mut count {
        offset += *c;
        *c = offset;
    }
    out[0] = dims[0] as u32;
    out[1] = dims[1] as u32;
    out[2] = origin[0].to_bits();
    out[3] = origin[1].to_bits();
    out[4] = per_point.to_bits();
    out.extend_from_slice(&count);
    out.resize(offset as usize, 0);
    let mut cursor: Vec<u32> = count[..bins].to_vec();
    for &i in &occluders {
        let [xs, ys] = span(i);
        for y in ys {
            for x in xs.clone() {
                let at = &mut cursor[y * dims[0] + x];
                out[*at as usize] = i;
                *at += 1;
            }
        }
    }
}

/// The two pipelines that sweep blur cells.
///
/// One module and one constructor because they share a cell quad off the same
/// box stream and differ only in which axis they read. No blend anywhere here:
/// each writes its cell's texels outright.
#[derive(Clone)]
pub(crate) struct CellPipelines {
    pub(crate) blur_x: wgpu::RenderPipeline,
    pub(crate) blur_y: wgpu::RenderPipeline,
}

pub(crate) fn create_cell_pipelines(
    device: &wgpu::Device,
    atlas: &wgpu::BindGroupLayout,
) -> CellPipelines {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shadow_shader"),
        source: wgpu::ShaderSource::Wgsl(SHADOW_SRC.into()),
    });
    let layout = |label, groups: &[Option<&wgpu::BindGroupLayout>]| {
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: groups,
            ..Default::default()
        })
    };
    let blur_layout = layout("lattice_shadow_blur_pipeline_layout", &[Some(atlas)]);
    let pipeline =
        |entry: &str, pipeline_layout: &wgpu::PipelineLayout, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    // Share one entry point so Metal can reuse the compiled vertex stage.
                    entry_point: Some("vs_cell"),
                    compilation_options: Default::default(),
                    buffers: &[ShadowBox::LAYOUT],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
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
        };
    CellPipelines {
        blur_x: pipeline("fs_blur_x", &blur_layout, ATLAS_FORMAT),
        blur_y: pipeline("fs_blur_y", &blur_layout, ATLAS_FORMAT),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The occluder grid holds, for every point, every later node caster whose
    /// box holds that point, in painter order — which is what makes
    /// `node_visibility` multiply the same factors in the same order as the
    /// linked list through every later node caster it replaced (#1098).
    ///
    /// The fixture is a label receiver with a disabled shadow, overlapped by
    /// two later nodes and one later node whose shadow does not land, behind an
    /// earlier node, under a marker box covering everything, and beside a row
    /// of later nodes far away. Points run over a half-point lattice and every
    /// box edge, where a bin boundary is most likely to drop one.
    #[test]
    fn the_occluder_grid_lists_every_later_node_box_under_a_point() {
        assert_eq!(
            shader_const(crate::COMMON_SRC, "OCCLUDER_HEADER").trim_end_matches('u'),
            OCCLUDER_HEADER.to_string()
        );
        let caster = |rect: [f32; 4], shade: f32| ShadowCaster {
            rect,
            shade: [shade, 0.0, 0.0, 0.0],
            ..NO_CASTER
        };
        let mut casters = vec![
            caster([-50.0, -50.0, 800.0, 500.0], 1.0), // a marker, not a node
            caster([10.0, 10.0, 30.0, 30.0], 1.0),     // an earlier node
            caster([20.0, 20.0, 40.0, 12.0], 0.0),     // the label receiver
            caster([30.0, 15.0, 25.0, 25.0], 1.0),     // later, overlapping
            caster([50.0, 20.0, 20.0, 20.0], 1.0),     // later, overlapping
            caster([25.0, 25.0, 10.0, 10.0], 0.0),     // later, shadow shut
            caster([44.5, 26.0, 0.0, 0.0], 1.0),       // later, a bare point
        ];
        casters.extend((0..20).map(|i| caster([200.0 + 20.0 * i as f32, 300.0, 15.0, 15.0], 1.0)));
        let node_cells: Vec<u32> = (1..casters.len() as u32).filter(|&i| i != 2).collect();
        let grid = occluders_of(&casters, &node_cells);

        // The shader's own arithmetic, in f32.
        let list = |p: [f32; 2]| -> &[u32] {
            let origin = [f32::from_bits(grid[2]), f32::from_bits(grid[3])];
            let per_point = f32::from_bits(grid[4]);
            let bin = [0, 1].map(|a| ((p[a] - origin[a]) * per_point).floor());
            if bin[0] < 0.0 || bin[1] < 0.0 || bin[0] >= grid[0] as f32 || bin[1] >= grid[1] as f32
            {
                return &[];
            }
            let slot = OCCLUDER_HEADER + bin[1] as usize * grid[0] as usize + bin[0] as usize;
            &grid[grid[slot] as usize..grid[slot + 1] as usize]
        };
        let holds = |i: u32, p: [f32; 2]| {
            let r = casters[i as usize].rect;
            p[0] >= r[0] && p[1] >= r[1] && p[0] <= r[0] + r[2] && p[1] <= r[1] + r[3]
        };
        let mut points: Vec<[f32; 2]> = (-20..1400)
            .flat_map(|x| (-20..760).step_by(7).map(move |y| [x as f32 * 0.5, y as f32 * 0.5]))
            .collect();
        for c in &casters {
            let [x, y, w, h] = c.rect;
            for px in [x, x + w] {
                for py in [y, y + h, y + h * 0.5] {
                    points.push([px, py]);
                    points.push([x + w * 0.5, py]);
                }
            }
        }
        let (mut crowded, mut compared) = (0, 0);
        for &p in &points {
            let listed = list(p);
            assert!(listed.is_sorted(), "a bin out of painter order at {p:?}");
            crowded += usize::from(listed.len() >= 2);
            for who in 0..casters.len() as u32 {
                let want: Vec<u32> = node_cells
                    .iter()
                    .copied()
                    .filter(|&j| j > who && casters[j as usize].shade[0] > 0.0 && holds(j, p))
                    .collect();
                let got: Vec<u32> =
                    listed.iter().copied().filter(|&j| j > who && holds(j, p)).collect();
                assert_eq!(got, want, "receiver {who} at {p:?}");
                compared += want.len();
            }
        }
        assert!(crowded > 100 && compared > 1000, "the fixture overlaps too little: {crowded}");
        // And it is a grid rather than the whole list again: a point on the
        // label under both overlapping nodes lists them and none of the far row.
        let near = list([52.0, 30.0]);
        assert!(near.contains(&3) && near.contains(&4) && !near.contains(&5));
        assert!(near.iter().all(|&j| j < 7), "the far row reached the label: {near:?}");
        assert_eq!(occluders_of(&casters, &[2]), vec![0; OCCLUDER_HEADER], "no occluder");
    }

    /// A shader's `const NAME: T = value;`, as text.
    pub(crate) fn shader_const(src: &str, name: &str) -> String {
        src.lines()
            .find_map(|l| l.trim().strip_prefix(&format!("const {name}: ")))
            .and_then(|l| l.split('=').nth(1))
            .map(|v| v.trim().trim_end_matches(';').to_string())
            .unwrap_or_else(|| panic!("the shader declares {name}"))
    }

    /// A half float, as the atlas is read back.
    pub(crate) fn half(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exp = i32::from((bits >> 10) & 0x1f);
        let mant = f32::from(bits & 0x3ff) / 1024.0;
        sign * match exp {
            0 => mant * 2f32.powi(-14),
            31 => f32::INFINITY,
            e => (1.0 + mant) * 2f32.powi(e - 15),
        }
    }

    /// A caster at the picture's own width, which is what every caster but a
    /// note name is. The tests that ask about the RATIO name it themselves.
    fn caster(x: f32, y: f32, w: f32, h: f32) -> Caster {
        Caster { rect: [x, y, w, h], level: 1.0, sigma_points: 1.0, spread_points: 0.0, stops: 0.0 }
    }

    /// Packing into the buffers a bigger frame left is the packing done fresh,
    /// and so is the occluder grid built from it.
    #[test]
    fn packing_into_last_frames_buffers_is_packing_fresh() {
        // A square field for the first, so its occluder grid is the square
        // one; a wide thin pair for the second, whose grid is a few rows.
        let big: Vec<Caster> = (0..42)
            .map(|i| caster((i % 7) as f32 * 60.0, (i / 7) as f32 * 60.0, 50.0, 50.0))
            .collect();
        let small = [caster(10.0, 10.0, 200.0, 4.0), caster(100.0, 12.0, 50.0, 4.0)];
        let (mut packed, mut scratch, mut grid) =
            (Packed::default(), PackScratch::default(), vec![]);
        pack_into(&big, 2.0, 16384, &mut packed, &mut scratch);
        node_occluders(&packed.casters, &(0..42).collect::<Vec<_>>(), &mut grid);
        let left = (packed.boxes.len(), grid.len());
        pack_into(&small, 2.0, 16384, &mut packed, &mut scratch);
        node_occluders(&packed.casters, &[0, 1], &mut grid);
        assert!(left.0 > packed.boxes.len() && left.1 > grid.len(), "{left:?}");
        assert_eq!(packed, pack(&small, 2.0, 16384));
        assert_eq!(grid, occluders_of(&packed.casters, &[0, 1]));
        assert!(grid.len() > OCCLUDER_HEADER, "the two boxes overlap and occlude");
    }

    #[test]
    fn gaussian_spread_pads_the_source_without_changing_blur() {
        let base = Caster { sigma_points: 5.877344, ..caster(30.0, 30.0, 10.0, 2.0) };
        let expanded = Caster { spread_points: 2.0, ..base };
        let a = pack(&[base], 2.0, 4096);
        let b = pack(&[expanded], 2.0, 4096);
        assert_eq!(a.boxes[0].cell_map, b.boxes[0].cell_map);
        assert_eq!(b.boxes[0].rect[0], a.boxes[0].rect[0] - 2.0);
        assert_eq!(b.boxes[0].rect[2], a.boxes[0].rect[2] + 4.0);
        assert_eq!(b.boxes[0].who[3], 2.0);
        assert!(b.boxes[0].cell[2] > a.boxes[0].cell[2]);
    }

    fn occluders_of(casters: &[ShadowCaster], node_cells: &[u32]) -> Vec<u32> {
        let mut out = Vec::new();
        node_occluders(casters, node_cells, &mut out);
        out
    }

    /// A whole frame at ONE style: every caster at `sigma_px` in the target's
    /// pixels.
    ///
    /// What a claim about the PACKING rather than about the groups is asked
    /// at — the shelving, the resolution ladder, the atlas limit — none of
    /// which is a question about which group a caster belongs to. σ in target
    /// pixels because that is the unit those claims are stated in; the caster
    /// carries points and the packer multiplies. The claims that ARE about
    /// groups build their casters by hand.
    fn pack_at(casters: &[Caster], sigma_px: f32, px_per_point: f32, max_side: u32) -> Packed {
        let casters: Vec<Caster> = casters
            .iter()
            .map(|c| Caster { sigma_points: sigma_px / px_per_point, ..*c })
            .collect();
        pack(&casters, px_per_point, max_side)
    }

    /// The shader's caster is as many rows as the one the CPU writes.
    ///
    /// Two files with no linkage: common.wgsl declares the struct the scene
    /// draws read a caster's entry out of, and [`ShadowCaster`] is what `pack`
    /// writes into that buffer. A shader one row short reads every field after
    /// the missing one out of the next caster's entry — a shadow of the right
    /// shape in the wrong place, on no diagnostic at all.
    ///
    /// The rows and not the field names, which is what a wgsl parser would buy
    /// and is not what goes wrong: every row here is a `vec4<f32>`, so the
    /// count is the size and a row added on one side alone is what the count
    /// catches.
    #[test]
    fn the_shaders_caster_is_the_packers() {
        let src = crate::with_common("");
        let body = src
            .split_once("struct ShadowCaster {")
            .and_then(|(_, rest)| rest.split_once('}'))
            .expect("common.wgsl declares ShadowCaster")
            .0;
        let rows = body.matches("vec4<f32>,").count();
        assert_eq!(
            rows,
            std::mem::size_of::<ShadowCaster>() / 16,
            "common.wgsl's ShadowCaster is {rows} rows where the packer writes {}",
            std::mem::size_of::<ShadowCaster>() / 16,
        );
    }

    /// The shader's kernel and the packer's padding reach the same number of
    /// σ, and the loop bound is what the packer's cap implies — the three
    /// constants that have to agree across two files with no linkage.
    #[test]
    fn the_blurs_reach_and_loop_bound_are_the_packers_own() {
        let reach: f32 = shader_const(SHADOW_SRC, "REACH").parse().expect("a number");
        assert_eq!(reach, REACH_SIGMAS, "the kernel reaches a different distance than the padding");
        let radius: i32 = shader_const(SHADOW_SRC, "MAX_RADIUS").parse().expect("a number");
        assert_eq!(
            radius,
            (REACH_SIGMAS * SIGMA_CELL_MAX).ceil() as i32,
            "the loop bound is not the widest kernel the packer can ask for",
        );
    }

    /// The pedestal the kernel is lowered by is the Gaussian's own value at
    /// the distance the kernel reaches, which is the whole of why the blur
    /// lands on zero rather than stepping off a cliff there.
    ///
    /// Two constants in one file and no linkage between them: a pedestal short
    /// of this leaves part of the step, and one past it clips the kernel inside
    /// the padding the packer laid down and narrows the Shadow by more than the
    /// 4.3% the bar is documented to lose.
    #[test]
    fn the_kernels_pedestal_is_its_own_weight_at_the_reach() {
        let reach: f32 = shader_const(SHADOW_SRC, "REACH").parse().expect("a number");
        let pedestal: f32 = shader_const(SHADOW_SRC, "PEDESTAL").parse().expect("a number");
        let want = (-0.5 * reach * reach).exp();
        assert!(
            (pedestal - want).abs() < 5e-7,
            "the kernel is lowered by {pedestal} where its own weight at {reach}σ is {want}",
        );
    }

    /// σ never exceeds the cap in any cell, at any width of the Shadow: past
    /// the cap the cell shrinks rather than the kernel widening.
    #[test]
    fn a_cells_sigma_is_at_most_three_texels_at_every_shadow_width() {
        for sigma_px in [0.05f32, 0.5, 1.0, 2.9, 3.0, 3.1, 10.0, 100.0, 5000.0] {
            let packed = pack_at(&[caster(10.0, 10.0, 40.0, 12.0)], sigma_px, 2.0, 16384);
            let b = packed.boxes[0];
            assert!(
                b.cell_map[1] <= SIGMA_CELL_MAX + 1e-4,
                "σ {sigma_px} px packed a cell at σ {} texels",
                b.cell_map[1]
            );
            // Under the cap the cell is drawn at the target's own pixels;
            // over it, exactly at the cap.
            if sigma_px <= SIGMA_CELL_MAX {
                assert!(
                    (b.cell_map[0] - 2.0).abs() < 1e-5 && (b.cell_map[1] - sigma_px).abs() < 1e-4
                );
            } else {
                assert!((b.cell_map[1] - SIGMA_CELL_MAX).abs() < 1e-4);
            }
            // And the padding holds the whole kernel plus the sampling texel.
            let pad_texels = (REACH_SIGMAS * b.cell_map[1]).ceil() + 1.0;
            let grown = (b.rect[2] - 40.0) * 0.5 * b.cell_map[0];
            assert!((grown - pad_texels).abs() < 1e-3, "padded {grown} texels for {pad_texels}");
        }
    }

    /// A frame with no caster packs nothing at all; a caster whose group is
    /// SHUT keeps its entry and takes no cell.
    ///
    /// The entry has to stay, where the cell may not: a caster's index is what
    /// every scene draw reaches it by, and dropping a shut group's entries
    /// would slide every caster after them onto a neighbour's cell. What says
    /// the group is shut is the zeroed entry — level 0, no cell — which every
    /// reader answers 1 to with nothing sampled.
    ///
    /// The NaN is a second claim and a separate one: `shape` runs for every
    /// non-direct caster, shut or not, so a corrupt σ has to come out of it as
    /// a number the vertex stage can read rather than as a NaN geometry term
    /// on a box that is drawn either way.
    #[test]
    fn a_frame_with_no_caster_packs_no_cell() {
        assert_eq!(pack_at(&[], 4.0, 2.0, 16384), Packed::default());
        for sigma in [0.0, f32::NAN] {
            let packed = pack_at(&[caster(0.0, 0.0, 10.0, 10.0)], sigma, 2.0, 16384);
            assert_eq!(packed.casters, vec![NO_CASTER], "a shut caster lost or kept an entry");
            assert!(
                packed.boxes.iter().all(|b| b.cell == [0.0; 4] && b.cell_map[2] == 0.0),
                "a caster at σ {sigma} packed a cell",
            );
            assert!(
                packed.boxes.iter().all(|b| b
                    .cell_map
                    .iter()
                    .chain(b.who.iter())
                    .all(|t| t.is_finite())),
                "a caster at σ {sigma} packed a box carrying a NaN: {:?}",
                packed.boxes,
            );
        }
    }

    /// Every cell lies inside the atlas and no two overlap, over a frame of
    /// names of mixed sizes — including one wider than the square the total
    /// area suggests, which is what forces the width up.
    #[test]
    fn packed_cells_are_disjoint_and_inside_the_atlas() {
        let casters: Vec<Caster> = (0..40)
            .map(|i| {
                let f = i as f32;
                caster(
                    f * 7.0,
                    (f * 13.0) % 200.0,
                    20.0 + (f * 31.0) % 90.0,
                    8.0 + (f * 5.0) % 14.0,
                )
            })
            .chain([caster(0.0, 0.0, 700.0, 10.0)])
            .collect();
        let packed = pack_at(&casters, 6.0, 2.0, 16384);
        assert_eq!(packed.boxes.len(), casters.len());
        let rects: Vec<[u32; 4]> = packed.boxes.iter().map(|b| b.cell.map(|v| v as u32)).collect();
        for (i, r) in rects.iter().enumerate() {
            assert!(r[2] > 0 && r[3] > 0, "cell {i} is empty");
            assert!(
                r[0] + r[2] <= packed.size[0] && r[1] + r[3] <= packed.size[1],
                "cell {i} {r:?} overflows {:?}",
                packed.size
            );
            for (j, s) in rects.iter().enumerate().skip(i + 1) {
                let apart = r[0] + r[2] <= s[0]
                    || s[0] + s[2] <= r[0]
                    || r[1] + r[3] <= s[1]
                    || s[1] + s[3] <= r[1];
                assert!(apart, "cells {i} {r:?} and {j} {s:?} overlap");
            }
        }
        // The same frame packs the same way: a layout is a function of the
        // frame and nothing else.
        assert_eq!(pack_at(&casters, 6.0, 2.0, 16384), packed);
    }

    /// A cell past the texture limit is no cell, with its level zeroed so its
    /// box draws nothing, and the rest of the frame keeps its shadows.
    #[test]
    fn a_cell_the_atlas_cannot_hold_casts_nothing() {
        let casters: Vec<Caster> = (0..8).map(|_| caster(0.0, 0.0, 100.0, 100.0)).collect();
        let packed = pack_at(&casters, 2.0, 1.0, 256);
        assert_eq!(packed.size, [256, 256]);
        let cast: Vec<bool> = packed.boxes.iter().map(|b| b.cell_map[2] > 0.0).collect();
        assert!(cast.iter().any(|&c| c), "nothing fit an atlas that holds four cells");
        assert!(!cast.iter().all(|&c| c), "eight 100-pt cells fit a 256-texel atlas");
        for b in packed.boxes.iter().filter(|b| b.cell_map[2] == 0.0) {
            assert_eq!(b.cell, [0.0; 4]);
        }
    }

    /// A caster that darkens nothing takes no cell, and the frame packs as if
    /// it had never been handed over.
    ///
    /// Nodes clipped off the pane and nodes projected behind the eye arrive at
    /// level 0 in numbers (`node_caster`), and `shadow_through` hands the frame
    /// back whole below that level. A cell for one is atlas area every blur
    /// pass sweeps, and a rasterization of the whole node shader, for texels
    /// nothing ever samples.
    #[test]
    fn a_caster_that_darkens_nothing_takes_no_cell_and_no_room() {
        let live = caster(0.0, 0.0, 100.0, 100.0);
        let dead = Caster { level: 0.0, ..live };
        let without = pack_at(&[live, live], 2.0, 1.0, 4096);
        let with_dead = pack_at(&[live, dead, live], 2.0, 1.0, 4096);
        assert_eq!(with_dead.boxes[1].cell, [0.0; 4], "the dead caster took a cell");
        assert_eq!(with_dead.boxes[1].cell_map[2], 0.0, "the dead caster kept a level");
        assert_eq!(with_dead.size, without.size, "the dead caster sized the atlas");
        assert_eq!(
            [with_dead.boxes[0].cell, with_dead.boxes[2].cell],
            [without.boxes[0].cell, without.boxes[1].cell],
            "the dead caster moved a live cell",
        );
    }

    /// σ is half the Shadow's width in the pane's points, and shut for a width
    /// or a node radius that is not a positive number.
    #[test]
    fn sigma_is_half_the_shadow_in_pane_points() {
        assert!((sigma_points(0.2, 30.0) - 3.0).abs() < 1e-5);
        assert_eq!(sigma_points(0.0, 30.0), 0.0);
        assert_eq!(sigma_points(0.2, 0.0), 0.0);
        assert_eq!(sigma_points(f32::NAN, 30.0), 0.0);
        assert_eq!(sigma_points(-1.0, 30.0), 0.0);
    }

    /// Reach is paint reach, not merely the kernel's mathematical support. Either shut bar leaves no pixels for the caller to cull or
    /// enlarge geometry around.
    #[test]
    fn a_spectral_style_that_cannot_cast_has_no_reach() {
        let style =
            |width, depth| harmonigraph_scene::ShadowStyle { width, depth, ..Default::default() };
        assert!(spectral_shadow_reach(style(1.0, 1.0), 1.0) > 0.0, "the live fixture is shut");
        assert_eq!(spectral_shadow_reach(style(0.0, 1.0), 1.0), 0.0, "width endpoint");
        assert_eq!(spectral_shadow_reach(style(1.0, 0.0), 1.0), 0.0, "depth endpoint");
        assert_eq!(spectral_shadow_reach(style(f32::NAN, 1.0), 1.0), 0.0, "repaired width");
        assert_eq!(spectral_shadow_reach(style(1.0, f32::NAN), 1.0), 0.0, "repaired depth");
    }

    /// The blur of one cell's ink stays inside that cell and keeps its mass:
    /// a block in one cell leaves the cell packed beside it at exactly zero,
    /// and the block's own cell holds as much after the blur as before.
    ///
    /// The half-plane reading is the third claim and the one that pins the
    /// normalisation: at the block's own edge the blur reads half, which is
    /// only true if a tap that falls outside the cell counts as zero rather
    /// than being dropped from the kernel's sum.
    #[test]
    fn a_cells_blur_stays_inside_its_own_cell_and_keeps_its_mass() {
        const SIGMA: f32 = 3.0;
        let Some((device, queue)) = crate::gpu_harness::headless_device() else {
            return;
        };
        // Two casters side by side, both far wider than the blur reaches, so
        // the pair packs on one shelf with the second's cell touching the
        // first's.
        let packed = pack_at(
            &[caster(0.0, 0.0, 80.0, 80.0), caster(0.0, 0.0, 80.0, 80.0)],
            SIGMA,
            1.0,
            4096,
        );
        assert_eq!(packed.boxes.len(), 2);
        let [a, b] = [packed.boxes[0], packed.boxes[1]];
        assert_eq!(a.cell[1], b.cell[1], "the pair must share a shelf to be neighbours");
        assert_eq!(a.cell[0] + a.cell[2], b.cell[0], "the pair must touch");
        // A width the harness's readback can take: it copies 4-byte rows.
        let size = [packed.size[0].max(64), packed.size[1]];
        let layout = read_layout(&device);
        let sampler = device.create_sampler(&Default::default());
        let target = ShadowTarget::new(&device, &layout, &sampler, size);
        let pipelines = create_cell_pipelines(&device, &layout);

        // The block: cell A's ink region — everything inside its padding —
        // filled solid, as a caster wider than the blur is.
        let pad = ((REACH_SIGMAS * a.cell_map[1]).ceil() + 1.0) as u32;
        let (ax, ay, aw, ah) =
            (a.cell[0] as u32, a.cell[1] as u32, a.cell[2] as u32, a.cell[3] as u32);
        let inked = |x: u32, y: u32| {
            x >= ax + pad && x < ax + aw - pad && y >= ay + pad && y < ay + ah - pad
        };
        let mut ink = vec![0u8; (size[0] * size[1] * 4) as usize];
        let mut mass_before = 0.0f64;
        for y in 0..size[1] {
            for x in 0..size[0] {
                if inked(x, y) {
                    let at = ((y * size[0] + x) * 4) as usize;
                    ink[at..at + 2].copy_from_slice(&0x3c00u16.to_le_bytes());
                    mass_before += 1.0;
                }
            }
        }
        assert!(mass_before > 100.0, "the fixture's block covers {mass_before} texels");
        queue.write_texture(
            target.texture().as_image_copy(),
            &ink,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] * 4),
                rows_per_image: Some(size[1]),
            },
            wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        );
        let boxes = crate::gpu_memory::buffer(
            &device,
            &wgpu::BufferDescriptor {
                label: Some("test_boxes"),
                size: (2 * std::mem::size_of::<ShadowBox>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
        );
        queue.write_buffer(&boxes, 0, bytemuck::cast_slice(&packed.boxes));
        let mut encoder = device.create_command_encoder(&Default::default());
        target.blur(&mut encoder, (&pipelines.blur_x, &pipelines.blur_y), &boxes, 2);
        queue.submit([encoder.finish()]);
        let bytes = crate::gpu_harness::readback(&device, &queue, target.texture(), size);
        let at = |x: u32, y: u32| -> f32 {
            let i = ((y * size[0] + x) * 4) as usize;
            half(u16::from_le_bytes([bytes[i], bytes[i + 1]]))
        };

        // Cell B: nothing at all, though it touches A and A's ink stands
        // within the kernel's reach of its edge.
        let (bx, by, bw, bh) =
            (b.cell[0] as u32, b.cell[1] as u32, b.cell[2] as u32, b.cell[3] as u32);
        let mut leaked = 0;
        for y in by..by + bh {
            for x in bx..bx + bw {
                if at(x, y) != 0.0 {
                    leaked += 1;
                }
            }
        }
        assert_eq!(leaked, 0, "cell A's blur reached {leaked} texels of cell B");
        // Cell A: the mass is where it was.
        let mut mass_after = 0.0f64;
        for y in ay..ay + ah {
            for x in ax..ax + aw {
                mass_after += f64::from(at(x, y));
            }
        }
        assert!(
            (mass_after - mass_before).abs() < 0.01 * mass_before,
            "the blur left {mass_after} of {mass_before}",
        );
        // The half-plane: at the block's own edge, half.
        let edge = at(ax + aw - pad, ay + ah / 2);
        let inside = at(ax + aw - pad - 1, ay + ah / 2);
        assert!(
            (0.5 * (edge + inside) - 0.5).abs() < 0.03,
            "the blur reads {edge} and {inside} either side of the block's edge, not half",
        );
    }

    /// A frame whose GROUPS disagree packs each caster at its own width, on
    /// one shelf.
    ///
    /// The claim the per-group design rests on, and the one a packer that kept
    /// σ per FRAME would fail while every uniform-frame reading above went on
    /// passing. The σ apart by a factor of four so the two resolutions cannot
    /// agree by accident, and both past `SIGMA_CELL_MAX` so each cell is really
    /// sized by its own σ rather than pinned at the target's resolution.
    #[test]
    fn a_mixed_frame_packs_each_caster_at_its_own_width() {
        let at = |sigma: f32| Caster { sigma_points: sigma, ..caster(0.0, 0.0, 40.0, 12.0) };
        let (near, far) = (10.0, 40.0);
        let packed = pack(&[at(near), at(far)], 1.0, 16384);
        let [n, f] = [packed.boxes[0], packed.boxes[1]];
        for (b, sigma) in [(n, near), (f, far)] {
            assert!(
                (b.cell_map[0] - SIGMA_CELL_MAX / sigma).abs() < 1e-5,
                "the cell at σ {sigma} packed {} texels a point rather than following its own σ",
                b.cell_map[0],
            );
            assert!(
                (b.cell_map[1] - SIGMA_CELL_MAX).abs() < 1e-4,
                "the cell at σ {sigma} is off the cap"
            );
        }
        // One shelf, disjoint: a mixed frame is one packing rather than two.
        assert!(n.cell[2] > 0.0 && f.cell[2] > 0.0, "a caster in the mixed frame got no cell");
        assert!(
            n.cell[0] + n.cell[2] <= f.cell[0] || f.cell[0] + f.cell[2] <= n.cell[0],
            "the two cells overlap: {:?} and {:?}",
            n.cell,
            f.cell,
        );
    }
}
