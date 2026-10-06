//! A GPU-timer probe: what a frame of the piano roll costs to draw, over an
//! empty-roll control. `#[ignore]`d — it prints figures and asserts nothing.
//!
//! The interval is the lattice probe's (`lattice_tests/timing.rs`): from the
//! BEGINNING of an empty 1x1 pass encoded ahead of `prepare` to the END of the
//! pass `paint` draws into, in one command buffer. That covers `prepare` (the
//! bloom's notes and chain, the body holdout), the shared spectral shadow's
//! `finish` (the Gaussian atlas's ink and blur) and `paint` (outline layer,
//! core layer, bloom composite). The closing pass READS the holdout, the atlas
//! and the bloom's quarter, so its end is ordered after every pass that wrote
//! them (#1113). It ends in one draw covering no pixel, because Metal leaves
//! the end stamp of a pass with no draw unwritten, and the control's paint
//! draws nothing.
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

const NOOP_SRC: &str = "
@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
@fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }
";

fn noop_pipeline(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("roll_timing_noop"),
        source: wgpu::ShaderSource::Wgsl(NOOP_SRC.into()),
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("roll_timing_noop"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[test]
#[ignore = "a probe: prints a timing and asserts nothing"]
fn a_frame_of_the_roll_costs_this_much() {
    crate::shader_assets::initialize();
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

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let set = device.create_query_set(&wgpu::QuerySetDescriptor {
        label: Some("roll_timing"),
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
    let resolve = buffer(
        "roll_timing_resolve",
        wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
    );
    let staging =
        buffer("roll_timing_staging", wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ);
    let target = |size: [u32; 2]| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("roll_timing_target"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    let stamp_view = target([1, 1]);
    let pane_view = target(size);
    let noop = noop_pipeline(&device, format);
    let stamped_pass = |encoder: &mut wgpu::CommandEncoder,
                        view: &wgpu::TextureView,
                        begin: Option<u32>,
                        end: Option<u32>| {
        encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("roll_timing_pass"),
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
                    beginning_of_pass_write_index: begin,
                    end_of_pass_write_index: end,
                }),
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime()
    };
    let period = f64::from(queue.get_timestamp_period());
    let points = [size[0] as f32 / ppp, size[1] as f32 / ppp];
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(points[0], points[1]));
    let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: ppp };
    let bloom = crate::bloom_strength(harmonigraph_scene::ViewConfig::default().note_bloom);

    // Median and p10 of `frames` frames of one roll, after ten to warm up.
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
        let mut samples = Vec::with_capacity(frames);
        for frame in 0..frames + 10 {
            let mut encoder = device.create_command_encoder(&Default::default());
            drop(stamped_pass(&mut encoder, &stamp_view, Some(0), None));
            let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
            crate::spectral_shadow::finish(
                &device,
                &queue,
                &screen,
                &mut encoder,
                &mut resources,
                0,
            );
            {
                let mut pass = stamped_pass(&mut encoder, &pane_view, None, Some(1));
                cb.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: ppp,
                        screen_size_px: size,
                    },
                    &mut pass,
                    &resources,
                );
                pass.set_pipeline(&noop);
                pass.draw(0..3, 0..1);
            }
            encoder.resolve_query_set(&set, 0..2, &resolve, 0);
            encoder.copy_buffer_to_buffer(&resolve, 0, &staging, 0, 16);
            queue.submit(bufs.into_iter().chain([encoder.finish()]));
            let slice = staging.slice(..);
            slice.map_async(wgpu::MapMode::Read, |r| r.expect("map"));
            device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
            let ticks: Vec<u64> =
                bytemuck::cast_slice::<u8, u64>(&slice.get_mapped_range()).to_vec();
            staging.unmap();
            assert!(ticks[0] > 0 && ticks[1] >= ticks[0], "unsupported timestamp pair: {ticks:?}");
            if frame >= 10 {
                samples.push((ticks[1] - ticks[0]) as f64 * period / 1.0e6);
            }
        }
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
