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

/// How many σ out a cell is padded, which is how far the kernel reaches —
/// `REACH` in shadow.wgsl, and the two have to agree or a blur is cut off in a
/// straight line at the cell's edge.
pub(crate) const REACH_SIGMAS: f32 = 3.0;

/// σ of a caster's blur in the target's pixels, for a Shadow of `shadow` node
/// radii over a node of `node_points` points, on a pane at `pixels_per_point`
/// drawn at `render_scale`.
///
/// HALF the bar's width. A half-plane blurred at σ keeps `erfc(d / (σ√2)) / 2`
/// of the light `d` out from its edge, which at `d = 2σ` is 2.3% — so one
/// Shadow width is where a wide caster's shadow has all but run out, which is
/// what the bar says it is. One bar, one reach, across a ring, a cross and a
/// name.
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
    /// [`Packed::casters`]. y/z/w unused.
    ///
    /// Read by the node's SCENE draw alone, which needs every term at once and
    /// so reaches the array rather than the box. Carried on the box rather than
    /// in a buffer of its own because the node's two draws — the one that fills
    /// a cell and the one that reads the atlas — bind the same stream, and one
    /// row is cheaper than a second binding.
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
}

/// One caster's whole kernel, as the scene pass reads it: the quad to draw over
/// and every term's cell and mapping.
///
/// In a STORAGE BUFFER rather than beside the instance, which is the one place
/// this design departs from #527's sketch. A node's own rows reach location 15
/// and leave five free; four terms need eight, so the cells cannot ride the
/// node's vertex stream at all. Indexed by the caster's own index — the order
/// `pack` was handed — so a node, a name and the marker field all reach it the
/// same way and nothing carries a second copy.
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
    /// Each term's cell in atlas texels: origin, then size. Zeroed past the
    /// kernel's own term count, and zeroed whole where the caster casts
    /// nothing.
    pub cell: [[f32; 4]; harmonigraph_scene::SHADOW_TERMS_MAX],
    /// Each term's map from a point of the pane to a texel of its cell: x/y the
    /// origin, z the scale, so a texel is `xy + points * z`. w unused.
    ///
    /// Pre-composed on the CPU rather than sent as (cell origin, box origin,
    /// scale) for the shader to combine: the three differ per TERM, and a
    /// fragment mixing four of them would do the same subtraction four times
    /// for a number that never changes within a frame.
    pub map: [[f32; 4]; harmonigraph_scene::SHADOW_TERMS_MAX],
}

/// A caster the frame packed nothing for: what every draw carries when the
/// Shadow is shut, and what a reader answers 1 to with nothing sampled.
pub(crate) const NO_CASTER: ShadowCaster = ShadowCaster {
    rect: [0.0; 4],
    level: [0.0; 4],
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
    // added to the table draws its first four terms rather than reading past
    // the array a caster carries.
    let kernel = &kernel[..kernel.len().min(harmonigraph_scene::SHADOW_TERMS_MAX)];
    // NORMALIZED where the row is read, so a table that sums to 1.001 is a
    // rounding in the fit rather than a tenth of a percent of extra darkness,
    // and a row of zeros is no shadow rather than a division.
    let total: f32 = kernel.iter().map(|t| t.weight.max(0.0)).sum();
    let weight = |t: &harmonigraph_scene::KernelTerm| {
        if positive(total) {
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
    let shape = |c: &Caster, t: &harmonigraph_scene::KernelTerm| {
        let sigma = sigma_of(c, t);
        // A σ of zero asks for no blur at all, and `SIGMA_CELL_MAX / 0` is an
        // infinity the `min` answers: the cell is at the target's own
        // resolution and its kernel collapses to the centre tap.
        let scale = (SIGMA_CELL_MAX / sigma).min(1.0);
        let k = scale * px_per_point;
        let sigma_cell = sigma * scale;
        let pad = ((REACH_SIGMAS * sigma_cell).ceil() + 1.0) / k;
        (scale, k, sigma_cell, pad)
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
    for (c, caster) in casters.iter().enumerate() {
        let whole = (0..n).all(|t| fits(c * n + t));
        let level = if whole { caster.level.clamp(0.0, 1.0) } else { 0.0 };
        let mut entry = NO_CASTER;
        entry.level[0] = level;
        let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
        for (t, term) in kernel.iter().enumerate() {
            let i = c * n + t;
            let (scale, k, sigma_cell, _) = shape(caster, term);
            let rect = rects[i];
            let [w, h] = sizes[i];
            let [x, y] = placed[i];
            let cell = if whole { [x as f32, y as f32, w as f32, h as f32] } else { [0.0; 4] };
            boxes.push(ShadowBox {
                rect,
                cell,
                terms: [k, sigma_cell, level, scale],
                who: [c as f32, 0.0, 0.0, 0.0],
            });
            if !whole {
                continue;
            }
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
    Packed { boxes, casters: packed_casters, size: [width, height] }
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
}

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
        }
    }

    /// Whether this atlas can hold a layout of `size`.
    pub(crate) fn holds(&self, size: [u32; 2]) -> bool {
        self.size[0] >= size[0] && self.size[1] >= size[1]
    }

    /// The pass that fills `views[0]` with the casters' ink: cleared, then the
    /// caller's draws.
    pub(crate) fn ink_pass<'a>(
        &'a self,
        encoder: &'a mut wgpu::CommandEncoder,
    ) -> wgpu::RenderPass<'a> {
        Self::pass(encoder, "lattice_shadow_ink_pass", &self.views[0])
    }

    /// The two blur passes over `count` cells of `boxes`, leaving the finished
    /// atlas in `views[0]`.
    ///
    /// Both targets are cleared first: a cell's quad writes its own texels and
    /// no others, and what a fragment of the y pass reads beside its cell has
    /// to be nothing rather than last frame's cell there.
    pub(crate) fn blur(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: (&wgpu::RenderPipeline, &wgpu::RenderPipeline),
        boxes: &wgpu::Buffer,
        count: u32,
    ) {
        let (blur_x, blur_y) = pipelines;
        for (target, read, pipeline) in
            [(&self.views[1], &self.reads[0], blur_x), (&self.views[0], &self.reads[1], blur_y)]
        {
            let mut pass = Self::pass(encoder, "lattice_shadow_blur_pass", target);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, read, &[]);
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

/// The blur's two pipelines, x then y. No blend: each writes its cell's texels
/// outright over a cleared target.
pub(crate) fn create_blur_pipelines(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shadow_shader"),
        source: wgpu::ShaderSource::Wgsl(SHADOW_SRC.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_shadow_blur_pipeline_layout"),
        bind_group_layouts: &[Some(layout)],
        ..Default::default()
    });
    let pipeline = |entry: &str| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_cell"),
                compilation_options: Default::default(),
                buffers: &[ShadowBox::LAYOUT],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: ATLAS_FORMAT,
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
    (pipeline("fs_blur_x"), pipeline("fs_blur_y"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

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
    #[test]
    fn every_kernel_row_is_a_mixture_of_the_width_the_bar_names() {
        use harmonigraph_scene::ShadowKernel::*;
        for kernel in [Gaussian, TwoScale, Sky, Exponential] {
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
            let narrowest = terms.iter().fold(f32::INFINITY, |w, t| w.min(t.sigma));
            assert!(
                narrowest <= 1.0 && kernel.widest() >= 1.0,
                "{kernel:?} spans {narrowest}..{}, which does not straddle the width the Shadow \
                 bar names",
                kernel.widest(),
            );
        }
        assert_eq!(
            Gaussian.terms(),
            &[harmonigraph_scene::KernelTerm { weight: 1.0, sigma: 1.0 }],
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
        let terms = harmonigraph_scene::ShadowKernel::Sky.terms();
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
        let terms = harmonigraph_scene::ShadowKernel::Sky.terms();
        // Small enough that the narrow term's cell — the biggest, its
        // resolution being the highest — cannot be shelved, while the wide
        // term's still could.
        let packed = pack(&[caster(0.0, 0.0, 300.0, 300.0)], 40.0, 1.0, 128, terms);
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
    /// The bound is eight, which is where a three-term row stops being a lab
    /// setting and starts being a reason the atlas hits `max_side` (see
    /// `a_node_close_to_the_eye_packs_a_cell_the_atlas_can_hold`, which reaches
    /// that limit N times sooner now).
    #[test]
    fn a_kernel_row_costs_this_much_atlas_against_one_gaussian() {
        use harmonigraph_scene::ShadowKernel::{Exponential, Gaussian, Sky, TwoScale};
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
            for kernel in [TwoScale, Sky, Exponential] {
                let ratio = area(kernel) / plain;
                eprintln!("{kernel:?} at {what}: {ratio:.2}x one Gaussian's cells");
                assert!(
                    ratio <= 8.0,
                    "{kernel:?} packs {ratio:.2}x one Gaussian's cells at {what}, which is a \
                     row that reaches the device's texture limit rather than a row to compare",
                );
            }
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
        let pipelines = create_blur_pipelines(&device, &layout);

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
        target.blur(&mut encoder, (&pipelines.0, &pipelines.1), &boxes, 2);
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
}
