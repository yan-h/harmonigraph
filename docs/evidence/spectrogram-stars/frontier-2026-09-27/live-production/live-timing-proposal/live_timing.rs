//! Temporary production timing evidence. Remove after capture.
//! Reuses the existing production SOURCE_QUERY timestamp hook only.
//! All pipelines, grouped resources and shader sources remain production paths.
use super::*;
use harmonigraph_scene::{CloudStyle, SpectralAtmosphere, StarHaloProfile};
type Turn = fn(&mut SpectralAtmosphere);
fn half(s: &mut SpectralAtmosphere) {
    s.cloud_style = CloudStyle::Stars;
    s.star_halo_profile = StarHaloProfile::Uniform;
    s.star_halo_resolution = 0.5;
    memory(s);
}
fn full(s: &mut SpectralAtmosphere) {
    half(s);
    s.star_halo_resolution = 1.0;
}
fn p2(s: &mut SpectralAtmosphere) {
    full(s);
    s.star_halo_profile = StarHaloProfile::P2;
}
fn p3(s: &mut SpectralAtmosphere) {
    full(s);
    s.star_halo_profile = StarHaloProfile::P3;
}
fn memory(s: &mut SpectralAtmosphere) {
    let defaults = SpectralAtmosphere::default();
    s.color_pickup = defaults.color_pickup;
    s.color_release = defaults.color_release;
}
const CASES: &[(&str, Option<Turn>)] = &[
    ("half-a", Some(half)),
    ("half-b", Some(half)),
    ("full-a", Some(full)),
    ("full-b", Some(full)),
    ("p2", Some(p2)),
    ("p3", Some(p3)),
];

struct Case<'a> {
    name: &'a str,
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
fn stars_live_production_timings() {
    let size: [u32; 2] = std::env::var("PROBE_SIZE")
        .ok()
        .map(|v| {
            let (w, h) = v.split_once('x').expect("PROBE_SIZE is WxH");
            [
                w.parse().expect("width in pixels"),
                h.parse().expect("height in pixels"),
            ]
        })
        .unwrap_or([3840, 2160]);
    let ppp: f32 = std::env::var("PROBE_PPP")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2.0);
    let frames: usize = std::env::var("PROBE_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(240);
    assert!(
        frames > 0 && frames <= 240 && frames % 12 == 0,
        "PROBE_FRAMES must be12..240, divisible by12 for balanced order"
    );
    let raw_path = std::env::var("PROBE_RAW").expect("PROBE_RAW required");
    assert!(
        std::env::var_os("PROBE_STAGES").is_none(),
        "stage timing is excluded"
    );
    assert!(
        std::env::var_os("RESEARCH_CASES").is_none(),
        "all six production cases are always measured"
    );
    let take = match std::env::var("PROBE_INPUT").ok().as_deref() {
        None | Some("synthetic") => false,
        Some("take") => true,
        Some(other) => panic!("unknown PROBE_INPUT {other:?}"),
    };
    let histories: Vec<f32> = std::env::var("PROBE_HISTORY_SECONDS")
        .unwrap_or_else(|_| if take { "20" } else { "10" }.to_owned())
        .split(',')
        .map(|v| {
            let seconds: f32 = v.trim().parse().expect("history duration in seconds");
            assert!(seconds.is_finite() && seconds > 0.0);
            seconds
        })
        .collect();
    let fills: Vec<f32> = std::env::var("PROBE_FILLS")
        .ok()
        .map(|v| {
            v.split(',')
                .map(|v| v.trim().parse().expect("a fill fraction"))
                .collect()
        })
        .unwrap_or_else(|| vec![1.0]);
    assert_eq!(fills.len(), 1, "research CSV requires one fill");
    assert_eq!(
        histories.len(),
        1,
        "research CSV requires one history duration"
    );
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
        panic!("GPU required for explicit production timing");
    };
    eprintln!("adapter: {:?}", adapter.get_info());
    let features = wgpu::Features::TIMESTAMP_QUERY;
    if !adapter.features().contains(features) {
        panic!("timestamp queries required for explicit production timing");
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: crate::device_limits(&adapter),
        required_features: features,
        ..Default::default()
    }))
    .expect("a device with timestamps");

    // Both fixtures use a live-sized ring with retention headroom. The take
    // contains exactly 960 slabs of 1024 bins; keep every one visible.
    let slabs: u32 = std::env::var("PROBE_SLABS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(if take { 960 } else { 1024 });
    if take {
        assert_eq!(slabs, 960, "the take fixture has exactly 960 visible slabs");
    } else {
        assert!((2..=1024).contains(&slabs));
    }
    let take_span = 12.0 * (9000.0_f32 / 70.0).log2();
    let span: f32 = if take {
        take_span
    } else {
        std::env::var("PROBE_SPAN_SEMITONES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(96.0)
    };
    let bins = if take { 1024 } else { 3828 };
    let take_slabs = take.then(|| {
        let bytes = std::fs::read("/private/tmp/stars-full-halo-compare/take-levels.u8")
            .expect("recorded spectrogram levels");
        assert_eq!(bytes.len(), 960 * 1024, "take fixture size");
        SpectrogramGrid::slabs_of(&bytes, bins as usize)
    });
    let take_palette = take.then(|| {
        let bytes = std::fs::read("/private/tmp/stars-full-halo-compare/palette.rgba")
            .expect("recorded spectrogram palette");
        assert_eq!(bytes.len(), 4096 * 4, "take palette size");
        Arc::new(
            bytes
                .chunks_exact(4)
                .map(|rgba| rgba.try_into().unwrap())
                .collect::<Vec<[u8; 4]>>(),
        )
    });
    let grid = if let Some(run) = &take_slabs {
        SpectrogramGrid {
            capacity: 1032,
            bins,
            first_key: 0,
            run: run.clone(),
            full_uploads: Arc::default(),
        }
    } else {
        grid_of(noisy_grid(bins as usize, slabs as usize), bins, 1032, 0)
    };
    let mut read = read_of(
        if take { 35.0 } else { SPECTRUM_MIN_MIDI + 10.0 },
        span,
        size[1],
    );
    read.level_per_step = 1.0 / 255.0;
    if take {
        read.spectrum_min_midi = 35.0;
        read.bins_per_semitone = 1024.0 / span;
        read.level_per_midi = 0.0;
    }
    let points = egui::vec2(size[0] as f32 / ppp, size[1] as f32 / ppp);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
    let quad = |fill: f32| {
        let (w, h, n) = (points.x * fill, points.y, slabs as f32);
        let v = |x: f32, y: f32| SpectrogramVertex {
            pos: [x, y],
            slab: x / w * n,
            t: 1.0 - y / h,
        };
        vec![
            v(0.0, 0.0),
            v(w, 0.0),
            v(w, h),
            v(0.0, 0.0),
            v(w, h),
            v(0.0, h),
        ]
    };

    // Same production two-query bracket; no stage instrumentation.
    let query_count = 2;
    let query_bytes = 16;
    let set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("timing_probe"),
        ty: wgpu::QueryType::Timestamp,
        count: query_count,
    });
    let buffer = |label, usage| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: query_bytes,
            usage,
            mapped_at_creation: false,
        })
    };
    let resolve = buffer(
        "timing_resolve",
        wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
    );
    let staging = buffer(
        "timing_staging",
        wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
    );
    let target = |label, size: [u32; 2]| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
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
    let screen = ScreenDescriptor {
        size_in_pixels: size,
        pixels_per_point: ppp,
    };
    eprintln!(
        "pane {}x{} px at {ppp} px/pt, {slabs} slabs of {bins} buckets",
        size[0], size[1]
    );
    if take {
        eprintln!("input: recorded take, cyclic scrolling one slab/frame, clock advancing 1/48 second/slab; 960 visible slabs in 1032 slots");
    }

    let mut cases: Vec<Case<'_>> = CASES
        .iter()
        .flat_map(|&(name, turn)| {
            fills
                .clone()
                .into_iter()
                .map(move |fill| (name, turn, fill))
        })
        .flat_map(|(name, turn, fill)| {
            histories
                .iter()
                .map(move |&history_seconds| (name, turn, fill, history_seconds))
        })
        .map(|(name, turn, fill, history_seconds)| {
            let mut cb = callback(quad(fill), &grid, &read);
            cb.rect = rect;
            if let Some(palette) = &take_palette {
                cb.shades.lut = palette.clone();
            }
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
    let warmup: usize = std::env::var("PROBE_WARMUP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    let offset: usize = std::env::var("PROBE_ORDER_OFFSET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let orders = orders(cases.len());
    assert_eq!(warmup, 60, "production comparison retains60 warmup rounds");
    assert_eq!(orders.len(), 12);
    let mut raw = String::from("frame,slot,case,gpu_ms,wall_ms\n");
    for frame in 0..frames + warmup {
        // All cases in this round see the same run. Sharing each slab's Arc
        // with the preceding run makes only the entering key a dirty upload.
        let frame_grid = take_slabs.as_ref().map(|source| {
            let run: Arc<[SpectrogramSlab]> = (0..slabs as usize)
                .map(|j| source[(frame + j) % source.len()].clone())
                .collect();
            SpectrogramGrid {
                first_key: frame as i64,
                run,
                ..grid.clone()
            }
        });
        for (slot, index) in orders[(frame + offset) % orders.len()]
            .iter()
            .copied()
            .enumerate()
        {
            let case = &mut cases[index];
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
            if let Some(frame_grid) = &frame_grid {
                cb.grid = frame_grid.clone();
            }
            cb.atmosphere = turn.map(|turn| {
                let mut settings = SpectralAtmosphere {
                    cloud_style: CloudStyle::Mosaic,
                    color_pickup: 0.0,
                    color_release: 0.0,
                    ..Default::default()
                };
                turn(&mut settings);
                settings.star_jitter = std::env::var("PROBE_JITTER")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0.5);
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
                if frame == 0 {
                    eprintln!("{name} settings: {settings:?}");
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
                    now: 1.0 + frame as f64 / if take { 48.0 } else { 144.0 },
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
            assert!(
                source_stamped,
                "Stars timing must begin in its real source pass"
            );
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
            encoder.resolve_query_set(&set, 0..query_count, &resolve, 0);
            encoder.copy_buffer_to_buffer(&resolve, 0, &staging, 0, query_bytes);
            let start = std::time::Instant::now();
            queue.submit(bufs.into_iter().chain([encoder.finish()]));
            let slice = staging.slice(..);
            slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("poll");
            let wall_ms = start.elapsed().as_secs_f64() * 1000.0;
            let ticks: Vec<u64> =
                bytemuck::cast_slice::<u8, u64>(&slice.get_mapped_range()).to_vec();
            staging.unmap();
            if frame >= warmup {
                assert!(ticks[0] > 0 && ticks[1] >= ticks[0],
                    "reversed/zero source-to-composite interval {name} {history_seconds}s: {ticks:?}");
                raw.push_str(&format!(
                    "{},{},{},{:.9},{:.9}",
                    frame - warmup,
                    slot,
                    name,
                    (ticks[1] - ticks[0]) as f64 * period / 1.0e6,
                    wall_ms
                ));
                raw.push('\n');
                wall.push(wall_ms);
                gpu_total.push((ticks[1] - ticks[0]) as f64 * period / 1.0e6);
                cpu_prepare.push(prepare_ms);
            }
        }
    }

    std::fs::write(&raw_path, raw).unwrap();
    eprintln!("production settings only; saved {raw_path};60 warmup,{frames} measured,order offset{offset}");
    for case in &mut cases {
        case.wall.sort_by(f64::total_cmp);
        case.gpu_total.sort_by(f64::total_cmp);
        case.cpu_prepare.sort_by(f64::total_cmp);
    }
    eprintln!("case / fill / span s: source/full GPU min/p10/med/p90/max ms; wall median; CPU prepare median");
    for case in &cases {
        let spread = |samples: &[f64]| {
            let at = |fraction: usize| samples[(samples.len() - 1) * fraction / 100];
            format!(
                "{:.3}/{:.3}/{:.3}/{:.3}/{:.3}",
                at(0),
                at(10),
                at(50),
                at(90),
                at(100)
            )
        };
        let median = |samples: &[f64]| samples[samples.len() / 2];
        let source = case
            .cb
            .atmosphere
            .map(|a| atmosphere::source_size(size, ppp, a));
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
    }
}

fn orders(n: usize) -> Vec<Vec<usize>> {
    fn perm(v: &mut Vec<usize>, at: usize, out: &mut Vec<Vec<usize>>) {
        if at == v.len() {
            out.push(v.clone());
            return;
        }
        for i in at..v.len() {
            v.swap(at, i);
            perm(v, at + 1, out);
            v.swap(at, i);
        }
    }
    let mut out = Vec::new();
    if n <= 4 {
        perm(&mut (0..n).collect(), 0, &mut out);
    } else {
        for reverse in [false, true] {
            for offset in 0..n {
                out.push(
                    (0..n)
                        .map(|i| {
                            if reverse {
                                (offset + n - i) % n
                            } else {
                                (offset + i) % n
                            }
                        })
                        .collect(),
                );
            }
        }
    }
    out
}
