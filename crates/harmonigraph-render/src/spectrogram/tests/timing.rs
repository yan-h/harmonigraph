//! What the spectrogram's cloud textures cost per frame, on whatever GPU runs
//! this. `#[ignore]`d — it prints figures and asserts nothing.
//!
//! ```sh
//! cargo test --release -p harmonigraph-render cloud_costs_by_style_and_dial \
//!   -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `PROBE_SIZE=WxH` (pixels, default 3840x2160), `PROBE_PPP` (default 2),
//! `PROBE_FRAMES` (default 60) and `PROBE_CASE` (a substring of a case's name)
//! narrow it. `docs/spectrogram-cloud-performance.md` holds the readings.

use super::*;
use harmonigraph_scene::{CloudStyle, SpectralAtmosphere};

type Turn = fn(&mut SpectralAtmosphere);

/// How much of the pane's time axis the measured history covers. The rest is
/// the backdrop's alone, so the pair of fills separates what the backdrop
/// costs UNDER the history from what the texture costs at all.
const FILLS: [f32; 2] = [1.0, 0.02];

fn watercolor(s: &mut SpectralAtmosphere) {
    s.cloud_style = CloudStyle::Watercolor;
}

const CASES: &[(&str, Option<Turn>)] = &[
    ("plain", None),
    ("blur only", Some(|s| (s.contour_strength, s.cloud_depth) = (0.0, 0.0))),
    ("blur + terraces", Some(|s| s.cloud_depth = 0.0)),
    ("mosaic, defaults", Some(|_| {})),
    ("mosaic, no terraces", Some(|s| s.contour_strength = 0.0)),
    ("mosaic, variety 0", Some(|s| s.scale_variety = 0.0)),
    ("mosaic, rock 1", Some(|s| s.scale_rock = 1.0)),
    ("mosaic, no blur", Some(|s| (s.pitch_softness, s.time_softness) = (0.0, 0.0))),
    ("watercolor, defaults", Some(watercolor)),
    ("watercolor, layers 0", Some(|s| (watercolor(s), s.wash_layers = 0.0).0)),
    ("watercolor, lobe 0", Some(|s| (watercolor(s), s.wash_lobe = 0.0).0)),
    ("watercolor, ragged 0", Some(|s| (watercolor(s), s.wash_ragged = 0.0).0)),
    (
        "watercolor, lobe 0 ragged 0 layers 0",
        Some(|s| {
            watercolor(s);
            (s.wash_lobe, s.wash_ragged, s.wash_layers) = (0.0, 0.0, 0.0);
        }),
    ),
];

struct Case {
    name: &'static str,
    fill: f32,
    turn: Option<Turn>,
    cb: SpectrogramCallback,
    resources: CallbackResources,
    /// Light field, paint, and submit-to-completion, in ms.
    samples: [Vec<f64>; 3],
}

#[test]
#[ignore = "a probe: prints timings and asserts nothing"]
fn cloud_costs_by_style_and_dial() {
    let size: [u32; 2] = std::env::var("PROBE_SIZE")
        .ok()
        .map(|v| {
            let (w, h) = v.split_once('x').expect("PROBE_SIZE is WxH");
            [w.parse().expect("width in pixels"), h.parse().expect("height in pixels")]
        })
        .unwrap_or([3840, 2160]);
    let ppp: f32 = std::env::var("PROBE_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(2.0);
    let frames: usize =
        std::env::var("PROBE_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
    crate::shader_assets::initialize();
    let instance = wgpu::Instance::default();
    let Ok(adapter) =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        eprintln!("no GPU adapter; nothing timed");
        return;
    };
    eprintln!("adapter: {:?}", adapter.get_info());
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

    // The analyzer's own shape: a whole-song ring of 3828-bucket slabs, of
    // which a thousand are on screen, read over eight octaves.
    let (bins, slabs, span) = (3828u32, 1024u32, 96.0f32);
    let grid = grid_of(noisy_grid(bins as usize, slabs as usize), bins, 4096, 0);
    let mut read = read_of(SPECTRUM_MIN_MIDI + 10.0, span, size[1]);
    read.level_per_step = 1.0 / 255.0;
    let points = egui::vec2(size[0] as f32 / ppp, size[1] as f32 / ppp);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
    let quad = |fill: f32| {
        let (w, h, n) = (points.x * fill, points.y, slabs as f32);
        let v = |x: f32, y: f32| SpectrogramVertex { pos: [x, y], slab: x / w * n, t: 1.0 - y / h };
        vec![v(0.0, 0.0), v(w, 0.0), v(w, h), v(0.0, 0.0), v(w, h), v(0.0, h)]
    };

    let set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("timing_probe"),
        ty: wgpu::QueryType::Timestamp,
        count: 3,
    });
    let buffer = |label, usage| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: 24,
            usage,
            mapped_at_creation: false,
        })
    };
    let resolve =
        buffer("timing_resolve", wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC);
    let staging =
        buffer("timing_staging", wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ);
    let target = |label, size: [u32; 2]| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    let stamp_view = target("timing_stamp", [1, 1]);
    // Held across frames, as a swapchain's is: allocating 33 MB per frame would
    // be the larger half of what a wall clock read.
    let pane_view = target("timing_pane", size);
    // Every stamp is the END of a pass's fragment stage. A tile-based GPU runs
    // a later pass's vertex stage ahead of an earlier pass's fragments, so a
    // beginning-of-pass stamp lands before the work it is meant to follow;
    // fragment stages alone finish in order.
    let pass_ending_at =
        |encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, index: u32| {
            encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("timing_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                        query_set: &set,
                        beginning_of_pass_write_index: None,
                        end_of_pass_write_index: Some(index),
                    }),
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime()
        };
    let period = f64::from(queue.get_timestamp_period());
    let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: ppp };
    eprintln!("pane {}x{} px at {ppp} px/pt, {slabs} slabs of {bins} buckets", size[0], size[1]);

    let mut cases: Vec<Case> = CASES
        .iter()
        .filter(|(name, _)| {
            std::env::var("PROBE_CASE").ok().is_none_or(|v| name.contains(v.as_str()))
        })
        .flat_map(|&(name, turn)| FILLS.map(|fill| (name, turn, fill)))
        .map(|(name, turn, fill)| {
            let mut cb = callback(quad(fill), &grid, &read);
            cb.rect = rect;
            let resources = CallbackResources::default();
            Case { name, fill, turn, cb, resources, samples: Default::default() }
        })
        .collect();

    // INTERLEAVED: one frame of every case per round, so a GPU that another
    // process is also drawing on, or one that changes its clock mid-run, moves
    // every case together rather than whichever ran last.
    for frame in 0..frames + 10 {
        for case in &mut cases {
            let Case { turn, cb, resources, samples, .. } = case;
            cb.pass_nr = frame as u64;
            cb.atmosphere = turn.map(|turn| {
                let mut settings = SpectralAtmosphere::default();
                turn(&mut settings);
                SpectrogramAtmosphere {
                    settings,
                    region: rect,
                    pitch_vertical: true,
                    points_per_cent: points.y / (span * 100.0),
                    // Ten seconds of history across the pane.
                    points_per_ms: points.x / 10_000.0,
                    now: 1.0 + frame as f64 / 144.0,
                }
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            drop(pass_ending_at(&mut encoder, &stamp_view, 0));
            let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, resources);
            drop(pass_ending_at(&mut encoder, &stamp_view, 1));
            {
                let mut pass = pass_ending_at(&mut encoder, &pane_view, 2);
                cb.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: ppp,
                        screen_size_px: size,
                    },
                    &mut pass,
                    resources,
                );
            }
            encoder.resolve_query_set(&set, 0..3, &resolve, 0);
            encoder.copy_buffer_to_buffer(&resolve, 0, &staging, 0, 24);
            let start = std::time::Instant::now();
            queue.submit(bufs.into_iter().chain([encoder.finish()]));
            let slice = staging.slice(..);
            slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            let wall_ms = start.elapsed().as_secs_f64() * 1000.0;
            let ticks: Vec<u64> =
                bytemuck::cast_slice::<u8, u64>(&slice.get_mapped_range()).to_vec();
            staging.unmap();
            if frame >= 10 {
                let ms =
                    |a: usize, b: usize| ticks[b].saturating_sub(ticks[a]) as f64 * period / 1.0e6;
                samples[0].push(ms(0, 1));
                samples[1].push(ms(1, 2));
                samples[2].push(wall_ms);
            }
        }
    }

    // The minimum beside the median: on a shared GPU the fastest frame is the
    // one nothing else interrupted, and it is the steadier of the two.
    eprintln!(
        "{:<38} {:>5}  {:>15}  {:>15}  {:>15}",
        "case", "fill", "light med/min", "paint med/min", "wall med/min"
    );
    for case in &mut cases {
        let [light, paint, wall] = case.samples.each_mut().map(|samples| {
            samples.sort_by(f64::total_cmp);
            format!("{:>7.2}/{:>7.2}", samples[samples.len() / 2], samples[0])
        });
        eprintln!("{:<38} {:>5.2}  {light}  {paint}  {wall}", case.name, case.fill);
    }
}
