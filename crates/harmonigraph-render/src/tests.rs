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
        size,
        format,
        0,
        Some(std::sync::Arc::new(LatticeStats::default())),
    );
    let preview = LatticeCallback::from_scene(&scene, size, format, 1, None);

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
        let (low, _) = layout.slot_range(0.0);
        let (_, high) = layout.slot_range(950.0);
        let slot = |k: usize| (low as usize) + k % (high - low + 1) as usize;
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
        let cb = LatticeCallback::from_scene(scene, vec_size, format, pane_id, None);
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
        let cb = LatticeCallback::from_scene(scene, vec_size, format, pane_id, None);
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
    // Every octave the Range draws for THIS pitch class, and only those: a
    // level on a slot no sector draws is a state `derive_scene` cannot reach,
    // and the swirl and the glow would still take a color from it.
    let (low, high) = layout.slot_range(cents);
    let node = &mut scene.nodes[0];
    node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
    for slot in low..=high {
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
    // tapered band is heavier on the side its wide octaves fall, which would
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
/// than on the layout that feeds it: every octave a node has inside the Range
/// gets an indicator, and together they close the ring — whatever the window,
/// the taper or the node's pitch class. So the only unlit stretches are the
/// Gap setting's slits, one per boundary, and the seam is one of them on every
/// node, since the two end indicators reach it from either side.
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
        let cb = LatticeCallback::from_scene(scene, vec_size, format, pane_id, None);
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

    // The narrowest window there is (MIN_WINDOW, where the taper takes a
    // sector widest and the union branch of the wedge test is stressed
    // hardest), the widest, the default, and two that are neither a whole
    // number of octaves nor centered on a C — the case the end indicators
    // exist for. Each at a C node (whose octaves land flush on the default
    // window's ends) and at three pitch classes that do not.
    let mut pane = 60;
    for (low, high) in [
        (36.0f32, 84.0f32),
        (0.0, 127.0),
        (6.0, 114.0),
        (30.0, 90.0),
        (24.0, 97.5),
    ] {
        // An even axis, a straight ramp, the sharpest shape, and a plateau.
        //
        // Not the FULLEST amount, which is the one thing here the picture
        // cannot answer: the Gap is cut out of the sectors either side of it,
        // and at 0.9 the edge slices are thinner than that padding, so their
        // two slits meet and the ring reads as one indicator short. That is
        // the setting talking rather than a missing indicator — `octaves.rs`
        // pins the extreme exactly, on angles, where no padding is involved.
        for (amount, shape) in [(0.0, 0.5), (0.6, 0.5), (0.7, 0.0), (0.6, 0.85)] {
            for cents in [0.0, 350.0, 700.0, 1150.0] {
                let layout = octave_layout(low, high, amount, shape);
                let px = shot(&octave_wheel_scene(layout, cents), pane);
                pane += 1;
                let profile = band_profile(&px, SIZE[0]);
                let case =
                    format!("{low}..{high}, amount {amount} shape {shape}, {cents}c");

                // One indicator per octave this node has in the window,
                // closing the ring: that is one slit per boundary and no
                // other break. A missing indicator merges two slits into one
                // hole, so the count is what says all of them are there.
                let (first, last) = layout.slot_range(cents);
                let want = (last - first + 1) as usize;
                let runs = unlit_runs(&profile);
                // Except at the two ENDS under a heavy taper, and that is the
                // settings talking rather than a missing indicator. An end
                // indicator reaches from the seam to its own octave's edge, so
                // it can be as little as half an octave — the window generally
                // stops part way between two of a node's octaves — while the
                // Gap is cut out of it from both sides at full width. Past
                // about 0.6 the edge of the axis is thin enough that those two
                // slits meet on a short one, and the ring reads an indicator
                // short. `octaves.rs` pins the count exactly, on angles, where
                // no padding is involved.
                if amount <= 0.6 {
                    assert_eq!(runs.len(), want, "{case}: unlit runs {runs:?} for {want} sectors");
                } else {
                    assert!(
                        runs.len() >= want - 2 && runs.len() <= want,
                        "{case}: unlit runs {runs:?} for {want} sectors — at most the two \
                         end indicators can be lost to the Gap"
                    );
                }

                // 270 degrees is straight down: the window's two ends at
                // once, and now a boundary on EVERY node — the lowest
                // indicator reaches the seam from one side and the highest
                // from the other, whatever the pitch class. That is what
                // replaced the wrap, and it is the whole of why the ring
                // closes at a window like 24..97.5.
                assert!(gap_at(&profile, 270.0) > 0.0, "{case}: no seam at the bottom");

                // The middle of the window is straight up, and a node whose
                // octaves straddle it covers the top. Read on the two pitch
                // classes that sit clear of a boundary there, at windows
                // whose middle is a C: the others legitimately put a slit at
                // the top, which is the axis being read rather than a hole.
                if 0.5 * (low + high) == 60.0 && (cents == 0.0 || cents == 700.0) {
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
    for (low, high, amount, shape) in [(30.0f32, 90.0f32, 0.0, 0.5), (6.0, 126.0, 0.7, 0.25)] {
        let layout = octave_layout(low, high, amount, shape);
        // A C node and a node a fifth up: same slot, pitches 7 semitones
        // apart, so the bright arc must move by exactly that much of the axis.
        // The octave holding the window's middle, and one further round the
        // wheel, where a wrong anchor or a wrong direction shows.
        //
        // Both held INSIDE the node's own range rather than at its ends: an
        // end indicator can be half an octave, which a heavy taper leaves
        // thinner than the Gap's slits, and a centroid needs an arc to
        // measure. That the ends reach the seam at all is
        // `every_octave_in_the_range_is_drawn_and_they_close_the_ring`.
        for (cents, offset) in [(0.0f32, 0u32), (700.0, 0), (0.0, 2), (700.0, 2)] {
            let (first, last) = layout.slot_range(cents);
            let slot =
                (harmonigraph_scene::MIDDLE_C_SLOT as u32 + offset).clamp(first + 1, last - 1);
            let mut scene = octave_wheel_scene(layout, cents);
            // One octave sounding. The silent slots still ghost in behind it
            // at GHOST_LEVEL, which the brightness threshold below sorts out.
            scene.nodes[0].octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
            scene.nodes[0].octaves[slot as usize] = 1.0;

            let cb = LatticeCallback::from_scene(&scene, vec_size, format, pane, None);
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
            assert!(peak > 60, "{low}..{high}: nothing bright enough to measure");
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
            // between its two edges in ANGLE, which a taper can shift off the
            // pitch itself.
            let (e0, e1) = layout.sector(slot, cents);
            let expected = (0.5 * (e0 + e1)).to_degrees().rem_euclid(360.0);
            let off = (drawn.rem_euclid(360.0) - expected).rem_euclid(360.0);
            let off = off.min(360.0 - off);
            assert!(
                off < 6.0,
                "{low}..{high}, {cents}c, slot {slot}: indicator drawn at {drawn:.1} deg, \
                 the axis puts its pitch at {expected:.1}"
            );
        }
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
        let cb = LatticeCallback::from_scene(scene, vec_size, format, pane_id, None);
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
        ..ViewConfig::default()
    };
    for projection in [Projection::Cabinet, Projection::Perspective, Projection::Orthographic]
    {
        let scene = harmonigraph_scene::derive_scene(
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
        let call = LatticeCallback::from_scene(
            &scene,
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
        assert!(depths.len() > 1, "the window has to hold several sheets");
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
        node.trail = if i % 2 == 0 { 0.8 } else { 0.0 };
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

    for (name, scene) in [("lit", parity_scene()), ("idle", idle_scene())] {
        let cb = LatticeCallback::from_scene(
            &scene,
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

