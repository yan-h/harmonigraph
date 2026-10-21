//! A spectral text pane's own shadow atlas: its names' cells, packed, inked
//! and blurred in the pane's `prepare`, and read back by its shadow composite
//! in `paint`, which egui runs after every callback has prepared.
//!
//! Text is the only thing on a spectral surface that casts into an atlas (the
//! roll's ribbons and the Spiral's dots cut their backdrop straight off their
//! own distance, `crate::surround`), so each pane holds its atlas the way it
//! holds its glyphs, and nothing is assembled after it.

use crate::{create_vertex_buffer, shadow, wgpu};

const INITIAL_CAPACITY: usize = 16;

/// What every casting pane shares: the layouts its atlas and casters are
/// bound through, the atlas sampler, and the blur's two pipelines.
pub(crate) struct Shared {
    pub atlas: wgpu::BindGroupLayout,
    pub casters: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    blur: shadow::CellPipelines,
}

impl Shared {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let atlas = shadow::read_layout(device);
        let casters = shadow::caster_layout(device);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("spectral_shadow_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let blur = shadow::create_cell_pipelines(device, &atlas);
        Shared { atlas, casters, sampler, blur }
    }
}

/// The draw that fills the pane's cells with its glyphs: the cell pipeline,
/// the pane's own bind group (whose uniforms carry the atlas size this
/// returns), and its glyph buffer.
pub(crate) struct Ink<'a> {
    pub pipeline: &'a wgpu::RenderPipeline,
    pub locals: &'a wgpu::BindGroup,
    pub glyphs: &'a wgpu::Buffer,
}

/// The cells packed last frame, for the tests that size them.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CellStats {
    pub cells: u32,
    /// Their total area in atlas texels.
    pub cell_texels: u64,
}

/// One casting pane's shadow atlas and the buffers that describe it.
pub(crate) struct PaneShadow {
    /// Each glyph's own name's `ShadowBox`, beside the pane's glyph buffer at
    /// the same capacity: what the cell fill and the shadow composite read the
    /// name's cell from, as the lattice's `cell_buffer` is.
    cells: wgpu::Buffer,
    cell_capacity: usize,
    boxes: wgpu::Buffer,
    box_capacity: usize,
    casters: wgpu::Buffer,
    caster_bind: wgpu::BindGroup,
    caster_capacity: usize,
    /// None while no name packed a cell, which is a pane that draws no shadow.
    target: Option<shadow::ShadowTarget>,
    #[cfg(test)]
    pub(crate) stats: CellStats,
}

impl PaneShadow {
    pub(crate) fn new(device: &wgpu::Device, shared: &Shared, glyph_capacity: usize) -> Self {
        let (casters, caster_bind) =
            shadow::caster_buffer(device, &shared.casters, INITIAL_CAPACITY);
        PaneShadow {
            cells: cell_buffer(device, glyph_capacity),
            cell_capacity: glyph_capacity,
            boxes: create_vertex_buffer::<shadow::ShadowBox>(
                device,
                "spectral_shadow_boxes",
                INITIAL_CAPACITY,
            ),
            box_capacity: INITIAL_CAPACITY,
            casters,
            caster_bind,
            caster_capacity: INITIAL_CAPACITY,
            target: None,
            #[cfg(test)]
            stats: CellStats::default(),
        }
    }

    /// Pack `casters`, one per name of `names` glyphs each, fill their cells
    /// with `ink` and blur them, recording into `encoder`. Returns the atlas
    /// size the pane's uniforms must carry for `vs_glyph_cell` and the
    /// composite to find the cells.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        shared: &Shared,
        pixels_per_point: f32,
        casters: &[shadow::Caster],
        names: &[u32],
        glyph_capacity: usize,
        ink: Ink,
    ) -> [f32; 2] {
        if glyph_capacity > self.cell_capacity {
            self.cells = cell_buffer(device, glyph_capacity);
            self.cell_capacity = glyph_capacity;
        }
        let packed = shadow::pack(
            casters,
            pixels_per_point.max(f32::EPSILON),
            device.limits().max_texture_dimension_2d,
        );
        if packed.boxes.len() > self.box_capacity {
            self.box_capacity = packed.boxes.len().next_power_of_two();
            self.boxes = create_vertex_buffer::<shadow::ShadowBox>(
                device,
                "spectral_shadow_boxes",
                self.box_capacity,
            );
        }
        if packed.casters.len() > self.caster_capacity {
            self.caster_capacity = packed.casters.len().next_power_of_two();
            (self.casters, self.caster_bind) =
                shadow::caster_buffer(device, &shared.casters, self.caster_capacity);
        }
        if !packed.boxes.is_empty() {
            queue.write_buffer(&self.boxes, 0, bytemuck::cast_slice(&packed.boxes));
            queue.write_buffer(&self.casters, 0, bytemuck::cast_slice(&packed.casters));
        }

        // Each glyph beside its own name's box: the boxes repeated by their
        // names' lengths, as the lattice writes its `cell_buffer`. The name's
        // first glyph is marked as the one that draws the name's shadow
        // (`vs_spectral_shadow`), so a layer's composite is one instanced draw.
        let rows = packed.boxes.iter().zip(names).flat_map(|(b, &n)| {
            let lead = shadow::ShadowBox { who: [b.who[0], 1.0, b.who[2], b.who[3]], ..*b };
            (0..n).map(move |i| if i == 0 { lead } else { *b })
        });
        let glyphs = names.iter().map(|&n| n as usize).sum();
        crate::write_rows::<_, { crate::BOX_BYTES }>(queue, &self.cells, glyphs, rows);

        let packed_cells = packed.boxes.iter().filter(|b| b.cell[2] > 0.0 && b.cell[3] > 0.0);
        #[cfg(test)]
        {
            self.stats = CellStats {
                cells: packed_cells.clone().count() as u32,
                cell_texels: packed_cells.clone().map(|b| (b.cell[2] * b.cell[3]) as u64).sum(),
            };
        }
        if packed_cells.count() == 0 {
            self.target = None;
            return [1.0, 1.0];
        }
        shadow::ShadowTarget::ensure(
            &mut self.target,
            device,
            &shared.atlas,
            &shared.sampler,
            packed.size,
            "spectral",
        );
        let target = self.target.as_ref().expect("ensured above");
        {
            let mut pass = target.ink_pass(encoder);
            pass.set_pipeline(ink.pipeline);
            pass.set_bind_group(0, ink.locals, &[]);
            pass.set_vertex_buffer(0, ink.glyphs.slice(..));
            pass.set_vertex_buffer(1, self.cells.slice(..));
            pass.draw(0..4, 0..glyphs as u32);
        }
        target.blur(
            encoder,
            (&shared.blur.blur_x, &shared.blur.blur_y),
            &self.boxes,
            packed.boxes.len() as u32,
        );
        target.size.map(|v| v as f32)
    }

    /// What the shadow composite binds: the finished atlas at group 2, the
    /// casters at group 3, and the per-glyph cells at vertex slot 1. None
    /// when nothing cast this frame.
    pub(crate) fn binding(&self) -> Option<(&wgpu::BindGroup, &wgpu::BindGroup, &wgpu::Buffer)> {
        let target = self.target.as_ref()?;
        Some((target.read(), &self.caster_bind, &self.cells))
    }

    #[cfg(test)]
    pub(crate) fn target_allocated(&self) -> bool {
        self.target.is_some()
    }
}

fn cell_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    create_vertex_buffer::<shadow::ShadowBox>(device, "text_shadow_cells", capacity)
}
