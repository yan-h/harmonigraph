//! What the spectrogram's cloud textures cost per frame, on whatever GPU runs
//! this. `#[ignore]`d — it prints figures and asserts timestamp ordering.
//! The GPU interval starts on the first real source pass (or paint for plain
//! and terraces-only cases), and ends on the final composite. Historical
//! `end/full`, `begin/full`, light and paint columns used independent stamp
//! passes and are not comparable to this `source/full` interval (#1203).
//! Memory is off except in explicitly named memory cases; Mosaic is selected
//! explicitly because the production default style can change.
//!
//! ```sh
//! cargo test --release -p harmonigraph-render cloud_costs_by_style_and_dial \
//!   -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `PROBE_SIZE=WxH` (pixels, default 3840x2160), `PROBE_PPP` (default 2),
//! `PROBE_FRAMES` (default 60) and `PROBE_CASE` (comma-separated substrings of
//! the case names, any of which selects a case) narrow it, and `PROBE_FILLS`
//! replaces the pair of coverages below. `PROBE_CASE=medium` selects both the
//! explicit Medium memory-off and Medium memory-on cases.
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

fn memory(s: &mut SpectralAtmosphere) {
    let defaults = SpectralAtmosphere::default();
    s.color_pickup = defaults.color_pickup;
    s.color_release = defaults.color_release;
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
    ("velvet, defaults", Some(|s| s.cloud_style = CloudStyle::VelvetScales)),
    (
        "velvet, small cells",
        Some(|s| {
            s.cloud_style = CloudStyle::VelvetScales;
            s.material_settings.velvet_size = 0.2;
        }),
    ),
    (
        "velvet, memory",
        Some(|s| {
            s.cloud_style = CloudStyle::VelvetScales;
            memory(s);
        }),
    ),
    ("mosaic, defaults", Some(|_| {})),
    ("mosaic, no terraces", Some(|s| s.contour_strength = 0.0)),
    ("mosaic, variety 0", Some(|s| s.material_settings.scale_variety = 0.0)),
    ("mosaic, no blur", Some(|s| (s.pitch_softness, s.time_softness) = (0.0, 0.0))),
    ("watercolor, defaults", Some(watercolor)),
    ("watercolor, layers 0", Some(|s| (watercolor(s), s.material_settings.wash_layers = 0.0).0)),
    ("watercolor, lobe 0", Some(|s| (watercolor(s), s.material_settings.wash_lobe = 0.0).0)),
    (
        "watercolor, lobe 0 layers 0",
        Some(|s| {
            watercolor(s);
            (s.material_settings.wash_lobe, s.material_settings.wash_layers) = (0.0, 0.0);
        }),
    ),
    (
        "stars, high",
        Some(|s| {
            s.cloud_style = CloudStyle::Stars;
            s.stars.star_halo_profile = harmonigraph_scene::StarHaloProfile::P3;
        }),
    ),
    (
        "stars, medium",
        Some(|s| {
            s.cloud_style = CloudStyle::Stars;
            s.stars.star_halo_profile = harmonigraph_scene::StarHaloProfile::Medium;
        }),
    ),
    (
        "stars, low",
        Some(|s| {
            s.cloud_style = CloudStyle::Stars;
            s.stars.star_halo_profile = harmonigraph_scene::StarHaloProfile::Low;
        }),
    ),
    ("mosaic, memory", Some(memory)),
    (
        "watercolor, memory",
        Some(|s| {
            watercolor(s);
            memory(s);
        }),
    ),
    (
        "stars, medium memory",
        Some(|s| {
            s.cloud_style = CloudStyle::Stars;
            s.stars.star_halo_profile = harmonigraph_scene::StarHaloProfile::Medium;
            memory(s);
        }),
    ),
];

struct Case {
    name: &'static str,
    fill: f32,
    history_seconds: f32,
    turn: Option<Turn>,
    cb: SpectrogramCallback,
    resources: CallbackResources,
    wall: Vec<f64>,
    gpu_total: Vec<f64>,
    cpu_prepare: Vec<f64>,
}

#[test]
#[ignore = "a probe: prints timings and asserts timestamp ordering"]
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
    assert!(frames > 0, "PROBE_FRAMES must be positive");
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
        required_limits: crate::device_limits(&adapter),
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
        count: 2,
    });
    let buffer = |label, usage| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: 16,
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
    // Held across frames, as a swapchain's is: allocating 33 MB per frame would
    // be the larger half of what a wall clock read.
    let pane_view = target("timing_pane", size);
    let stamped_pass = |encoder: &mut wgpu::CommandEncoder, source_stamped: bool| {
        encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("timing_paint"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &pane_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                timestamp_writes: Some(wgpu::RenderPassTimestampWrites {
                    query_set: &set,
                    beginning_of_pass_write_index: (!source_stamped).then_some(0),
                    end_of_pass_write_index: Some(1),
                }),
                ..Default::default()
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
                wall: Vec::new(),
                gpu_total: Vec::new(),
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
                wall,
                gpu_total,
                cpu_prepare,
                name,
                history_seconds,
                fill,
                ..
            } = case;
            cb.pass_nr = frame as u64;
            cb.atmosphere = turn.map(|turn| {
                let mut settings = SpectralAtmosphere {
                    cloud_style: CloudStyle::Mosaic,
                    color_pickup: 0.0,
                    color_release: 0.0,
                    ..Default::default()
                };
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
            SOURCE_QUERY.with_borrow_mut(|query| *query = Some(set.clone()));
            let prepare_start = std::time::Instant::now();
            let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, resources);
            let prepare_ms = prepare_start.elapsed().as_secs_f64() * 1000.0;
            // The source consumes the query when it actually encodes a pass.
            // With no source (plain or terraces only), paint owns both stamps.
            let source_stamped = SOURCE_QUERY.with_borrow_mut(|query| query.take().is_none());
            {
                let mut pass = stamped_pass(&mut encoder, source_stamped);
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
            encoder.resolve_query_set(&set, 0..2, &resolve, 0);
            encoder.copy_buffer_to_buffer(&resolve, 0, &staging, 0, 16);
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
                assert!(ticks[0] > 0 && ticks[1] >= ticks[0],
                    "reversed/zero source-to-composite interval {name} {history_seconds}s: {ticks:?}");
                wall.push(wall_ms);
                gpu_total.push((ticks[1] - ticks[0]) as f64 * period / 1.0e6);
                cpu_prepare.push(prepare_ms);
            }
        }
    }

    for case in &mut cases {
        case.wall.sort_by(f64::total_cmp);
        case.gpu_total.sort_by(f64::total_cmp);
        case.cpu_prepare.sort_by(f64::total_cmp);
    }
    eprintln!("case / fill / span s: source/full GPU min/p10/med/p90/max ms; wall median; CPU prepare median");
    for case in &cases {
        let spread = |samples: &[f64]| {
            let at = |fraction: usize| samples[(samples.len() - 1) * fraction / 100];
            format!("{:.3}/{:.3}/{:.3}/{:.3}/{:.3}", at(0), at(10), at(50), at(90), at(100))
        };
        let median = |samples: &[f64]| samples[samples.len() / 2];
        let source = case.cb.atmosphere.map(|a| atmosphere::source_size(size, ppp, a));
        eprintln!(
            "{} / {:.2} / {:.1}: {}; wall {:.3}; CPU {:.3}; source px {:?}",
            case.name,
            case.fill,
            case.history_seconds,
            spread(&case.gpu_total),
            median(&case.wall),
            median(&case.cpu_prepare),
            source
        );
        let baseline =
            if case.name.ends_with("memory") { "mosaic, memory" } else { "mosaic, defaults" };
        if let Some(mosaic) = cases.iter().find(|c| {
            c.name == baseline && c.fill == case.fill && c.history_seconds == case.history_seconds
        }) {
            let mosaic_ms = median(&mosaic.gpu_total);
            eprintln!(
                "  {:.3}x Mosaic ({mosaic_ms:.3} ms, {baseline})",
                median(&case.gpu_total) / mosaic_ms
            );
        }
    }
}
