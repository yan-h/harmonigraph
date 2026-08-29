//! The shadow atlas: every caster's ink blurred into a cell of its own, which
//! is what that caster multiplies the frame by in its own draw
//! (`fs_shadow_box` in text.wgsl).
//!
//! One cell per caster, at a scale that keeps the blur's cost flat: a cell is
//! the caster's box grown by the blur's reach, drawn at `min(1, 3 / σ)` of the
//! target's pixels, so σ is at most `SIGMA_CELL_MAX` texels in every cell and
//! the kernel at most nineteen taps whatever the Shadow bar says. The atlas is
//! about the names' own area at the fresh Shadow and shrinks as the bar widens.
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
/// the frame, and a tail quantized to 1/255 steps across a wide soft shadow
/// where the light under it has none — `GLOW_SHADE_FORMAT`'s argument, for the
/// same reader.
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
/// where the standoff curve the rings still cast on puts its own tail
/// (`exp(-SHADOW_TAIL)` at the edge in lattice.wgsl is 1.8%). One bar, one
/// reach, across a ring and a name.
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
/// (min, then size) and how much of its shadow lands, 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Caster {
    pub rect: [f32; 4],
    pub level: f32,
}

/// The caster a name's glyphs make: the box round every glyph's rect, and the
/// strength the name's rim colour carries — the one number a lattice name's
/// `rim` holds (`LABEL_SHADOW` in harmonigraph_ui), so a name easing in as the
/// marker under it eases out grows its shadow on the clock its ink arrives on.
///
/// A run with no ink in it — every rect empty — is a caster of nothing, with
/// its level zeroed rather than a box of infinities for the packer to size.
pub(crate) fn caster_of(glyphs: &[crate::GlyphInstance]) -> Caster {
    let (mut min, mut max) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for g in glyphs {
        for axis in 0..2 {
            min[axis] = min[axis].min(g.rect[axis]);
            max[axis] = max[axis].max(g.rect[axis] + g.rect[axis + 2]);
        }
    }
    if !(max[0] > min[0] && max[1] > min[1]) {
        return Caster { rect: [0.0; 4], level: 0.0 };
    }
    let level = glyphs.iter().map(|g| f32::from(g.rim[3]) / 255.0).fold(0.0, f32::max);
    Caster { rect: [min[0], min[1], max[0] - min[0], max[1] - min[1]], level }
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
    /// caster's level, 0..=1; w unused.
    pub terms: [f32; 4],
}

impl ShadowBox {
    pub(crate) const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ShadowBox>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4],
    };

    /// The same rows at the locations after a glyph's five, for the draw that
    /// rasterizes a glyph into its cell alongside `GlyphInstance::LAYOUT`.
    pub(crate) const BESIDE_GLYPHS: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ShadowBox>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![5 => Float32x4, 6 => Float32x4, 7 => Float32x4],
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
        attributes: &wgpu::vertex_attr_array![5 => Float32x4, 9 => Float32x4, 14 => Float32x4],
    };
}

/// A caster with no cell at all: what a draw carries when the frame packed
/// nothing, and what every reader of a box answers 1 to (`shadow_through` in
/// lattice.wgsl, `fs_shadow_box` in text.wgsl) — the frame left exactly whole,
/// with nothing sampled.
pub(crate) const NO_CELL: ShadowBox = ShadowBox { rect: [0.0; 4], cell: [0.0; 4], terms: [0.0; 4] };

/// A frame's cells, packed: one box per caster in the caster's own order, and
/// the atlas size that holds them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Packed {
    pub boxes: Vec<ShadowBox>,
    pub size: [u32; 2],
}

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
pub(crate) fn pack(casters: &[Caster], sigma_px: f32, px_per_point: f32, max_side: u32) -> Packed {
    // A finite positive number, which a NaN or an infinity out of a corrupt
    // blob is not: either is no shadow rather than a kernel of nothing.
    let positive = |x: f32| x.is_finite() && x > 0.0;
    if casters.is_empty() || !positive(sigma_px) || !positive(px_per_point) {
        return Packed::default();
    }
    let scale = (SIGMA_CELL_MAX / sigma_px).min(1.0);
    // Points to cell texels, and σ in the same texels.
    let k = scale * px_per_point;
    let sigma_cell = sigma_px * scale;
    // The kernel's reach, plus one texel so the scene pass's bilinear tap at
    // the box's own edge still lands inside the cell.
    let pad_cell = (REACH_SIGMAS * sigma_cell).ceil() + 1.0;
    let pad = pad_cell / k;
    let cells: Vec<([f32; 4], [u32; 2])> = casters
        .iter()
        .map(|c| {
            let rect =
                [c.rect[0] - pad, c.rect[1] - pad, c.rect[2] + 2.0 * pad, c.rect[3] + 2.0 * pad];
            let texels = |points: f32| ((points * k).ceil() as u32).max(1);
            (rect, [texels(rect[2]), texels(rect[3])])
        })
        .collect();
    // Wide enough for the widest cell and about square over the total area,
    // so a pane's worth of names packs into a few shelves rather than one.
    let widest = cells.iter().map(|(_, [w, _])| *w).max().unwrap_or(1);
    let area: u64 = cells.iter().map(|(_, [w, h])| u64::from(*w) * u64::from(*h)).sum();
    let square = ((area as f64 * 4.0 / 3.0).sqrt().ceil() as u32).max(1);
    let width = widest.max(square).next_power_of_two().min(max_side);
    let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
    let mut placed = Vec::with_capacity(cells.len());
    for &(_, [w, h]) in &cells {
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
    let boxes = casters
        .iter()
        .zip(&cells)
        .zip(&placed)
        .map(|((caster, &(rect, [w, h])), &[x, y])| {
            let fits = x + w <= width && y + h <= height;
            ShadowBox {
                rect,
                cell: if fits { [x as f32, y as f32, w as f32, h as f32] } else { [0.0; 4] },
                terms: [k, sigma_cell, if fits { caster.level.clamp(0.0, 1.0) } else { 0.0 }, 0.0],
            }
        })
        .collect();
    Packed { boxes, size: [width, height] }
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

    fn caster(x: f32, y: f32, w: f32, h: f32) -> Caster {
        Caster { rect: [x, y, w, h], level: 1.0 }
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

    /// σ never exceeds the cap in any cell, at any width of the Shadow: past
    /// the cap the cell shrinks rather than the kernel widening.
    #[test]
    fn a_cells_sigma_is_at_most_three_texels_at_every_shadow_width() {
        for sigma_px in [0.05f32, 0.5, 1.0, 2.9, 3.0, 3.1, 10.0, 100.0, 5000.0] {
            let packed = pack(&[caster(10.0, 10.0, 40.0, 12.0)], sigma_px, 2.0, 16384);
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
        assert_eq!(pack(&[], 4.0, 2.0, 16384), Packed::default());
        assert_eq!(pack(&[caster(0.0, 0.0, 10.0, 10.0)], 0.0, 2.0, 16384), Packed::default());
        assert_eq!(pack(&[caster(0.0, 0.0, 10.0, 10.0)], f32::NAN, 2.0, 16384), Packed::default());
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
        let packed = pack(&casters, 6.0, 2.0, 16384);
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
        assert_eq!(pack(&casters, 6.0, 2.0, 16384), packed);
    }

    /// A cell past the texture limit is no cell, with its level zeroed so its
    /// box draws nothing, and the rest of the frame keeps its shadows.
    #[test]
    fn a_cell_the_atlas_cannot_hold_casts_nothing() {
        let casters: Vec<Caster> = (0..8).map(|_| caster(0.0, 0.0, 100.0, 100.0)).collect();
        let packed = pack(&casters, 2.0, 1.0, 256);
        assert_eq!(packed.size, [256, 256]);
        let cast: Vec<bool> = packed.boxes.iter().map(|b| b.terms[2] > 0.0).collect();
        assert!(cast.iter().any(|&c| c), "nothing fit an atlas that holds four cells");
        assert!(!cast.iter().all(|&c| c), "eight 100-pt cells fit a 256-texel atlas");
        for b in packed.boxes.iter().filter(|b| b.terms[2] == 0.0) {
            assert_eq!(b.cell, [0.0; 4]);
        }
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
        let packed =
            pack(&[caster(0.0, 0.0, 80.0, 80.0), caster(0.0, 0.0, 80.0, 80.0)], SIGMA, 1.0, 4096);
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
