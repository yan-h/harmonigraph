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
//! exactly as it would without the offscreen pass) but not yet read by
//! anything. `offscreen_composite_matches_direct_draw` in the tests pins
//! down that this path matches drawing straight into the egui pass.
//!
//! With the `hot-reload` feature (enabled by the standalone harness), the
//! .wgsl file is watched on disk and the pipeline rebuilds on save —
//! validated first, so a broken edit logs an error and keeps the old
//! pipeline instead of crashing. Release plugin builds keep `include_str!`
//! only.

use std::collections::HashMap;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use harmonigraph_scene::Scene;

/// The piano roll's own callback — a different picture with the same
/// problem, solved the same way. It shares this crate's wgpu version and
/// buffer helpers and nothing else; the lattice's offscreen target, depth
/// buffer and bloom chain are all beside the point for a flat ribbon.
mod roll;
pub use roll::{roll_paint_callback, RollAxes, RollInstance};

/// Label text, for the same reason the roll has its own callback: what a
/// label costs is the rim, and the rim was the text drawn again once per
/// stamp.
mod text;
pub use text::{text_paint_callback, FontAtlas, GlyphInstance, TextRing};

// Shells name texture formats through this re-export so every crate agrees
// on the wgpu version.
pub use egui_wgpu::wgpu;

const SHADER_SRC: &str = include_str!("shaders/lattice.wgsl");
const BLIT_SRC: &str = include_str!("shaders/blit.wgsl");

/// Depth format of the offscreen pass. The scene pipelines test `Always`, so
/// it never rejects a fragment of the lattice itself; what reads it is the
/// LABEL pass, which asks it what covers a name (see [`text`]).
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
    /// sanitized by the scene so z > y); w unused (it carried the outer
    /// backdrop opacity, now fixed on in the shader — a retired slot rather
    /// than a repack, which would renumber the ones around it for nothing).
    misc3: [f32; 4],
    /// Pitch->color lookup for the dots octave style (see harmonigraph_scene's
    /// `pitch_ramp_lut`), matching the node disc gradient.
    pitch_lut: [[f32; 4]; harmonigraph_scene::PITCH_LUT_N],
    /// Idle node color (the view's grid color at full alpha, so the grid
    /// lines and idle markers read as one layer): the home-sheet
    /// placeholder ring is drawn in this ONE constant color, so a
    /// releasing note's ring never shows the note's own color or snaps
    /// color when the voice is pruned.
    node_idle: [f32; 4],
    /// x: core solidity (0 = soft glow, 1 = solid orb), the single axis the
    /// core layer runs on; y unused (it carried the outer solidity, now
    /// fixed crisp in the shader — retired in place, like `misc3.w`);
    /// z: idle marker radius; w: idle marker style (0 none, 1 dot, 2
    /// circle). (The blit pipeline binds only the head of this buffer, so
    /// trailing fields are safe to add here.)
    misc4: [f32; 4],
    /// x: grid line thickness as a multiple of the shader's built-in grid
    /// width; y: draw the melody/bass mark on the core (pitch class
    /// indicator); z: draw it on the octave glyphs; w unused. Every
    /// earlier misc slot is spoken for, so the grid's knob starts a new
    /// one — safe per the note on `misc4`.
    misc5: [f32; 4],
    /// x: trail mark style (0 off, 1 lift, 2 ring, 3 tint); y: trail
    /// strength 0..1 — both feed the idle-marker branch alone (see
    /// `TrailMark`); misc5 was full, so the trail started its own slot.
    /// z: the sevens knockout's fade width, in the uv of a full-size node
    /// (`Scene::sevens_soft`); w unused.
    misc6: [f32; 4],
    /// The ground the lattice is drawn onto — the pane fill this pass gets
    /// composited over — as the sevens knockout's target color. Without it
    /// the gutter can only knock out to black, which on this skin is
    /// several shades DARKER than the pane and so reads as a blob sitting
    /// on the picture rather than as a hole through it. See
    /// `Scene::background`.
    background: [f32; 4],
    /// The wheel's pitch axis. x: octaves one turn is cut into
    /// (`OctaveLayout::span`); y: the MIDI pitch at the top of every node
    /// (`OctaveLayout::center`); z, w unused.
    ///
    /// No slot range and no per-node angle: both depend on the node's own
    /// pitch class — which of its octaves are the ones nearest the center, and
    /// how far its ring is turned to put them on their pitches — so the shader
    /// derives them per node from these two.
    misc7: [f32; 4],
    /// `OctaveLayout::bounds` — the angle from a ring's seam to each of its
    /// slice boundaries, the same table for every node — four to a row, which
    /// is how a uniform array is laid out anyway.
    oct_bounds: [[f32; 4]; 3],
}

// Two fixed-size GPU homes, one per table, and both are exact rather than
// one-sided ceilings — the uploads below fill every entry unconditionally, so
// a SMALLER constant in harmonigraph-scene reads off the end of a shorter
// array and panics on the first frame, which a ceiling would wave through.
//
// 3 u32 words hold 12 packed levels, one byte per octave slot.
const _: () = assert!(harmonigraph_scene::OCTAVE_SLOTS == 11);
// `oct_bounds`'s 3 vec4s hold 12 boundary angles, and the layout needs one per
// slice plus the closing one — so a span of 11 is the ceiling, which is also
// every MIDI octave there is. Raising MAX_SPAN in harmonigraph-scene is what
// would break this.
const _: () = assert!(harmonigraph_scene::MAX_SPAN as usize + 1 == 12);

// The shader declares `pitch_lut` with a literal length; keep the two in
// lockstep so the uniform buffer and the WGSL agree.
const _: () = assert!(harmonigraph_scene::PITCH_LUT_N == 64);

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
    /// The node's pitch class in cents (0..1200). It both PLACES the octave
    /// indicators and COLORS them, off the one quantity: an indicator's
    /// octave has a pitch, that octave's C plus this, and the indicator sits
    /// at that pitch's angle on the shared axis and in that pitch's color
    /// (see `harmonigraph_scene::octaves`).
    cents: f32,
    /// 1 when the node is on the home (center sevens) sheet: idle home
    /// nodes draw a blank placeholder ring.
    home: f32,
    /// Melody/bass marks: `[melody_slots, bass_slots]`, one bit per octave
    /// slot (see `NodeInstance::melody_slots`). Kept as integers rather
    /// than folded into the dead `params.y`/`params.z` floats because the
    /// shader masks them bitwise, which needs a flat-interpolated `u32`.
    marks: [u32; 2],
    /// Each mark's own color (see `NodeInstance::melody_color`): the sector it
    /// links back to, not a fixed livery, so a ring reads as belonging to the
    /// indicator it points at.
    melody_color: [f32; 4],
    bass_color: [f32; 4],
    /// How strongly the music is remembered at this node, 0..1 (see
    /// `NodeInstance::trail`). Reaches only the shader's idle-marker
    /// branch — a memory must never read as a sounding note.
    visited: f32,
    /// The sevens layer, packed: x = billboard size factor (1 on the home
    /// sheet), y = knockout gutter width in uv units (0 on the home sheet).
    /// See `NodeInstance::scale` / `::gutter`.
    sevens: [f32; 2],
}

impl GpuInstance {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GpuInstance>() as u64,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x4, 2 => Float32x4, 3 => Uint32x3, 4 => Float32,
            5 => Float32, 6 => Float32, 7 => Uint32x2,
            8 => Float32x4, 9 => Float32x4, 10 => Float32, 11 => Float32x2
        ],
    };
}

/// Pack the per-octave activation levels into the bit layout
/// `octave_level()` in lattice.wgsl unpacks: 8 bits per slot,
/// little-endian (slot 0 = lowest byte of the first word).
fn pack_octaves(levels: &[f32; harmonigraph_scene::OCTAVE_SLOTS]) -> [u32; 3] {
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
/// Where the lattice callback publishes what it measures about itself.
///
/// An atomic bag rather than return values because none of this comes back up
/// the call stack that asked for it: `prepare` runs inside egui-wgpu, and the
/// GPU timing arrives several frames after the frame it describes. All three
/// are f32 bits.
#[derive(Default)]
pub struct LatticeStats {
    /// GPU time of the lattice's passes, carrying the
    /// [`GPU_TIME_UNSUPPORTED`] / [`GPU_TIME_PENDING`] sentinels.
    pub gpu_ms: std::sync::atomic::AtomicU32,
    /// Wall time of the whole `prepare` callback. egui-wgpu runs this from
    /// inside `update_buffers`, so it is billed to the frame's upload stage
    /// and is invisible from outside.
    ///
    /// "Prepare" undersells it, and the three fields below exist because the
    /// name misled for a long time: this callback does not merely stage data.
    /// It also ENCODES the lattice's whole scene pass and the four-pass bloom
    /// chain onto egui's encoder — the largest piece of CPU work in the
    /// frame, sitting inside a row the overlay calls "buf up".
    pub prepare_ms: std::sync::atomic::AtomicU32,
    /// Of that, the time in `device.poll` draining the timestamp readback:
    /// what the GPU measurement costs to take. Kept separate so the
    /// instrumentation can be caught spending the budget it exists to
    /// measure.
    pub poll_ms: std::sync::atomic::AtomicU32,
    /// Of that, staging this frame's data: sizing the offscreen targets,
    /// recreating them when the size moved, and the three `queue.write_buffer`
    /// calls for instances, edges and uniforms.
    pub write_ms: std::sync::atomic::AtomicU32,
    /// Of that, encoding the scene pass and the bloom chain — five
    /// `begin_render_pass` calls and the draws inside them. No GPU work
    /// happens here; this is the cost of BUILDING the command stream.
    ///
    /// Split from `write_ms` because the two answer different questions and
    /// move for different reasons. A cost that tracks the node count is
    /// staging; a cost that does not is the encoder, and the fixes have
    /// nothing in common.
    pub scene_ms: std::sync::atomic::AtomicU32,
}

/// `stats` receives this pane's own measurements. Pass `None` for panes whose
/// cost isn't the one being reported, so a second lattice on screen can't
/// overwrite the readings.
pub fn lattice_paint_callback(
    rect: egui::Rect,
    scene: &Scene,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
    stats: Option<std::sync::Arc<LatticeStats>>,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        LatticeCallback::from_scene(scene, rect.size(), target_format, pane_id, stats),
    )
}

/// Per-frame, per-pane draw data, computed on the UI thread.
struct LatticeCallback {
    instances: Vec<GpuInstance>,
    /// Index into `instances` where the grid is drawn (see `from_scene`).
    grid_at: u32,
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
    /// Where to publish this pane's own measurements.
    stats: Option<std::sync::Arc<LatticeStats>>,
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
        stats: Option<std::sync::Arc<LatticeStats>>,
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
        //
        // Sheets back to front FIRST, then painter's order within a sheet.
        // That is still just back-to-front — world z IS the sevens axis, and
        // the first key is only its depth — but it stays EXACT when the
        // camera is orbited, where two nodes on one sheet have different
        // depths and a plain depth sort interleaves the sheets. Interleaving
        // is not a cosmetic problem: it puts the grid in the wrong place in
        // the order and leaves the home sheet's clearings with almost
        // nothing drawn before them to clear, so the knockout quietly did
        // nothing under perspective and orthographic while working under
        // cabinet (where every home node shares one depth and the two sorts
        // agree).
        //
        // Do not reorder the sheets on top of this. Forcing the home sheet
        // to the bottom (so off-sheet notes could never be hidden by it)
        // inverts the far half of the axis: the sheet BEHIND home then draws
        // last, and its clearing takes a bite out of the home sheet in front
        // of it. Grouping by distance from home does the same thing more
        // thoroughly. Depth is what the reader is being shown; it is what
        // the order has to follow.
        //
        // The `forward.z` factor is what keeps it honest if the view is
        // orbited right around past the sheets: which way along z is "away"
        // is the camera's business, not an assumption.
        let eye = camera.eye();
        let forward = (camera.target - eye).normalize_or_zero();
        let sheet_depth = |n: &harmonigraph_scene::NodeInstance| n.world_pos.z * forward.z;
        let mut order: Vec<(f32, f32, &harmonigraph_scene::NodeInstance)> = scene
            .nodes
            .iter()
            .map(|n| (sheet_depth(n), (n.world_pos - eye).dot(forward), n))
            .collect();
        order.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.total_cmp(&a.1)));

        // The grid belongs to the home sheet — that is the only sheet that
        // draws one — so its place in the order is between the sheets behind
        // it and the home sheet itself. Under it, a node on a sheet BEHIND
        // the home one punches its clearing through the home grid, putting
        // a hole in the layer that is supposed to be hiding it.
        //
        // The home sheet then draws after the grid, so a home node's
        // clearing cuts the grid lines as well as the sheets behind — the
        // node sits in a clean gap in the lattice rather than on top of it.
        // (Drawing the home clearings in a pass of their own ahead of the
        // grid would spare the lines; the lines are wanted cut.)
        //
        // World z is measured from the home sheet, so its whole run sits at
        // sheet depth 0 — behind it is positive, in front negative. Sorting
        // by that above is what makes the home sheet one contiguous run,
        // under every projection rather than only the face-on one.
        let to_gpu = |n: &harmonigraph_scene::NodeInstance, gutter: f32| GpuInstance {
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
                sevens: [n.scale, gutter],
        };

        let split = order
            .iter()
            .position(|&(plane, _, _)| plane <= 0.0)
            .unwrap_or(order.len());
        // A node that can paint nothing is not shipped at all. The shader
        // already discards it per fragment, but the billboard is deliberately
        // bigger than the node (QUAD_MARGIN and then some), so the discard is
        // paid a fragment at a time over a quad the disc never reaches — and
        // an unplayed lattice is almost entirely such nodes. With the default
        // idle marker (None) and trails off, ALL of them are: a still lattice
        // then ships its grid and nothing else.
        //
        // The gates are the ones `fs_main`'s idle branch and `idle_marker`
        // read, in the same order, off the packed instance rather than the
        // scene node — so this asks the question the shader answers, not a
        // restatement of it that could drift. Reading the PACKED octave word
        // is what makes that exact rather than close: an octave level under
        // half a byte quantizes to zero on the way to the GPU, and a node
        // dropped for that is a node the shader would have discarded anyway.
        // The two settings it does NOT read — the idle radius, and the trail
        // ring's own constants — can only make a kept node paint less, never
        // make a dropped one paint, so they err toward keeping a quad.
        let trail_level = |visited: f32| match scene.trail_mark {
            harmonigraph_scene::TrailMark::Off => 0.0,
            _ => visited.clamp(0.0, 1.0) * scene.trail_strength.clamp(0.0, 1.0),
        };
        let paints = |g: &GpuInstance| {
            if g.params[0] > 0.0
                || g.params[1] > 0.0
                || g.params[2] > 0.0
                || (g.octaves[0] | g.octaves[1] | g.octaves[2]) != 0
            {
                return true;
            }
            let trail = trail_level(g.visited);
            // The idle marker needs a style, and a home sheet or a memory to
            // show on; the trail's pale ring draws with the marker off.
            let marked = scene.idle_marker != harmonigraph_scene::IdleMarker::None
                && (g.home >= 0.5 || trail > 0.0);
            marked || (scene.trail_mark == harmonigraph_scene::TrailMark::Ring && trail > 0.0)
        };
        let drawn = |out: &mut Vec<GpuInstance>,
                     ns: &[(f32, f32, &harmonigraph_scene::NodeInstance)]| {
            out.extend(ns.iter().map(|(_, _, n)| to_gpu(n, n.gutter)).filter(&paints));
        };
        let mut instances = Vec::with_capacity(order.len());
        drawn(&mut instances, &order[..split]);
        // Where the grid is drawn inside that run: after the sheets BEHIND the
        // home one, counted over the kept instances rather than over `split`,
        // which indexes the list before the cull.
        let grid_at = instances.len() as u32;
        drawn(&mut instances, &order[split..]);

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
            grid_at,
            edges,
            uniforms: Uniforms {
                view_proj: view_proj.to_cols_array(),
                cam_right: right.extend(0.0).to_array(),
                cam_up: up.extend(0.0).to_array(),
                misc: [
                    scene.time,
                    scene.node_radius,
                    0.0,
                    scene.node_style.shader_index() as f32,
                ],
                misc2: [
                    scene.darkest_pitch,
                    scene.brightest_pitch,
                    render_scale,
                    scene.bloom_strength.clamp(0.0, 4.0),
                ],
                misc3: [scene.core_radius, scene.outer_inner, scene.outer_outer, 0.0],
                pitch_lut: std::array::from_fn(|k| scene.pitch_lut[k].to_array()),
                node_idle: scene.node_idle.to_array(),
                misc4: [
                    scene.core_solidity,
                    0.0,
                    scene.idle_radius,
                    scene.idle_marker.shader_index() as f32,
                ],
                misc5: [scene.grid_thickness, 0.0, scene.outer_gap, scene.mark_thickness],
                misc6: [
                    scene.trail_mark.shader_index() as f32,
                    scene.trail_strength,
                    scene.sevens_soft,
                    0.0,
                ],
                background: scene.background.to_array(),
                misc7: [
                    scene.octave_layout.span as f32,
                    scene.octave_layout.center,
                    0.0,
                    0.0,
                ],
                // Straight indexing: the table is exactly as long as the
                // rows are wide (the const assert above is what keeps it so),
                // and a fallback here would quietly ship a wheel with a wrong
                // angle in it rather than failing the build.
                oct_bounds: std::array::from_fn(|row| {
                    std::array::from_fn(|col| scene.octave_layout.bounds[row * 4 + col])
                }),
            },
            target_format,
            pane_id,
            size_points: [size_points.x, size_points.y],
            render_scale,
            stats,
        }
    }

    /// Whether this callback may drive the shared [`GpuTimer`] — true only for
    /// the one carrying a stats sink, i.e. the pane that publishes the reading.
    ///
    /// There is ONE timer per device, and its three-step readback cycle assumes
    /// each step lands in a different frame — specifically that the encoder
    /// holding `close`'s `copy_buffer_to_buffer` has been submitted before the
    /// next `poll` asks the staging buffer to map. egui-wgpu submits once per
    /// frame, AFTER running every callback's `prepare` on one shared encoder,
    /// so that only holds while a single callback drives the cycle.
    ///
    /// Two do exist: the Video tab's preview is a second live lattice, and the
    /// frame it first appears in ran `prepare` twice — the docked pane
    /// recording the copy, then the preview immediately calling `map_async` on
    /// the buffer that copy still had to write. Submitting that encoder is a
    /// wgpu validation error ("Buffer with 'lattice_gpu_timer_staging' label is
    /// still mapped"), which is fatal by default and took the plugin down with
    /// it — reproducibly, the moment the preview came into view.
    ///
    /// Gating on the stats sink is also what the reading MEANS: the overlay
    /// reports the cost of the docked lattice, and letting the preview consume
    /// the cycle would have published the preview's frame time under the
    /// docked pane's name.
    fn drives_timer(&self) -> bool {
        self.stats.is_some()
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
    /// 1x1 target for the trailing pass that carries the closing sample.
    /// One pixel, so beginning it costs nothing worth measuring.
    tail: wgpu::TextureView,
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

/// Published in place of a measurement when the device can't do timestamp
/// queries at all.
///
/// A NaN bit pattern, as is [`GPU_TIME_PENDING`]. Zero would have been the
/// obvious sentinel and is the wrong choice: a real reading of 0.0 ms is
/// perfectly possible, and using it to mean "nothing yet" is what made a
/// landed-but-zero measurement indistinguishable from a stuck one.
pub const GPU_TIME_UNSUPPORTED: u32 = 0x7fc0_0001;

/// The initial value: a timer exists, but no measurement has come back yet.
pub const GPU_TIME_PENDING: u32 = 0x7fc0_0002;

impl GpuTimer {
    /// Build the query set and buffers, or `None` when the device can't.
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
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
            tail: device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("lattice_gpu_timer_tail"),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default()),
        })
    }

    /// Advance the readback cycle, returning a measurement in milliseconds on
    /// the frame one finally lands.
    fn poll(&mut self, device: &wgpu::Device) -> Option<f32> {
        use std::sync::atomic::Ordering;
        match self.state {
            TimerState::Idle => None,
            TimerState::Recorded => {
                // The encoder holding those queries has been submitted by now,
                // so the map can be asked for. That is true because egui-wgpu
                // submits once per frame and only ONE callback per frame gets
                // here — see `LatticeCallback::drives_timer`, which is what
                // keeps a second lattice view from mapping this buffer between
                // the copy being recorded and the submit that performs it.
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

    /// The opening sample, to hang on the first lattice pass.
    ///
    /// Both samples are BEGINNING-of-pass writes. The obvious shape —
    /// `write_timestamp` on the encoder, or beginning-and-end on one pass —
    /// does not work here: Metal advertises and grants both
    /// `TIMESTAMP_QUERY_INSIDE_ENCODERS` and end-of-pass writes, then
    /// silently records ZERO for them. Only the beginning-of-pass sample
    /// comes back with a real value, so the bracket is built from two of
    /// those, the closing one on a pass that exists only to carry it.
    fn opening(&self) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.set,
            beginning_of_pass_write_index: Some(0),
            end_of_pass_write_index: None,
        })
    }

    /// Close the bracket with a 1x1 no-op pass, and stage the result for a
    /// later frame to map.
    fn close(&mut self, encoder: &mut wgpu::CommandEncoder) {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lattice_gpu_timer_tail_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.tail,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                query_set: &self.set,
                beginning_of_pass_write_index: Some(1),
                end_of_pass_write_index: None,
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
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
    /// Where the grid is drawn inside the node run: instances before this
    /// are the sheets behind the home one plus the home sheet's own
    /// clearings, and must land under the grid; the rest go over it. See
    /// `LatticeCallback::from_scene`.
    grid_at: u32,
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
    /// Names the TEXTURES behind these views, so a pane that binds the depth
    /// buffer elsewhere can tell that a resize has replaced it. A size pair
    /// nearly says the same thing and is the wrong thing to say it with: a
    /// pane can be recreated at the size it already had, and a stale bind
    /// group then names a destroyed texture.
    epoch: u64,
}

/// Hands out [`Offscreen::epoch`]. Global rather than per-[`LatticeResources`]
/// so it can be taken inside `Offscreen::new` without a second borrow of the
/// resources the shared layouts are being read out of.
static OFFSCREEN_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
        // Sampled as well as attached: the label pass reads it to find out
        // what the lattice drew in front of a name.
        let depth =
            tex("lattice_offscreen_depth", size[0], size[1], DEPTH_FORMAT, attach_and_sample);
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
            epoch: OFFSCREEN_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        }
    }
}

/// The depth buffer a pane's lattice wrote THIS frame, and the epoch naming
/// the texture behind it.
///
/// What the label pass needs to hide a name behind a nearer node, and the
/// reason [`Offscreen`]'s depth attachment is sampleable. `None` where there
/// is nothing to hide behind: no lattice on that pane, no offscreen target
/// yet, or a frame where the lattice shipped neither a node nor an edge and
/// so never began the pass that fills it. That last case is the one worth
/// naming — the texture still holds whatever the last frame drew, and a
/// silent lattice would otherwise go on cutting labels out of a picture that
/// is no longer there.
///
/// Ordering is egui-wgpu's: every callback's `prepare` runs before any
/// `paint`, and the panes add their lattice callback before the labels they
/// draw over it, so the depth read here is this frame's.
pub(crate) fn lattice_occluder(
    callback_resources: &CallbackResources,
    pane_id: u64,
) -> Option<(wgpu::TextureView, u64)> {
    let pane = callback_resources
        .get::<LatticeResources>()?
        .panes
        .get(&pane_id)?;
    if pane.instance_count == 0 && pane.edge_count == 0 {
        return None;
    }
    let offscreen = pane.offscreen.as_ref()?;
    Some((offscreen.depth_view.clone(), offscreen.epoch))
}

/// Build one of the scene pipelines from WGSL source (startup uses the
/// baked-in source; hot-reload rebuilds from disk). Node and edge pipelines
/// share the module, bind group layout, blending, and topology; only entry
/// points and vertex layout differ.
///
/// `depth` says what this pipeline does with the offscreen pass's depth
/// attachment; see [`Depth`].
fn create_pipeline(
    device: &wgpu::Device,
    shader_src: &str,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    entry_points: (&str, &str),
    vertex_layout: wgpu::VertexBufferLayout<'_>,
    depth: Depth,
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
        // The depth buffer never rejects a fragment of the lattice itself
        // (`Always`): translucent glows composite by draw order, exactly as
        // they did directly in the egui pass. What `Always` plus a write
        // leaves behind is the depth of whatever was drawn LAST at each
        // pixel, which under this pass's back-to-front order is the thing
        // that covers — and that is precisely the question the label pass
        // asks it.
        depth_stencil: (depth != Depth::None).then(|| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(depth == Depth::Write),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// What a scene pipeline does with the offscreen pass's depth attachment.
///
/// A pipeline that writes is one whose fragments can hide a label, so this
/// is a statement about the LABELS rather than about the lattice: nothing
/// here changes a pixel of the lattice, whose fragments composite by draw
/// order under `Always` whatever this says.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Depth {
    /// No attachment at all. The parity test builds depthless variants that
    /// draw straight into the egui pass, as its reference.
    None,
    /// Declares the attachment and writes to it, so these fragments cover a
    /// label behind them.
    Write,
    /// Declares the attachment and leaves it alone: drawn, but never in the
    /// way of a name.
    Keep,
}

/// Build both scene pipelines from one source.
///
/// The nodes write depth and the GRID does not, which is the whole of what
/// decides that a name can be covered by a node in front of it and never by
/// a lattice line. Two reasons, and the second is the load-bearing one:
///
///   - a hairline through a note name reads as a rendering fault, where a
///     disc over it reads as depth;
///   - the depth test is binary and the grid is faint. Depth carries no
///     alpha, so a line at a few percent opacity would take a fully opaque
///     bite out of a label — a hole where the picture shows almost nothing.
///
/// The same arithmetic is why the node's own fade is a known limit rather
/// than a settled question: a note released almost to nothing goes on
/// cutting a name behind it until the scene culls it. That needs the
/// COVERAGE of the frontmost fragment, which is a second attachment, not a
/// threshold here.
fn create_pipelines(
    device: &wgpu::Device,
    shader_src: &str,
    target_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
    depth: bool,
) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
    let (nodes, edges) = if depth {
        (Depth::Write, Depth::Keep)
    } else {
        (Depth::None, Depth::None)
    };
    (
        create_pipeline(
            device,
            shader_src,
            target_format,
            bind_group_layout,
            ("vs_main", "fs_main"),
            GpuInstance::LAYOUT,
            nodes,
        ),
        create_pipeline(
            device,
            shader_src,
            target_format,
            bind_group_layout,
            ("vs_edge", "fs_edge"),
            GpuEdge::LAYOUT,
            edges,
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
                grid_at: 0,
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
        //
        // ONLY the callback carrying a stats sink touches the timer — see
        // `drives_timer`.
        let prepare_start = std::time::Instant::now();
        let poll_start = std::time::Instant::now();
        match (resources.timer.as_mut(), &self.stats) {
            (Some(timer), Some(out)) => {
                if let Some(ms) = timer.poll(device) {
                    out.gpu_ms.store(ms.to_bits(), std::sync::atomic::Ordering::Relaxed);
                }
            }
            // No timer at all: the device refused the feature. Say so, rather
            // than leaving the readout on the same "nothing yet" it shows
            // while a first measurement is still in flight — those are very
            // different answers and the overlay could not tell them apart.
            (None, Some(out)) => {
                out.gpu_ms.store(GPU_TIME_UNSUPPORTED, std::sync::atomic::Ordering::Relaxed);
            }
            (_, None) => {}
        }
        let poll_ms = poll_start.elapsed().as_secs_f32() * 1000.0;

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
                    eprintln!("[harmonigraph-render] shader hot-reloaded");
                }
                Err(err) => {
                    eprintln!("[harmonigraph-render] shader reload REJECTED, keeping old pipeline:\n{err}");
                }
            }
        }

        // Offscreen pixel size: the callback rect at native resolution,
        // scaled by the render-scale view setting (clamped in from_scene).
        // The unscaled screen size drives the bloom chain.
        let write_start = std::time::Instant::now();
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
        // target and pass entirely. The EDGES count as much as the nodes —
        // `from_scene` drops nodes that can paint nothing, so a still lattice
        // with the idle marker off is exactly a frame of grid and no
        // instances, and keying this on the instances alone would take the
        // grid down with them.
        let anything = !self.instances.is_empty() || !self.edges.is_empty();
        let offscreen_size = anything.then_some(size);

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
        pane.grid_at = self.grid_at.min(pane.instance_count);
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
        let write_ms = write_start.elapsed().as_secs_f32() * 1000.0;

        let scene_start = std::time::Instant::now();
        // The scene pass: draw into the pane's offscreen target, on the
        // encoder egui-wgpu executes before its own render pass. paint()
        // then just composites the finished texture.
        let pane = resources
            .panes
            .get(&self.pane_id)
            .expect("created by pane_buffers above");
        let draws = pane.instance_count > 0 || pane.edge_count > 0;
        if let Some(offscreen) = pane.offscreen.as_ref().filter(|_| draws) {
            // Bracket the scene pass and the bloom chain together: what the
            // overlay wants is the cost of drawing THE LATTICE, which is both.
            // Skipped while a readback is still in flight, so the query set is
            // never overwritten mid-cycle.
            let timing =
                self.drives_timer() && resources.timer.as_ref().is_some_and(GpuTimer::arming);
            let opening = if timing {
                resources.timer.as_ref().and_then(GpuTimer::opening)
            } else {
                None
            };
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
                timestamp_writes: opening,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // The grid sits at the home sheet's own depth, so it goes
            // between the sheets behind it and the home sheet itself —
            // NOT under everything. Under everything, a node on a sheet
            // behind the home one punches its clearing through the home
            // grid, which puts a hole in the layer it is supposed to be
            // hidden by. `grid_at` is where that seam falls; the home
            // sheet's own clearings are the tail of the first run, ahead of
            // the grid, so they can hide the sheets behind without eating
            // the grid they sit on.
            let nodes = |pass: &mut wgpu::RenderPass<'_>, range: std::ops::Range<u32>| {
                if range.is_empty() {
                    return;
                }
                pass.set_pipeline(&resources.pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                pass.draw(0..4, range);
            };
            nodes(&mut pass, 0..pane.grid_at);
            if pane.edge_count > 0 {
                pass.set_pipeline(&resources.edge_pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_vertex_buffer(0, pane.edge_buffer.slice(..));
                pass.draw(0..4, 0..pane.edge_count);
            }
            nodes(&mut pass, pane.grid_at..pane.instance_count);
            drop(pass);

            // Skipped entirely at strength 0: the composite multiplies the
            // never-written quarter texture by 0, and fresh wgpu textures
            // read as zero anyway.
            if self.uniforms.misc2[3] > 0.0 {
                self.run_bloom_chain(egui_encoder, resources, offscreen);
            }

            if timing {
                if let Some(timer) = resources.timer.as_mut() {
                    timer.close(egui_encoder);
                }
            }
        } else if let Some(out) = &self.stats {
            // No pass was encoded, so no reading can land and the timer's
            // cycle does not turn over. Say "nothing measured" rather than
            // leaving the last real figure sitting there: `poll` returns None
            // from Idle forever, and the overlay would keep re-averaging a
            // number from whenever the lattice last drew.
            //
            // The distinction is why GPU_TIME_PENDING exists at all — a frozen
            // reading and a live one are the same bits otherwise. A lattice
            // CAN sit here indefinitely: with the grid's alpha at zero, no
            // idle marker and no trail, a silent lattice ships neither a node
            // nor an edge.
            out.gpu_ms.store(GPU_TIME_PENDING, std::sync::atomic::Ordering::Relaxed);
        }

        let scene_ms = scene_start.elapsed().as_secs_f32() * 1000.0;

        if let Some(stats) = &self.stats {
            use std::sync::atomic::Ordering::Relaxed;
            let prepare_ms = prepare_start.elapsed().as_secs_f32() * 1000.0;
            stats.prepare_ms.store(prepare_ms.to_bits(), Relaxed);
            stats.poll_ms.store(poll_ms.to_bits(), Relaxed);
            stats.write_ms.store(write_ms.to_bits(), Relaxed);
            stats.scene_ms.store(scene_ms.to_bits(), Relaxed);
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
        // Nothing was rendered into the offscreen target. The edges count as
        // much as the nodes here — see `prepare`, where the same test decides
        // whether the target exists at all.
        if pane.instance_count == 0 && pane.edge_count == 0 {
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
