//! What the spectrogram's cloud textures cost per frame, on whatever GPU runs
//! this. `#[ignore]`d — it prints figures and asserts timestamp ordering.
//! The GPU interval starts on the first real source pass (or paint for the
//! plain case), and ends on the final composite. Historical
//! `end/full`, `begin/full`, light and paint columns used independent stamp
//! passes and are not comparable to this `source/full` interval (#1203).
//! The stamps, the pane and the readback are `crate::frame_timer`'s, shared
//! with the lattice's and roll's probes; only this opening is its own.
//! Memory is off except in explicitly named memory cases; Watercolor is
//! selected explicitly because the production default style can change.
//!
//! ```sh
//! cargo test --release -p harmonigraph-render cloud_costs_by_style_and_dial \
//!   -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `PROBE_SIZE=WxH` (pixels, default 3840x2160), `PROBE_PPP` (default 2),
//! `PROBE_FRAMES` (default 60) and `PROBE_CASE` (comma-separated substrings of
//! the case names, any of which selects a case) narrow it, and `PROBE_FILLS`
//! replaces the pair of coverages below. `PROBE_CASE=75%` selects both the
//! default-resolution memory-off and memory-on cases.
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
//! shows, and `PROBE_PITCH_SOFTNESS` and `PROBE_TIME_SOFTNESS`
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
use crate::frame_timer::{FrameTimer, Opening, WARM_UP};
use harmonigraph_scene::{CloudStyle, SpectralAtmosphere};

type Turn = fn(&mut SpectralAtmosphere);

/// How much of the pane's time axis the measured history covers. The rest is
/// the backdrop's alone, so the pair of fills separates what the backdrop
/// costs UNDER the history from what the texture costs at all.
const FILLS: [f32; 2] = [1.0, 0.02];

fn memory(s: &mut SpectralAtmosphere) {
    let defaults = SpectralAtmosphere::default();
    s.color_pickup = defaults.color_pickup;
    s.color_release = defaults.color_release;
}

const CASES: &[(&str, Option<Turn>)] = &[
    ("plain", None),
    ("blur only", Some(|s| s.cloud_depth = 0.0)),
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
    ("watercolor, defaults", Some(|_| {})),
    ("watercolor, no blur", Some(|s| (s.pitch_softness, s.time_softness) = (0.0, 0.0))),
    ("watercolor, layers 0", Some(|s| s.material_settings.wash_layers = 0.0)),
    ("watercolor, lobe 0", Some(|s| s.material_settings.wash_lobe = 0.0)),
    (
        "watercolor, lobe 0 layers 0",
        Some(|s| (s.material_settings.wash_lobe, s.material_settings.wash_layers) = (0.0, 0.0)),
    ),
    (
        "stars, 100%",
        Some(|s| {
            s.cloud_style = CloudStyle::Stars;
            s.stars.star_resolution = 1.0;
        }),
    ),
    ("stars, 75%", Some(|s| s.cloud_style = CloudStyle::Stars)),
    (
        "stars, 50%",
        Some(|s| {
            s.cloud_style = CloudStyle::Stars;
            s.stars.star_resolution = 0.5;
        }),
    ),
    ("watercolor, memory", Some(memory)),
    (
        "stars, 75% memory",
        Some(|s| {
            s.cloud_style = CloudStyle::Stars;
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
    let Some(timer) = FrameTimer::new(FORMAT) else { return };

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
    let pane = timer.pane(size, ppp);
    let (rect, points) = (pane.rect, pane.rect.size());
    let quad = |fill: f32| {
        let (w, h, n) = (points.x * fill, points.y, slabs as f32);
        let v = |x: f32, y: f32| SpectrogramVertex { pos: [x, y], slab: x / w * n, t: 1.0 - y / h };
        vec![v(0.0, 0.0), v(w, 0.0), v(w, h), v(0.0, 0.0), v(w, h), v(0.0, h)]
    };

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
    for frame in 0..frames + WARM_UP {
        for case in &mut cases {
            let Case {
                turn,
                cb,
                resources,
                wall,
                gpu_total,
                cpu_prepare,
                history_seconds,
                fill,
                ..
            } = case;
            cb.pass_nr = frame as u64;
            cb.atmosphere = turn.map(|turn| {
                let mut settings = SpectralAtmosphere {
                    cloud_style: CloudStyle::Watercolor,
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
                    history_epoch: 0,
                }
            });
            let cb = &*cb;
            // The source pass consumes the query when it actually encodes.
            // With no source (plain), the closing pass opens the frame too.
            let source_stamped = || SOURCE_QUERY.with_borrow_mut(|query| query.take().is_none());
            let sample = timer.frame(
                &pane,
                resources,
                Opening::InPrepare(&source_stamped),
                |encoder, resources| {
                    SOURCE_QUERY.with_borrow_mut(|query| *query = Some(timer.query_set().clone()));
                    cb.prepare(&timer.device, &timer.queue, &pane.screen, encoder, resources)
                },
                |pass, resources| cb.paint(pane.paint_info(), pass, resources),
            );
            if frame >= WARM_UP {
                wall.push(sample.completion_ms);
                gpu_total.push(sample.gpu_ms);
                cpu_prepare.push(sample.prepare_cpu_ms);
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
        // `prepare` draws no light field when no effect is on, so neither is
        // one reported.
        let source = case
            .cb
            .atmosphere
            .filter(|a| a.settings.sanitized().effects().light())
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
        let baseline = if case.name.ends_with("memory") {
            "watercolor, memory"
        } else {
            "watercolor, defaults"
        };
        if let Some(watercolor) = cases.iter().find(|c| {
            c.name == baseline && c.fill == case.fill && c.history_seconds == case.history_seconds
        }) {
            let watercolor_ms = median(&watercolor.gpu_total);
            eprintln!(
                "  {:.3}x Watercolor ({watercolor_ms:.3} ms, {baseline})",
                median(&case.gpu_total) / watercolor_ms
            );
        }
    }
}
