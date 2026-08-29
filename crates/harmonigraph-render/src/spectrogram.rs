//! The spectrogram's heatmap, read out of the aggregator's slab grid in the
//! fragment shader rather than out of a picture composed for it.
//!
//! The grid arrives as DATA — `capacity` slots of `bins` stored-dB bytes, one
//! slot per slab — and each fragment works out for itself which run of buckets
//! sits under it, what that run reads as, and what colour that is. So a pitch
//! zoom, a resize, a Level drag or a palette change moves uniforms and nothing
//! else: there is no picture to remake.
//!
//! The read lives once, in shaders/spectrogram.wgsl, each piece carrying the
//! constraint that pins it; this crate is handed the constants as data and
//! never learns what a bucket is. `harmonigraph-ui` folds the grid and derives
//! those constants — the row geometry's margin, the level mapping's affine —
//! and holds nothing that reads a slab.
//!
//! Four taps blended in GAMMA space per fragment, which is the filtering an
//! `Rgba8Unorm` egui texture gets — that is what makes this a port rather than
//! a new picture, and the vertex rule feeding those taps is `heatmap_mesh`'s.
//! egui's path also dithers the blended colour (`RendererOptions::dithering`,
//! on by default) where this one hands over the blend itself, which is where a
//! heatmap's single-level flips against a golden frame come from.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::{create_vertex_buffer, wgpu, EGUI_BLEND};

const SPECTROGRAM_SRC: &str = include_str!("shaders/spectrogram.wgsl");

/// Entry points the spectrogram shader must provide: the vertex stage, and the
/// fragment stage in each of the two shadings
/// [`create_spectrogram_pipeline`] picks between. Its entry point is assembled
/// from the shading, so a rename in the WGSL is a panic at pipeline creation
/// and nothing sooner.
#[cfg(test)]
pub(crate) const SPECTROGRAM_ENTRY_POINTS: &[&str] =
    &["vs_heatmap", "fs_heatmap_gamma", "fs_heatmap_linear"];

/// The stored-dB grid the shader reads: `capacity` slots of `bins` bytes, slab
/// `key` living in slot `key.rem_euclid(capacity)`.
///
/// The GPU copy is padded per slab (see [`slab_stride`]) and is otherwise
/// these bytes exactly — the aggregator's own store, not a picture built from
/// it.
#[derive(Clone)]
pub struct SpectrogramGrid {
    /// A new value forces the GPU copy to be rebuilt from [`run`](Self::run);
    /// the copy is keyed on `(generation, capacity, bins)` per `pane_id`.
    ///
    /// Bump it whenever a slab that is NOT named in
    /// [`dirty`](Self::dirty) stops matching what the slot holds — a refold, a
    /// gap, a backward jump — and whenever the caller has lost track of what
    /// the GPU holds. A stale slot is a wrong column and nothing on the CPU
    /// can see it.
    pub generation: u64,
    /// This handover's own number, echoed into [`uploaded`](Self::uploaded)
    /// once the writes below have been queued. Distinct per handover, where
    /// [`generation`](Self::generation) is deliberately not.
    pub serial: u64,
    /// The last [`serial`](Self::serial) a `prepare` finished, shared with the
    /// caller.
    ///
    /// A callback is not certain to run — egui drops one whose clip rect is
    /// empty — so this is the only evidence the caller has that the slots it
    /// believes are written really were. A caller computing its next delta
    /// against a run this never named is computing it against a buffer that
    /// never received it.
    pub uploaded: Arc<AtomicU64>,
    pub capacity: u32,
    pub bins: u32,
    /// The visible run: keys `first_key .. first_key + run.len() / bins`,
    /// contiguous, slab-major, bytes exactly as the aggregator holds them.
    pub first_key: i64,
    pub run: Arc<Vec<u8>>,
    /// Keys inside the run whose slot is written this frame — the steady
    /// state's delta, a slab or two. Ignored on the frame the copy is rebuilt,
    /// which writes every slab of the run.
    pub dirty: Vec<i64>,
}

/// The row read's scalars: the row geometry, the two arms and the level
/// mapping, as uniforms.
///
/// The constants ride in as data because this crate does not depend on
/// `harmonigraph-core` and must not start to.
#[derive(Clone)]
pub struct SpectrogramRead {
    /// MIDI at pitch fraction 0, and semitones across the visible range.
    pub min_midi: f32,
    pub span: f32,
    /// How far past the visible range the edge rows reach, in pitch fraction:
    /// one bucket, `(1 / BINS_PER_SEMITONE / span).min(0.5)`.
    pub margin: f32,
    /// Rows of the picture along pitch — the pane's pitch-axis device pixels.
    /// It decides both the row geometry and the pitch axis' own filtering.
    pub rows: u32,
    /// MIDI at bucket 0's lower edge, and buckets per semitone.
    pub spectrum_min_midi: f32,
    pub bins_per_semitone: f32,
    /// The level mapping, before its 0..1 clamp:
    /// `level0 + level_per_step * byte + level_per_midi * midi`.
    ///
    /// Derived on the CPU from `spectrogram_level_raw`, which is affine in
    /// both — never re-derived here, so the mapping has one definition and the
    /// window, the tilt and their pivot stay the pane's business.
    pub level0: f32,
    pub level_per_step: f32,
    pub level_per_midi: f32,
    /// `ROW_MEAN_STEPS`: stored steps the power mean falls per halving of the
    /// summed weight.
    pub mean_steps: f32,
    /// `ROW_WEIGHT`: the weight of a bucket `j` stored steps below its run's
    /// loudest.
    pub weight: Arc<[f32; 256]>,
}

/// The gradient sampled at equal level slices — `cell_color((i + 0.5) / n)` for
/// each of `n` — as opaque RGBA8 in gamma space, exactly the bytes `Color32`
/// carries.
///
/// A table and not a mapping, for the reason the CPU one is a table: a cell's
/// colour is otherwise a gamut bisection and a Newton solve, per fragment.
#[derive(Clone)]
pub struct SpectrogramShades {
    /// Changes when the table does; the GPU copy is re-uploaded on a new value.
    pub generation: u64,
    pub lut: Arc<Vec<[u8; 4]>>,
}

/// One corner of the heatmap's geometry — a triangle list, so the caller keeps
/// whatever split its own interpolation rule needs.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SpectrogramVertex {
    /// Screen position in egui points, the same convention
    /// [`crate::RollInstance`] takes.
    pub pos: [f32; 2],
    /// Position along the run in SLABS from the first visible slab's left
    /// edge: 0 is that edge, `n` the newest slab's right edge, `n - 0.5` the
    /// newest slab's centre. Interpolated per fragment, so a vertex sitting
    /// mid-bend rescales the whole image — split the mesh at any corner in the
    /// mapping rather than spanning it.
    pub slab: f32,
    /// Pitch fraction across the visible range: 0 at `min_midi`, 1 at
    /// `min_midi + span`. Interpolated per fragment.
    pub t: f32,
}

impl SpectrogramVertex {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<SpectrogramVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x2, // pos
            1 => Float32,   // slab
            2 => Float32,   // t
        ],
    };
}

/// Draw `vertices` (a triangle list) into `rect`. `pane_id` must be unique per
/// spectrogram shown in the same frame — each gets its own grid copy, which is
/// the expensive thing here, and the pipeline is shared.
pub fn spectrogram_paint_callback(
    rect: egui::Rect,
    vertices: Vec<SpectrogramVertex>,
    grid: SpectrogramGrid,
    read: SpectrogramRead,
    shades: SpectrogramShades,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        SpectrogramCallback { vertices, grid, read, shades, target_format, pane_id },
    )
}

/// Per-frame, per-pane draw data, built on the UI thread.
struct SpectrogramCallback {
    vertices: Vec<SpectrogramVertex>,
    grid: SpectrogramGrid,
    read: SpectrogramRead,
    shades: SpectrogramShades,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
}

/// Bytes one slab occupies in the grid buffer: `bins` rounded up to
/// `COPY_BUFFER_ALIGNMENT`.
///
/// A steady-state frame writes ONE slab on its own, and `write_buffer` takes a
/// whole multiple of four bytes at a multiple of four — so the padding is what
/// makes a slab individually writable, not a packing preference. The shader
/// addresses `slot * stride + bucket` and never reads the pad.
fn slab_stride(bins: u32) -> u32 {
    bins.next_multiple_of(4)
}

/// The slot slab `key` lives in, for a ring of `capacity` slots.
///
/// One definition, two readers: the scatter in [`SpectrogramCallback::prepare`]
/// places a slab by it, and `first_slot` in the uniforms is this for the run's
/// first key — the shader then walks FORWARD from there with the same modulus,
/// which is the same rule only because a run is read forward. `capacity` is
/// non-zero on every path that draws.
fn slot_of(key: i64, capacity: u32) -> u32 {
    key.rem_euclid(i64::from(capacity)) as u32
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpectrogramUniforms {
    origin_points: [f32; 2],
    viewport_points: [f32; 2],
    min_midi: f32,
    span: f32,
    margin: f32,
    spectrum_min_midi: f32,
    bins_per_semitone: f32,
    level0: f32,
    level_per_step: f32,
    level_per_midi: f32,
    mean_steps: f32,
    rows: u32,
    bins: u32,
    stride: u32,
    capacity: u32,
    first_slot: u32,
    run_slabs: u32,
    _pad: u32,
}

/// GPU objects cached across frames in egui-wgpu's `CallbackResources`.
struct SpectrogramResources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    panes: HashMap<u64, SpectrogramPane>,
    /// Counts `prepare` calls, which is what [`SpectrogramPane::last_seen`] is
    /// stamped with — a clock the callback already has, where a frame count
    /// would need one plumbed in.
    prepares: u64,
}

/// How many `prepare` calls a pane may go unseen before its buffers are
/// dropped.
///
/// A pane is prepared once per frame while it is on screen, so with the two
/// spectrograms that can be live at once this is about a second at 60 fps.
/// What is being held is the grid copy — up to 15.7 MB for a whole-song ring
/// at 4096 slabs — so a closed tab keeping one is worth a sweep, and a pane
/// hidden for a frame keeping one is worth not rebuilding.
const PANE_TTL_PREPARES: u64 = 120;

/// The grid's GPU copy and what it was built from. A new key rebuilds it from
/// the whole run; the same key patches only the dirty slabs.
struct GridBuffer {
    buffer: wgpu::Buffer,
    key: (u64, u32, u32),
}

impl GridBuffer {
    /// Whether this buffer is the right SHAPE for a grid of `capacity` slots
    /// of `bins` bytes — which is all its allocation depends on, the
    /// generation deciding only what is written into it.
    fn fits(&self, capacity: u32, bins: u32) -> bool {
        (self.key.1, self.key.2) == (capacity, bins)
    }
}

/// The gradient table's GPU copy: a `levels` x 1 texture read with
/// `textureLoad`, so the shader owns the blend and no sampler filters between
/// two entries of a table that is already sampled per level.
struct LutTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    levels: u32,
    generation: u64,
}

struct SpectrogramPane {
    uniform_buffer: wgpu::Buffer,
    weight_buffer: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize,
    count: u32,
    grid: Option<GridBuffer>,
    lut: Option<LutTexture>,
    /// Made with the grid buffer and the table, so it is remade whenever
    /// either is.
    bind_group: Option<wgpu::BindGroup>,
    /// The value of [`SpectrogramResources::prepares`] when this pane was last
    /// drawn.
    last_seen: u64,
}

/// Starting size of a pane's vertex buffer; it grows by `next_power_of_two`
/// when a frame overflows it. The mesh is a handful of quads.
const INITIAL_VERTEX_CAPACITY: usize = 64;

impl SpectrogramResources {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let buffer_entry = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let storage = wgpu::BufferBindingType::Storage { read_only: true };
        // The grid and the weights are read from `read_row` and nothing else,
        // so the fragment stage is the whole of their visibility. Naming the
        // vertex stage as well would ask the adapter for `VERTEX_STORAGE`,
        // which a downlevel backend need not have: the layout is then refused
        // and the pipeline never builds, for a binding no vertex shader reads.
        // Only the uniforms are wanted in both, by `vs_heatmap`'s projection.
        let vs_fs = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let fs = wgpu::ShaderStages::FRAGMENT;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spectrogram_bind_group_layout"),
            entries: &[
                buffer_entry(0, vs_fs, wgpu::BufferBindingType::Uniform),
                buffer_entry(1, fs, storage),
                buffer_entry(2, fs, storage),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: fs,
                    ty: wgpu::BindingType::Texture {
                        // Loaded, never sampled: the four taps and their blend
                        // are the shader's own.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        SpectrogramResources {
            pipeline: create_spectrogram_pipeline(device, target_format, &layout),
            layout,
            target_format,
            panes: HashMap::new(),
            prepares: 0,
        }
    }
}

impl SpectrogramPane {
    /// This pane's buffers, made on first sight of its id and stamped with
    /// `prepares` so [`SpectrogramPane::evict_unseen`] can tell a live pane
    /// from one whose tab was closed.
    fn get<'a>(
        panes: &'a mut HashMap<u64, SpectrogramPane>,
        device: &wgpu::Device,
        pane_id: u64,
        prepares: u64,
    ) -> &'a mut SpectrogramPane {
        let pane = panes.entry(pane_id).or_insert_with(|| SpectrogramPane {
            uniform_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("spectrogram_uniforms"),
                size: std::mem::size_of::<SpectrogramUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            weight_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("spectrogram_weights"),
                size: std::mem::size_of::<[f32; 256]>() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            vertex_buffer: create_vertex_buffer::<SpectrogramVertex>(
                device,
                "spectrogram_vertices",
                INITIAL_VERTEX_CAPACITY,
            ),
            vertex_capacity: INITIAL_VERTEX_CAPACITY,
            count: 0,
            grid: None,
            lut: None,
            bind_group: None,
            last_seen: prepares,
        });
        pane.last_seen = prepares;
        pane
    }

    /// Drop every pane that has not been drawn for [`PANE_TTL_PREPARES`].
    ///
    /// A spectrogram's id is its surface (the docked pane, the Render
    /// preview), and a closed tab simply stops calling back — there is no
    /// teardown to hang this on, so the panes still being prepared are the
    /// only evidence of which ones exist. Run from whichever pane IS
    /// preparing, so a lone survivor still clears the others.
    fn evict_unseen(panes: &mut HashMap<u64, SpectrogramPane>, prepares: u64) {
        panes.retain(|_, pane| prepares.saturating_sub(pane.last_seen) < PANE_TTL_PREPARES);
    }
}

/// The heatmap pipeline: a triangle list, blended exactly the way egui blends
/// its own shapes so the heatmap composites under the notes identically to the
/// tessellated mesh it replaces.
fn create_spectrogram_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("spectrogram_shader"),
        source: wgpu::ShaderSource::Wgsl(SPECTROGRAM_SRC.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("spectrogram_pipeline_layout"),
        bind_group_layouts: &[Some(layout)],
        ..Default::default()
    });
    // Same fork egui makes, for the same reason: an sRGB-aware target wants
    // linear values and encodes them itself.
    let shade = if target_format.is_srgb() { "linear" } else { "gamma" };
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("spectrogram"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_heatmap"),
            compilation_options: Default::default(),
            buffers: &[SpectrogramVertex::LAYOUT],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(&format!("fs_heatmap_{shade}")),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(EGUI_BLEND),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl CallbackTrait for SpectrogramCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let recreate = callback_resources
            .get::<SpectrogramResources>()
            .is_none_or(|r| r.target_format != self.target_format);
        if recreate {
            callback_resources.insert(SpectrogramResources::new(device, self.target_format));
        }
        let resources: &mut SpectrogramResources =
            callback_resources.get_mut().expect("inserted above when missing");
        resources.prepares = resources.prepares.wrapping_add(1);
        let prepares = resources.prepares;

        let SpectrogramResources { layout, panes, .. } = resources;
        SpectrogramPane::evict_unseen(panes, prepares);
        let pane = SpectrogramPane::get(panes, device, self.pane_id, prepares);

        let bins = self.grid.bins as usize;
        let stride = slab_stride(self.grid.bins);
        let run_slabs = if bins == 0 { 0 } else { self.grid.run.len() / bins };
        let levels = self.shades.lut.len() as u32;
        // Every one of these is a degenerate the shader has no answer for — a
        // zero modulus, an empty run to clamp into, a table with no entry to
        // land on — so the frame draws nothing rather than the pipeline being
        // asked what it means.
        if self.grid.capacity == 0
            || bins == 0
            || run_slabs == 0
            || levels == 0
            || self.read.rows == 0
            || self.vertices.is_empty()
        {
            pane.count = 0;
            return Vec::new();
        }
        debug_assert!(
            run_slabs <= self.grid.capacity as usize,
            "a run of {run_slabs} slabs puts two keys in one of {} slots",
            self.grid.capacity
        );

        let key = (self.grid.generation, self.grid.capacity, self.grid.bins);
        let mut remade = false;
        if pane.grid.as_ref().is_none_or(|g| g.key != key) {
            let size = u64::from(self.grid.capacity) * u64::from(stride);
            // Kept when the shape is unchanged, so a rebuild of the same grid
            // is one write rather than a fresh 15.7 MB allocation. It is not
            // the rare path the reallocation would be sized for: a Span parked
            // on a ladder rung refolds on every frame, and each refold moves
            // enough of the run to be uploaded whole.
            let kept = pane.grid.take().filter(|g| g.fits(self.grid.capacity, self.grid.bins));
            remade = kept.is_none();
            let buffer = match kept {
                Some(held) => held.buffer,
                None => device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("spectrogram_grid"),
                    size,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
            };
            // The whole ring in one write, so the slots the run does not cover
            // are zero rather than whatever the buffer held before — which is
            // what lets one be kept above. A
            // whole-song ring is 4096 x 3828 bytes = 15.7 MB, comfortably
            // inside wgpu's default 128 MiB storage binding, and this runs
            // only on a refold or a lost buffer.
            let mut staging = vec![0u8; size as usize];
            for j in 0..run_slabs {
                let at = slot_of(self.grid.first_key + j as i64, self.grid.capacity) as usize
                    * stride as usize;
                staging[at..at + bins].copy_from_slice(&self.grid.run[j * bins..(j + 1) * bins]);
            }
            queue.write_buffer(&buffer, 0, &staging);
            pane.grid = Some(GridBuffer { buffer, key });
        } else {
            let buffer = &pane.grid.as_ref().expect("the branch above holds a buffer").buffer;
            let mut slab = vec![0u8; stride as usize];
            for &dirty in &self.grid.dirty {
                let j = dirty - self.grid.first_key;
                debug_assert!(
                    j >= 0 && (j as usize) < run_slabs,
                    "dirty slab {dirty} is outside the run at {}",
                    self.grid.first_key
                );
                if j < 0 || j as usize >= run_slabs {
                    continue;
                }
                let j = j as usize;
                slab[..bins].copy_from_slice(&self.grid.run[j * bins..(j + 1) * bins]);
                let slot = slot_of(dirty, self.grid.capacity);
                queue.write_buffer(buffer, u64::from(slot) * u64::from(stride), &slab);
            }
        }

        let fresh_lut = pane.lut.as_ref().is_none_or(|l| l.levels != levels);
        if fresh_lut {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("spectrogram_lut"),
                size: wgpu::Extent3d { width: levels, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            pane.lut =
                Some(LutTexture { texture, view, levels, generation: self.shades.generation });
            remade = true;
        }
        let lut = pane.lut.as_mut().expect("created above when missing");
        if fresh_lut || lut.generation != self.shades.generation {
            lut.generation = self.shades.generation;
            queue.write_texture(
                lut.texture.as_image_copy(),
                bytemuck::cast_slice(self.shades.lut.as_slice()),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(levels * 4),
                    rows_per_image: None,
                },
                wgpu::Extent3d { width: levels, height: 1, depth_or_array_layers: 1 },
            );
        }

        if remade || pane.bind_group.is_none() {
            let grid = pane.grid.as_ref().expect("a drawable frame holds a grid");
            let lut = pane.lut.as_ref().expect("created above when missing");
            pane.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("spectrogram_bind_group"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: pane.uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry { binding: 1, resource: grid.buffer.as_entire_binding() },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: pane.weight_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&lut.view),
                    },
                ],
            }));
        }

        if self.vertices.len() > pane.vertex_capacity {
            pane.vertex_capacity = self.vertices.len().next_power_of_two();
            pane.vertex_buffer = create_vertex_buffer::<SpectrogramVertex>(
                device,
                "spectrogram_vertices",
                pane.vertex_capacity,
            );
        }
        pane.count = self.vertices.len() as u32;
        queue.write_buffer(&pane.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        queue.write_buffer(&pane.weight_buffer, 0, bytemuck::cast_slice(&self.read.weight[..]));

        let ppp = screen_descriptor.pixels_per_point.max(f32::EPSILON);
        let uniforms = SpectrogramUniforms {
            // The whole surface, which is the viewport `paint` draws into.
            origin_points: [0.0, 0.0],
            viewport_points: [
                screen_descriptor.size_in_pixels[0] as f32 / ppp,
                screen_descriptor.size_in_pixels[1] as f32 / ppp,
            ],
            min_midi: self.read.min_midi,
            span: self.read.span,
            margin: self.read.margin,
            spectrum_min_midi: self.read.spectrum_min_midi,
            bins_per_semitone: self.read.bins_per_semitone,
            level0: self.read.level0,
            level_per_step: self.read.level_per_step,
            level_per_midi: self.read.level_per_midi,
            mean_steps: self.read.mean_steps,
            rows: self.read.rows,
            bins: self.grid.bins,
            stride,
            capacity: self.grid.capacity,
            first_slot: slot_of(self.grid.first_key, self.grid.capacity),
            run_slabs: run_slabs as u32,
            _pad: 0,
        };
        queue.write_buffer(&pane.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        // Last, and only on the path that wrote: the caller reads this to
        // decide whether its next delta may be computed against this run, so
        // it has to name a run whose slabs are in their slots. Every early
        // return above leaves it standing at the previous handover, which is
        // what makes an undrawn frame legible rather than silent.
        self.grid.uploaded.store(self.grid.serial, Ordering::Relaxed);

        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<SpectrogramResources>() else {
            return;
        };
        let Some(pane) = resources.panes.get(&self.pane_id) else {
            return;
        };
        let Some(bind_group) = &pane.bind_group else {
            return;
        };
        if pane.count == 0 {
            return;
        }
        // Draw against the WHOLE surface rather than the viewport egui-wgpu
        // helpfully set to the callback rect: the geometry is in screen
        // points, so this shader's clip mapping is egui's own and there is no
        // second rounding of the pane rect into pixels to disagree with.
        // egui-wgpu resets the viewport after a callback, and the SCISSOR it
        // set from the clip rect is left alone — that is what keeps the
        // heatmap inside its pane.
        render_pass.set_viewport(
            0.0,
            0.0,
            info.screen_size_px[0] as f32,
            info.screen_size_px[1] as f32,
            0.0,
            1.0,
        );
        render_pass.set_pipeline(&resources.pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.set_vertex_buffer(0, pane.vertex_buffer.slice(..));
        render_pass.draw(0..pane.count, 0..1);
    }
}

/// A headless GPU a dependent crate's pixel tests draw through: one device
/// and one set of `CallbackResources`, held across frames.
///
/// Held, because a single-shot frame can only ever take the full-upload path —
/// fresh resources hold no grid to patch. A test of the DELTA (which slabs the
/// caller says have moved) has to hand the same resources one frame after
/// another, exactly as a pane does.
///
/// `None` where the machine has no GPU adapter, which every caller returns on.
pub struct SpectrogramHeadless {
    device: wgpu::Device,
    queue: wgpu::Queue,
    resources: CallbackResources,
}

impl SpectrogramHeadless {
    pub fn new() -> Option<SpectrogramHeadless> {
        let (device, queue) = crate::gpu_harness::headless_device()?;
        Some(SpectrogramHeadless { device, queue, resources: CallbackResources::default() })
    }

    /// One frame: the same `prepare`/`paint` a pane takes, into a fresh
    /// `Rgba8Unorm` texture cleared to opaque black, read back as tightly
    /// packed RGBA8 rows.
    ///
    /// The shipping callback and not a reimplementation of it: a parity test
    /// against a second draw path can only measure the second path.
    ///
    /// `size[0]` must be a multiple of 64, so the readback's rows stay
    /// 256-byte aligned. `pane_id` picks which grid copy this frame patches,
    /// so a test wanting a full upload beside a delta asks on a second id.
    pub fn frame(
        &mut self,
        pane_id: u64,
        size: [u32; 2],
        vertices: Vec<SpectrogramVertex>,
        grid: SpectrogramGrid,
        read: SpectrogramRead,
        shades: SpectrogramShades,
    ) -> Vec<u8> {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0] as f32, size[1] as f32));
        let callback =
            SpectrogramCallback { vertices, grid, read, shades, target_format: format, pane_id };
        let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: 1.0 };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let buffers =
            callback.prepare(&self.device, &self.queue, &screen, &mut encoder, &mut self.resources);
        self.queue.submit(buffers.into_iter().chain([encoder.finish()]));

        let texture = crate::gpu_harness::render_to_texture(
            &self.device,
            &self.queue,
            size,
            format,
            wgpu::Color::BLACK,
            |pass| {
                callback.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: 1.0,
                        screen_size_px: size,
                    },
                    pass,
                    &self.resources,
                );
            },
        );
        crate::gpu_harness::readback(&self.device, &self.queue, &texture, size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_harness::{headless_device, readback, render_to_texture};

    /// 128 x 128 at one point per pixel. Every coordinate below is a dyadic
    /// fraction of that, so the rasterizer's interpolation of `slab` and `t`
    /// is exact and [`Reference`] can reproduce which taps a pixel took —
    /// otherwise a fragment landing a float's width across a row boundary
    /// reads a different run and the parity numbers measure the fixture.
    const SIZE: [u32; 2] = [128, 128];
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    /// Buckets per semitone, and the spectrum's own floor — the analyzer's
    /// numbers, so the row geometry is the shape the pane hands over.
    const BINS_PER_SEMITONE: f32 = 32.0;
    const SPECTRUM_MIN_MIDI: f32 = 15.486_82;
    /// Buckets in a test spectrum: 32 semitones of it, which is enough range
    /// for a fixture to sit inside and for another to run off the top.
    const BINS: u32 = 1024;
    /// `ROW_MEAN_STEPS`: stored steps per halving of the summed weight, at
    /// order 4 over a 0.5 dB store.
    const MEAN_STEPS: f32 = 10.0 / (4.0 * 0.5 * std::f32::consts::LOG2_10);

    /// `ROW_WEIGHT`: the weight of a bucket `j` stored steps below its run's
    /// loudest.
    fn weights() -> Arc<[f32; 256]> {
        Arc::new(std::array::from_fn(|j| 10f32.powf(-0.1 * 4.0 * (j as f32 * 0.5))))
    }

    /// A gradient table shaped like the real one: 4096 samples of a ramp whose
    /// channels are 8-bit, so one index of slack is one level of one channel
    /// and a wrong INDEX is still a wrong colour.
    fn ramp_lut() -> Arc<Vec<[u8; 4]>> {
        Arc::new(
            (0..4096)
                .map(|i| {
                    let v = i as f32 / 4095.0;
                    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
                    [b(v), b(1.0 - v), b(v * v), 255]
                })
                .collect(),
        )
    }

    fn shades() -> SpectrogramShades {
        SpectrogramShades { generation: 1, lut: ramp_lut() }
    }

    /// The read's scalars for a visible range of `span` semitones from
    /// `min_midi`, drawn at `rows`. The level mapping spends most of the 0..1
    /// on the fixtures' own byte range and tilts with pitch, so a row that
    /// lands on the wrong bucket lands on the wrong colour.
    fn read_of(min_midi: f32, span: f32, rows: u32) -> SpectrogramRead {
        SpectrogramRead {
            min_midi,
            span,
            margin: (1.0 / BINS_PER_SEMITONE / span).min(0.5),
            rows,
            spectrum_min_midi: SPECTRUM_MIN_MIDI,
            bins_per_semitone: BINS_PER_SEMITONE,
            level0: 0.0,
            level_per_step: 0.0035,
            level_per_midi: 0.002,
            mean_steps: MEAN_STEPS,
            weight: weights(),
        }
    }

    /// A grid no read can get right by luck: a ramp across the buckets, a
    /// one-bucket peak that moves slab to slab, and an LCG noise bed over
    /// both.
    ///
    /// The noise is what makes the MEAN arm measurable at all. Over a pure
    /// tone every bucket of a run but one is the floor, so the weighted sum is
    /// within a hair of the run's own maximum and a shader that returned the
    /// max would pass.
    fn noisy_grid(bins: usize, slabs: usize) -> Arc<Vec<u8>> {
        let mut seed = 0x2545_f491u32;
        let mut out = vec![0u8; bins * slabs];
        for s in 0..slabs {
            for b in 0..bins {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (seed >> 24) % 40;
                let ramp = (b * 120 / bins) as u32;
                let peak = if b % 61 == s % 61 { 90 } else { 0 };
                out[s * bins + b] = (20 + ramp + noise + peak).min(250) as u8;
            }
        }
        Arc::new(out)
    }

    fn grid_of(run: Arc<Vec<u8>>, bins: u32, capacity: u32, first_key: i64) -> SpectrogramGrid {
        SpectrogramGrid {
            generation: 1,
            serial: 1,
            uploaded: Arc::default(),
            capacity,
            bins,
            first_key,
            run,
            dirty: Vec::new(),
        }
    }

    /// A quad over the whole surface: `slab` running 0..n left to right, `t`
    /// running 1 at the top down to 0 at the bottom.
    fn full_quad(run_slabs: u32) -> Vec<SpectrogramVertex> {
        let (w, h) = (SIZE[0] as f32, SIZE[1] as f32);
        let n = run_slabs as f32;
        let v = |x: f32, y: f32| SpectrogramVertex { pos: [x, y], slab: x / w * n, t: 1.0 - y / h };
        vec![v(0.0, 0.0), v(w, 0.0), v(w, h), v(0.0, 0.0), v(w, h), v(0.0, h)]
    }

    /// One row of the picture, as `row_of` lays them out.
    #[derive(Clone, Copy)]
    struct Row {
        lo_t: f32,
        hi_t: f32,
        t: f32,
        midi: f32,
    }

    /// The read the shader performs, in Rust — the same arms, the same
    /// rounding, the same four-tap blend in gamma space, indexing the run
    /// DIRECTLY rather than through a slot, so a slot rule that disagrees with
    /// the scatter shows up as a wrong column.
    struct Reference {
        grid: SpectrogramGrid,
        read: SpectrogramRead,
        shades: SpectrogramShades,
    }

    /// Nearest whole number, ties away from zero — `f32::round`, which the
    /// shader's `round_half_away` reproduces because WGSL's own `round` takes
    /// ties to even.
    fn round_half_away(x: f32) -> f32 {
        let f = x.floor();
        if x - f >= 0.5 {
            f + 1.0
        } else {
            f
        }
    }

    impl Reference {
        fn bins(&self) -> usize {
            self.grid.bins as usize
        }

        fn run_slabs(&self) -> usize {
            self.grid.run.len() / self.bins()
        }

        fn stored(&self, j: usize, bucket: usize) -> u8 {
            self.grid.run[j * self.bins() + bucket]
        }

        fn row_of(&self, r: u32) -> Row {
            let m = self.read.margin;
            let reach = 1.0 + 2.0 * m;
            let rows = self.read.rows as f32;
            let lo_t = -m + reach * r as f32 / rows;
            let hi_t = -m + reach * (r + 1) as f32 / rows;
            let t = 0.5 * (lo_t + hi_t);
            Row { lo_t, hi_t, t, midi: self.read.min_midi + t * self.read.span }
        }

        fn bucket_of(&self, t: f32) -> usize {
            let midi = self.read.min_midi + t * self.read.span;
            let b = ((midi - self.read.spectrum_min_midi) * self.read.bins_per_semitone).floor();
            b.clamp(0.0, self.bins() as f32 - 1.0) as usize
        }

        fn read_row(&self, j: usize, row: Row) -> f32 {
            let idx = self.bucket_of(row.lo_t);
            let last = self.bucket_of(row.hi_t);
            if last > idx {
                let to = (last + 1).min(self.bins());
                let top = (idx..to).map(|b| self.stored(j, b)).max().expect("a non-empty run");
                let n = to - idx;
                if n < 2 {
                    return f32::from(top);
                }
                let sum: f32 =
                    (idx..to).map(|b| self.read.weight[usize::from(top - self.stored(j, b))]).sum();
                let steps = -(sum / n as f32).log2() * self.read.mean_steps;
                return (f32::from(top) - round_half_away(steps)).max(0.0);
            }
            let x = (row.midi - self.read.spectrum_min_midi) * self.read.bins_per_semitone - 0.5;
            let lo = x.floor().clamp(0.0, self.bins() as f32 - 2.0) as usize;
            let f = (x - lo as f32).clamp(0.0, 1.0);
            let (a, b) = (f32::from(self.stored(j, lo)), f32::from(self.stored(j, lo + 1)));
            round_half_away(a + (b - a) * f)
        }

        fn shade(&self, j: usize, row: Row) -> [f32; 4] {
            let value = self.read_row(j, row);
            let row0 = self.read.level0 + self.read.level_per_midi * row.midi;
            let level = (row0 + self.read.level_per_step * value).clamp(0.0, 1.0);
            let levels = self.shades.lut.len();
            let i = ((level * levels as f32) as usize).min(levels - 1);
            let c = self.shades.lut[i];
            [
                f32::from(c[0]) / 255.0,
                f32::from(c[1]) / 255.0,
                f32::from(c[2]) / 255.0,
                f32::from(c[3]) / 255.0,
            ]
        }

        fn color_at(&self, slab: f32, t: f32) -> [u8; 4] {
            let n = self.run_slabs();
            let jx = (slab - 0.5).floor().clamp(0.0, n as f32 - 1.0);
            let j0 = jx as usize;
            let j1 = (j0 + 1).min(n - 1);
            let fx = (slab - 0.5 - jx).clamp(0.0, 1.0);

            let rows = self.read.rows;
            let t0c = self.row_of(0).t;
            let tnc = self.row_of(rows - 1).t;
            let denom = tnc - t0c;
            let v = if denom == 0.0 { 0.0 } else { (t - t0c) / denom };
            let y = v * rows as f32 - 0.5;
            let ry = y.floor().clamp(0.0, rows as f32 - 1.0);
            let r0 = ry as u32;
            let r1 = (r0 + 1).min(rows - 1);
            let fy = (y - ry).clamp(0.0, 1.0);

            let (low, high) = (self.row_of(r0), self.row_of(r1));
            let mix = |a: [f32; 4], b: [f32; 4], f: f32| {
                std::array::from_fn(|c| a[c] * (1.0 - f) + b[c] * f)
            };
            let c = mix(
                mix(self.shade(j0, low), self.shade(j1, low), fx),
                mix(self.shade(j0, high), self.shade(j1, high), fx),
                fy,
            );
            [
                (c[0] * 255.0).round() as u8,
                (c[1] * 255.0).round() as u8,
                (c[2] * 255.0).round() as u8,
                255,
            ]
        }

        /// The frame [`full_quad`] draws, pixel by pixel.
        fn frame(&self) -> Vec<u8> {
            let (w, h) = (SIZE[0], SIZE[1]);
            let n = self.run_slabs() as f32;
            let mut out = vec![0u8; (w * h * 4) as usize];
            for py in 0..h {
                for px in 0..w {
                    let slab = (px as f32 + 0.5) / w as f32 * n;
                    let t = 1.0 - (py as f32 + 0.5) / h as f32;
                    let c = self.color_at(slab, t);
                    let i = ((py * w + px) * 4) as usize;
                    out[i..i + 4].copy_from_slice(&c);
                }
            }
            out
        }

        /// How many rows take each arm — `(mean, lerp)`. A fixture claiming an
        /// arm is checked against this rather than assumed to reach it.
        fn arms(&self) -> (usize, usize) {
            let mut mean = 0;
            let mut lerp = 0;
            for r in 0..self.read.rows {
                let row = self.row_of(r);
                if self.bucket_of(row.hi_t) > self.bucket_of(row.lo_t) {
                    mean += 1;
                } else {
                    lerp += 1;
                }
            }
            (mean, lerp)
        }

        /// How many rows read a run of at least `n` buckets.
        fn runs_at_least(&self, n: usize) -> usize {
            (0..self.read.rows)
                .filter(|&r| {
                    let row = self.row_of(r);
                    let (idx, last) = (self.bucket_of(row.lo_t), self.bucket_of(row.hi_t));
                    last > idx && (last + 1).min(self.bins()) - idx >= n
                })
                .count()
        }

        /// How many rows take the LERP arm with its lower tap pinned at the
        /// top of the spectrum.
        fn top_clamped(&self) -> usize {
            (0..self.read.rows)
                .filter(|&r| {
                    let row = self.row_of(r);
                    if self.bucket_of(row.hi_t) > self.bucket_of(row.lo_t) {
                        return false;
                    }
                    let x = (row.midi - self.read.spectrum_min_midi) * self.read.bins_per_semitone
                        - 0.5;
                    x.floor() >= self.bins() as f32 - 2.0
                })
                .count()
        }
    }

    fn callback(
        vertices: Vec<SpectrogramVertex>,
        grid: &SpectrogramGrid,
        read: &SpectrogramRead,
    ) -> SpectrogramCallback {
        SpectrogramCallback {
            vertices,
            grid: grid.clone(),
            read: read.clone(),
            shades: shades(),
            target_format: FORMAT,
            pane_id: 0,
        }
    }

    /// One `prepare` of `cb` against `resources`, submitted — the unit a
    /// pane's age is measured in.
    fn prepare_once(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &mut CallbackResources,
        cb: &SpectrogramCallback,
    ) {
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(device, queue, &screen, &mut encoder, resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
    }

    /// `prepare` then `paint` against resources the caller owns, so a test can
    /// hand the same ones a sequence of frames.
    fn frame_with(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &mut CallbackResources,
        cb: &SpectrogramCallback,
    ) -> Vec<u8> {
        prepare_once(device, queue, resources, cb);
        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        let texture = render_to_texture(device, queue, SIZE, FORMAT, wgpu::Color::BLACK, |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: 1.0,
                    screen_size_px: SIZE,
                },
                pass,
                resources,
            );
        });
        readback(device, queue, &texture, SIZE)
    }

    /// The same frame from resources that have never seen this pane — the
    /// full-upload path.
    fn fresh_frame(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cb: &SpectrogramCallback,
    ) -> Vec<u8> {
        let mut resources = CallbackResources::default();
        frame_with(device, queue, &mut resources, cb)
    }

    /// Largest channel difference and how many pixels match exactly.
    fn compare(got: &[u8], want: &[u8]) -> (u8, usize) {
        let mut worst = 0u8;
        let mut exact = 0usize;
        for (g, w) in got.chunks_exact(4).zip(want.chunks_exact(4)) {
            let d = g.iter().zip(w).map(|(&a, &b)| a.abs_diff(b)).max().unwrap_or(0);
            worst = worst.max(d);
            exact += usize::from(d == 0);
        }
        (worst, exact)
    }

    /// The frame the shader draws against the read written out in Rust,
    /// reported as `(max channel diff, fraction exact)`.
    fn parity(device: &wgpu::Device, queue: &wgpu::Queue, reference: &Reference) -> (u8, f64) {
        let cb =
            callback(full_quad(reference.run_slabs() as u32), &reference.grid, &reference.read);
        let got = fresh_frame(device, queue, &cb);
        let (worst, exact) = compare(&got, &reference.frame());
        (worst, exact as f64 / (SIZE[0] * SIZE[1]) as f64)
    }

    #[test]
    fn baked_spectrogram_shader_validates() {
        let module = naga::front::wgsl::parse_str(SPECTROGRAM_SRC)
            .map_err(|e| e.emit_to_string(SPECTROGRAM_SRC))
            .expect("spectrogram.wgsl must parse");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("spectrogram.wgsl must validate");
        for required in SPECTROGRAM_ENTRY_POINTS {
            assert!(
                module.entry_points.iter().any(|ep| ep.name == *required),
                "missing entry point `{required}`"
            );
        }
    }

    /// The vertex-layout <-> shader-input contract (attribute locations,
    /// formats, strides) and the bind-group layout against the shader's own
    /// bindings, neither of which the naga check or the type system covers — a
    /// mismatch otherwise panics at first paint inside a host.
    #[test]
    fn the_pipeline_builds_against_a_headless_device() {
        let Some((device, _queue)) = headless_device() else {
            return;
        };
        let _resources = SpectrogramResources::new(&device, FORMAT);
    }

    /// Every row wider than a bucket: the shader's MEAN arm against the same
    /// weighted sum written in Rust, over runs of sixteen buckets.
    ///
    /// The arm is the one thing a pure tone cannot measure, so the fixture is
    /// a noise bed with peaks in it — see [`noisy_grid`] — and the row count
    /// is checked below rather than assumed from the span.
    ///
    /// The read itself comes out bit for bit: what is left is one pixel of
    /// 16384 whose blended channel rounds the other way into eight bits, the
    /// GPU fusing the multiply-add the Rust reference writes out.
    #[test]
    fn a_row_wider_than_a_bucket_reads_the_weighted_mean_of_its_run() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let run = noisy_grid(BINS as usize, 6);
        let reference = Reference {
            grid: grid_of(run, BINS, 8, 0),
            read: read_of(18.0, 24.0, 48),
            shades: shades(),
        };
        let (mean, lerp) = reference.arms();
        assert_eq!((mean, lerp), (48, 0), "every row of this fixture must take the mean");
        assert_eq!(reference.runs_at_least(16), 48, "the runs must be wide enough to average");

        let (worst, exact) = parity(&device, &queue, &reference);
        assert!(worst <= 1, "channels differ by {worst} levels, not the blend's own rounding");
        assert!(exact >= 0.99, "only {:.4} of pixels are exact", exact);
    }

    /// Rows narrower than a bucket: the LERP arm, with the half-bucket centre
    /// offset and the clamp that keeps its upper tap in the spectrum.
    ///
    /// A row narrower than a bucket still STRADDLES one now and then, so this
    /// fixture reaches both arms and the counts below say in what proportion —
    /// a shader that took the mean everywhere would fail on the majority.
    ///
    /// Two pixels of 16384 differ, both by one level of one channel and both
    /// on a rounding boundary of the blend rather than in the read.
    #[test]
    fn a_row_narrower_than_a_bucket_reads_between_the_two_under_it() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let run = noisy_grid(BINS as usize, 6);
        let reference = Reference {
            grid: grid_of(run, BINS, 8, 0),
            read: read_of(20.0, 2.0, 512),
            shades: shades(),
        };
        let (mean, lerp) = reference.arms();
        assert!(lerp > 3 * mean, "this fixture must be mostly lerp, got {lerp} against {mean}");
        assert!(mean > 0, "and must still straddle a boundary somewhere");

        let (worst, exact) = parity(&device, &queue, &reference);
        assert!(worst <= 1, "channels differ by {worst} levels, not the blend's own rounding");
        assert!(exact >= 0.99, "only {:.4} of pixels are exact", exact);
    }

    /// A visible range running off the top of the spectrum: the reads that
    /// clamp — `bucket_of` at the last bucket, and the lerp's lower tap pinned
    /// at `bins - 2` so its upper tap still exists.
    ///
    /// Byte-identical to the Rust read across the whole frame.
    #[test]
    fn a_range_past_the_top_of_the_spectrum_reads_its_last_two_buckets() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let run = noisy_grid(BINS as usize, 6);
        let reference = Reference {
            grid: grid_of(run, BINS, 8, 0),
            read: read_of(42.0, 8.0, 512),
            shades: shades(),
        };
        let clamped = reference.top_clamped();
        assert!(clamped > 50, "only {clamped} rows reach the clamp at the top bucket");
        assert!(clamped < reference.read.rows as usize, "the fixture must also read inside");

        let (worst, exact) = parity(&device, &queue, &reference);
        assert!(worst <= 1, "channels differ by {worst} levels, not the blend's own rounding");
        assert!(exact >= 0.99, "only {:.4} of pixels are exact", exact);
    }

    /// Slab `key` at `version`, distinct in both.
    fn versioned_slab(key: i64, version: u32, bins: usize) -> Vec<u8> {
        let mut seed = (key as u64 as u32).wrapping_mul(2_654_435_761).wrapping_add(version);
        (0..bins)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (20 + (seed >> 24) % 200) as u8
            })
            .collect()
    }

    /// One frame of the sequence
    /// [`a_delta_upload_draws_what_a_full_upload_draws`] plays: what the
    /// window is, what changed under it, and what the caller declares changed.
    struct Step {
        label: &'static str,
        generation: u64,
        capacity: u32,
        first_key: i64,
        /// Keys whose content is rewritten before this frame.
        mutate: &'static [i64],
        /// Keys whose slot does not hold their content — what the UI side
        /// owes, and the only thing standing between the delta and a stale
        /// column.
        dirty: &'static [i64],
    }

    impl Step {
        const fn new(
            label: &'static str,
            generation: u64,
            capacity: u32,
            first_key: i64,
            mutate: &'static [i64],
            dirty: &'static [i64],
        ) -> Step {
            Step { label, generation, capacity, first_key, mutate, dirty }
        }
    }

    /// Slabs in the window every [`Step`] draws.
    const SLABS: usize = 6;

    /// A frame built from the delta path equals one built from a full upload,
    /// through a sequence that moves every part of the cache key: the newest
    /// slab and an interior one rewritten in place, a window that advances
    /// into slots another key held, keys before zero, a run that wraps past
    /// `capacity`, a generation bump, and a capacity change.
    ///
    /// This is the new cache key and the only thing checking it: a stale slot
    /// is a wrong column, and nothing on the CPU can see one.
    ///
    /// Each frame's acknowledgement is checked alongside, because it is what
    /// entitles the CALLER to send the next delta: a serial that arrives for a
    /// frame that wrote nothing would license a delta against a buffer that
    /// never received the run.
    #[test]
    fn a_delta_upload_draws_what_a_full_upload_draws() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let bins = 256usize;
        let read = read_of(18.0, 6.0, 96);
        let mut version: HashMap<i64, u32> = HashMap::new();
        let mut resources = CallbackResources::default();
        let mut previous: Option<Vec<u8>> = None;
        let mut moved = 0;
        let uploaded: Arc<AtomicU64> = Arc::default();

        let steps = &[
            Step::new("the first upload", 1, 8, 0, &[], &[]),
            Step::new("the newest slab rewritten", 1, 8, 0, &[5], &[5]),
            Step::new("an interior slab rewritten", 1, 8, 0, &[2], &[2]),
            Step::new("the window advanced by one", 1, 8, 1, &[], &[6]),
            Step::new("advanced again", 1, 8, 2, &[], &[7]),
            Step::new("advanced past capacity", 1, 8, 4, &[], &[8, 9]),
            Step::new("a wrapped slab rewritten", 1, 8, 4, &[9], &[9]),
            Step::new("a generation bump onto keys before zero", 2, 8, -3, &[], &[]),
            Step::new("a negative key rewritten", 2, 8, -3, &[-1], &[-1]),
            Step::new("advanced across zero", 2, 8, -2, &[], &[3]),
            Step::new("a capacity change", 2, 12, -2, &[], &[]),
        ];

        for step in steps {
            for &key in step.mutate {
                *version.entry(key).or_insert(0) += 1;
            }
            let mut run = Vec::with_capacity(SLABS * bins);
            for j in 0..SLABS as i64 {
                let key = step.first_key + j;
                run.extend(versioned_slab(key, *version.entry(key).or_insert(0), bins));
            }
            let serial = uploaded.load(Ordering::Relaxed) + 1;
            let grid = SpectrogramGrid {
                generation: step.generation,
                serial,
                uploaded: uploaded.clone(),
                capacity: step.capacity,
                bins: bins as u32,
                first_key: step.first_key,
                run: Arc::new(run),
                dirty: step.dirty.to_vec(),
            };
            let cb = callback(full_quad(SLABS as u32), &grid, &read);
            let incremental = frame_with(&device, &queue, &mut resources, &cb);
            assert_eq!(
                uploaded.load(Ordering::Relaxed),
                serial,
                "{} drew without acknowledging its run",
                step.label
            );
            let full = fresh_frame(&device, &queue, &cb);
            assert_eq!(
                incremental, full,
                "{} did not land where a full upload puts it",
                step.label
            );
            if previous.replace(incremental.clone()).is_some_and(|p| p != incremental) {
                moved += 1;
            }
        }
        // Every step but the two that only re-declare the same picture moves
        // it, so the equality above is being asked of frames that differ.
        assert!(moved >= 8, "only {moved} steps changed the picture");

        // A frame with nothing to draw writes no slab, so it must not claim
        // one: the caller reads the acknowledgement as "the run I handed over
        // is in its slots", which an early return has not made true.
        let standing = uploaded.load(Ordering::Relaxed);
        let empty = SpectrogramGrid {
            generation: 9,
            serial: standing + 1,
            uploaded: uploaded.clone(),
            capacity: 8,
            bins: bins as u32,
            first_key: 0,
            run: Arc::new(Vec::new()),
            dirty: Vec::new(),
        };
        prepare_once(&device, &queue, &mut resources, &callback(full_quad(1), &empty, &read));
        assert_eq!(
            uploaded.load(Ordering::Relaxed),
            standing,
            "a frame that drew nothing acknowledged a run it never wrote",
        );
    }

    /// Keys before zero and a run that wraps past the end of the ring land
    /// where the shader reads them.
    ///
    /// Each slab is one constant byte, so a column is one colour whichever arm
    /// its rows take, and a mis-slotted column is a colour from somewhere else
    /// in the run rather than a blur. One slab per pixel column puts every
    /// sample on a slab's own centre, where the time axis' blend weight is 0.
    #[test]
    fn slab_keys_before_zero_and_a_wrapping_run_land_where_the_shader_reads() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let bins = 128usize;
        let slabs = SIZE[0] as usize;
        let run: Vec<u8> =
            (0..slabs).flat_map(|j| std::iter::repeat_n((j * 2) as u8, bins)).collect();
        let run = Arc::new(run);
        // No tilt: a column is then one colour top to bottom, so a wrong slot
        // cannot hide behind the pitch axis.
        let read = SpectrogramRead { level_per_midi: 0.0, ..read_of(18.0, 4.0, 96) };

        for (capacity, first_key) in [(128u32, -37i64), (160, -37)] {
            let grid = SpectrogramGrid {
                generation: 1,
                serial: 1,
                uploaded: Arc::default(),
                capacity,
                bins: bins as u32,
                first_key,
                run: run.clone(),
                dirty: Vec::new(),
            };
            // The run really does cross the ring's end.
            let first_slot = slot_of(first_key, capacity);
            assert!(
                first_slot + slabs as u32 > capacity,
                "capacity {capacity} does not make this run wrap"
            );
            let reference = Reference { grid: grid.clone(), read: read.clone(), shades: shades() };
            let want: Vec<[u8; 4]> =
                (0..slabs).map(|j| reference.color_at(j as f32 + 0.5, 0.5)).collect();
            for j in 1..slabs {
                assert_ne!(want[j - 1], want[j], "slabs {j} and {} draw alike", j - 1);
            }

            let cb = callback(full_quad(slabs as u32), &grid, &read);
            let frame = fresh_frame(&device, &queue, &cb);
            for (px, expected) in want.iter().enumerate() {
                let i = ((SIZE[1] / 2) * SIZE[0] + px as u32) as usize * 4;
                let got: [u8; 4] = frame[i..i + 4].try_into().expect("four channels");
                assert_eq!(
                    got, *expected,
                    "column {px} of a run at {first_key} in a ring of {capacity}"
                );
            }
        }
    }

    /// Two spectrograms in one frame keep their own grid copies, and a pane
    /// that stops drawing gives its copy back.
    ///
    /// A whole-song ring is 15.7 MB, so a closed tab holding one is the reason
    /// the sweep exists — and there is no teardown to hang it on, a closed tab
    /// simply stopping calling back, so the sweep runs from whichever pane IS
    /// preparing.
    #[test]
    fn a_pane_that_stops_drawing_gives_its_grid_back() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let read = read_of(18.0, 24.0, 48);
        let grid = grid_of(noisy_grid(BINS as usize, 6), BINS, 8, 0);
        let of_pane =
            |pane_id| SpectrogramCallback { pane_id, ..callback(full_quad(6), &grid, &read) };
        let (docked, preview) = (of_pane(0), of_pane(1));
        let mut resources = CallbackResources::default();
        prepare_once(&device, &queue, &mut resources, &docked);
        prepare_once(&device, &queue, &mut resources, &preview);
        let live = |resources: &CallbackResources| {
            let spectrogram: &SpectrogramResources =
                resources.get().expect("prepare inserts its resources");
            let mut ids: Vec<u64> = spectrogram.panes.keys().copied().collect();
            ids.sort_unstable();
            ids
        };
        assert_eq!(live(&resources), vec![0, 1], "both spectrograms should hold buffers");
        let held = |resources: &CallbackResources, id: u64| {
            let spectrogram: &SpectrogramResources = resources.get().expect("resources");
            spectrogram.panes[&id].grid.as_ref().expect("a drawn pane holds a grid").buffer.size()
        };
        assert_eq!(held(&resources, 0), held(&resources, 1), "each pane sizes its own copy");

        for _ in 0..PANE_TTL_PREPARES {
            prepare_once(&device, &queue, &mut resources, &docked);
        }
        assert_eq!(live(&resources), vec![0], "the closed pane is still holding its grid");
    }

    /// [`SpectrogramHeadless::frame`] is the callback, so a dependent crate's
    /// parity test measures the path that ships rather than a second one.
    #[test]
    fn the_headless_frame_is_the_frame_the_callback_paints() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let read = read_of(18.0, 24.0, 48);
        let grid = grid_of(noisy_grid(BINS as usize, 6), BINS, 8, 0);
        let cb = callback(full_quad(6), &grid, &read);
        let through_callback = fresh_frame(&device, &queue, &cb);
        let through_entry = SpectrogramHeadless::new()
            .expect("this machine has an adapter, the harness above just used it")
            .frame(0, SIZE, full_quad(6), grid.clone(), read.clone(), shades());
        assert_eq!(through_entry, through_callback);
    }
}
