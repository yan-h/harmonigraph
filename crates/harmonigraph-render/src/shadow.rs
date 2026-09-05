//! The shadow atlas: every caster's ink blurred into a cell of its own, which
//! is what that caster multiplies the frame by in its own draw
//! (`fs_shadow_box` in text.wgsl).
//!
//! ONE cell per caster, at a scale that keeps the blur's cost flat: the cell is
//! the caster's box grown by the kernel's reach, drawn at `min(1, 3 / σ)` of
//! the target's pixels, so σ is at most `SIGMA_CELL_MAX` texels in every cell
//! and the kernel at most nineteen taps whatever the Shadow bar says. The atlas
//! is about the names' own area at the fresh Shadow under a Gaussian, and
//! shrinks as the bar widens. A DISTANCE cell parts from that at the quality
//! floor, which is where its cost stops falling.
//!
//! What lives here is the arithmetic that runs without a GPU — the packer and
//! σ — beside the textures and the blur passes over them (shadow.wgsl). The
//! rasterizer that fills a cell is the caster's own: a name's is text.wgsl's
//! `fs_glyph_ink`, drawn at the cell's transform by `vs_glyph_cell`.

use crate::wgpu;

const SHADOW_SRC: &str = include_str!("shaders/shadow.wgsl");

/// What the atlas is kept in: one half-float coverage per texel.
///
/// Half floats rather than a byte because the blur's tail is MULTIPLIED into
/// the frame: a tail quantized to 1/255 steps across a wide soft shadow, where
/// the light under it has no steps of its own to hide them in.
pub(crate) const ATLAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// The most texels a cell's σ may be. What bounds the kernel, at
/// `2 * ceil(REACH_SIGMAS * this) + 1` taps, and so the cost of a shadow at
/// any width: past this a cell is drawn SMALLER rather than blurred wider.
/// `MAX_RADIUS` in shadow.wgsl is the loop bound this implies.
pub(crate) const SIGMA_CELL_MAX: f32 = 3.0;

/// The least resolution an atlas-backed distance field keeps, in texels per
/// pane point.
///
/// Eight tenths keeps both a projected node contour and a roughly 30-point
/// monospace stem represented finely enough that bilinear reconstruction does
/// not expose the cell grid. The unit is pane points so the editor and offline
/// renderer keep the same source samples at every target scale. A direct field
/// has no cell and therefore pays none of this floor.
pub(crate) const DISTANCE_TEXELS_PER_POINT: f32 = 0.8;

/// Screen-point reference behind the spectral groups' dimensionless Shadow
/// width. At the top of the bar a spectral edge is four points wide; changing
/// pane size or pitch zoom does not change it.
pub const SPECTRAL_WIDTH_POINTS: f32 = 4.0;

/// A spectral style's σ in screen points.
pub(crate) fn spectral_sigma_points(style: harmonigraph_scene::ShadowStyle) -> f32 {
    sigma_points(style.width, SPECTRAL_WIDTH_POINTS)
}

/// How far a spectral style's selected renderer can paint past its caster.
pub fn spectral_shadow_reach(style: harmonigraph_scene::ShadowStyle) -> f32 {
    let style = style.clamped();
    if style.casts() {
        spectral_sigma_points(style) * style.kernel.reach_sigmas()
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
/// what the bar says it is, and the distance renderer's decay is windowed to
/// nothing at `SHADOW_STOP` of the same widths.
///
/// PER CASTER and not per frame: a caster carries the σ its style group asked
/// for ([`Caster::sigma_points`]), so a frame whose groups disagree packs each
/// at its own width with no second conversion between them.
///
/// In points and not in the target's pixels — points being what a distance cell
/// holds and what the sampler measures against. The target's own scale enters
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
    /// Which renderer turns this caster's ink into its shadow — its group's.
    ///
    /// Per caster rather than per frame, which is what lets a frame whose
    /// groups disagree schedule both paths: the fill branches on it, the blur
    /// chain sweeps only the cells that hold coverage, and the scene draw reads
    /// it back off [`ShadowCaster::shade`].
    pub kernel: harmonigraph_scene::ShadowKernel,
    /// Whether this caster's distance is evaluated by its own scene draw
    /// instead of read out of a cell.
    ///
    /// The marker already has its exact field in `plus_paint`, so rasterizing
    /// the same field into a cell would spend an atlas allocation and a second
    /// scene draw for no new information. A Gaussian still needs the shared
    /// cell that holds its convolution.
    pub direct_distance: bool,
}

/// The caster a name's glyphs make: the box round every glyph's rect, the
/// strength the name's rim colour carries — the one number a lattice name's
/// `rim` holds (`LABEL_SHADOW` in harmonigraph_ui), so a name easing in as the
/// marker under it eases out grows its shadow on the clock its ink arrives on —
/// and the style its group is dialled to (`ShadowSettings::lattice_text`).
///
/// A run with no ink in it — every rect empty — is a caster of nothing, with
/// its level zeroed rather than a box of infinities for the packer to size.
pub(crate) fn caster_of(
    glyphs: &[crate::GlyphInstance],
    sigma_points: f32,
    kernel: harmonigraph_scene::ShadowKernel,
) -> Caster {
    let (mut min, mut max) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for g in glyphs {
        for axis in 0..2 {
            min[axis] = min[axis].min(g.rect[axis]);
            max[axis] = max[axis].max(g.rect[axis] + g.rect[axis + 2]);
        }
    }
    let empty = Caster { rect: [0.0; 4], level: 0.0, sigma_points, kernel, direct_distance: false };
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
    /// [`Packed::casters`]; y: what this cell HOLDS, 0 blurred ink and 1 a
    /// distance ([`DISTANCE_KIND`]); z: how far past the caster's ink this cell
    /// reaches, in the pane's points — the pad the rect was grown by, which is
    /// where the standoff's curve is windowed to nothing and so the value a
    /// texel past its encoded reach holds; w: unused.
    ///
    /// x is read by the node's SCENE draw, which needs the caster's whole entry
    /// and so reaches the array rather than the box; y and z by the passes that
    /// sweep the CELLS, which take one at a time and have to know which chain
    /// this one belongs to. Carried on the box rather than in a buffer of its
    /// own because the node's two draws — the one that fills a cell and the one
    /// that reads the atlas — bind the same stream, and one row is cheaper than
    /// a second binding.
    pub who: [f32; 4],
}

/// What `ShadowBox::who`'s y and `ShadowCaster::shade`'s y hold for a cell that
/// is a DISTANCE rather than blurred ink — `DISTANCE_KIND` in shadow.wgsl
/// and common.wgsl, pinned by
/// `the_shaders_distance_kind_and_window_are_the_packers`.
///
/// A float and not a flag because it rides in a vertex attribute and a storage
/// array that are floats already; compared with a `> 0.5` wherever it is read,
/// so nothing turns on the exact bits surviving an interpolator.
pub(crate) const DISTANCE_KIND: f32 = 1.0;

impl ShadowBox {
    pub(crate) const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ShadowBox>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4
        ],
    };

    /// The same rows at the locations after a glyph's eight, for the draw that
    /// rasterizes a glyph into its cell alongside `GlyphInstance::LAYOUT`.
    pub(crate) const BESIDE_GLYPHS: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ShadowBox>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            8 => Float32x4, 9 => Float32x4, 10 => Float32x4, 11 => Float32x4
        ],
    };

    /// The same rows again, at the three locations a node's own instance rows
    /// leave free (`Instance` in lattice.wgsl, `GpuInstance::LAYOUT`) — the
    /// second instance-step buffer the node draw and the cell draw both bind.
    ///
    /// Scattered rather than consecutive because a vertex attribute's location
    /// has to be under sixteen and a node's rows already reach fifteen; which
    /// three are free is what picks them.
    pub(crate) const BESIDE_NODES: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ShadowBox>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            5 => Float32x4, 9 => Float32x4, 14 => Float32x4, 13 => Float32x4
        ],
    };

    /// After a roll instance's ten attributes, for rasterizing its box SDF
    /// into a Gaussian cell.
    pub(crate) const BESIDE_ROLL: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ShadowBox>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            10 => Float32x4, 11 => Float32x4, 12 => Float32x4, 13 => Float32x4
        ],
    };

    /// After a spiral dot's centre, radius and color, for rasterizing its
    /// circle field into a Gaussian cell.
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
    /// caster casts nothing, and where its distance is evaluated directly by
    /// its own scene draw.
    pub cell: [f32; 4],
    /// The map from a point of the pane to a texel of that cell: x/y the
    /// origin, z the scale, so a texel is `xy + points * z`. w unused.
    ///
    /// Pre-composed on the CPU rather than sent as (cell origin, box origin,
    /// scale) for the shader to combine: a fragment would otherwise repeat the
    /// subtraction for a number that never changes within a frame.
    pub map: [f32; 4],
    /// What this caster SPENDS, none of it a coordinate.
    ///
    /// x: how much of its shadow lands, 0..=1 — zero where the atlas could not
    /// hold its cell, which is a caster that darkens nothing.
    ///
    /// y: what its cell HOLDS, 0 blurred ink and [`DISTANCE_KIND`] a distance.
    /// Per caster rather than in a uniform, because the two shader modules that
    /// read a caster declare this struct once between them (common.wgsl) and
    /// their uniforms separately — so a caster's own properties riding here are
    /// written down once where a copy in each `Locals` would be two.
    ///
    /// z: its σ in the pane's POINTS, which is what a distance read out of a
    /// cell is measured against, one Shadow width being 2σ. In points because
    /// that is what an exact distance cell holds: the sampler divides one by
    /// the other and the target's own pixel scale never enters, so a Render
    /// scale moves neither. Per caster because [`Caster::sigma_points`] is.
    ///
    /// w: unused.
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
/// σ and their kernels — the target scale and the device limit, and nothing a
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
/// kinds and widths differ, packed in one pass.
pub(crate) fn pack(casters: &[Caster], px_per_point: f32, max_side: u32) -> Packed {
    // A finite positive number, which a NaN or an infinity out of a corrupt
    // blob is not: either is no shadow rather than a kernel of nothing.
    let positive = |x: f32| x.is_finite() && x > 0.0;
    if casters.is_empty() || !positive(px_per_point) {
        return Packed::default();
    }
    let is_distance = |c: &Caster| c.kernel.is_distance();
    // One caster's σ in the target's pixels, which is where its cell is drawn.
    //
    // Floored at zero because `shape` runs for every non-direct caster,
    // including the ones `casts` goes on to reject: a NaN σ out of a corrupt
    // blob would otherwise reach `min`, which returns its OTHER operand for a
    // NaN, and come out as a NaN pad on a box the vertex stage reads.
    let sigma_of = |c: &Caster| (c.sigma_points * px_per_point).max(0.0);
    // A cell, sized off ITS OWN σ: drawn at `min(1, SIGMA_CELL_MAX / σ)` of the
    // target's pixels so σ is at most `SIGMA_CELL_MAX` texels whatever the
    // caster asked for, and padded by the kernel's reach in those same texels,
    // plus one so the scene pass's bilinear tap at the box's own edge still
    // lands inside the cell. That cap is what holds the blur's cost flat across
    // the whole bar: the chain reads σ off the cell and clamps its taps to that
    // cell's rect, so a wider Shadow is a smaller cell rather than a wider
    // kernel.
    //
    // A DISTANCE cell parts from that in pad, stored σ and a quality floor. Its
    // pad is the standoff's own stop rather than the blur's reach, that being
    // where the curve is windowed to zero (`ShadowKernel::reach_sigmas`, which
    // is the one place the two are written down). The floor keeps both fixed
    // glyph fields and analytic node fields from being reduced to visible
    // rectangles. σ in the CELL is zero because the distance cell bypasses the
    // blur chain entirely.
    let shape = |c: &Caster| {
        let sigma = sigma_of(c);
        let fit = (SIGMA_CELL_MAX / sigma).min(1.0);
        let floor = if is_distance(c) { DISTANCE_TEXELS_PER_POINT / px_per_point } else { 0.0 };
        // More samples than the target has pixels buy no smoother contour and
        // turn the floor into supersampling at the small end.
        let scale = fit.max(floor).min(1.0);
        let k = scale * px_per_point;
        // σ in the cell's own texels, which is what the PADDING is in whatever
        // the kind — the two renderers reach different multiples of it and
        // `ShadowKernel::reach_sigmas` is the one place that is written down.
        let texels = sigma * scale;
        let pad = ((c.kernel.reach_sigmas() * texels).ceil() + 1.0) / k;
        // The BLUR chain's σ, which a distance cell has none of because it
        // bypasses that chain.
        (scale, k, if is_distance(c) { 0.0 } else { texels }, pad)
    };
    // What a caster comes to where it is spent. A level at zero, a NaN out of a
    // corrupt blob, or a group with either of its bars at the bottom — the
    // width arriving as a σ of nothing — all darken nothing.
    let casts = |c: &Caster| c.level.clamp(0.0, 1.0) > 0.0 && positive(c.sigma_points);
    // Whether this caster's own scene draw evaluates the field, which is a
    // caster with no cell to pack.
    let direct = |c: &Caster| c.direct_distance && is_distance(c);

    // One entry per caster, in the order they arrived — the order every index
    // below is in. A direct caster keeps an empty entry to preserve those
    // indices, but asks for no shelf space.
    let mut rects: Vec<[f32; 4]> = Vec::with_capacity(casters.len());
    let mut sizes: Vec<[u32; 2]> = Vec::with_capacity(casters.len());
    for caster in casters {
        // A caster that darkens nothing takes NO cell: a reader hands the
        // frame back whole at level 0, so the cell it would fill is one
        // nothing ever samples. Nodes clipped off the pane and nodes
        // projected behind the eye arrive here at level 0 in numbers
        // (`node_caster`), and a cell each would widen the atlas every blur
        // pass sweeps and be rasterized by the whole node shader for no
        // picture at all.
        if !casts(caster) || direct(caster) {
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
    let mut placed = Vec::with_capacity(sizes.len());
    for &[w, h] in &sizes {
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
    // at the box. A direct caster is not missing a cell: its scene draw owns
    // the field and needs none.
    let fits = |i: usize| {
        let [w, h] = sizes[i];
        let [x, y] = placed[i];
        w > 0 && h > 0 && x + w <= width && y + h <= height
    };
    let mut boxes = Vec::with_capacity(rects.len());
    let mut packed_casters = Vec::with_capacity(casters.len());
    for (c, caster) in casters.iter().enumerate() {
        let whole = casts(caster) && (direct(caster) || fits(c));
        let level = if whole { caster.level.clamp(0.0, 1.0) } else { 0.0 };
        let mut entry = NO_CASTER;
        let direct = direct(caster);
        // A direct caster has no cell, so it also has no cell-resolution or
        // padding metadata. Keeping either here would make the packed frame
        // churn when an atlas-only setting changes even though the scene draw
        // cannot read it.
        let (scale, k, sigma_cell, pad) = if direct { (0.0, 0.0, 0.0, 0.0) } else { shape(caster) };
        let kind = if is_distance(caster) { DISTANCE_KIND } else { 0.0 };
        let rect = rects[c];
        let [w, h] = sizes[c];
        let [x, y] = placed[c];
        let cell =
            if whole && !direct { [x as f32, y as f32, w as f32, h as f32] } else { [0.0; 4] };
        boxes.push(ShadowBox {
            rect,
            cell,
            cell_map: [k, sigma_cell, level, scale],
            who: [c as f32, kind, pad, 0.0],
        });
        if whole {
            entry.shade = [level, kind, caster.sigma_points, 0.0];
            entry.cell = cell;
            if !direct {
                entry.map = [cell[0] - rect[0] * k, cell[1] - rect[1] * k, k, 0.0];
                entry.rect = rect;
            }
        }
        packed_casters.push(entry);
    }
    Packed { boxes, casters: packed_casters, size: [width, height] }
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
    /// The half-blurred middle, held exactly while a frame packs a COVERAGE
    /// cell ([`ensure_half`](Self::ensure_half)).
    ///
    /// A second plane the atlas's whole size, so a frame whose every group
    /// answers a distance carries none of it — and that is the frame where it
    /// would cost most, a distance cell's texels being floored at
    /// [`DISTANCE_TEXELS_PER_POINT`] where a blur cell's shrink with σ.
    half: Option<Plane>,
}

/// One atlas-sized R16Float target and the bind group that reads it, which are
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
        let texture = device.create_texture(&wgpu::TextureDescriptor {
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
        });
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
        ShadowTarget { size, atlas, half: None }
    }

    /// Whether this atlas can hold a layout of `size`.
    pub(crate) fn holds(&self, size: [u32; 2]) -> bool {
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

    /// Whether the blur's intermediate is held, which is the whole of what
    /// `only_a_frame_that_blurs_holds_the_blurs_intermediate` measures.
    #[cfg(test)]
    pub(crate) fn holds_half(&self) -> bool {
        self.half.is_some()
    }

    /// Hold the blur's intermediate while `want`, and drop it when not.
    ///
    /// Kept off the atlas's own allocation because a frame with no coverage
    /// cell in it runs no blur pass and reads the plane nowhere else, so
    /// holding it there would be a texture the atlas's whole size allocated,
    /// regrown with the atlas, and never touched.
    ///
    /// Sized to the atlas it belongs to, so it is rebuilt with the atlas and
    /// never separately — [`blur`](Self::blur) reads one at the other's
    /// coordinates.
    pub(crate) fn ensure_half(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        want: bool,
    ) {
        match (want, self.half.is_some()) {
            (true, false) => {
                let label = "lattice_shadow_atlas_half";
                self.half = Some(Plane::new(device, layout, sampler, self.size, label));
            }
            (false, true) => self.half = None,
            _ => {}
        }
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
    /// The intermediate is cleared and the atlas is NOT. A frame whose groups
    /// disagree holds both kinds of cell in the atlas, where a distance cell
    /// arrives from the ink pass already final and no draw here rewrites it —
    /// so clearing on the way back would leave those cells at zero, which the
    /// scene pass reads as a standoff of nothing and spends as a WHOLE shadow
    /// over the caster's whole padded box. Every blur cell has its own rect
    /// redrawn in full, so loading costs the Gaussian nothing.
    ///
    /// A no-op with no intermediate held, which is a frame that packed no
    /// coverage cell and so has nothing here to convolve
    /// ([`ensure_half`](Self::ensure_half)).
    pub(crate) fn blur(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: (&wgpu::RenderPipeline, &wgpu::RenderPipeline),
        boxes: &wgpu::Buffer,
        count: u32,
    ) {
        let Some(half) = self.half.as_ref() else {
            return;
        };
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

/// What binds the casters to the scene pipelines: one read-only storage
/// buffer, at group 3.
///
/// A group of its own rather than a third binding beside the atlas, because
/// the two have different lifetimes and different readers. The atlas layout is
/// shared with the BLUR pipelines, which sweep the cells and have no use for
/// the array; and the atlas's own bind groups are made once with its textures,
/// where this buffer is rewritten and regrown every frame. One layout carrying
/// both would rebuild the atlas's bind groups whenever a name arrived.
pub(crate) fn caster_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("lattice_shadow_casters_layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            // The VERTEX stage as well: a caster's quad is its widest term's
            // box, and that box is in here (`vs_shadow_box` in text.wgsl).
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

/// A buffer for `capacity` casters' kernels and the bind group naming it.
///
/// The two together because they cannot come apart: a storage buffer's bind
/// group names the buffer, so a pane that outgrows one rebuilds both. Floored
/// at one entry, an empty storage binding being a validation error and a frame
/// with no caster in it still having to bind SOMETHING for the pipeline's
/// layout.
pub(crate) fn caster_buffer(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    capacity: usize,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("lattice_shadow_casters"),
        size: (std::mem::size_of::<ShadowCaster>() * capacity.max(1)) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("lattice_shadow_casters_bind_group"),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
    });
    (buffer, bind_group)
}

/// The two pipelines that sweep blur cells.
///
/// One module and one constructor because they share a cell quad off the same
/// box stream and differ only in which axis they read. No blend anywhere here:
/// each writes its cell's texels outright.
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
    let pipeline = |entry: &str,
                    vertex: &str,
                    pipeline_layout: &wgpu::PipelineLayout,
                    format: wgpu::TextureFormat| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(vertex),
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
        blur_x: pipeline("fs_blur_x", "vs_cell", &blur_layout, ATLAS_FORMAT),
        blur_y: pipeline("fs_blur_y", "vs_cell_blur", &blur_layout, ATLAS_FORMAT),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use harmonigraph_scene::REACH_SIGMAS;

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
        Caster {
            rect: [x, y, w, h],
            level: 1.0,
            sigma_points: 1.0,
            kernel: harmonigraph_scene::ShadowKernel::Gaussian,
            direct_distance: false,
        }
    }

    /// A whole frame at ONE style: every caster at `sigma_px` in the target's
    /// pixels, drawn by `kernel`.
    ///
    /// What a claim about the PACKING rather than about the groups is asked
    /// at — the shelving, the resolution ladder, the atlas limit — none of
    /// which is a question about which group a caster belongs to. σ in target
    /// pixels because that is the unit those claims are stated in; the caster
    /// carries points and the packer multiplies. The claims that ARE about
    /// groups build their casters by hand.
    fn pack_at(
        casters: &[Caster],
        sigma_px: f32,
        px_per_point: f32,
        max_side: u32,
        kernel: harmonigraph_scene::ShadowKernel,
    ) -> Packed {
        let casters: Vec<Caster> = casters
            .iter()
            .map(|c| Caster { sigma_points: sigma_px / px_per_point, kernel, ..*c })
            .collect();
        pack(&casters, px_per_point, max_side)
    }

    /// One Gaussian, which is what every claim about the packing that is not
    /// about the DISTANCE renderer is asked at.
    fn one() -> harmonigraph_scene::ShadowKernel {
        harmonigraph_scene::ShadowKernel::Gaussian
    }

    /// What the DISTANCE renderer costs in ATLAS against the Gaussian, at both
    /// ends of the Shadow bar.
    ///
    /// The number `timing.rs` cannot give: a frame's GPU time on a contended
    /// machine swings by a factor of ten between its p10 and its p90, where the
    /// packing is arithmetic and answers the same way every run. It is also the
    /// cost that decides whether the pair is affordable, the blur chain over
    /// the cells being unchanged — every cell's σ is capped in TEXELS, so what
    /// a renderer buys is area rather than a wider kernel.
    ///
    /// Read at both ends because the two are not the same question. Cells are
    /// drawn at `min(1, SIGMA_CELL_MAX / σ)`, so at a NARROW Shadow both are at
    /// the pane's own resolution and cost about the same. At a wide one a blur
    /// cell scales down with its own σ while a Distance cell stops at the
    /// quality floor.
    ///
    /// Past the bound below, the renderer is a reason the atlas hits `max_side`
    /// rather than a shape to compare (see
    /// `a_node_close_to_the_eye_packs_a_cell_the_atlas_can_hold`).
    #[test]
    fn a_kernel_row_costs_this_much_atlas_against_one_gaussian() {
        use harmonigraph_scene::ShadowKernel::{Distance, Gaussian};
        // A pane's worth of names: a run of type is the caster the atlas is
        // mostly made of, and a node's box is the same shape at a bigger size.
        let casters: Vec<Caster> =
            (0..30).map(|i| caster(i as f32 * 3.0, 0.0, 40.0, 12.0)).collect();
        for (sigma, what) in [(6.0f32, "a fresh Shadow"), (60.0, "the top of the bar")] {
            // The CELLS' own area, not the texture's: a target is rounded up to
            // a power of two in each direction, which quantizes every reading
            // to a factor of two and hides what a row actually asked for.
            let area = |kernel: harmonigraph_scene::ShadowKernel| -> f64 {
                pack_at(&casters, sigma, 2.0, 16384, kernel)
                    .boxes
                    .iter()
                    .map(|b| f64::from(b.cell[2]) * f64::from(b.cell[3]))
                    .sum()
            };
            let plain = area(Gaussian);
            assert!(plain > 0.0, "one Gaussian packed nothing at {what}");
            // The DISTANCE renderer on a bound of its own, two orders of
            // magnitude above the Gaussian's rather than beside it. A blur cell
            // shrinks with σ and a distance cell stops at its quality floor, so
            // the top of the bar is where the two costs part company by
            // construction. This fixture measures about 87x at the top of the
            // bar; the ceiling catches a change that walks the atlas into
            // `max_side`, where a caster stops casting with nothing on screen
            // to say so.
            let ratio = area(Distance) / plain;
            eprintln!(
                "Distance at {what}, {DISTANCE_TEXELS_PER_POINT:.2} tex/pt: {ratio:.2}x one \
                 Gaussian's cells"
            );
            assert!(
                ratio <= 120.0,
                "Distance packs {ratio:.2}x one Gaussian's cells at {what}, which is a row that \
                 reaches the device's texture limit rather than a row to compare",
            );
        }
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
        assert_eq!(
            reach,
            harmonigraph_scene::REACH_SIGMAS,
            "the kernel reaches a different distance than the padding"
        );
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
            let packed = pack_at(&[caster(10.0, 10.0, 40.0, 12.0)], sigma_px, 2.0, 16384, one());
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
        assert_eq!(pack_at(&[], 4.0, 2.0, 16384, one()), Packed::default());
        for sigma in [0.0, f32::NAN] {
            let packed = pack_at(&[caster(0.0, 0.0, 10.0, 10.0)], sigma, 2.0, 16384, one());
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
        let packed = pack_at(&casters, 6.0, 2.0, 16384, one());
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
        assert_eq!(pack_at(&casters, 6.0, 2.0, 16384, one()), packed);
    }

    /// A cell past the texture limit is no cell, with its level zeroed so its
    /// box draws nothing, and the rest of the frame keeps its shadows.
    #[test]
    fn a_cell_the_atlas_cannot_hold_casts_nothing() {
        let casters: Vec<Caster> = (0..8).map(|_| caster(0.0, 0.0, 100.0, 100.0)).collect();
        let packed = pack_at(&casters, 2.0, 1.0, 256, one());
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
        let without = pack_at(&[live, live], 2.0, 1.0, 4096, one());
        let with_dead = pack_at(&[live, dead, live], 2.0, 1.0, 4096, one());
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

    /// Reach is paint reach, not merely the selected kernel's mathematical
    /// support. Either shut bar leaves no pixels for the caller to cull or
    /// enlarge geometry around.
    #[test]
    fn a_spectral_style_that_cannot_cast_has_no_reach() {
        for kernel in
            [harmonigraph_scene::ShadowKernel::Distance, harmonigraph_scene::ShadowKernel::Gaussian]
        {
            let style = |width, depth| harmonigraph_scene::ShadowStyle { width, depth, kernel };
            assert!(spectral_shadow_reach(style(1.0, 1.0)) > 0.0, "the live fixture is shut");
            assert_eq!(spectral_shadow_reach(style(0.0, 1.0)), 0.0, "width endpoint");
            assert_eq!(spectral_shadow_reach(style(1.0, 0.0)), 0.0, "depth endpoint");
            assert_eq!(spectral_shadow_reach(style(f32::NAN, 1.0)), 0.0, "repaired width");
            assert_eq!(spectral_shadow_reach(style(1.0, f32::NAN)), 0.0, "repaired depth");
        }
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
            one(),
        );
        assert_eq!(packed.boxes.len(), 2);
        let [a, b] = [packed.boxes[0], packed.boxes[1]];
        assert_eq!(a.cell[1], b.cell[1], "the pair must share a shelf to be neighbours");
        assert_eq!(a.cell[0] + a.cell[2], b.cell[0], "the pair must touch");
        // A width the harness's readback can take: it copies 4-byte rows.
        let size = [packed.size[0].max(64), packed.size[1]];
        let layout = read_layout(&device);
        let sampler = device.create_sampler(&Default::default());
        let mut target = ShadowTarget::new(&device, &layout, &sampler, size);
        target.ensure_half(&device, &layout, &sampler, true);
        let pipelines = create_cell_pipelines(&device, &layout);

        // The block: cell A's ink region — everything inside its padding —
        // filled solid, as a caster wider than the blur is.
        let pad = ((REACH_SIGMAS * a.cell_map[1]).ceil() + 1.0) as u32;
        let (ax, ay, aw, ah) =
            (a.cell[0] as u32, a.cell[1] as u32, a.cell[2] as u32, a.cell[3] as u32);
        let inked = |x: u32, y: u32| {
            x >= ax + pad && x < ax + aw - pad && y >= ay + pad && y < ay + ah - pad
        };
        let mut ink = vec![0u8; (size[0] * size[1] * 2) as usize];
        let mut mass_before = 0.0f64;
        for y in 0..size[1] {
            for x in 0..size[0] {
                if inked(x, y) {
                    let at = ((y * size[0] + x) * 2) as usize;
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
                bytes_per_row: Some(size[0] * 2),
                rows_per_image: Some(size[1]),
            },
            wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        );
        let boxes = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test_boxes"),
            size: (2 * std::mem::size_of::<ShadowBox>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&boxes, 0, bytemuck::cast_slice(&packed.boxes));
        let mut encoder = device.create_command_encoder(&Default::default());
        target.blur(&mut encoder, (&pipelines.blur_x, &pipelines.blur_y), &boxes, 2);
        queue.submit([encoder.finish()]);
        let bytes = crate::gpu_harness::readback(&device, &queue, target.texture(), size);
        let at = |x: u32, y: u32| -> f32 {
            let i = ((y * size[0]) * 4 + x * 2) as usize;
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

    /// The two shader modules that spell a distance cell's kind agree on it,
    /// and the scene and consumer agree on the standoff's window.
    ///
    /// Each has to be written more than once — there is no linkage between
    /// shader modules here, and the scene crate has no shader at all — and each
    /// fails SILENTLY if the copies part. A kind that has drifted collapses
    /// every distance cell's quad and draws no shadow at all; a window that has
    /// drifted pads a cell to one radius and cuts its curve off at another,
    /// which is a screen-aligned step in the halo.
    #[test]
    fn the_shaders_distance_kind_and_window_are_the_packers() {
        let common = crate::with_common("");
        for src in [SHADOW_SRC, common.as_str()] {
            let kind: f32 = shader_const(src, "DISTANCE_KIND").parse().expect("a number");
            assert_eq!(kind, DISTANCE_KIND, "a module reads a different kind than the packer");
        }
        for (name, want) in [
            ("SHADOW_TAIL", harmonigraph_scene::SHADOW_TAIL),
            ("SHADOW_STOP", harmonigraph_scene::SHADOW_STOP),
        ] {
            let held: f32 = shader_const(&common, name).parse().expect("a number");
            assert_eq!(held, want, "common.wgsl's {name} and the scene's have parted");
        }
    }

    /// A distance cell is padded to exactly where its curve is windowed
    /// to nothing and stops shrinking at the renderer's quality floor.
    ///
    /// The pad has to REACH the stop or the shadow ends in a straight line at
    /// the cell's edge. The floor is in pane points so editor and offline
    /// renderings sample the same grid at every target scale. A blur cell keeps
    /// shrinking with σ, which is what bounds its convolution cost.
    #[test]
    fn a_distance_cell_reaches_the_stop_and_holds_its_quality_floor() {
        use harmonigraph_scene::ShadowKernel;
        let caster = Caster {
            rect: [40.0, 40.0, 20.0, 20.0],
            level: 1.0,
            sigma_points: 1.0,
            kernel: ShadowKernel::Distance,
            direct_distance: false,
        };
        // A σ well past `SIGMA_CELL_MAX`, so the floor rather than the
        // target's full resolution or the blur's fit decides the cell.
        let sigma = 40.0;
        let kernel = ShadowKernel::Distance;
        let out = pack_at(&[caster], sigma, 1.0, 4096, kernel);
        let cell = out.boxes[0];
        for px_per_point in [1.0f32, 1.5, 2.0, 4.0] {
            let held =
                pack_at(&[caster], sigma * px_per_point, px_per_point, 8192, kernel).boxes[0];
            assert!(
                (held.cell_map[0] - DISTANCE_TEXELS_PER_POINT).abs() < 1e-5,
                "at {px_per_point} pixels a point the cell packed {} texels per point rather \
                 than {DISTANCE_TEXELS_PER_POINT}",
                held.cell_map[0],
            );
        }
        assert_eq!(cell.cell_map[1], 0.0, "a distance cell carries a blur σ");
        assert_eq!(cell.who[1], DISTANCE_KIND, "the box does not say it holds a distance");
        // The pad is the stop in points, plus the one texel the sampler's
        // bilinear tap at the box's own edge needs.
        let want = 2.0 * harmonigraph_scene::SHADOW_STOP * sigma;
        let pad = cell.who[2];
        assert!(
            pad >= want && pad <= want + 1.0 / DISTANCE_TEXELS_PER_POINT + 1.0,
            "a distance cell is padded {pad} points where its curve reaches {want}",
        );
        // A blur cell belongs to the other fill and sampling branch.
        let blur = pack_at(&[caster], sigma, 1.0, 4096, ShadowKernel::Gaussian);
        assert_eq!(blur.boxes[0].who[1], 0.0, "a blur box says it holds a distance");
        assert!((blur.boxes[0].cell_map[0] - SIGMA_CELL_MAX / sigma).abs() < 1e-5);
    }

    /// A frame whose GROUPS disagree packs each caster at its own kernel and
    /// its own width, on one shelf.
    ///
    /// The claim the two-group design rests on, and the one a packer that kept
    /// either property per FRAME would fail while every uniform-frame reading
    /// above went on passing: the kinds part, the resolutions part in the
    /// direction each renderer's own rule says, and the σ each caster carries
    /// to the shader is its own.
    ///
    /// The σ apart by a factor of four so the two resolutions cannot agree by
    /// accident, and both past `SIGMA_CELL_MAX` so each is really sized by its
    /// own rule rather than pinned at the target's resolution: the blur cell
    /// then follows σ down and the distance cell stops at its quality floor.
    #[test]
    fn a_mixed_frame_packs_each_caster_at_its_own_style() {
        use harmonigraph_scene::ShadowKernel::{Distance, Gaussian};
        let at = |sigma: f32, kernel| Caster {
            rect: [0.0, 0.0, 40.0, 12.0],
            level: 1.0,
            sigma_points: sigma,
            kernel,
            direct_distance: false,
        };
        let (near, far) = (10.0, 40.0);
        let packed = pack(&[at(near, Distance), at(far, Gaussian)], 1.0, 16384);
        let [d, g] = [packed.boxes[0], packed.boxes[1]];
        assert_eq!(d.who[1], DISTANCE_KIND, "the distance caster packed a coverage cell");
        assert_eq!(g.who[1], 0.0, "the Gaussian caster packed a distance cell");
        assert_eq!(d.cell_map[0], DISTANCE_TEXELS_PER_POINT, "the distance cell missed its floor");
        assert!(
            (g.cell_map[0] - SIGMA_CELL_MAX / far).abs() < 1e-5,
            "the Gaussian cell packed {} texels a point rather than following its own σ",
            g.cell_map[0],
        );
        assert_eq!(d.cell_map[1], 0.0, "the distance cell carries a blur σ");
        assert!((g.cell_map[1] - SIGMA_CELL_MAX).abs() < 1e-4, "the Gaussian cell is past the cap");
        // And what each hands the SHADER: its own kind and its own σ in points,
        // which is what a sampler reading one frame's worth of casters spends.
        let [ed, eg] = [packed.casters[0], packed.casters[1]];
        assert_eq!([ed.shade[1], ed.shade[2]], [DISTANCE_KIND, near]);
        assert_eq!([eg.shade[1], eg.shade[2]], [0.0, far]);
        // One shelf, disjoint: a mixed frame is one packing rather than two.
        assert!(d.cell[2] > 0.0 && g.cell[2] > 0.0, "a caster in the mixed frame got no cell");
        assert!(
            d.cell[0] + d.cell[2] <= g.cell[0] || g.cell[0] + g.cell[2] <= d.cell[0],
            "the two cells overlap: {:?} and {:?}",
            d.cell,
            g.cell,
        );
    }

    /// A caster that owns its exact distance keeps the profile metadata without
    /// occupying an atlas cell. Under the GAUSSIAN the same caster still takes
    /// a real cell, because its scene draw does not hold that convolution.
    #[test]
    fn a_direct_distance_keeps_only_the_blur_cell() {
        use harmonigraph_scene::ShadowKernel;
        let caster = Caster {
            rect: [40.0, 40.0, 20.0, 20.0],
            level: 1.0,
            sigma_points: 1.0,
            kernel: ShadowKernel::Distance,
            direct_distance: true,
        };
        let distance = pack_at(&[caster], 40.0, 2.0, 4096, ShadowKernel::Distance);
        assert_eq!(distance.boxes[0].cell, [0.0; 4], "the direct caster packed an atlas cell");
        assert_eq!(distance.casters[0].shade[1], DISTANCE_KIND);
        assert_eq!(distance.casters[0].shade[2], 20.0);
        assert_eq!(distance.casters[0].shade[0], 1.0);

        let blur = pack_at(&[caster], 40.0, 2.0, 4096, ShadowKernel::Gaussian);
        assert!(blur.boxes[0].cell[2] > 0.0 && blur.boxes[0].cell[3] > 0.0);
        assert_eq!(blur.casters[0].shade[1], 0.0, "a Gaussian caster says it holds a distance");
    }

    /// A caster whose distance is held in the atlas keeps the cell its scene
    /// draw samples and marks it as a distance field. Names and nodes share
    /// that representation.
    #[test]
    fn an_atlas_distance_keeps_the_cell_its_scene_draw_samples() {
        use harmonigraph_scene::ShadowKernel;
        let node = Caster {
            rect: [40.0, 40.0, 20.0, 20.0],
            level: 1.0,
            sigma_points: 1.0,
            kernel: ShadowKernel::Distance,
            direct_distance: false,
        };
        let exact = pack_at(&[node], 40.0, 2.0, 4096, ShadowKernel::Distance);
        assert!(exact.boxes[0].cell[2] > 0.0 && exact.boxes[0].cell[3] > 0.0);
        assert_eq!(exact.boxes[0].who[1], DISTANCE_KIND);

        let name = caster_of(&[crate::text::tests::glyph()], 1.0, ShadowKernel::Distance);
        let name = pack_at(&[name], 40.0, 2.0, 4096, ShadowKernel::Distance);
        assert!(name.boxes[0].cell[2] > 0.0 && name.boxes[0].cell[3] > 0.0);
        assert_eq!(name.boxes[0].who[1], DISTANCE_KIND);
    }

    /// Every atlas-backed distance cell keeps the quality floor while a blur
    /// cell remains sigma-relative.
    ///
    /// The caster is the node path: its field is analytic when filled, but the
    /// scene still samples that field out of an atlas cell. Letting that cell
    /// follow σ alone makes its grid visible and changes the grid under a
    /// moving Shadow bar.
    #[test]
    fn a_node_distance_cell_keeps_the_quality_floor_and_a_blur_does_not() {
        use harmonigraph_scene::ShadowKernel;
        let caster = caster(40.0, 40.0, 20.0, 20.0);
        for sigma_points in [1.0f32, 4.0, 40.0] {
            for px_per_point in [1.0f32, 2.0, 4.0] {
                let sigma_px = sigma_points * px_per_point;
                let blur_resolution = (SIGMA_CELL_MAX / sigma_points).min(px_per_point);
                let distance_resolution = blur_resolution.max(DISTANCE_TEXELS_PER_POINT);
                let distance =
                    pack_at(&[caster], sigma_px, px_per_point, 8192, ShadowKernel::Distance).boxes
                        [0];
                let gaussian =
                    pack_at(&[caster], sigma_px, px_per_point, 8192, ShadowKernel::Gaussian).boxes
                        [0];
                assert!(
                    (distance.cell_map[0] - distance_resolution).abs() < 1e-5,
                    "Distance at σ {sigma_points} points and {px_per_point} px/pt packed {} \
                     texels per point instead of {distance_resolution}",
                    distance.cell_map[0],
                );
                assert!(
                    (gaussian.cell_map[0] - blur_resolution).abs() < 1e-5,
                    "Gaussian at σ {sigma_points} points and {px_per_point} px/pt packed {} \
                     texels per point instead of {blur_resolution}",
                    gaussian.cell_map[0],
                );
            }
        }
    }
}
