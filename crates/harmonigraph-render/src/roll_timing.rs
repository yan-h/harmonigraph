//! A GPU-timer probe: what a frame of the piano roll costs to draw, over an
//! empty-roll control. `#[ignore]`d — it prints figures and asserts nothing.
//!
//! The interval is `crate::frame_timer`'s, shared with the lattice probe: from
//! an empty 1x1 pass encoded ahead of `prepare` to the END of the pass `paint`
//! draws into, in one command buffer. That covers `prepare` (the bloom's notes
//! and chain, the body holdout), the shared spectral shadow's `finish` (the
//! Gaussian atlas's ink and blur) and `paint` (outline layer, core layer, bloom
//! composite). The closing pass READS the holdout, the atlas and the bloom's
//! quarter, so its end is ordered after every pass that wrote them (#1113).
//! The control's paint draws nothing, which is what the timer's no-pixel draw
//! is for.
//!
//! ```text
//! cargo test --release -p harmonigraph-render a_frame_of_the_roll \
//!   -- --ignored --nocapture --test-threads=1
//! ```
//! `PROBE_SIZE=WxH` (device pixels, default 3840x1000: the roll is a strip),
//! `PROBE_PPP` (default 2), `PROBE_FRAMES` (default 120) and `PROBE_CASE` (a
//! substring of the case name, e.g. `dense` or `Distance`).
//!
//! The notes are synthetic, since the pane that builds them lives in
//! `harmonigraph-ui`: ribbons at the pane's default width (0.3 semitone over
//! 72, floored at 1.5 pt) and length floor (1 pt), mostly plain held boxes with
//! leads, glides, tapers, two-piece fades and tremolo mixed in.
//! Both workloads are far denser than a real one: the live pane (about
//! 420x674 pt, 2026-10-09) showing a whole recorded take drew 51 pieces, for
//! about 1.1 ms over the empty roll under Gaussian and 0.7 ms under Distance
//! (#1458). `PROBE_SIZE=839x1348` is that pane.

use super::*;

const AXES: RollAxes = RollAxes { pitch_dir: [0.0, -1.0], depth_dir: [1.0, 0.0] };
const SPAN_SEMITONES: f32 = 72.0;

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// A deterministic stream in 0..1, so every run draws the same notes.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f32 {
        self.0 =
            self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 40) as f32) / ((1u64 << 24) as f32)
    }
}

struct Workload {
    name: &'static str,
    span_seconds: f32,
    notes: usize,
    duration: (f32, f32),
}

const WORKLOADS: [Workload; 2] = [
    Workload { name: "normal 10 s", span_seconds: 10.0, notes: 300, duration: (0.1, 1.5) },
    Workload { name: "dense 600 s", span_seconds: 600.0, notes: 3000, duration: (0.05, 0.4) },
];

fn instances(work: &Workload, points: [f32; 2], reach: f32) -> Vec<RollInstance> {
    let mut rng = Lcg(0x005e_ed0f_a011);
    let per_second = points[0] / work.span_seconds;
    let semitone = points[1] / SPAN_SEMITONES;
    let half_pitch = (0.15 * semitone).max(0.75);
    let mut out = Vec::with_capacity(work.notes + work.notes / 5);
    for i in 0..work.notes {
        let start = rng.next() * work.span_seconds;
        let duration = work.duration.0 + rng.next() * (work.duration.1 - work.duration.0);
        let key = (rng.next() * SPAN_SEMITONES).floor();
        let half_depth = (0.5 * duration * per_second).max(0.5);
        let shade = (key / SPAN_SEMITONES * 200.0) as u8;
        let mut note = RollInstance {
            center: [(start + 0.5 * duration) * per_second, points[1] - (key + 0.5) * semitone],
            half_extent: [half_pitch, half_depth],
            shear: 0.0,
            lead: 0.0,
            lead_fade: 0.0,
            lead_alpha: 0.0,
            cap_reach: reach,
            core: [230, 40 + shade / 2, 255 - shade, 255],
            outline: [0, 0, 0, 255],
            span: RollInstance::WHOLE,
            ramp: [0.0, 0.0],
            fade: [1.0, 1.0],
            taper_depth: [0.0; 4],
            taper: RollInstance::UNTAPERED,
            tremolo: [0.0; 2],
        };
        if i % 7 == 0 && half_depth > 4.0 {
            note.lead = 0.4 * half_depth;
            note.lead_fade = if i % 2 == 0 { 0.0 } else { 0.5 * note.lead };
            note.lead_alpha = if i % 3 == 0 { 0.5 } else { 0.0 };
        }
        if i % 11 == 0 {
            note.shear = 0.02 + 0.2 * rng.next();
        }
        if i % 13 == 0 {
            note.taper_depth = [-half_depth, -half_depth, half_depth, half_depth];
            note.taper = [0.6, 0.6, 1.0, 1.0];
        }
        if i % 17 == 0 {
            note.tremolo = [rng.next() * 6.0, 1.0];
        }
        if i % 5 == 0 {
            let (cut, ramp) = (0.2 * half_depth, [-half_depth, half_depth]);
            out.push(RollInstance { span: [f32::MIN, cut], ramp, fade: [1.0, 0.6], ..note });
            out.push(RollInstance { span: [cut, f32::MAX], ramp, fade: [1.0, 0.6], ..note });
        } else {
            out.push(note);
        }
    }
    out
}

#[test]
#[ignore = "a probe: prints a timing and asserts nothing"]
fn a_frame_of_the_roll_costs_this_much() {
    let size: [u32; 2] = std::env::var("PROBE_SIZE")
        .ok()
        .map(|v| {
            let (w, h) = v.split_once('x').expect("PROBE_SIZE is WxH");
            [w.parse().expect("width"), h.parse().expect("height")]
        })
        .unwrap_or([3840, 1000]);
    let ppp: f32 = env("PROBE_PPP", 2.0);
    let frames: usize = env("PROBE_FRAMES", 120);
    let filter = std::env::var("PROBE_CASE").ok();

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let Some(timer) = crate::frame_timer::FrameTimer::new(format) else { return };
    let pane = timer.pane(size, ppp);
    let rect = pane.rect;
    let points = [rect.width(), rect.height()];
    let bloom = crate::bloom_strength(harmonigraph_scene::ViewConfig::default().note_bloom);

    // Median and p10 of `frames` frames of one roll, after the timer's warm-up.
    let time = |instances: Vec<RollInstance>, shadow: harmonigraph_scene::ShadowStyle| {
        let cb = RollCallback {
            point_scale: 1.0,
            rect,
            pane_size: rect.size(),
            instances,
            clipped_tail: None,
            axes: AXES,
            bloom,
            shadow,
            target_format: format,
            pane_id: 0,
            shadow_surface_id: 0,
            pass_nr: 0,
        };
        let mut resources = CallbackResources::default();
        let mut samples: Vec<f64> = timer
            .time_frames(frames, |_| {
                timer.frame(
                    &pane,
                    &mut resources,
                    crate::frame_timer::Opening::AheadOfPrepare,
                    |encoder, resources| {
                        let (device, queue) = (&timer.device, &timer.queue);
                        let bufs = cb.prepare(device, queue, &pane.screen, encoder, resources);
                        crate::spectral_shadow::finish(
                            device,
                            queue,
                            &pane.screen,
                            encoder,
                            resources,
                            0,
                        );
                        bufs
                    },
                    |pass, resources| cb.paint(pane.paint_info(), pass, resources),
                )
            })
            .into_iter()
            .map(|sample| sample.gpu_ms)
            .collect();
        samples.sort_by(f64::total_cmp);
        (samples[samples.len() / 2], samples[samples.len() / 10])
    };

    eprintln!(
        "roll probe: {}x{} px at {ppp} ppp, {frames} frames, bloom {bloom}",
        size[0], size[1]
    );
    let gaussian = harmonigraph_scene::ShadowSettings::default().spectral_geometry;
    let shadows = [
        gaussian,
        harmonigraph_scene::ShadowStyle {
            kernel: harmonigraph_scene::ShadowKernel::Distance,
            ..gaussian
        },
    ];
    let (control, control_p10) = time(Vec::new(), gaussian);
    eprintln!("control (empty roll): median {control:.3} ms, p10 {control_p10:.3} ms");
    for work in &WORKLOADS {
        for shadow in shadows {
            let case = format!("{} / {:?}", work.name, shadow.kernel);
            if filter.as_ref().is_some_and(|f| !case.contains(f.as_str())) {
                continue;
            }
            let reach = crate::shadow::spectral_shadow_reach(shadow, 1.0);
            let notes = instances(work, points, reach);
            let count = notes.len();
            let (median, p10) = time(notes, shadow);
            eprintln!(
                "{case}: {count} instances, reach {reach:.2} pt: median {median:.3} ms \
                 (p10 {p10:.3}); the roll alone {:.3} ms (p10 {:.3})",
                median - control,
                p10 - control_p10,
            );
        }
    }
    eprintln!("METAL_TIMING_ASSETS {:?}", crate::shader_assets::statistics());
}
