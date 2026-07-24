//! Unit tests for the lattice renderer. The GPU-backed ones no-op when
//! no headless adapter is available (CI without a GPU).

use super::*;

#[test]
fn baked_shader_validates() {
    validate_wgsl(SHADER_SRC)
        .expect("baked lattice.wgsl must parse, validate, and keep its entry points");
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
    let Some((device, _queue)) = headless_device() else {
        return;
    };
    let _resources =
        LatticeResources::new(&device, wgpu::TextureFormat::Bgra8Unorm);
}

fn headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
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

/// A scene exercising every draw path: lit + idle + outlined + hovered
/// nodes with octave indicators, a chord beam, and solid + dashed grid
/// lines, all overlapping so blend order matters.
fn parity_scene() -> Scene {
    use glam::{Vec3, Vec4};
    use lattice_core::LatticePos;

    let mut nodes = Vec::new();
    for i in 0..6u32 {
        let f = i as f32;
        let mut octaves = [0.0f32; lattice_scene::OCTAVE_SLOTS];
        octaves[(i as usize) % lattice_scene::OCTAVE_SLOTS] = 1.0 - f * 0.1;
        octaves[(i as usize + 5) % lattice_scene::OCTAVE_SLOTS] = 0.4;
        nodes.push(lattice_scene::NodeInstance {
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
            melody_slots: if i == 0 || i == 4 { 1 << (i as usize % lattice_scene::OCTAVE_SLOTS) } else { 0 },
            bass_slots: if i == 2 || i == 4 { 1 << (i as usize % lattice_scene::OCTAVE_SLOTS) } else { 0 },
            melody_level: if i == 0 || i == 4 { 1.0 } else { 0.0 },
            bass_level: if i == 2 || i == 4 { 1.0 } else { 0.0 },
            melody_color: Vec4::new(1.0, 0.85, 0.4, 1.0),
            bass_color: Vec4::new(0.45, 0.8, 1.0, 1.0),
            trail: 0.0,
        });
    }
    let grid = vec![
        lattice_scene::EdgeInstance {
            a: Vec3::new(-1.8, -0.6, -0.3),
            b: Vec3::new(1.6, -0.6, -0.3),
            color: Vec4::new(0.16, 0.17, 0.20, 0.55),
            strength: 0.55,
            dashed: false,
        },
        lattice_scene::EdgeInstance {
            a: Vec3::new(-1.2, 0.7, -0.6),
            b: Vec3::new(1.2, 0.4, 0.6),
            color: Vec4::new(0.16, 0.17, 0.20, 0.55),
            strength: 0.4,
            dashed: true,
        },
    ];
    Scene {
        nodes,
        camera: lattice_scene::Camera::default(),
        time: 1.25,
        // The ground the sevens knockout clears to; the half of this
        // scene's nodes that carry a gutter exercise it.
        background: lattice_scene::skin::panel_color(),
        sevens_soft: 0.24,
        node_radius: 0.34,
        outer_style: Default::default(),
        mark_unlinked: 1.0,
        mark_thickness: 0.09,
        node_style: Default::default(),
        core_radius: 0.46,
        core_solidity: 1.0,
        outer_inner: 0.545,
        outer_outer: 0.795,
        outer_backdrop: 0.0,
        outer_solidity: 1.0,
        outer_gap: 0.12,
        idle_marker: lattice_scene::IdleMarker::None,
        idle_radius: 0.0,
        grid,
        grid_thickness: 1.0,
        // The parity image is about how a NOTE is drawn; the trail marks
        // only idle nodes and has its own tests. Off keeps this baseline
        // comparable to the ones taken before it existed.
        trail_mark: Default::default(),
        trail_strength: 0.0,
        node_idle: Vec4::new(0.27, 0.29, 0.34, 1.0),
        pitch_lut: std::array::from_fn(|k| {
            Vec4::new(k as f32 / 15.0, 0.4, 1.0 - k as f32 / 15.0, 1.0)
        }),
        darkest_pitch: 24.0,
        brightest_pitch: 108.0,
        render_scale: 1.0,
        // Parity with the old direct renderer requires bloom off.
        bloom_strength: 0.0,
    }
}

/// Render into a fresh texture cleared to `clear`, handing the pass to
/// `draw`, and return the texture for readback.
fn render_to_texture(
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

fn readback(
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

/// Bloom must add light (halo energy over the bloom-off output) —
/// and only when asked: strength 0 keeps the parity test above valid.
/// One big centered node, sounding, with one octave slot lit: a clean
/// backdrop for measuring how much of the picture a mark actually
/// covers. parity_scene deliberately overlaps its nodes, which hides
/// most of a mark behind whatever draws in front of it.
fn single_marked_node(melody_slots: u32, bass_slots: u32) -> Scene {
    use glam::{Vec3, Vec4};
    use lattice_core::LatticePos;

    let mut octaves = [0.0f32; lattice_scene::OCTAVE_SLOTS];
    octaves[0] = 1.0;
    let mut scene = parity_scene();
    scene.nodes = vec![lattice_scene::NodeInstance {
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
        // rings apart; in the app these are the marked notes' own colors.
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
        let cb = LatticeCallback::from_scene(scene, vec_size, format, pane_id);
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
    let melody = shot(&single_marked_node(1, 0), 41);
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
    // and bottom share a pitch class -- used to be blanked, so the mark
    // vanished exactly when two things were true at once. The two ends are
    // now rings at DIFFERENT radii, so both simply draw: the result must
    // cover at least as much as one end alone. This guards that.
    let split = shot(&single_marked_node(1, 1), 45);
    let split_px = changed_px(&split);
    eprintln!("split mark {split_px} px of {node_px}");
    assert!(
        split_px >= both_px,
        "a mark claimed by both ends all but disappeared: \
         {split_px} px against {both_px} for one end alone"
    );

    // ...and it really is BOTH rings, not one end quietly winning: the
    // melody-only and bass-only pictures must each differ from it.
    let bass_only = shot(&single_marked_node(0, 1), 46);
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
    use lattice_core::{NoteEvent, NoteEventKind, NoteTracker, Tuning};
    use lattice_scene::{derive_scene, Camera, FrameParams, HighlightExtremes, ViewConfig};

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
    let scene_for = |which: HighlightExtremes| {
        derive_scene(
            &tracker,
            &Tuning::default(),
            &ViewConfig { highlight_extremes: which, ..base.clone() },
            &FrameParams::default(),
            Camera::default(),
            None,
            // Past OCTAVE_ATTACK_TIME: the octave glyphs ease in over the
            // first 0.15s, and the mark rides one of them, so at t=0 there
            // is deliberately nothing on that layer yet.
            0.5,
        )
    };

    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor {
        size_in_pixels: SIZE,
        pixels_per_point: 1.0,
    };
    let mut shot = |scene: &Scene, pane_id: u64| -> Vec<u8> {
        let cb = LatticeCallback::from_scene(scene, vec_size, format, pane_id);
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
    let marked = scene_for(HighlightExtremes::Both);
    let melody_nodes = marked.nodes.iter().filter(|n| n.melody_slots != 0).count();
    let bass_nodes = marked.nodes.iter().filter(|n| n.bass_slots != 0).count();
    assert!(
        melody_nodes > 0 && bass_nodes > 0,
        "derive_scene marked nothing: {melody_nodes} melody, {bass_nodes} bass nodes"
    );

    let off = shot(&scene_for(HighlightExtremes::Off), 50);
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
        let cb = LatticeCallback::from_scene(scene, vec_size, format, pane_id);
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
    use lattice_scene::{Camera, FrameParams, Projection, ViewConfig};

    let view = ViewConfig {
        extent_threes: 1,
        extent_fives: 1,
        extent_sevens: 2,
        ..ViewConfig::default()
    };
    for projection in [Projection::Cabinet, Projection::Perspective, Projection::Orthographic]
    {
        let scene = lattice_scene::derive_scene(
            &lattice_core::NoteTracker::new(),
            &lattice_core::Tuning::default(),
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
