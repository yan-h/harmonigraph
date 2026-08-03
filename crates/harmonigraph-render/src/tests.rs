//! Unit tests for the lattice renderer. The GPU-backed ones no-op when
//! no headless adapter is available (CI without a GPU).

use super::*;

#[test]
fn baked_shader_validates() {
    validate_wgsl(SHADER_SRC)
        .expect("baked lattice.wgsl must parse, validate, and keep its entry points");
}

/// The `const _: () = assert!(PITCH_LUT_N == 64)` in `lib.rs` ties one Rust
/// literal to another; the two that decide what the GPU actually reads are in
/// WGSL, where no compiler is checking them against the scene's constant. This
/// is the half that catches a one-sided bump.
///
/// Both slips are silent otherwise. Raise the array but not the const and the
/// shader walks 63 entries of a longer table, painting a top-of-range glyph the
/// color of a pitch halfway down the ramp while the disc under it takes the
/// top — the mismatch #165 closed, reopened at several times the width. Raise
/// the const but not the array and the surplus indices clamp at runtime, which
/// is not a validation error either. naga sees a well-formed shader both ways,
/// `min_binding_size: None` means an over-long buffer never complains, and the
/// scene tests read PITCH_LUT_N symbolically, so they pass at any value.
#[test]
fn the_shaders_pitch_lut_is_the_length_the_scene_says() {
    let n = harmonigraph_scene::PITCH_LUT_N;
    for needle in [format!("array<vec4<f32>, {n}>"), format!("const PITCH_LUT_N: u32 = {n}u;")] {
        assert!(
            SHADER_SRC.contains(&needle),
            "lattice.wgsl must declare `{needle}` to match harmonigraph_scene::PITCH_LUT_N \
             ({n}); the CPU uploads that many entries and the GPU would index a different table",
        );
    }
}

/// blit.wgsl has no hot-reload path, so a broken edit would otherwise
/// first surface as a pipeline panic inside a DAW.
#[test]
fn baked_blit_shader_validates() {
    let module = naga::front::wgsl::parse_str(BLIT_SRC)
        .map_err(|e| e.emit_to_string(BLIT_SRC))
        .expect("blit.wgsl must parse");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("blit.wgsl must validate");
    for required in
        ["vs_blit", "fs_blit", "fs_bright", "fs_blur_h", "fs_blur_v", "fs_composite"]
    {
        assert!(
            module.entry_points.iter().any(|ep| ep.name == required),
            "missing entry point `{required}`"
        );
    }
}

#[test]
fn octave_packing_matches_the_documented_layout() {
    let mut levels = [0.0f32; harmonigraph_scene::OCTAVE_SLOTS];
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
    let words = pack_octaves(&[2.0; harmonigraph_scene::OCTAVE_SLOTS]);
    assert_eq!(words[0], 0xFFFF_FFFF);
}

/// Build the real pipelines against a headless device. This validates
/// the vertex-layout <-> shader-input contract (attribute locations,
/// formats, strides) that neither the naga check (shader only) nor the
/// type system (Rust side only) covers — a mismatch otherwise panics
/// at first paint inside a host.
#[test]
fn pipelines_build_against_a_headless_device() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let _resources =
        LatticeResources::new(&device, &queue, wgpu::TextureFormat::Bgra8Unorm);
}

pub(crate) fn headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let Ok(adapter) =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        eprintln!("no GPU adapter available; skipping");
        return None;
    };
    let pair = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
        .expect("headless device");
    Some(pair)
}

/// A device that actually granted `TIMESTAMP_QUERY`, so `GpuTimer::new`
/// returns `Some` and the readback cycle is live. Without the feature the
/// timer is `None` and any test about it would pass vacuously — hence a
/// separate constructor rather than a flag on [`headless_device`].
fn headless_device_with_timestamps() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let Ok(adapter) =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        eprintln!("no GPU adapter available; skipping");
        return None;
    };
    if !adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        eprintln!("adapter has no timestamp queries; skipping");
        return None;
    }
    let pair = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))
    .expect("headless device with timestamp queries");
    Some(pair)
}

/// Two lattice views in ONE frame — the docked pane plus the Video tab's
/// preview — must survive the submit that follows them.
///
/// egui-wgpu runs every callback's `prepare` on one shared encoder and submits
/// once, at the end. The GPU timer's readback cycle is per-device and assumes
/// its steps land in different frames: `close` records a copy into the staging
/// buffer, and the next frame's `poll` maps that buffer. With two callbacks
/// driving one timer, both steps happened inside a single frame — the second
/// callback mapped the buffer the first had just recorded a copy into — and
/// submitting that encoder is a validation error ("Buffer with
/// 'lattice_gpu_timer_staging' label is still mapped"), fatal by default and
/// enough to take a plugin's host process down. This is that frame.
#[test]
fn a_second_lattice_view_in_the_same_frame_does_not_break_the_submit() {
    let Some((device, queue)) = headless_device_with_timestamps() else {
        return;
    };
    const SIZE: [u32; 2] = [128, 128];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let scene = parity_scene();
    let size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    // Exactly the plugin's pairing: the docked Lattice pane owns id 0 and the
    // stats sink; the Video preview is a second view with neither.
    let docked = LatticeCallback::from_scene(
        &scene,
        LatticeLabels::default(),
        size,
        format,
        0,
        Some(std::sync::Arc::new(LatticeStats::default())),
    );
    let preview = LatticeCallback::from_scene(
        &scene,
        LatticeLabels::default(),
        size,
        format,
        1,
        None,
    );

    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    // Several frames: the cycle is Idle -> Recorded -> Mapping -> Idle, so the
    // premature map can only be recorded once a frame has armed the timer.
    for _ in 0..4 {
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut bufs = docked.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        bufs.extend(preview.prepare(&device, &queue, &screen, &mut encoder, &mut resources));
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
    }
}

/// A scene exercising every draw path: lit + idle + outlined + hovered
/// nodes with octave indicators, a chord beam, and solid + dashed grid
/// lines, all overlapping so blend order matters.
fn parity_scene() -> Scene {
    use glam::{Vec3, Vec4};
    use harmonigraph_core::LatticePos;

    let mut nodes = Vec::new();
    for i in 0..6u32 {
        let f = i as f32;
        // Slots walked inside the DEFAULT Range: a level on a slot the view
        // doesn't show is drawn by no sector, so a scene claiming to exercise
        // every draw path has to sound in the range it renders. Asked of the
        // same layout the scene carries below rather than re-derived from the
        // slot count, which would part company with it the first time the
        // default window moved. The slots a node draws depend on its pitch
        // class, so this is the set EVERY node here has: the highest node's
        // low end, the lowest node's high end.
        let layout = harmonigraph_scene::OctaveLayout::default();
        let (low, _) = layout.slots(0.0);
        let (_, high) = layout.slots(950.0);
        let slot = |k: usize| (low.max(0) as usize) + k % (high - low + 1) as usize;
        let mut octaves = [0.0f32; harmonigraph_scene::OCTAVE_SLOTS];
        octaves[slot(i as usize)] = 1.0 - f * 0.1;
        octaves[slot(i as usize + 5)] = 0.4;
        nodes.push(harmonigraph_scene::NodeInstance {
            lattice_pos: LatticePos::new(i as i32 - 3, i as i32 % 2, 0),
            // Cluster tightly around the origin so discs overlap and
            // draw order shows in the output.
            world_pos: Vec3::new(f * 0.45 - 1.1, (f % 3.0) * 0.4 - 0.4, f * 0.3 - 0.75),
            color: Vec4::new(0.25 + f * 0.12, 0.55 - f * 0.05, 0.95 - f * 0.1, 1.0),
            activation: if i % 3 == 0 { 1.0 } else { 0.3 + f * 0.1 },
            octaves,
            seed: f * 0.13,
            outlined: i == 4,
            hovered: i == 1,
            on_home: i % 2 == 0,
            // The off-sheet half draws small and knocks out, so the
            // every-draw-path scene exercises both sevens-layer branches
            // (the scaled billboard and the gutter's extra alpha) as well.
            scale: if i % 2 == 0 { 1.0 } else { 0.55 },
            gutter: if i % 2 == 0 { 0.0 } else { 0.12 },
            comma: if i % 2 == 0 { 0.0 } else { -27.26 },
            cents: f * 190.0,
            // Exercise the mark paths: one node marked melody, one bass,
            // and one claiming both slots at once (the split mark).
            melody_slots: if i == 0 || i == 4 { 1 << slot(i as usize) } else { 0 },
            bass_slots: if i == 2 || i == 4 { 1 << slot(i as usize) } else { 0 },
            melody_level: if i == 0 || i == 4 { 1.0 } else { 0.0 },
            bass_level: if i == 2 || i == 4 { 1.0 } else { 0.0 },
            melody_color: Vec4::new(1.0, 0.85, 0.4, 1.0),
            bass_color: Vec4::new(0.45, 0.8, 1.0, 1.0),
            trail: 0.0,
        });
    }
    let grid = vec![
        harmonigraph_scene::EdgeInstance {
            a: Vec3::new(-1.8, -0.6, -0.3),
            b: Vec3::new(1.6, -0.6, -0.3),
            color: Vec4::new(0.16, 0.17, 0.20, 0.55),
            strength: 0.55,
            dashed: false,
        },
        harmonigraph_scene::EdgeInstance {
            a: Vec3::new(-1.2, 0.7, -0.6),
            b: Vec3::new(1.2, 0.4, 0.6),
            color: Vec4::new(0.16, 0.17, 0.20, 0.55),
            strength: 0.4,
            dashed: true,
        },
    ];
    Scene {
        nodes,
        camera: harmonigraph_scene::Camera::default(),
        time: 1.25,
        // The ground the sevens knockout clears to; the half of this
        // scene's nodes that carry a gutter exercise it.
        background: harmonigraph_scene::skin::panel_color(),
        sevens_soft: 0.24,
        node_radius: 0.34,
        mark_thickness: 0.09,
        // Off, on the same grounds as pulse_octaves below: a single-instant
        // parity image can't depend on which moment of a cycle it lands on.
        pulse_marks: Default::default(),
        node_style: Default::default(),
        core_radius: 0.46,
        core_solidity: 1.0,
        outer_inner: 0.545,
        outer_outer: 0.795,
        outer_gap: 0.12,
        // The plain circular division: this scene is about how the draw
        // paths composite, so the indicators are the ones every other
        // setting is a departure from.
        octave_layout: harmonigraph_scene::OctaveLayout::default(),
        // Off, on the same grounds as trail_mark below: the parity image is
        // about how a note draws at a single instant, and a pulse would make
        // that instant depend on which one the fixture happened to land on.
        pulse_octaves: Default::default(),
        idle_marker: harmonigraph_scene::IdleMarker::None,
        idle_radius: 0.0,
        grid,
        grid_thickness: 1.0,
        // The parity image is about how a NOTE is drawn; the trail marks
        // only idle nodes and has its own tests. Off keeps this baseline
        // comparable to the ones taken before it existed.
        trail_mark: Default::default(),
        trail_strength: 0.0,
        node_idle: Vec4::new(0.27, 0.29, 0.34, 1.0),
        // A blue->red sweep across the whole table, so a glyph's color is a
        // reading of which entry it landed on. Spanned off PITCH_LUT_N rather
        // than a literal: `from_fn` sizes itself from the field, so a literal
        // divisor would silently stop covering the table when the constant
        // grows, leaving every entry past it out of gamut and clamping to one
        // color — the fixture would still render, and would still be green.
        pitch_lut: std::array::from_fn(|k| {
            let t = k as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
            Vec4::new(t, 0.4, 1.0 - t, 1.0)
        }),
        darkest_pitch: 24.0,
        brightest_pitch: 108.0,
        render_scale: 1.0,
        // Parity with the direct-to-egui-pass reference requires bloom off.
        bloom_strength: 0.0,
    }
}

/// Render into a fresh texture cleared to `clear`, handing the pass to
/// `draw`, and return the texture for readback.
pub(crate) fn render_to_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    size: [u32; 2],
    format: wgpu::TextureFormat,
    clear: wgpu::Color,
    draw: impl FnOnce(&mut wgpu::RenderPass<'static>),
) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("parity_target"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("parity_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        draw(&mut pass);
    }
    queue.submit([encoder.finish()]);
    texture
}

pub(crate) fn readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    size: [u32; 2],
) -> Vec<u8> {
    let bytes_per_row = size[0] * 4; // 256-wide RGBA rows are aligned
    assert_eq!(bytes_per_row % 256, 0, "test sizes keep rows aligned");
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("parity_readback"),
        size: (bytes_per_row * size[1]) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
    device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    slice.get_mapped_range().to_vec()
}

/// The refactor's core claim: rendering offscreen (with the depth
/// attachment) and compositing through blit.wgsl reproduces what the
/// old renderer produced by drawing straight into the egui pass. Runs
/// the same scene through both paths and compares pixels; tolerance 3
/// covers the 8-bit quantization of the intermediate texture.
#[test]
fn offscreen_composite_matches_direct_draw() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [256, 256];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let scene = parity_scene();
    let cb = LatticeCallback::from_scene(
        &scene,
        LatticeLabels::default(),
        egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
        format,
        7,
        None,
    );

    // prepare(): uploads buffers and renders the offscreen scene pass.
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor {
        size_in_pixels: SIZE,
        pixels_per_point: 1.0,
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    let user_bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
    queue.submit(user_bufs.into_iter().chain([encoder.finish()]));

    let clear = wgpu::Color {
        r: 0.07,
        g: 0.08,
        b: 0.09,
        a: 1.0,
    };
    let rect =
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));

    // Path A: composite the offscreen texture, as paint() now does.
    let composite_tex = render_to_texture(&device, &queue, SIZE, format, clear, |pass| {
        cb.paint(
            egui::PaintCallbackInfo {
                viewport: rect,
                clip_rect: rect,
                pixels_per_point: 1.0,
                screen_size_px: SIZE,
            },
            pass,
            &resources,
        );
    });

    // Path B: the pre-offscreen renderer — same buffers and draw order,
    // depthless pipelines, straight into the target pass.
    let res: &LatticeResources = resources.get().expect("prepare created resources");
    let (node_pipeline, edge_pipeline) =
        create_pipelines(&device, SHADER_SRC, format, &res.bind_group_layout, false);
    let pane = res.panes.get(&7).expect("prepare created the pane");
    let direct_tex = render_to_texture(&device, &queue, SIZE, format, clear, |pass| {
        // The grid sits at the home sheet's depth, so it is drawn INSIDE the
        // node run, at `grid_at` — mirror that here or the two paths differ
        // by draw order rather than by the thing under test.
        let nodes = |pass: &mut wgpu::RenderPass<'static>, range: std::ops::Range<u32>| {
            if !range.is_empty() {
                pass.set_pipeline(&node_pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                pass.draw(0..4, range);
            }
        };
        nodes(pass, 0..pane.grid_at);
        if pane.edge_count > 0 {
            pass.set_pipeline(&edge_pipeline);
            pass.set_bind_group(0, &pane.bind_group, &[]);
            pass.set_vertex_buffer(0, pane.edge_buffer.slice(..));
            pass.draw(0..4, 0..pane.edge_count);
        }
        nodes(pass, pane.grid_at..pane.instance_count);
    });

    let composite = readback(&device, &queue, &composite_tex, SIZE);
    let direct = readback(&device, &queue, &direct_tex, SIZE);

    // Guard against vacuous success: the scene must actually have drawn
    // over the clear color somewhere.
    let bg = [18u8, 20, 23, 255]; // clear color as 8-bit RGBA
    assert!(
        direct
            .chunks(4)
            .any(|px| px.iter().zip(bg).any(|(&c, b)| c.abs_diff(b) > 8)),
        "direct render drew nothing; the parity comparison is vacuous"
    );

    let (mut max_diff, mut at) = (0u8, 0usize);
    for (i, (&a, &b)) in composite.iter().zip(&direct).enumerate() {
        if a.abs_diff(b) > max_diff {
            max_diff = a.abs_diff(b);
            at = i;
        }
    }
    assert!(
        max_diff <= 3,
        "offscreen+composite diverges from direct draw: max channel diff \
         {max_diff} at byte {at} (composite {:?} vs direct {:?})",
        &composite[at & !3..(at & !3) + 4],
        &direct[at & !3..(at & !3) + 4],
    );
}

/// `pulse_octaves` is Off in `parity_scene` and every fixture derived from
/// it (deliberately — see that scene's own comment), so nothing above ever
/// takes a `mode != 0u` branch in the shader's pulse code: `pulse_wave`, the
/// `select` that splits Together from Alternating, and the
/// `oct_pulse.x`/`.y` multiplies in the octave-glyph loop are validated by
/// `baked_shader_validates` (parsed, never run) but not actually exercised
/// by any render. This runs them: Together and Alternating must draw
/// differently from EACH OTHER at one instant (the near/rest split is the
/// whole feature), and Alternating must draw differently across time (or it
/// isn't animating at all). The two times are picked without reference to
/// the shader's `PULSE_HZ` — the claim is that time matters, not a
/// particular phase, so retuning the rate can't make this pass by accident.
#[test]
fn octave_pulse_alternating_differs_from_together_and_moves_with_time() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [256, 256];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vec_size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    let mut shot = |scene: &Scene, pane_id: u64| -> Vec<u8> {
        let cb = LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            vec_size,
            format,
            pane_id,
            None,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let tex = render_to_texture(&device, &queue, SIZE, format, wgpu::Color::BLACK, |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: 1.0,
                    screen_size_px: SIZE,
                },
                pass,
                &resources,
            );
        });
        readback(&device, &queue, &tex, SIZE)
    };
    let differs =
        |a: &[u8], b: &[u8]| a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| x != y).count();

    let mut off = parity_scene();
    off.time = 0.4;
    let off_a = shot(&off, 70);
    off.time = 1.1;
    let off_b = shot(&off, 71);
    assert_eq!(differs(&off_a, &off_b), 0, "Pulse::Off must not depend on scene.time");

    let mut together = parity_scene();
    together.pulse_octaves = harmonigraph_scene::Pulse::Together;
    together.time = 0.4;
    let together_a = shot(&together, 72);

    let mut alternating = parity_scene();
    alternating.pulse_octaves = harmonigraph_scene::Pulse::Alternating;
    alternating.time = 0.4;
    let alternating_a = shot(&alternating, 73);
    assert!(
        differs(&together_a, &alternating_a) > 0,
        "Together and Alternating must draw differently at the same instant \
         -- that split is the whole feature"
    );

    alternating.time = 1.1;
    let alternating_b = shot(&alternating, 74);
    assert!(
        differs(&alternating_a, &alternating_b) > 0,
        "Alternating did not change between two different times; \
         pulse_wave is not actually reading the clock"
    );
}

/// The slot mask naming middle C's octave — the one the node below sounds
/// in, and so the one a mark can link back to.
const MIDDLE_C: u32 = 1 << harmonigraph_scene::MIDDLE_C_SLOT;

/// Bloom must add light (halo energy over the bloom-off output) —
/// and only when asked: strength 0 keeps the parity test above valid.
/// One big centered node, sounding, with one octave slot lit: a clean
/// backdrop for measuring how much of the picture a mark actually
/// covers. parity_scene deliberately overlaps its nodes, which hides
/// most of a mark behind whatever draws in front of it.
fn single_marked_node(melody_slots: u32, bass_slots: u32) -> Scene {
    use glam::{Vec3, Vec4};
    use harmonigraph_core::LatticePos;

    let mut octaves = [0.0f32; harmonigraph_scene::OCTAVE_SLOTS];
    // Middle C's slot: the marks link back to a sector, so the note has to
    // sound in one the view actually shows.
    octaves[harmonigraph_scene::MIDDLE_C_SLOT] = 1.0;
    let mut scene = parity_scene();
    scene.nodes = vec![harmonigraph_scene::NodeInstance {
        lattice_pos: LatticePos::ORIGIN,
        world_pos: Vec3::ZERO,
        color: Vec4::new(0.35, 0.55, 0.85, 1.0),
        activation: 1.0,
        octaves,
        seed: 0.0,
        outlined: false,
        hovered: false,
        on_home: true,
        scale: 1.0,
        gutter: 0.0,
        comma: 0.0,
        cents: 0.0,
        melody_slots,
        bass_slots,
        // A mark draws at the level its own note is at; these stand in for
        // a freshly-held note.
        melody_level: f32::from(melody_slots != 0),
        bass_level: f32::from(bass_slots != 0),
        // Distinct hues so the both-ends check below can tell the two
        // rings apart; in the app these are the marked SECTORS' colors.
        melody_color: Vec4::new(1.0, 0.85, 0.4, 1.0),
        bass_color: Vec4::new(0.45, 0.8, 1.0, 1.0),
        trail: 0.0,
    }];
    scene.grid.clear();
    // Fill a good share of the frame, so the measurements below are
    // about the mark's design rather than about pixel quantization.
    scene.node_radius = 1.1;
    scene
}

#[test]
fn melody_bass_marks_are_visible_as_rings_around_the_band() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [256, 256];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vec_size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);

    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor {
        size_in_pixels: SIZE,
        pixels_per_point: 1.0,
    };
    let mut shot = |scene: &Scene, pane_id: u64| -> Vec<u8> {
        let cb = LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            vec_size,
            format,
            pane_id,
            None,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let tex = render_to_texture(
            &device,
            &queue,
            SIZE,
            format,
            wgpu::Color::BLACK,
            |pass| {
                cb.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: 1.0,
                        screen_size_px: SIZE,
                    },
                    pass,
                    &resources,
                );
            },
        );
        readback(&device, &queue, &tex, SIZE)
    };

    let unmarked = shot(&single_marked_node(0, 0), 40);
    let changed_px = |other: &[u8]| -> usize {
        unmarked
            .chunks(4)
            .zip(other.chunks(4))
            .filter(|(a, b)| a != b)
            .count()
    };

    // Measure against the node's OWN footprint, not an absolute pixel
    // count: what matters is that the mark claims a real share of the
    // thing it is marking, at whatever size it happens to be drawn.
    let node_px = unmarked.chunks(4).filter(|px| px[..3] != [0, 0, 0]).count();
    let melody = shot(&single_marked_node(MIDDLE_C, 0), 41);
    let both_px = changed_px(&melody);
    eprintln!("node {node_px} px; mark {both_px}");
    // A floor, not a target, measured against the node's whole lit
    // footprint (glow included). The mark is a full ring bracketing the
    // octave band, so it claims a real share; the floor exists because an
    // early version drew a sub-pixel arc that read as nothing at all in the
    // DAW (well under 1%), which is what this catches. Current: ~36%.
    assert!(
        both_px * 8 > node_px,
        "the mark ring covers too little of the node to find: \
         {both_px} px of {node_px}"
    );

    // Nothing marked draws no mark at all.
    let off = shot(&single_marked_node(0, 0), 44);
    assert_eq!(changed_px(&off), 0, "an unmarked node must draw no mark");

    // A note claimed by BOTH ends -- a lone held note, or a chord whose top
    // and bottom share a pitch class -- must not be blanked: that vanishes
    // the mark exactly when two things are true at once. The two ends are
    // rings at DIFFERENT radii, so both simply draw: the result must cover
    // at least as much as one end alone. This guards that.
    let split = shot(&single_marked_node(MIDDLE_C, MIDDLE_C), 45);
    let split_px = changed_px(&split);
    eprintln!("split mark {split_px} px of {node_px}");
    assert!(
        split_px >= both_px,
        "a mark claimed by both ends all but disappeared: \
         {split_px} px against {both_px} for one end alone"
    );

    // ...and it really is BOTH rings, not one end quietly winning: the
    // melody-only and bass-only pictures must each differ from it.
    let bass_only = shot(&single_marked_node(0, MIDDLE_C), 46);
    let differs = |a: &[u8], b: &[u8]| {
        a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| x != y).count()
    };
    assert!(
        differs(&split, &melody) > 0 && differs(&split, &bass_only) > 0,
        "a both-ends mark is indistinguishable from a single-ended one"
    );
}

/// The mark-ring twin of `octave_pulse_alternating_differs_from_together_and_moves_with_time`
/// above: `pulse_marks` is Off in every fixture in this file, so
/// `mark_ring_alpha`'s `near` accumulation and its `mix(pair.y, pair.x,
/// near)` blend have never run under a mode where `pair.x != pair.y`. Same
/// two claims, on the ring instead of the glyphs.
#[test]
fn mark_pulse_alternating_differs_from_together_and_moves_with_time() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [256, 256];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vec_size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    let mut shot = |scene: &Scene, pane_id: u64| -> Vec<u8> {
        let cb = LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            vec_size,
            format,
            pane_id,
            None,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let tex = render_to_texture(&device, &queue, SIZE, format, wgpu::Color::BLACK, |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: 1.0,
                    screen_size_px: SIZE,
                },
                pass,
                &resources,
            );
        });
        readback(&device, &queue, &tex, SIZE)
    };
    let differs =
        |a: &[u8], b: &[u8]| a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| x != y).count();

    let mut off = single_marked_node(MIDDLE_C, 0);
    off.time = 0.4;
    let off_a = shot(&off, 90);
    off.time = 1.1;
    let off_b = shot(&off, 91);
    assert_eq!(differs(&off_a, &off_b), 0, "Pulse::Off must not depend on scene.time");

    let mut together = single_marked_node(MIDDLE_C, 0);
    together.pulse_marks = harmonigraph_scene::Pulse::Together;
    together.time = 0.4;
    let together_a = shot(&together, 92);

    let mut alternating = single_marked_node(MIDDLE_C, 0);
    alternating.pulse_marks = harmonigraph_scene::Pulse::Alternating;
    alternating.time = 0.4;
    let alternating_a = shot(&alternating, 93);
    assert!(
        differs(&together_a, &alternating_a) > 0,
        "Together and Alternating must draw differently at the same instant \
         -- that split is the whole feature"
    );

    alternating.time = 1.1;
    let alternating_b = shot(&alternating, 94);
    assert!(
        differs(&alternating_a, &alternating_b) > 0,
        "Alternating did not change between two different times; \
         pulse_wave is not actually reading the clock"
    );
}

/// The bug the fix above two tests replaced: `pulse_octaves` originally
/// keyed the octave-glyph loop's near/rest split off `level > 0.0` alone,
/// so ANY sounding octave took the "near" phase under Alternating -- not
/// just the one a melody or bass ring actually points at. A chord tone
/// that is neither the highest nor the lowest held note isn't an indicator
/// this feature is about, and would have pulsed as if it were.
///
/// Isolates the octave-glyph layer from the ring itself (`mark_thickness =
/// 0`, so `mark_ring` returns no coverage regardless of `marks`) and
/// renders the SAME sounding octave once not marked and once marked as the
/// melody. Under the old behavior these are pixel-identical -- both are
/// sounding, so both took the near phase. Under the fix only the marked
/// one does.
#[test]
fn octave_pulse_only_lights_the_melody_or_bass_slot_not_every_sounding_one() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [256, 256];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vec_size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    let mut shot = |scene: &Scene, pane_id: u64| -> Vec<u8> {
        let cb = LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            vec_size,
            format,
            pane_id,
            None,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let tex = render_to_texture(&device, &queue, SIZE, format, wgpu::Color::BLACK, |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: 1.0,
                    screen_size_px: SIZE,
                },
                pass,
                &resources,
            );
        });
        readback(&device, &queue, &tex, SIZE)
    };
    let differs =
        |a: &[u8], b: &[u8]| a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| x != y).count();

    let mut not_extreme = single_marked_node(0, 0);
    not_extreme.mark_thickness = 0.0;
    not_extreme.pulse_octaves = harmonigraph_scene::Pulse::Alternating;
    not_extreme.time = 0.4;

    let mut extreme = single_marked_node(MIDDLE_C, 0);
    extreme.mark_thickness = 0.0;
    extreme.pulse_octaves = harmonigraph_scene::Pulse::Alternating;
    extreme.time = 0.4;

    let not_extreme_px = shot(&not_extreme, 95);
    let extreme_px = shot(&extreme, 96);
    assert!(
        differs(&not_extreme_px, &extreme_px) > 0,
        "a sounding octave that is the melody/bass extreme must pulse \
         differently from one that merely sounds; keying the split off \
         level alone lit every sounding octave the same way"
    );
}

/// `Pulse::Shimmer` is a whole-layer sweep rather than the near/rest breathe
/// the two modes above are, so the claims that pin it are different ones: it
/// must draw differently from Off, it must move with the clock, and — the
/// part `Together`/`Alternating` cannot do — it must do both on a node with
/// NO mark at all, since the sheet of bands is nothing to do with which
/// octave a ring points at. Run on both layers, since each reads its own
/// mode uniform and its own direction.
///
/// The instants are picked without reference to `SHIMMER_SPEED` or
/// `SHIMMER_PERIOD`: the claim is that the clock reaches the layer, not that
/// a particular phase does, so retuning the sweep cannot make this pass by
/// accident.
#[test]
fn shimmer_sweeps_an_unmarked_layer_and_moves_with_time() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [256, 256];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vec_size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    let mut shot = |scene: &Scene, pane_id: u64| -> Vec<u8> {
        let cb = LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            vec_size,
            format,
            pane_id,
            None,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let tex = render_to_texture(&device, &queue, SIZE, format, wgpu::Color::BLACK, |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: 1.0,
                    screen_size_px: SIZE,
                },
                pass,
                &resources,
            );
        });
        readback(&device, &queue, &tex, SIZE)
    };
    let differs =
        |a: &[u8], b: &[u8]| a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| x != y).count();

    // No mark on either end: `Together` and `Alternating` would have no
    // slot to single out here, which is exactly the state Shimmer has to
    // work in.
    let mut off = single_marked_node(0, 0);
    off.time = 0.4;
    let off_a = shot(&off, 100);

    let mut octaves = single_marked_node(0, 0);
    octaves.pulse_octaves = harmonigraph_scene::Pulse::Shimmer;
    octaves.time = 0.4;
    let octaves_a = shot(&octaves, 101);
    assert!(
        differs(&off_a, &octaves_a) > 0,
        "the octave layer's Shimmer drew the steady picture on an unmarked \
         node; the sheet of bands does not depend on a mark"
    );
    octaves.time = 1.1;
    let octaves_b = shot(&octaves, 102);
    assert!(
        differs(&octaves_a, &octaves_b) > 0,
        "the octave layer's Shimmer did not change between two different \
         times; the bands are not reading the clock"
    );

    // The rings need a mark to exist at all -- that is the ring, not the
    // shimmer -- so this half marks one end and leaves the octave layer
    // steady, which isolates what `pulse_marks` did.
    let mut ring_off = single_marked_node(MIDDLE_C, 0);
    ring_off.time = 0.4;
    let ring_off_a = shot(&ring_off, 103);

    let mut marks = single_marked_node(MIDDLE_C, 0);
    marks.pulse_marks = harmonigraph_scene::Pulse::Shimmer;
    marks.time = 0.4;
    let marks_a = shot(&marks, 104);
    assert!(
        differs(&ring_off_a, &marks_a) > 0,
        "the mark rings' Shimmer drew the steady picture"
    );
    marks.time = 1.1;
    let marks_b = shot(&marks, 105);
    assert!(
        differs(&marks_a, &marks_b) > 0,
        "the mark rings' Shimmer did not change between two different \
         times; the bands are not reading the clock"
    );
}

/// Mirrors `SHIMMER_ANGLE` in lattice.wgsl, as a fraction of a turn from the
/// camera's right axis toward its up axis — the direction the octave layer's
/// bands travel, and a quarter turn short of the mark rings'.
///
/// Held to the shader's own literal by `the_probe_moves_along_the_angle_the_shader_sweeps`
/// rather than by a comment asking for it: the probe below moves a node along
/// this and along the quarter turn from it and reads which layer each move
/// leaves alone, so an angle that drifted from the shader's would leave the
/// test comparing two arbitrary directions — passing on its margin while
/// measuring nothing about squareness.
const SHIMMER_ANGLE_TURNS: f32 = 0.125;

/// The mirror above, enforced. `SHIMMER_ANGLE` invites retuning in its own
/// comment ("so the two cannot drift out of square when the diagonal is
/// retuned"), which is exactly the edit that would strand the probe.
#[test]
fn the_probe_moves_along_the_angle_the_shader_sweeps() {
    let needle = format!("const SHIMMER_ANGLE: f32 = {SHIMMER_ANGLE_TURNS} * TAU;");
    assert!(
        SHADER_SRC.contains(&needle),
        "lattice.wgsl must declare `{needle}` to match SHIMMER_ANGLE_TURNS; the probe in \
         the_shimmer_is_one_field_across_the_lattice_and_the_layers_run_square moves nodes \
         along that angle to find which layer each move leaves alone, and against a \
         different one it measures neither layer's direction",
    );
}

/// How far that probe moves the node, in world units: about half the
/// shader's band period, so a move ALONG a layer's own travel lands it on a
/// very different part of the sweep rather than back where it started.
const SHIMMER_PROBE_STEP: f32 = 2.5;

/// `scene`'s only node, moved [`SHIMMER_PROBE_STEP`] world units along the
/// camera-plane direction `turns` of a turn from the camera's right axis.
fn move_node_across_the_view(scene: &mut Scene, turns: f32) {
    let (right, up) = scene.camera.right_up();
    let a = turns * std::f32::consts::TAU;
    scene.nodes[0].world_pos = (right * a.cos() + up * a.sin()) * SHIMMER_PROBE_STEP;
}

/// Two claims that are the whole point of the shimmer, and that the test
/// above would pass without either:
///
/// **It is ONE sheet across the lattice, not a copy per node.** The field is
/// the fragment's place on the plane the billboards face, so a node MOVED
/// across that plane meets the bands at a different phase and draws with a
/// different amount of light in it. Read off a per-node coordinate (`in.uv`,
/// say) every node would run an identical private copy, moving one would
/// change nothing but where it landed, and both "along" measurements below
/// would collapse into their own controls.
///
/// **The two layers run square to each other.** Moving a node ALONG a band —
/// perpendicular to the direction that band travels — slides it down a line
/// the field is constant on, so that layer's picture is the one it was.
/// Which move is the harmless one is therefore a direct reading of a layer's
/// direction, and the mark rings' harmless move has to be the octave
/// glyphs' telling one, and the other way about.
///
/// The two directions are mirror images across the camera's up axis, so each
/// layer's "along" and "across" moves put the node in exactly mirrored
/// places: whatever the move costs in rasterization and perspective, it
/// costs both equally, and what is left between them is the shimmer.
#[test]
fn the_shimmer_is_one_field_across_the_lattice_and_the_layers_run_square() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [256, 256];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vec_size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    let mut pane = 200u64;
    let mut shot = |scene: &Scene| -> Vec<u8> {
        pane += 1;
        let cb = LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            vec_size,
            format,
            pane,
            None,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let tex = render_to_texture(&device, &queue, SIZE, format, wgpu::Color::BLACK, |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: 1.0,
                    screen_size_px: SIZE,
                },
                pass,
                &resources,
            );
        });
        readback(&device, &queue, &tex, SIZE)
    };
    // All the light in the frame. A total rather than a pixel-by-pixel
    // count, because a moved node lands in different pixels by design: what
    // is being compared is how much of it the shimmer let through, not
    // where it went.
    let light = |px: &[u8]| -> i64 {
        px.chunks(4).map(|p| p[0] as i64 + p[1] as i64 + p[2] as i64).sum()
    };
    // How much the picture's total light changes when `make`'s node moves
    // `turns` of a turn off the camera's right axis, against leaving it at
    // the origin.
    let mut move_cost = |make: &dyn Fn() -> Scene, turns: f32| -> i64 {
        let still = light(&shot(&make()));
        let mut moved = make();
        move_node_across_the_view(&mut moved, turns);
        (light(&shot(&moved)) - still).abs()
    };

    let across_the_octave_bands = SHIMMER_ANGLE_TURNS;
    let along_the_octave_bands = SHIMMER_ANGLE_TURNS + 0.25;

    // The control: with nothing shimmering, a move costs only what moving
    // costs — a node landing on its own pixel grid, and the perspective at
    // a place that is not the middle of the frame.
    let steady = || {
        let mut scene = single_marked_node(MIDDLE_C, 0);
        scene.time = 0.4;
        scene
    };
    let steady_across = move_cost(&steady, across_the_octave_bands);
    let steady_along = move_cost(&steady, along_the_octave_bands);

    let octaves = || {
        let mut scene = steady();
        scene.pulse_octaves = harmonigraph_scene::Pulse::Shimmer;
        scene
    };
    let octave_across = move_cost(&octaves, across_the_octave_bands);
    let octave_along = move_cost(&octaves, along_the_octave_bands);

    let marks = || {
        let mut scene = steady();
        scene.pulse_marks = harmonigraph_scene::Pulse::Shimmer;
        scene
    };
    // The rings' bands are the quarter turn round, so the two moves swap
    // roles: what slides the octave glyphs down a band crosses these.
    let mark_across = move_cost(&marks, along_the_octave_bands);
    let mark_along = move_cost(&marks, across_the_octave_bands);

    eprintln!(
        "steady {steady_across}/{steady_along}, octaves {octave_across}/{octave_along}, \
         marks {mark_across}/{mark_along} (across/along)"
    );
    // The control has to STAY small, or the ratios below stop being about the
    // shimmer. Should a bare node move ever get expensive or lopsided — a new
    // depth-dependent layer, a cull edge inside the probe's reach, anything
    // keyed on world position — both figures for a layer would inflate off
    // the same base, the ratio would collapse, and the failure would be
    // reported as a shimmer defect it is not. Measured 25/25.
    let steady = steady_across.max(steady_along);
    assert!(
        steady * 10 < octave_across.min(mark_across),
        "moving a node costs {steady} even with nothing shimmering, which is too near \
         what the shimmering layers cost ({octave_across} and {mark_across}) for the \
         difference between them to be the shimmer's"
    );
    // A multiple, not a threshold: the claim is that crossing the bands
    // dominates sliding along them, and the along-figure is the same move
    // mirrored, so it carries this layer's own share of the control above.
    assert!(
        octave_across > octave_along * 4,
        "moving a node across the octave bands ({octave_across}) barely beat \
         moving it along them ({octave_along}; the steady control costs \
         {steady_across}/{steady_along}) -- either the field is per-node \
         rather than one sheet over the lattice, or the octave layer's bands \
         are not running the way SHIMMER_ANGLE says"
    );
    assert!(
        mark_across > mark_along * 4,
        "moving a node across the mark rings' bands ({mark_across}) barely \
         beat moving it along them ({mark_along}) -- the rings' bands are not \
         a quarter turn from the octave glyphs', which is the 90 degrees \
         between the two textures"
    );
}

#[test]
fn a_real_held_chord_shows_its_melody_and_bass_marks() {
    // End to end, exactly how the app runs it: a held chord through
    // derive_scene, NOT a Scene assembled by hand. The by-hand test
    // above pins the shader down but would happily pass while the
    // tracker -> view -> node-mask path was broken, which is the half
    // that actually reaches a user.
    let Some((device, queue)) = headless_device() else {
        return;
    };
    use harmonigraph_core::{NoteEvent, NoteEventKind, NoteTracker, Tuning};
    use harmonigraph_scene::{derive_scene, Camera, FrameParams, ViewConfig};

    const SIZE: [u32; 2] = [256, 256];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vec_size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);

    let mut tracker = NoteTracker::new();
    for note in [60u8, 64, 67] {
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
    }
    // A small window so the nodes draw big enough to measure.
    let base = ViewConfig {
        extent_threes: 2,
        extent_fives: 2,
        extent_sevens: 0,
        ..ViewConfig::default()
    };
    let scene_for = |marks: bool| {
        derive_scene(
            &tracker,
            &Tuning::default(),
            &ViewConfig { mark_melody: marks, mark_bass: marks, ..base.clone() },
            &FrameParams::default(),
            Camera::default(),
            None,
            // Past ATTACK_TIME: the octave glyphs and the mark rings both
            // ease in over the first 0.15s, so at t=0 there is deliberately
            // nothing on that layer yet.
            0.5,
        )
    };

    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor {
        size_in_pixels: SIZE,
        pixels_per_point: 1.0,
    };
    let mut shot = |scene: &Scene, pane_id: u64| -> Vec<u8> {
        let cb = LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            vec_size,
            format,
            pane_id,
            None,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let tex = render_to_texture(
            &device,
            &queue,
            SIZE,
            format,
            wgpu::Color::BLACK,
            |pass| {
                cb.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: 1.0,
                        screen_size_px: SIZE,
                    },
                    pass,
                    &resources,
                );
            },
        );
        readback(&device, &queue, &tex, SIZE)
    };

    // The masks must survive derive_scene in the first place.
    let marked = scene_for(true);
    let melody_nodes = marked.nodes.iter().filter(|n| n.melody_slots != 0).count();
    let bass_nodes = marked.nodes.iter().filter(|n| n.bass_slots != 0).count();
    assert!(
        melody_nodes > 0 && bass_nodes > 0,
        "derive_scene marked nothing: {melody_nodes} melody, {bass_nodes} bass nodes"
    );

    let off = shot(&scene_for(false), 50);
    let on = shot(&marked, 51);
    let lit = off.chunks(4).filter(|px| px[..3] != [0, 0, 0]).count();
    let changed = off
        .chunks(4)
        .zip(on.chunks(4))
        .filter(|(a, b)| a != b)
        .count();
    eprintln!("chord: {lit} lit px, {changed} changed by the marks");
    // Same reasoning as the by-hand test above; at this node density the
    // ring's screen-space minimum (MARK_RING_MIN_AA) is what keeps it from
    // going sub-pixel.
    assert!(
        changed * 20 > lit,
        "turning the marks on barely changed a real chord: \
         {changed} px of {lit} lit"
    );
}

/// A node showing every octave it can, for reading the wheel's geometry off
/// the picture: no core and no mark rings, so the only thing drawn is the
/// band, and a wide gap so the seams between indicators are several pixels
/// across at the size this renders at.
fn octave_wheel_scene(layout: harmonigraph_scene::OctaveLayout, cents: f32) -> Scene {
    let mut scene = single_marked_node(0, 0);
    scene.octave_layout = layout;
    scene.core_radius = 0.0;
    scene.outer_inner = 0.30;
    scene.outer_outer = 0.95;
    scene.outer_gap = 0.10;
    scene.mark_thickness = 0.0;
    // Every octave the wheel draws for THIS pitch class, and only those: a
    // level on a slot no sector draws is a state `derive_scene` cannot reach,
    // and the swirl and the glow would still take a color from it. Slots
    // outside the packing are what a ring near the pitch limits reaches for,
    // and no note can light one.
    let (low, high) = layout.slots(cents);
    let node = &mut scene.nodes[0];
    node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
    for slot in low.max(0)..=high.min(harmonigraph_scene::OCTAVE_SLOTS as i32 - 1) {
        node.octaves[slot as usize] = 1.0;
    }
    node.cents = cents;
    scene
}

/// The band's lit/unlit profile around a rendered node: index `i` is the
/// angle `360 * i / STEPS` counter-clockwise from screen right.
/// Self-calibrating — it finds the node's center and the band's radius from
/// the image rather than reproducing the camera's arithmetic, which would
/// only re-assert it.
const PROFILE_STEPS: usize = 720;
fn band_profile(px: &[u8], size: u32) -> Vec<bool> {
    let w = size as usize;
    let lit = |x: f32, y: f32| -> bool {
        if x < 0.0 || y < 0.0 || x >= size as f32 || y >= size as f32 {
            return false;
        }
        let i = (y as usize * w + x as usize) * 4;
        px[i] as u32 + px[i + 1] as u32 + px[i + 2] as u32 > 24
    };
    // The node is alone at the world origin and the camera looks at it, so
    // the frame's center is its center. Not the lit pixels' centroid: a
    // fringed band is heavier on the side its wide octaves fall, which would
    // pull a centroid off-center by roughly what the measurement below is
    // trying to see.
    let drawn = (0..size * size)
        .filter(|k| lit((k % size) as f32, (k / size) as f32))
        .count();
    assert!(drawn > 100, "nothing drawn to measure ({drawn} lit px)");
    let (cx, cy) = (size as f32 / 2.0, size as f32 / 2.0);

    // Sample at whichever radius has the most band on it: picking one by
    // arithmetic would land in a seam or off the band as the settings move.
    // Screen y grows downward, so the sample angle is negated.
    let ring = |r: f32| -> Vec<bool> {
        (0..PROFILE_STEPS)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / PROFILE_STEPS as f32;
                lit(cx + r * a.cos(), cy - r * a.sin())
            })
            .collect()
    };
    let best = (4..size / 2)
        .map(|r| (ring(r as f32).iter().filter(|b| **b).count(), r))
        .max()
        .expect("a band to sample")
        .1;
    ring(best as f32)
}

/// Width in degrees of the unlit run containing `at_degrees`, or 0 if that
/// direction is lit.
fn gap_at(profile: &[bool], at_degrees: f32) -> f32 {
    let step = 360.0 / PROFILE_STEPS as f32;
    let start = (at_degrees / step).round() as usize % PROFILE_STEPS;
    if profile[start] {
        return 0.0;
    }
    let mut run = 1;
    let mut k = 1;
    while k < PROFILE_STEPS && !profile[(start + k) % PROFILE_STEPS] {
        run += 1;
        k += 1;
    }
    let mut k = 1;
    while k < PROFILE_STEPS && !profile[(start + PROFILE_STEPS - k) % PROFILE_STEPS] {
        run += 1;
        k += 1;
    }
    run as f32 * step
}

/// Every unlit run around the profile, in degrees. On a closed ring of
/// indicators the only unlit stretches are the Gap setting's slits, one per
/// boundary between neighbours — so counting these counts the indicators,
/// and a missing one shows as two slits merged into a wider hole.
fn unlit_runs(profile: &[bool]) -> Vec<f32> {
    let step = 360.0 / PROFILE_STEPS as f32;
    // Start from a lit sample so the walk cannot begin mid-run and count one
    // run as two.
    let from = profile.iter().position(|b| *b).expect("something lit to measure from");
    let mut runs = Vec::new();
    let mut run = 0;
    for k in 0..PROFILE_STEPS {
        if profile[(from + k) % PROFILE_STEPS] {
            if run > 0 {
                runs.push(run as f32 * step);
                run = 0;
            }
        } else {
            run += 1;
        }
    }
    if run > 0 {
        runs.push(run as f32 * step);
    }
    runs
}

/// The invariant the wheel is built around, checked on the picture rather
/// than on the layout that feeds it: every octave of the span gets an
/// indicator and together they close the ring — whatever the counts, the
/// center, the fringe or the node's pitch class. So the only unlit stretches
/// are the Gap setting's slits, one per boundary, and the seam is one of them
/// on every node, wherever that node's turn has carried it.
///
/// Reading it off rendered pixels is the point. The layout's own tests pin
/// the angles down; this one says the shader draws the axis the table
/// describes, in the right direction, anchored where it claims — and in
/// particular that it draws the end indicators out to the seam, which is what
/// closes the ring when the window is not a whole number of octaves.
#[test]
fn every_octave_in_the_range_is_drawn_and_they_close_the_ring() {
    use harmonigraph_scene::octave_layout;

    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [512, 512];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vec_size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    let mut shot = |scene: &Scene, pane_id: u64| -> Vec<u8> {
        let cb = LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            vec_size,
            format,
            pane_id,
            None,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let tex = render_to_texture(&device, &queue, SIZE, format, wgpu::Color::BLACK, |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: 1.0,
                    screen_size_px: SIZE,
                },
                pass,
                &resources,
            );
        });
        readback(&device, &queue, &tex, SIZE)
    };

    // The widest wheel, the default, an even count — where the ring reaches an
    // octave further on one side — a center that is neither a C nor near the
    // middle of the keyboard, where the ring names octaves the packing has no
    // room for, and three fringed wheels: the narrowest there is (where the
    // one full-size octave takes a sector widest and the union branch of the
    // wedge test is stressed hardest), a plain register with a pair either
    // side, and a deep fringe filling the budget. Each at a C node (whose
    // octaves land flush on the center) and at three pitch classes that do
    // not, one of them the tritone that turns furthest.
    //
    // An even wheel, a flat fringe, a graded one, and then a fringe thin
    // enough to be eaten by the Gap.
    const FRINGES: [(f32, f32); 4] = [(1.0, 0.0), (0.6, 0.0), (0.6, 1.0), (0.15, 0.0)];
    let mut pane = 60;
    for (count, extras, center) in [
        (11u32, 0u32, 60.0f32),
        (5, 0, 60.0),
        (4, 0, 60.0),
        (5, 0, 103.0),
        (1, 1, 60.0),
        (5, 2, 60.0),
        (3, 4, 60.0),
    ] {
        for (i, &(size, blend)) in FRINGES.iter().enumerate() {
            // Both fringe settings are inert without extras, so the other
            // three would render the same picture at four times the cost.
            if extras == 0 && i > 0 {
                continue;
            }
            for cents in [0.0, 350.0, 600.0, 1150.0] {
                let layout = octave_layout(count, center, extras, size, blend);
                let px = shot(&octave_wheel_scene(layout, cents), pane);
                pane += 1;
                let profile = band_profile(&px, SIZE[0]);
                let case = format!(
                    "{count}+2x{extras} at {center}, size {size} blend {blend}, {cents}c"
                );

                // One indicator per octave of the wheel, closing the ring:
                // that is one slit per boundary and no other break. A missing
                // indicator merges two slits into one hole, so the count is
                // what says all of them are there — including the ones drawn
                // for octaves no note can reach.
                let want = layout.span as usize;
                let runs = unlit_runs(&profile);
                // Except under a thin fringe, and that is the settings talking
                // rather than a missing indicator: the Gap is cut out of every
                // sector from both sides at full width, so an extra thinner
                // than twice that padding has its two slits meet and reads as
                // no indicator at all. At 0.6 of an even slice they still
                // resolve; 0.15 is where they go, and only the extras are ever
                // that thin. `octaves.rs` pins the count exactly, on angles,
                // where no padding is involved.
                if size >= 0.4 {
                    assert_eq!(runs.len(), want, "{case}: unlit runs {runs:?} for {want} sectors");
                } else {
                    let lost = 2 * extras as usize;
                    assert!(
                        runs.len() + lost >= want && runs.len() <= want,
                        "{case}: unlit runs {runs:?} for {want} sectors — at most the \
                         {lost} extras can be lost to the Gap"
                    );
                }

                // The seam TURNS with the node: it is the bottom only for the
                // center's own pitch class, and every other class carries it
                // round by however far its octaves sit from the center. Read
                // off the layout rather than at a fixed 270 degrees, which is
                // the whole difference between this wheel and a window.
                let seam = layout.ring(cents).seam.to_degrees().rem_euclid(360.0);
                assert!(gap_at(&profile, seam) > 0.0, "{case}: no seam at {seam:.1} deg");

                // The CENTER pitch is straight up on every node, so a slice
                // covers the top — except on the node exactly a tritone from
                // it, where the center is a boundary and a slit there is the
                // axis being read rather than a hole.
                let tritone = (cents - 600.0).abs() < 1e-3;
                if !tritone {
                    assert!(gap_at(&profile, 90.0) == 0.0, "{case}: nothing covers the top");
                }
            }
        }
    }
}

/// Which PITCH each indicator is drawn at — the whole of what "positioned by
/// absolute pitch" means, and the part a seam test cannot see. One octave
/// sounds; the bright arc has to land where the layout puts that pitch, and
/// a node's pitch class has to move it.
#[test]
fn an_indicator_is_drawn_at_its_own_pitchs_angle() {
    use harmonigraph_scene::octave_layout;

    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [512, 512];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vec_size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };

    let mut pane = 100;
    for (count, extras, center, size, blend) in
        [(5u32, 0u32, 60.0f32, 1.0, 0.0), (8, 1, 66.0, 0.3, 0.0)]
    {
        let layout = octave_layout(count, center, extras, size, blend);
        // A C node and a node a fifth up: same slot, pitches 7 semitones
        // apart, so the bright arc must move by exactly that much of the axis.
        // The octave holding the center pitch, and one further round the
        // wheel, where a wrong anchor or a wrong direction shows.
        //
        // Both held INSIDE the ring rather than at its edges: a thin fringe
        // leaves the extras narrower than the Gap's slits, and a centroid
        // needs an arc to measure. That the edges reach the seam at all is
        // `every_octave_in_the_range_is_drawn_and_they_close_the_ring`.
        for (cents, offset) in [(0.0f32, 0i32), (700.0, 0), (0.0, 2), (700.0, 2)] {
            let (first, last) = layout.slots(cents);
            let slot =
                (harmonigraph_scene::MIDDLE_C_SLOT as i32 + offset).clamp(first + 1, last - 1);
            let mut scene = octave_wheel_scene(layout, cents);
            // One octave sounding. The silent slots still ghost in behind it
            // at GHOST_LEVEL, which the brightness threshold below sorts out.
            scene.nodes[0].octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
            scene.nodes[0].octaves[slot as usize] = 1.0;

            let cb = LatticeCallback::from_scene(
                &scene,
                LatticeLabels::default(),
                vec_size,
                format,
                pane,
                None,
            );
            pane += 1;
            let mut encoder = device.create_command_encoder(&Default::default());
            let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
            queue.submit(bufs.into_iter().chain([encoder.finish()]));
            let tex =
                render_to_texture(&device, &queue, SIZE, format, wgpu::Color::BLACK, |pass| {
                    cb.paint(
                        egui::PaintCallbackInfo {
                            viewport: rect,
                            clip_rect: rect,
                            pixels_per_point: 1.0,
                            screen_size_px: SIZE,
                        },
                        pass,
                        &resources,
                    );
                });
            let px = readback(&device, &queue, &tex, SIZE);

            // Where the BRIGHT pixels are, as a mean direction. The lit
            // indicator runs several times the ghosts' level, so half the
            // maximum separates them cleanly whatever the node color is.
            let bright = |i: usize| px[i] as u32 + px[i + 1] as u32 + px[i + 2] as u32;
            let peak = (0..px.len() / 4).map(|k| bright(k * 4)).max().unwrap_or(0);
            assert!(peak > 60, "{count}+2x{extras} at {center}: nothing bright enough");
            let (mut vx, mut vy) = (0f64, 0f64);
            let c = SIZE[0] as f64 / 2.0;
            for y in 0..SIZE[1] {
                for x in 0..SIZE[0] {
                    let i = ((y * SIZE[0] + x) * 4) as usize;
                    if bright(i) > peak / 2 {
                        // Screen y grows downward; flip it for an ordinary angle.
                        vx += x as f64 - c;
                        vy += c - y as f64;
                    }
                }
            }
            let drawn = vy.atan2(vx).to_degrees() as f32;
            // The indicator's own middle, from the layout: the pitch halfway
            // between its two edges in ANGLE, which a fringe can shift off the
            // pitch itself.
            let (e0, e1) = layout.sector(slot, cents);
            let expected = (0.5 * (e0 + e1)).to_degrees().rem_euclid(360.0);
            let off = (drawn.rem_euclid(360.0) - expected).rem_euclid(360.0);
            let off = off.min(360.0 - off);
            assert!(
                off < 6.0,
                "{count}+2x{extras} at {center}, {cents}c, slot {slot}: indicator drawn \
                 at {drawn:.1} deg, the axis puts its pitch at {expected:.1}"
            );
        }
    }
}

/// The seams between a chord's colors have to soften WITH the core. They are
/// laid down as lobes of fixed ANGULAR width, so the arc each one spans shrinks
/// with the radius and they meet in a cusp at the node's centre — a core dialed
/// soft would otherwise blur at its rim and stay knife-sharp in the middle,
/// which is the one place its edge softening cannot reach.
///
/// Both halves of the bargain, because either alone has a trivial cheat: the
/// centre has to lose its seam, AND the rim has to keep its colors, which is
/// what stops the cure from being "average the whole node". Run over the
/// shipped default view as well as a deliberately soft one, so a retune cannot
/// move the out-of-the-box look with the suite green.
///
/// Measured as how far the colors around a ring point APART as directions, not
/// as how much they differ: a soft core is also a dimmer one, and any measure
/// of magnitude would read that dimming as a blur and pass on it.
#[test]
fn a_soft_core_blurs_the_seams_between_its_colors_at_the_centre() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [512, 512];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vec_size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };

    let mut pane = 300;
    let mut shot = |solidity: f32, core_radius: f32| -> Vec<u8> {
        let mut scene = single_marked_node(0, 0);
        scene.core_solidity = solidity;
        // The disc held at one size on screen whatever fraction of the quad it
        // is, so both cases below are measured at the same pixel scale. What
        // the seam floor asks for is a fraction of the core radius, so the
        // profile it produces is the same shape at any size; keeping the disc
        // big just leaves the `aa` floor — the one absolute term — too small to
        // be what these readings are about.
        scene.node_radius *= 0.46 / core_radius;
        scene.core_radius = core_radius;
        // Every octave the wheel draws for this pitch class. A single sounding
        // voice takes the node's own color everywhere (octave_glow_color's solo
        // fallback), which leaves no seam to measure at all.
        let layout = scene.octave_layout;
        let node = &mut scene.nodes[0];
        let (low, high) = layout.slots(node.cents);
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        for slot in low.max(0)..=high.min(harmonigraph_scene::OCTAVE_SLOTS as i32 - 1) {
            node.octaves[slot as usize] = 1.0;
        }
        let labels = LatticeLabels::default();
        let cb = LatticeCallback::from_scene(&scene, labels, vec_size, format, pane, None);
        pane += 1;
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let tex = render_to_texture(&device, &queue, SIZE, format, wgpu::Color::BLACK, |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: 1.0,
                    screen_size_px: SIZE,
                },
                pass,
                &resources,
            );
        });
        readback(&device, &queue, &tex, SIZE)
    };

    // The node is alone at the world origin and the camera looks at it, so the
    // frame's centre is its centre.
    let c = (SIZE[0] / 2) as i32;
    let rgb = |px: &[u8], x: i32, y: i32| -> glam::Vec3 {
        let i = ((y as u32 * SIZE[0] + x as u32) * 4) as usize;
        glam::Vec3::new(px[i] as f32, px[i + 1] as f32, px[i + 2] as f32) / 255.0
    };
    // How far apart, in degrees, the most divergent pair of colors around a
    // ring of radius `r` point. Zero is one flat color all the way round.
    let spread = |px: &[u8], r: f32| -> f32 {
        let dirs: Vec<glam::Vec3> = (0..64)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / 64.0;
                // Screen y grows downward; the sample angle is negated.
                rgb(px, c + (r * a.cos()).round() as i32, c - (r * a.sin()).round() as i32)
            })
            .filter(|v| v.length() > 0.02)
            .map(|v| v.normalize())
            .collect();
        let lit = dirs.len();
        assert!(lit > 56, "the ring at r={r:.0} is not on the node: {lit} lit samples of 64");
        let mut worst = 0.0f32;
        for (i, a) in dirs.iter().enumerate() {
            for b in &dirs[i + 1..] {
                worst = worst.max(a.dot(*b).clamp(-1.0, 1.0).acos().to_degrees());
            }
        }
        worst
    };

    // The disc's radius in pixels, read off the SOLID end, where its edge is
    // the first sharp fall from the centre; at the soft end it has dissolved
    // into the glow and there is no edge to find. It is the same disc either
    // way, so one reading sizes every ring. Taken as the median of eight rays
    // rather than one: a single ray's answer depends on which octave's hue
    // happens to lie along it and how bright that one is, so a fixture whose
    // colors moved would quietly resize the rings instead of failing.
    let disc_radius = |px: &[u8]| -> f32 {
        let ray = |a: f32| -> f32 {
            let at = |r: f32| rgb(px, c + (r * a.cos()) as i32, c - (r * a.sin()) as i32).length();
            let centre = at(0.0);
            (2..c).map(|r| r as f32).find(|r| at(*r) < centre * 0.4).unwrap_or(c as f32)
        };
        let mut rs: Vec<f32> = (0..8)
            .map(|i| ray(std::f32::consts::TAU * i as f32 / 8.0))
            .collect();
        rs.sort_by(f32::total_cmp);
        rs[4]
    };

    for (name, solidity, core_radius) in
        [("the shipped default view", 0.4f32, 0.2f32), ("a core dialed soft", 0.25, 0.46)]
    {
        let solid = shot(1.0, core_radius);
        let soft = shot(solidity, core_radius);
        let r_disc = disc_radius(&solid);
        assert!(r_disc > 20.0, "{name}: the disc is too small to sample rings in ({r_disc} px)");

        let (inner, outer) = (r_disc * 0.2, r_disc * 0.75);
        let solid_in = spread(&solid, inner);
        let solid_out = spread(&solid, outer);
        let soft_in = spread(&soft, inner);
        let soft_out = spread(&soft, outer);
        // The cusp this is about: at full solidity the hues stay fully
        // separated right into the middle. Without it the rest measures
        // nothing.
        assert!(solid_in > 30.0, "{name}: no seam at the centre to soften: {solid_in:.0} deg");
        assert!(
            soft_in < solid_in * 0.5,
            "{name}: a soft core's centre is still a hard seam: {soft_in:.0} deg across, \
             against {solid_in:.0} at full solidity"
        );
        // And what stops the cure being "average the node": the seams are never
        // held wider than the arc they already span at the rim, so out there
        // the colors are as separated as the solid orb's — measured against
        // THAT, not against the blurred centre, which would pass a node washed
        // to one color.
        assert!(
            soft_out > solid_out * 0.8,
            "{name}: the soft core lost the colors it should only have blurred: \
             {soft_out:.0} deg at the rim against the solid orb's {solid_out:.0}"
        );
    }
}

#[test]
fn bloom_adds_light_over_the_plain_composite() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [256, 256];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vec_size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);
    let clear = wgpu::Color::BLACK;

    let scene_off = parity_scene();
    let mut scene_on = parity_scene();
    scene_on.bloom_strength = 1.0;

    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor {
        size_in_pixels: SIZE,
        pixels_per_point: 1.0,
    };
    let mut total = |scene: &Scene, pane_id: u64| -> u64 {
        let cb = LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            vec_size,
            format,
            pane_id,
            None,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let tex = render_to_texture(&device, &queue, SIZE, format, clear, |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: 1.0,
                    screen_size_px: SIZE,
                },
                pass,
                &resources,
            );
        });
        readback(&device, &queue, &tex, SIZE)
            .chunks(4)
            .map(|px| u64::from(px[0]) + u64::from(px[1]) + u64::from(px[2]))
            .sum()
    };

    let plain = total(&scene_off, 31);
    let bloomed = total(&scene_on, 32);
    assert!(
        bloomed > plain + plain / 20,
        "bloom at strength 1.0 should add clearly visible light: \
         plain {plain} vs bloomed {bloomed}"
    );
}













/// A sheet in FRONT of the home sheet is drawn over it; a sheet BEHIND it
/// is drawn under. Both directions matter, and only one of them is obvious:
/// forcing the home sheet to the bottom (so an off-sheet note could never be
/// hidden by it) inverts the far half of the axis, and since the knockout
/// gutter clears whatever was drawn before it, the sheet behind then takes a
/// bite out of the home sheet in front of it.
#[test]
fn sheets_draw_back_to_front_along_the_sevens_axis() {
    use harmonigraph_scene::{Camera, FrameParams, Projection, ViewConfig};

    let view = ViewConfig {
        extent_threes: 1,
        extent_fives: 1,
        extent_sevens: 2,
        // The pale trail ring, which draws with no idle marker and on any
        // sheet — the one mark that can put something at EVERY position.
        // An idle marker cannot: it reaches only the home sheet or a visited
        // node (`idle_marker` in lattice.wgsl), and trails are recorded on
        // the home sheet alone (`TrailField::apply`), so with one of those
        // the cull leaves a single sheet and the order below is one depth
        // compared against itself.
        trail_mark: harmonigraph_scene::TrailMark::Ring,
        trail_strength: 1.0,
        ..ViewConfig::default()
    };
    for projection in [Projection::Cabinet, Projection::Perspective, Projection::Orthographic]
    {
        let mut scene = harmonigraph_scene::derive_scene(
            &harmonigraph_core::NoteTracker::new(),
            &harmonigraph_core::Tuning::default(),
            &view,
            &FrameParams::default(),
            // Orbited, deliberately: this is the case a plain depth sort
            // gets wrong, because two nodes on one sheet then sit at
            // different depths and the sheets interleave.
            Camera { projection, ..Camera::default() },
            None,
            0.0,
        );
        // Every position visited. Set here rather than played in, because
        // what this test needs is one node per position on every sheet, and
        // which nodes a tracker lights is a question about tuning.
        for node in &mut scene.nodes {
            node.trail = 1.0;
        }
        let call = LatticeCallback::from_scene(
            &scene,
            LatticeLabels::default(),
            egui::vec2(800.0, 600.0),
            wgpu::TextureFormat::Bgra8Unorm,
            0,
            // No stats slot: this is about draw ORDER, not about timing.
            None,
        );
        // World z IS the sevens axis (see lattice_to_world), so the draw
        // order must run from the most negative sheet to the most positive
        // — and it has to hold under EVERY projection, not only the face-on
        // one. When it doesn't, the sheets interleave, the grid lands in
        // the wrong place in the order, and the home sheet's clearings have
        // nothing drawn before them left to clear.
        let depths: Vec<f32> = call.instances.iter().map(|i| i.world_pos[2]).collect();
        // Several SHEETS, not several nodes. A node count passes on one
        // sheet's worth of identical depths, where every pair below holds
        // whatever the sort did — which is what culling the off-sheet nodes
        // reduced this to, silently, while it went on reading as coverage.
        let (lo, hi) = depths.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &d| {
            (lo.min(d), hi.max(d))
        });
        assert!(
            hi - lo > 1e-6,
            "{projection:?}: every node drawn is at one depth ({lo}), so the order \
             below compares a sheet with itself: {depths:?}",
        );
        for pair in depths.windows(2) {
            assert!(
                pair[1] >= pair[0] - 1e-6,
                "{projection:?}: a sheet behind is drawn after one in front: {pair:?}"
            );
        }
    }
}

/// Every node idle: no note, no marks, no octaves — the state most of a
/// lattice is in most of the time, and the one the fragment shader's idle
/// branch takes. Markers and trails ON, since they are the only thing an
/// idle node paints and therefore the only thing that branch has to get
/// right.
fn idle_scene() -> Scene {
    let mut scene = parity_scene();
    for (i, node) in scene.nodes.iter_mut().enumerate() {
        node.activation = 0.0;
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.melody_slots = 0;
        node.bass_slots = 0;
        node.melody_level = 0.0;
        node.bass_level = 0.0;
        node.hovered = false;
        node.on_home = i % 2 == 0;
        // Visited on a DIFFERENT cycle from the home sheet, so the two are
        // separable: `idle_marker` shows where the node is home OR visited,
        // and a fixture whose visited set is its home set makes that
        // disjunction untestable — either branch alone reproduces it, and so
        // does the complement of either. Node 3 is the one that matters: off
        // the home sheet and visited, so it draws for exactly one reason.
        node.trail = if i % 3 == 0 { 0.8 } else { 0.0 };
    }
    scene.idle_marker = harmonigraph_scene::IdleMarker::Circle;
    scene.idle_radius = 0.24;
    scene.trail_mark = harmonigraph_scene::TrailMark::Ring;
    scene.trail_strength = 1.0;
    scene
}

/// The fragment shader's early-outs — skipping the fragments outside
/// anything a node can paint, and the whole note path for an idle node —
/// must be exactly that: an optimization. Same scene through the real
/// shader and through one compiled with `EARLY_OUT` off, pixel for pixel.
///
/// Worth a GPU test rather than a reading of the code, because the bound in
/// `paint_reach` is a claim about EVERY layer's falloff at once: add a layer
/// that reaches further, or widen one's soft edge past its radius, and the
/// only symptom is a quietly clipped halo somewhere off the node.
#[test]
fn the_fragment_early_outs_do_not_change_a_pixel() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [256, 256];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let reference_src = SHADER_SRC.replace(
        "const EARLY_OUT: bool = true;",
        "const EARLY_OUT: bool = false;",
    );
    assert_ne!(
        reference_src, SHADER_SRC,
        "the EARLY_OUT switch was renamed; this test is no longer comparing anything",
    );

    // A core dialed near the top of its bar, with the solidity axis in the
    // middle. Every other fixture here runs the default 0.46 radius at full
    // solidity, where `core_reach` is GLOW_LIMIT and the radius arm of it is
    // never the larger — so collapsing that bound to the constant passes.
    // At 0.9 the disc still carries alpha where the glow's window has closed,
    // and clipping it there is a hard circular cut across a soft core.
    let fat_core = || {
        let mut scene = parity_scene();
        scene.core_radius = 0.9;
        scene.core_solidity = 0.5;
        scene
    };
    // Both layers shimmering, at one fixed instant. `paint_reach` is where
    // the claim that shimmer keeps every bound exact has to be checked, and
    // it can only be checked here: the shimmer scales two layers' coverage,
    // and a term that ever came out ABOVE 1 would push a layer past the reach
    // the early-out proved it could not cross — visible as a ring clipped
    // flat in the fast pipeline alone, which no other fixture would catch
    // because every other one leaves both pulses Off.
    let shimmering = || {
        let mut scene = parity_scene();
        scene.pulse_octaves = harmonigraph_scene::Pulse::Shimmer;
        scene.pulse_marks = harmonigraph_scene::Pulse::Shimmer;
        scene
    };
    for (name, scene) in [
        ("lit", parity_scene()),
        ("idle", idle_scene()),
        ("fat core", fat_core()),
        ("shimmering", shimmering()),
    ] {
        let cb = LatticeCallback::from_scene(
            &scene,
            LatticeLabels::default(),
            egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
            format,
            11,
            None,
        );
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));

        let res: &LatticeResources = resources.get().expect("prepare created resources");
        let (fast, _) = create_pipelines(&device, SHADER_SRC, format, &res.bind_group_layout, false);
        let (slow, _) =
            create_pipelines(&device, &reference_src, format, &res.bind_group_layout, false);
        let pane = res.panes.get(&11).expect("prepare created the pane");

        let clear = wgpu::Color { r: 0.07, g: 0.08, b: 0.09, a: 1.0 };
        let draw = |pipeline: &wgpu::RenderPipeline| {
            let texture = render_to_texture(&device, &queue, SIZE, format, clear, |pass| {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                pass.draw(0..4, 0..pane.instance_count);
            });
            readback(&device, &queue, &texture, SIZE)
        };
        let with_early_out = draw(&fast);
        let without = draw(&slow);

        // Guard against a vacuous pass: the nodes must have drawn something
        // over the clear color.
        let bg = [18u8, 20, 23, 255];
        assert!(
            without.chunks(4).any(|px| px.iter().zip(bg).any(|(&c, b)| c.abs_diff(b) > 8)),
            "the {name} scene drew nothing; the comparison is vacuous",
        );

        let differing = with_early_out
            .iter()
            .zip(&without)
            .enumerate()
            .find(|(_, (&a, &b))| a != b);
        assert!(
            differing.is_none(),
            "the {name} scene changed when the early-outs were enabled: byte {:?}",
            differing.map(|(i, (a, b))| (i, *a, *b)),
        );
    }
}

/// A node that can paint nothing is not shipped at all — and the grid it
/// sits in still is.
///
/// The billboard is deliberately bigger than the node, so a node the shader
/// discards every fragment of still costs a quad's worth of rasterizing; on
/// an unplayed lattice that is nearly every node. With the default idle
/// marker (None) and trails off it is ALL of them, which is the case worth
/// pinning: the frame drops to a grid and nothing else, and the callback
/// has to keep drawing that grid, which is why neither `prepare` nor `paint`
/// may read "no instances" as "nothing to draw": that test takes the grid
/// down with the nodes.
#[test]
fn a_silent_lattice_ships_no_nodes_and_still_draws_its_grid() {
    let scene = {
        let mut scene = idle_scene();
        // What a fresh view opens at: nothing to show at an unplayed node.
        scene.idle_marker = harmonigraph_scene::IdleMarker::None;
        scene.trail_mark = harmonigraph_scene::TrailMark::Off;
        scene
    };
    assert!(!scene.grid.is_empty(), "the fixture has to carry a grid");
    let cb = LatticeCallback::from_scene(
        &scene,
        LatticeLabels::default(),
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        31,
        None,
    );
    assert!(
        cb.instances.is_empty(),
        "every node is idle with nothing to draw at one, so none should ship",
    );
    assert!(!cb.edges.is_empty(), "the grid is not a node and must survive");

    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [256, 256];
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    let mut encoder = device.create_command_encoder(&Default::default());
    let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
    queue.submit(bufs.into_iter().chain([encoder.finish()]));
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(256.0, 256.0));
    let clear = wgpu::Color { r: 0.07, g: 0.08, b: 0.09, a: 1.0 };
    let texture = render_to_texture(&device, &queue, SIZE, format_of(&cb), clear, |pass| {
        cb.paint(
            egui::PaintCallbackInfo {
                viewport: rect,
                clip_rect: rect,
                pixels_per_point: 1.0,
                screen_size_px: SIZE,
            },
            pass,
            &resources,
        );
    });
    let px = readback(&device, &queue, &texture, SIZE);
    let bg = [18u8, 20, 23, 255];
    assert!(
        px.chunks(4).any(|p| p.iter().zip(bg).any(|(&c, b)| c.abs_diff(b) > 4)),
        "the grid vanished with the nodes",
    );
}

/// The target format the callback was built for, so the test above renders
/// into the one its composite pipeline expects.
fn format_of(cb: &LatticeCallback) -> wgpu::TextureFormat {
    cb.target_format
}

/// What brings a silent node back: the two marks an idle node can wear.
/// Each is read off a different uniform by a different branch of
/// `idle_marker`, so each needs its own case — the cull has to ask the same
/// question the shader does, and a cull that only knew about one of them
/// would blank the other with no symptom but a missing mark.
///
/// Asserted as the SET of positions kept, not how many. A count cannot tell
/// a predicate from its complement: `idle_scene` has three home nodes, and a
/// cull that shipped the three off-home ones instead would agree with every
/// count this could make.
#[test]
fn an_idle_marker_or_a_trail_ring_keeps_its_nodes() {
    let ships = |scene: &Scene| {
        let kept = LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            egui::vec2(256.0, 256.0),
            wgpu::TextureFormat::Rgba8Unorm,
            32,
            None,
        )
        .instances;
        // The home flag and the memory are what the predicate reads, so they
        // are what identifies a kept node here.
        let mut ids: Vec<(bool, bool)> =
            kept.iter().map(|g| (g.home >= 0.5, g.visited > 0.0)).collect();
        ids.sort();
        ids
    };
    let of = |f: fn(&harmonigraph_scene::NodeInstance) -> bool| {
        let mut ids: Vec<(bool, bool)> = idle_scene()
            .nodes
            .iter()
            .filter(|n| f(n))
            .map(|n| (n.on_home, n.trail > 0.0))
            .collect();
        ids.sort();
        ids
    };
    // The fixture separates the two: home and visited are different sets, and
    // node 3 is visited OFF the home sheet, which is the only node that can
    // tell the marker's `home || trail` disjunction from its `home` half.
    assert_ne!(of(|n| n.on_home), of(|n| n.trail > 0.0), "the two sets must differ");
    assert!(
        idle_scene().nodes.iter().any(|n| !n.on_home && n.trail > 0.0),
        "one node has to be visited off the home sheet",
    );

    // The marker alone: it shows where the node is home OR carries a memory.
    let mut marker_only = idle_scene();
    marker_only.trail_mark = harmonigraph_scene::TrailMark::Off;
    // Trails Off zeroes the memory the marker would also have shown on, so
    // this is the home sheet exactly.
    assert_eq!(
        ships(&marker_only),
        of(|n| n.on_home),
        "with trails off the idle marker draws on the home sheet",
    );

    // The marker with trails on reaches one node further: the off-sheet one
    // the music visited. That node is the whole of the disjunction.
    let mut marker_and_trail = idle_scene();
    marker_and_trail.trail_mark = harmonigraph_scene::TrailMark::Lift;
    assert_eq!(
        ships(&marker_and_trail),
        of(|n| n.on_home || n.trail > 0.0),
        "a visited node keeps its marker off the home sheet",
    );

    // The pale ring alone: it draws with the marker off, on what was visited,
    // wherever that is.
    let mut ring_only = idle_scene();
    ring_only.idle_marker = harmonigraph_scene::IdleMarker::None;
    assert_eq!(
        ships(&ring_only),
        of(|n| n.trail > 0.0),
        "the trail ring draws on visited nodes, home sheet or not",
    );

    // And a strength of zero is a ring that isn't there.
    let mut faded = idle_scene();
    faded.idle_marker = harmonigraph_scene::IdleMarker::None;
    faded.trail_strength = 0.0;
    assert!(ships(&faded).is_empty(), "a ring at zero strength paints nothing");
}

/// One way of making a node sound, for the table in the test below.
type LightUp = fn(&mut harmonigraph_scene::NodeInstance);

/// Each of the four things that make a node sounding keeps it on its own.
///
/// The cull's first question is whether anything is lit — an envelope, a
/// melody or bass ring's level, or a lit octave — and it is a disjunction
/// over four terms that ordinarily move together: `derive_scene` rides all
/// of them on one envelope, so a scene built by playing notes has either
/// none of them or several, and no single term ever decides whether a node
/// ships. That is not a hypothetical gap. Inverting any one of the four,
/// or turning any `||` between them into `&&`, passed the whole suite.
///
/// They come apart in practice, which is why each gets a node here. An
/// octave's level is `envelope * attack(...)` and is packed to a byte, so
/// for the first frames after a note-on a node has a full activation and an
/// octave word of exactly zero — `params[0]` alone is holding it in the
/// buffer, and a cull that stopped reading it would drop the first frame of
/// a note. The mark levels ride their own ease-in (`melody_attack`) rather
/// than the node's, so they part company the same way.
///
/// Built by hand rather than played in: the point is one term at a time,
/// and a tracker cannot be asked for that.
#[test]
fn each_thing_that_makes_a_node_sounding_keeps_it_alone() {
    let bare = || {
        let mut scene = idle_scene();
        // Nothing an IDLE node could draw, so the only reason to keep one is
        // the term under test.
        scene.idle_marker = harmonigraph_scene::IdleMarker::None;
        scene.trail_mark = harmonigraph_scene::TrailMark::Off;
        for node in &mut scene.nodes {
            node.trail = 0.0;
        }
        scene
    };
    let ships = |scene: &Scene| {
        LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            egui::vec2(256.0, 256.0),
            wgpu::TextureFormat::Rgba8Unorm,
            33,
            None,
        )
        .instances
        .len()
    };
    assert_eq!(ships(&bare()), 0, "the fixture has to start with nothing drawn");

    // One node per term, each the ONLY lit thing about it.
    let cases: [(&str, LightUp); 4] = [
        ("activation", |n| n.activation = 1.0),
        ("melody level", |n| n.melody_level = 1.0),
        ("bass level", |n| n.bass_level = 1.0),
        ("a lit octave", |n| n.octaves[harmonigraph_scene::MIDDLE_C_SLOT] = 1.0),
    ];
    for (what, set) in cases {
        let mut scene = bare();
        set(&mut scene.nodes[0]);
        assert_eq!(ships(&scene), 1, "{what} alone has to keep its node");
    }

    // The octave levels pack into three u32s, four slots to a word, and the
    // three are OR'd. Two octaves of one pitch class held at the same level
    // is an ordinary voicing, and it puts the SAME byte in two words — where
    // anything but an OR cancels them against each other and reads the node
    // as unlit. Every pairing of words is needed, because which two cancel
    // depends on where the operator lands and how it then binds: `^` binds
    // tighter than `|`, so swapping either one regroups the whole expression
    // as well as changing it.
    for (a, b) in [(0usize, 4usize), (0, 8), (4, 8)] {
        let mut spread = bare();
        spread.nodes[0].octaves[a] = 1.0;
        spread.nodes[0].octaves[b] = 1.0;
        assert_eq!(
            ships(&spread),
            1,
            "octaves {a} and {b} held at one level keep their node",
        );
    }
}

/// The grid's place in the draw order is counted over the nodes actually
/// shipped, not over the ones the scene held.
///
/// `grid_at` is the seam between the sheets BEHIND the home sheet and the
/// home sheet itself, and the whole argument for it is in `from_scene`: put
/// the grid under everything and a node on a sheet behind the home one
/// punches its clearing through the home grid. Culling breaks the old
/// expression silently, because `split` indexes the list before the cull —
/// with the sheets behind home mostly idle, `split` runs past the end of the
/// kept run, `prepare`'s `.min(instance_count)` pins the grid to the very
/// end, and it draws over every node instead of under the home sheet.
///
/// One lit node behind home and one on it, with an idle node behind home
/// between them: the seam has to land at 1, and it is `split` (2) that says
/// otherwise.
#[test]
fn the_grid_seam_counts_the_nodes_that_ship() {
    let mut scene = idle_scene();
    scene.idle_marker = harmonigraph_scene::IdleMarker::None;
    scene.trail_mark = harmonigraph_scene::TrailMark::Off;
    scene.nodes.truncate(3);
    for node in &mut scene.nodes {
        node.trail = 0.0;
        node.activation = 0.0;
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
    }
    // Which world z is "behind" is the camera's to say — the sort keys on
    // `world_pos.z * forward.z` — and the default camera's eye sits at +z, so
    // the sheets behind the home one are the negative side.
    scene.nodes[0].world_pos.z = -2.0;
    scene.nodes[0].activation = 1.0; // behind home, lit: ships, before the grid
    scene.nodes[1].world_pos.z = -1.0; // behind home, idle: culled
    scene.nodes[2].world_pos.z = 0.0;
    scene.nodes[2].activation = 1.0; // the home sheet: ships, after the grid
    scene.nodes[2].on_home = true;

    let call = LatticeCallback::from_scene(
        &scene,
        LatticeLabels::default(),
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        34,
        None,
    );
    assert_eq!(call.instances.len(), 2, "the idle node behind home is culled");
    assert_eq!(
        call.grid_at, 1,
        "the grid draws after the one sheet-behind node that ships, not after \
         the two the scene held",
    );
}

/// A lattice that stops drawing stops reporting a GPU time, rather than
/// holding the last one it measured.
///
/// The reading is a cross-frame value: it is only overwritten when the
/// timer's readback cycle turns over, and that cycle is driven from inside
/// the scene pass. Since nodes that paint nothing are no longer shipped, a
/// lattice can encode no pass at all — with the grid's alpha at zero, no
/// idle marker and no trail, a silent one ships neither a node nor an edge —
/// and it can sit there indefinitely. Left alone, the overlay would keep
/// re-averaging a figure from whenever the lattice last drew, which is the
/// one thing `GPU_TIME_PENDING` exists to make impossible to confuse with a
/// live reading.
#[test]
fn a_lattice_with_nothing_to_draw_reports_no_gpu_time() {
    let Some((device, queue)) = headless_device_with_timestamps() else {
        return;
    };
    const SIZE: [u32; 2] = [128, 128];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let stats = std::sync::Arc::new(LatticeStats::default());
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    let mut frame = |cb: &LatticeCallback| {
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
    };

    // Draw a real scene until a measurement lands. The cycle is
    // Idle -> Recorded -> Mapping -> Idle, so it takes a few frames.
    let lit = LatticeCallback::from_scene(
        &parity_scene(),
        LatticeLabels::default(),
        size,
        format,
        40,
        Some(stats.clone()),
    );
    let mut measured = None;
    for _ in 0..12 {
        frame(&lit);
        let bits = stats.gpu_ms.load(std::sync::atomic::Ordering::Relaxed);
        if bits != GPU_TIME_PENDING && bits != GPU_TIME_UNSUPPORTED {
            measured = Some(bits);
            break;
        }
    }
    let Some(measured) = measured else {
        // No reading ever landed, so there is no stale value to go stale.
        return;
    };
    assert!(f32::from_bits(measured) >= 0.0, "a real reading, not a sentinel");

    // Now the same pane with nothing at all in it.
    let mut empty = idle_scene();
    empty.idle_marker = harmonigraph_scene::IdleMarker::None;
    empty.trail_mark = harmonigraph_scene::TrailMark::Off;
    empty.grid.clear();
    let blank = LatticeCallback::from_scene(
        &empty,
        LatticeLabels::default(),
        size,
        format,
        40,
        Some(stats.clone()),
    );
    assert!(blank.instances.is_empty() && blank.edges.is_empty(), "nothing to draw");
    frame(&blank);
    assert_eq!(
        stats.gpu_ms.load(std::sync::atomic::Ordering::Relaxed),
        GPU_TIME_PENDING,
        "a pane that encodes no pass must not keep reporting the time it took \
         when it last drew",
    );
}

/// Where the pair of nodes below stands, in world units. Off-center in both
/// axes on purpose: a label is drawn into the pane's own pass, and a mapping
/// that flipped or transposed that pane would still land on the right pixel
/// in the middle of the picture.
const STACK_AT: glam::Vec2 = glam::Vec2::new(0.7, -0.5);

/// The pane this scene is drawn into. Bigger than the text fixtures, because
/// this one is about a node's DISC rather than about a glyph: at the real
/// ratio of node radius to lattice spacing, 64 points across puts the whole
/// disc inside a couple of pixels.
const SCENE_SIZE: [u32; 2] = [256, 256];

/// Two nodes on the same pixels, one sevens step apart, and nothing else.
/// Face-on and orthographic, which is the arrangement that puts one node
/// squarely behind another; a sheared or orbited view only spreads the same
/// overlap out.
///
/// Node 0 is the NEARER of the two, so it is the one drawn last.
fn one_node_behind_another() -> Scene {
    let mut scene = parity_scene();
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    scene.node_radius = 0.25;
    let mut near = scene.nodes[0];
    near.world_pos = STACK_AT.extend(0.0);
    near.activation = 1.0;
    near.scale = 1.0;
    near.gutter = 0.0;
    near.hovered = false;
    near.on_home = true;
    let mut far = near;
    far.world_pos = STACK_AT.extend(-1.0);
    far.on_home = false;
    scene.nodes = vec![near, far];
    // The grid would draw across the same pixels, and whether IT covers a
    // label is a separate question that this fixture cannot answer twice.
    scene.grid.clear();
    scene
}

/// A node in front covers the label of the node behind it, the way it covers
/// that node itself.
///
/// This is the whole feature in one picture, and it is end to end on purpose:
/// the name is drawn inside the lattice's own scene pass, at its node's place
/// in the back-to-front order, so what covers a name is whatever the pass
/// draws after it. Nothing short of running that pass checks it.
///
/// Read as three renders of one scene — the picture with no label, with the
/// FAR node's name, and with the same glyph on the NEAR node, which puts it
/// after everything. The third is what makes the first assertion mean
/// something: a mapping that put the glyph anywhere but on the node would
/// leave the "covered" picture looking exactly right and this one looking
/// blank.
#[test]
fn a_nearer_node_covers_the_label_of_the_node_behind() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let scene = one_node_behind_another();
    let points = egui::vec2(SCENE_SIZE[0] as f32, SCENE_SIZE[1] as f32);
    let projector = scene.projector(glam::Vec2::new(points.x, points.y));

    // The pixel both discs are painted on, which is where the labels go and
    // where they are read back.
    let on = projector
        .project(scene.nodes[0].world_pos)
        .expect("the stack is in front of the camera");
    let (x, y) = (on.x.round(), on.y.round());
    assert!(
        (x - points.x / 2.0).abs() > 8.0 && (y - points.y / 2.0).abs() > 8.0,
        "the fixture's nodes must sit off-center, at ({x}, {y}) of {points:?}",
    );

    // One glyph, `off` points to the right of that pixel, named by `node`. No
    // rim: the fill alone answers the question, and a rim would spread the
    // reading over pixels nothing is being asked about.
    let bare = [TextRing::default(); 2];
    let picture = |off: f32, label: Option<u32>| -> Vec<u8> {
        let (glyphs, labels) = match label {
            Some(node) => (
                vec![GlyphInstance {
                    rect: [x + off - 4.0, y - 4.0, 8.0, 8.0],
                    ..crate::text::tests::glyph()
                }],
                vec![Label { node, glyphs: 1 }],
            ),
            None => (Vec::new(), Vec::new()),
        };
        let cb = LatticeCallback::from_scene(
            &scene,
            LatticeLabels {
                glyphs,
                labels,
                rings: bare,
                atlas: Some(crate::text::tests::atlas()),
            },
            points,
            format,
            9,
            None,
        );
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SCENE_SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));

        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
        let texture = render_to_texture(
            &device,
            &queue,
            SCENE_SIZE,
            format,
            wgpu::Color::TRANSPARENT,
            |pass| {
                cb.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: 1.0,
                        screen_size_px: SCENE_SIZE,
                    },
                    pass,
                    &resources,
                );
            },
        );
        readback(&device, &queue, &texture, SCENE_SIZE)
    };
    // What the probe pixel reads. The glyph is white and everything under it
    // is not, so one channel is the whole of what "how much name is here"
    // means.
    const NEAR: u32 = 0;
    const FAR: u32 = 1;
    let at = |off: f32, label: Option<u32>| -> u8 {
        let frame = picture(off, label);
        let i = (((y as u32) * SCENE_SIZE[0] + (x + off) as u32) * 4) as usize;
        frame[i + 1]
    };

    // On the disc, which is opaque: the far node's name is gone, exactly
    // gone — this is compositing, not a mask, so "under an opaque disc" is
    // the picture with no label in it at all.
    let (bare_disc, under, over) = (at(0.0, None), at(0.0, Some(FAR)), at(0.0, Some(NEAR)));
    assert_eq!(under, bare_disc, "a name under an opaque disc must leave no trace of itself");
    assert!(
        over.abs_diff(bare_disc) > 32,
        "the same glyph drawn after the disc must be plainly visible on it, \
         got {over} against a bare disc's {bare_disc} — if these agree the \
         glyph is not landing on the node and the assertion above is vacuous",
    );

    // Across the disc's own fading edge, where the difference between
    // covering and cutting shows: the name dims by exactly what the disc
    // took, rather than being taken out whole or left alone.
    let (bare_edge, under_edge, over_edge) =
        (at(6.0, None), at(6.0, Some(FAR)), at(6.0, Some(NEAR)));
    assert!(
        under_edge > bare_edge && under_edge < over_edge,
        "over the disc's fading edge a name must dim rather than vanish: \
         {under_edge} against {bare_edge} bare and {over_edge} drawn on top",
    );

    // And out in the glow — inside the node's quad, a percent or two of
    // opacity, nothing a reader can see — a name is left alone.
    let (bare_halo, under_halo, over_halo) =
        (at(16.0, None), at(16.0, Some(FAR)), at(16.0, Some(NEAR)));
    assert!(
        over_halo.abs_diff(bare_halo) > 32,
        "the halo probe must be somewhere a glyph shows at all: {over_halo} \
         against {bare_halo}",
    );
    assert!(
        under_halo.abs_diff(over_halo) <= 3,
        "out in the invisible glow a name must be left alone: {under_halo} \
         against {over_halo} drawn on top",
    );
}

/// A label is drawn immediately after the node it names, counted over the
/// instances that actually SHIP.
///
/// Two things that are easy to get right by accident and wrong in the
/// picture. The cull drops a node that can paint nothing, and such a node can
/// still carry a name — a hovered idle one draws no disc and is named all the
/// same — so a label's place is not its node's index in the sorted list. And
/// the labels arrive in the scene's order, which is not the order they are
/// drawn in.
#[test]
fn a_label_takes_its_own_nodes_place_in_the_order() {
    let mut scene = parity_scene();
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    scene.grid.clear();
    scene.idle_marker = harmonigraph_scene::IdleMarker::None;
    scene.trail_mark = harmonigraph_scene::TrailMark::Off;
    let node = |z: f32, activation: f32| harmonigraph_scene::NodeInstance {
        world_pos: glam::Vec3::new(0.0, 0.0, z),
        activation,
        octaves: [0.0; harmonigraph_scene::OCTAVE_SLOTS],
        melody_slots: 0,
        bass_slots: 0,
        melody_level: 0.0,
        bass_level: 0.0,
        trail: 0.0,
        on_home: z == 0.0,
        ..scene.nodes[0]
    };
    // In the scene's own order: the near sheet first, then two silent nodes
    // behind everything, then the home sheet. Drawn back to front that is
    // hush, hush, home, near — so nothing here is in the order it is drawn,
    // and the two silent ones ship no instance at all.
    scene.nodes = vec![node(1.0, 1.0), node(-1.0, 0.0), node(-1.0, 0.0), node(0.0, 1.0)];
    let (near, hush_a, hush_b, home) = (0u32, 1u32, 2u32, 3u32);

    // A glyph per label, told apart by where it claims to be.
    let glyph =
        |at: f32| GlyphInstance { rect: [at, 0.0, 1.0, 1.0], ..crate::text::tests::glyph() };
    let call = LatticeCallback::from_scene(
        &scene,
        LatticeLabels {
            glyphs: vec![glyph(0.0), glyph(1.0), glyph(2.0), glyph(3.0)],
            labels: [near, hush_a, hush_b, home]
                .map(|node| Label { node, glyphs: 1 })
                .to_vec(),
            rings: [TextRing::default(); 2],
            atlas: Some(crate::text::tests::atlas()),
        },
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        11,
        None,
    );

    assert_eq!(call.instances.len(), 2, "only the two sounding nodes ship an instance");
    assert_eq!(
        call.seams,
        vec![
            // Both silent nodes: nothing has been drawn yet, and two labels
            // at one seam are one uninterrupted draw. They are behind the
            // home sheet, so they are also behind the grid.
            GlyphSeam { at: 0, start: 0, count: 2, after_grid: false },
            // The home sheet's own name, after its disc.
            GlyphSeam { at: 1, start: 2, count: 1, after_grid: true },
            // And the near sheet's, after everything.
            GlyphSeam { at: 2, start: 3, count: 1, after_grid: true },
        ],
        "a label goes after its own node, over the instances that ship",
    );
    assert_eq!(
        call.glyphs.iter().map(|g| g.rect[0]).collect::<Vec<_>>(),
        vec![1.0, 2.0, 3.0, 0.0],
        "the glyphs are regrouped into the order they are drawn in",
    );
}

/// A name on a node that ships no disc still draws over the grid, not under
/// it — the case where the two runs meet at the same number.
///
/// `grid_at` is where the grid goes, counted over the instances that SHIP.
/// The sheets behind the home one draw before it and the home sheet after,
/// so the boundary between the two runs is exactly `grid_at`. A node that
/// paints nothing ships nothing, which leaves its seam sitting on the
/// boundary rather than past it: with every node on the home sheet — the
/// stock `extent_sevens: 0` — the far run is empty, `grid_at` is 0, and the
/// first home node to be culled takes seam 0 as well. Reading the side off
/// `at > grid_at` then files that node's name with the sheets BEHIND the
/// grid, and the grid is painted over the name.
///
/// The state is the plugin's resting one, which is what makes it worth a
/// test of its own: stock view, nothing played, hover any node. An idle node
/// draws no disc under `IdleMarker::None` with the trail lifting rather than
/// ringing, and a hovered node is named whether or not it draws.
#[test]
fn a_culled_home_nodes_name_draws_over_the_grid_it_shares_a_seam_with() {
    let mut scene = parity_scene();
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    // The stock resting state: one sheet, no idle disc, the trail lifting
    // rather than ringing, so an untouched node paints nothing at all.
    scene.idle_marker = harmonigraph_scene::IdleMarker::None;
    scene.trail_mark = harmonigraph_scene::TrailMark::Lift;
    let node = |activation: f32| harmonigraph_scene::NodeInstance {
        world_pos: glam::Vec3::new(0.0, 0.0, 0.0),
        activation,
        octaves: [0.0; harmonigraph_scene::OCTAVE_SLOTS],
        melody_slots: 0,
        bass_slots: 0,
        melody_level: 0.0,
        bass_level: 0.0,
        trail: 0.0,
        on_home: true,
        ..scene.nodes[0]
    };
    // The hovered one is silent and first, so it is culled before anything
    // has shipped and its seam is 0 — the same number as `grid_at`.
    scene.nodes = vec![node(0.0), node(1.0)];
    let glyph = GlyphInstance { rect: [0.0, 0.0, 1.0, 1.0], ..crate::text::tests::glyph() };
    let call = LatticeCallback::from_scene(
        &scene,
        LatticeLabels {
            glyphs: vec![glyph],
            labels: vec![Label { node: 0, glyphs: 1 }],
            rings: [TextRing::default(); 2],
            atlas: Some(crate::text::tests::atlas()),
        },
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        13,
        None,
    );

    assert!(!scene.grid.is_empty(), "the fixture needs a grid for the name to be covered BY");
    assert_eq!(call.grid_at, 0, "with one sheet there is nothing to draw before the grid");
    assert_eq!(
        call.seams,
        vec![GlyphSeam { at: 0, start: 0, count: 1, after_grid: true }],
        "a home node's name draws after the grid even when the cull leaves it on grid_at",
    );
}

/// A label is not in the bloom: the light the bloom adds to a frame is the
/// same whether the names are drawn or not.
///
/// Two halves, and the second is the one that is easy to miss. A name in the
/// bloom input GLOWS, which is the obvious half — white type well over the
/// bright pass's threshold, haloed like a lit node. It also takes a BITE out
/// of the halo of the node it covers, by standing where that node's own
/// bright pixels were, so a node dims as a name crosses it. Both are what a
/// second colour attachment buys, and both show up here as the same
/// difference.
///
/// Measured as bloom LIGHT rather than as pixels, since that is the thing
/// that must not change: the composite adds `bloom * strength` on top of the
/// scene, so rendering each arrangement with the bloom on and off and
/// subtracting isolates exactly the term under test. The scene itself does
/// change with a label in it — that is the feature — so comparing frames
/// directly would be comparing the label to itself.
#[test]
fn a_label_adds_no_light_through_the_bloom() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let scene = one_node_behind_another();
    let points = egui::vec2(SCENE_SIZE[0] as f32, SCENE_SIZE[1] as f32);
    let on = scene
        .projector(glam::Vec2::new(points.x, points.y))
        .project(scene.nodes[0].world_pos)
        .expect("the stack is in front of the camera");
    let (x, y) = (on.x.round(), on.y.round());

    // The near node's name, right on the node — over its brightest pixels,
    // which is where a label robs a halo of the most light.
    let label = |on: bool| -> (Vec<GlyphInstance>, Vec<Label>) {
        if !on {
            return (Vec::new(), Vec::new());
        }
        (
            vec![GlyphInstance {
                rect: [x - 6.0, y - 6.0, 12.0, 12.0],
                ..crate::text::tests::glyph()
            }],
            vec![Label { node: 0, glyphs: 1 }],
        )
    };
    let frame = |labelled: bool, bloom: f32| -> Vec<u8> {
        let (glyphs, labels) = label(labelled);
        // A fresh fixture per frame: `Scene` is not `Clone`, and the only
        // thing that varies is the strength the composite reads.
        let mut scene = one_node_behind_another();
        scene.bloom_strength = bloom;
        let cb = LatticeCallback::from_scene(
            &scene,
            LatticeLabels {
                glyphs,
                labels,
                rings: [TextRing::default(); 2],
                atlas: Some(crate::text::tests::atlas()),
            },
            points,
            format,
            13,
            None,
        );
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SCENE_SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
        let clear = wgpu::Color::TRANSPARENT;
        let texture =
            render_to_texture(&device, &queue, SCENE_SIZE, format, clear, |pass| {
                cb.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: 1.0,
                        screen_size_px: SCENE_SIZE,
                    },
                    pass,
                    &resources,
                );
            });
        readback(&device, &queue, &texture, SCENE_SIZE)
    };
    let (lit, plain) = (frame(true, 1.0), frame(true, 0.0));
    let (bare_lit, bare_plain) = (frame(false, 1.0), frame(false, 0.0));

    // Non-vacuous first: there has to BE bloom, and it has to reach the
    // pixels the label covers — otherwise this passes on a frame with no
    // light in it to move.
    let light = |on: &[u8], off: &[u8], i: usize| on[i] as i32 - off[i] as i32;
    let near_label = |i: usize| {
        let row = SCENE_SIZE[0] as usize;
        let (px, py) = (((i / 4) % row) as f32, ((i / 4) / row) as f32);
        (px - x).abs() <= 10.0 && (py - y).abs() <= 10.0
    };
    let peak = (0..lit.len())
        .filter(|i| i % 4 != 3 && near_label(*i))
        .map(|i| light(&bare_lit, &bare_plain, i))
        .max()
        .unwrap_or(0);
    assert!(peak > 8, "the fixture must bloom where the label sits, peak {peak}");

    // And the label changes none of it. Saturated channels are skipped: the
    // glyph is white, so where it lands the composite is already at 255 with
    // the bloom off and the added light has nowhere to go — a clamp, not a
    // reading.
    let (mut worst, mut at) = (0i32, 0usize);
    for i in (0..lit.len()).filter(|i| i % 4 != 3) {
        if lit[i] == 255 || bare_lit[i] == 255 {
            continue;
        }
        let d = light(&lit, &plain, i) - light(&bare_lit, &bare_plain, i);
        if d.abs() > worst.abs() {
            (worst, at) = (d, i);
        }
    }
    assert!(
        worst.abs() <= 1,
        "a label changed the bloom by {worst}/255 at pixel ({}, {}) — it is in the bright \
         pass's input, so it glows and eats the halo of the node it covers",
        (at / 4) % SCENE_SIZE[0] as usize,
        (at / 4) / SCENE_SIZE[0] as usize,
    );
}

