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

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

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
    panes: HashMap<u64, SpectrogramPane>,
}

/// How many egui passes a pane may go unseen before its buffers are dropped.
///
/// A pane is prepared once per pass while it is on screen, so this is about
/// two seconds at 60 fps however many placements are live.
/// What is being held is the grid copy — up to 15.7 MB for a whole-song ring
/// at 4096 slabs — so a closed tab keeping one is worth a sweep, and a pane
/// hidden for a frame keeping one is worth not rebuilding.
const PANE_TTL_PASSES: u64 = 120;

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
    /// Egui's cumulative pass number when this pane was last drawn.
    last_seen_pass: u64,
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
            panes: HashMap::new(),
        }
    }
}

impl SpectrogramPane {
    /// This pane's buffers, made on first sight of its id and stamped with
    /// `pass_nr` so [`SpectrogramPane::evict_unseen`] can tell a live pane
    /// from one whose tab was closed.
    fn get<'a>(
        panes: &'a mut HashMap<u64, SpectrogramPane>,
        device: &wgpu::Device,
        pane_id: u64,
        pass_nr: u64,
    ) -> &'a mut SpectrogramPane {
        let pane = panes.entry(pane_id).or_insert_with(|| SpectrogramPane {
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
            last_seen_pass: pass_nr,
            cloud: None,
            cloud_ready: false,
        });
        pane.last_seen_pass = pass_nr;
        pane
    }

    /// Drop every pane that has not been drawn for [`PANE_TTL_PASSES`].
    ///
    /// A spectrogram's id is its surface (the docked pane, the Render
    /// preview), and a closed tab simply stops calling back — there is no
    /// teardown to hang this on, so the panes still being prepared are the
    /// only evidence of which ones exist. Run from whichever pane IS
    /// preparing, so a lone survivor still clears the others.
    fn evict_unseen(panes: &mut HashMap<u64, SpectrogramPane>, pass_nr: u64) {
        panes.retain(|_, pane| pass_nr.saturating_sub(pane.last_seen_pass) < PANE_TTL_PASSES);
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
        SpectrogramPane::evict_unseen(panes, self.pass_nr);
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
            .filter(|a| {
                a.settings.style != harmonigraph_scene::SpectrogramStyle::Plain
                    && ((a.settings.pitch_softness > 0.0 || a.settings.time_softness > 0.0)
                        || a.settings.style == harmonigraph_scene::SpectrogramStyle::Lava)
            })
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
                let resize = pane.cloud.as_ref().is_none_or(|c| c.size != size);
                if resize {
                    pane.cloud =
                        Some(atmosphere::Targets::new(device, cloud, size, layout, grid, lut));
                }
                let target = pane.cloud.as_mut().expect("allocated above");
                target.update(queue, uniforms, rect, ppp, settings);
                // Lava still needs its transfer/composite at zero widths,
                // but the one-pixel source would integrate the whole history
                // only for smoothed_level to discard that expensive result.
                if settings.settings.pitch_softness > 0.0 || settings.settings.time_softness > 0.0 {
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
        let (w, h) = (SIZE[0] as f32, SIZE[1] as f32);
        let n = run_slabs as f32;
        let v = |x: f32, y: f32| SpectrogramVertex { pos: [x, y], slab: x / w * n, t: 1.0 - y / h };
        vec![v(0.0, 0.0), v(w, 0.0), v(w, h), v(0.0, 0.0), v(w, h), v(0.0, h)]
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
        let texture =
            render_to_texture(device, queue, SIZE, cb.target_format, wgpu::Color::BLACK, |pass| {
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
                style: harmonigraph_scene::SpectrogramStyle::Blur,
                // The probes read the diffusion transfer; a cloud over them
                // would move the very pixels they measure.
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

    #[test]
    fn lava_preserves_silence_quiet_fields_and_nested_levels() {
        let Some((device, queue)) = headless_device() else { return };
        let mut cb = cloud_fixture();
        cb.atmosphere.as_mut().unwrap().settings.style = harmonigraph_scene::SpectrogramStyle::Lava;
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
        for style in [
            harmonigraph_scene::SpectrogramStyle::Plain,
            harmonigraph_scene::SpectrogramStyle::Blur,
            harmonigraph_scene::SpectrogramStyle::Lava,
        ] {
            let mut cb = cloud_fixture();
            cb.target_format = wgpu::TextureFormat::Rgba16Float;
            cb.grid.run = Arc::new(vec![96; cb.grid.run.len()]);
            cb.shades.lut = Arc::new(vec![[128, 128, 128, 255]; 256]);
            cb.atmosphere.as_mut().unwrap().settings.style = style;
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
                assert!((actual - expected).abs() < 0.001, "{style:?}: {actual} != {expected}");
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
            resources.get::<SpectrogramResources>().unwrap().panes[&0]
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
        cb.atmosphere.as_mut().unwrap().settings.style = harmonigraph_scene::SpectrogramStyle::Lava;
        for width in [0.0, -1.0] {
            cb.atmosphere.as_mut().unwrap().settings.pitch_softness = width;
            cb.atmosphere.as_mut().unwrap().settings.time_softness = width;
            let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
            let mut encoder = device.create_command_encoder(&Default::default());
            let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
            let pane = &resources.get::<SpectrogramResources>().unwrap().panes[&0];
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
        for (style, name) in [
            (harmonigraph_scene::SpectrogramStyle::Plain, "spectrogram-style-plain"),
            (harmonigraph_scene::SpectrogramStyle::Blur, "spectrogram-style-blur"),
            (harmonigraph_scene::SpectrogramStyle::Lava, "spectrogram-style-lava"),
        ] {
            let settings = &mut cb.atmosphere.as_mut().unwrap().settings;
            settings.style = style;
            let frame = fresh_frame(&device, &queue, &cb);
            harmonigraph_golden::Gate::new(env!("CARGO_MANIFEST_DIR")).check(name, SIZE, &frame);
        }
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
                phases.push(
                    [
                        harmonigraph_scene::SpectrogramStyle::Plain,
                        harmonigraph_scene::SpectrogramStyle::Blur,
                    ]
                    .map(|style| {
                        cb.atmosphere.as_mut().unwrap().settings.style = style;
                        fresh_frame(&device, &queue, &cb)
                    }),
                );
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
    fn spectral_diffusion_softens_faint_detail_with_one_fade_to_black() {
        let Some((device, queue)) = headless_device() else { return };
        let mut cb = cloud_fixture();
        cb.grid.run = Arc::new(cb.grid.run.iter().map(|&v| if v > 0 { 102 } else { 0 }).collect());
        // An edited palette may start above black. The diffused tail must
        // still reach actual black rather than leave that first slice glowing.
        Arc::make_mut(&mut cb.shades.lut)[0] = [80, 0, 0, 255];
        let soft = fresh_frame(&device, &queue, &cb);
        cb.atmosphere.as_mut().unwrap().settings.style =
            harmonigraph_scene::SpectrogramStyle::Plain;
        let zero = fresh_frame(&device, &queue, &cb);
        cb.atmosphere = None;
        let plain = fresh_frame(&device, &queue, &cb);
        assert_eq!(zero, plain, "Plain style did not restore the measured heatmap");
        let pixel = |frame: &[u8], y| frame[(y * SIZE[0] as usize + 64) * 4 + 2];
        assert_eq!(pixel(&plain, 63), 102, "fixture missed the faint ridge");
        assert!(pixel(&soft, 63) < 82, "faint grain kept its original contrast");
        assert!(pixel(&soft, 60) > pixel(&plain, 60), "softened body never formed");
        let far = (16 * SIZE[0] as usize + 64) * 4;
        assert_eq!(&soft[far..far + 4], &[0, 0, 0, 255], "diffusion lifted the black background");
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
                let target = resources.get::<SpectrogramResources>().unwrap().panes[&0]
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
            resources.get::<SpectrogramResources>().unwrap().panes[&0].cloud.as_ref().unwrap().size,
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
        cb.atmosphere.as_mut().unwrap().settings.style =
            harmonigraph_scene::SpectrogramStyle::Plain;
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
            "Plain style changed the original heatmap"
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
        assert!(!resources.get::<SpectrogramResources>().unwrap().panes[&0].cloud_ready);
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
        let (mut docked, preview) = (of_pane(0), of_pane(1));
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

        for pass_nr in 1..=PANE_TTL_PASSES {
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
    /// as against any other. It is the negative control now, and the inversion
    /// is the change: a layer cut out of the sound has nothing to cut here.
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
    /// same cloud shape, same relief, same glint, same ambient. Only
    /// `scale_refract` moves, so what is measured is the displacement alone.
    ///
    /// Measured: 3.2% of the pane over the ridge fixture, and EXACTLY zero
    /// over the flat one. The 3.2 is not small for the wrong reason — this
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
                // Overcast, so the cloud covers the pane and the comparison is
                // not dominated by how much sky each frame happens to have.
                s.cloud_cover = 1.0;
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

    /// Cover is the knob that opens the sky, over its whole range.
    ///
    /// It is a threshold on the band-passed field, so this also says the band
    /// pass is wired up: a threshold on a plain blur would move with the
    /// picture's level rather than with this dial.
    #[test]
    fn cloud_cover_opens_the_sky() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let mut cb = cloud_fixture();
        cb.atmosphere.as_mut().unwrap().settings.cloud_depth = 0.0;
        let bare = fresh_frame(&device, &queue, &cb);
        cb.atmosphere.as_mut().unwrap().settings.cloud_depth = 0.9;
        let mut painted = |cover: f32| {
            cb.atmosphere.as_mut().unwrap().settings.cloud_cover = cover;
            let frame = fresh_frame(&device, &queue, &cb);
            bare.chunks_exact(4)
                .zip(frame.chunks_exact(4))
                .filter(|(a, b)| (0..3).any(|c| a[c].abs_diff(b[c]) > 3))
                .count()
        };
        let clear = painted(0.0);
        let mid = painted(0.5);
        let overcast = painted(1.0);
        assert!(
            clear < mid && mid < overcast,
            "cover did not open the sky monotonically: {clear} then {mid} then {overcast}"
        );
        // A factor rather than a doubling, and the ceiling here is the
        // FIXTURE's and not the knob's: this pane holds one narrow ridge in a
        // mostly dark field, so there is only so much concentration for any
        // threshold to find. That is the layer working — cover decides how
        // much of the sound's structure is drawn as cloud, not how much of the
        // pane is painted regardless. Over a pane of real music the same knob
        // runs from a scatter of billows to a near overcast.
        assert!(
            overcast > (clear as f32 * 1.4) as usize,
            "the whole range of the knob barely moved the cloud: {clear} to {overcast}"
        );
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
    /// Run at the deepest cover and the most pigment the dials reach, which is
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
        settings.cloud_cover = 1.0;
        settings.scale_glint = 1.0;
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
}

#[cfg(all(test, target_os = "macos", feature = "shader-assets-tools"))]
pub(super) fn asset_catalog(device: &wgpu::Device, format: wgpu::TextureFormat) {
    let resources = SpectrogramResources::new(device, format);
    // Runtime creation stays lazy, but the strict Metal catalog must cover
    // every production route before the first enabled atmospheric frame.
    drop(atmosphere::Pipelines::new(device, format, &resources.layout));
}
