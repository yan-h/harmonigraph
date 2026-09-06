//! A probe of what the glow gather costs per frame as lit nodes accumulate,
//! at the lattice pane size and glow settings of the saved live view
//! (2026-09-06). Prints and asserts nothing.

use super::fixtures::*;
use crate::*;

/// Lattice pane at 1176x820 points, 2x Retina: 607x794 points of lattice.
const SIZE: [u32; 2] = [1214, 1588];
const FRAMES: usize = 60;

#[test]
#[ignore = "a probe: prints a timing and asserts nothing"]
fn a_frame_of_the_gather_at_the_live_pane_costs_this_much() {
    let env = |k: &str, d: f32| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let spacing = env("PROBE_SPACING", 1.0);
    let distance = env("PROBE_DISTANCE", 8.186253);
    let reach = env("PROBE_REACH", 4.546375);
    let accumulation = env("PROBE_ACCUM", 0.0);
    let bloom = env("PROBE_BLOOM", 0.80615485);
    let counts: Vec<usize> = std::env::var("PROBE_COUNTS")
        .ok()
        .map(|v| v.split(',').filter_map(|s| s.parse().ok()).collect())
        .unwrap_or_else(|| vec![0, 1, 5, 10, 20, 50, 100, 200, 400]);

    let base = single_marked_node(0, 0);
    let mut lit = base.nodes[0];
    lit.activation = 1.0;
    lit.octaves.fill(1.0);
    lit.glow = harmonigraph_scene::GlowStep { level: 1.0, row: 0, mix: 1.0, marked: 0.0 };
    let mut idle = lit;
    idle.activation = 0.0;
    idle.octaves.fill(0.0);
    idle.audio_ring = 0.0;
    idle.glow = harmonigraph_scene::GlowStep::default();

    // A 25x41 window like the saved view's extents, in world spacing; the
    // nearest-to-centre N get lit so the lit set is the on-screen one.
    let mut cells: Vec<(i32, i32)> =
        (-20..=20).flat_map(|y| (-12..=12).map(move |x| (x, y))).collect();
    cells.sort_by_key(|(x, y)| x * x + y * y);

    for &n in &counts {
        let mut scene = single_marked_node(0, 0);
        scene.camera.distance = distance;
        scene.camera.yaw = 0.4;
        scene.camera.pitch = 0.3;
        scene.camera.projection = harmonigraph_scene::Projection::Cabinet;
        scene.camera.cabinet_angle = 0.7853982;
        scene.camera.cabinet_scale = 0.6;
        scene.node_radius = spacing * 0.25;
        scene.marker_unit = scene.node_radius * 1.8;
        scene.glow_reach = reach;
        scene.glow_strength = 0.75871396;
        scene.glow_curve = harmonigraph_scene::GlowCurve { shape: 1.7532053 };
        scene.glow_blend = 0.37715346;
        scene.glow_accumulation = accumulation;
        scene.glow_wash = 1.0;
        scene.bloom_strength = bloom;
        scene.nodes = cells
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| {
                let mut node = if i < n { lit } else { idle };
                node.lattice_pos = harmonigraph_core::LatticePos::new(x, y, 0);
                node.world_pos = glam::vec3(x as f32 * spacing, y as f32 * spacing, 0.0);
                node.cents = ((x * 7 + y * 4).rem_euclid(12) * 100) as f32;
                node
            })
            .collect();
        rows_per_node(&mut scene);
        time_a_gather_frame(&scene, n);
    }
}

fn time_a_gather_frame(scene: &Scene, n: usize) {
    let instance = wgpu::Instance::default();
    let Ok(adapter) =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        eprintln!("no GPU adapter; nothing timed");
        return;
    };
    let features = wgpu::Features::TIMESTAMP_QUERY;
    if !adapter.features().contains(features) {
        eprintln!("the adapter carries no timestamps; nothing timed");
        return;
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: features,
        ..Default::default()
    }))
    .expect("a device with timestamps");
    let pane = glam::Vec2::new(SIZE[0] as f32, SIZE[1] as f32);

    let set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("gather_probe"),
        ty: wgpu::QueryType::Timestamp,
        count: 2,
    });
    let resolve = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gather_resolve"),
        size: 16,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gather_staging"),
        size: 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let stamp_view = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("gather_stamp"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&Default::default());
    let stamp = |encoder: &mut wgpu::CommandEncoder, index: u32| {
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gather_stamp_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &stamp_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Discard,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                query_set: &set,
                beginning_of_pass_write_index: Some(index),
                end_of_pass_write_index: None,
            }),
            occlusion_query_set: None,
            multiview_mask: None,
        });
    };
    let period = queue.get_timestamp_period();
    let mut resources = CallbackResources::default();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(pane.x, pane.y));
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut gpu = Vec::with_capacity(FRAMES);
    let mut cpu = Vec::with_capacity(FRAMES);
    let mut wall = Vec::with_capacity(FRAMES);
    let mut listed = 0usize;
    let mut global = 0usize;
    let mut radius = 0.0f32;
    for frame in 0..FRAMES + 10 {
        let cb = LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            egui::vec2(pane.x, pane.y),
            format,
            1,
            None,
        );
        if frame == 0 {
            let nodes = cb.glow_nodes(SIZE);
            listed = nodes.len();
            radius = nodes.first().map(|n| n.mark[1]).unwrap_or(0.0);
            let packed = glow_tiles::pack(&nodes, SIZE);
            global = packed.get(2).zip(packed.get(1)).map(|(e, s)| (e - s) as usize).unwrap_or(0);
        }
        let wall_start = std::time::Instant::now();
        let mut encoder = device.create_command_encoder(&Default::default());
        stamp(&mut encoder, 0);
        let cpu_start = std::time::Instant::now();
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        let cpu_ms = cpu_start.elapsed().as_secs_f64() * 1000.0;
        stamp(&mut encoder, 1);
        encoder.resolve_query_set(&set, 0..2, &resolve, 0);
        encoder.copy_buffer_to_buffer(&resolve, 0, &staging, 0, 16);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let _ = crate::gpu_harness::render_to_texture(
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
        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let ticks: Vec<u64> = {
            let view = slice.get_mapped_range();
            bytemuck::cast_slice::<u8, u64>(&view).to_vec()
        };
        staging.unmap();
        let wall_ms = wall_start.elapsed().as_secs_f64() * 1000.0;
        assert!(ticks[0] > 0 && ticks[1] >= ticks[0], "unsupported timestamp pair: {ticks:?}");
        let ms = (ticks[1] - ticks[0]) as f64 * f64::from(period) / 1.0e6;
        if frame >= 10 {
            gpu.push(ms);
            cpu.push(cpu_ms);
            wall.push(wall_ms);
        }
    }
    let med = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        (v[v.len() / 2], v[v.len() * 9 / 10])
    };
    let (g, g90) = med(&mut gpu);
    let (c, _) = med(&mut cpu);
    let (w, w90) = med(&mut wall);
    eprintln!(
        "lit {n:>4}: listed {listed:>4} (global {global:>4}, halo r {radius:>6.1} px) | \
         prepare GPU {g:7.3} ms (p90 {g90:7.3}) | prepare CPU {c:6.3} ms | \
         whole frame incl. readback wait {w:7.3} ms (p90 {w90:7.3})"
    );
}
