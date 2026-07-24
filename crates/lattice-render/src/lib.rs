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
//! sorted back-to-front on the CPU, rendered in `prepare()` into a per-pane
//! offscreen color + depth target and composited into the egui pass in
//! `paint()` as one textured quad (blit.wgsl). Owning the pass is what
//! makes the render-scale option (super/sub-sampling) possible, and gives
//! post-processing (bloom etc.) a texture to read; the depth buffer is
//! written (pass-through `Always` test, so draw order still composites
//! exactly like the pre-offscreen renderer) but not yet read by anything.
//! `offscreen_composite_matches_direct_draw` in the tests pins down that
//! this path reproduces the old direct-to-egui-pass output.
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
const BLIT_SRC: &str = include_str!("shaders/blit.wgsl");

/// Depth format of the offscreen pass. Written for future depth-reading
/// effects; the scene pipelines test `Always` so it never affects output.
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Clamp on the render-scale view setting, over whatever the UI offers.
const RENDER_SCALE_RANGE: (f32, f32) = (0.25, 4.0);

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
    /// x: darkest_pitch, y: brightest_pitch (MIDI notes); z: render scale
    /// (the shader converts its screen-pixel AA softness to render
    /// pixels with it); w: bloom strength, which blit.wgsl reads (this
    /// slot is NOT free). The dots style maps a dot's pitch through x/y
    /// to index `pitch_lut`.
    misc2: [f32; 4],
    /// x: core radius in quad UV units (0 turns the core off); y/z: the
    /// outer octave layer's inner/outer band radii (same units, pre-
    /// sanitized by the scene so z > y); w: outer backdrop opacity 0..1
    /// (ghost the silent octaves to complete the ring, independent of the
    /// core; 0 = no backdrop, 1 = the full built-in ghost level).
    misc3: [f32; 4],
    /// Pitch->color lookup for the dots octave style (see lattice_scene's
    /// `pitch_ramp_lut`), matching the node disc gradient.
    pitch_lut: [[f32; 4]; lattice_scene::PITCH_LUT_N],
    /// Idle node color (the view's grid color at full alpha, so the grid
    /// lines and idle markers read as one layer): the home-sheet
    /// placeholder ring is drawn in this ONE constant color, so a
    /// releasing note's ring never shows the note's own color or snaps
    /// color when the voice is pruned.
    node_idle: [f32; 4],
    /// x: core solidity (0 = soft glow, 1 = solid orb), the single axis the
    /// core layer runs on; y: outer solidity (0 = soft glowy glyphs, 1 =
    /// crisp octave shapes); z: idle marker radius; w: idle marker style
    /// (0 none, 1 dot, 2 circle). (The blit pipeline binds only the head of
    /// this buffer, so trailing fields are safe to add here.)
    misc4: [f32; 4],
    /// x: grid line thickness as a multiple of the shader's built-in grid
    /// width; y: draw the melody/bass mark on the core (pitch class
    /// indicator); z: draw it on the octave glyphs; w unused. Every
    /// earlier misc slot is spoken for, so the grid's knob starts a new
    /// one — safe per the note on `misc4`.
    misc5: [f32; 4],
    /// x: trail mark style (0 off, 1 lift, 2 ring, 3 tint); y: trail
    /// strength 0..1; z/w unused. Both feed the idle-marker branch alone
    /// (see `TrailMark`); misc5 is full, so the trail starts its own slot.
    misc6: [f32; 4],
}

// The octave packing fits OCTAVE_SLOTS 8-bit levels into 3 u32 words;
// growing the constant in lattice-scene past 12 would index out of bounds
// at runtime here, so fail the build instead.
const _: () = assert!(lattice_scene::OCTAVE_SLOTS <= 12);

// The shader declares `pitch_lut` with a literal length; keep the two in
// lockstep so the uniform buffer and the WGSL agree.
const _: () = assert!(lattice_scene::PITCH_LUT_N == 16);

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuInstance {
    world_pos: [f32; 3],
    color: [f32; 4],
    /// x: activation, y: melody mark level, z: bass mark level, w: outlined
    /// (see lattice.wgsl). The mark levels ride here rather than in a vertex
    /// attribute of their own because y and z were dead padding — the vec4
    /// was always shipped whole.
    params: [f32; 4],
    /// Per-octave activation, 8 bits per slot, little-endian packed
    /// (slot 0 = lowest byte of the first word).
    octaves: [u32; 3],
    /// Per-note animation seed (small constant, not a timestamp).
    seed: f32,
    /// The node's pitch class in cents (0..1200); dots mode uses it to place
    /// each octave dot at the note's absolute-pitch angle.
    cents: f32,
    /// 1 when the node is on the home (center sevens) sheet: idle home
    /// nodes draw a blank placeholder ring.
    home: f32,
    /// Melody/bass marks: `[melody_slots, bass_slots]`, one bit per octave
    /// slot (see `NodeInstance::melody_slots`). Kept as integers rather
    /// than folded into the dead `params.y`/`params.z` floats because the
    /// shader masks them bitwise, which needs a flat-interpolated `u32`.
    marks: [u32; 2],
    /// Each mark's own color (see `NodeInstance::melody_color`): the marked
    /// note's, not a fixed livery, so a ring reads as belonging to the note
    /// it marks.
    melody_color: [f32; 4],
    bass_color: [f32; 4],
    /// How strongly the music is remembered at this node, 0..1 (see
    /// `NodeInstance::trail`). Reaches only the shader's idle-marker
    /// branch — a memory must never read as a sounding note.
    visited: f32,
}

impl GpuInstance {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GpuInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x4, 2 => Float32x4, 3 => Uint32x3, 4 => Float32,
            5 => Float32, 6 => Float32, 7 => Uint32x2,
            8 => Float32x4, 9 => Float32x4, 10 => Float32
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

/// One edge-pipeline instance: a chord beam between two active adjacent
/// nodes, or a faint background grid line.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuEdge {
    /// xyz: endpoint A, w: strength (grid lines: opacity).
    a_strength: [f32; 4],
    /// xyz: endpoint B, w: kind (0 chord beam, 1 grid line, 2 dashed
    /// grid line).
    b_kind: [f32; 4],
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
/// `gpu_ms` receives the GPU time of this pane's passes, in milliseconds, as
/// f32 bits — a few frames late (see [`GpuTimer`]) and only where the device
/// granted timestamp queries. Pass `None` for panes whose cost isn't the one
/// being reported, so a second lattice on screen can't overwrite the reading.
pub fn lattice_paint_callback(
    rect: egui::Rect,
    scene: &Scene,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    gpu_ms: Option<std::sync::Arc<std::sync::atomic::AtomicU32>>,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        LatticeCallback::from_scene(scene, rect.size(), target_format, pane_id, gpu_ms),
    )
}

/// Per-frame, per-pane draw data, computed on the UI thread.
struct LatticeCallback {
    instances: Vec<GpuInstance>,
    edges: Vec<GpuEdge>,
    uniforms: Uniforms,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    /// The callback rect's size in egui points; `prepare` multiplies by the
    /// screen's pixels-per-point and `render_scale` to size the offscreen
    /// target.
    size_points: [f32; 2],
    /// From the scene (a view setting), clamped to [`RENDER_SCALE_RANGE`].
    render_scale: f32,
    /// Where to publish this pane's GPU time; see [`lattice_paint_callback`].
    gpu_ms: Option<std::sync::Arc<std::sync::atomic::AtomicU32>>,
}

/// One pass of the bloom chain: the pipeline to run, its bind group, and the
/// texture it renders into. See [`LatticeCallback::run_bloom_chain`].
type BloomStep<'a> = (&'a wgpu::RenderPipeline, &'a wgpu::BindGroup, &'a wgpu::TextureView);

impl LatticeCallback {
    fn from_scene(
        scene: &Scene,
        size_points: egui::Vec2,
        target_format: wgpu::TextureFormat,
        pane_id: u64,
        gpu_ms: Option<std::sync::Arc<std::sync::atomic::AtomicU32>>,
    ) -> Self {
        let aspect = size_points.x / size_points.y.max(1.0);
        let render_scale = scene
            .render_scale
            .clamp(RENDER_SCALE_RANGE.0, RENDER_SCALE_RANGE.1);
        let camera = scene.camera;
        let view_proj = camera.view_proj(aspect);
        let (right, up) = camera.right_up();

        // Sort back-to-front along the view direction. The offscreen pass
        // does have a depth attachment, but its test is `Always` (see
        // create_scene_pipeline), so alpha blending still relies on draw
        // order — exactly as it did before the offscreen pass existed.
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
                    n.melody_level,
                    n.bass_level,
                    if n.outlined { 1.0 } else { 0.0 },
                ],
                octaves: pack_octaves(&n.octaves),
                seed: n.seed,
                cents: n.cents,
                home: if n.on_home { 1.0 } else { 0.0 },
                marks: [n.melody_slots, n.bass_slots],
                melody_color: n.melody_color.to_array(),
                bass_color: n.bass_color.to_array(),
                visited: n.trail,
            })
            .collect();

        // The grid draws under the nodes.
        let edges = scene
            .grid
            .iter()
            .map(|g| (g, if g.dashed { 2.0 } else { 1.0 }))
            .map(|(e, kind)| GpuEdge {
                a_strength: [e.a.x, e.a.y, e.a.z, e.strength],
                b_kind: [e.b.x, e.b.y, e.b.z, kind],
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
                    scene.outer_style.shader_index() as f32,
                    scene.node_style.shader_index() as f32,
                ],
                misc2: [
                    scene.darkest_pitch,
                    scene.brightest_pitch,
                    render_scale,
                    scene.bloom_strength.clamp(0.0, 4.0),
                ],
                misc3: [
                    scene.core_radius,
                    scene.outer_inner,
                    scene.outer_outer,
                    scene.outer_backdrop,
                ],
                pitch_lut: std::array::from_fn(|k| scene.pitch_lut[k].to_array()),
                node_idle: scene.node_idle.to_array(),
                misc4: [
                    scene.core_solidity,
                    scene.outer_solidity,
                    scene.idle_radius,
                    scene.idle_marker.shader_index() as f32,
                ],
                misc5: [
                    scene.grid_thickness,
                    scene.mark_unlinked,
                    scene.outer_gap,
                    scene.mark_thickness,
                ],
                misc6: [
                    scene.trail_mark.shader_index() as f32,
                    scene.trail_strength,
                    0.0,
                    0.0,
                ],
            },
            target_format,
            pane_id,
            size_points: [size_points.x, size_points.y],
            render_scale,
            gpu_ms,
        }
    }

    /// The bloom post-process, as four full-screen passes in a fixed order:
    /// bright-pass into half res, downsample to quarter, then a separable
    /// blur ping-ponging quarter A -> B (horizontal) -> A (vertical). The
    /// composite in [`CallbackTrait::paint`] samples quarter A, so the
    /// vertical blur MUST be the step that lands there.
    ///
    /// This is the only place that ordering is written down; the pipelines
    /// themselves are created independently in [`LatticeResources::new`].
    fn run_bloom_chain(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        resources: &LatticeResources,
        offscreen: &Offscreen,
    ) {
        let steps: [BloomStep; 4] = [
            (&resources.bright_pipeline, &offscreen.bright_bind_group, &offscreen.half_view),
            (
                &resources.downsample_pipeline,
                &offscreen.downsample_bind_group,
                &offscreen.quarter_a_view,
            ),
            (&resources.blur_h_pipeline, &offscreen.blur_h_bind_group, &offscreen.quarter_b_view),
            (&resources.blur_v_pipeline, &offscreen.blur_v_bind_group, &offscreen.quarter_a_view),
        ];
        for (pipeline, bind_group, target) in steps {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lattice_bloom_pass"),
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
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..4, 0..1);
        }
    }
}

/// GPU objects cached across frames in egui-wgpu's `CallbackResources`.
struct LatticeResources {
    pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    /// Bloom chain: bright pass, half->quarter downsample, blur x2.
    bright_pipeline: wgpu::RenderPipeline,
    downsample_pipeline: wgpu::RenderPipeline,
    blur_h_pipeline: wgpu::RenderPipeline,
    blur_v_pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    /// One sampled texture + the shared sampler (bloom chain passes).
    filter_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    target_format: wgpu::TextureFormat,
    panes: HashMap<u64, PaneBuffers>,
    /// GPU-side timing of the lattice passes. `None` when the device didn't
    /// grant timestamp queries — plenty of GPUs (and the offline renderer,
    /// which never asks for the feature) don't, and the readout says so
    /// rather than pretending.
    timer: Option<GpuTimer>,
    #[cfg(feature = "hot-reload")]
    watcher: ShaderWatcher,
}

/// Wall-clock time the GPU spends on one pane's lattice passes, read back
/// with timestamp queries.
///
/// Deliberately a lagging measurement. The queries resolve into a buffer that
/// must be MAPPED to be read, mapping can only be requested once the encoder
/// is submitted (egui-wgpu owns the submit, one `prepare` later), and the map
/// completes whenever the driver gets to it. Blocking on any of that would
/// stall the very pipeline being measured, and the reading would then describe
/// a frame that was slow *because* it was timed. So it runs as a three-step
/// cycle and publishes a result a few frames old. For "is the GPU the
/// bottleneck", stale and honest beats fresh and self-inflicted.
struct GpuTimer {
    set: wgpu::QuerySet,
    /// `resolve_query_set` destination. Not mappable, hence the copy.
    resolve: wgpu::Buffer,
    staging: wgpu::Buffer,
    /// Nanoseconds per timestamp tick.
    period: f32,
    state: TimerState,
    /// Set by the map callback, which the driver may run on another thread.
    ready: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[derive(PartialEq, Clone, Copy)]
enum TimerState {
    /// Nothing in flight; the next frame may record.
    Idle,
    /// Queries sit in an encoder that has not been submitted yet.
    Recorded,
    /// Submitted, and the staging buffer has been asked to map.
    Mapping,
}

/// Two timestamps, 8 bytes each.
const TIMER_BYTES: u64 = 16;

impl GpuTimer {
    /// Build the query set and buffers, or `None` when the device can't.
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        // Bracketing the passes from the ENCODER needs the inside-encoders
        // feature. The plain timestamp feature alone only permits writes at
        // pass boundaries, which would mean threading them through two
        // separate pass descriptors for the same measurement.
        let needed =
            wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        if !device.features().contains(needed) {
            return None;
        }
        Some(GpuTimer {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("lattice_gpu_timer"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lattice_gpu_timer_resolve"),
                size: TIMER_BYTES,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            staging: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("lattice_gpu_timer_staging"),
                size: TIMER_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            period: queue.get_timestamp_period(),
            state: TimerState::Idle,
            ready: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    /// Advance the readback cycle, returning a measurement in milliseconds on
    /// the frame one finally lands.
    fn poll(&mut self, device: &wgpu::Device) -> Option<f32> {
        use std::sync::atomic::Ordering;
        match self.state {
            TimerState::Idle => None,
            TimerState::Recorded => {
                // The encoder holding those queries has been submitted by now
                // (egui-wgpu submits between prepares), so the map can be
                // asked for.
                let ready = self.ready.clone();
                self.staging.slice(..).map_async(wgpu::MapMode::Read, move |result| {
                    if result.is_ok() {
                        ready.store(true, Ordering::Release);
                    }
                });
                // Poll, never Wait: a stall here would be the measurement
                // interfering with what it measures.
                let _ = device.poll(wgpu::PollType::Poll);
                self.state = TimerState::Mapping;
                None
            }
            TimerState::Mapping => {
                let _ = device.poll(wgpu::PollType::Poll);
                if !self.ready.swap(false, Ordering::Acquire) {
                    return None;
                }
                let ms = {
                    let view = self.staging.slice(..).get_mapped_range();
                    let ticks: &[u64] = bytemuck::cast_slice(&view);
                    // Saturating: both timestamps come off the same queue and
                    // should be ordered, but an out-of-order pair must not
                    // wrap into an astronomical reading.
                    let delta = ticks[1].saturating_sub(ticks[0]) as f64;
                    (delta * self.period as f64 / 1.0e6) as f32
                };
                self.staging.unmap();
                self.state = TimerState::Idle;
                Some(ms)
            }
        }
    }

    /// Whether this frame should be timed — false while a readback is still
    /// in flight, so the query set is never overwritten mid-cycle.
    fn arming(&self) -> bool {
        self.state == TimerState::Idle
    }

    fn begin(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.write_timestamp(&self.set, 0);
    }

    /// Close the bracket and stage the result for the next frame to map.
    fn end(&mut self, encoder: &mut wgpu::CommandEncoder) {
        encoder.write_timestamp(&self.set, 1);
        encoder.resolve_query_set(&self.set, 0..2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.staging, 0, TIMER_BYTES);
        self.state = TimerState::Recorded;
    }
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
    offscreen: Option<Offscreen>,
}

/// The per-pane offscreen render target and bloom chain, recreated when
/// the pane's pixel size (or render scale) changes.
///
/// The scene target uses the render-scaled size; the bloom textures use
/// fractions of the pane's NATIVE screen size, so the halo's on-screen
/// width doesn't change with the render-scale setting.
struct Offscreen {
    color_view: wgpu::TextureView,
    depth_view: wgpu::TextureView,
    /// Bloom chain targets: half res (bright pass) and two quarter-res
    /// ping-pong textures for the separable blur.
    half_view: wgpu::TextureView,
    quarter_a_view: wgpu::TextureView,
    quarter_b_view: wgpu::TextureView,
    /// Bind groups, named by the pass that USES them (source texture +
    /// shared sampler): bright samples the scene, downsample the half,
    /// blur_h quarter A, blur_v quarter B.
    bright_bind_group: wgpu::BindGroup,
    downsample_bind_group: wgpu::BindGroup,
    blur_h_bind_group: wgpu::BindGroup,
    blur_v_bind_group: wgpu::BindGroup,
    /// Composite: scene color + blurred bloom (quarter A) + uniforms.
    composite_bind_group: wgpu::BindGroup,
    size: [u32; 2],
    screen_size: [u32; 2],
}

/// The shared, pane-independent objects an [`Offscreen`] binds against.
struct OffscreenShared<'a> {
    format: wgpu::TextureFormat,
    composite_layout: &'a wgpu::BindGroupLayout,
    filter_layout: &'a wgpu::BindGroupLayout,
    sampler: &'a wgpu::Sampler,
}

impl Offscreen {
    fn new(
        device: &wgpu::Device,
        shared: &OffscreenShared<'_>,
        uniform_buffer: &wgpu::Buffer,
        size: [u32; 2],
        screen_size: [u32; 2],
    ) -> Self {
        let OffscreenShared { format, composite_layout, filter_layout, sampler } = *shared;
        let tex = |label, w: u32, h: u32, format, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let attach_and_sample =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;

        let color = tex("lattice_offscreen_color", size[0], size[1], format, attach_and_sample);
        let depth = tex(
            "lattice_offscreen_depth",
            size[0],
            size[1],
            DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let (hw, hh) = (screen_size[0].div_ceil(2).max(1), screen_size[1].div_ceil(2).max(1));
        let (qw, qh) = (screen_size[0].div_ceil(4).max(1), screen_size[1].div_ceil(4).max(1));
        let half = tex("lattice_bloom_half", hw, hh, format, attach_and_sample);
        let quarter_a = tex("lattice_bloom_quarter_a", qw, qh, format, attach_and_sample);
        let quarter_b = tex("lattice_bloom_quarter_b", qw, qh, format, attach_and_sample);

        let color_view = color.create_view(&Default::default());
        let half_view = half.create_view(&Default::default());
        let quarter_a_view = quarter_a.create_view(&Default::default());
        let quarter_b_view = quarter_b.create_view(&Default::default());

        let filter_bg = |label, source: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: filter_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            })
        };

        let composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lattice_composite_bind_group"),
            layout: composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&quarter_a_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        Offscreen {
            bright_bind_group: filter_bg("lattice_bright_bind_group", &color_view),
            downsample_bind_group: filter_bg("lattice_downsample_bind_group", &half_view),
            blur_h_bind_group: filter_bg("lattice_blur_h_bind_group", &quarter_a_view),
            blur_v_bind_group: filter_bg("lattice_blur_v_bind_group", &quarter_b_view),
            composite_bind_group,
            color_view,
            depth_view: depth.create_view(&Default::default()),
            half_view,
            quarter_a_view,
            quarter_b_view,
            size,
            screen_size,
        }
    }
}

/// Build one of the scene pipelines from WGSL source (startup uses the
/// baked-in source; hot-reload rebuilds from disk). Node and edge pipelines
/// share the module, bind group layout, blending, and topology; only entry
/// points and vertex layout differ.
///
/// `depth` is true for the production pipelines, which render into the
/// offscreen pass and must declare its depth attachment. The parity test
/// builds depthless variants to reproduce the old draw-directly-into-the-
/// egui-pass renderer as its reference.
fn create_pipeline(
    device: &wgpu::Device,
    shader_src: &str,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    entry_points: (&str, &str),
    vertex_layout: wgpu::VertexBufferLayout<'_>,
    depth: bool,
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
        // Name the pipeline after its vertex entry point, so a GPU capture
        // can tell the node and edge passes apart.
        label: Some(entry_points.0),
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
        // The depth buffer is written for future depth-reading effects but
        // never rejects a fragment (`Always`): translucent glows composite
        // by draw order, exactly as they did directly in the egui pass.
        depth_stencil: depth.then(|| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Build both scene pipelines from one source.
fn create_pipelines(
    device: &wgpu::Device,
    shader_src: &str,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    depth: bool,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    (
        create_pipeline(
            device,
            shader_src,
            target_format,
            bind_group_layout,
            ("vs_main", "fs_main"),
            GpuInstance::LAYOUT,
            depth,
        ),
        create_pipeline(
            device,
            shader_src,
            target_format,
            bind_group_layout,
            ("vs_edge", "fs_edge"),
            GpuEdge::LAYOUT,
            depth,
        ),
    )
}

/// One post-process pipeline over the blit.wgsl module: a fullscreen quad
/// with the given fragment entry point. The composite (into the egui
/// pass) blends premultiplied; the bloom-chain passes overwrite their
/// whole target and pass `blend: None`.
fn create_post_pipeline(
    device: &wgpu::Device,
    entry_point: &str,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("lattice_blit_shader"),
        source: wgpu::ShaderSource::Wgsl(BLIT_SRC.into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lattice_post_pipeline_layout"),
        bind_group_layouts: &[Some(bind_group_layout)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry_point),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_blit"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(entry_point),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend,
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
}

impl LatticeResources {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
    ) -> Self {
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
            create_pipelines(device, SHADER_SRC, target_format, &bind_group_layout, true);

        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let filter_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lattice_filter_bind_group_layout"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lattice_composite_bind_group_layout"),
            entries: &[
                texture_entry(0),
                sampler_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let composite_pipeline = create_post_pipeline(
            device,
            "fs_composite",
            target_format,
            &composite_layout,
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        );
        let filter = |entry| create_post_pipeline(device, entry, target_format, &filter_layout, None);
        let bright_pipeline = filter("fs_bright");
        let downsample_pipeline = filter("fs_blit");
        let blur_h_pipeline = filter("fs_blur_h");
        let blur_v_pipeline = filter("fs_blur_v");
        // Linear filtering: identity when render scale is 1 (texel-aligned
        // sampling), smooth resampling at any other scale.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lattice_composite_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        LatticeResources {
            pipeline,
            edge_pipeline,
            composite_pipeline,
            bright_pipeline,
            downsample_pipeline,
            blur_h_pipeline,
            blur_v_pipeline,
            bind_group_layout,
            composite_layout,
            filter_layout,
            sampler,
            target_format,
            panes: HashMap::new(),
            timer: GpuTimer::new(device, queue),
            #[cfg(feature = "hot-reload")]
            watcher: ShaderWatcher::new(),
        }
    }

    /// Fetch (or create) a pane's GPU objects, and when `offscreen_size` is
    /// given, make sure its offscreen target exists at exactly that pixel
    /// size (pane resizes and render-scale changes recreate it).
    /// `screen_size` is the pane's native (unscaled) pixel size, which
    /// sizes the bloom chain.
    fn pane_buffers(
        &mut self,
        device: &wgpu::Device,
        pane_id: u64,
        offscreen_size: Option<[u32; 2]>,
        screen_size: [u32; 2],
    ) -> &mut PaneBuffers {
        let layout = &self.bind_group_layout;
        let shared = OffscreenShared {
            format: self.target_format,
            composite_layout: &self.composite_layout,
            filter_layout: &self.filter_layout,
            sampler: &self.sampler,
        };
        let pane = self.panes.entry(pane_id).or_insert_with(|| {
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
                instance_buffer: create_vertex_buffer::<GpuInstance>(
                    device,
                    "lattice_instances",
                    INITIAL_INSTANCE_CAPACITY,
                ),
                instance_capacity: INITIAL_INSTANCE_CAPACITY,
                instance_count: 0,
                edge_buffer: create_vertex_buffer::<GpuEdge>(
                    device,
                    "lattice_edges",
                    INITIAL_EDGE_CAPACITY,
                ),
                edge_capacity: INITIAL_EDGE_CAPACITY,
                edge_count: 0,
                offscreen: None,
            }
        });
        if let Some(size) = offscreen_size {
            if pane
                .offscreen
                .as_ref()
                .is_none_or(|o| o.size != size || o.screen_size != screen_size)
            {
                pane.offscreen =
                    Some(Offscreen::new(device, &shared, &pane.uniform_buffer, size, screen_size));
            }
        }
        pane
    }
}

/// Starting element counts for a pane's per-instance and per-edge buffers;
/// both grow by `next_power_of_two` when a frame overflows them.
const INITIAL_INSTANCE_CAPACITY: usize = 256;
const INITIAL_EDGE_CAPACITY: usize = 64;

/// A `capacity`-element vertex buffer (VERTEX | COPY_DST) sized for `T`.
/// Used for both the instance and edge buffers, which differ only in label
/// and element type.
fn create_vertex_buffer<T>(device: &wgpu::Device, label: &str, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: (capacity * std::mem::size_of::<T>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl CallbackTrait for LatticeCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        // Lazily (re)create shared resources. Recreate if the target format
        // changed (it can't today, but this keeps the invariant explicit).
        let recreate = callback_resources
            .get::<LatticeResources>()
            .is_none_or(|r| r.target_format != self.target_format);
        if recreate {
            callback_resources.insert(LatticeResources::new(device, queue, self.target_format));
        }
        let resources: &mut LatticeResources = callback_resources
            .get_mut()
            .expect("inserted above when missing");

        // Advance the GPU timer's readback cycle first: a result that landed
        // is published now, and the cycle returns to Idle so this frame can be
        // the next one sampled.
        if let Some(timer) = resources.timer.as_mut() {
            if let (Some(ms), Some(out)) = (timer.poll(device), &self.gpu_ms) {
                out.store(ms.to_bits(), std::sync::atomic::Ordering::Relaxed);
            }
        }

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
                        true,
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

        // Offscreen pixel size: the callback rect at native resolution,
        // scaled by the render-scale view setting (clamped in from_scene).
        // The unscaled screen size drives the bloom chain.
        let max_dim = device.limits().max_texture_dimension_2d;
        let px_size = |scale: f32| {
            let px = screen_descriptor.pixels_per_point * scale;
            [
                ((self.size_points[0] * px).round() as u32).clamp(1, max_dim),
                ((self.size_points[1] * px).round() as u32).clamp(1, max_dim),
            ]
        };
        let size = px_size(self.render_scale);
        let screen_size = px_size(1.0);
        // Nothing to draw (matches paint()'s early-out): skip the offscreen
        // target and pass entirely.
        let offscreen_size = (!self.instances.is_empty()).then_some(size);

        let pane = resources.pane_buffers(device, self.pane_id, offscreen_size, screen_size);

        if self.instances.len() > pane.instance_capacity {
            pane.instance_capacity = self.instances.len().next_power_of_two();
            pane.instance_buffer = create_vertex_buffer::<GpuInstance>(
                device,
                "lattice_instances",
                pane.instance_capacity,
            );
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
            pane.edge_buffer =
                create_vertex_buffer::<GpuEdge>(device, "lattice_edges", pane.edge_capacity);
        }
        pane.edge_count = self.edges.len() as u32;
        if !self.edges.is_empty() {
            queue.write_buffer(&pane.edge_buffer, 0, bytemuck::cast_slice(&self.edges));
        }

        queue.write_buffer(&pane.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));

        // The scene pass: draw into the pane's offscreen target, on the
        // encoder egui-wgpu executes before its own render pass. paint()
        // then just composites the finished texture.
        let pane = resources
            .panes
            .get(&self.pane_id)
            .expect("created by pane_buffers above");
        if let Some(offscreen) = pane.offscreen.as_ref().filter(|_| pane.instance_count > 0) {
            // Bracket the scene pass and the bloom chain together: what the
            // overlay wants is the cost of drawing THE LATTICE, which is both.
            // Skipped while a readback is still in flight, so the query set is
            // never overwritten mid-cycle.
            let timing = resources.timer.as_ref().is_some_and(GpuTimer::arming);
            if timing {
                if let Some(timer) = resources.timer.as_ref() {
                    timer.begin(egui_encoder);
                }
            }
            let mut pass = egui_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lattice_scene_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &offscreen.color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Transparent black: premultiplied "nothing", so
                        // compositing over the pane background reproduces
                        // drawing straight into the egui pass.
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &offscreen.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // Edges draw under the nodes so discs own the joints.
            if pane.edge_count > 0 {
                pass.set_pipeline(&resources.edge_pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_vertex_buffer(0, pane.edge_buffer.slice(..));
                pass.draw(0..4, 0..pane.edge_count);
            }
            pass.set_pipeline(&resources.pipeline);
            pass.set_bind_group(0, &pane.bind_group, &[]);
            pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
            pass.draw(0..4, 0..pane.instance_count);
            drop(pass);

            // Skipped entirely at strength 0: the composite multiplies the
            // never-written quarter texture by 0, and fresh wgpu textures
            // read as zero anyway.
            if self.uniforms.misc2[3] > 0.0 {
                self.run_bloom_chain(egui_encoder, resources, offscreen);
            }

            if timing {
                if let Some(timer) = resources.timer.as_mut() {
                    timer.end(egui_encoder);
                }
            }
        }

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
        let Some(offscreen) = &pane.offscreen else {
            return;
        };

        // The scene was rendered in prepare(); stretch it over the
        // viewport (egui-wgpu sets the viewport to the callback rect).
        render_pass.set_pipeline(&resources.composite_pipeline);
        render_pass.set_bind_group(0, &offscreen.composite_bind_group, &[]);
        render_pass.draw(0..4, 0..1);
    }
}

#[cfg(test)]
mod tests;
