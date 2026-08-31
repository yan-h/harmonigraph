//! The shadow atlas: every caster's ink blurred into a cell of its own, which
//! is what that caster multiplies the frame by in its own draw
//! (`fs_shadow_box` in text.wgsl).
//!
//! One cell per caster PER TERM of the kernel, each at a scale that keeps the
//! blur's cost flat: a cell is the caster's box grown by that term's reach,
//! drawn at `min(1, 3 / σ_t)` of the target's pixels, so σ is at most
//! `SIGMA_CELL_MAX` texels in every cell and the kernel at most nineteen taps
//! whatever the Shadow bar says and whatever row of the kernel table is
//! chosen. The atlas is about the names' own area at the fresh Shadow and one
//! Gaussian, shrinks as the bar widens, and grows with the term count.
//!
//! Each term at its OWN resolution is the whole point of the shape: a mixture's
//! narrow term carries the kernel's core, and a shared resolution picked for
//! the widest term would resample that core away and leave every row of the
//! table reading as the same soft blob.
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

/// The fewest texels of a DISTANCE cell one POINT of the pane may be drawn at.
///
/// A blur cell shrinks without limit as σ grows (`SIGMA_CELL_MAX / σ`), because
/// a blur needs no more texels than its own width. A distance cell cannot: FORM
/// is the point of the family, and form lives at the ink's own resolution, so
/// what has to survive is the thinnest stroke in the picture.
///
/// Two texels of it. The lattice's thinnest ink is a stroke of type, and a name
/// is set at `NAME_SIZE` 30 points (`harmonigraph-ui`'s `marks.rs`) whose
/// monospace stem is about a twelfth of the em — two and a half POINTS. Two
/// texels of that is 0.8 a point, and under it a stem is one texel or none: the
/// flood's seeds come off a coverage contour (`INK_FLOOR`), and a stroke that
/// covers no texel to half seeds nothing at all, so its shadow does not thin —
/// it disappears.
///
/// In the pane's POINTS and not in the target's pixels, which is the whole
/// reason it is written this way round: a stem is a fixed number of points and
/// a varying number of pixels, so a floor on the fraction of the target would
/// hold two texels only at the framing it was fitted to. The editor runs at 2
/// pixels a point and the offline renderer at 1 to 4 (`default_scale` in
/// harmonigraph-offline's `main.rs`, 1.5 for a 1080p export), so a fraction
/// fitted to the editor is a name whose shadow is present on screen and gone
/// from the mp4. The cell's cost is unchanged by the swap: what it decides is
/// `k`, and `k` is this number.
///
/// Above the floor a distance cell shrinks with the Shadow bar exactly as a
/// blur cell does, so the widest settings cost no more atlas per cell than the
/// floor allows. What it does NOT bound is the cell's area, which grows with
/// the pad: `timing.rs` is what reads that back at the top of the bar, and
/// `a_kernel_row_costs_this_much_atlas_against_one_gaussian` bounds it.
pub(crate) const DISTANCE_TEXELS_PER_POINT: f32 = 0.8;

/// What the flood's ping-pong pair is kept in: one texel's nearest seed, as a
/// pair of ABSOLUTE atlas coordinates.
///
/// The low fourteen bits hold an atlas coordinate, enough for every 2D texture
/// wgpu exposes here. Three high bits hold the contour's line direction, and
/// the remaining high bit marks a directionless point seed. An all-ones x is
/// the sentinel, so no third channel is needed for any of the three states.
pub(crate) const SEED_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg16Uint;

/// The largest atlas dimension the packed seed coordinates can name.
///
/// Fourteen coordinate bits address 0..16383. A device offering a larger
/// texture is still held here for a distance row, so its normal bits cannot be
/// mistaken for position (`pack_seed` in shadow.wgsl).
pub(crate) const SEED_COORD_LIMIT: u32 = 1 << 14;

/// The largest jump the chain starts from, as a power of two in texels.
///
/// A bound on the PASS COUNT rather than on the Shadow: 2^14 is 16384 texels,
/// past the widest atlas any device here reports, so the cap is unreachable in
/// the picture and is here to keep a nonsense reach — a NaN out of a corrupt
/// blob — from asking for an unbounded chain.
const MAX_LOG_STEP: u32 = 14;

/// The jumps the flood takes, largest first, halving to 1.
///
/// A jump flood carries a seed `2^k + 2^(k-1) + ... + 1` texels, one short of
/// `2^(k+1)`, so starting at the first power of two at or above `reach` reaches
/// every seed within it with the whole of the top jump to spare.
///
/// The tail is doubled when the count comes out ODD, which buys two things at
/// the price of one pass. It is the standard extra step-of-1 refinement — the
/// flood is exact for a single seed and approximate for many, and the errors it
/// makes are all within a texel or two of a territory boundary, which one more
/// local pass mops up. And it lands the answer in a FIXED one of the two
/// textures, so the resolve binds one bind group rather than choosing by
/// parity. Written as the POSITIVE test so a NaN reach takes the empty schedule
/// rather than `log2`'s answer for one.
///
/// A reach under a texel runs no chain at all: the only texels such a shadow
/// speaks for are the inked ones, and what those are owed comes off the
/// coverage directly (`fs_flood_resolve`). Empty is even, which is the property
/// the resolve depends on.
pub(crate) fn steps(reach: f32) -> Vec<i32> {
    if reach >= 1.0 {
        let top = (reach.log2().ceil() as u32).min(MAX_LOG_STEP);
        let mut out: Vec<i32> = (0..=top).rev().map(|k| 1i32 << k).collect();
        if out.len() % 2 == 1 {
            out.push(1);
        }
        return out;
    }
    Vec::new()
}

/// σ of a caster's blur in the target's pixels, for a Shadow of `shadow` node
/// radii over a node of `node_points` points, on a pane at `pixels_per_point`
/// drawn at `render_scale`.
///
/// HALF the bar's width. A half-plane blurred at σ keeps `erfc(d / (σ√2)) / 2`
/// of the light `d` out from its edge, which at `d = 2σ` is 2.3% — so one
/// Shadow width is where a wide caster's shadow has all but run out, which is
/// what the bar says it is.
///
/// The frame's one σ, which is what a caster takes a RATIO of
/// ([`Caster::sigma_scale`]) rather than what every caster is blurred at: one
/// bar, one reach, across a ring, a cross and a name, save where a note name is
/// dialled off it on purpose.
///
/// Target pixels rather than points, because the cell is drawn at the target's
/// own resolution and sampled back in points: `render_scale` is the term #496
/// found missing from the field's reach, and it is here on purpose. Written as
/// the POSITIVE test so a NaN out of a corrupt blob is no shadow rather than a
/// kernel of NaNs.
pub(crate) fn sigma_px(
    shadow: f32,
    node_points: f32,
    pixels_per_point: f32,
    render_scale: f32,
) -> f32 {
    let sigma = 0.5 * shadow * node_points * pixels_per_point * render_scale;
    if sigma > 0.0 {
        sigma
    } else {
        0.0
    }
}

/// What a caster hands the packer: its ink's bounding box in the pane's points
/// (min, then size), how much of its shadow lands, 0..=1, and what its own σ
/// takes against the frame's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Caster {
    pub rect: [f32; 4],
    pub level: f32,
    /// A RATIO on the frame's one σ ([`sigma_px`]), so a caster that wants the
    /// picture's own width says 1 and the bar that dials the width dials this
    /// caster with it.
    ///
    /// It is per caster because a note NAME is dialled against the rest of the
    /// lattice (`ViewConfig::glow_shadow_name`): a letterform is the only ink
    /// here whose shape is meant to be read, and the blur that says how thick a
    /// ring is may not be the blur that leaves an `e` an `e`. Every other
    /// caster passes 1.
    ///
    /// Each cell's own scale, pad and σ in texels come off this, so a caster at
    /// three times the width is a cell drawn a third the size rather than a
    /// kernel three times as wide: the blur's tap count is flat in this exactly
    /// as it is in the bar.
    pub sigma_scale: f32,
}

/// The caster a name's glyphs make: the box round every glyph's rect, the
/// strength the name's rim colour carries — the one number a lattice name's
/// `rim` holds (`LABEL_SHADOW` in harmonigraph_ui), so a name easing in as the
/// marker under it eases out grows its shadow on the clock its ink arrives on —
/// and `sigma_scale`, which is a name's alone to be other than 1
/// (`ViewConfig::glow_shadow_name`).
///
/// A run with no ink in it — every rect empty — is a caster of nothing, with
/// its level zeroed rather than a box of infinities for the packer to size.
pub(crate) fn caster_of(glyphs: &[crate::GlyphInstance], sigma_scale: f32) -> Caster {
    let (mut min, mut max) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for g in glyphs {
        for axis in 0..2 {
            min[axis] = min[axis].min(g.rect[axis]);
            max[axis] = max[axis].max(g.rect[axis] + g.rect[axis + 2]);
        }
    }
    if !(max[0] > min[0] && max[1] > min[1]) {
        return Caster { rect: [0.0; 4], level: 0.0, sigma_scale };
    }
    let level = glyphs.iter().map(|g| f32::from(g.rim[3]) / 255.0).fold(0.0, f32::max);
    Caster { rect: [min[0], min[1], max[0] - min[0], max[1] - min[1]], level, sigma_scale }
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
    pub terms: [f32; 4],
    /// x: which caster this box belongs to, as an index into
    /// [`Packed::casters`]; y: what this cell HOLDS, 0 blurred ink and 1 a
    /// distance ([`DISTANCE_KIND`]); z: how far past the caster's ink this cell
    /// reaches, in the pane's points — the pad the rect was grown by, which is
    /// where the standoff's curve is windowed to nothing and so the value a
    /// texel out of every seed's reach is resolved to; w unused.
    ///
    /// x is read by the node's SCENE draw, which needs every term at once and
    /// so reaches the array rather than the box; y and z by the passes that
    /// sweep the CELLS, which take one at a time and have to know which chain
    /// this one belongs to. Carried on the box rather than in a buffer of its
    /// own because the node's two draws — the one that fills a cell and the one
    /// that reads the atlas — bind the same stream, and one row is cheaper than
    /// a second binding.
    pub who: [f32; 4],
}

/// What `ShadowBox::who`'s y and `ShadowCaster::kind` hold for a term whose
/// cell is a DISTANCE rather than blurred ink — `DISTANCE_KIND` in shadow.wgsl
/// and common.wgsl, pinned by `the_shaders_distance_kind_is_the_packers`.
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

    /// The same rows at the locations after a glyph's five, for the draw that
    /// rasterizes a glyph into its cell alongside `GlyphInstance::LAYOUT`.
    pub(crate) const BESIDE_GLYPHS: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ShadowBox>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4
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
}

/// A caster with no cell at all: what a draw carries when the frame packed
/// nothing, and what every reader of a box answers 1 to (`shadow_through` in
/// lattice.wgsl, `fs_shadow_box` in text.wgsl) — the frame left exactly whole,
/// with nothing sampled.
pub(crate) const NO_CELL: ShadowBox =
    ShadowBox { rect: [0.0; 4], cell: [0.0; 4], terms: [0.0; 4], who: [0.0; 4] };

/// A frame's cells, packed: one box per caster per TERM, in the caster's own
/// order with the terms consecutive inside it, the same casters gathered as the
/// scene pass reads them, and the atlas size that holds it all.
///
/// The two views of one packing. `boxes` is what FILLS and BLURS a cell — one
/// instance per cell, each carrying its own σ and scale — and `casters` is what
/// SAMPLES them, one entry per caster carrying every term at once, because a
/// mixture has to be mixed before it is spent (`shadow_transmittance`) and a
/// draw cannot take one term at a time.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Packed {
    pub boxes: Vec<ShadowBox>,
    pub casters: Vec<ShadowCaster>,
    pub size: [u32; 2],
    /// How far the flood has to carry a seed, in texels: the widest distance
    /// cell's own pad, and 0 where this frame packed no distance cell at all.
    ///
    /// ONE chain over every distance cell in the atlas, so the schedule is the
    /// widest cell's and a narrower one pays passes whose every tap lands
    /// outside its own rect and is dropped. The alternative — a chain per cell
    /// — is a render pass per cell per step, which is the cost this design
    /// exists to not pay.
    ///
    /// A pure function of the frame, like the rest of the packing, which is
    /// what the offline renderer's determinism rests on.
    pub flood: f32,
}

/// One caster's whole kernel, as the scene pass reads it: the quad to draw over
/// and every term's cell and mapping.
///
/// In a STORAGE BUFFER rather than beside the instance, which is the one place
/// this design departs from #527's sketch. A node's own rows reach location 15
/// and leave five free; two terms consume four of those rows for their cells
/// and maps before the caster's rect and term metadata have been carried.
/// Indexed by the caster's own index — the order `pack` was handed — so a node,
/// a name and the marker field all reach it the same way and nothing carries a
/// second copy.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ShadowCaster {
    /// The union of every term's padded box, in the pane's points: min, then
    /// size. What a caster's shadow is drawn OVER, so the widest term finishes
    /// inside the quad rather than being cut off in a straight line at it.
    pub rect: [f32; 4],
    /// x: how much of this caster's shadow lands, 0..=1 — zero where the atlas
    /// could not hold every one of its cells, which is a caster that darkens
    /// nothing rather than one that darkens by part of its kernel. y/z/w unused.
    pub level: [f32; 4],
    /// What each term's cell HOLDS: 0 blurred ink, [`DISTANCE_KIND`] a
    /// distance. Zeroed past the kernel's own term count.
    ///
    /// Per caster rather than in a uniform, though every caster in a frame is
    /// packed off one row and so carries the same kinds: the two shader modules
    /// that read a kernel declare this struct once between them (common.wgsl)
    /// and their uniforms separately, so a term's own properties riding here
    /// are written down once and a copy in each `Locals` would be two.
    pub kind: [f32; harmonigraph_scene::SHADOW_TERMS_MAX],
    /// Each term's σ in the pane's POINTS — what a distance read out of a cell
    /// is measured against, one Shadow width being 2σ.
    ///
    /// In points because that is what the cell holds
    /// (`fs_flood_resolve`): the sampler divides one by the other and the
    /// target's own pixel scale never enters, so a Render scale moves neither.
    /// Per caster because [`Caster::sigma_scale`] is.
    pub sigma: [f32; harmonigraph_scene::SHADOW_TERMS_MAX],
    /// Each term's cell in atlas texels: origin, then size. Zeroed past the
    /// kernel's own term count, and zeroed whole where the caster casts
    /// nothing.
    pub cell: [[f32; 4]; harmonigraph_scene::SHADOW_TERMS_MAX],
    /// Each term's map from a point of the pane to a texel of its cell: x/y the
    /// origin, z the scale, so a texel is `xy + points * z`; w this term's
    /// share of the mixture, which is 0 on a distance term — normalized over
    /// the BLUR terms alone, so a lone Gaussian standing beside a distance term
    /// arrives whole.
    ///
    /// Pre-composed on the CPU rather than sent as (cell origin, box origin,
    /// scale) for the shader to combine: the three differ per TERM, and a
    /// fragment would otherwise repeat the subtraction for every term for a
    /// number that never changes within a frame.
    pub map: [[f32; 4]; harmonigraph_scene::SHADOW_TERMS_MAX],
}

/// A caster the frame packed nothing for: what every draw carries when the
/// Shadow is shut, and what a reader answers 1 to with nothing sampled.
pub(crate) const NO_CASTER: ShadowCaster = ShadowCaster {
    rect: [0.0; 4],
    level: [0.0; 4],
    kind: [0.0; harmonigraph_scene::SHADOW_TERMS_MAX],
    sigma: [0.0; harmonigraph_scene::SHADOW_TERMS_MAX],
    cell: [[0.0; 4]; harmonigraph_scene::SHADOW_TERMS_MAX],
    map: [[0.0; 4]; harmonigraph_scene::SHADOW_TERMS_MAX],
};

/// Every caster's cell, shelf-packed in the order the casters arrive.
///
/// `px_per_point` is the target's pixels per pane point — the device's scale
/// times the render scale — and `max_side` the device's texture limit.
///
/// A PURE function of this frame, which is what the offline renderer's
/// determinism rests on: the layout depends on the casters, σ and nothing a
/// previous frame left behind. The texture that holds it may be larger than
/// `size` (it grows to demand and never shrinks, see [`ShadowTarget`]); the
/// cells' texel coordinates are absolute, so that changes nothing sampled.
///
/// A cell the atlas cannot hold — past `max_side` in either direction — is
/// packed as no cell at all, its level zeroed so the box draws nothing. At the
/// scales here that is over a hundred pane-fuls of names; the fallback
/// criterion in #498 is what a frame that reaches it calls for.
pub(crate) fn pack(
    casters: &[Caster],
    sigma_px: f32,
    px_per_point: f32,
    max_side: u32,
    kernel: &[harmonigraph_scene::KernelTerm],
) -> Packed {
    // A finite positive number, which a NaN or an infinity out of a corrupt
    // blob is not: either is no shadow rather than a kernel of nothing.
    let positive = |x: f32| x.is_finite() && x > 0.0;
    if casters.is_empty() || kernel.is_empty() || !positive(sigma_px) || !positive(px_per_point) {
        return Packed::default();
    }
    // Held to what the atlas and the sampler are built for, so a longer row
    // added to the table stops at the representation's bound rather than
    // reading past the array a caster carries.
    let kernel = &kernel[..kernel.len().min(harmonigraph_scene::SHADOW_TERMS_MAX)];
    let is_distance =
        |t: &harmonigraph_scene::KernelTerm| t.kind == harmonigraph_scene::TermKind::Distance;
    // NORMALIZED where the row is read, so a table that sums to 1.001 is a
    // rounding in the fit rather than a tenth of a percent of extra darkness,
    // and a row of zeros is no shadow rather than a division.
    //
    // Over the BLUR terms alone. A distance term is not a share of a mixture —
    // it is the shape the row draws — so counting it would hand a share of the
    // darkness to a term that never asked for one, and halve any Gaussian
    // standing beside it.
    let total: f32 = kernel.iter().filter(|t| !is_distance(t)).map(|t| t.weight.max(0.0)).sum();
    let weight = |t: &harmonigraph_scene::KernelTerm| {
        if positive(total) && !is_distance(t) {
            t.weight.max(0.0) / total
        } else {
            0.0
        }
    };
    // One term's σ for one caster, in the target's pixels: the frame's, scaled
    // by what the caster asked for and by what the term is. `max` and not a
    // branch, so a NaN out of a corrupt blob comes out as 0 — a hard-edged
    // shadow of the caster's own ink rather than a kernel of nothing.
    let sigma_of = |c: &Caster, t: &harmonigraph_scene::KernelTerm| {
        (sigma_px * c.sigma_scale * t.sigma).max(0.0)
    };
    // A cell, sized off ITS OWN σ: drawn at `min(1, SIGMA_CELL_MAX / σ)` of the
    // target's pixels so σ is at most `SIGMA_CELL_MAX` texels whatever the term
    // or the caster asked for, and padded by the kernel's reach in those same
    // texels, plus one so the scene pass's bilinear tap at the box's own edge
    // still lands inside the cell.
    //
    // Per term and not per frame, which is what holds the blur's cost flat
    // across the whole table: the chain reads σ off each cell and clamps its
    // taps to that cell's rect, so N terms are N cells the existing pass sweeps
    // rather than N kernels any one of them is blurred by.
    // A DISTANCE cell parts from that in both numbers, and each for its own
    // reason. Its resolution is floored ([`DISTANCE_TEXELS_PER_POINT`]) because
    // form is what the family draws and a stroke under two texels seeds nothing
    // at all; its pad is the standoff's own stop rather than the kernel's reach,
    // that being where the curve is windowed to zero
    // (`KernelTerm::reach_sigmas`, which is the one place the two are written
    // down). σ in the CELL is zero: what the blur chain does to such a cell is
    // a pass-through, which is what carries its coverage to the resolve.
    let shape = |c: &Caster, t: &harmonigraph_scene::KernelTerm| {
        let sigma = sigma_of(c, t);
        // A σ of zero asks for no blur at all, and `SIGMA_CELL_MAX / 0` is an
        // infinity the `min` answers: the cell is at the target's own
        // resolution and its kernel collapses to the centre tap.
        let fit = (SIGMA_CELL_MAX / sigma).min(1.0);
        // The floor is a `k` and not a fraction of the target, so it holds two
        // texels of a stem at every framing rather than at one.
        let floor = (DISTANCE_TEXELS_PER_POINT / px_per_point).min(1.0);
        let scale = if is_distance(t) { fit.max(floor) } else { fit };
        let k = scale * px_per_point;
        // This term's σ in the cell's own texels, which is what the PADDING is
        // in whatever the kind — the two families reach different multiples of
        // it and `KernelTerm::reach_sigmas` is the one place that is written
        // down.
        let texels = sigma * scale;
        let pad = ((t.kind.reach_sigmas() * texels).ceil() + 1.0) / k;
        // The BLUR chain's σ, which a distance cell has none of: the chain
        // sweeps every cell and a σ of zero makes it a pass-through, which is
        // what carries the coverage a distance cell was filled with through to
        // the resolve.
        (scale, k, if is_distance(t) { 0.0 } else { texels }, pad)
    };
    // What a caster's level comes to where it is spent: a level at zero, or a
    // NaN out of the same corrupt blob, darkens nothing.
    let casts = |c: &Caster| c.level.clamp(0.0, 1.0) > 0.0;

    // One entry per (caster, term), the terms consecutive inside each caster —
    // the order every index below is in.
    let mut rects: Vec<[f32; 4]> = Vec::with_capacity(casters.len() * kernel.len());
    let mut sizes: Vec<[u32; 2]> = Vec::with_capacity(casters.len() * kernel.len());
    for caster in casters {
        for term in kernel {
            // A caster that darkens nothing takes NO cell: a reader hands the
            // frame back whole at level 0, so the cells it would fill are ones
            // nothing ever samples. Nodes clipped off the pane and nodes
            // projected behind the eye arrive here at level 0 in numbers
            // (`node_caster`), and N cells each would widen the atlas every
            // blur pass sweeps and be rasterized by the whole node shader for
            // no picture at all.
            if !casts(caster) {
                rects.push([0.0; 4]);
                sizes.push([0, 0]);
                continue;
            }
            let (_, k, _, pad) = shape(caster, term);
            let r = caster.rect;
            let rect = [r[0] - pad, r[1] - pad, r[2] + 2.0 * pad, r[3] + 2.0 * pad];
            let texels = |points: f32| ((points * k).ceil() as u32).max(1);
            rects.push(rect);
            sizes.push([texels(rect[2]), texels(rect[3])]);
        }
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

    // ALL of a caster's cells or none of them. A kernel is mixed before it is
    // spent, so a caster drawn with one term missing is not a fainter shadow —
    // it is a different kernel, and a narrow one at that, drawn on whichever
    // casters happened to fall off the end of the atlas.
    let fits = |i: usize| {
        let [w, h] = sizes[i];
        let [x, y] = placed[i];
        w > 0 && h > 0 && x + w <= width && y + h <= height
    };
    let n = kernel.len();
    let mut boxes = Vec::with_capacity(rects.len());
    let mut packed_casters = Vec::with_capacity(casters.len());
    // How far the one chain has to carry a seed: the widest PACKED distance
    // cell's pad in its own texels. Taken over the cells that made it into the
    // atlas alone — a caster whose cells did not fit casts nothing, so a chain
    // sized for it would be passes over a cell nothing samples.
    let mut flood = 0.0f32;
    for (c, caster) in casters.iter().enumerate() {
        let whole = (0..n).all(|t| fits(c * n + t));
        let level = if whole { caster.level.clamp(0.0, 1.0) } else { 0.0 };
        let mut entry = NO_CASTER;
        entry.level[0] = level;
        let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
        for (t, term) in kernel.iter().enumerate() {
            let i = c * n + t;
            let (scale, k, sigma_cell, pad) = shape(caster, term);
            let kind = if is_distance(term) { DISTANCE_KIND } else { 0.0 };
            let rect = rects[i];
            let [w, h] = sizes[i];
            let [x, y] = placed[i];
            let cell = if whole { [x as f32, y as f32, w as f32, h as f32] } else { [0.0; 4] };
            boxes.push(ShadowBox {
                rect,
                cell,
                terms: [k, sigma_cell, level, scale],
                who: [c as f32, kind, pad, 0.0],
            });
            if !whole {
                continue;
            }
            if is_distance(term) {
                flood = flood.max(pad * k);
            }
            entry.kind[t] = kind;
            entry.sigma[t] = sigma_of(caster, term) / px_per_point;
            entry.cell[t] = cell;
            entry.map[t] = [cell[0] - rect[0] * k, cell[1] - rect[1] * k, k, weight(term)];
            for axis in 0..2 {
                lo[axis] = lo[axis].min(rect[axis]);
                hi[axis] = hi[axis].max(rect[axis] + rect[axis + 2]);
            }
        }
        if whole && hi[0] > lo[0] && hi[1] > lo[1] {
            entry.rect = [lo[0], lo[1], hi[0] - lo[0], hi[1] - lo[1]];
        }
        packed_casters.push(entry);
    }
    Packed { boxes, casters: packed_casters, size: [width, height], flood }
}

/// One pane's atlas: the two textures the blur ping-pongs between, and a bind
/// group reading each.
///
/// `views[0]` holds the casters' ink after the pre-pass and the finished blur
/// after [`blur`](Self::blur); `views[1]` is the half-blurred middle. Grown on
/// demand and never shrunk (`Offscreen::ensure_shadow`), on the pane's own
/// [`Offscreen`](crate::Offscreen) so two panes never share one.
pub(crate) struct ShadowTarget {
    pub(crate) size: [u32; 2],
    /// Kept only so a test can put ink in and read the blur back out
    /// (`a_cells_blur_stays_inside_its_own_cell_and_keeps_its_mass`). The
    /// `COPY_*` usages that needs are granted whatever the build, being a
    /// property of the texture rather than of the test.
    #[cfg(test)]
    pub(crate) textures: [wgpu::Texture; 2],
    pub(crate) views: [wgpu::TextureView; 2],
    /// Reading `views[i]`, as every consumer of the atlas takes it
    /// ([`read_layout`]).
    pub(crate) reads: [wgpu::BindGroup; 2],
    /// The jump flood's own pair, present exactly while a frame packs a
    /// DISTANCE cell ([`ensure_flood`](Self::ensure_flood)).
    ///
    /// Two more textures the size of the atlas, at four bytes a texel against
    /// the atlas's two — so a blur row does not carry them, which is what keeps
    /// the distance family free in a frame that does not draw one.
    pub(crate) flood: Option<FloodTarget>,
}

/// The jump flood's ping-pong pair, over one atlas.
///
/// `views[0]` holds the finished field: [`steps`] always runs an EVEN number of
/// passes, so the resolve binds one group rather than choosing by parity.
pub(crate) struct FloodTarget {
    /// The pair the chain ping-pongs between. Not read back directly by any
    /// test: what a test wants is the DISTANCE the resolve wrote, which is a
    /// texel of the atlas beside this and is the number the picture is drawn
    /// from — a field whose only check is a picture is a field whose errors go
    /// unnoticed, which is how #487's SPIKE lasted.
    views: [wgpu::TextureView; 2],
    /// Reading `views[i]`, for the pass that writes `views[i ^ 1]`.
    chain: [wgpu::BindGroup; 2],
    /// One [`Jump`] per step, at dynamic-offset stride. Rewritten every frame
    /// the chain runs: it is at most seventeen aligned cells against two
    /// atlas-sized textures beside it.
    jumps: wgpu::Buffer,
}

/// What one step's uniform holds: the jump, in `x`.
///
/// A whole `vec4<i32>` because that is what the uniform address space lays a
/// struct out on — `Jump` in shadow.wgsl, which has to be the same 16 bytes or
/// the pipeline is rejected at first paint for a `min_binding_size` it cannot
/// meet. Written [`JUMP_STRIDE`] apart, which is the alignment the BINDING
/// wants and a different number entirely.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Jump {
    step: [i32; 4],
}

/// The stride the jump buffer's dynamic offsets step by:
/// `min_uniform_buffer_offset_alignment`'s own guaranteed value, which every
/// backend wgpu targets meets. Sixteen bytes of [`Jump`] in each.
const JUMP_STRIDE: u64 = 256;

impl ShadowTarget {
    pub(crate) fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        size: [u32; 2],
    ) -> Self {
        let size = [size[0].max(1), size[1].max(1)];
        let texture = |label| {
            device.create_texture(&wgpu::TextureDescriptor {
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
            })
        };
        let textures = [texture("lattice_shadow_atlas"), texture("lattice_shadow_atlas_half")];
        let views = [
            textures[0].create_view(&Default::default()),
            textures[1].create_view(&Default::default()),
        ];
        let read = |view: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lattice_shadow_atlas_read"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            })
        };
        let reads = [read(&views[0]), read(&views[1])];
        ShadowTarget {
            size,
            #[cfg(test)]
            textures,
            views,
            reads,
            flood: None,
        }
    }

    /// Whether this atlas can hold a layout of `size`.
    pub(crate) fn holds(&self, size: [u32; 2]) -> bool {
        self.size[0] >= size[0] && self.size[1] >= size[1]
    }

    /// Hold the flood's pair while `want`, and drop it when not.
    ///
    /// Kept off the atlas's own allocation because a blur row never runs the
    /// chain and the pair is twice the atlas's bytes a texel: a frame on the
    /// Gaussian row pays exactly what it paid before this family existed, which
    /// is what makes the second family free to have in the tree.
    ///
    /// Sized to the atlas it belongs to, so it is rebuilt with the atlas and
    /// never separately — the two ping-pong over the same coordinates.
    pub(crate) fn ensure_flood(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        want: bool,
    ) {
        match (want, self.flood.is_some()) {
            (true, false) => self.flood = Some(FloodTarget::new(device, layout, self.size)),
            (false, true) => self.flood = None,
            _ => {}
        }
    }

    /// The pass that fills `views[0]` with the casters' ink: cleared, then the
    /// caller's draws.
    pub(crate) fn ink_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
    ) -> wgpu::RenderPass<'a> {
        Self::pass(encoder, "lattice_shadow_ink_pass", &self.views[0])
    }

    /// The jump flood over this frame's DISTANCE cells, carrying a seed
    /// `reach` texels and leaving the nearest-seed field in the pair's
    /// `views[0]`.
    ///
    /// Run between the ink pass and the blur, off the atlas's `views[0]`, which
    /// holds the casters' raw coverage until the blur's y pass overwrites it.
    /// Every draw here is over the cells and collapses a blur cell's quad, so a
    /// mixture of kinds costs the chain only what its distance cells are.
    ///
    /// A no-op with no pair held, which is a frame that packed no distance cell
    /// (`ensure_flood`).
    pub(crate) fn flood(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: (&wgpu::RenderPipeline, &wgpu::RenderPipeline),
        boxes: &wgpu::Buffer,
        count: u32,
        reach: f32,
    ) {
        let Some(flood) = self.flood.as_ref() else {
            return;
        };
        let (seed, step) = pipelines;
        let schedule = steps(reach);
        let jumps: Vec<u8> = schedule
            .iter()
            .flat_map(|&step| {
                let mut cell = [0u8; JUMP_STRIDE as usize];
                let jump = Jump { step: [step, 0, 0, 0] };
                cell[..std::mem::size_of::<Jump>()].copy_from_slice(bytemuck::bytes_of(&jump));
                cell
            })
            .collect();
        if !jumps.is_empty() {
            queue.write_buffer(&flood.jumps, 0, &jumps);
        }
        // The seed pass reads the atlas at group 0 and binds the chain's own
        // group only to satisfy the layout — it takes nothing out of it, and
        // naming the texture it is about to write would be a pass sampling its
        // own attachment.
        {
            let mut pass = Self::pass(encoder, "lattice_shadow_flood_seed", &flood.views[0]);
            pass.set_pipeline(seed);
            pass.set_bind_group(0, &self.reads[0], &[]);
            pass.set_bind_group(1, &flood.chain[1], &[0]);
            pass.set_vertex_buffer(0, boxes.slice(..));
            pass.draw(0..4, 0..count);
        }
        let mut src = 0usize;
        for (i, _) in schedule.iter().enumerate() {
            let mut pass = Self::pass(encoder, "lattice_shadow_flood_step", &flood.views[src ^ 1]);
            pass.set_pipeline(step);
            pass.set_bind_group(0, &self.reads[0], &[]);
            pass.set_bind_group(1, &flood.chain[src], &[(i as u64 * JUMP_STRIDE) as u32]);
            pass.set_vertex_buffer(0, boxes.slice(..));
            pass.draw(0..4, 0..count);
            src ^= 1;
        }
        debug_assert_eq!(src, 0, "the schedule is even, so the field lands in views[0]");
    }

    /// The two blur passes over `count` cells of `boxes`, leaving the finished
    /// atlas in `views[0]`.
    ///
    /// Both targets are cleared first: a cell's quad writes its own texels and
    /// no others, and what a fragment of the y pass reads beside its cell has
    /// to be nothing rather than last frame's cell there.
    ///
    /// THREE draws over two passes, and the third is what a distance cell's
    /// answer arrives by. The x pass sweeps every cell — a distance cell's σ is
    /// zero, so what it does there is copy the coverage into `views[1]`, which
    /// is where the resolve then reads the seed's own coverage from. The second
    /// pass then splits by kind: the y pass finishes the blur cells, and the
    /// resolve turns each distance cell's field into a distance in points.
    /// Both write `views[0]` and the two never touch one texel, the kinds being
    /// disjoint by construction.
    pub(crate) fn blur(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: (&wgpu::RenderPipeline, &wgpu::RenderPipeline),
        resolve: &wgpu::RenderPipeline,
        boxes: &wgpu::Buffer,
        count: u32,
    ) {
        let (blur_x, blur_y) = pipelines;
        {
            let mut pass = Self::pass(encoder, "lattice_shadow_blur_pass", &self.views[1]);
            pass.set_pipeline(blur_x);
            pass.set_bind_group(0, &self.reads[0], &[]);
            pass.set_vertex_buffer(0, boxes.slice(..));
            pass.draw(0..4, 0..count);
        }
        let mut pass = Self::pass(encoder, "lattice_shadow_blur_pass", &self.views[0]);
        pass.set_pipeline(blur_y);
        pass.set_bind_group(0, &self.reads[1], &[]);
        pass.set_vertex_buffer(0, boxes.slice(..));
        pass.draw(0..4, 0..count);
        if let Some(flood) = self.flood.as_ref() {
            pass.set_pipeline(resolve);
            pass.set_bind_group(0, &self.reads[1], &[]);
            // The FINISHED field, which the even schedule leaves in `views[0]`.
            pass.set_bind_group(1, &flood.chain[0], &[0]);
            pass.set_vertex_buffer(0, boxes.slice(..));
            pass.draw(0..4, 0..count);
        }
    }

    fn pass<'a>(
        encoder: &'a mut wgpu::CommandEncoder,
        label: &'static str,
        target: &'a wgpu::TextureView,
    ) -> wgpu::RenderPass<'a> {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        })
    }
}

impl FloodTarget {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, size: [u32; 2]) -> Self {
        let texture = |label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0].max(1),
                    height: size[1].max(1),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: SEED_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let textures = [texture("lattice_shadow_flood_0"), texture("lattice_shadow_flood_1")];
        let views = [
            textures[0].create_view(&Default::default()),
            textures[1].create_view(&Default::default()),
        ];
        let jumps = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lattice_shadow_flood_jumps"),
            size: JUMP_STRIDE * (MAX_LOG_STEP as u64 + 3),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = |src: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lattice_shadow_flood_chain"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &jumps,
                            offset: 0,
                            size: std::num::NonZeroU64::new(std::mem::size_of::<Jump>() as u64),
                        }),
                    },
                ],
            })
        };
        let chain = [group(&views[0]), group(&views[1])];
        FloodTarget { views, chain, jumps }
    }
}

/// The bindings a flood pass takes at GROUP 1: the field it reads and the jump
/// it takes.
///
/// A group of its own beside the atlas's rather than one layout carrying both,
/// because the three passes read different halves — the seed takes the atlas
/// alone, the step the field alone, the resolve both — and a slot cannot hold a
/// float texture and a uint one at once. One layout for the three, each reading
/// what it reads: what an entry point does not read is pruned before the
/// pipeline is built, so the seed carries the field at no cost.
pub(crate) fn flood_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("lattice_shadow_flood_layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<Jump>() as u64),
                },
                count: None,
            },
        ],
    })
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

/// What binds the casters' terms to the scene pipelines: one read-only storage
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

/// Every pipeline that sweeps the CELLS: the blur's two, the resolve that turns
/// a flooded cell into a distance, and the flood's own seed and step.
///
/// One module and one function for the five because they share the vertex
/// shader — a cell's quad, off the same box stream — and differ only in what
/// they read and what they write. No blend anywhere here: each writes its
/// cell's texels outright over a cleared target, a later answer being simply
/// the better one rather than something to mix.
pub(crate) struct CellPipelines {
    pub(crate) blur_x: wgpu::RenderPipeline,
    pub(crate) blur_y: wgpu::RenderPipeline,
    pub(crate) resolve: wgpu::RenderPipeline,
    pub(crate) seed: wgpu::RenderPipeline,
    pub(crate) step: wgpu::RenderPipeline,
}

pub(crate) fn create_cell_pipelines(
    device: &wgpu::Device,
    atlas: &wgpu::BindGroupLayout,
    flood: &wgpu::BindGroupLayout,
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
    // The atlas alone for the blur, both for the resolve, and the chain alone
    // for the step — which still declares group 0 so the chain sits at 1 in
    // every one of them and the shader spells one set of `@group` numbers.
    let blur_layout = layout("lattice_shadow_blur_pipeline_layout", &[Some(atlas)]);
    let both_layout = layout("lattice_shadow_resolve_pipeline_layout", &[Some(atlas), Some(flood)]);
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
        // Every cell: a distance cell's σ is 0 and this copies its coverage
        // into the half-blur target, which is where the resolve reads it.
        blur_x: pipeline("fs_blur_x", "vs_cell", &blur_layout, ATLAS_FORMAT),
        blur_y: pipeline("fs_blur_y", "vs_cell_blur", &blur_layout, ATLAS_FORMAT),
        resolve: pipeline("fs_flood_resolve", "vs_cell_distance", &both_layout, ATLAS_FORMAT),
        seed: pipeline("fs_flood_seed", "vs_cell_distance", &both_layout, SEED_FORMAT),
        step: pipeline("fs_flood_step", "vs_cell_distance", &both_layout, SEED_FORMAT),
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
        Caster { rect: [x, y, w, h], level: 1.0, sigma_scale: 1.0 }
    }

    /// One Gaussian: one cell per caster, which is what every claim about the
    /// packing that is not about the MIXTURE is asked at. The rows with more
    /// terms in them name themselves.
    fn one() -> &'static [harmonigraph_scene::KernelTerm] {
        harmonigraph_scene::ShadowKernel::Gaussian.terms()
    }

    /// Every row of the kernel table is a mixture the atlas and the sampler are
    /// built to hold, and every one is scaled to the same reach.
    ///
    /// The table is arithmetic nothing else checks: a row is four numbers typed
    /// out of a least-squares fit, and a transposed digit in one draws a
    /// perfectly plausible shadow of the wrong width. Three things say a row is
    /// the shape it claims. Its weights sum to 1, so switching rows does not
    /// change how DARK the shadow is — the mix is normalized where it is read,
    /// which turns a bad sum into a silent brightness change rather than an
    /// error. Its σ ratios straddle 1, that being what "scaled to the same
    /// reach" means for a mixture: a row entirely under 1 is a shadow narrower
    /// than the bar says. And it fits in what a caster carries.
    ///
    /// Walked off the ENUM and not off a list, so a row added to the table is
    /// checked by having been added. The rows the claims are about are the ones
    /// that carry a blur term, and `floods` is what parts them — a distance row
    /// has no mixture to sum and is measured by the two readings in
    /// `lattice_tests::shadows` instead.
    #[test]
    fn every_kernel_row_is_a_mixture_of_the_width_the_bar_names() {
        use harmonigraph_scene::ShadowKernel::*;
        for kernel in [Gaussian, TwoScale, Distance] {
            if kernel.floods() {
                continue;
            }
            let terms = kernel.terms();
            assert!(
                !terms.is_empty() && terms.len() <= harmonigraph_scene::SHADOW_TERMS_MAX,
                "{kernel:?} has {} terms, which is not something a caster can carry",
                terms.len(),
            );
            let total: f32 = terms.iter().map(|t| t.weight).sum();
            assert!(
                (total - 1.0).abs() < 0.005,
                "{kernel:?}'s weights sum to {total}, so the row is a brightness change as well \
                 as a shape",
            );
            assert!(
                terms.iter().all(|t| t.weight > 0.0 && t.sigma > 0.0),
                "{kernel:?} carries a term that draws nothing: {terms:?}",
            );
            let widest = terms.iter().fold(0.0f32, |w, t| w.max(t.sigma));
            let narrowest = terms.iter().fold(f32::INFINITY, |w, t| w.min(t.sigma));
            assert!(
                narrowest <= 1.0 && widest >= 1.0,
                "{kernel:?} spans {narrowest}..{widest}, which does not straddle the width the \
                 Shadow bar names",
            );
        }
        assert_eq!(
            Gaussian.terms(),
            &[harmonigraph_scene::KernelTerm {
                weight: 1.0,
                sigma: 1.0,
                kind: harmonigraph_scene::TermKind::Blur,
            }],
            "the fresh row is not one Gaussian at the bar's own width, so every other reading \
             in this suite is against a different picture",
        );
    }

    /// A mixture packs one cell per caster per term, each at the resolution its
    /// OWN σ asks for — which is the whole reason the shape is N cells rather
    /// than one blurred N ways.
    ///
    /// The claim to make is about the narrow term. A row's core is the term the
    /// eye reads as the shadow's edge, and a shared resolution picked for the
    /// widest term would draw that core through the wide term's texel grid and
    /// leave every row looking like the same blob. So: the narrow cell is drawn
    /// FINER than the wide one, and every cell's σ is still under the cap that
    /// holds the blur's tap count flat.
    #[test]
    fn a_mixtures_cells_are_each_at_their_own_terms_resolution() {
        let terms = harmonigraph_scene::ShadowKernel::TwoScale.terms();
        // Past the cap at every term, so each cell is really drawn smaller than
        // the pane and the resolutions below are the packer's own choice rather
        // than the pane's.
        let sigma = SIGMA_CELL_MAX * 40.0;
        let packed = pack(&[caster(0.0, 0.0, 40.0, 12.0)], sigma, 1.0, 16384, terms);
        assert_eq!(packed.boxes.len(), terms.len(), "one cell per term");
        assert_eq!(packed.casters.len(), 1, "one entry per caster, whatever the term count");
        for (t, term) in terms.iter().enumerate() {
            let b = packed.boxes[t];
            assert!(
                b.terms[1] <= SIGMA_CELL_MAX + 1e-4,
                "term {t} is {} texels of σ, past the cap the tap count rests on",
                b.terms[1],
            );
            assert!(b.cell[2] > 0.0 && b.cell[3] > 0.0, "term {t} got no cell");
            let expected = (SIGMA_CELL_MAX / (sigma * term.sigma)).min(1.0);
            assert!(
                (b.terms[3] - expected).abs() < 1e-5,
                "term {t} is drawn at {} of the target where its own σ asks for {expected}",
                b.terms[3],
            );
        }
        let (narrow, wide) = (packed.boxes[0], packed.boxes[terms.len() - 1]);
        assert!(
            narrow.terms[0] > wide.terms[0] * 2.0,
            "the narrow term is drawn at {} texels a point against the wide one's {}, which is \
             not a finer picture of the core",
            narrow.terms[0],
            wide.terms[0],
        );
        // And the caster's own entry spans the WIDEST term's box, which is the
        // quad the mix is drawn over: a quad on the narrow term's reach cuts
        // the wide one off in a straight line.
        let entry = packed.casters[0];
        for b in &packed.boxes {
            assert!(
                entry.rect[0] <= b.rect[0] + 1e-3
                    && entry.rect[1] <= b.rect[1] + 1e-3
                    && entry.rect[0] + entry.rect[2] >= b.rect[0] + b.rect[2] - 1e-3
                    && entry.rect[1] + entry.rect[3] >= b.rect[1] + b.rect[3] - 1e-3,
                "the caster's quad {:?} does not hold a term's box {:?}",
                entry.rect,
                b.rect,
            );
        }
        let weights: f32 = entry.map.iter().take(terms.len()).map(|m| m[3]).sum();
        assert!(
            (weights - 1.0).abs() < 1e-4,
            "the terms a caster carries sum to {weights} of a mixture, so its shadow is a \
             different darkness from a Gaussian's",
        );
    }

    /// A caster the atlas cannot hold ALL of casts nothing at all.
    ///
    /// All or none, and the trap is that the middle is plausible: a mixture with
    /// its wide term dropped is not a fainter shadow, it is the narrow rows of
    /// the table drawn on whichever casters happened to fall off the end of the
    /// shelves — a different kernel, chosen by the packing order.
    #[test]
    fn a_caster_the_atlas_cannot_hold_every_term_of_casts_none_of_it() {
        let terms = harmonigraph_scene::ShadowKernel::TwoScale.terms();
        // Each term's cell fits alone, while the pair cannot both be shelved in
        // this atlas. That reaches the partial-kernel branch rather than the
        // simpler case where the whole row fits.
        let packed = pack(&[caster(0.0, 0.0, 300.0, 300.0)], 40.0, 1.0, 96, terms);
        assert_eq!(packed.casters.len(), 1);
        assert_eq!(
            packed.casters[0].level[0], 0.0,
            "a caster missing a cell still casts, so some frame draws a kernel nobody chose",
        );
        assert!(
            packed.boxes.iter().all(|b| b.cell == [0.0; 4] && b.terms[2] == 0.0),
            "a caster that casts nothing kept a cell, which is ink drawn into the atlas origin \
             over whatever is packed there",
        );
    }

    /// What a row of the table costs in ATLAS, against one Gaussian, at both
    /// ends of the Shadow bar.
    ///
    /// The number `timing.rs` cannot give: a frame's GPU time on a contended
    /// machine swings by a factor of ten between its p10 and its p90, where the
    /// packing is arithmetic and answers the same way every run. It is also the
    /// cost that decides whether a row is affordable, the blur chain over the
    /// cells being unchanged — every cell's σ is capped in TEXELS, so N terms
    /// buy area rather than a wider kernel.
    ///
    /// Read at both ends because the two are not the same question. Cells are
    /// drawn at `min(1, SIGMA_CELL_MAX / σ)`, so at a NARROW Shadow the terms
    /// are all at the pane's own resolution and a row costs about N times one
    /// Gaussian; at a wide one each cell is scaled down by its own σ and the
    /// narrow term — the finest, and so the biggest — is what the row costs.
    ///
    /// The bound is four, just above the core-and-skirt row at both ends of the
    /// bar. Past it the row is a reason the atlas hits `max_side` rather than a
    /// shape to compare (see
    /// `a_node_close_to_the_eye_packs_a_cell_the_atlas_can_hold`).
    #[test]
    fn a_kernel_row_costs_this_much_atlas_against_one_gaussian() {
        use harmonigraph_scene::ShadowKernel::{Distance, Gaussian, TwoScale};
        // A pane's worth of names: a run of type is the caster the atlas is
        // mostly made of, and a node's box is the same shape at a bigger size.
        let casters: Vec<Caster> =
            (0..30).map(|i| caster(i as f32 * 3.0, 0.0, 40.0, 12.0)).collect();
        for (sigma, what) in [(6.0f32, "a fresh Shadow"), (60.0, "the top of the bar")] {
            // The CELLS' own area, not the texture's: a target is rounded up to
            // a power of two in each direction, which quantizes every reading
            // to a factor of two and hides what a row actually asked for.
            let area = |kernel: harmonigraph_scene::ShadowKernel| -> f64 {
                pack(&casters, sigma, 2.0, 16384, kernel.terms())
                    .boxes
                    .iter()
                    .map(|b| f64::from(b.cell[2]) * f64::from(b.cell[3]))
                    .sum()
            };
            let plain = area(Gaussian);
            assert!(plain > 0.0, "one Gaussian packed nothing at {what}");
            let ratio = area(TwoScale) / plain;
            eprintln!("TwoScale at {what}: {ratio:.2}x one Gaussian's cells");
            assert!(
                ratio <= 4.0,
                "TwoScale packs {ratio:.2}x one Gaussian's cells at {what}, which is a row that \
                 reaches the device's texture limit rather than a row to compare",
            );
            // The DISTANCE row on a bound of its own, and two orders of
            // magnitude above the blur rows' rather than beside them. A blur
            // cell shrinks with σ and a distance cell stops at
            // `DISTANCE_TEXELS_PER_POINT`, so the top of the bar is where the
            // two families' costs part company by construction — the number
            // measures 87x there, and the bound is a CEILING on that rather
            // than a claim it should be smaller. What it catches is the same
            // thing the blur-row bound above does: a change that walks the
            // atlas into `max_side`, where a caster stops casting with nothing
            // on screen to say so.
            let ratio = area(Distance) / plain;
            eprintln!("Distance at {what}: {ratio:.2}x one Gaussian's cells");
            assert!(
                ratio <= 120.0,
                "Distance packs {ratio:.2}x one Gaussian's cells at {what}, which is a row that \
                 reaches the device's texture limit rather than a row to compare",
            );
        }
    }

    /// The shader's term count is the scene crate's.
    ///
    /// Two files with no linkage: `SHADOW_TERMS` sizes the arrays a caster's
    /// entry is read out of, and `SHADOW_TERMS_MAX` sizes the ones the CPU
    /// writes. A shader smaller than the struct reads a term's cell out of the
    /// next term's slot, which is a shadow of the right shape in the wrong
    /// place.
    #[test]
    fn the_shaders_term_count_is_the_scenes() {
        let src = crate::with_common(crate::SHADER_SRC);
        assert!(
            src.contains(&format!(
                "const SHADOW_TERMS: u32 = {}u;",
                harmonigraph_scene::SHADOW_TERMS_MAX
            )),
            "common.wgsl declares a different SHADOW_TERMS than the struct a caster is written \
             into",
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
            let packed = pack(&[caster(10.0, 10.0, 40.0, 12.0)], sigma_px, 2.0, 16384, one());
            let b = packed.boxes[0];
            assert!(
                b.terms[1] <= SIGMA_CELL_MAX + 1e-4,
                "σ {sigma_px} px packed a cell at σ {} texels",
                b.terms[1]
            );
            // Under the cap the cell is drawn at the target's own pixels;
            // over it, exactly at the cap.
            if sigma_px <= SIGMA_CELL_MAX {
                assert!((b.terms[0] - 2.0).abs() < 1e-5 && (b.terms[1] - sigma_px).abs() < 1e-4);
            } else {
                assert!((b.terms[1] - SIGMA_CELL_MAX).abs() < 1e-4);
            }
            // And the padding holds the whole kernel plus the sampling texel.
            let pad_texels = (REACH_SIGMAS * b.terms[1]).ceil() + 1.0;
            let grown = (b.rect[2] - 40.0) * 0.5 * b.terms[0];
            assert!((grown - pad_texels).abs() < 1e-3, "padded {grown} texels for {pad_texels}");
        }
    }

    /// A frame with no caster packs no cell and asks for no atlas; so does one
    /// whose Shadow is shut.
    #[test]
    fn a_frame_with_no_caster_packs_no_cell() {
        assert_eq!(pack(&[], 4.0, 2.0, 16384, one()), Packed::default());
        assert_eq!(
            pack(&[caster(0.0, 0.0, 10.0, 10.0)], 0.0, 2.0, 16384, one()),
            Packed::default()
        );
        assert_eq!(
            pack(&[caster(0.0, 0.0, 10.0, 10.0)], f32::NAN, 2.0, 16384, one()),
            Packed::default()
        );
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
        let packed = pack(&casters, 6.0, 2.0, 16384, one());
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
        assert_eq!(pack(&casters, 6.0, 2.0, 16384, one()), packed);
    }

    /// A cell past the texture limit is no cell, with its level zeroed so its
    /// box draws nothing, and the rest of the frame keeps its shadows.
    #[test]
    fn a_cell_the_atlas_cannot_hold_casts_nothing() {
        let casters: Vec<Caster> = (0..8).map(|_| caster(0.0, 0.0, 100.0, 100.0)).collect();
        let packed = pack(&casters, 2.0, 1.0, 256, one());
        assert_eq!(packed.size, [256, 256]);
        let cast: Vec<bool> = packed.boxes.iter().map(|b| b.terms[2] > 0.0).collect();
        assert!(cast.iter().any(|&c| c), "nothing fit an atlas that holds four cells");
        assert!(!cast.iter().all(|&c| c), "eight 100-pt cells fit a 256-texel atlas");
        for b in packed.boxes.iter().filter(|b| b.terms[2] == 0.0) {
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
        let without = pack(&[live, live], 2.0, 1.0, 4096, one());
        let with_dead = pack(&[live, dead, live], 2.0, 1.0, 4096, one());
        assert_eq!(with_dead.boxes[1].cell, [0.0; 4], "the dead caster took a cell");
        assert_eq!(with_dead.boxes[1].terms[2], 0.0, "the dead caster kept a level");
        assert_eq!(with_dead.size, without.size, "the dead caster sized the atlas");
        assert_eq!(
            [with_dead.boxes[0].cell, with_dead.boxes[2].cell],
            [without.boxes[0].cell, without.boxes[1].cell],
            "the dead caster moved a live cell",
        );
    }

    /// σ is half the Shadow's width in target pixels, and shut for a width or
    /// a node radius that is not a positive number.
    #[test]
    fn sigma_is_half_the_shadow_in_target_pixels() {
        assert!((sigma_px(0.2, 30.0, 2.0, 1.5) - 9.0).abs() < 1e-5);
        assert_eq!(sigma_px(0.0, 30.0, 2.0, 1.0), 0.0);
        assert_eq!(sigma_px(0.2, 0.0, 2.0, 1.0), 0.0);
        assert_eq!(sigma_px(f32::NAN, 30.0, 2.0, 1.0), 0.0);
        assert_eq!(sigma_px(-1.0, 30.0, 2.0, 1.0), 0.0);
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
        let packed = pack(
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
        let target = ShadowTarget::new(&device, &layout, &sampler, size);
        let pipelines = create_cell_pipelines(&device, &layout, &flood_layout(&device));

        // The block: cell A's ink region — everything inside its padding —
        // filled solid, as a caster wider than the blur is.
        let pad = ((REACH_SIGMAS * a.terms[1]).ceil() + 1.0) as u32;
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
            target.textures[0].as_image_copy(),
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
        target.blur(
            &mut encoder,
            (&pipelines.blur_x, &pipelines.blur_y),
            &pipelines.resolve,
            &boxes,
            2,
        );
        queue.submit([encoder.finish()]);
        let bytes = crate::gpu_harness::readback(&device, &queue, &target.textures[0], size);
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

    /// One distance cell filling a whole texture at one texel per point, with
    /// the flood's pair beside it — the fixture both readings below are taken
    /// on.
    fn one_distance_cell(
        device: &wgpu::Device,
        size: [u32; 2],
        pad: f32,
    ) -> (ShadowTarget, CellPipelines, wgpu::Buffer, ShadowBox) {
        let atlas_layout = read_layout(device);
        let chain_layout = flood_layout(device);
        let sampler = device.create_sampler(&Default::default());
        let mut target = ShadowTarget::new(device, &atlas_layout, &sampler, size);
        target.ensure_flood(device, &chain_layout, true);
        let pipelines = create_cell_pipelines(device, &atlas_layout, &chain_layout);
        let cell = ShadowBox {
            rect: [0.0, 0.0, size[0] as f32, size[1] as f32],
            cell: [0.0, 0.0, size[0] as f32, size[1] as f32],
            // One texel per point, no blur, level 1, full resolution: what the
            // resolve writes is then in the units the fixtures measure in.
            terms: [1.0, 0.0, 1.0, 1.0],
            who: [0.0, DISTANCE_KIND, pad, 0.0],
        };
        let boxes = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test_boxes"),
            size: std::mem::size_of::<ShadowBox>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        (target, pipelines, boxes, cell)
    }

    /// A float as the atlas holds it, which is [`half`] the other way round.
    fn half_bits(v: f32) -> u16 {
        let bits = v.to_bits();
        let exp = ((bits >> 23) & 0xff) as i32 - 127;
        let mant = ((bits >> 13) & 0x3ff) as u16;
        // Zero, and everything a half cannot hold as a NORMAL number. Only the
        // normal range is written out — a subnormal is under 6e-5 and every
        // caller here is coverage, where that is zero — and the guard is what
        // keeps the shortcut honest: `exp + 15` below -14 goes NEGATIVE, and a
        // negative exponent shifted into place wraps to a large negative half,
        // which is a fixture writing garbage rather than the small number it
        // meant.
        if !v.is_finite() || v < 6.104e-5 {
            return 0;
        }
        (((exp + 15) as u16) << 10) | mant
    }

    /// The flood answers the TRUE distance, everywhere inside its reach, for a
    /// pair of DIAGONAL strokes as far apart as two letters of a name.
    ///
    /// The measurement a sampled dilation could not pass, and one an
    /// axis-aligned fixture cannot make. A dilation sampled at N offsets is a
    /// binary dilation by a disc wherever the coverage is flat, and N taps on a
    /// spiral sit about `R / sqrt(N)` apart — at a reach of 32 texels, 48 of
    /// them leave gaps four texels wide between the copies of a two-texel
    /// stroke, and the shortfall draws the letters again inside their own
    /// shadow. What is asserted is what makes that impossible: an exact
    /// distance at every texel, at a cost that does not move with the reach.
    ///
    /// DIAGONAL, and that is the fixture's whole point. #487's SPIKE was a
    /// seed's coverage spent on the profile's HEIGHT rather than on the
    /// distance, and coverage is constant along an axis-aligned edge — so the
    /// same test on an upright stroke reads alike either way and passes for the
    /// wrong reason (#450). Along a diagonal the coverage runs the whole of
    /// `[INK_FLOOR, 1]` and the two readings part.
    ///
    /// Five hundredths of a texel of tolerance. A seed at its texel centre is
    /// off by nearly half along this diagonal; the reconstructed contour
    /// segment is what makes the tighter claim reachable.
    #[test]
    fn the_flood_answers_the_true_distance_between_two_strokes() {
        const SIZE: [u32; 2] = [256, 128];
        // Two diagonals, `SPLIT` apart along x and `2 * HALF` wide across it —
        // a name's two stems, and the gap a sampled dilation could not fill.
        const SPLIT: f32 = 40.0;
        const HALF: f32 = 1.0;
        const REACH: f32 = 32.0;

        let Some((device, queue)) = crate::gpu_harness::headless_device() else {
            return;
        };
        let (target, pipelines, boxes, cell) = one_distance_cell(&device, SIZE, 1.0e4);
        // The SIGNED distance to the nearer diagonal, each at 45° — negative
        // inside a stroke, which is what makes the coverage below a
        // rasterizer's rather than a half everywhere the ink is solid.
        let signed = |x: f32, y: f32| {
            let arm = |at: f32| (x - y - at).abs() / std::f32::consts::SQRT_2 - HALF;
            arm(0.0).min(arm(SPLIT))
        };
        let truth = |x: f32, y: f32| signed(x, y).max(0.0);
        // The coverage a rasterizer writes: the straight-edge approximation the
        // seed's own correction is derived against, one texel of ramp across
        // the contour.
        let mut ink = vec![0u8; (SIZE[0] * SIZE[1] * 2) as usize];
        for y in 0..SIZE[1] {
            for x in 0..SIZE[0] {
                let cov = (0.5 - signed(x as f32 + 0.5, y as f32 + 0.5)).clamp(0.0, 1.0);
                let at = ((y * SIZE[0] + x) * 2) as usize;
                ink[at..at + 2].copy_from_slice(&half_bits(cov).to_le_bytes());
            }
        }
        queue.write_texture(
            target.textures[0].as_image_copy(),
            &ink,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE[0] * 2),
                rows_per_image: Some(SIZE[1]),
            },
            wgpu::Extent3d { width: SIZE[0], height: SIZE[1], depth_or_array_layers: 1 },
        );
        queue.write_buffer(&boxes, 0, bytemuck::bytes_of(&cell));
        let mut encoder = device.create_command_encoder(&Default::default());
        target.flood(&queue, &mut encoder, (&pipelines.seed, &pipelines.step), &boxes, 1, REACH);
        target.blur(
            &mut encoder,
            (&pipelines.blur_x, &pipelines.blur_y),
            &pipelines.resolve,
            &boxes,
            1,
        );
        queue.submit([encoder.finish()]);
        let bytes = crate::gpu_harness::readback(&device, &queue, &target.textures[0], SIZE);
        let at = |x: u32, y: u32| -> f32 {
            let i = ((y * SIZE[0]) * 4 + x * 2) as usize;
            half(u16::from_le_bytes([bytes[i], bytes[i + 1]]))
        };

        // A margin of the reach off every edge. `truth` is the distance to an
        // ENDLESS diagonal, and a texel nearer the border than that has its
        // perpendicular foot outside the cell — where the flood is right and
        // the reference is not, the nearest ink actually present being further.
        let margin = REACH as u32 + 2;
        let (mut checked, mut worst, mut worst_at) = (0, 0.0f32, (0u32, 0u32));
        for y in margin..SIZE[1] - margin {
            for x in margin..SIZE[0] - margin {
                let want = truth(x as f32 + 0.5, y as f32 + 0.5);
                if want > REACH {
                    continue;
                }
                let err = (at(x, y) - want).abs();
                if err > worst {
                    worst = err;
                    worst_at = (x, y);
                }
                checked += 1;
            }
        }
        // The loop is the assertion, so a loop that ran over nothing is a green
        // test measuring an empty field.
        assert!(checked > 5000, "only {checked} texels stood inside the reach");
        assert!(
            worst <= 0.05,
            "the flood is off by {worst} texels at {worst_at:?}, inside its own reach",
        );
    }

    /// A seed with no coverage gradient remains a point, so its distance is
    /// isotropic rather than stretched along an arbitrary contour tangent.
    #[test]
    fn an_isolated_seeds_distance_is_the_same_in_every_direction() {
        const SIZE: [u32; 2] = [128, 64];
        const SEED: [u32; 2] = [64, 32];
        const REACH: f32 = 8.0;

        let Some((device, queue)) = crate::gpu_harness::headless_device() else {
            return;
        };
        let (target, pipelines, boxes, cell) = one_distance_cell(&device, SIZE, REACH);
        let mut ink = vec![0u8; (SIZE[0] * SIZE[1] * 2) as usize];
        let seed_at = ((SEED[1] * SIZE[0] + SEED[0]) * 2) as usize;
        ink[seed_at..seed_at + 2].copy_from_slice(&half_bits(1.0).to_le_bytes());
        queue.write_texture(
            target.textures[0].as_image_copy(),
            &ink,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE[0] * 2),
                rows_per_image: Some(SIZE[1]),
            },
            wgpu::Extent3d { width: SIZE[0], height: SIZE[1], depth_or_array_layers: 1 },
        );
        queue.write_buffer(&boxes, 0, bytemuck::bytes_of(&cell));
        let mut encoder = device.create_command_encoder(&Default::default());
        target.flood(&queue, &mut encoder, (&pipelines.seed, &pipelines.step), &boxes, 1, REACH);
        target.blur(
            &mut encoder,
            (&pipelines.blur_x, &pipelines.blur_y),
            &pipelines.resolve,
            &boxes,
            1,
        );
        queue.submit([encoder.finish()]);
        let bytes = crate::gpu_harness::readback(&device, &queue, &target.textures[0], SIZE);
        let at = |x: u32, y: u32| -> f32 {
            let i = ((y * SIZE[0]) * 4 + x * 2) as usize;
            half(u16::from_le_bytes([bytes[i], bytes[i + 1]]))
        };

        for (x, y) in [
            (SEED[0] - 2, SEED[1]),
            (SEED[0] + 2, SEED[1]),
            (SEED[0], SEED[1] - 2),
            (SEED[0], SEED[1] + 2),
        ] {
            let held = at(x, y);
            assert!(
                (held - 1.5).abs() < 1.0e-3,
                "the seed resolves to {held} texels at ({x}, {y}), not 1.5",
            );
        }
    }

    /// A texel out of every seed's reach resolves to the cell's own pad, which
    /// is where the standoff's curve is windowed to nothing.
    ///
    /// The claim is that "no ink within reach" and "further out than the shadow
    /// goes" are ONE answer. A sentinel of its own would be a value the
    /// sampler's bilinear tap interpolates toward, and a step in a distance
    /// field is a contour in the picture.
    #[test]
    fn a_texel_no_seed_reaches_resolves_to_the_cells_own_pad() {
        const SIZE: [u32; 2] = [64, 64];
        const PAD: f32 = 7.0;

        let Some((device, queue)) = crate::gpu_harness::headless_device() else {
            return;
        };
        let (target, pipelines, boxes, cell) = one_distance_cell(&device, SIZE, PAD);
        queue.write_buffer(&boxes, 0, bytemuck::bytes_of(&cell));
        let mut encoder = device.create_command_encoder(&Default::default());
        // The ink pass clears the atlas and nothing draws into it: a cell whose
        // coverage never reaches `INK_FLOOR` seeds nowhere at all.
        drop(target.ink_pass(&mut encoder));
        target.flood(&queue, &mut encoder, (&pipelines.seed, &pipelines.step), &boxes, 1, 8.0);
        target.blur(
            &mut encoder,
            (&pipelines.blur_x, &pipelines.blur_y),
            &pipelines.resolve,
            &boxes,
            1,
        );
        queue.submit([encoder.finish()]);
        let bytes = crate::gpu_harness::readback(&device, &queue, &target.textures[0], SIZE);
        for (x, y) in [(1u32, 1u32), (32u32, 32u32), (62u32, 62u32)] {
            let i = ((y * SIZE[0]) * 4 + x * 2) as usize;
            let held = half(u16::from_le_bytes([bytes[i], bytes[i + 1]]));
            assert!(
                (held - PAD).abs() < 1e-3,
                "an unseeded texel at ({x}, {y}) holds {held}, not the cell's pad of {PAD}",
            );
        }
    }

    /// The schedule reaches at least as far as it is asked to, and lands on an
    /// even number of passes so the resolve can name one texture.
    ///
    /// The reach is what a jump flood actually carries — the SUM of its jumps —
    /// rather than the first of them, which is the number it is easy to check
    /// and the wrong one: a chain starting at 32 with no tail carries 32, and
    /// the same chain down to 1 carries 63.
    #[test]
    fn a_floods_jumps_reach_past_the_cell_they_are_built_for() {
        for reach in [1.0f32, 2.0, 3.0, 31.0, 32.0, 33.0, 200.0, 4000.0] {
            let schedule = steps(reach);
            let carried: i32 = schedule.iter().sum();
            assert!(
                carried as f32 >= reach,
                "a chain {schedule:?} carries {carried} and is asked for {reach}",
            );
            assert_eq!(schedule.len() % 2, 0, "an odd chain lands the field in views[1]");
            assert_eq!(*schedule.last().expect("a step"), 1, "the tail is a local pass");
        }
    }

    /// A reach under a texel runs no flood, and a nonsense one stops at the cap
    /// rather than looping on a number no `log2` can answer for.
    #[test]
    fn a_reach_under_a_texel_or_past_every_atlas_runs_a_bounded_chain() {
        for reach in [0.0f32, 0.5, 0.999] {
            assert!(steps(reach).is_empty(), "a reach of {reach} asked for a flood");
        }
        for reach in [f32::INFINITY, f32::NAN, 1.0e30, -5.0] {
            let schedule = steps(reach);
            assert!(
                schedule.len() <= MAX_LOG_STEP as usize + 2,
                "a reach of {reach} asks for {} passes",
                schedule.len(),
            );
            assert_eq!(schedule.len() % 2, 0, "an odd chain lands the field in views[1]");
        }
    }

    /// The three modules that spell a distance term's kind agree on it, and the
    /// two that spell the standoff's window agree on that.
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

    /// The packed seed leaves one high bit for a point distinct from every
    /// coordinate and line direction, and one value past every real x for the
    /// sentinel.
    #[test]
    fn the_seed_fields_bits_hold_the_largest_atlas_a_point_and_the_sentinel() {
        let held = |name: &str| {
            shader_const(SHADOW_SRC, name)
                .trim_end_matches('u')
                .parse::<u32>()
                .unwrap_or_else(|_| panic!("shadow.wgsl's {name} is a u32 literal"))
        };
        assert_eq!(held("SEED_COORD_MASK") + 1, SEED_COORD_LIMIT);
        assert_eq!(held("SEED_POINT_BIT"), 2 * SEED_COORD_LIMIT);
        assert_eq!(held("NO_SEED"), 4 * SEED_COORD_LIMIT - 1);
    }

    /// A distance term's cell is padded to exactly where its curve is windowed
    /// to nothing, and floored in resolution so a stroke of type survives it.
    ///
    /// The pair the family's cost is decided by, and the two are opposite
    /// claims. The pad has to REACH the stop or the shadow ends in a straight
    /// line at the cell's edge; the scale has to stop shrinking or the ink the
    /// flood seeds off thins to nothing. A blur cell keeps neither property,
    /// which is what makes this the branch worth pinning.
    #[test]
    fn a_distance_cell_reaches_the_stop_and_holds_its_resolution() {
        use harmonigraph_scene::ShadowKernel;
        let caster = Caster { rect: [40.0, 40.0, 20.0, 20.0], level: 1.0, sigma_scale: 1.0 };
        // A σ well past `SIGMA_CELL_MAX`, which is where a blur cell shrinks
        // without limit and a distance cell stops.
        let sigma = 40.0;
        let packed = ShadowKernel::Distance.terms();
        let out = pack(&[caster], sigma, 1.0, 4096, packed);
        let cell = out.boxes[0];
        let scale = cell.terms[3];
        // At EVERY framing, which is the half a fraction of the target cannot
        // hold: the editor draws at 2 pixels a point and an export at 1 to 4
        // (`default_scale` in harmonigraph-offline), so a floor fitted to one
        // of them is a shadow present on screen and gone from the mp4. What has
        // to hold still is the texels a POINT is drawn at.
        for px_per_point in [1.0f32, 1.5, 2.0, 4.0] {
            let held = pack(&[caster], sigma * px_per_point, px_per_point, 8192, packed).boxes[0];
            let k = held.terms[0];
            assert!(
                k >= DISTANCE_TEXELS_PER_POINT - 1e-5,
                "at {px_per_point} pixels a point a distance cell draws a point across {k} \
                 texels, so a stem of type is under two of them and seeds nothing",
            );
        }
        assert_eq!(cell.terms[1], 0.0, "a distance cell carries a blur σ, so the chain blurs it");
        assert_eq!(cell.who[1], DISTANCE_KIND, "the box does not say it holds a distance");
        // The pad is the stop in points, plus the one texel the sampler's
        // bilinear tap at the box's own edge needs.
        let want = 2.0 * harmonigraph_scene::SHADOW_STOP * sigma;
        let pad = cell.who[2];
        assert!(
            pad >= want && pad <= want + 1.0 / scale + 1.0,
            "a distance cell is padded {pad} points where its curve reaches {want}",
        );
        assert!(out.flood >= want * scale, "the chain is sized {} for a pad of {pad}", out.flood);
        // And the same row on the Gaussian's own terms floods nothing.
        let blur = pack(&[caster], sigma, 1.0, 4096, ShadowKernel::Gaussian.terms());
        assert_eq!(blur.flood, 0.0, "a blur row asked for a flood chain");
        assert_eq!(blur.boxes[0].who[1], 0.0, "a blur box says it holds a distance");
    }
}
