//! The wgpu lattice renderer, packaged as an egui paint callback.
//!
//! The same code path runs in both shells: the standalone harness (eframe
//! with the wgpu backend) and the plugin editor (egui-baseview with the wgpu
//! backend). A pane that wants to show the lattice allocates a rect and
//! adds [`lattice_paint_callback`] to the painter; pipelines and buffers are
//! created lazily on first paint and cached in egui-wgpu's
//! `CallbackResources`.
//!
//! Rendering model: one instanced draw of camera-facing quads (billboards),
//! sorted back-to-front on the CPU (the egui render pass has no depth
//! buffer). This is plenty for a lattice-sized scene. When effects need
//! depth testing or post-processing (bloom etc.), the upgrade path is to
//! render the scene into our own offscreen texture + depth buffer in
//! `prepare()` and composite that texture here instead.
//!
//! With the `hot-reload` feature (enabled by the standalone harness), the
//! .wgsl file is watched on disk and the pipeline rebuilds on save —
//! validated first, so a broken edit logs an error and keeps the old
//! pipeline instead of crashing. Release plugin builds keep `include_str!`
//! only.

use std::collections::HashMap;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use lattice_scene::Scene;

// Shells name texture formats through this re-export so every crate agrees
// on the wgpu version.
pub use egui_wgpu::wgpu;

const SHADER_SRC: &str = include_str!("shaders/lattice.wgsl");

/// Entry points a (re)loaded shader must provide.
#[cfg(any(test, feature = "hot-reload"))]
const REQUIRED_ENTRY_POINTS: &[&str] = &["vs_main", "fs_main", "vs_edge", "fs_edge"];

/// Watches the shader source on disk (dev builds only). The first sighting
/// of the file only records a baseline mtime; edits after launch trigger
/// reloads.
#[cfg(feature = "hot-reload")]
struct ShaderWatcher {
    path: std::path::PathBuf,
    mtime: Option<std::time::SystemTime>,
    next_check: std::time::Instant,
}

#[cfg(feature = "hot-reload")]
impl ShaderWatcher {
    fn new() -> Self {
        ShaderWatcher {
            path: std::path::PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/shaders/lattice.wgsl"
            )),
            mtime: None,
            next_check: std::time::Instant::now(),
        }
    }


    /// Returns the new shader source when the file changed since last poll.
    fn poll(&mut self) -> Option<String> {
        let now = std::time::Instant::now();
        if now < self.next_check {
            return None;
        }
        self.next_check = now + std::time::Duration::from_millis(500);

        let mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok()?;
        match self.mtime.replace(mtime) {
            None => None, // baseline; the baked shader is current
            Some(previous) if previous == mtime => None,
            Some(_) => std::fs::read_to_string(&self.path).ok(),
        }
    }
}

/// Parse + validate WGSL and check our entry points exist, so a bad edit
/// never reaches wgpu's panicking error handler. Also exercised by a unit
/// test against the baked-in source: plugin builds have no hot-reload, so
/// without that test a broken commit would first surface as a crash inside
/// a DAW at first paint.
#[cfg(any(test, feature = "hot-reload"))]
fn validate_wgsl(source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| e.emit_to_string(source))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| format!("{e:?}"))?;
    for required in REQUIRED_ENTRY_POINTS {
        if !module.entry_points.iter().any(|ep| ep.name == *required) {
            return Err(format!("missing entry point `{required}`"));
        }
    }
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [f32; 16],
    cam_right: [f32; 4],
    cam_up: [f32; 4],
    misc: [f32; 4],
}

// The octave packing fits OCTAVE_SLOTS 8-bit levels into 3 u32 words;
// growing the constant in lattice-scene past 12 would index out of bounds
// at runtime here, so fail the build instead.
const _: () = assert!(lattice_scene::OCTAVE_SLOTS <= 12);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuInstance {
    world_pos: [f32; 3],
    color: [f32; 4],
    /// x: activation, y: hovered, z: age (s since note-on), w: outlined.
    params: [f32; 4],
    /// Per-octave activation, 8 bits per slot, little-endian packed
    /// (slot 0 = lowest byte of the first word).
    octaves: [u32; 3],
    /// Per-note animation seed (small constant, not a timestamp).
    seed: f32,
    /// The node's pitch class in cents (0..1200); dots mode uses it to place
    /// each octave dot at the note's absolute-pitch angle.
    cents: f32,
}

impl GpuInstance {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GpuInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x4, 2 => Float32x4, 3 => Uint32x3, 4 => Float32, 5 => Float32
        ],
    };
}

/// Pack the per-octave activation levels into the bit layout
/// `octave_level()` in lattice.wgsl unpacks: 8 bits per slot,
/// little-endian (slot 0 = lowest byte of the first word).
fn pack_octaves(levels: &[f32; lattice_scene::OCTAVE_SLOTS]) -> [u32; 3] {
    let mut octaves = [0u32; 3];
    for (slot, &level) in levels.iter().enumerate() {
        let byte = (level.clamp(0.0, 1.0) * 255.0).round() as u32;
        octaves[slot / 4] |= byte << ((slot % 4) * 8);
    }
    octaves
}

/// One chord edge (a beam between two active adjacent nodes).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuEdge {
    /// xyz: endpoint A, w: strength.
    a_strength: [f32; 4],
    /// xyz: endpoint B, w: unused.
    b_pad: [f32; 4],
    color: [f32; 4],
}

impl GpuEdge {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GpuEdge>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4
        ],
    };
}

/// Build the egui shape that renders `scene` into `rect`. `pane_id` must be
/// unique per lattice view shown in the same frame (each gets its own GPU
/// buffers; the pipeline is shared).
pub fn lattice_paint_callback(
    rect: egui::Rect,
    scene: &Scene,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
) -> egui::PaintCallback {
    let aspect = rect.width() / rect.height().max(1.0);
    egui_wgpu::Callback::new_paint_callback(
        rect,
        LatticeCallback::from_scene(scene, aspect, target_format, pane_id),
    )
}

/// Per-frame, per-pane draw data, computed on the UI thread.
struct LatticeCallback {
    instances: Vec<GpuInstance>,
    edges: Vec<GpuEdge>,
    uniforms: Uniforms,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
}

impl LatticeCallback {
    fn from_scene(
        scene: &Scene,
        aspect: f32,
        target_format: wgpu::TextureFormat,
        pane_id: u64,
    ) -> Self {
        let camera = scene.camera;
        let view_proj = camera.view_proj(aspect);
        let (right, up) = camera.right_up();

        // Sort back-to-front along the view direction: no depth buffer in
        // the egui pass, so alpha blending relies on draw order.
        let eye = camera.eye();
        let forward = (camera.target - eye).normalize_or_zero();
        let mut order: Vec<(f32, &lattice_scene::NodeInstance)> = scene
            .nodes
            .iter()
            .map(|n| ((n.world_pos - eye).dot(forward), n))
            .collect();
        order.sort_by(|a, b| b.0.total_cmp(&a.0));

        let instances = order
            .into_iter()
            .map(|(_, n)| GpuInstance {
                world_pos: n.world_pos.to_array(),
                color: n.color.to_array(),
                params: [
                    n.activation,
                    if n.hovered { 1.0 } else { 0.0 },
                    n.age,
                    if n.outlined { 1.0 } else { 0.0 },
                ],
                octaves: pack_octaves(&n.octaves),
                seed: n.seed,
                cents: n.cents,
            })
            .collect();

        let edges = scene
            .edges
            .iter()
            .map(|e| GpuEdge {
                a_strength: [e.a.x, e.a.y, e.a.z, e.strength],
                b_pad: [e.b.x, e.b.y, e.b.z, 0.0],
                color: e.color.to_array(),
            })
            .collect();

        LatticeCallback {
            instances,
            edges,
            uniforms: Uniforms {
                view_proj: view_proj.to_cols_array(),
                cam_right: right.extend(0.0).to_array(),
                cam_up: up.extend(0.0).to_array(),
                misc: [
                    scene.time,
                    scene.node_radius,
                    scene.octave_style.shader_index() as f32,
                    scene.node_style.shader_index() as f32,
                ],
            },
            target_format,
            pane_id,
        }
    }
}

/// GPU objects cached across frames in egui-wgpu's `CallbackResources`.
struct LatticeResources {
    pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    panes: HashMap<u64, PaneBuffers>,
    #[cfg(feature = "hot-reload")]
    watcher: ShaderWatcher,
}

struct PaneBuffers {
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    instance_count: u32,
    edge_buffer: wgpu::Buffer,
    edge_capacity: usize,
    edge_count: u32,
}

/// Build one of our pipelines from WGSL source (startup uses the baked-in
/// source; hot-reload rebuilds from disk). Node and edge pipelines share
/// the module, bind group layout, blending, and topology; only entry
/// points and vertex layout differ.
fn create_pipeline(
    device: &wgpu::Device,
    shader_src: &str,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    entry_points: (&str, &str),
    vertex_layout: wgpu::VertexBufferLayout<'_>,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("lattice_shader"),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_pipeline_layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        ..Default::default()
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("lattice_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some(entry_points.0),
            compilation_options: Default::default(),
            buffers: &[vertex_layout],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(entry_points.1),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                // Shader outputs premultiplied alpha.
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        // Must match the egui render pass, which is created without
        // MSAA in both eframe (default) and egui-baseview (default).
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Build both pipelines from one source.
fn create_pipelines(
    device: &wgpu::Device,
    shader_src: &str,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    (
        create_pipeline(
            device,
            shader_src,
            target_format,
            bind_group_layout,
            ("vs_main", "fs_main"),
            GpuInstance::LAYOUT,
        ),
        create_pipeline(
            device,
            shader_src,
            target_format,
            bind_group_layout,
            ("vs_edge", "fs_edge"),
            GpuEdge::LAYOUT,
        ),
    )
}

impl LatticeResources {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lattice_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let (pipeline, edge_pipeline) =
            create_pipelines(device, SHADER_SRC, target_format, &bind_group_layout);

        LatticeResources {
            pipeline,
            edge_pipeline,
            bind_group_layout,
            target_format,
            panes: HashMap::new(),
            #[cfg(feature = "hot-reload")]
            watcher: ShaderWatcher::new(),
        }
    }

    fn pane_buffers(&mut self, device: &wgpu::Device, pane_id: u64) -> &mut PaneBuffers {
        let layout = &self.bind_group_layout;
        self.panes.entry(pane_id).or_insert_with(|| {
            let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lattice_uniforms"),
                size: std::mem::size_of::<Uniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lattice_bind_group"),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            });
            PaneBuffers {
                uniform_buffer,
                bind_group,
                instance_buffer: create_instance_buffer(device, 256),
                instance_capacity: 256,
                instance_count: 0,
                edge_buffer: create_edge_buffer(device, 64),
                edge_capacity: 64,
                edge_count: 0,
            }
        })
    }
}

fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("lattice_instances"),
        size: (capacity * std::mem::size_of::<GpuInstance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_edge_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("lattice_edges"),
        size: (capacity * std::mem::size_of::<GpuEdge>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl CallbackTrait for LatticeCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        // Lazily (re)create shared resources. Recreate if the target format
        // changed (it can't today, but this keeps the invariant explicit).
        let recreate = callback_resources
            .get::<LatticeResources>()
            .is_none_or(|r| r.target_format != self.target_format);
        if recreate {
            callback_resources.insert(LatticeResources::new(device, self.target_format));
        }
        let resources: &mut LatticeResources = callback_resources
            .get_mut()
            .expect("inserted above when missing");

        // Dev builds: pick up edits to the .wgsl on disk. A broken edit is
        // rejected with a message; the previous pipeline keeps rendering.
        #[cfg(feature = "hot-reload")]
        if let Some(source) = resources.watcher.poll() {
            match validate_wgsl(&source) {
                Ok(()) => {
                    let (pipeline, edge_pipeline) = create_pipelines(
                        device,
                        &source,
                        resources.target_format,
                        &resources.bind_group_layout,
                    );
                    resources.pipeline = pipeline;
                    resources.edge_pipeline = edge_pipeline;
                    eprintln!("[lattice-render] shader hot-reloaded");
                }
                Err(err) => {
                    eprintln!("[lattice-render] shader reload REJECTED, keeping old pipeline:\n{err}");
                }
            }
        }

        let pane = resources.pane_buffers(device, self.pane_id);

        if self.instances.len() > pane.instance_capacity {
            pane.instance_capacity = self.instances.len().next_power_of_two();
            pane.instance_buffer = create_instance_buffer(device, pane.instance_capacity);
        }
        pane.instance_count = self.instances.len() as u32;
        if !self.instances.is_empty() {
            queue.write_buffer(
                &pane.instance_buffer,
                0,
                bytemuck::cast_slice(&self.instances),
            );
        }

        if self.edges.len() > pane.edge_capacity {
            pane.edge_capacity = self.edges.len().next_power_of_two();
            pane.edge_buffer = create_edge_buffer(device, pane.edge_capacity);
        }
        pane.edge_count = self.edges.len() as u32;
        if !self.edges.is_empty() {
            queue.write_buffer(&pane.edge_buffer, 0, bytemuck::cast_slice(&self.edges));
        }

        queue.write_buffer(&pane.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));

        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<LatticeResources>() else {
            return;
        };
        let Some(pane) = resources.panes.get(&self.pane_id) else {
            return;
        };
        if pane.instance_count == 0 {
            return;
        }

        // Edges draw under the nodes so discs own the joints.
        if pane.edge_count > 0 {
            render_pass.set_pipeline(&resources.edge_pipeline);
            render_pass.set_bind_group(0, &pane.bind_group, &[]);
            render_pass.set_vertex_buffer(0, pane.edge_buffer.slice(..));
            render_pass.draw(0..4, 0..pane.edge_count);
        }

        render_pass.set_pipeline(&resources.pipeline);
        render_pass.set_bind_group(0, &pane.bind_group, &[]);
        render_pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
        render_pass.draw(0..4, 0..pane.instance_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baked_shader_validates() {
        validate_wgsl(SHADER_SRC)
            .expect("baked lattice.wgsl must parse, validate, and keep its entry points");
    }

    #[test]
    fn octave_packing_matches_the_documented_layout() {
        let mut levels = [0.0f32; lattice_scene::OCTAVE_SLOTS];
        levels[0] = 1.0; // lowest byte of word 0
        levels[3] = 0.5; // highest byte of word 0
        levels[4] = 1.0; // lowest byte of word 1
        levels[9] = 1.0; // second byte of word 2
        let words = pack_octaves(&levels);
        assert_eq!(words[0] & 0xFF, 255);
        assert_eq!((words[0] >> 24) & 0xFF, 128);
        assert_eq!(words[1] & 0xFF, 255);
        assert_eq!(words[2] & 0xFF, 0);
        assert_eq!((words[2] >> 8) & 0xFF, 255);
        // Out-of-range levels clamp instead of corrupting neighbors.
        let words = pack_octaves(&[2.0; lattice_scene::OCTAVE_SLOTS]);
        assert_eq!(words[0], 0xFFFF_FFFF);
    }

    /// Build the real pipelines against a headless device. This validates
    /// the vertex-layout <-> shader-input contract (attribute locations,
    /// formats, strides) that neither the naga check (shader only) nor the
    /// type system (Rust side only) covers — a mismatch otherwise panics
    /// at first paint inside a host.
    #[test]
    fn pipelines_build_against_a_headless_device() {
        let instance = wgpu::Instance::default();
        let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        else {
            eprintln!("no GPU adapter available; skipping pipeline-build test");
            return;
        };
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("headless device");
        let _resources =
            LatticeResources::new(&device, wgpu::TextureFormat::Bgra8Unorm);
    }
}
