//! A GPU-timer probe: what a frame of the live view with a name on every lit
//! node costs to draw, for the number each PR of #498 states against main.
//! `#[ignore]`d — it prints a figure and asserts nothing.
//!
//! Timestamps bracket the whole of `prepare`'s encoder rather than the plugin
//! overlay's own bracket, which opens at the scene pass: the atlas and the
//! light are drawn before that, and they are the cost this rework moves. Two
//! empty passes carry the stamps, because a stamp written straight into the
//! encoder never signals on Metal.
//!
//! Two settings, because the Shadow bar's two ends cost differently: a
//! caster's blur is `glow_shadow` node radii wide, so the top of the bar is
//! the widest atlas cell and the largest grown quad the pass ever draws.
//!
//! ```text
//! cargo test -p harmonigraph-render -- --ignored --nocapture a_frame_of_names
//! ```

use super::fixtures::*;
use super::golden::the_live_view;
use crate::*;

/// Wide enough that the names are about the size the lattice typesets them.
const SIZE: [u32; 2] = [768, 768];
const FRAMES: usize = 120;

#[test]
#[ignore = "a probe: prints a timing and asserts nothing"]
fn a_frame_of_names_costs_this_much() {
    time_a_frame_of_names(the_live_view(), "the live view");
}

/// The same view with the Shadow at the top of its bar.
///
/// The live view already carries the depth at 1.0, so the Shadow's WIDTH is
/// the whole of the difference — and width is what sizes both the cell a
/// caster blurs in and the quad it is multiplied back through.
#[test]
#[ignore = "a probe: prints a timing and asserts nothing"]
fn a_frame_of_names_at_the_top_of_the_shadow_bar_costs_this_much() {
    let mut scene = the_live_view();
    scene.glow_shadow = harmonigraph_scene::GLOW_SHADOW_MAX;
    time_a_frame_of_names(scene, "the top of the Shadow bar");
}

/// Both renderers, at both ends of the Shadow bar.
///
/// The two ends are here for the same reason the two probes above are: the
/// atlas SHRINKS as the bar opens and the quads grow, so a renderer's cost is
/// not one number.
///
/// DISTANCE is the expensive one to measure before a merge (#536). Its cell
/// does not shrink past the renderer's quality floor, so its atlas stays finer
/// as the bar opens; its fill is the whole node shader at that resolution, once
/// per node (#507).
#[test]
#[ignore = "a probe: prints a timing and asserts nothing"]
fn a_frame_of_names_at_each_kernel_costs_this_much() {
    use harmonigraph_scene::ShadowKernel::{Distance, Gaussian};
    for kernel in [Gaussian, Distance] {
        for (shadow, where_) in [
            (the_live_view().glow_shadow, "the live view"),
            (harmonigraph_scene::GLOW_SHADOW_MAX, "the top of the bar"),
        ] {
            let mut scene = the_live_view();
            scene.glow_shadow = shadow;
            scene.glow_shadow_kernel = kernel;
            time_a_frame_of_names(scene, &format!("{kernel:?} at {where_}"));
        }
    }
}

fn time_a_frame_of_names(mut scene: Scene, what: &str) {
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

    // The default distance rather than the golden's close one, so the pane
    // holds a lattice's worth of nodes and names.
    scene.camera.distance = harmonigraph_scene::Camera::default().distance;
    let pane = glam::Vec2::new(SIZE[0] as f32, SIZE[1] as f32);
    let projector = scene.projector(pane);
    let unit = scene.node_radius * scene.camera.points_per_world(SIZE[1] as f32);
    // Three strokes per lit node on the pane, about a name's size.
    let (w, h, gap) = (0.22 * unit, 0.55 * unit, 0.12 * unit);
    let runs: Vec<(u32, Vec<GlyphInstance>)> = scene
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.activation > 0.0)
        .filter_map(|(i, n)| {
            let at = projector.project(n.world_pos)?;
            let inside = at.cmpge(glam::Vec2::splat(unit)).all()
                && at.cmple(pane - glam::Vec2::splat(unit)).all();
            inside.then(|| {
                let span = 3.0 * w + 2.0 * gap;
                let glyphs = (0..3)
                    .map(|k| {
                        let x = at.x - span / 2.0 + k as f32 * (w + gap);
                        name_glyph(&scene, [x, at.y - h / 2.0, w, h])
                    })
                    .collect();
                (i as u32, glyphs)
            })
        })
        .collect();
    let named = runs.len();

    let set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("timing_probe"),
        ty: wgpu::QueryType::Timestamp,
        count: 2,
    });
    let resolve = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("timing_resolve"),
        size: 16,
        usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("timing_staging"),
        size: 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    // The two empty passes' target.
    let stamp_view = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("timing_stamp"),
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
        let writes = wgpu::RenderPassTimestampWrites {
            query_set: &set,
            beginning_of_pass_write_index: (index == 0).then_some(0),
            end_of_pass_write_index: (index == 1).then_some(1),
        };
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("timing_stamp_pass"),
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
            timestamp_writes: Some(writes),
            occlusion_query_set: None,
            multiview_mask: None,
        });
    };
    let period = queue.get_timestamp_period();
    let mut resources = CallbackResources::default();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(pane.x, pane.y));
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let frames: usize =
        std::env::var("PROBE_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(FRAMES);
    let mut samples = Vec::with_capacity(frames);
    for frame in 0..frames + 10 {
        let labels = names(runs.clone());
        let cb = LatticeCallback::from_scene(
            &scene,
            labels,
            egui::vec2(pane.x, pane.y),
            format,
            1,
            None,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        stamp(&mut encoder, 0);
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
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
        let ms = (ticks[1].wrapping_sub(ticks[0])) as f64 * f64::from(period) / 1.0e6;
        if frame >= 10 {
            samples.push(ms);
        }
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    let median = samples[samples.len() / 2];
    let (lo, hi) = (samples[samples.len() / 10], samples[samples.len() * 9 / 10]);
    eprintln!(
        "{what}: {named} names on {} lit nodes at {}x{}: prepare's encoder \
         {median:.3} ms/frame (p10 {lo:.3}, p90 {hi:.3}, {} frames)",
        scene.nodes.iter().filter(|n| n.activation > 0.0).count(),
        SIZE[0],
        SIZE[1],
        samples.len(),
    );
}
