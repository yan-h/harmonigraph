//! The harness itself — that a device comes up, that a second view in one
//! frame still submits, and that the offscreen composite matches a direct draw.

use super::fixtures::*;
use crate::gpu_harness::{headless_device, readback, render_to_texture};
use crate::*;

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
    let _resources = LatticeResources::new(&device, &queue, wgpu::TextureFormat::Bgra8Unorm);
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
    let preview =
        LatticeCallback::from_scene(&scene, LatticeLabels::default(), size, format, 1, None);

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
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    let mut encoder = device.create_command_encoder(&Default::default());
    let user_bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
    queue.submit(user_bufs.into_iter().chain([encoder.finish()]));

    let clear = wgpu::Color { r: 0.07, g: 0.08, b: 0.09, a: 1.0 };
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
    let layouts = SceneLayouts {
        uniforms: &res.bind_group_layout,
        glow: &res.filter_layout,
        shadow: &res.shadow_layout,
        casters: &res.caster_layout,
    };
    let (node_pipeline, plus_pipeline) =
        create_pipelines(&device, &with_common(SHADER_SRC), format, layouts, false);
    // The stand-in light at group 1: this path has no glow pass to composite,
    // and the fixture asks for none (`parity_scene` holds the reach at 0), so
    // the offscreen path is reading the same transparent nothing.
    let light = &res.glow_dummy_bind_group;
    let pane = res.panes.get(&7).expect("prepare created the pane");
    // The atlas the pass above filled, which this path samples rather than
    // fills: the shadows are part of the picture the two are compared on.
    let cells = pane
        .offscreen
        .as_ref()
        .and_then(|o| o.shadow.as_ref())
        .map_or(&res.shadow_dummy_bind_group, |a| a.read());
    let direct_tex = render_to_texture(&device, &queue, SIZE, format, clear, |pass| {
        // The pane's own order, walked the same way `prepare` walks it — a
        // second expression of it here would make the two paths differ by draw
        // order rather than by the thing under test. The fixture carries no
        // name, so that arm would be dead code standing in for a claim nothing
        // here makes; it fails instead.
        for draw in &pane.draws {
            match *draw {
                Draw::Nodes(a, b) => {
                    pass.set_pipeline(&node_pipeline);
                    pass.set_bind_group(0, &pane.bind_group, &[]);
                    pass.set_bind_group(1, light, &[]);
                    pass.set_bind_group(2, cells, &[]);
                    pass.set_bind_group(3, &pane.caster_bind_group, &[]);
                    pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                    pass.set_vertex_buffer(1, pane.node_cell_buffer.slice(..));
                    pass.draw(0..4, a..b);
                }
                Draw::Pluses(a, b) => {
                    pass.set_pipeline(&plus_pipeline);
                    pass.set_bind_group(0, &pane.bind_group, &[]);
                    pass.set_bind_group(1, light, &[]);
                    pass.set_bind_group(2, cells, &[]);
                    pass.set_bind_group(3, &pane.caster_bind_group, &[]);
                    pass.set_vertex_buffer(0, pane.plus_buffer.slice(..));
                    pass.draw(0..4, a..b);
                }
                other => panic!("the parity fixture drew a {other:?}, which this path cannot"),
            }
        }
    });

    let composite = readback(&device, &queue, &composite_tex, SIZE);
    let direct = readback(&device, &queue, &direct_tex, SIZE);

    // Guard against vacuous success: the scene must actually have drawn
    // over the clear color somewhere.
    let bg = [18u8, 20, 23, 255]; // clear color as 8-bit RGBA
    assert!(
        direct.chunks(4).any(|px| px.iter().zip(bg).any(|(&c, b)| c.abs_diff(b) > 8)),
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

/// A lattice that stops drawing stops reporting a GPU time, rather than
/// holding the last one it measured.
///
/// The reading is a cross-frame value: it is only overwritten when the
/// timer's readback cycle turns over, and that cycle is driven from inside
/// the scene pass. Since nodes that paint nothing are no longer shipped, a
/// lattice can encode no pass at all, and can then sit there indefinitely
/// rather than for a frame. Left alone, the overlay would keep re-averaging
/// a figure from whenever the lattice last drew, which is the one thing
/// `GPU_TIME_PENDING` exists to make impossible to confuse with a live
/// reading.
///
/// The scene is built empty here rather than dialled empty, which keeps the
/// guard independent of which settings happen to reach the state. Two do: a
/// silent lattice ships no node already, and the marker field under it goes
/// whole at Arm length 0 (`derive_pluses` returns nothing) or under Note
/// names All, where a name claims every position. So this is a state a person
/// can dial into and hold, not only one a fixture can build.
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
    empty.pluses.clear();
    let blank = LatticeCallback::from_scene(
        &empty,
        LatticeLabels::default(),
        size,
        format,
        40,
        Some(stats.clone()),
    );
    assert!(blank.instances.is_empty() && blank.pluses.is_empty(), "nothing to draw");
    frame(&blank);
    assert_eq!(
        stats.gpu_ms.load(std::sync::atomic::Ordering::Relaxed),
        GPU_TIME_PENDING,
        "a pane that encodes no pass must not keep reporting the time it took \
         when it last drew",
    );
}
