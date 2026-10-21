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

use crate::uniforms::{uniform_group, Float2};

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::pass_aged::PassAged;
use crate::{create_vertex_buffer, wgpu, EGUI_BLEND};

const SPECTROGRAM_SRC: &str = concat!(
    include_str!("shaders/atmosphere_geometry.wgsl"),
    "\n",
    include_str!("shaders/stars.wgsl"),
    "\n",
    include_str!("shaders/spectrogram.wgsl"),
);

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
    "fs_velvet_tone",
    "fs_star_bake",
    "fs_stars",
    "fs_color_memory",
];

/// The stored-dB grid the shader reads: `capacity` slots of `bins` bytes, slab
/// `key` living in slot `key.rem_euclid(capacity)`.
///
/// The GPU copy is padded per slab (see [`slab_stride`]) and is otherwise
/// these bytes exactly — the aggregator's own store, not a picture built from
/// it.
#[derive(Clone)]
pub struct SpectrogramGrid {
    pub capacity: u32,
    pub bins: u32,
    /// Keys `first_key .. first_key + run.len()`, one [`SpectrogramSlab`] of `bins` bytes
    /// each, oldest first.
    pub first_key: i64,
    pub run: Arc<[SpectrogramSlab]>,
    /// Actual full uploads, for the performance overlay. Never a cache input.
    pub full_uploads: Arc<AtomicU32>,
}

/// One slab's stored bytes, shared rather than copied between snapshots.
///
/// A scrolling run changes by a slab or two a frame, so consecutive snapshots
/// hold the same allocations for everything else, and
/// [`GridBuffer::changed_slabs`] finds the few that moved by pointer instead of
/// reading the whole run. That is exact only because a slab is never written
/// once shared: the aggregator writes the slab it is still filling through
/// `Arc::make_mut`, which copies it whenever a snapshot holds it.
pub type SpectrogramSlab = Arc<[u8]>;

impl SpectrogramGrid {
    /// Cut a slab-major byte run into slabs of `bins` bytes, in fresh
    /// allocations — a fixture helper for tests in this crate and the UI's; the
    /// live aggregator builds its slabs one at a time and shares them.
    pub fn slabs_of(bytes: &[u8], bins: usize) -> Arc<[SpectrogramSlab]> {
        bytes.chunks_exact(bins).map(SpectrogramSlab::from).collect()
    }
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

/// Draw `vertices` (a triangle list) into `rect`. See [`crate::PaneIds`]: each
/// pane id gets its own grid copy, which is the expensive thing here, and the
/// pipeline is shared.
#[allow(clippy::too_many_arguments)]
pub fn spectrogram_paint_callback(
    rect: egui::Rect,
    vertices: Vec<SpectrogramVertex>,
    grid: SpectrogramGrid,
    read: SpectrogramRead,
    shades: SpectrogramShades,
    target_format: wgpu::TextureFormat,
    ids: crate::PaneIds,
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
            pane_id: ids.pane,
            pass_nr: ids.pass_nr,
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

uniform_group! {
struct SpectrogramUniforms {
    origin_points: Float2,
    viewport_points: Float2,
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
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}
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

/// The resource owner remembers only the snapshot it actually uploaded.
/// Dropped callbacks never change this state; a new context starts without it.
struct GridBuffer {
    buffer: wgpu::Buffer,
    snapshot: SpectrogramGrid,
}

impl GridBuffer {
    fn fits(&self, grid: &SpectrogramGrid) -> bool {
        (self.snapshot.capacity, self.snapshot.bins) == (grid.capacity, grid.bins)
    }

    /// Indices needing writes, matched by absolute slab key rather than ring slot.
    /// A newly entering key must be written even if another key had equal bytes.
    ///
    /// The same allocation is the same bytes (see [`SpectrogramSlab`]), so a slab both
    /// snapshots share costs a pointer comparison. A different allocation —
    /// the slab being filled, or every slab after a refold — is compared by
    /// bytes, so a refold that reproduced a slab still skips its write.
    fn changed_slabs(&self, grid: &SpectrogramGrid) -> Vec<usize> {
        let previous = &self.snapshot;
        if previous.first_key == grid.first_key && Arc::ptr_eq(&previous.run, &grid.run) {
            return Vec::new();
        }
        (0..grid.run.len())
            .filter(|&j| {
                let at = grid.first_key + j as i64 - previous.first_key;
                usize::try_from(at).ok().and_then(|at| previous.run.get(at)).is_none_or(|held| {
                    let fresh = &grid.run[j];
                    !Arc::ptr_eq(held, fresh) && held[..] != fresh[..]
                })
            })
            .collect()
    }
}

/// The gradient table's GPU copy: a `levels` x 1 texture read with
/// `textureLoad`, so the shader owns the blend and no sampler filters between
/// two entries of a table that is already sampled per level.
struct LutTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    levels: u32,
    /// The immutable table actually uploaded, independent of callback construction.
    snapshot: Arc<Vec<[u8; 4]>>,
    #[cfg(test)]
    uploads: usize,
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

#[cfg(test)]
pub(super) fn timing_pipeline_startup(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
) -> [std::time::Duration; 2] {
    let started = std::time::Instant::now();
    let resources = SpectrogramResources::new(device, format);
    let plain = started.elapsed();
    let started = std::time::Instant::now();
    let _cloud = atmosphere::Pipelines::new(device, format, &resources.layout);
    [plain, started.elapsed()]
}

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
        let shader = spectrogram_shader(device);
        SpectrogramResources {
            pipeline: create_spectrogram_pipeline(
                device,
                &shader,
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
            uniform_buffer: crate::gpu_memory::buffer(
                device,
                &wgpu::BufferDescriptor {
                    label: Some("spectrogram_uniforms"),
                    size: std::mem::size_of::<SpectrogramUniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            ),
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

fn spectrogram_shader(device: &wgpu::Device) -> wgpu::ShaderModule {
    #[cfg(not(test))]
    let source = SPECTROGRAM_SRC.into();
    #[cfg(test)]
    let source = tests::pipeline_source();
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("spectrogram_shader"),
        source: wgpu::ShaderSource::Wgsl(source),
    })
}

/// The heatmap pipeline: a triangle list, blended exactly the way egui blends
/// its own shapes so the heatmap composites under the notes identically to the
/// tessellated mesh it replaces.
fn create_spectrogram_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::BindGroupLayout,
    extra_layout: Option<&wgpu::BindGroupLayout>,
    fragment: &str,
) -> wgpu::RenderPipeline {
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
            module: shader,
            entry_point: Some("vs_heatmap"),
            compilation_options: Default::default(),
            buffers: &[SpectrogramVertex::LAYOUT],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
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
        let sampling = atmosphere::CloudSampling::default();
        #[cfg(test)]
        let sampling =
            callback_resources.get::<atmosphere::CloudSampling>().copied().unwrap_or(sampling);
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
        let run_slabs = self.grid.run.len();
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
            if let Some(target) = pane.cloud.as_mut() {
                target.invalidate_memory();
            }
            return Vec::new();
        }
        debug_assert!(self.grid.run.iter().all(|slab| slab.len() == bins));
        debug_assert!(
            run_slabs <= self.grid.capacity as usize,
            "a run of {run_slabs} slabs puts two keys in one of {} slots",
            self.grid.capacity
        );

        let previous = pane.grid.as_ref().filter(|g| g.fits(&self.grid));
        let changed = previous.map(|g| g.changed_slabs(&self.grid));
        let full = changed.as_ref().is_none_or(|slabs| slabs.len() * 2 > run_slabs);
        let mut remade = previous.is_none();
        if full {
            let size = u64::from(self.grid.capacity) * u64::from(stride);
            // A content replacement can reuse the allocation and bind group.
            let kept = pane.grid.take().filter(|g| g.fits(&self.grid));
            let buffer = match kept {
                Some(held) => held.buffer,
                None => crate::gpu_memory::buffer(
                    device,
                    &wgpu::BufferDescriptor {
                        label: Some("spectrogram_grid"),
                        size,
                        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    },
                ),
            };
            // Zero unoccupied ring slots too, so reusing a buffer is equivalent
            // to a fresh allocation. Bulk replacement avoids many scattered writes.
            let mut staging = vec![0u8; size as usize];
            for j in 0..run_slabs {
                let at = slot_of(self.grid.first_key + j as i64, self.grid.capacity) as usize
                    * stride as usize;
                staging[at..at + bins].copy_from_slice(&self.grid.run[j]);
            }
            queue.write_buffer(&buffer, 0, &staging);
            self.grid.full_uploads.fetch_add(1, Ordering::Relaxed);
            pane.grid = Some(GridBuffer { buffer, snapshot: self.grid.clone() });
        } else {
            let held = pane.grid.as_mut().expect("a partial upload has an existing buffer");
            // Production slabs are aligned already; generic test grids may need padding.
            let mut padded = (bins != stride as usize).then(|| vec![0u8; stride as usize]);
            for j in changed.expect("an existing buffer was compared") {
                let slab = &self.grid.run[j][..];
                let bytes = match padded.as_mut() {
                    Some(padded) => {
                        padded[..bins].copy_from_slice(slab);
                        padded.as_slice()
                    }
                    None => slab,
                };
                let slot = slot_of(self.grid.first_key + j as i64, self.grid.capacity);
                queue.write_buffer(&held.buffer, u64::from(slot) * u64::from(stride), bytes);
            }
            // Even a zero-write comparison can change the retained run extent.
            held.snapshot = self.grid.clone();
        }

        let fresh_lut = pane.lut.as_ref().is_none_or(|l| l.levels != levels);
        if fresh_lut {
            let texture = crate::gpu_memory::texture(
                device,
                &wgpu::TextureDescriptor {
                    label: Some("spectrogram_lut"),
                    size: wgpu::Extent3d { width: levels, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                },
            );
            let view = texture.create_view(&Default::default());
            pane.lut = Some(LutTexture {
                texture,
                view,
                levels,
                snapshot: self.shades.lut.clone(),
                #[cfg(test)]
                uploads: 0,
            });
            remade = true;
        }
        let lut = pane.lut.as_mut().expect("created above when missing");
        if fresh_lut
            || (!Arc::ptr_eq(&lut.snapshot, &self.shades.lut) && lut.snapshot != self.shades.lut)
        {
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
            #[cfg(test)]
            {
                lut.uploads += 1;
            }
        }
        // An equal-content replacement becomes the identity fast path next frame.
        lut.snapshot = self.shades.lut.clone();

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

        // The light-only quad rides after the painted mesh: the source pass
        // draws both, `paint` only the first `count`.
        let light_hold = self.atmosphere.and_then(|a| a.light_hold);
        let vertex_total = self.vertices.len() + light_hold.map_or(0, |quad| quad.len());
        if vertex_total > pane.vertex_capacity {
            pane.vertex_capacity = vertex_total.next_power_of_two();
            pane.vertex_buffer = create_vertex_buffer::<SpectrogramVertex>(
                device,
                "spectrogram_vertices",
                pane.vertex_capacity,
            );
        }
        pane.count = self.vertices.len() as u32;
        queue.write_buffer(&pane.vertex_buffer, 0, bytemuck::cast_slice(&self.vertices));
        if let Some(quad) = &light_hold {
            let offset = std::mem::size_of_val(self.vertices.as_slice()) as wgpu::BufferAddress;
            queue.write_buffer(&pane.vertex_buffer, offset, bytemuck::cast_slice(quad));
        }

        let ppp = screen_descriptor.pixels_per_point.max(f32::EPSILON);
        let uniforms = SpectrogramUniforms {
            // The whole surface, which is the viewport `paint` draws into.
            origin_points: Float2([0.0, 0.0]),
            viewport_points: Float2([
                screen_descriptor.size_in_pixels[0] as f32 / ppp,
                screen_descriptor.size_in_pixels[1] as f32 / ppp,
            ]),
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
            _pad0: 0,
            _pad1: 0,
            _pad2: 0,
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
            .filter(|a| a.settings.effects().light())
        {
            let viewport = egui::epaint::ViewportInPixels::from_points(
                &self.rect,
                ppp,
                screen_descriptor.size_in_pixels,
            );
            let pixels = [viewport.width_px.max(0) as u32, viewport.height_px.max(0) as u32];
            if let Some(plan) = atmosphere::Plan::new(
                pixels,
                ppp,
                settings,
                sampling,
                pane.cloud.as_ref(),
                device.limits().max_texture_dimension_2d,
            ) {
                let cloud = cloud.get_or_insert_with(|| {
                    atmosphere::Pipelines::new(device, self.target_format, layout)
                });
                let rect = egui::Rect::from_min_size(
                    egui::pos2(viewport.left_px as f32 / ppp, viewport.top_px as f32 / ppp),
                    egui::vec2(pixels[0] as f32 / ppp, pixels[1] as f32 / ppp),
                );
                let grid = &pane.grid.as_ref().expect("drawable grid").buffer;
                let lut = &pane.lut.as_ref().expect("drawable gradient").view;
                // The callback can include the spectrum above the Stars region.
                // Shade only pixels the backdrop or measured mesh can read.
                // Include both so clipped/extended data quads stay supported.
                let star_coverage = plan.stars.map(|_| {
                    let bounds = self.vertices.iter().fold(settings.region, |bounds, v| {
                        bounds.union(egui::Rect::from_min_max(v.pos.into(), v.pos.into()))
                    });
                    let lo = [bounds.left(), bounds.top()];
                    let hi = [bounds.right(), bounds.bottom()];
                    let origin = [viewport.left_px, viewport.top_px];
                    // Conservative raster bounds include fractional edge pixels.
                    // The pane's own scissor still limits the final paint.
                    let start: [u32; 2] = std::array::from_fn(|i| {
                        ((lo[i] * ppp).floor() as i32 - origin[i]).clamp(0, pixels[i] as i32) as u32
                    });
                    let end: [u32; 2] = std::array::from_fn(|i| {
                        ((hi[i] * ppp).ceil() as i32 - origin[i])
                            .clamp(start[i] as i32, pixels[i] as i32) as u32
                    });
                    [start[0], start[1], end[0] - start[0], end[1] - start[1]]
                });
                let target = atmosphere::Targets::prepare(
                    &mut pane.cloud,
                    device,
                    cloud,
                    plan.shape,
                    layout,
                    grid,
                    lut,
                );
                target.update(
                    queue,
                    uniforms,
                    rect,
                    ppp,
                    settings,
                    &plan,
                    self.shades.lut.clone(),
                    self.grid.first_key + self.grid.run.len() as i64,
                );
                target.encode(
                    egui_encoder,
                    cloud,
                    &plan,
                    atmosphere::Draw {
                        style: settings.settings.cloud_style,
                        mesh: pane.vertex_buffer.slice(..),
                        count: pane.count + settings.light_hold.map_or(0, |quad| quad.len() as u32),
                        pixels,
                        star_coverage,
                    },
                );
                pane.cloud_ready = true;
            }
        }

        // No cloud this frame — every effect off, or no light field to draw
        // — so its history is freed rather than held for a look not drawn.
        if !pane.cloud_ready {
            if let Some(target) = pane.cloud.as_mut() {
                target.release_memory();
            }
        }
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
            render_pass.set_bind_group(1, cloud.composite_group(), &[]);
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
/// fresh resources hold no grid to patch. A test of incremental uploads
/// has to hand the same resources one frame after
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

    /// Simulate closing the renderer while preserving its callers' CPU snapshots.
    pub fn reset_resources(&mut self) {
        self.resources = CallbackResources::default();
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
    /// so a full-upload reference needs a fresh id or cleared resources.
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
mod tests;

#[cfg(all(test, target_os = "macos", feature = "shader-assets-tools"))]
pub(super) fn asset_catalog(device: &wgpu::Device, format: wgpu::TextureFormat) {
    let resources = SpectrogramResources::new(device, format);
    // Runtime creation stays lazy, but the strict Metal catalog must cover
    // every production route before the first enabled atmospheric frame.
    drop(atmosphere::Pipelines::new(device, format, &resources.layout));
}
