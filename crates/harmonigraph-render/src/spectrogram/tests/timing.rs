//! What the spectrogram's cloud textures cost per frame, on whatever GPU runs
//! this. `#[ignore]`d — it prints figures and asserts nothing.
//!
//! ```sh
//! cargo test --release -p harmonigraph-render cloud_costs_by_style_and_dial \
//!   -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `PROBE_SIZE=WxH` (pixels, default 3840x2160), `PROBE_PPP` (default 2),
//! `PROBE_FRAMES` (default 60) and `PROBE_CASE` (comma-separated substrings of
//! the case names, any of which selects a case) narrow it, and `PROBE_FILLS`
//! replaces the pair of coverages below.
//! `docs/spectrogram-cloud-performance.md` holds the readings.
//! `PROBE_HISTORY_SECONDS` (default 10) is the visible span, not elapsed age;
//! it controls the time-softness scale, without changing the supplied grid.
//! Comma-separated values interleave multiple history durations in one run.
//! `PROBE_SLABS` (default 1024, range 2..=1024) controls the visible grid width.
//! The fixture uses a live-sized 1032-slot ring. Fill changes rasterized area,
//! not slab count: it is a coverage comparison, not simulated history growth.
//! Callback CPU time excludes aggregation, egui, submission and polling;
//! the fixed grid has no steady-state dirty uploads. Synchronous GPU waits
//! also mean these durations are not end-to-end DAW frame times.
//!
//! `PROBE_SPAN_SEMITONES` (default 96) is how much of the spectrum the pane
//! shows, and `PROBE_PITCH_SOFTNESS`, `PROBE_TIME_SOFTNESS` and `PROBE_SPREAD`
//! replace the light field's own dials in every case that draws one — which is
//! what replays a SAVED pane's settings instead of the fresh ones. They
//! override a case's `turn`, so a control that works by turning a softness to
//! zero is not one under them; `plain` still is.
//!
//! `PROBE_CLOUD_PIXEL` (points per cloud sample), `PROBE_CLOUD_TILE` (the
//! walk's period in cells, never 0 since #1100 retired the composite's live
//! walk) and `PROBE_BLUR_TIME_STEP`
//! (slabs per source texel on the light field's time axis, 0 for off), each
//! defaulting to the production values (0.5 pt, 40 cells and the saved blur
//! default), apply to EVERY case after its own `turn`, so one run reads the whole table at one setting
//! and two runs are what that setting is worth. They are probe controls rather than
//! cases because the question is how much each of the rows above falls, not how
//! one of them does — and they stack, which is the other thing two runs cannot
//! show.
//!
//! `PROBE_BLUR_TIME_STEP` now defaults to a step of ONE, so every case runs
//! with the light field's time axis capped unless the run says
//! `PROBE_BLUR_TIME_STEP=0`. Every figure in
//! `docs/spectrogram-cloud-performance.md` and in issue #1015 was taken before
//! that default landed, which is to say uncapped, and a fresh run is only
//! comparable to them at 0.
//!
//! `PROBE_BLUR_TIME_STEP` is measured against the slab width `quad` actually
//! lays out, `points.x * fill / PROBE_SLABS`, rather than against the whole
//! pane's: the fixture puts every slab inside the filled fraction, so at a fill
//! below one the data really is finer per point than the pane and the dial is
//! right to bound nothing. At `PROBE_FILLS=1` the two are the same number.

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
    (
        "terraces only",
        Some(|s| {
            (s.pitch_softness, s.time_softness, s.cloud_depth) = (0.0, 0.0, 0.0);
        }),
    ),
    ("mosaic, defaults", Some(|_| {})),
    ("mosaic, no terraces", Some(|s| s.contour_strength = 0.0)),
    ("mosaic, variety 0", Some(|s| s.scale_variety = 0.0)),
    ("mosaic, no blur", Some(|s| (s.pitch_softness, s.time_softness) = (0.0, 0.0))),
    ("watercolor, defaults", Some(watercolor)),
    ("watercolor, layers 0", Some(|s| (watercolor(s), s.wash_layers = 0.0).0)),
    ("watercolor, lobe 0", Some(|s| (watercolor(s), s.wash_lobe = 0.0).0)),
    (
        "watercolor, lobe 0 layers 0",
        Some(|s| {
            watercolor(s);
            (s.wash_lobe, s.wash_layers) = (0.0, 0.0);
        }),
    ),
    ("stars, defaults", Some(|s| s.cloud_style = CloudStyle::Stars)),
    ("stars, near-leaning", Some(|s| (s.cloud_style, s.star_balance) = (CloudStyle::Stars, 1.0))),
    (
        "stars, no speed spread",
        Some(|s| (s.cloud_style, s.star_speed_spread) = (CloudStyle::Stars, 0.0)),
    ),
];

struct Case {
    name: &'static str,
    fill: f32,
    history_seconds: f32,
    turn: Option<Turn>,
    cb: SpectrogramCallback,
    resources: CallbackResources,
    /// Light field, paint, and submit-to-completion, in ms.
    samples: [Vec<f64>; 3],
    gpu_total: Vec<f64>,
    gpu_from_open_begin: Vec<f64>,
    gpu_old_bracket: Vec<f64>,
    reversed_total: usize,
    reversed_old: usize,
    cpu_prepare: Vec<f64>,
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
    let histories: Vec<f32> = std::env::var("PROBE_HISTORY_SECONDS")
        .unwrap_or_else(|_| "10".to_owned())
        .split(',')
        .map(|v| {
            let seconds: f32 = v.trim().parse().expect("history duration in seconds");
            assert!(seconds.is_finite() && seconds > 0.0);
            seconds
        })
        .collect();
    let fills: Vec<f32> = std::env::var("PROBE_FILLS")
        .ok()
        .map(|v| v.split(',').map(|v| v.trim().parse().expect("a fill fraction")).collect())
        .unwrap_or_else(|| FILLS.to_vec());
    let dial =
        |name: &str| -> Option<f32> { std::env::var(name).ok().and_then(|v| v.parse().ok()) };
    let cloud_pixel = dial("PROBE_CLOUD_PIXEL");
    let cloud_tile = dial("PROBE_CLOUD_TILE");
    let blur_time_step = dial("PROBE_BLUR_TIME_STEP");
    let pitch_softness = dial("PROBE_PITCH_SOFTNESS");
    let time_softness = dial("PROBE_TIME_SOFTNESS");
    let spread = dial("PROBE_SPREAD");
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

    // A full-size live ring: 1024 slabs plus eight of retention headroom,
    // read over eight octaves. Smaller runs keep the same allocation.
    let slabs: u32 = std::env::var("PROBE_SLABS").ok().and_then(|v| v.parse().ok()).unwrap_or(1024);
    assert!((2..=1024).contains(&slabs));
    // The grid is the whole 20 Hz - 20 kHz spectrum; `span` is how much of it
    // the pane shows, which decides both the pitch footprint under one pixel
    // and the points per cent the pitch softness is measured in. The default
    // shows eight octaves of it; the live pane at full zoom-out shows all
    // 119.59 semitones.
    let span: f32 =
        std::env::var("PROBE_SPAN_SEMITONES").ok().and_then(|v| v.parse().ok()).unwrap_or(96.0);
    let bins = 3828u32;
    let grid = grid_of(noisy_grid(bins as usize, slabs as usize), bins, 1032, 0);
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
        count: 8,
    });
    let buffer = |label, usage| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: 64,
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
    // Paired begin/end samples compare the production timer's old bracket
    // (paint begin -> independent tail begin) with the proposed full bracket
    // (opening pass end -> paint end) on the same GPU submissions.
    let stamped_pass =
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
                        beginning_of_pass_write_index: Some(index),
                        end_of_pass_write_index: Some(index + 1),
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
            std::env::var("PROBE_CASE")
                .ok()
                .is_none_or(|v| v.split(',').any(|v| name.contains(v.trim())))
        })
        .flat_map(|&(name, turn)| fills.clone().into_iter().map(move |fill| (name, turn, fill)))
        .flat_map(|(name, turn, fill)| {
            histories.iter().map(move |&history_seconds| (name, turn, fill, history_seconds))
        })
        .map(|(name, turn, fill, history_seconds)| {
            let mut cb = callback(quad(fill), &grid, &read);
            cb.rect = rect;
            let mut resources = CallbackResources::default();
            let mut sampling = atmosphere::CloudSampling::default();
            if let Some(pixel) = cloud_pixel {
                sampling.pixel_points = pixel;
            }
            if let Some(period) = cloud_tile {
                sampling.tile_cells = period as u32;
            }
            resources.insert(sampling);
            Case {
                name,
                fill,
                history_seconds,
                turn,
                cb,
                resources,
                samples: Default::default(),
                gpu_total: Vec::new(),
                gpu_from_open_begin: Vec::new(),
                gpu_old_bracket: Vec::new(),
                reversed_total: 0,
                reversed_old: 0,
                cpu_prepare: Vec::new(),
            }
        })
        .collect();

    // INTERLEAVED: one frame of every case per round, so a GPU that another
    // process is also drawing on, or one that changes its clock mid-run, moves
    // every case together rather than whichever ran last.
    for frame in 0..frames + 10 {
        for case in &mut cases {
            let Case {
                turn,
                cb,
                resources,
                samples,
                gpu_total,
                gpu_from_open_begin,
                gpu_old_bracket,
                reversed_total,
                reversed_old,
                cpu_prepare,
                name,
                history_seconds,
                fill,
                ..
            } = case;
            cb.pass_nr = frame as u64;
            cb.atmosphere = turn.map(|turn| {
                let mut settings = SpectralAtmosphere::default();
                turn(&mut settings);
                // After the turn, so each case uses the requested blur step.
                if let Some(blur_time_step) = blur_time_step {
                    settings.blur_time_step = blur_time_step;
                }
                // The light field's own dials, for replaying a saved pane's
                // settings rather than the fresh ones. A case that turns a
                // softness to zero to BE a control loses that here, so a run
                // with these set wants `plain` as its control.
                if let Some(pitch_softness) = pitch_softness {
                    settings.pitch_softness = pitch_softness;
                }
                if let Some(time_softness) = time_softness {
                    settings.time_softness = time_softness;
                }
                if let Some(spread) = spread {
                    settings.spread = spread;
                }
                SpectrogramAtmosphere {
                    settings,
                    region: rect,
                    pitch_vertical: true,
                    points_per_cent: points.y / (span * 100.0),
                    points_per_ms: points.x / (*history_seconds * 1000.0),
                    // What `quad` lays out: every slab inside the filled
                    // fraction, so this is `w / n` and not the pane's width
                    // over the slab count.
                    points_per_slab: points.x * *fill / slabs as f32,
                    now: 1.0 + frame as f64 / 144.0,
                }
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            drop(stamped_pass(&mut encoder, &stamp_view, 0));
            let prepare_start = std::time::Instant::now();
            let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, resources);
            let prepare_ms = prepare_start.elapsed().as_secs_f64() * 1000.0;
            drop(stamped_pass(&mut encoder, &stamp_view, 2));
            {
                let mut pass = stamped_pass(&mut encoder, &pane_view, 4);
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
            drop(stamped_pass(&mut encoder, &stamp_view, 6));
            encoder.resolve_query_set(&set, 0..8, &resolve, 0);
            encoder.copy_buffer_to_buffer(&resolve, 0, &staging, 0, 64);
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
                assert!(
                    ticks.iter().all(|&tick| tick > 0),
                    "Metal returned a zero timestamp: {ticks:?}"
                );
                if ticks[5] < ticks[0] || ticks[5] < ticks[1] {
                    eprintln!("reversed full interval {name} {history_seconds}s: {ticks:?}");
                }
                if ticks[6] < ticks[4] && *reversed_old == 0 {
                    eprintln!("first old-bracket reversal {name} {history_seconds}s: {ticks:?}");
                }
                *reversed_total += usize::from(ticks[5] < ticks[0]);
                *reversed_old += usize::from(ticks[6] < ticks[4]);
                let ms =
                    |a: usize, b: usize| ticks[b].saturating_sub(ticks[a]) as f64 * period / 1.0e6;
                samples[0].push(ms(1, 3));
                samples[1].push(ms(3, 5));
                samples[2].push(wall_ms);
                gpu_total.push(ms(1, 5));
                gpu_from_open_begin.push(ms(0, 5));
                gpu_old_bracket.push(ms(4, 6));
                cpu_prepare.push(prepare_ms);
            }
        }
    }

    // Keep the historical split columns, but prefer the directly measured
    // total: independent timestamp passes can overlap on a tile-based GPU.
    // Split minima can be zero and do not establish an uncontended cost.
    eprintln!(
        "{:<38} {:>5} {:>8}  {:>15}  {:>15}  {:>15}  {:>10}  {:>10}  {:>10}  {:>10}  {:>10}  source px",
        "case",
        "fill",
        "span s",
        "light med/min",
        "paint med/min",
        "wall med/min",
        "end/full",
        "begin/full",
        "old med",
        "rev new/old",
        "CPU prep"
    );
    for case in &mut cases {
        let [light, paint, wall] = case.samples.each_mut().map(|samples| {
            samples.sort_by(f64::total_cmp);
            format!("{:>7.2}/{:>7.2}", samples[samples.len() / 2], samples[0])
        });
        case.gpu_total.sort_by(f64::total_cmp);
        case.gpu_from_open_begin.sort_by(f64::total_cmp);
        case.gpu_old_bracket.sort_by(f64::total_cmp);
        case.cpu_prepare.sort_by(f64::total_cmp);
        let source = case.cb.atmosphere.map(|a| atmosphere::source_size(size, ppp, a));
        eprintln!(
            "{:<38} {:>5.2} {:>8.1}  {light}  {paint}  {wall}  {:>10.3}  {:>10.3}  {:>10.3}  {:>4}/{:<4}  {:>10.3}  {source:?}",
            case.name,
            case.fill,
            case.history_seconds,
            case.gpu_total[case.gpu_total.len() / 2],
            case.gpu_from_open_begin[case.gpu_from_open_begin.len() / 2],
            case.gpu_old_bracket[case.gpu_old_bracket.len() / 2],
            case.reversed_total,
            case.reversed_old,
            case.cpu_prepare[case.cpu_prepare.len() / 2]
        );
        let spread = |samples: &[f64]| {
            let at = |fraction: usize| samples[(samples.len() - 1) * fraction / 100];
            format!("{:.3}/{:.3}/{:.3}/{:.3}/{:.3}", at(0), at(10), at(50), at(90), at(100))
        };
        eprintln!(
            "  {} {:.1}s GPU min/p10/med/p90/max begin/full={} end/full={} old={}",
            case.name,
            case.history_seconds,
            spread(&case.gpu_from_open_begin),
            spread(&case.gpu_total),
            spread(&case.gpu_old_bracket)
        );
    }
}
