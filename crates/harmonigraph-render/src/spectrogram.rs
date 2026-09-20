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
//! those constants — the visible pitch range, the level mapping's affine —
//! and holds nothing that reads a slab.
//!
//! The read is an image resample: a fragment covers its own footprint of the
//! pitch axis and combines the bucket levels under it, so the pane's pixel
//! height sets how finely the image is sampled and not how bright it is. The
//! vertex rule feeding the two slab taps is `heatmap_mesh`'s.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::pass_aged::PassAged;
use crate::{create_vertex_buffer, wgpu, EGUI_BLEND};

const SPECTROGRAM_SRC: &str = include_str!("shaders/spectrogram.wgsl");

mod atmosphere;
pub use atmosphere::SpectrogramAtmosphere;

/// The detailed heatmap, reduced cloud material, and final composite entry
/// points, including both target color spaces. Validate their names before
/// a lazy runtime pipeline is the first place a WGSL rename gets noticed.
#[cfg(test)]
pub(crate) const SPECTROGRAM_ENTRY_POINTS: &[&str] = &[
    "vs_heatmap",
    "fs_heatmap_gamma",
    "fs_heatmap_linear",
    "fs_density_source",
    "fs_cloud_light",
    "fs_cloud_gamma",
    "fs_cloud_linear",
    "fs_cloud_backdrop_gamma",
    "fs_cloud_backdrop_linear",
    "vs_cloud_tile",
    "fs_cloud_tile",
];

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
    /// Pixels the pane spends on the pitch axis. It sets how wide one
    /// fragment's footprint is, and so how finely the image is sampled — not
    /// how bright it comes out.
    pub rows: u32,
    /// MIDI at bucket 0's lower edge, and buckets per semitone.
    pub spectrum_min_midi: f32,
    pub bins_per_semitone: f32,
    /// The level mapping, before its 0..1 clamp:
    /// `level0 + level_per_step * byte + level_per_midi * midi`.
    ///
    /// Derived on the CPU from `spectrogram_level_raw`, which is affine in
    /// both — never re-derived here, so the mapping has one definition and the
    /// window, the tilt and their pivot stay the pane's business. Evaluated
    /// per BUCKET and clamped there, which is what makes the resample a
    /// resample of the picture rather than of the arithmetic behind it.
    pub level0: f32,
    pub level_per_step: f32,
    pub level_per_midi: f32,
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
/// `pass_nr` is the painter context's cumulative pass number.
#[allow(clippy::too_many_arguments)]
pub fn spectrogram_paint_callback(
    rect: egui::Rect,
    vertices: Vec<SpectrogramVertex>,
    grid: SpectrogramGrid,
    read: SpectrogramRead,
    shades: SpectrogramShades,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    pass_nr: u64,
    atmosphere: Option<SpectrogramAtmosphere>,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        SpectrogramCallback {
            rect,
            vertices,
            grid,
            read,
            shades,
            target_format,
            pane_id,
            pass_nr,
            atmosphere,
        },
    )
}

/// Per-frame, per-pane draw data, built on the UI thread.
struct SpectrogramCallback {
    rect: egui::Rect,
    vertices: Vec<SpectrogramVertex>,
    grid: SpectrogramGrid,
    read: SpectrogramRead,
    shades: SpectrogramShades,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    pass_nr: u64,
    atmosphere: Option<SpectrogramAtmosphere>,
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
    spectrum_min_midi: f32,
    bins_per_semitone: f32,
    level0: f32,
    level_per_step: f32,
    level_per_midi: f32,
    rows: u32,
    bins: u32,
    stride: u32,
    capacity: u32,
    first_slot: u32,
    run_slabs: u32,
    _pad: [u32; 3],
}

/// GPU objects cached across frames in egui-wgpu's `CallbackResources`.
struct SpectrogramResources {
    pipeline: wgpu::RenderPipeline,
    cloud: Option<atmosphere::Pipelines>,
    layout: wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    /// A closed spectrogram would otherwise hold its grid copy, so a closed
    /// tab keeping one is worth a sweep, and a pane hidden for a frame is worth
    /// not rebuilding. The age it sweeps at is
    /// [`crate::pass_aged::TTL_PASSES`].
    panes: PassAged<SpectrogramPane>,
}

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
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize,
    count: u32,
    grid: Option<GridBuffer>,
    lut: Option<LutTexture>,
    /// Made with the grid buffer and the table, so it is remade whenever
    /// either is.
    bind_group: Option<wgpu::BindGroup>,
    cloud: Option<atmosphere::Targets>,
    cloud_ready: bool,
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
        // The grid is read from `read_level` and nothing else, so the fragment
        // stage is the whole of its visibility. Naming the vertex stage as
        // well would ask the adapter for `VERTEX_STORAGE`, which a downlevel
        // backend need not have: the layout is then refused and the pipeline
        // never builds, for a binding no vertex shader reads. Only the
        // uniforms are wanted in both, by `vs_heatmap`'s projection.
        let vs_fs = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let fs = wgpu::ShaderStages::FRAGMENT;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spectrogram_bind_group_layout"),
            entries: &[
                buffer_entry(0, vs_fs, wgpu::BufferBindingType::Uniform),
                buffer_entry(1, fs, storage),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: fs,
                    ty: wgpu::BindingType::Texture {
                        // Loaded, never sampled: the resample and the blend
                        // that feed it are the shader's own.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        SpectrogramResources {
            pipeline: create_spectrogram_pipeline(
                device,
                target_format,
                &layout,
                None,
                if target_format.is_srgb() || target_format == wgpu::TextureFormat::Rgba16Float {
                    "fs_heatmap_linear"
                } else {
                    "fs_heatmap_gamma"
                },
            ),
            cloud: None,
            layout,
            target_format,
            panes: PassAged::new(),
        }
    }
}

impl SpectrogramPane {
    /// This pane's buffers, made on first sight of its id and stamped with
    /// `pass_nr` so the sweep can tell a live pane from one whose tab was
    /// closed.
    fn get<'a>(
        panes: &'a mut PassAged<SpectrogramPane>,
        device: &wgpu::Device,
        pane_id: u64,
        pass_nr: u64,
    ) -> &'a mut SpectrogramPane {
        panes.touched_or_insert_with(pane_id, pass_nr, || SpectrogramPane {
            uniform_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("spectrogram_uniforms"),
                size: std::mem::size_of::<SpectrogramUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
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
            cloud: None,
            cloud_ready: false,
        })
    }
}

/// The heatmap pipeline: a triangle list, blended exactly the way egui blends
/// its own shapes so the heatmap composites under the notes identically to the
/// tessellated mesh it replaces.
fn create_spectrogram_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::BindGroupLayout,
    extra_layout: Option<&wgpu::BindGroupLayout>,
    fragment: &str,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("spectrogram_shader"),
        source: wgpu::ShaderSource::Wgsl(SPECTROGRAM_SRC.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("spectrogram_pipeline_layout"),
        bind_group_layouts: &std::iter::once(Some(layout))
            .chain(extra_layout.map(Some))
            .collect::<Vec<_>>(),
        ..Default::default()
    });
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
            entry_point: Some(fragment),
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
        egui_encoder: &mut wgpu::CommandEncoder,
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
        let SpectrogramResources { layout, panes, cloud, .. } = resources;
        // A spectrogram's id is its surface (the docked pane, the Render
        // preview), and a closed tab simply stops calling back — so the panes
        // still being prepared are the only evidence of which ones exist.
        // Swept from whichever pane IS preparing, so a lone survivor still
        // clears the others.
        panes.evict_unseen(self.pass_nr);
        let pane = SpectrogramPane::get(panes, device, self.pane_id, self.pass_nr);
        pane.cloud_ready = false;

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
            // what lets one be kept above. This runs only on a refold or a
            // lost buffer.
            let mut staging = vec![0u8; size as usize];
            for j in 0..run_slabs {
                let at = slot_of(self.grid.first_key + j as i64, self.grid.capacity) as usize
                    * stride as usize;
                staging[at..at + bins].copy_from_slice(&self.grid.run[j * bins..(j + 1) * bins]);
            }
            queue.write_buffer(&buffer, 0, &staging);
            pane.grid = Some(GridBuffer { buffer, key });
        } else if !self.grid.dirty.is_empty() {
            let buffer = &pane.grid.as_ref().expect("the branch above holds a buffer").buffer;
            // Production slabs are already aligned. Only generic bin counts
            // need padding; an unchanged run needs no staging at all.
            let mut padded = (bins != stride as usize).then(|| vec![0u8; stride as usize]);
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
                let slab = &self.grid.run[j * bins..(j + 1) * bins];
                let bytes = match padded.as_mut() {
                    Some(padded) => {
                        padded[..bins].copy_from_slice(slab);
                        padded.as_slice()
                    }
                    None => slab,
                };
                let slot = slot_of(dirty, self.grid.capacity);
                queue.write_buffer(buffer, u64::from(slot) * u64::from(stride), bytes);
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
                        resource: wgpu::BindingResource::TextureView(&lut.view),
                    },
                ],
            }));
            // A retained filter must follow allocation changes even while
            // diffusion is disabled, before its next frame uses this grid.
            if let Some(target) = pane.cloud.as_mut() {
                target.rebind(device, layout, &grid.buffer, &lut.view);
            }
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
            spectrum_min_midi: self.read.spectrum_min_midi,
            bins_per_semitone: self.read.bins_per_semitone,
            level0: self.read.level0,
            level_per_step: self.read.level_per_step,
            level_per_midi: self.read.level_per_midi,
            rows: self.read.rows,
            bins: self.grid.bins,
            stride,
            capacity: self.grid.capacity,
            first_slot: slot_of(self.grid.first_key, self.grid.capacity),
            run_slabs: run_slabs as u32,
            _pad: [0; 3],
        };
        queue.write_buffer(&pane.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        if let Some(settings) = self
            .atmosphere
            .map(|mut atmosphere| {
                atmosphere.settings = atmosphere.settings.sanitized();
                atmosphere
            })
            // The measured picture is every effect at zero, and it takes the
            // plain pipeline: no target, no pass, nothing paid for a look that
            // is not being drawn. This is what the `Plain` style used to say.
            .filter(|a| !a.settings.effects().none())
        {
            let viewport = egui::epaint::ViewportInPixels::from_points(
                &self.rect,
                ppp,
                screen_descriptor.size_in_pixels,
            );
            let pixels = [viewport.width_px.max(0) as u32, viewport.height_px.max(0) as u32];
            let size = atmosphere::retained_size(
                atmosphere::source_size(pixels, ppp, settings),
                pixels,
                pane.cloud.as_ref().map(|c| c.size),
            );
            if size.iter().all(|&v| v > 0) {
                let cloud = cloud.get_or_insert_with(|| {
                    atmosphere::Pipelines::new(device, self.target_format, layout)
                });
                let rect = egui::Rect::from_min_size(
                    egui::pos2(viewport.left_px as f32 / ppp, viewport.top_px as f32 / ppp),
                    egui::vec2(pixels[0] as f32 / ppp, pixels[1] as f32 / ppp),
                );
                let grid = &pane.grid.as_ref().expect("drawable grid").buffer;
                let lut = &pane.lut.as_ref().expect("drawable gradient").view;
                let tone_size = atmosphere::tone_size(pixels, ppp, settings);
                let tile = atmosphere::tile_key(pixels, settings);
                // Any of the three sizes rebuilds the whole set, and that is
                // deliberate: nothing here is retained across frames — every
                // target is refilled every frame — so a rebuild costs an
                // allocation and no picture. The tone size moves on a pane
                // resize or a drag of `Cloud pixel size` and on nothing else,
                // where the LIGHT size follows the musical radius and would
                // otherwise be reallocated through every zoom and Span drag,
                // which is what `retained_size` is here to stop.
                //
                // The TILE is the exception to "refilled every frame": filling
                // it is a whole cell walk, tens of milliseconds. So it is
                // carried across a rebuild whenever its texels are unchanged —
                // a zoom reallocates the light around a tile that keeps its
                // bake — and `tile_owes` decides separately whether the walk in
                // it is still the one this frame wants.
                let texels = tile.map(atmosphere::TileKey::texels);
                let resize = pane.cloud.as_ref().is_none_or(|c| {
                    c.size != size || c.tone_size() != tone_size || c.tile_texels() != texels
                });
                if resize {
                    let carried = pane
                        .cloud
                        .take()
                        .filter(|held| held.tile_texels() == texels)
                        .and_then(atmosphere::Targets::into_tile);
                    let wanted = atmosphere::Allocation { size, tone: tone_size, tile, carried };
                    pane.cloud =
                        Some(atmosphere::Targets::new(device, cloud, wanted, layout, grid, lut));
                }
                let target = pane.cloud.as_mut().expect("allocated above");
                target.update(queue, uniforms, rect, ppp, settings, tile);
                // Terraces alone still need their transfer/composite, but the
                // one-pixel source would integrate the whole history only for
                // the composite to discard that expensive result. A cloud is
                // the case that needs the field WITHOUT a blur: it reads its
                // light out of these targets, so they are filled at zero
                // softness too, where each filter pass is a one-tap copy.
                if settings.settings.effects().light() {
                    {
                        #[cfg(test)]
                        target.encoded_passes.fetch_add(1, Ordering::Relaxed);
                        let mut pass =
                            egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("spectral_cloud_source"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &target.source_view,
                                    depth_slice: None,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                ..Default::default()
                            });
                        pass.set_pipeline(&cloud.source);
                        pass.set_bind_group(0, &target.source_group, &[]);
                        pass.set_vertex_buffer(0, pane.vertex_buffer.slice(..));
                        pass.draw(0..pane.count, 0..1);
                    }
                    target.blur(egui_encoder, cloud);
                    {
                        // Once filtering is finished, the raw source texture is
                        // free to hold the soft intensity. Bake across the whole
                        // spectrogram region so the Gaussian tail survives past
                        // the moving history edge. The raw detail keeps its measured mesh.
                        #[cfg(test)]
                        target.encoded_passes.fetch_add(1, Ordering::Relaxed);
                        let mut pass =
                            egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("spectral_cloud_material"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &target.source_view,
                                    depth_slice: None,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                ..Default::default()
                            });
                        pass.set_pipeline(&cloud.bake);
                        pass.set_bind_group(0, &target.source_group, &[]);
                        pass.set_bind_group(1, &target.bake_group, &[]);
                        pass.set_vertex_buffer(0, target.coverage_vertices.slice(..));
                        pass.draw(0..6, 0..1);
                    }
                    // One period of the cell walk, when `Cloud tile` asks for
                    // one and what is in the tile is not already it. Before the
                    // tone pass and the composite because both read it; it
                    // reads neither the light nor the pane, so where it sits
                    // among the light passes decides nothing.
                    if let Some(key) = tile.filter(|&key| target.tile_owes(key)) {
                        if let Some((views, group)) = target.tile_pass() {
                            #[cfg(test)]
                            target.encoded_passes.fetch_add(1, Ordering::Relaxed);
                            let attachment = |view| {
                                Some(wgpu::RenderPassColorAttachment {
                                    view,
                                    depth_slice: None,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })
                            };
                            let mut pass =
                                egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                    label: Some("spectral_cloud_tile"),
                                    color_attachments: &[
                                        attachment(&views[0]),
                                        attachment(&views[1]),
                                    ],
                                    ..Default::default()
                                });
                            pass.set_pipeline(&cloud.tile);
                            pass.set_bind_group(0, &target.source_group, &[]);
                            pass.set_bind_group(1, group, &[]);
                            pass.draw(0..3, 0..1);
                        }
                        target.tile_baked(key);
                    }
                    // The cloud's own tone, once per `Cloud pixel size` of pane
                    // rather than once per pixel of the composite. After the
                    // bake because it reads the finished material out of the
                    // same coverage quad, and only when the dial asks for a
                    // reduction — at the fresh size there is no target and the
                    // composite walks the cells itself.
                    if let Some(((tone_view, _), tone_group)) =
                        target.tone.as_ref().zip(target.tone_group.as_ref())
                    {
                        #[cfg(test)]
                        target.encoded_passes.fetch_add(1, Ordering::Relaxed);
                        let mut pass =
                            egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("spectral_cloud_tone"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: tone_view,
                                    depth_slice: None,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                ..Default::default()
                            });
                        pass.set_pipeline(&cloud.tone);
                        pass.set_bind_group(0, &target.source_group, &[]);
                        pass.set_bind_group(1, tone_group, &[]);
                        pass.set_vertex_buffer(0, target.coverage_vertices.slice(..));
                        pass.draw(0..6, 0..1);
                    }
                }
                pane.cloud_ready = true;
            }
        }

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
        let Some(pane) = resources.panes.get(self.pane_id) else {
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
        if pane.cloud_ready {
            let pipelines = resources.cloud.as_ref().expect("prepared cloud pipelines");
            let cloud = pane.cloud.as_ref().expect("prepared cloud");
            // The spectrogram's bed is black, including unwritten history.
            // Color the diffused intensity there first; then the measured mesh
            // replaces its own pixels with the unified core and soft field.
            render_pass.set_pipeline(&pipelines.backdrop);
            render_pass.set_bind_group(0, bind_group, &[]);
            render_pass.set_bind_group(1, &cloud.composite_group, &[]);
            render_pass.set_vertex_buffer(0, cloud.coverage_vertices.slice(..));
            render_pass.draw(0..6, 0..1);
            render_pass.set_pipeline(&pipelines.composite);
        } else {
            render_pass.set_pipeline(&resources.pipeline);
        }
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
/// `None` without an adapter only when GPU tests are optional; CI requires one.
pub struct SpectrogramHeadless {
    device: wgpu::Device,
    queue: wgpu::Queue,
    resources: CallbackResources,
    pass_nr: u64,
}

impl SpectrogramHeadless {
    pub fn new() -> Option<SpectrogramHeadless> {
        let (device, queue) = crate::gpu_harness::headless_device()?;
        Some(SpectrogramHeadless {
            device,
            queue,
            resources: CallbackResources::default(),
            pass_nr: 0,
        })
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
        self.pass_nr = self.pass_nr.wrapping_add(1);
        let callback = SpectrogramCallback {
            rect,
            atmosphere: None,
            vertices,
            grid,
            read,
            shades,
            target_format: format,
            pane_id,
            pass_nr: self.pass_nr,
        };
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
    use std::collections::HashMap;

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
    /// `min_midi`, at a pane spending `rows` pixels on pitch. The level
    /// mapping spends most of the 0..1 on the fixtures' own byte range and
    /// tilts with pitch, so a fragment that lands on the wrong bucket lands on
    /// the wrong colour.
    fn read_of(min_midi: f32, span: f32, rows: u32) -> SpectrogramRead {
        SpectrogramRead {
            min_midi,
            span,
            rows,
            spectrum_min_midi: SPECTRUM_MIN_MIDI,
            bins_per_semitone: BINS_PER_SEMITONE,
            level0: 0.0,
            level_per_step: 0.0035,
            level_per_midi: 0.002,
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
        full_quad_in(run_slabs, SIZE)
    }

    /// [`full_quad`] over a pane of the caller's own size, for a fixture that
    /// needs one — see [`rough_band_fixture`], which needs a pane the cloud's
    /// coarsest texture is still coarse on.
    fn full_quad_in(run_slabs: u32, size: [u32; 2]) -> Vec<SpectrogramVertex> {
        let (w, h) = (size[0] as f32, size[1] as f32);
        let n = run_slabs as f32;
        let v = |x: f32, y: f32| SpectrogramVertex { pos: [x, y], slab: x / w * n, t: 1.0 - y / h };
        vec![v(0.0, 0.0), v(w, 0.0), v(w, h), v(0.0, 0.0), v(w, h), v(0.0, h)]
    }

    /// The pane a callback draws over, in pixels — its own rect rather than
    /// [`SIZE`], so a fixture that wants a bigger pane gets one by saying so.
    /// Every fixture built by [`callback`] is [`SIZE`] and renders exactly as
    /// it always did.
    fn pane_of(cb: &SpectrogramCallback) -> [u32; 2] {
        [cb.rect.width() as u32, cb.rect.height() as u32]
    }

    /// The pitch fraction pixel row `py` of the frame samples, as
    /// [`full_quad`] lays the coordinate out.
    fn t_at(py: u32) -> f32 {
        1.0 - (py as f32 + 0.5) / SIZE[1] as f32
    }

    /// The read the shader performs, in Rust — the same arms, the same
    /// weights, the same blend in level space, indexing the run DIRECTLY
    /// rather than through a slot, so a slot rule that disagrees with the
    /// scatter shows up as a wrong column.
    struct Reference {
        grid: SpectrogramGrid,
        read: SpectrogramRead,
        shades: SpectrogramShades,
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

        /// Where a pitch fraction sits on the bucket axis.
        fn bucket_x(&self, t: f32) -> f32 {
            let midi = self.read.min_midi + t * self.read.span;
            (midi - self.read.spectrum_min_midi) * self.read.bins_per_semitone
        }

        /// The footprint a fragment at `t` covers, in bucket coordinates.
        fn foot(&self, t: f32) -> (f32, f32) {
            let half = 0.5 / self.read.rows as f32;
            (self.bucket_x(t - half), self.bucket_x(t + half))
        }

        /// The buckets a fragment at `t` covers, where it covers two or more —
        /// which is the arm as well as the run, since one bucket is the lerp.
        fn run_at(&self, t: f32) -> Option<std::ops::Range<usize>> {
            let (x0, x1) = self.foot(t);
            let top = self.bins() as f32 - 1.0;
            let idx = x0.floor().clamp(0.0, top) as usize;
            let last = x1.floor().clamp(0.0, top) as usize;
            (last > idx).then_some(idx..last + 1)
        }

        fn bucket_level(&self, j: usize, b: usize) -> f32 {
            let midi = self.read.spectrum_min_midi + (b as f32 + 0.5) / self.read.bins_per_semitone;
            let v = f32::from(self.stored(j, b));
            (self.read.level0 + self.read.level_per_step * v + self.read.level_per_midi * midi)
                .clamp(0.0, 1.0)
        }

        fn read_level(&self, j: usize, t: f32) -> f32 {
            let (x0, x1) = self.foot(t);
            let bins = self.bins();
            let top = bins as f32 - 1.0;
            let idx = x0.floor().clamp(0.0, top) as usize;
            let last = x1.floor().clamp(0.0, top) as usize;
            if last > idx {
                let lo = x0.clamp(0.0, bins as f32);
                let hi = x1.clamp(0.0, bins as f32);
                let (mut sum, mut total) = (0.0f32, 0.0f32);
                for b in idx..=last {
                    let w = (hi.min(b as f32 + 1.0) - lo.max(b as f32)).max(0.0);
                    sum += w * self.bucket_level(j, b);
                    total += w;
                }
                if total <= 0.0 {
                    return self.bucket_level(j, idx);
                }
                return sum / total;
            }
            let x = self.bucket_x(t) - 0.5;
            let b = x.floor().clamp(0.0, bins as f32 - 2.0) as usize;
            let f = (x - b as f32).clamp(0.0, 1.0);
            let (a, c) = (self.bucket_level(j, b), self.bucket_level(j, b + 1));
            a + (c - a) * f
        }

        fn color_at(&self, slab: f32, t: f32) -> [u8; 4] {
            let n = self.run_slabs();
            let jx = (slab - 0.5).floor().clamp(0.0, n as f32 - 1.0);
            let j0 = jx as usize;
            let j1 = (j0 + 1).min(n - 1);
            let fx = (slab - 0.5 - jx).clamp(0.0, 1.0);

            let (a, b) = (self.read_level(j0, t), self.read_level(j1, t));
            let level = a + (b - a) * fx;
            let levels = self.shades.lut.len();
            let i = ((level * levels as f32) as usize).min(levels - 1);
            let c = self.shades.lut[i];
            [c[0], c[1], c[2], 255]
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

        /// How many of the frame's sampled rows take each arm — `(mean,
        /// lerp)`. A fixture claiming an arm is checked against this rather
        /// than assumed to reach it.
        fn arms(&self) -> (usize, usize) {
            let mean = (0..SIZE[1]).filter(|&py| self.run_at(t_at(py)).is_some()).count();
            (mean, SIZE[1] as usize - mean)
        }

        /// How many sampled rows cover a run of at least `n` buckets.
        fn runs_at_least(&self, n: usize) -> usize {
            (0..SIZE[1])
                .filter(|&py| self.run_at(t_at(py)).is_some_and(|run| run.len() >= n))
                .count()
        }

        /// How many sampled rows take the LERP arm with its lower tap pinned
        /// at the top of the spectrum.
        fn top_clamped(&self) -> usize {
            (0..SIZE[1])
                .filter(|&py| {
                    let t = t_at(py);
                    self.run_at(t).is_none()
                        && (self.bucket_x(t) - 0.5).floor() >= self.bins() as f32 - 2.0
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
            rect: egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
            ),
            atmosphere: None,
            vertices,
            grid: grid.clone(),
            read: read.clone(),
            shades: shades(),
            target_format: FORMAT,
            pane_id: 0,
            pass_nr: 0,
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
        let screen = ScreenDescriptor { size_in_pixels: pane_of(cb), pixels_per_point: 1.0 };
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
        let size = pane_of(cb);
        let rect = cb.rect;
        let texture =
            render_to_texture(device, queue, size, cb.target_format, wgpu::Color::BLACK, |pass| {
                cb.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: 1.0,
                        screen_size_px: size,
                    },
                    pass,
                    resources,
                );
            });
        readback(device, queue, &texture, size)
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

    fn cloud_fixture() -> SpectrogramCallback {
        // Three device pixels of pitch over the middle third of the time
        // axis: wide enough to seed the quarter target, with dark room on
        // every side where only the new surrounding light can draw.
        let mut bytes = vec![0; 12 * BINS as usize];
        for slab in 4..8 {
            bytes[slab * BINS as usize + 500..slab * BINS as usize + 524].fill(255);
        }
        let grid = grid_of(Arc::new(bytes), BINS, 12, 0);
        let mut read = read_of(SPECTRUM_MIN_MIDI, 32.0, SIZE[1]);
        read.level0 = 0.0;
        read.level_per_step = 1.0 / 255.0;
        read.level_per_midi = 0.0;
        let mut cb = callback(full_quad(12), &grid, &read);
        cb.shades.lut =
            Arc::new((0..256).map(|v| [0, (v as f32 * 0.7) as u8, v as u8, 255]).collect());
        cb.atmosphere = Some(SpectrogramAtmosphere {
            // Pin the visible diffusion used by the pixel probes independently
            // of the fresh appearance's gentler setting.
            settings: harmonigraph_scene::SpectralAtmosphere {
                // The blur alone. The probes read the diffusion transfer;
                // terraces or a cloud over them would move the very pixels
                // they measure.
                contour_strength: 0.0,
                cloud_depth: 0.0,
                ..Default::default()
            },
            region: cb.rect,
            pitch_vertical: true,
            points_per_cent: 0.03,
            points_per_ms: 0.01,
            now: 0.0,
        });
        cb
    }

    /// Every effect at zero, which is the measured heatmap — what selecting the
    /// `Plain` style used to mean, now that the three effects are dials.
    fn every_effect_off(cb: &mut SpectrogramCallback) {
        let settings = &mut cb.atmosphere.as_mut().unwrap().settings;
        settings.pitch_softness = 0.0;
        settings.time_softness = 0.0;
        settings.contour_strength = 0.0;
        settings.cloud_depth = 0.0;
    }

    #[test]
    fn lava_preserves_silence_quiet_fields_and_nested_levels() {
        let Some((device, queue)) = headless_device() else { return };
        let mut cb = cloud_fixture();
        cb.atmosphere.as_mut().unwrap().settings.contour_strength = 1.0;
        for (smooth, contours) in [(false, 7.0), (true, 7.0), (false, 64.0), (true, 64.0)] {
            cb.atmosphere.as_mut().unwrap().settings.contours = contours;
            cb.atmosphere.as_mut().unwrap().settings.pitch_softness =
                if smooth { 35.0 } else { 0.0 };
            cb.atmosphere.as_mut().unwrap().settings.time_softness =
                if smooth { 120.0 } else { 0.0 };
            let mut previous = 0;
            for value in [0, 1, 3, 8, 13, 20, 64, 96, 128, 160, 192, 224, 255] {
                cb.grid.run = Arc::new(vec![value; cb.grid.run.len()]);
                let frame = fresh_frame(&device, &queue, &cb);
                let blue = frame[(64 * 128 + 64) * 4 + 2];
                if value == 0 {
                    assert_eq!(blue, 0);
                } else if value > 1 {
                    assert!(blue > previous, "quiet and nested levels survive: {value}");
                }
                // The first byte can round to black after the palette's
                // half-sample interpolation; allow one output byte of slack.
                if value <= 13 {
                    assert!(
                        f32::from(blue) + 1.0 >= f32::from(value) * 0.7,
                        "first contour collapsed quiet intensity: {value} -> {blue}"
                    );
                }
                previous = blue;
                for y in 0..128 {
                    for x in 0..128 {
                        assert!(
                            frame[(y * 128 + x) * 4 + 2].abs_diff(blue) <= 1,
                            "flat field gained texture"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn float_output_uses_linear_palette_for_every_style() {
        let Some((device, queue)) = headless_device() else { return };
        // The three pipelines a float target can be drawn through: the plain
        // heatmap with every effect at zero, the composite over a blurred
        // field, and the same composite with the terrace transfer in it.
        for (style, soft, contour_strength) in
            [("measured", false, 0.0), ("blurred", true, 0.0), ("terraced", true, 1.0)]
        {
            let mut cb = cloud_fixture();
            cb.target_format = wgpu::TextureFormat::Rgba16Float;
            cb.grid.run = Arc::new(vec![96; cb.grid.run.len()]);
            cb.shades.lut = Arc::new(vec![[128, 128, 128, 255]; 256]);
            let settings = &mut cb.atmosphere.as_mut().unwrap().settings;
            settings.contour_strength = contour_strength;
            if !soft {
                settings.pitch_softness = 0.0;
                settings.time_softness = 0.0;
            }
            let mut resources = CallbackResources::default();
            prepare_once(&device, &queue, &mut resources, &cb);
            let texture = render_to_texture(
                &device,
                &queue,
                SIZE,
                cb.target_format,
                wgpu::Color::BLACK,
                |pass| {
                    cb.paint(
                        egui::PaintCallbackInfo {
                            viewport: cb.rect,
                            clip_rect: cb.rect,
                            pixels_per_point: 1.0,
                            screen_size_px: SIZE,
                        },
                        pass,
                        &resources,
                    );
                },
            );
            // One actual float pixel distinguishes linear 0.216 from an
            // erroneously gamma-encoded 0.502; an RGBA8 readback cannot.
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("float_palette_pixel"),
                size: 8,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            let mut source = texture.as_image_copy();
            source.origin = wgpu::Origin3d { x: 64, y: 64, z: 0 };
            encoder.copy_texture_to_buffer(
                source,
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: None,
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            );
            queue.submit([encoder.finish()]);
            let slice = buffer.slice(..);
            slice.map_async(wgpu::MapMode::Read, |r| r.expect("map float pixel"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            let mapped = slice.get_mapped_range();
            let expected = ((128.0_f32 / 255.0 + 0.055) / 1.055).powf(2.4);
            for channel in mapped[..6].chunks_exact(2) {
                let half = u16::from_le_bytes([channel[0], channel[1]]);
                // The fixture must produce positive normal half-floats.
                assert!((0x0400..0x7c00).contains(&half));
                let actual = f32::from_bits((u32::from(half) << 13) + 0x3800_0000);
                assert!((actual - expected).abs() < 0.001, "{style}: {actual} != {expected}");
            }
        }
    }

    #[test]
    fn zero_width_lava_skips_offscreen_passes_at_live_capacity() {
        let Some((device, queue)) = headless_device() else { return };
        let mut resources = CallbackResources::default();
        let mut cb = cloud_fixture();
        frame_with(&device, &queue, &mut resources, &cb);
        assert_eq!(
            resources
                .get::<SpectrogramResources>()
                .unwrap()
                .panes
                .get(0)
                .expect("the spectrogram prepared a pane")
                .cloud
                .as_ref()
                .unwrap()
                .encoded_passes
                .load(Ordering::Relaxed),
            6,
            "the counter must observe actual source, filter and bake passes"
        );
        // The prior one-pixel source integrated every slab and visible bin:
        // reach the live cap with the production bin count, not a tiny grid.
        let bins = harmonigraph_core::spectrum::SPECTRUM_BINS as u32;
        cb.grid = grid_of(Arc::new(vec![96; 1024 * bins as usize]), bins, 1024, 0);
        cb.vertices = full_quad(1024);
        cb.read.span = bins as f32 / BINS_PER_SEMITONE;
        cb.atmosphere.as_mut().unwrap().settings.contour_strength = 1.0;
        for width in [0.0, -1.0] {
            cb.atmosphere.as_mut().unwrap().settings.pitch_softness = width;
            cb.atmosphere.as_mut().unwrap().settings.time_softness = width;
            let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
            let mut encoder = device.create_command_encoder(&Default::default());
            let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
            let pane = resources
                .get::<SpectrogramResources>()
                .unwrap()
                .panes
                .get(0)
                .expect("the spectrogram prepared a pane");
            assert!(pane.cloud_ready, "Lava must still use its transfer/composite");
            assert_eq!(
                pane.cloud.as_ref().unwrap().encoded_passes.load(Ordering::Relaxed),
                0,
                "zero widths encoded unused history integration/filter passes"
            );
            // Check before submission so a regression cannot run millions of
            // bucket reads in one fragment before this assertion reports it.
            queue.submit(bufs.into_iter().chain([encoder.finish()]));
        }
        let lava = frame_with(&device, &queue, &mut resources, &cb);
        cb.atmosphere = None;
        assert_ne!(lava, fresh_frame(&device, &queue, &cb), "zero widths disabled Lava contours");
    }

    #[test]
    fn held_newest_slab_and_zero_softness_preserve_the_field() {
        let Some((device, queue)) = headless_device() else { return };
        let mut cb = cloud_fixture();
        cb.grid = grid_of(Arc::new(vec![96; 128 * BINS as usize]), BINS, 128, 0);
        for v in &mut cb.vertices {
            v.slab = 100.5;
        }
        let held = fresh_frame(&device, &queue, &cb);
        assert!((95..=97).contains(&held[(64 * 128 + 64) * 4 + 2]), "held strip became black");
        let mut raw = cloud_fixture();
        let settings = &mut raw.atmosphere.as_mut().unwrap().settings;
        settings.pitch_softness = 0.0;
        settings.time_softness = 0.0;
        let zero = fresh_frame(&device, &queue, &raw);
        raw.atmosphere = None;
        assert!(zero == fresh_frame(&device, &queue, &raw), "zero widths added smoothing");
    }

    #[test]
    fn wide_musical_blur_integrates_periodic_broadband_without_phase_aliasing() {
        let Some((device, queue)) = headless_device() else { return };
        let mut cb = cloud_fixture();
        cb.vertices = full_quad(512);
        cb.atmosphere.as_mut().unwrap().settings.time_softness = 2000.0;
        cb.atmosphere.as_mut().unwrap().points_per_ms = 0.128;
        let mut frames = Vec::new();
        for phase in 0..2 {
            let bytes = (0..512)
                .flat_map(|slab| vec![if (slab + phase) % 2 == 0 { 0 } else { 255 }; BINS as usize])
                .collect();
            cb.grid = grid_of(Arc::new(bytes), BINS, 512, 0);
            frames.push(fresh_frame(&device, &queue, &cb));
        }
        assert!(compare(&frames[0], &frames[1]).0 <= 2, "wide source aliased alternating columns");
        let blue = frames[0][(64 * 128 + 64) * 4 + 2];
        // Half zero and half one average to 0.5 in the encoded domain.
        // Encoding after source reduction would incorrectly remain at 0.5
        // display intensity instead of this brighter decoded result.
        let expected = (255.0_f32 / (0.1 + 1.81_f32.sqrt())).round() as u8;
        assert!(blue.abs_diff(expected) <= 2, "all encoded columns contribute, got {blue}");
    }

    #[test]
    fn density_weights_bright_buckets_before_pitch_source_reduction() {
        let Some((device, queue)) = headless_device() else { return };
        let mut cb = cloud_fixture();
        // One bright bucket per period: both equal mixtures and narrow
        // bright bands must survive averaging over many source buckets.
        for period in [2, 8] {
            let bytes =
                (0..12 * BINS as usize).map(|i| if i % period == 0 { 255 } else { 0 }).collect();
            cb.grid.run = Arc::new(bytes);
            let encoded_mean = 1.0 / period as f32;
            let expected = (255.0 * 2.0 * encoded_mean / (0.1 + (0.01 + 3.6 * encoded_mean).sqrt()))
                .round() as u8;
            for width in [140.0, 280.0] {
                cb.atmosphere.as_mut().unwrap().settings.pitch_softness = width;
                let frame = fresh_frame(&device, &queue, &cb);
                let blue = frame[(64 * 128 + 64) * 4 + 2];
                assert!(
                    blue.abs_diff(expected) <= 2,
                    "pitch source averaged before encoding: {blue}, expected {expected}"
                );
            }
        }
    }

    #[test]
    fn golden_spectrogram_styles_broadband_quiet_and_silence() {
        let Some((device, queue)) = headless_device() else { return };
        let mut cb = cloud_fixture();
        // A broad smooth field, quiet band, granular broadband patch, and black
        // margins. No peak recognition can explain these shapes.
        let mut bytes = vec![0; cb.grid.run.len()];
        for slab in 1..11 {
            for bin in 80..944 {
                let y = bin as f32 / 1024.0;
                let x = slab as f32 / 12.0;
                let blob = (1.0 - ((x - 0.55).powi(2) * 7.0 + (y - 0.55).powi(2) * 5.0)).max(0.0);
                let level = if bin < 220 { 0.13 } else { blob * 0.8 };
                let grain = if slab < 5 && bin > 650 {
                    ((bin * 17 + slab * 31) % 13) as f32 / 50.0
                } else {
                    0.0
                };
                bytes[slab * BINS as usize + bin] = ((level + grain).min(1.0) * 255.0) as u8;
            }
        }
        cb.grid.run = Arc::new(bytes);
        // The three looks the retired style enum named, each reached by its
        // dials, against the frames that enum drew: the blur the fixture pins,
        // the same with the terraces at full strength, and everything off.
        let gate = harmonigraph_golden::Gate::new(env!("CARGO_MANIFEST_DIR"));
        gate.check("spectrogram-style-blur", SIZE, &fresh_frame(&device, &queue, &cb));
        cb.atmosphere.as_mut().unwrap().settings.contour_strength = 1.0;
        gate.check("spectrogram-style-lava", SIZE, &fresh_frame(&device, &queue, &cb));
        every_effect_off(&mut cb);
        gate.check("spectrogram-style-plain", SIZE, &fresh_frame(&device, &queue, &cb));
    }

    #[test]
    fn spectral_clouds_light_the_surroundings_and_leave_silence_dark() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let mut cb = cloud_fixture();
        let lit = fresh_frame(&device, &queue, &cb);
        let mut plain = cloud_fixture();
        plain.atmosphere = None;
        let core = fresh_frame(&device, &queue, &plain);
        let pixel = |frame: &[u8], x, y| frame[(y * SIZE[0] as usize + x) * 4 + 2];
        assert_eq!(pixel(&core, 64, 56), 0, "fixture put core ink in the halo probe");
        assert!(pixel(&lit, 64, 56) > 3, "no light outside the measured ridge");
        assert_eq!(pixel(&core, 64, 63), 255, "fixture missed its narrow ridge");
        assert!(pixel(&lit, 64, 63) > 4 * pixel(&lit, 64, 56), "diffusion lost the pitch ridge");
        assert!(pixel(&lit, 64, 63) < pixel(&core, 64, 63), "bright ridge bypassed diffusion");
        assert_eq!(lit, fresh_frame(&device, &queue, &cb), "paused clouds moved");
        cb.target_format = wgpu::TextureFormat::Rgba8UnormSrgb;
        assert!(
            compare(&lit, &fresh_frame(&device, &queue, &cb)).0 <= 1,
            "sRGB target changed the cloud material"
        );
        cb.target_format = FORMAT;
        cb.grid.run = Arc::new(vec![0; cb.grid.run.len()]);
        let silent = fresh_frame(&device, &queue, &cb);
        assert!(silent.chunks_exact(4).all(|p| p == [0, 0, 0, 255]), "silence emitted light");
    }

    #[test]
    fn spectral_diffusion_colors_the_combined_intensity_once() {
        let Some((device, queue)) = headless_device() else { return };
        let mut cb = cloud_fixture();
        // A curved ramp exposes RGB mixing: every correctly colored pixel
        // must lie on red = green squared, including the dim cloud tail.
        cb.shades.lut = Arc::new(
            (0..256).map(|v| [((v * v) as f32 / 255.0).round() as u8, v as u8, 0, 255]).collect(),
        );
        let frame = fresh_frame(&device, &queue, &cb);
        let mut body_pixels = 0;
        for p in frame.chunks_exact(4) {
            let expected = (f32::from(p[1]).powi(2) / 255.0).round() as u8;
            assert!(p[0].abs_diff(expected) <= 2, "cloud left the intensity palette: {p:?}");
            body_pixels += usize::from(p[1] > 8 && p[1] < 240);
        }
        assert!(body_pixels > 200, "fixture never reached the diffused body");
    }

    #[test]
    fn spectral_diffusion_removes_bright_grain_in_both_axes() {
        let Some((device, queue)) = headless_device() else { return };
        for temporal in [false, true] {
            let mut phases = Vec::new();
            for phase in 0..2 {
                let mut cb = cloud_fixture();
                // A four-pixel stripe period, softened with a four-pixel
                // Gaussian sigma in either axis. Musical controls explicitly
                // provide that width; the source has no hidden quarter-res blur.
                // Isolate the close field: the wide field intentionally carries
                // the differently phased band edges into these interior probes.
                cb.atmosphere.as_mut().unwrap().settings.spread = 0.0;
                cb.atmosphere.as_mut().unwrap().settings.pitch_softness = 140.0;
                cb.atmosphere.as_mut().unwrap().settings.time_softness = 400.0;
                let mut bytes = vec![0; 128 * BINS as usize];
                for slab in 0..128 {
                    for bucket in 256..768 {
                        let stripe = if temporal { slab } else { bucket / 8 };
                        if matches!((stripe + phase * 2) % 4, 1 | 2) {
                            bytes[slab * BINS as usize + bucket] = 255;
                        }
                    }
                }
                cb.grid = grid_of(Arc::new(bytes), BINS, 128, 0);
                cb.vertices = full_quad(128);
                // The measured grain first, then the same picture softened.
                let soft = fresh_frame(&device, &queue, &cb);
                every_effect_off(&mut cb);
                phases.push([fresh_frame(&device, &queue, &cb), soft]);
            }
            // Stay inside the band, away from the history/filter boundaries.
            let difference = |setting: usize| -> u32 {
                (48..80)
                    .flat_map(|y| (32..96).map(move |x| (y * 128 + x) * 4 + 2))
                    .map(|i| u32::from(phases[0][setting][i].abs_diff(phases[1][setting][i])))
                    .sum()
            };
            let raw = difference(0);
            assert!(raw > 2048 * 100, "fixture missed bright grain, temporal={temporal}");
            assert!(
                difference(1) <= 2048,
                "Gaussian retained fine phase, temporal={temporal}, difference={}",
                difference(1)
            );
            for phase in &phases {
                let full = &phase[1];
                assert!(full[(64 * 128 + 64) * 4 + 2] > 80, "diffusion erased the pitch band");
                assert!(full[(8 * 128 + 64) * 4 + 2] < 5, "diffusion lost the band separation");
            }
        }
    }

    #[test]
    fn spectral_diffusion_softens_faint_detail_with_one_fade_to_the_palettes_floor() {
        let Some((device, queue)) = headless_device() else { return };
        let mut cb = cloud_fixture();
        cb.grid.run = Arc::new(cb.grid.run.iter().map(|&v| if v > 0 { 102 } else { 0 }).collect());
        // An edited palette may start above black, and then the diffused tail
        // has to land on THAT and stop, not carry on down to a black the scheme
        // never named. What the tail owes is an end, which the monotone check
        // below is: one fade, no moat, no second floor under the first.
        Arc::make_mut(&mut cb.shades.lut)[0] = [80, 0, 0, 255];
        let soft = fresh_frame(&device, &queue, &cb);
        every_effect_off(&mut cb);
        let zero = fresh_frame(&device, &queue, &cb);
        cb.atmosphere = None;
        let plain = fresh_frame(&device, &queue, &cb);
        assert_eq!(zero, plain, "every effect at zero did not restore the measured heatmap");
        let pixel = |frame: &[u8], y| frame[(y * SIZE[0] as usize + 64) * 4 + 2];
        assert_eq!(pixel(&plain, 63), 102, "fixture missed the faint ridge");
        assert!(pixel(&soft, 63) < 82, "faint grain kept its original contrast");
        assert!(pixel(&soft, 60) > pixel(&plain, 60), "softened body never formed");
        let far = (16 * SIZE[0] as usize + 64) * 4;
        assert_eq!(
            &soft[far..far + 4],
            &[80, 0, 0, 255],
            "diffusion left the background off the palette's own floor"
        );
        for y in 16..63 {
            assert!(
                pixel(&soft, y) <= pixel(&soft, y + 1).saturating_add(1),
                "dark moat before the ridge at row {y}"
            );
        }
    }

    #[test]
    fn spectral_clouds_extend_past_the_scrolling_history_edge() {
        let Some((device, queue)) = headless_device() else { return };
        for turns in 0..4 {
            let mut cb = cloud_fixture();
            // History starts inside the pane. A ten-pixel ridge seeds enough
            // scalar density for a visible tail six pixels beyond that edge.
            let mut bytes = vec![0; cb.grid.run.len()];
            for slab in 0..4 {
                bytes[slab * BINS as usize + 472..slab * BINS as usize + 552].fill(255);
            }
            cb.grid.run = Arc::new(bytes);
            for vertex in &mut cb.vertices {
                vertex.pos[0] = 40.0 + vertex.pos[0] * 0.375;
                for _ in 0..turns {
                    vertex.pos = [SIZE[1] as f32 - vertex.pos[1], vertex.pos[0]];
                }
            }
            cb.atmosphere.as_mut().unwrap().pitch_vertical = turns % 2 == 0;
            let mut resources = CallbackResources::default();
            let lit = frame_with(&device, &queue, &mut resources, &cb);
            if turns == 0 {
                cb.target_format = wgpu::TextureFormat::Rgba8UnormSrgb;
                assert!(
                    compare(&lit, &fresh_frame(&device, &queue, &cb)).0 <= 1,
                    "sRGB target changed the light beyond recorded history"
                );
                cb.target_format = FORMAT;
            }
            let mut corners = [egui::pos2(36.0, 0.0), egui::pos2(112.0, 128.0)];
            for corner in &mut corners {
                for _ in 0..turns {
                    *corner = egui::pos2(SIZE[1] as f32 - corner.y, corner.x);
                }
            }
            cb.atmosphere.as_mut().unwrap().region =
                egui::Rect::from_two_pos(corners[0], corners[1]);
            let bounded = frame_with(&device, &queue, &mut resources, &cb);
            cb.atmosphere = None;
            let core = fresh_frame(&device, &queue, &cb);
            let pixel = |frame: &[u8], mut x: usize, mut y: usize| {
                for _ in 0..turns {
                    (x, y) = (SIZE[1] as usize - 1 - y, x);
                }
                frame[(y * SIZE[0] as usize + x) * 4 + 2]
            };
            assert!(pixel(&lit, 34, 63) > 4, "fixture did not reach the region boundary");
            assert_eq!(pixel(&bounded, 34, 63), 0, "cloud crossed into the analyzer region");
            assert!(pixel(&bounded, 38, 63) > 4, "updating the region lost its history tail");
            assert_eq!(pixel(&core, 34, 63), 0, "fixture smeared data beyond history");
            assert!(pixel(&lit, 34, 63) > 4, "cloud cropped at history edge, turn {turns}");
            assert_eq!(pixel(&lit, 8, 63), 0, "cloud did not decay into the empty history");
            assert_eq!(pixel(&core, 46, 63), 255, "fixture missed the measured ridge");
            assert!(pixel(&lit, 46, 63) > 128, "diffusion lost the measured pitch band");
            for x in 8..46 {
                assert!(
                    pixel(&lit, x, 63) <= pixel(&lit, x + 1, 63).saturating_add(1),
                    "dark moat at history edge, turn {turns}, column {x}"
                );
            }
        }
    }

    #[test]
    fn spectral_diffusion_adds_no_texture_or_analyzer_coupling() {
        let Some((device, queue)) = headless_device() else { return };
        let mut cb = cloud_fixture();
        cb.grid.run = Arc::new(vec![96; cb.grid.run.len()]);
        let smooth = fresh_frame(&device, &queue, &cb);
        // A broad uniform field must stay uniform. Keep the probes beyond
        // the wide filter's reach from the image edges.
        for y in 32..96 {
            for x in 32..96 {
                let blue = smooth[(y * 128 + x) * 4 + 2];
                assert!((95..=97).contains(&blue), "texture modulated the field at {x},{y}");
            }
        }
        cb.atmosphere.as_mut().unwrap().settings.analyzer_softness = 0.0;
        cb.atmosphere.as_mut().unwrap().settings.note_glow = 0.0;
        assert_eq!(smooth, fresh_frame(&device, &queue, &cb));
    }

    #[test]
    fn spectral_cloud_targets_survive_pitch_zoom_and_span_drags() {
        let Some((device, queue)) = headless_device() else { return };
        for pitch in [false, true] {
            let mut resources = CallbackResources::default();
            let mut cb = cloud_fixture();
            let settings = cb.atmosphere.as_mut().unwrap();
            settings.points_per_cent = 0.128;
            settings.points_per_ms = 0.04;
            let mut previous = None;
            let mut previous_requested = None;
            let mut allocations = 0;
            let mut exact_allocations = 0;
            // Two-second drags at 60 Hz in each direction, moving 0.3% per
            // frame. Reversing also traverses every prior resize boundary.
            for step in (0..120).chain((0..120).rev()) {
                let scale = 1.003_f32.powi(step);
                let settings = cb.atmosphere.as_mut().unwrap();
                if pitch {
                    settings.points_per_cent = 0.128 * scale;
                } else {
                    settings.points_per_ms = 0.04 / scale;
                }
                let requested = atmosphere::source_size(SIZE, 1.0, *settings);
                exact_allocations += usize::from(previous_requested != Some(requested));
                previous_requested = Some(requested);
                frame_with(&device, &queue, &mut resources, &cb);
                let target = resources
                    .get::<SpectrogramResources>()
                    .unwrap()
                    .panes
                    .get(0)
                    .expect("the spectrogram prepared a pane")
                    .cloud
                    .as_ref()
                    .unwrap();
                allocations += usize::from(previous.as_ref() != Some(&target.source_view));
                previous = Some(target.source_view.clone());
                for (held, requested) in target.size.into_iter().zip(requested) {
                    assert!(held * 10 >= requested * 9 && held * 10 <= requested * 11);
                }
            }
            eprintln!("pitch={pitch}: {allocations} target allocations vs {exact_allocations} exact-size allocations over 240 drag frames");
            assert!(exact_allocations > 25, "fixture did not exercise size churn");
            assert!(allocations <= 8 && allocations * 5 < exact_allocations);
        }
    }

    #[test]
    fn spectral_cloud_targets_refresh_after_palette_resize_disable_and_empty_frames() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let mut resources = CallbackResources::default();
        let mut cb = cloud_fixture();
        let first = frame_with(&device, &queue, &mut resources, &cb);
        cb.shades.generation += 1;
        cb.shades.lut = Arc::new(cb.shades.lut.iter().map(|c| [c[2], c[1], 0, 255]).collect());
        let recolored = frame_with(&device, &queue, &mut resources, &cb);
        assert_ne!(first, recolored, "palette change retained old cloud colors");
        assert_eq!(recolored, fresh_frame(&device, &queue, &cb));
        cb.rect.max.y *= 0.5;
        for vertex in &mut cb.vertices {
            vertex.pos[1] *= 0.5;
        }
        let smaller = frame_with(&device, &queue, &mut resources, &cb);
        assert_eq!(smaller, fresh_frame(&device, &queue, &cb));
        assert_eq!(
            resources
                .get::<SpectrogramResources>()
                .unwrap()
                .panes
                .get(0)
                .expect("the spectrogram prepared a pane")
                .cloud
                .as_ref()
                .unwrap()
                .size,
            atmosphere::source_size([128, 64], 1.0, cb.atmosphere.unwrap())
        );
        let mut other = cloud_fixture();
        other.pane_id = 1;
        frame_with(&device, &queue, &mut resources, &other);
        assert_eq!(
            smaller,
            frame_with(&device, &queue, &mut resources, &cb),
            "unequal pane replaced this cloud target"
        );
        every_effect_off(&mut cb);
        // Mode or viewport changes can replace the grid while diffusion is
        // disabled. Re-enabling at the same pane size must use that new grid.
        cb.grid.capacity *= 2;
        cb.grid.generation += 1;
        cb.grid.run = Arc::new(vec![64; cb.grid.run.len()]);
        let disabled = frame_with(&device, &queue, &mut resources, &cb);
        cb.atmosphere = None;
        assert_eq!(
            disabled,
            fresh_frame(&device, &queue, &cb),
            "every effect at zero changed the original heatmap"
        );
        cb.atmosphere = other.atmosphere;
        assert_eq!(
            frame_with(&device, &queue, &mut resources, &cb),
            fresh_frame(&device, &queue, &cb),
            "re-enabled smoothing retained the replaced grid"
        );
        cb.vertices.clear();
        let empty = frame_with(&device, &queue, &mut resources, &cb);
        assert!(empty.chunks_exact(4).all(|p| p == [0, 0, 0, 255]));
        assert!(
            !resources
                .get::<SpectrogramResources>()
                .unwrap()
                .panes
                .get(0)
                .expect("the spectrogram prepared a pane")
                .cloud_ready
        );
    }

    #[test]
    fn spectral_clouds_follow_offset_panes_at_both_pixel_scales() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        for ppp in [1.0, 2.0] {
            let size = [256, 256];
            let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: ppp };
            let mut resources = CallbackResources::default();
            let mut cb = cloud_fixture();
            cb.rect = egui::Rect::from_min_size(egui::pos2(0.24, 0.24), egui::vec2(96.49, 96.49));
            cb.atmosphere.as_mut().unwrap().region = cb.rect;
            cb.read.rows = (96.0 * ppp) as u32;
            for v in &mut cb.vertices {
                v.pos = v.pos.map(|x| x * 96.49 / 128.0 + 0.24);
            }
            let mut draw = |cb: &SpectrogramCallback| {
                let mut encoder = device.create_command_encoder(&Default::default());
                let commands = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
                queue.submit(commands.into_iter().chain([encoder.finish()]));
                let target =
                    render_to_texture(&device, &queue, size, FORMAT, wgpu::Color::BLACK, |pass| {
                        cb.paint(
                            egui::PaintCallbackInfo {
                                viewport: cb.rect,
                                clip_rect: cb.rect,
                                pixels_per_point: ppp,
                                screen_size_px: size,
                            },
                            pass,
                            &resources,
                        );
                    });
                readback(&device, &queue, &target, size)
            };
            let origin = draw(&cb);
            let offset = egui::vec2(8.0, 12.0);
            cb.rect = cb.rect.translate(offset);
            cb.atmosphere.as_mut().unwrap().region = cb.rect;
            for v in &mut cb.vertices {
                v.pos[0] += offset.x;
                v.pos[1] += offset.y;
            }
            let shifted = draw(&cb);
            let dx = (offset.x * ppp) as usize;
            let dy = (offset.y * ppp) as usize;
            let side = (96.0 * ppp) as usize;
            let mut worst = 0;
            for y in 1..side - 1 {
                for x in 1..side - 1 {
                    for channel in 0..4 {
                        let a = origin[(y * 256 + x) * 4 + channel];
                        let b = shifted[((y + dy) * 256 + x + dx) * 4 + channel];
                        worst = worst.max(a.abs_diff(b));
                    }
                }
            }
            assert!(
                worst <= 1,
                "offset pane moved the cloud against its source at {ppp}×: {worst}"
            );
        }
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
        for (source, required) in [
            (SPECTROGRAM_SRC, SPECTROGRAM_ENTRY_POINTS),
            (
                atmosphere::SOURCE,
                &["vs_fullscreen", "fs_close_h", "fs_close_v", "fs_wide_h", "fs_wide_v"][..],
            ),
        ] {
            let module = naga::front::wgsl::parse_str(source)
                .map_err(|e| e.emit_to_string(source))
                .expect("spectral shaders must parse");
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .expect("spectral shaders must validate");
            for required in required {
                assert!(
                    module.entry_points.iter().any(|ep| ep.name == *required),
                    "missing entry point `{required}`"
                );
            }
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

    /// Every pixel wider than a bucket: the shader's MINIFYING arm against the
    /// same area-weighted mean written in Rust, over runs of seven buckets.
    ///
    /// The arm is the one thing a pure tone cannot measure, so the fixture is
    /// a noise bed with peaks in it — see [`noisy_grid`] — and the arm counts
    /// are checked below rather than assumed from the span.
    #[test]
    fn a_pixel_wider_than_a_bucket_reads_the_mean_of_the_buckets_under_it() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let run = noisy_grid(BINS as usize, 6);
        let reference = Reference {
            grid: grid_of(run, BINS, 8, 0),
            read: read_of(18.0, 24.0, SIZE[1]),
            shades: shades(),
        };
        let rows = SIZE[1] as usize;
        let (mean, lerp) = reference.arms();
        assert_eq!((mean, lerp), (rows, 0), "every pixel of this fixture must take the mean");
        assert_eq!(reference.runs_at_least(7), rows, "the runs must be wide enough to average");

        let (worst, exact) = parity(&device, &queue, &reference);
        assert!(worst <= 1, "channels differ by {worst} levels, not the blend's own rounding");
        assert!(exact >= 0.99, "only {:.4} of pixels are exact", exact);
    }

    /// Pixels narrower than a bucket: the MAGNIFYING arm, with the half-bucket
    /// centre offset and the clamp that keeps its upper tap in the spectrum.
    ///
    /// A footprint narrower than a bucket still STRADDLES one now and then, so
    /// this fixture reaches both arms and the counts below say in what
    /// proportion — a shader that took the mean everywhere would fail on the
    /// majority.
    ///
    /// The span is what puts a straddle anywhere at all. The frame samples the
    /// pitch axis on a fixed grid, so a span that makes the bucket spacing a
    /// tidy fraction of the pixel spacing (2 semitones is half a bucket per
    /// pixel) gives every sample the same two phases against the buckets and
    /// the mean arm is reached by nothing.
    #[test]
    fn a_pixel_narrower_than_a_bucket_reads_between_the_two_under_it() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let run = noisy_grid(BINS as usize, 6);
        let reference = Reference {
            grid: grid_of(run, BINS, 8, 0),
            read: read_of(20.0, 1.7, 512),
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
    /// clamp — the footprint's buckets at the last one, and the lerp's lower
    /// tap pinned at `bins - 2` so its upper tap still exists.
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
        assert!(clamped > 10, "only {clamped} sampled rows reach the clamp at the top bucket");
        assert!(clamped < SIZE[1] as usize, "the fixture must also read inside");

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
        for bins in [256, harmonigraph_core::spectrum::SPECTRUM_BINS, 3827, 3829] {
            check_delta_upload(&device, &queue, bins);
        }
    }

    fn check_delta_upload(device: &wgpu::Device, queue: &wgpu::Queue, bins: usize) {
        let read = SpectrogramRead {
            level_per_midi: 0.0,
            ..read_of(SPECTRUM_MIN_MIDI, bins as f32 / BINS_PER_SEMITONE, 96)
        };
        let mut version: HashMap<i64, u32> = HashMap::new();
        let mut resources = CallbackResources::default();
        let mut previous: Option<Vec<u8>> = None;
        let mut moved = 0;
        let uploaded: Arc<AtomicU64> = Arc::default();

        let steps = &[
            Step::new("the first upload", 1, 8, 0, &[], &[]),
            Step::new("an unchanged warm run", 1, 8, 0, &[], &[]),
            Step::new("the newest slab rewritten", 1, 8, 0, &[5], &[5]),
            Step::new("an interior slab rewritten", 1, 8, 0, &[2], &[2]),
            Step::new("the window advanced by one", 1, 8, 1, &[], &[6]),
            Step::new("advanced again", 1, 8, 2, &[], &[7]),
            Step::new("advanced past capacity", 1, 8, 4, &[], &[8, 9]),
            Step::new("a wrapped slab rewritten", 1, 8, 4, &[9], &[9]),
            Step::new("three wrapped slabs rewritten", 1, 8, 4, &[7, 8, 9], &[7, 8, 9]),
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
            let incremental = frame_with(device, queue, &mut resources, &cb);
            assert_eq!(
                uploaded.load(Ordering::Relaxed),
                serial,
                "{} drew without acknowledging its run",
                step.label
            );
            let full = fresh_frame(device, queue, &cb);
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
        prepare_once(device, queue, &mut resources, &callback(full_quad(1), &empty, &read));
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
    /// A closed tab holding a grid is the reason the sweep exists — and there
    /// is no teardown to hang it on, a closed tab
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
        let (mut docked, preview) = (of_pane(0), of_pane(1));
        let mut resources = CallbackResources::default();
        prepare_once(&device, &queue, &mut resources, &docked);
        prepare_once(&device, &queue, &mut resources, &preview);
        let live = |resources: &CallbackResources| {
            let spectrogram: &SpectrogramResources =
                resources.get().expect("prepare inserts its resources");
            let mut ids: Vec<u64> = spectrogram.panes.keys().collect();
            ids.sort_unstable();
            ids
        };
        assert_eq!(live(&resources), vec![0, 1], "both spectrograms should hold buffers");
        let held = |resources: &CallbackResources, id: u64| {
            let spectrogram: &SpectrogramResources = resources.get().expect("resources");
            spectrogram
                .panes
                .get(id)
                .expect("a drawn pane holds a grid")
                .grid
                .as_ref()
                .expect("a drawn pane holds a grid")
                .buffer
                .size()
        };
        assert_eq!(held(&resources, 0), held(&resources, 1), "each pane sizes its own copy");

        for pass_nr in 1..=crate::pass_aged::TTL_PASSES {
            docked.pass_nr = pass_nr;
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

    /// A pane carrying plenty of energy with no CONCENTRATION anywhere in it:
    /// the same level in every bin of every slab.
    ///
    /// It used to be the fixture the cloud layer was measured over, because a
    /// pile of puffs draws its own texture against a flat picture as readily
    /// as against any other. It is the negative control now: everything that
    /// moves the LOOKUP has to leave this pane alone, because a displaced
    /// constant is that constant.
    fn flat_cloud_fixture() -> SpectrogramCallback {
        let mut cb = cloud_fixture();
        cb.grid.run = Arc::new(vec![150; cb.grid.run.len()]);
        cb
    }

    /// The layer BENDS the picture rather than painting over it.
    ///
    /// This is the whole of what Yan asked for twice and what rounds 2 through
    /// 6 removed: the light is read where each scale's face points, so the
    /// spectrogram is seen THROUGH the cloud, displaced. Round 6 painted
    /// palette colour over the picture instead, and that is what made it read
    /// as *"some wisps overlaying the spectrogram"*.
    ///
    /// The measurement is the defining property of a lens, and it needs both
    /// halves or it passes for the wrong reason. **A lens over a featureless
    /// field is invisible** — bending a flat picture samples the same value
    /// from somewhere else and returns it unchanged — while over a structured
    /// one it moves a great deal. A layer that merely brightened or tinted
    /// would move BOTH, and a layer that did nothing would move neither.
    ///
    /// Everything but the refraction is held still between the two frames:
    /// same field, same relief, same glint, same ambient. Only `scale_refract`
    /// moves, so what is measured is the displacement alone.
    ///
    /// Measured: 6.6% of the pane over the ridge fixture, and EXACTLY zero
    /// over the flat one. The 6.6 is not small for the wrong reason — this
    /// fixture is one narrow ridge in a mostly dark pane, and bending black
    /// gives black, so only the neighbourhood of the ridge can move at all.
    /// The zero is the half that carries the claim, and it is exact rather
    /// than merely small because a displaced constant IS that constant.
    #[test]
    fn the_layer_bends_the_picture_rather_than_painting_over_it() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let moved_by_refraction = |cb: &mut SpectrogramCallback| {
            {
                let s = &mut cb.atmosphere.as_mut().unwrap().settings;
                s.cloud_depth = 1.0;
                s.scale_refract = 0.0;
            }
            let straight = fresh_frame(&device, &queue, cb);
            cb.atmosphere.as_mut().unwrap().settings.scale_refract = 1.0;
            let bent = fresh_frame(&device, &queue, cb);
            let n = straight.len() / 4;
            let moved = straight
                .chunks_exact(4)
                .zip(bent.chunks_exact(4))
                .filter(|(a, b)| (0..3).any(|c| a[c].abs_diff(b[c]) > 4))
                .count();
            moved as f32 / n as f32
        };
        let over_structure = moved_by_refraction(&mut cloud_fixture());
        let over_flat = moved_by_refraction(&mut flat_cloud_fixture());
        assert!(
            over_structure > 0.02,
            "turning the refraction from nothing to full moved almost none of the pane over \
             a picture with structure in it, so the lookup is not being displaced at all: \
             {over_structure}"
        );
        assert!(
            over_flat < over_structure / 5.0,
            "the refraction moved a FEATURELESS picture nearly as much as a structured one, \
             so it is adding something of its own rather than bending what is behind it: \
             {over_flat} flat against {over_structure} over structure"
        );
    }

    /// The NEGATIVE half of `Refraction` QUANTIZES that bend, and adds nothing of
    /// its own. It was a dial of its own, `Facet`, and "the facet" below is what
    /// it draws at -1.
    ///
    /// Reading the light at the nearest scale's CENTRE is the other half of
    /// what round 1 had and round 5 removed, and round 5 was right about the
    /// defect: a nearest-cell pick STEPS across the bisector between two
    /// scales, which is a straight edge through a cloud. The soft union keeps
    /// the reading and loses the step. `Pile::to_centre` is the union's own
    /// weights against each dome's offset to its own centre, so inside a dome
    /// one weight runs away with the sum and the reading is that dome's centre
    /// — one value for the whole interior, which is the flat patch — while on a
    /// bisector the two weights are equal and the reading crosses over
    /// continuously.
    ///
    /// Measured exactly as the refraction above, and for the same reason: the
    /// dial moves 7.1% of the pane over the ridge fixture and EXACTLY zero over
    /// the flat one. The zero is the half that carries the claim. A displaced
    /// lookup over a constant field returns that constant wherever it reads, so
    /// a facet that moved a featureless picture would be PAINTING its scales
    /// on rather than quantizing what is behind them — and paint is the easy
    /// thing to mistake for this effect, because a mosaic drawn over a flat
    /// field looks like a mosaic too.
    #[test]
    fn negative_refraction_quantizes_the_bend_rather_than_painting_scales() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let moved_by_facet = |cb: &mut SpectrogramCallback| {
            // From the fresh bend through the faces to the read at the centres.
            cb.atmosphere.as_mut().unwrap().settings.cloud_depth = 1.0;
            let bent = fresh_frame(&device, &queue, cb);
            cb.atmosphere.as_mut().unwrap().settings.scale_refract = -1.0;
            let faceted = fresh_frame(&device, &queue, cb);
            let n = bent.len() / 4;
            let moved = bent
                .chunks_exact(4)
                .zip(faceted.chunks_exact(4))
                .filter(|(a, b)| (0..3).any(|c| a[c].abs_diff(b[c]) > 4))
                .count();
            moved as f32 / n as f32
        };
        let over_structure = moved_by_facet(&mut cloud_fixture());
        let over_flat = moved_by_facet(&mut flat_cloud_fixture());
        assert!(
            over_structure > 0.02,
            "carrying the lookup from the scales' faces to their centres moved almost none of \
             the pane over a picture with structure in it, so the facet is not reaching the \
             lookup at all: {over_structure}"
        );
        assert_eq!(
            over_flat, 0.0,
            "the facet moved a FEATURELESS picture, so it is drawing its scales rather than \
             quantizing what is behind them: {over_flat} flat against {over_structure} over \
             structure"
        );
    }

    /// `Variety`, which does not move the LOOKUP, reaches the shader.
    ///
    /// It does not change where the light is read, so it does not show up in the
    /// measurement above — and it rides in the same uniform, which is read by
    /// OFFSET rather than by name. A field added in the wrong place there swaps
    /// two values silently and nothing in either type system notices, so what
    /// this holds is that this one separately moves the picture it is supposed
    /// to move.
    ///
    /// At the fixture's fresh relief the scales are barely domed and at its
    /// fresh refraction the lookup hardly moves, which is a fixture too small
    /// to reach the knob; both are turned up here. Measured at 6.7% of the
    /// pane. That is smaller than it was before the glint went: the glint was
    /// an ADDITIVE term that carried a lot of whatever moved the normal, and
    /// with it gone everything the dial does has to arrive through `diffuse`
    /// and the lookup alone.
    ///
    /// What it changes is each dome's radius AND how loudly each argues for its
    /// own ground, which moves every face and so the whole shading. The
    /// property that makes it safe — that its smallest radius still covers the
    /// plane, and that a weight is a share of a mean rather than a licence to
    /// leave — is geometry rather than pixels and is held by
    /// [`the_dome_grid_covers_the_plane_and_the_ring_holds_it`].
    ///
    /// **The floor is 5% on purpose.** It used to move 2.6% here, and that was
    /// the complaint: a dial that passed this test and still read as doing
    /// nothing, because the radius band it opened was pinned by the coverage
    /// proof to about a sixth either way while the cell grid that sets the
    /// apparent size never moved at all. A floor set just under the old reading
    /// is a floor that cannot tell the two apart, so it is set above it instead
    /// — this fails if the dial ever goes back to being a radius band alone.
    #[test]
    fn the_variety_reaches_the_scales() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // The fixture's own relief is too small to reach it: a scale barely
        // domed has hardly any face for the shading to find.
        let lit = |variety| {
            let mut cb = cloud_fixture();
            let s = &mut cb.atmosphere.as_mut().unwrap().settings;
            s.cloud_depth = 1.0;
            s.scale_relief = 1.0;
            // Refraction up as well: `Variety` redraws each dome's RADIUS,
            // which reaches the picture through the lookup as much as through
            // the shading, and at the fresh 30% the lookup barely moves.
            s.scale_refract = 1.0;
            s.scale_variety = variety;
            fresh_frame(&device, &queue, &cb)
        };
        let plain = lit(0.0);
        let frame = lit(1.0);
        let moved = plain
            .chunks_exact(4)
            .zip(frame.chunks_exact(4))
            .filter(|(a, b)| (0..3).any(|c| a[c].abs_diff(b[c]) > 4))
            .count() as f32
            / (plain.len() / 4) as f32;
        assert!(moved > 0.05, "Variety moved almost none of the pane: {moved}");
    }

    /// The layer saturates no channel the picture had not saturated already.
    ///
    /// A clipped channel is a flat patch with a hard edge in a shifted hue —
    /// the metallic patches of round (4), which needed a tone map to hold back
    /// because the shading was a PRODUCT of transmission, relief and sheen and
    /// ran past what the palette could hold. There is no such product now:
    /// every wash is a `mix` between the picture and a palette colour, both
    /// already inside the ramp, so this cannot clip by construction. The test
    /// stays as the guard on that construction — an additive term
    /// reintroduced anywhere in the paint would trip it.
    ///
    /// Run at the deepest layer and the most pigment the dials reach, which is
    /// where the old one went over.
    #[test]
    fn the_layer_saturates_no_channel_the_picture_had_not() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let mut cb = cloud_fixture();
        cb.atmosphere.as_mut().unwrap().settings.cloud_depth = 0.0;
        let bare = fresh_frame(&device, &queue, &cb);
        let settings = &mut cb.atmosphere.as_mut().unwrap().settings;
        settings.cloud_depth = 1.0;
        settings.scale_relief = 1.0;
        // Undiluted: the most pigment there is, and the least water to pale it.
        settings.scale_refract = 1.0;
        let clouded = fresh_frame(&device, &queue, &cb);
        let clipped = |frame: &[u8]| {
            frame.chunks_exact(4).filter(|p| p[..3].iter().any(|&c| c >= 254)).count()
        };
        assert_eq!(clipped(&bare), 0, "the fixture clips on its own and measures nothing");
        assert_eq!(clipped(&clouded), 0, "the cloud layer clipped a channel flat");
    }

    /// The pane [`rough_band_fixture`] draws over: [`SIZE`] made TALLER, and
    /// deliberately not wider.
    ///
    /// The cloud's frame is ten cloud units across the pane's HEIGHT, so the
    /// height alone decides how coarse the top of `Scale size` can be — on a
    /// 128-point pane the top of the dial is a ten-point scale, and this
    /// fixture needs one near twenty for the reason its own comment gives.
    /// Doubling the height restores exactly the scale the retired 4x drew.
    ///
    /// The width is left alone because it is load-bearing in the other
    /// direction. This fixture's whole subject is a crest that wanders between
    /// NEIGHBOURING COLUMNS, and the wander is twelve slabs laid across the
    /// pane's width: widening the pane spreads the same twelve over twice the
    /// columns and halves the very roughness the test exists to put a sun on.
    const ROUGH_BAND_SIZE: [u32; 2] = [SIZE[0], SIZE[1] * 2];

    /// A loud band with a sharp pitch edge against silence, its level rough
    /// from column to column the way a real spectrogram's is.
    ///
    /// Both halves are load-bearing and neither is decoration. The BAND is what
    /// puts a crest in the light field, and a crest is where the picture's
    /// gradient reverses. The per-column ROUGHNESS is what makes the crest
    /// wander between neighbouring columns instead of sitting on one row, which
    /// is what turned a seam into the row of vertical tears Yan photographed.
    /// `cloud_fixture`'s own band is four slabs of a flat 255 and cannot show
    /// it: a crest that does not move has nothing to tear along.
    fn rough_band_fixture() -> SpectrogramCallback {
        let mut cb = cloud_fixture();
        cb.rect = egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(ROUGH_BAND_SIZE[0] as f32, ROUGH_BAND_SIZE[1] as f32),
        );
        cb.vertices = full_quad_in(12, ROUGH_BAND_SIZE);
        cb.read.rows = ROUGH_BAND_SIZE[1];
        cb.atmosphere.as_mut().unwrap().region = cb.rect;
        let mut bytes = vec![0u8; 12 * BINS as usize];
        let mut seed = 0x9e37_79b9u32;
        for slab in 0..12 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            // The band's upper edge wanders by up to three rows between one
            // column and the next, which is what makes its crest a ragged line
            // rather than a straight one.
            let top = 560 - ((seed >> 26) as usize);
            for b in 240..top {
                bytes[slab * BINS as usize + b] = 255;
            }
        }
        cb.grid = grid_of(Arc::new(bytes), BINS, 12, 0);
        let s = &mut cb.atmosphere.as_mut().unwrap().settings;
        s.cloud_depth = 1.0;
        // The dials Yan had up when he found it: the shading has to be deep
        // enough that a sun on the wrong side of a face is visible.
        s.scale_relief = 1.0;
        // `Facet` at 100% then, which read at the centres whatever
        // `Refraction` said: the bottom of the one dial the two became.
        s.scale_refract = -1.0;
        // Scales about 19 points across, which is what the measurement needs:
        // a texture whose own detail is a few pixels wide has column steps of
        // its own, and column steps are exactly what this test counts. At the
        // fresh size they would be 9 points here and those steps would drown
        // the thing being measured.
        //
        // The number was 4x while the dial ran to 4x. The ceiling came down to
        // 2x — nothing above it was ever usable on a real pane — so the 19
        // points now come from `ROUGH_BAND_SIZE` doubling the pane instead,
        // which is the half of the product this fixture always actually
        // wanted. Read from the constant rather than copied, so the next move
        // of the ceiling takes this with it.
        s.scale_size = harmonigraph_scene::CLOUD_SIZE_MAX;
        // The shade floor LIFTS a turned-away face, so it hides exactly what
        // this is measuring — and the relief of 1 above already drives it to 0,
        // which is the raw Lambert the defect lived in. It is the same line it
        // always was, now spelled by the dial that absorbed it.
        cb
    }

    /// The sun leans with the picture but never JUMPS across it.
    ///
    /// Yan, on the build before this one: *"There are some areas with high
    /// contrast which looks really rough. Can't seem to get rid of it by
    /// adjusting the settings."* — a row of near-black vertical tears along the
    /// top of a loud band, and he was right that no dial reached it, because
    /// the flip was in the SUN rather than in the scales.
    ///
    /// The sun's direction used to be `normalize(grad / magnitude)`, a unit
    /// vector aimed along the picture's gradient. A gradient reverses across
    /// every crest, so the sun crossed to the opposite side of the sky along
    /// the top of every band and every face that had been lit turned away in
    /// one pixel step. Scaling the gradient instead of normalising it takes the
    /// lean smoothly through zero at a crest — overhead there, and back down
    /// the other side — so there is no step left to draw.
    ///
    /// Measured as the count of adjacent-COLUMN luminance steps past 24/255,
    /// which is the shape a row of vertical tears makes. The bare picture under
    /// this fixture has NONE, so any the layer shows are its own; on the old
    /// sun it shows 5 and on this one 0. The check is on columns rather than on
    /// rows because the tears run down the band, and a row-wise measure would
    /// find the band's own sharp edges instead.
    ///
    /// Non-vacuous, and measured both ways rather than reasoned about: putting
    /// the two old lines back into the shader fails this at 5 against a floor
    /// of 0, on the 128-point pane the fixture used to draw over. The 5 was
    /// small because that pane had about six columns of band on it; the same
    /// defect over a 1280-point render of the same content is 2666 such steps,
    /// and that is the picture Yan saw. The pane is `ROUGH_BAND_SIZE` now, so
    /// the figure a reverted shader would show here is larger still.
    #[test]
    fn the_sun_leans_across_a_loud_band_without_jumping_sides() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let lum =
            |p: &[u8]| 0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2]);
        let torn = |frame: &[u8]| {
            let mut count = 0u32;
            for y in 0..ROUGH_BAND_SIZE[1] as usize {
                for x in 1..ROUGH_BAND_SIZE[0] as usize {
                    let a = (y * ROUGH_BAND_SIZE[0] as usize + x) * 4;
                    count +=
                        u32::from((lum(&frame[a..a + 4]) - lum(&frame[a - 4..a])).abs() > 24.0);
                }
            }
            count
        };
        let mut bare = rough_band_fixture();
        bare.atmosphere.as_mut().unwrap().settings.cloud_depth = 0.0;
        let floor = torn(&fresh_frame(&device, &queue, &bare));
        let layered = torn(&fresh_frame(&device, &queue, &rough_band_fixture()));
        assert!(
            layered <= floor,
            "the layer tore {layered} column steps past 24/255 over a loud band where the bare \
             picture has {floor}, so it is adding roughness of its own — which is the sun \
             crossing sides along the band's crest"
        );
    }

    /// Every point of the plane is inside some dome, and every dome that
    /// reaches a point is inside the ring the union walks.
    ///
    /// This is the one property the whole "texture everywhere" change rests on,
    /// and it is NOT observable in a frame — an uncovered point draws a face of
    /// zero, which is also what the top of a dome draws. What it would cost is a
    /// discontinuity rather than a hole: `to_centre` falls from most of a radius
    /// to nothing at the rim of the last dome, so a pinhole is a hard edge in a
    /// picture whose entire construction is about not having one. So the claim
    /// is checked where it lives, in the geometry, against the constants the
    /// SHIPPED shader spells rather than a transcription of them.
    ///
    /// **Coverage.** A centre sits at its cell's middle give or take
    /// `JITTER / 2`. The point hardest to reach is a lattice corner with all
    /// four cells touching it pushed diagonally away,
    /// `(0.5 + JITTER / 2) * sqrt(2)` from every one of them, so the smallest
    /// radius any dome can draw has to clear that. At variety 0 that radius is
    /// `DOME_RADIUS`; at variety 1 it is `DOME_RADIUS_MIN`, and every setting
    /// between is a `mix` of the two and so never below the smaller.
    ///
    /// **Reach.** A cell two out can put its centre no nearer than
    /// `2.5 - JITTER / 2` from the pixel's own cell origin, and the pixel is at
    /// most 1 past that origin, so the largest radius has to stay under
    /// `1.5 - JITTER / 2` or a dome the 3x3 ring never visits can touch the
    /// pixel — which is a step on the cell grid every time `floor(r)` moves.
    ///
    /// Round 5's jitter of 0.75 failed BOTH (it needed a radius at once above
    /// 1.237 and below 1.125), and both failures were live in the picture.
    ///
    /// `DOME_VARIETY_GAIN` is deliberately absent from both. It scales a dome's
    /// WEIGHT, and the union is a weighted mean over whichever domes already
    /// cover the pixel: a gain changes whose face is read and never whether a
    /// face is there to read, so it moves neither radius and appears in neither
    /// inequality. What bounds it instead is smoothness, which is measured
    /// where the constant is declared.
    #[test]
    fn the_dome_grid_covers_the_plane_and_the_ring_holds_it() {
        let number = |name: &str| -> f32 {
            crate::shadow::tests::shader_const(SPECTROGRAM_SRC, name).parse().expect("a number")
        };
        let jitter = number("DOME_JITTER");
        let smallest = number("DOME_RADIUS").min(number("DOME_RADIUS_MIN"));
        let largest = number("DOME_RADIUS").max(number("DOME_RADIUS_MAX"));
        let farthest = (0.5 + jitter / 2.0) * std::f32::consts::SQRT_2;
        assert!(
            smallest > farthest,
            "a dome of {smallest} cannot reach a corner {farthest} away, so at some corner of \
             the cell grid no dome covers the pane and `to_centre` steps to nothing there"
        );
        let unvisited = 1.5 - jitter / 2.0;
        assert!(
            largest < unvisited,
            "a dome of {largest} reaches {unvisited} into a pixel the 3x3 ring never visits \
             it from, so the union gains and loses it as `floor(r)` crosses a cell"
        );
    }

    /// `Relief` carries the retired `Shade floor` through the pair Yan had set.
    ///
    /// The two dials were one product — both of them only decide how far
    /// `diffuse` dips below 1 — so the floor became a function of the relief.
    /// The exponent is the whole of that merge, and it is not a taste: it is
    /// fixed by the requirement that the merged dial pass through the defaults
    /// the pair shipped with, a relief of 0.35 against a floor of 0.25.
    ///
    /// Held here rather than in a rendered frame because a frame cannot see it.
    /// The floor only reaches the picture where a face is turned far enough off
    /// the sun for the Lambert term to approach zero, which is a small and
    /// fixture-dependent corner of any pane; a pixel test that turned `Relief`
    /// would be measuring the TILT, which moves the same picture much harder
    /// and would pass just as well with the exponent wrong. The arithmetic is
    /// the claim, so the arithmetic is what is checked — against the shipped
    /// shader's text and the shipped default, not a transcription of either.
    #[test]
    fn the_relief_dial_carries_the_retired_shade_floor() {
        let fall: f32 = crate::shadow::tests::shader_const(SPECTROGRAM_SRC, "RELIEF_FLOOR_FALL")
            .parse()
            .expect("a number");
        let fresh = harmonigraph_scene::SpectralAtmosphere::default();
        let floor = (1.0 - fresh.scale_relief).powf(fall);
        assert!(
            (floor - 0.25).abs() < 0.005,
            "the fresh relief of {} floors at {floor}, not the 0.25 the retired dial shipped, \
             so this build restyles a look Yan had already settled",
            fresh.scale_relief
        );
        // The two ends the dial promises, which the exponent only holds while
        // it is positive: nothing to floor where there is no tilt, and nothing
        // held back at the top.
        assert!(fall > 0.0, "a floor that does not fall as the relief rises is not a merge");
        assert_eq!(1.0_f32.powf(fall), 1.0);
        assert_eq!(0.0_f32.powf(fall), 0.0);
    }

    /// The same two inequalities for the WASH's glob grid, which pushes on them
    /// differently in three places.
    ///
    /// `Wander` is ABSENT from both, and that is the design rather than an
    /// oversight: it turns each glob's jitter offset about its own cell instead
    /// of adding a travel to it, so the offset's length never changes and the
    /// clock cannot carry a glob anywhere the ring does not already reach. What
    /// it costs instead is the `sqrt(2)` on the reach side — a rotated corner of
    /// the jitter box can point along either axis, where an unturned one is at
    /// most `JITTER / 2` along it.
    ///
    /// A retired `Ragged` dial pushed a RIM outward and never inward, so reach
    /// paid `* (1 + WASH_RAGGED)` where coverage read `WASH_RADIUS_MIN`
    /// untouched. One-sided also inflated every rim by about 15% at the setting
    /// that shipped, so the band was scaled by 1.15 as the wobble went and the
    /// globs are the size they always drew. Reach now carries `WASH_RADIUS_MAX`
    /// alone, which leaves room for a band wider than 1.63:1 — a look change,
    /// and Yan's, so the numbers here do not take it.
    ///
    /// And the ring is `WASH_RING` cells rather than a hard-coded 1, because at
    /// 3x3 these inequalities leave a radius band of about 1.2:1 at a jitter of
    /// 0.20 — a nearly regular grid of nearly equal globs, which is the one thing
    /// a field of DIFFERENT SIZED globs cannot be. At 5x5 they leave 1.63:1 at a
    /// jitter of 0.40, with 15.4% of a radius spare on coverage and 16.1% on
    /// reach.
    ///
    /// Both bounds are about globs that COVER the pixel. The tide line reads a
    /// glob it is OUTSIDE, over a window that runs past the rim and so past this
    /// ring; what holds that is the top-two-nearest rule in `wash_scan` rather
    /// than the ring, and the comment there says so.
    ///
    /// Read off the shipped shader text, not a transcription: an uncovered point
    /// is not visible as a hole, it is a pixel whose lookup falls from most of a
    /// radius to nothing, which is a hard edge in a construction whose whole
    /// point is not having one.
    #[test]
    fn the_wash_grid_covers_the_plane_and_the_ring_holds_it() {
        let number = |name: &str| -> f32 {
            crate::shadow::tests::shader_const(SPECTROGRAM_SRC, name).parse().expect("a number")
        };
        let slack = number("WASH_JITTER") / 2.0 * std::f32::consts::SQRT_2;
        let smallest = number("WASH_RADIUS_MIN");
        let largest = number("WASH_RADIUS_MAX");
        let farthest = 0.5 * std::f32::consts::SQRT_2 + slack;
        assert!(
            smallest > farthest,
            "a glob of {smallest} cannot reach a corner {farthest} away, so at some corner of \
             the cell grid no glob of the base octave covers the pane and the wash reads its \
             own light with no centre to borrow"
        );
        let unvisited = number("WASH_RING") + 0.5 - slack;
        assert!(
            largest < unvisited,
            "a rim of {largest} reaches {unvisited} into a pixel the ring never visits it from, \
             so the wash gains and loses that glob as `floor(r)` crosses a cell"
        );
    }

    /// Every period the `Cloud tile` bar offers makes a whole number of cells
    /// out of EVERY lattice the two walks hash on.
    ///
    /// A tile is one period of the walk read through a REPEATING sampler, so the
    /// walk has to be periodic or its far edge hashes cells that do not meet its
    /// near edge — a straight seam down the pane every period, which is the same
    /// failure the two ring proofs above exist to keep off the cell grid.
    ///
    /// Neither walk runs on one lattice. Each has a second octave at its own
    /// lacunarity, and the wash reads a shared warp noise whose cells are
    /// `WASH_WARP_SCALE` across, with a second octave of its own.
    /// `WASH_FBM_FINE`'s 2.07 is the one no period can make whole, which is why
    /// the tiled path runs that octave at `WASH_FBM_FINE_TILED` — so that
    /// constant is read here too, and moving it off a whole number fails this.
    ///
    /// Read off the shipped shader text rather than a transcription of it, for
    /// the reason the two proofs above give.
    #[test]
    fn the_tile_period_tiles_every_lattice() {
        let number = |name: &str| -> f32 {
            crate::shadow::tests::shader_const(SPECTROGRAM_SRC, name).parse().expect("a number")
        };
        let fine = number("WASH_FBM_FINE_TILED");
        let warp = number("WASH_WARP_SCALE");
        let lattices = [
            ("the mosaic's fine octave", number("DOME_LACUNARITY")),
            ("the wash's fine octave", number("WASH_LACUNARITY")),
            ("the warp noise", warp),
            ("the warp noise's own second octave", warp * fine),
        ];
        let step = harmonigraph_scene::CLOUD_TILE_STEP;
        let steps = (harmonigraph_scene::CLOUD_TILE_MAX / step) as u32;
        assert!(steps > 0, "the bar offers no period at all, so the dial cannot be turned on");
        for offered in 1..=steps {
            let period = offered as f32 * step;
            for (name, lattice) in lattices {
                let cells = lattice * period;
                assert!(
                    (cells - cells.round()).abs() < 1.0e-3,
                    "a period of {period} leaves {name} {cells} cells across, which is not a \
                     whole number of them, so the walk does not close on itself and the tile \
                     draws a seam every period"
                );
            }
        }
    }

    /// The fixture the wash is measured over: the ridge pane above, with the
    /// watercolour texture selected and its globs made big enough to BE a
    /// texture on a 128-point pane.
    ///
    /// The size is the part that has to be said. At the fresh cloud size this
    /// 128-point pane carries fifty cells, so a glob is under six points across
    /// — and a texture whose own detail is a handful of pixels wide measures its
    /// own aliasing rather than the dial being turned, the same trap
    /// `rough_band_fixture` names for the scales. The top of the dial takes the
    /// pane down to twenty-six cells and the glob up to about twelve points,
    /// which is where every figure below was measured.
    fn wash_fixture() -> SpectrogramCallback {
        let mut cb = cloud_fixture();
        // Off zero, where the retired `Wander` needed it: every figure measured
        // over this fixture was taken with the drift three seconds along.
        cb.atmosphere.as_mut().unwrap().now = 3.0;
        let s = &mut cb.atmosphere.as_mut().unwrap().settings;
        s.cloud_style = harmonigraph_scene::CloudStyle::Watercolor;
        s.cloud_depth = 1.0;
        s.wash_size = harmonigraph_scene::CLOUD_SIZE_MAX;
        cb
    }

    /// The wash BENDS the picture rather than painting over it — the same claim
    /// [`the_layer_bends_the_picture_rather_than_painting_over_it`] makes for the
    /// scales, and it needs both halves here for the same reason.
    ///
    /// Refraction is the only term in the wash that carries the sound: every
    /// other one is pigment, which is a function of the glob field alone. So
    /// over a picture with structure in it the dial has to move a lot of the
    /// pane, and over a FEATURELESS one it has to move EXACTLY nothing, because
    /// a lookup displaced across a constant field returns that constant wherever
    /// it lands. A wash that drew its globs from the paint rather than from the
    /// light would move both.
    ///
    /// Exactly zero and not merely small: the pigment cues (tide line, rim) are
    /// still drawn over the flat fixture and are still there in both frames, so
    /// anything this measures is the lookup alone.
    #[test]
    fn the_wash_reads_the_light_at_each_globs_centre_and_invents_none() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let moved_by_refraction = |cb: &mut SpectrogramCallback| {
            cb.atmosphere.as_mut().unwrap().settings.wash_refract = 0.0;
            let straight = fresh_frame(&device, &queue, cb);
            cb.atmosphere.as_mut().unwrap().settings.wash_refract = 1.0;
            let bent = fresh_frame(&device, &queue, cb);
            let n = straight.len() / 4;
            straight
                .chunks_exact(4)
                .zip(bent.chunks_exact(4))
                .filter(|(a, b)| (0..3).any(|c| a[c].abs_diff(b[c]) > 4))
                .count() as f32
                / n as f32
        };
        let over_structure = moved_by_refraction(&mut wash_fixture());
        let mut flat = wash_fixture();
        flat.grid.run = Arc::new(vec![150; flat.grid.run.len()]);
        let over_flat = moved_by_refraction(&mut flat);
        assert!(
            over_structure > 0.02,
            "carrying every glob's reading from under the pixel to its own centre moved \
             almost none of the pane over a picture with structure in it, so the wash is not \
             refracting at all: {over_structure}"
        );
        assert_eq!(
            over_flat, 0.0,
            "the refraction moved a FEATURELESS picture, so the wash is drawing its globs out \
             of the paint rather than reading them out of the light: {over_flat} flat against \
             {over_structure} over structure"
        );
    }

    /// The bottom of the picture is the GRADIENT'S bottom, whatever colour that
    /// is — the claim `palette_color` makes at level 0.
    ///
    /// Every other fixture in this file rides a palette that already starts at
    /// black, which is the fixture-too-small trap exactly: under the old rule,
    /// where the first half-slice faded to true black, all of them pass
    /// unchanged and none of them is looking at the thing. So this one authors
    /// a ramp with NO black anywhere in it, and then a black pixel can only
    /// have come from the renderer.
    ///
    /// The wash is where it is reachable at all, because its hold is the one
    /// thing that drives a level to exactly 0 — the lifted paper alone sits
    /// well above the slice and would pass either way.
    ///
    /// Both halves, because either alone passes for a bug. A renderer that had
    /// simply stopped drawing black would satisfy the first; one still ignoring
    /// the palette would satisfy the second. Together they are the claim: the
    /// floor of the picture FOLLOWS the floor of the scheme, down to and
    /// including black when that is what the scheme says.
    #[test]
    fn a_quiet_pane_takes_the_gradients_own_floor_rather_than_black() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let floor = [0u8, 153, 140];
        let mut cb = wash_fixture();
        cb.shades.lut = Arc::new(
            (0..256)
                .map(|v| {
                    let t = v as f32 / 255.0;
                    let up = |lo: u8| (f32::from(lo) + (255.0 - f32::from(lo)) * t).round() as u8;
                    [up(floor[0]), up(floor[1]), up(floor[2]), 255]
                })
                .collect(),
        );
        let share = |frame: &[u8], want: [u8; 3]| {
            let n = frame.len() / 4;
            frame.chunks_exact(4).filter(|px| px[..3] == want).count() as f32 / n as f32
        };
        let lifted = fresh_frame(&device, &queue, &cb);
        assert_eq!(
            share(&lifted, [0, 0, 0]),
            0.0,
            "a palette with no black in it still drew black over silence, so the picture has \
             a floor of its own under the scheme's"
        );
        assert!(
            share(&lifted, floor) > 0.5,
            "the quiet pane did not settle on the palette's own floor: {} of it",
            share(&lifted, floor)
        );

        // And the other direction, on the same fixture: a scheme whose floor IS
        // black still gets black, which is what keeps the fresh Aurora — the
        // whole `L*` axis, so `L*` 0 at the bottom — drawing the pane it always
        // drew.
        cb.shades.lut =
            Arc::new((0..256).map(|v| [0, (v as f32 * 0.7) as u8, v as u8, 255]).collect());
        let dark = fresh_frame(&device, &queue, &cb);
        assert!(
            share(&dark, [0, 0, 0]) > 0.5,
            "a palette that does start at black lost it: {} of the pane",
            share(&dark, [0, 0, 0])
        );
    }

    /// The wash returns silence to the palette's floor, and reaches no further
    /// up than the knee to do it.
    ///
    /// The lift under `paper` is an OFFSET — written out it is
    /// `1.15 * light + 0.0925` — so the quietest tone a glob could draw is
    /// palette level 0.09 whatever the picture holds, and `Cloud depth` would
    /// turn a quiet pane from the palette's floor to mid-tone as it came up.
    /// The scales beside it never do that: their light reaches 0. The hold is
    /// what puts the wash back on terms with them.
    ///
    /// Both halves, because either one alone passes for a bug. A hold that
    /// darkened the whole picture would satisfy the first; one soldered to
    /// nothing would satisfy the second.
    ///
    /// **The second half is weaker than it was, and deliberately.** It used to
    /// be byte-exact, by drawing the flat fixture twice with the retired
    /// `Black point` at each end and demanding the two match. That dial is gone
    /// — its
    /// whole travel parked silence at a colour the gradient never named, so its
    /// only correct position was the maximum it shipped at — and with it went
    /// the only lever that could turn the hold off. `WASH_BLACK_KNEE` is a
    /// const, so what is left is the light itself: the flat fixture sits at
    /// display intensity 0.59 against a knee of 0.175, and a hold that had
    /// become a tone control over the whole ramp would drag it down. That is
    /// what is measured, at the resolution of the palette rather than of a
    /// single byte.
    #[test]
    fn the_wash_returns_silence_to_the_palettes_floor() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let floor = |frame: &[u8]| {
            let n = frame.len() / 4;
            frame.chunks_exact(4).filter(|px| px[..3] == [0, 0, 0]).count() as f32 / n as f32
        };
        let held = fresh_frame(&device, &queue, &wash_fixture());
        let held_floor = floor(&held);
        assert!(
            held_floor > 0.5,
            "the wash left most of a silent pane off the palette's floor: {held_floor} of it",
        );

        // Nowhere near black, and it has to stay that way. The fixture's ramp
        // puts the palette index straight in the blue channel, so the darkest
        // blue in the pane IS the lowest level the wash drew.
        let mut flat = wash_fixture();
        flat.grid.run = Arc::new(vec![150; flat.grid.run.len()]);
        let open = fresh_frame(&device, &queue, &flat);
        assert_eq!(floor(&open), 0.0, "the hold reached a pane that is nowhere near black");
        let darkest = open.chunks_exact(4).map(|px| px[2]).min().unwrap();
        assert!(
            darkest > 96,
            "the darkest tone over a flat 0.59 pane fell to {darkest}, so the hold is a tone \
             control over the whole ramp rather than a floor under the dark end",
        );
    }

    /// Either texture is drawn over a SHARP picture: a cloud does not need a blur.
    ///
    /// It used to vanish with the softness, and at two gates. The draw path
    /// built the light field only while a softness was above zero, and both
    /// cloud shaders returned the base wherever the blur's step was zero, so
    /// with both softness dials at 0 `Cloud depth` moved nothing at all. The
    /// field is built whenever a cloud is drawn now, and at zero softness it is
    /// the measured picture copied through.
    ///
    /// Over a flat lit field, because the ridge fixture is under one percent
    /// light once nothing spreads it, and a cloud over silence is black for
    /// both textures — a fixture that would pass with the gates still shut.
    #[test]
    fn a_cloud_is_drawn_over_an_unblurred_picture() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        for (name, mut cb) in [("Mosaic", cloud_fixture()), ("Watercolor", wash_fixture())] {
            cb.grid.run = Arc::new(vec![150; cb.grid.run.len()]);
            let s = &mut cb.atmosphere.as_mut().unwrap().settings;
            s.pitch_softness = 0.0;
            s.time_softness = 0.0;
            s.cloud_depth = 1.0;
            let clouded = fresh_frame(&device, &queue, &cb);
            cb.atmosphere.as_mut().unwrap().settings.cloud_depth = 0.0;
            let bare = fresh_frame(&device, &queue, &cb);
            let moved = clouded
                .chunks_exact(4)
                .zip(bare.chunks_exact(4))
                .filter(|(a, b)| (0..3).any(|c| a[c].abs_diff(b[c]) > 4))
                .count() as f32
                / (bare.len() / 4) as f32;
            assert!(
                moved > 0.5,
                "{name} at full depth moved {moved} of a lit pane with both softness dials at \
                 0, so the cloud still needs a blur to be drawn",
            );
        }
    }

    /// Every wash dial separately reaches the shader.
    ///
    /// Seven `f32`s ride in one uniform read by OFFSET rather than by name,
    /// so a field added in the wrong place swaps two values silently and nothing
    /// in either type system notices. Folded into one test the way
    /// [`the_variety_reaches_the_scales`] is, because what each
    /// of them holds is the same thing about the same buffer.
    ///
    /// The base setting is not the fresh one. Several of these dials only have
    /// room to move away from a middle — `Fuzz` at its fresh 100% has already
    /// taken three quarters of the tide line away, so `Edge pooling` turned from
    /// there measures almost nothing — and a fixture too small to reach the
    /// branch is the failure this repo ships most often. Each dial is therefore
    /// carried from a middle to an end.
    #[test]
    fn the_wash_dials_each_reach_the_globs() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let painted = |turn: fn(&mut harmonigraph_scene::SpectralAtmosphere)| {
            let mut cb = wash_fixture();
            // Lift the whole fixture clear of the knee. Most of it is digital
            // silence, which the hold pins to the palette's floor, and a
            // pigment dial that only paints the dark half of the pane stops
            // being measurable there — `Edge pooling` reached 1.7% of it, under
            // this test's own bar. 80/255 is display intensity 0.31 against a
            // knee of 0.175, so the hold is exactly 1 over the whole pane and
            // what moves is the pigment, which is what each dial is here for.
            cb.grid.run = Arc::new(cb.grid.run.iter().map(|&v| v.max(80)).collect());
            let s = &mut cb.atmosphere.as_mut().unwrap().settings;
            s.wash_fuzz = 0.5;
            s.wash_lobe = 0.5;
            s.wash_pool = 0.5;
            s.wash_layers = 0.5;
            turn(s);
            fresh_frame(&device, &queue, &cb)
        };
        let plain = painted(|_| {});
        for (name, turn) in [
            (
                "Glob size",
                (|s: &mut harmonigraph_scene::SpectralAtmosphere| s.wash_size = 1.0)
                    as fn(&mut harmonigraph_scene::SpectralAtmosphere),
            ),
            ("Fuzz", |s| s.wash_fuzz = 0.0),
            ("Lobe shape", |s| s.wash_lobe = 0.0),
            ("Edge pooling", |s| s.wash_pool = 1.0),
            ("Layers", |s| s.wash_layers = 0.0),
        ] {
            let frame = painted(turn);
            let n = plain.len() / 4;
            let moved = plain
                .chunks_exact(4)
                .zip(frame.chunks_exact(4))
                .filter(|(a, b)| (0..3).any(|c| a[c].abs_diff(b[c]) > 4))
                .count() as f32
                / n as f32;
            assert!(moved > 0.02, "{name} moved almost none of the pane: {moved}");
        }
    }

    /// `Cloud pixel size` draws the SAME PICTURE, softened — which is the whole
    /// claim of the reduced path, and the only reason a performance dial is
    /// allowed to be one dial rather than a second look.
    ///
    /// Both halves are load-bearing. That the frame CHANGED is what says the
    /// reduced path ran at all, and it is checked twice over — once on the
    /// pixels and once on the pass count, because a frame can differ for any
    /// number of reasons while a third light pass appearing can only be this.
    /// That the change is SMALL is the claim itself: a reduced tone stretched
    /// back over the picture has to land within a rim's softening of the walk it
    /// replaces, and a reduction that had lost the drift, the pane offset or the
    /// style would differ by a great deal more. Measured at 2 pt against a
    /// 1 px/pt fixture — a quarter resolution — the mean absolute channel
    /// difference is 0.73/255 for the mosaic and 0.93/255 for the wash, no
    /// channel anywhere moves more than 9 and 18, and 1.7% and 6.3% of the pane
    /// moves past 4 at all.
    #[test]
    fn a_reduced_cloud_draws_the_same_picture_softened() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let passes = |resources: &CallbackResources| {
            resources
                .get::<SpectrogramResources>()
                .unwrap()
                .panes
                .get(0)
                .expect("the spectrogram prepared a pane")
                .cloud
                .as_ref()
                .expect("a clouded pane holds its targets")
                .encoded_passes
                .load(Ordering::Relaxed)
        };
        use harmonigraph_scene::CloudStyle;
        let mut means = Vec::new();
        for style in [CloudStyle::Mosaic, CloudStyle::Watercolor] {
            // The wash fixture's coarse glob, and the same coarseness asked of
            // the scales: at the fresh size this 128-point pane draws either
            // texture a handful of pixels wide, which measures its own aliasing
            // rather than what the tone target lost.
            let mut cb = wash_fixture();
            // The noisy grid rather than the fixture's own band, and that is the
            // fixture-reach half of this test rather than dressing. Neither
            // texture has much to say over a picture that is FLAT — the sun
            // stands overhead where the light has no gradient, and the wash's
            // paper is one tone — so over a fixture that is mostly silence both
            // resolutions draw nearly the same few levels and a reduction
            // reading its tone in the WRONG PLACE passes a mean bound
            // comfortably. Measured both ways: with the lookup deliberately
            // flipped end to end, the band fixture reads 0.93 and 2.0 against a
            // correct 0.26 and 0.90, which this bound would pass; over the noisy
            // grid it reads 29.9 and 37.9 against the 0.73 and 0.93 below.
            cb.grid = grid_of(noisy_grid(BINS as usize, 12), BINS, 12, 0);
            let s = &mut cb.atmosphere.as_mut().unwrap().settings;
            s.cloud_style = style;
            s.scale_size = harmonigraph_scene::CLOUD_SIZE_MAX;
            let mut native_resources = CallbackResources::default();
            let native = frame_with(&device, &queue, &mut native_resources, &cb);
            // 2 pt against the fixture's 1 pixel per point: a 64 by 64 tone
            // target under a 128 by 128 pane, which is a quarter of the walk.
            cb.atmosphere.as_mut().unwrap().settings.cloud_pixel = 2.0;
            let mut reduced_resources = CallbackResources::default();
            let reduced = frame_with(&device, &queue, &mut reduced_resources, &cb);
            assert_eq!(
                passes(&reduced_resources),
                passes(&native_resources) + 1,
                "{style:?} encoded no tone pass, so the reduced path never ran"
            );
            assert_ne!(native, reduced, "{style:?} drew the same frame at a quarter resolution");
            let channels = native.len() / 4 * 3;
            let mean = native
                .chunks_exact(4)
                .zip(reduced.chunks_exact(4))
                .flat_map(|(a, b)| (0..3).map(move |c| f64::from(a[c].abs_diff(b[c]))))
                .sum::<f64>()
                / channels as f64;
            let moved = native
                .chunks_exact(4)
                .zip(reduced.chunks_exact(4))
                .filter(|(a, b)| (0..3).any(|c| a[c].abs_diff(b[c]) > 4))
                .count() as f64
                / (native.len() / 4) as f64;
            means.push((style, mean, moved));
        }
        assert!(
            means.iter().all(|&(_, mean, _)| mean < 3.0),
            "a reduced cloud is not the picture the walk draws: {means:?}"
        );
    }

    /// The pane the tile is measured over: square, and twice [`SIZE`] on a side.
    ///
    /// The size is the fixture-reach half of
    /// [`a_tiled_cloud_draws_the_live_walk_inside_its_first_period`] and not
    /// dressing. The two pictures may only be compared where every cell the
    /// walk VISITS lies inside the first period, and the pane's cell
    /// coordinates run either side of zero — so at most the quarter of the pane
    /// whose cells are positive is ever comparable, and a small pane would
    /// leave that quarter a few hundred pixels.
    const TILE_SIZE: [u32; 2] = [SIZE[0] * 2, SIZE[1] * 2];

    /// A tiled cloud draws EXACTLY the live walk inside its first period.
    ///
    /// The tile wraps every cell a hash is taken at onto `[0, P)`, and inside
    /// that range a wrapped cell IS the unwrapped one — so where the whole ring
    /// a pixel visits lies in the first period, the tiled picture is the live
    /// walk resampled through one bilinear tap of a half-float texture, and
    /// nothing else. That is the claim that makes the tile a repeat of the real
    /// texture rather than a second look, and it is the one thing a wrong
    /// lookup — a scale, an offset, a `q` that forgot the drift, a missing
    /// second octave — cannot pass.
    ///
    /// Measured with the warp noise OFF (`Lobe shape` at 0) because it is the
    /// one place the tiled path deliberately differs: its second octave runs at
    /// `WASH_FBM_FINE_TILED` rather than 2.07, which is not the same field
    /// anywhere. Everything else is expected to agree.
    ///
    /// The window is worked out per pixel from the shader's own geometry, and
    /// the count of pixels in it is asserted — a mask that had drifted off the
    /// pane would otherwise compare nothing and pass. Measured over 9,540 pixels
    /// for the mosaic and 8,208 for the wash, out of a 65,536-pixel pane: a mean
    /// absolute channel difference of 0.02/255 and 0.36/255, NO mosaic pixel
    /// moving past 4 at all and 2.2% of the wash's, worst channel 1 and 21.
    ///
    /// The wash's worst is where its stored offset STEPS — the glob under the
    /// visible one changing, which the feather only smooths on the visible
    /// one's own rim — and one bilinear texel there is about three quarters of
    /// a pixel at this fixture's density. That is the resampling this dial is
    /// spending, and it is why the bound is a mean rather than a per-pixel one.
    #[test]
    fn a_tiled_cloud_draws_the_live_walk_inside_its_first_period() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // `SCALE_CELLS` is spelled as a quotient in the shader, so this reads
        // one as well as a plain number.
        let number = |name: &str| -> f32 {
            crate::shadow::tests::shader_const(SPECTROGRAM_SRC, name)
                .split('/')
                .map(|part| part.trim().parse::<f32>().expect("a number"))
                .reduce(|a, b| a / b)
                .expect("a constant has a value")
        };
        // The one number below that is not read out of the shader, because it
        // is not a constant there: both octaves are offset by this literal, in
        // `cloud_domes` and in `wash_field`. The window is only right while
        // that is what they say.
        let fine_offset = [17.3_f32, 5.9];
        assert!(
            SPECTROGRAM_SRC.matches("vec2<f32>(17.3, 5.9)").count() == 2,
            "the two fine octaves no longer sit at the offset this window assumes"
        );
        let units = number("CLOUD_UNITS");
        let period = 20.0_f32;
        use harmonigraph_scene::CloudStyle;
        let mut read = Vec::new();
        for (style, cells, ring) in [
            (CloudStyle::Mosaic, number("SCALE_CELLS") / 2.0, 1.0),
            (CloudStyle::Watercolor, number("WASH_CELLS") / 2.0, number("WASH_RING")),
        ] {
            let mut cb = cloud_fixture();
            cb.rect = egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(TILE_SIZE[0] as f32, TILE_SIZE[1] as f32),
            );
            cb.vertices = full_quad_in(12, TILE_SIZE);
            cb.read.rows = TILE_SIZE[1];
            cb.grid = grid_of(noisy_grid(BINS as usize, 12), BINS, 12, 0);
            cb.atmosphere.as_mut().unwrap().region = cb.rect;
            let s = &mut cb.atmosphere.as_mut().unwrap().settings;
            s.cloud_style = style;
            s.cloud_depth = 1.0;
            // The biggest cells the bars offer, so one period covers a real
            // fraction of the pane rather than repeating off the edge of it.
            s.scale_size = harmonigraph_scene::CLOUD_SIZE_MAX;
            s.wash_size = harmonigraph_scene::CLOUD_SIZE_MAX;
            s.wash_lobe = 0.0;
            // A still texture: `cloud_time` is zero, so the drift is the
            // constant `[0, 0.6]` its cosine starts at and both frames read the
            // field in the same place.
            s.cloud_speed = 0.0;
            let live = fresh_frame(&device, &queue, &cb);
            cb.atmosphere.as_mut().unwrap().settings.cloud_tile = period;
            let tiled = fresh_frame(&device, &queue, &cb);
            assert_ne!(live, tiled, "{style:?} drew the same frame tiled, so the tile never ran");

            // Which pixels the two are entitled to agree on: those whose whole
            // ring, in both octaves, lies inside the first period. The fine
            // octave counts in its own cells, `LACUNARITY` of them to one, so
            // its window is the tighter of the two and is what decides the
            // shape below.
            let lacunarity = number("DOME_LACUNARITY");
            let drift = [0.0_f32, 0.6];
            let inside = |axis: usize, pt: f32| {
                let half = TILE_SIZE[axis] as f32 / 2.0;
                let r = ((pt - half) / TILE_SIZE[1] as f32 * units + drift[axis]) * cells;
                let fine = lacunarity * r + fine_offset[axis];
                let held = |v: f32, last: f32| v >= ring + 1.0 && v <= last - ring - 1.0;
                held(r, period) && held(fine, (lacunarity * period).round())
            };
            let (mut compared, mut moved, mut worst, mut total) = (0u32, 0u32, 0u32, 0u64);
            for y in 0..TILE_SIZE[1] {
                for x in 0..TILE_SIZE[0] {
                    if !inside(0, x as f32 + 0.5) || !inside(1, y as f32 + 0.5) {
                        continue;
                    }
                    compared += 1;
                    let at = (y * TILE_SIZE[0] + x) as usize * 4;
                    let diff = |c: usize| u32::from(live[at + c].abs_diff(tiled[at + c]));
                    moved += u32::from((0..3).any(|c| diff(c) > 4));
                    for c in 0..3 {
                        worst = worst.max(diff(c));
                        total += u64::from(diff(c));
                    }
                }
            }
            let pane = TILE_SIZE[0] * TILE_SIZE[1];
            assert!(
                compared * 20 > pane,
                "{style:?} compared {compared} of {pane} pixels, which is too little of the \
                 pane to say the tile draws the walk"
            );
            let mean = total as f64 / f64::from(compared * 3);
            read.push((style, compared, mean, f64::from(moved) / f64::from(compared), worst));
            assert!(
                mean < 0.5 && worst < 32,
                "{style:?} tiled is not the walk inside its own first period: {read:?}"
            );
        }
    }

    /// The tile is rebaked when the WALK moves and never when the picture does.
    ///
    /// This is the cache-key half of the dial, and a key is wrong in two
    /// directions: a dial the walk reads and the key does not serves a stale
    /// texture, while one the key carries and the walk does not pays a whole
    /// cell walk — tens of milliseconds — on every frame of a drag of it. So
    /// both lists are here, and the clock is at the head of the second: the
    /// drift slides a fixed field rather than changing one, which is the whole
    /// reason this cache can exist.
    ///
    /// The key comparison is what decides, so it is what is checked; the pass
    /// count at the end is the seam that says `prepare` actually asks it,
    /// rather than rebaking around it on a resize of something else.
    #[test]
    fn the_tile_is_rebaked_only_when_the_walk_moves() {
        let key = |turn: fn(&mut harmonigraph_scene::SpectralAtmosphere), now| {
            let mut settings = harmonigraph_scene::SpectralAtmosphere {
                cloud_tile: 20.0,
                cloud_style: harmonigraph_scene::CloudStyle::Watercolor,
                ..Default::default()
            };
            turn(&mut settings);
            atmosphere::tile_key(
                [1920, 1080],
                SpectrogramAtmosphere {
                    settings,
                    region: egui::Rect::ZERO,
                    pitch_vertical: true,
                    points_per_cent: 0.03,
                    points_per_ms: 0.01,
                    now,
                },
            )
        };
        let fresh = key(|_| {}, 0.0);
        assert!(fresh.is_some(), "the fixture turned the tile on and got no tile");
        for now in [0.5, 7.0, 600.0] {
            assert_eq!(key(|_| {}, now), fresh, "a clock of {now} rebaked a field that only slid");
        }
        type Turn = fn(&mut harmonigraph_scene::SpectralAtmosphere);
        for (name, turn) in [
            ("Cloud speed", (|s| s.cloud_speed = 20.0) as Turn),
            ("Cloud depth", |s| s.cloud_depth = 0.5),
            ("Cloud pixel size", |s| s.cloud_pixel = 4.0),
            ("Refraction", |s| s.wash_refract = 0.0),
            ("Layers", |s| s.wash_layers = 0.0),
            ("Pitch softness", |s| s.pitch_softness = 300.0),
            ("Spread", |s| s.spread = 1.0),
            ("Contour strength", |s| s.contour_strength = 0.0),
            // The mosaic's own dial, which the wash's walk cannot read.
            ("Variety", |s| s.scale_variety = 1.0),
        ] {
            assert_eq!(key(turn, 0.0), fresh, "{name} rebaked a tile it cannot reach");
        }
        for (name, turn) in [
            ("Lobe shape", (|s| s.wash_lobe = 0.0) as Turn),
            ("Fuzz", |s| s.wash_fuzz = 0.0),
            ("Edge pooling", |s| s.wash_pool = 1.0),
            ("Cloud tile", |s| s.cloud_tile = 40.0),
            ("Texture", |s| s.cloud_style = harmonigraph_scene::CloudStyle::Mosaic),
            // Through the tile's texel size alone — how many cells cross the
            // pane, not what a cell draws.
            ("Glob size", |s| s.wash_size = harmonigraph_scene::CLOUD_SIZE_MAX),
        ] {
            assert_ne!(key(turn, 0.0), fresh, "{name} reaches the walk and did not rebake");
        }
        assert_eq!(key(|s| s.cloud_tile = 0.0, 0.0), None, "the dial at 0 still allocated a tile");

        let Some((device, queue)) = headless_device() else {
            return;
        };
        let passes = |resources: &CallbackResources| {
            resources
                .get::<SpectrogramResources>()
                .unwrap()
                .panes
                .get(0)
                .expect("the spectrogram prepared a pane")
                .cloud
                .as_ref()
                .expect("a clouded pane holds its targets")
                .encoded_passes
                .load(Ordering::Relaxed)
        };
        let mut cb = wash_fixture();
        cb.atmosphere.as_mut().unwrap().settings.cloud_tile = 20.0;
        let mut resources = CallbackResources::default();
        frame_with(&device, &queue, &mut resources, &cb);
        let first = passes(&resources);
        cb.atmosphere.as_mut().unwrap().now = 9.0;
        frame_with(&device, &queue, &mut resources, &cb);
        let steady = passes(&resources) - first;
        assert_eq!(first, steady + 1, "the first frame encoded no bake, so nothing is cached");
        cb.atmosphere.as_mut().unwrap().settings.wash_pool = 0.9;
        frame_with(&device, &queue, &mut resources, &cb);
        assert_eq!(
            passes(&resources) - first - steady,
            steady + 1,
            "a dial the walk reads left the tile standing"
        );
    }

    mod timing;
}

#[cfg(all(test, target_os = "macos", feature = "shader-assets-tools"))]
pub(super) fn asset_catalog(device: &wgpu::Device, format: wgpu::TextureFormat) {
    let resources = SpectrogramResources::new(device, format);
    // Runtime creation stays lazy, but the strict Metal catalog must cover
    // every production route before the first enabled atmospheric frame.
    drop(atmosphere::Pipelines::new(device, format, &resources.layout));
}
