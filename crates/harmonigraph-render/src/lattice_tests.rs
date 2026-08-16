//! Unit tests for the lattice renderer. The GPU-backed ones no-op when
//! no headless adapter is available (CI without a GPU); the headless
//! device and the render/readback round trip they share with `roll` and
//! `text` live in [`crate::gpu_harness`].

use super::*;
use crate::gpu_harness::{headless_device, readback, render_to_texture};

#[test]
fn baked_shader_validates() {
    validate_wgsl(SHADER_SRC)
        .expect("baked lattice.wgsl must parse, validate, and keep its entry points");
}

/// The `const _: () = assert!(PITCH_LUT_N == 64)` in `lib.rs` ties one Rust
/// literal to another; the two that decide what the GPU actually reads are in
/// WGSL, where no compiler is checking them against the scene's constant. This
/// is the half that catches a one-sided bump.
///
/// Both slips are silent otherwise. Raise the array but not the const and the
/// shader walks 63 entries of a longer table, painting a top-of-range glyph the
/// color of a pitch halfway down the ramp while the disc under it takes the
/// top — the mismatch #165 closed, reopened at several times the width. Raise
/// the const but not the array and the surplus indices clamp at runtime, which
/// is not a validation error either. naga sees a well-formed shader both ways,
/// `min_binding_size: None` means an over-long buffer never complains, and the
/// scene tests read PITCH_LUT_N symbolically, so they pass at any value.
///
/// Each table is named in its own needle rather than looked for by shape.
/// There are TWO of that length now — `pitch_lut`, walked by pitch, and the
/// `spectral_lut` the audio ring walks by level — and an unnamed
/// `array<vec4<f32>, N>` is satisfied by whichever of them still matches, so
/// it would pass with one table bumped and the other left behind. That is the
/// worse half of the mismatch, not a lesser one: the two sit in one uniform
/// block, so a length that disagrees with the CPU's upload moves every field
/// after it — `spectrum` included — to an offset the shader does not read it
/// at, and the picture that comes back is wrong everywhere rather than in one
/// ramp.
#[test]
fn the_shaders_pitch_luts_are_the_length_the_scene_says() {
    let n = harmonigraph_scene::PITCH_LUT_N;
    let needles = [
        format!("pitch_lut: array<vec4<f32>, {n}>"),
        format!("spectral_lut: array<vec4<f32>, {n}>"),
        format!("const PITCH_LUT_N: u32 = {n}u;"),
    ];
    for needle in &needles {
        assert!(
            SHADER_SRC.contains(needle),
            "lattice.wgsl must declare `{needle}` to match harmonigraph_scene::PITCH_LUT_N \
             ({n}); the CPU uploads that many entries and the GPU would index a different table",
        );
    }
    // And no third table of that shape has appeared without a needle of its
    // own, which is how the two got down to one check in the first place.
    assert_eq!(
        SHADER_SRC.matches(&format!("array<vec4<f32>, {n}>")).count(),
        2,
        "lattice.wgsl declares a table of {n} vec4s that this test does not name; give it a \
         needle, or a one-sided bump to it passes here",
    );
}

/// The field names `struct {name} { ... }` declares in `src`, in order — a
/// `//` or `///` comment line is skipped, and each remaining non-blank line
/// contributes the identifier before its first `:`. Neither language's
/// struct is parsed for real; this reads both the same shallow way the
/// [`the_shaders_pitch_lut_is_the_length_the_scene_says`] needle check
/// does, which is enough to catch the two lists disagreeing.
///
/// Assumes one field per line, which every field in both of today's structs
/// is short enough to be. A field whose type needs wrapping to a second line
/// panics here instead of parsing wrong — loud, but a confusing place to
/// land for whoever adds it, since the message names no field and no line.
fn struct_field_names(src: &str, name: &str) -> Vec<String> {
    let after_kw = src.split_once(&format!("struct {name}")).expect("struct not found").1;
    let body_start = after_kw.find('{').expect("struct has no body") + 1;
    let mut depth = 1u32;
    let mut end = body_start;
    for (i, c) in after_kw[body_start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = body_start + i;
                    break;
                }
            }
            _ => {}
        }
    }
    after_kw[body_start..end]
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .map(|l| l.split_once(':').expect("field line has no `:`").0.trim().to_string())
        .collect()
}

/// `misc`..`misc8` carry the picture's knobs packed several to a vec4 (see
/// the doc comments on [`Uniforms`] and its WGSL twin), and nothing checks
/// the two structs against each other: naga validates the WGSL side against
/// itself, rustc the Rust side against itself, and a slot added, dropped,
/// renamed, or reordered on only one side still compiles and validates —
/// only the byte offsets downstream of it drift, so every read after the
/// mismatch lands on the wrong vec4's `.x`/`.y`/`.z`/`.w`. Comparing the
/// field-name lists is the cheap half of the guard; the doc comments above
/// each field are the other half; a `.w` typo'd for a `.z` within an
/// otherwise-correctly-paired slot is neither this test's job nor the
/// PITCH_LUT_N one's — see their doc comments.
#[test]
fn the_uniforms_slots_pair_up_between_rust_and_wgsl() {
    let rust_fields = struct_field_names(include_str!("lib.rs"), "Uniforms");
    let wgsl_fields = struct_field_names(SHADER_SRC, "Uniforms");
    assert_eq!(
        rust_fields, wgsl_fields,
        "lib.rs's Uniforms and lattice.wgsl's Uniforms must declare the same fields in the \
         same order — they describe one GPU buffer from two ends, and every field here is a \
         multiple of 16 bytes, which is what lets Rust's #[repr(C)] layout match WGSL's without \
         either side spelling out padding; a name added, dropped, renamed, or reordered on only \
         one side is exactly what desyncs the offsets.",
    );
}

/// blit.wgsl has no hot-reload path, so a broken edit would otherwise
/// first surface as a pipeline panic inside a DAW.
#[test]
fn baked_blit_shader_validates() {
    let module = naga::front::wgsl::parse_str(BLIT_SRC)
        .map_err(|e| e.emit_to_string(BLIT_SRC))
        .expect("blit.wgsl must parse");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("blit.wgsl must validate");
    for required in [
        "vs_blit",
        "fs_blit",
        "fs_bright",
        "fs_blur_h",
        "fs_blur_v",
        "fs_composite",
        "fs_bloom_add",
    ] {
        assert!(
            module.entry_points.iter().any(|ep| ep.name == required),
            "missing entry point `{required}`"
        );
    }
}

#[test]
fn octave_packing_matches_the_documented_layout() {
    let mut levels = [0.0f32; harmonigraph_scene::OCTAVE_SLOTS];
    levels[0] = 1.0; // lowest byte of word 0
    levels[3] = 0.5; // highest byte of word 0
    levels[4] = 1.0; // lowest byte of word 1
    levels[9] = 1.0; // second byte of word 2
    let words = pack_octaves(&levels);
    assert_eq!(words[0] & 0xFF, 255);
    assert_eq!((words[0] >> 24) & 0xFF, 128);
    assert_eq!(words[1] & 0xFF, 255);
    assert_eq!(words[2] & 0xFF, 0);
    assert_eq!((words[2] >> 8) & 0xFF, 255);
    // Out-of-range levels clamp instead of corrupting neighbors.
    let words = pack_octaves(&[2.0; harmonigraph_scene::OCTAVE_SLOTS]);
    assert_eq!(words[0], 0xFFFF_FFFF);
}

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
    let _resources =
        LatticeResources::new(&device, &queue, wgpu::TextureFormat::Bgra8Unorm);
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
    let preview = LatticeCallback::from_scene(
        &scene,
        LatticeLabels::default(),
        size,
        format,
        1,
        None,
    );

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

/// The band width every fixture in this file sweeps at: a WIDE one, chosen
/// so a band is many pixels across at these render sizes and a probe can
/// walk it. It is deliberately not the fresh view's width, which is 0.64 —
/// fine enough that several bands cross a single node, which is the look the
/// bar's tight end exists for and the wrong regime to measure a sheet's
/// geometry in.
///
/// The gap is a known hole in this suite rather than a property of it: the
/// shipped picture is only ever rendered here at the tight end's control
/// case. Anything that goes wrong at 0.64 alone — the resolve fade closing
/// on the pixel footprint, the crest/trough balance at softness 1 — ships
/// green.
///
/// Named because [`SHIMMER_PROBE_STEP`] is sized off it: the width is a view
/// setting rather than the shader constant it was, so a fixture retuned in
/// one place and not the other is how the probe would come to measure
/// nothing.
const PARITY_SHIMMER_WIDTH: f32 = 5.0;

/// A scene exercising every draw path: lit + idle + hovered nodes with
/// octave indicators, a chord beam, and solid + dashed grid lines, all
/// overlapping so blend order matters.
fn parity_scene() -> Scene {
    use glam::{Vec3, Vec4};
    use harmonigraph_core::LatticePos;

    let mut nodes = Vec::new();
    for i in 0..6u32 {
        let f = i as f32;
        // Slots walked inside the DEFAULT Range: a level on a slot the view
        // doesn't show is drawn by no sector, so a scene claiming to exercise
        // every draw path has to sound in the range it renders. Asked of the
        // same layout the scene carries below rather than re-derived from the
        // slot count, which would part company with it the first time the
        // default window moved. The slots a node draws depend on its pitch
        // class, so this is the set EVERY node here has: the highest node's
        // low end, the lowest node's high end.
        let layout = harmonigraph_scene::OctaveLayout::default();
        let (low, _) = layout.slots(0.0);
        let (_, high) = layout.slots(950.0);
        let slot = |k: usize| (low.max(0) as usize) + k % (high - low + 1) as usize;
        let mut octaves = [0.0f32; harmonigraph_scene::OCTAVE_SLOTS];
        octaves[slot(i as usize)] = 1.0 - f * 0.1;
        octaves[slot(i as usize + 5)] = 0.4;
        nodes.push(harmonigraph_scene::NodeInstance {
            lattice_pos: LatticePos::new(i as i32 - 3, i as i32 % 2, 0),
            // Cluster tightly around the origin so discs overlap and
            // draw order shows in the output.
            world_pos: Vec3::new(f * 0.45 - 1.1, (f % 3.0) * 0.4 - 0.4, f * 0.3 - 0.75),
            color: Vec4::new(0.25 + f * 0.12, 0.55 - f * 0.05, 0.95 - f * 0.1, 1.0),
            activation: if i % 3 == 0 { 1.0 } else { 0.3 + f * 0.1 },
            // Nothing the shader draws reads this — it is the label layer's,
            // and labels are the UI crate's text pass, not the lattice pass.
            departing: false,
            octaves,
            hovered: i == 1,
            on_home: i % 2 == 0,
            // The off-sheet half draws small and knocks out, so the
            // every-draw-path scene exercises both sevens-layer branches
            // (the scaled billboard and the gutter's extra alpha) as well.
            scale: if i % 2 == 0 { 1.0 } else { 0.55 },
            gutter: if i % 2 == 0 { 0.0 } else { 0.12 },
            comma: if i % 2 == 0 { 0.0 } else { -27.26 },
            cents: f * 190.0,
            // Exercise the mark paths: one node marked melody, one bass, and
            // one wearing both — on its two lit slots, so the pair is drawn as
            // two extensions rather than as one slice claimed twice.
            melody_slots: if i == 0 || i == 4 { 1 << slot(i as usize) } else { 0 },
            bass_slots: match i {
                2 => 1 << slot(i as usize),
                4 => 1 << slot(i as usize + 5),
                _ => 0,
            },
            melody_level: if i == 0 || i == 4 { 1.0 } else { 0.0 },
            bass_level: if i == 2 || i == 4 { 1.0 } else { 0.0 },
            melody_color: Vec4::new(1.0, 0.85, 0.4, 1.0),
            bass_color: Vec4::new(0.45, 0.8, 1.0, 1.0),
            trail: 0.0,
        });
    }
    let grid = vec![
        harmonigraph_scene::EdgeInstance {
            a: Vec3::new(-1.8, -0.6, -0.3),
            b: Vec3::new(1.6, -0.6, -0.3),
            color: Vec4::new(0.16, 0.17, 0.20, 0.55),
            strength: 0.55,
            dashed: false,
        },
        harmonigraph_scene::EdgeInstance {
            a: Vec3::new(-1.2, 0.7, -0.6),
            b: Vec3::new(1.2, 0.4, 0.6),
            color: Vec4::new(0.16, 0.17, 0.20, 0.55),
            strength: 0.4,
            dashed: true,
        },
    ];
    Scene {
        nodes,
        camera: harmonigraph_scene::Camera::default(),
        now: 1.25,
        // The ground the sevens knockout clears to; the half of this
        // scene's nodes that carry a gutter exercise it.
        background: harmonigraph_scene::skin::panel_color(),
        sevens_soft: 0.24,
        node_radius: 0.34,
        mark_thickness: 0.09,
        // Off: a single-instant parity image can't depend on which moment
        // of a cycle it lands on.
        pulse_marks: Default::default(),
        // The sweep's own settings, at this suite's measurable values rather
        // than the fresh view's (see `PARITY_SHIMMER_WIDTH`). Inert while the
        // mode above is Off,
        // and stated rather than defaulted because a test that turns a mode
        // ON — every shimmer test builds on this fixture — has to be sweeping
        // something a reader can size against SHIMMER_PROBE_STEP.
        shimmer_speed: 1.6,
        shimmer_width: PARITY_SHIMMER_WIDTH,
        shimmer_intensity: 1.0,
        shimmer_softness: 0.8,
        core_radius: 0.46,
        core_solidity: 1.0,
        outer_inner: 0.545,
        outer_outer: 0.795,
        // The band is the outermost ring here, as it is on a fresh node, so it
        // is what the marks stand off — a gap out from its edge.
        rings_outer: 0.795,
        mark_inner: 0.795 + 0.12,
        ring_gap: 0.12,
        // No analyzer: the ring off, under either reading. It is a whole layer
        // more light in the middle of every node, and the sweep and mark
        // measurements here are sized against the picture without it —
        // `the_audio_ring_reads_the_spectrum_around_each_octave`,
        // `the_folded_ring_reads_each_wedge_at_its_own_octave` and the
        // early-out parity's own `ringing` fixture are where it is turned on.
        spectral: harmonigraph_scene::SpectralPaint::silent(),
        // The plain circular division: this scene is about how the draw
        // paths composite, so the indicators are the ones every other
        // setting is a departure from.
        octave_layout: harmonigraph_scene::OctaveLayout::default(),
        grid,
        grid_thickness: 1.0,
        // A blue->red sweep across the whole table, so a glyph's color is a
        // reading of which entry it landed on. Spanned off PITCH_LUT_N rather
        // than a literal: `from_fn` sizes itself from the field, so a literal
        // divisor would silently stop covering the table when the constant
        // grows, leaving every entry past it out of gamut and clamping to one
        // color — the fixture would still render, and would still be green.
        pitch_lut: std::array::from_fn(|k| {
            let t = k as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
            Vec4::new(t, 0.4, 1.0 - t, 1.0)
        }),
        darkest_pitch: 24.0,
        brightest_pitch: 108.0,
        render_scale: 1.0,
        // Parity with the direct-to-egui-pass reference requires bloom off.
        bloom_strength: 0.0,
    }
}

/// What the lattice pixel tests share on top of [`crate::gpu_harness`]: the
/// callback resources one lattice pass keeps between frames, and the
/// offscreen target a scene is drawn into.
///
/// It exists because the alternative had spread. Turning a `Scene` into a
/// buffer of pixels is twenty-seven lines that never vary — build the
/// callback, encode, submit, render to a texture, read it back — and every
/// test that wanted pixels carried its own copy of them, fourteen in this
/// file by the time they were counted, byte-identical but for where the pane
/// id came from. Nothing was wrong with any copy; what is wrong is the
/// arithmetic on the next change to that sequence, which then has to land in
/// fourteen places and silently keeps testing something else in the one it
/// misses. #112's mark-cache fix — a resource-lifetime correction between
/// `prepare` and `paint` — is exactly that shape of edit.
///
/// Each test owns its own `Shooter`, and so its own `CallbackResources`:
/// nothing here is shared between tests, which is what lets the pane counter
/// below start from the same place in all of them.
struct Shooter {
    device: wgpu::Device,
    queue: wgpu::Queue,
    resources: CallbackResources,
    format: wgpu::TextureFormat,
    size: [u32; 2],
    /// Bumped per shot, because a pane id keys the per-pane buffers inside
    /// `resources`, and a test comparing two pictures wants two of them
    /// rather than one reused. The tests that used to name these by hand all
    /// counted up the same way, and none ever asked for a repeat.
    pane: u64,
}

impl Shooter {
    /// `None` where the machine has no usable GPU — CI containers, mostly.
    /// Every caller returns on it.
    fn new(size: [u32; 2]) -> Option<Shooter> {
        let (device, queue) = headless_device()?;
        Some(Shooter {
            device,
            queue,
            resources: CallbackResources::default(),
            format: wgpu::TextureFormat::Rgba8Unorm,
            size,
            pane: 1,
        })
    }

    /// `scene` drawn to a fresh texture over black, read back as RGBA8.
    fn shot(&mut self, scene: &Scene) -> Vec<u8> {
        self.shot_with(scene, LatticeLabels::default())
    }

    /// [`shot`](Self::shot), with labels — the layer that carries its own
    /// atlas, and its own reasons to be tested.
    fn shot_with(&mut self, scene: &Scene, labels: LatticeLabels) -> Vec<u8> {
        self.pane += 1;
        let size = self.size;
        let vec_size = egui::vec2(size[0] as f32, size[1] as f32);
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);
        let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: 1.0 };
        let cb =
            LatticeCallback::from_scene(scene, labels, vec_size, self.format, self.pane, None);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let bufs =
            cb.prepare(&self.device, &self.queue, &screen, &mut encoder, &mut self.resources);
        self.queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let resources = &self.resources;
        let tex = render_to_texture(
            &self.device,
            &self.queue,
            size,
            self.format,
            wgpu::Color::BLACK,
            |pass| {
                cb.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: 1.0,
                        screen_size_px: size,
                    },
                    pass,
                    resources,
                );
            },
        );
        readback(&self.device, &self.queue, &tex, size)
    }
}

/// How many pixels of two shots of one size differ at all.
fn differing_pixels(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4).zip(b.chunks(4)).filter(|(x, y)| x != y).count()
}

/// One pixel's brightness, as the plain sum of its channels — a reading
/// rather than a colorimetric luminance, and every caller compares two of
/// them rather than asking what the number means on its own.
fn brightness(px: &[u8]) -> i64 {
    px[0] as i64 + px[1] as i64 + px[2] as i64
}

/// All the light in one shot (see [`brightness`]).
fn total_light(px: &[u8]) -> i64 {
    px.chunks(4).map(brightness).sum()
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
    let screen = ScreenDescriptor {
        size_in_pixels: SIZE,
        pixels_per_point: 1.0,
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    let user_bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
    queue.submit(user_bufs.into_iter().chain([encoder.finish()]));

    let clear = wgpu::Color {
        r: 0.07,
        g: 0.08,
        b: 0.09,
        a: 1.0,
    };
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
    let (node_pipeline, edge_pipeline) =
        create_pipelines(&device, SHADER_SRC, format, &res.bind_group_layout, false);
    let pane = res.panes.get(&7).expect("prepare created the pane");
    let direct_tex = render_to_texture(&device, &queue, SIZE, format, clear, |pass| {
        // The grid sits at the home sheet's depth, so it is drawn INSIDE the
        // node run, at `grid_at` — mirror that here or the two paths differ
        // by draw order rather than by the thing under test.
        let nodes = |pass: &mut wgpu::RenderPass<'static>, range: std::ops::Range<u32>| {
            if !range.is_empty() {
                pass.set_pipeline(&node_pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                pass.draw(0..4, range);
            }
        };
        nodes(pass, 0..pane.grid_at);
        if pane.edge_count > 0 {
            pass.set_pipeline(&edge_pipeline);
            pass.set_bind_group(0, &pane.bind_group, &[]);
            pass.set_vertex_buffer(0, pane.edge_buffer.slice(..));
            pass.draw(0..4, 0..pane.edge_count);
        }
        nodes(pass, pane.grid_at..pane.instance_count);
    });

    let composite = readback(&device, &queue, &composite_tex, SIZE);
    let direct = readback(&device, &queue, &direct_tex, SIZE);

    // Guard against vacuous success: the scene must actually have drawn
    // over the clear color somewhere.
    let bg = [18u8, 20, 23, 255]; // clear color as 8-bit RGBA
    assert!(
        direct
            .chunks(4)
            .any(|px| px.iter().zip(bg).any(|(&c, b)| c.abs_diff(b) > 8)),
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

/// Every pattern in the row draws its OWN picture — pairwise, at one instant.
///
/// `pulse_marks` is Off in `parity_scene` and every fixture derived from it
/// (deliberately — see that scene's own comment), so nothing else in this file
/// takes a `mode != 0u` branch in the shader: each arm of `shimmer_pattern` is
/// validated by `baked_shader_validates` (parsed, never run) but not otherwise
/// exercised by any render. This runs all of them.
///
/// Pairwise rather than each-against-Off, because "it changed something" is
/// the weaker half of the claim and the one an accident passes: two patterns
/// that fell through to the same arm of the shader, or a mode index off by one
/// anywhere along `Pulse::shader_index` -> misc6.w -> `shimmer_pattern`, would
/// each differ from Off perfectly well while being the same picture as each
/// other. The row is only a row if its options are distinguishable.
///
/// It is a single INSTANT for the same reason: two patterns compared across
/// their own frames would differ merely by having moved.
#[test]
fn every_shimmer_pattern_draws_a_different_picture() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    use harmonigraph_scene::Pulse;

    // Off first, so the loop below has the steady picture to measure against
    // as well as the other patterns.
    let modes = [Pulse::Off, Pulse::Bands, Pulse::Checker, Pulse::Hex];
    // One node with an end marked: the sheet needs a ring to belong to.
    let shots: Vec<(Pulse, Vec<u8>)> = modes
        .iter()
        .map(|&mode| {
            let mut scene = single_marked_node(MIDDLE_C, 0);
            scene.pulse_marks = mode;
            scene.now = 0.4;
            (mode, gpu.shot(&scene))
        })
        .collect();

    for (i, (mode, px)) in shots.iter().enumerate() {
        for (other, other_px) in &shots[i + 1..] {
            assert!(
                differing_pixels(px, other_px) > 0,
                "{mode:?} and {other:?} drew the same picture at the same instant; \
                 they are one option wearing two labels",
            );
        }
    }
}

/// One period of travel returns two of the three patterns to the picture they
/// drew, and Hex to its opposite.
///
/// This is the shape the shader's periodicity actually has, and
/// `Scene::shimmer_slide` reduces a song position against it — so what the
/// modulus there has to be is measured here rather than reasoned about at the
/// other end of the pipe. Hex crosses three gratings sixty degrees apart and
/// the outer two take the travel through a `cos 60`, which halves their rate
/// along their own axes: it closes a cycle over TWO periods, and reducing a
/// clock by one would land it on this test's second assertion, silently, at
/// every wrap.
///
/// Rendered rather than argued because the alternative — asserting that a
/// reduced clock draws what an unreduced one would — cannot be written: the
/// reduction is what produces the number the shader sees, so both sides of
/// that comparison are the same uniform. Turning it around is what makes it
/// observable, and this is the turned-around form.
#[test]
fn one_period_of_travel_repeats_every_pattern_but_hex() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    use harmonigraph_scene::Pulse;
    // The pair `the_mark_sheet_reaches_the_slice_whole` sweeps at: a period
    // well inside what this size resolves, at a pace that crosses it.
    const WIDTH: f32 = 1.2;
    const SPEED: f32 = 1.6;
    let at = |mode: Pulse, now: f64| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, 0);
        scene.pulse_marks = mode;
        scene.shimmer_width = WIDTH;
        scene.shimmer_speed = SPEED;
        scene.now = now;
        scene
    };
    // Seconds of clock that carry the sheet one period along.
    let period = (WIDTH / SPEED) as f64;
    // Off zero, so a pattern that ignored the clock entirely would not pass
    // the first half by drawing its rest state twice.
    const BASE: f64 = 0.3;

    for mode in [Pulse::Bands, Pulse::Checker] {
        let (before, after) = (gpu.shot(&at(mode, BASE)), gpu.shot(&at(mode, BASE + period)));
        // Half a percent rather than byte-equality, though it measures zero
        // here: the two shots reach the same phase by different arithmetic —
        // one is the other's plus a period, through a sine whose argument is
        // scaled by a reciprocal — so a driver rounding one of them the other
        // way is a byte, not a defect. What this guards against redraws the
        // sheet, not a byte of it.
        let moved = differing_pixels(&before, &after);
        assert!(
            moved * 200 < before.len(),
            "{mode:?} redrew {moved} pixels a period of travel later; it takes the sheet's \
             own period whole, so a period of travel is where it repeats",
        );
    }

    let (before, after) =
        (gpu.shot(&at(Pulse::Hex, BASE)), gpu.shot(&at(Pulse::Hex, BASE + period)));
    assert!(
        differing_pixels(&before, &after) > 0,
        "Hex drew the same picture a period of travel later, so its cycle is one period \
         and not two — and `Scene::shimmer_slide` is reducing the clock by twice what it \
         has to, or this pattern's gratings have moved off sixty degrees",
    );
}

/// The Softness bar reaches the picture, and it is the SHAPE it moves rather
/// than the amount of light.
///
/// Held still (speed 0) and at one instant, so what is compared is two
/// profiles of the same sheet in the same place rather than two moments of
/// one. Three claims, and it takes all three to say "shape":
///
/// - the two ends draw differently, which is the bar working at all;
/// - the gradual end lights MORE over the layer as a whole, the fall from the
///   peak taking most of the period instead of a narrow crest;
/// - and it does that without going any BRIGHTER at its brightest. That last
///   is what rules out the wiring this could otherwise have — a bar on
///   `SHIMMER_EXPOSURE`, raising the peak rather than widening the fall from
///   it, passes the first two and fails this one. The peak is Intensity's to
///   move, and the shape's own crest is pinned wherever it lands: `pow(1, n)`
///   is 1 for every exponent, so however the profile is dialled the brightest
///   pixel is the same pixel at the same value.
#[test]
fn shimmer_softness_spreads_the_light_without_raising_the_peak() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    // Both ends marked, so the sheet has two rings and the slice they name to
    // fall across: the light this measures is all of what a sweep puts on the
    // picture, and one ring's worth of it would be a thinner reading of the
    // same claim.
    let at = |softness: f32| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, MIDDLE_C);
        scene.pulse_marks = harmonigraph_scene::Pulse::Bands;
        scene.shimmer_softness = softness;
        scene.shimmer_speed = 0.0;
        scene.now = 0.4;
        scene
    };
    let light = |px: &[u8]| -> u64 {
        px.chunks(4).map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64).sum()
    };

    let crisp = gpu.shot(&at(0.0));
    let gradual = gpu.shot(&at(1.0));
    assert!(
        differing_pixels(&crisp, &gradual) > 0,
        "the Softness bar did not reach the picture at all",
    );
    let (crisp_light, gradual_light) = (light(&crisp), light(&gradual));
    assert!(
        gradual_light > crisp_light,
        "the gradual end lit {gradual_light} against the crisp end's {crisp_light}: \
         softening the profile is supposed to spread the light over more of the \
         period, not to dim it",
    );

    // The brightest pixel THE SHEET REACHES, which is the mask below rather
    // than the whole frame: the brightest pixel in the frame is the core disc,
    // which no sheet touches, and a peak read there would come out the same
    // number at both ends however the bar were wired -- the one claim this
    // test exists for, passing vacuously.
    let steady = {
        let mut scene = at(0.0);
        scene.pulse_marks = harmonigraph_scene::Pulse::Off;
        gpu.shot(&scene)
    };
    let swept: Vec<usize> = (0..steady.len() / 4)
        .filter(|&i| {
            let px = i * 4..i * 4 + 4;
            steady[px.clone()] != crisp[px.clone()] || steady[px.clone()] != gradual[px]
        })
        .collect();
    assert!(!swept.is_empty(), "neither end of the bar swept a single pixel");
    let peak = |img: &[u8]| -> u32 {
        swept
            .iter()
            .map(|&i| img[i * 4] as u32 + img[i * 4 + 1] as u32 + img[i * 4 + 2] as u32)
            .max()
            .unwrap_or(0)
    };

    // A hair of tolerance, and only for the rounding two paths through the
    // same arithmetic can land either side of: the claim is that the crest
    // does not MOVE, which a peak-wired bar would break by whole channel
    // steps. Measured dead equal at both ends.
    let (crisp_peak, gradual_peak) = (peak(&crisp), peak(&gradual));
    eprintln!("brightest pixel: {crisp_peak} crisp, {gradual_peak} gradual");
    assert!(
        gradual_peak <= crisp_peak + 2,
        "the gradual end's brightest pixel is {gradual_peak} against the crisp \
         end's {crisp_peak}: Softness is raising the peak rather than widening the \
         fall from it, which is Intensity's job and not this bar's",
    );
}

/// The slot mask naming middle C's octave — the one the node below sounds
/// in, and so the one a mark can link back to.
const MIDDLE_C: u32 = 1 << harmonigraph_scene::MIDDLE_C_SLOT;

/// `L*` of one pixel, off the curve `harmonigraph_scene::color` authors the
/// ramp on rather than a copy of it.
///
/// Real colorimetry where the rest of this file reads a channel sum, because
/// the tests below are claims about how bright a thing LOOKS, compared across
/// colors that differ in hue as well: a sum weights a blue channel like a green
/// one and would call a violet ring and a yellow one the same brightness.
/// Shared with the authoring code and not restated here, so a reading is in the
/// units the ramp is dialled in — a second copy of the constants could drift
/// and would then agree with itself and disagree with the picture.
fn lightness(px: &[u8]) -> f64 {
    let v = |b: u8| f64::from(b) / 255.0;
    harmonigraph_scene::color::lightness_of_encoded(v(px[0]), v(px[1]), v(px[2]))
}

/// Where a pixel sits on the hue circle in degrees, or `None` for one too near
/// grey to have a hue at all.
///
/// The hexcone hue rather than a perceptual one, for the same reason the chroma
/// reading below is a channel spread: what it has to do is move when the color
/// changes hue and hold still when the color only gets lighter or paler, and
/// both shots are read the same way. The grey guard is what keeps it honest —
/// hue is undefined on the achromatic axis, so a pixel washed out to white
/// would otherwise report an arbitrary angle and be counted as a rotation.
fn hue_degrees(px: &[u8]) -> Option<f64> {
    let (r, g, b) = (f64::from(px[0]), f64::from(px[1]), f64::from(px[2]));
    let (high, low) = (r.max(g).max(b), r.min(g).min(b));
    let c = high - low;
    if c < 8.0 {
        return None;
    }
    let h = if high == r {
        (g - b) / c
    } else if high == g {
        (b - r) / c + 2.0
    } else {
        (r - g) / c + 4.0
    };
    Some((h * 60.0).rem_euclid(360.0))
}

/// A color's steady shot and the eight swept ones taken over it.
type Shots = (Vec<u8>, Vec<Vec<u8>>);

/// One node wearing both rings in `color`, shot steady and then at eight
/// moments of one period of the sweep — the same geometry every time, so two
/// colors' readings line up pixel for pixel and a difference between them is a
/// difference the COLOR made.
///
/// Eight moments rather than one because the sheet is a plane crossing the
/// lattice: which part of the ring a crest is over depends on where the node
/// sits under it, and no single instant has every pixel at its own peak. Both
/// halves of the period come off the scene the fixture actually builds, so a
/// caller that retunes either bar still gets one whole cycle rather than eight
/// arbitrary phases of a longer one.
///
/// Eight samples leave the sampled peak a little under the true one: the worst
/// phase offset is an eighth of a turn, which puts `wave` at 0.962, and the
/// band the shader draws is `pow(wave, sharpness)` — 0.943 at this fixture's
/// Softness. Both colors are sampled at the same phases, so that 5.7% cancels
/// between them and none of it reaches a comparison.
fn sweep_over_color(gpu: &mut Shooter, color: glam::Vec4) -> Shots {
    let at = |pulse, time: f64| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, MIDDLE_C);
        scene.nodes[0].melody_color = color;
        scene.nodes[0].bass_color = color;
        scene.pulse_marks = pulse;
        scene.now = time;
        scene
    };
    let steady_scene = at(harmonigraph_scene::Pulse::Off, 0.0);
    let period = steady_scene.shimmer_width / steady_scene.shimmer_speed;
    let steady = gpu.shot(&steady_scene);
    let swept = (0..8)
        .map(|k| gpu.shot(&at(harmonigraph_scene::Pulse::Bands, period as f64 * k as f64 / 8.0)))
        .collect();
    (steady, swept)
}

/// Whether one shot's sweep moves this pixel: further from its steady self than
/// a byte's rounding at some moment of the period.
///
/// MOVED and not brightened, which is the whole of what an exposure changes
/// here. The sheet is a ratio fitted to the layer's own headroom
/// (`shimmer_light`), so where a color has room the sweep reads as light added
/// and where it has none it reads as shade between crests — and a color at the
/// top of a channel takes ALL of it as shade. Asking for a brighter pixel would
/// find the sheet at one end of the ramp and miss it at the other, which is the
/// difference these tests exist to measure rather than to filter out.
fn swept(shot: &Shots, i: usize) -> bool {
    let base = lightness(&shot.0[i * 4..i * 4 + 4]);
    shot.1.iter().any(|f| (lightness(&f[i * 4..i * 4 + 4]) - base).abs() > 1.0)
}

/// The pixels one color's sweep moves, for a reading taken over a single shot.
fn swept_pixels(shot: &Shots) -> Vec<usize> {
    (0..shot.0.len() / 4).filter(|&i| swept(shot, i)).collect()
}

/// The pixels a sweep moves in BOTH shots AND that the two colors draw
/// differently when steady.
///
/// Two filters and not one. The intersection is so the two readings are over
/// one set of pixels and neither is averaged over ground the other never
/// covered. The steady difference is what confines the set to the RINGS, which
/// are the only thing the color argument reaches: the octave slice shimmers
/// too, but it takes its color from `scene.pitch_lut` — the fixture's own
/// synthetic ramp, which neither shot varies — so every slice pixel is
/// byte-identical in both shots and lifts by exactly the same amount in each.
/// Left in, they are a block of guaranteed agreement pulling the two colors'
/// readings together, and there are enough of them to carry the assertions
/// below on their own: the comparison would still pass with the rings' shimmer
/// deleted outright, which is the one thing it exists to catch.
fn lifted_pixels(a: &Shots, b: &Shots) -> Vec<usize> {
    (0..a.0.len() / 4)
        .filter(|&i| a.0[i * 4..i * 4 + 3] != b.0[i * 4..i * 4 + 3])
        .filter(|&i| swept(a, i) && swept(b, i))
        .collect()
}

/// One sweep is worth the same CONTRAST on the pitch ramp's dark end as on its
/// bright one — the ratio between a crest and its trough, which is the currency
/// a texture this fine is seen in.
///
/// The currency is the claim. An added light is near-uniform in the `L*` it
/// ADDS — 21.6 to 22.4 across the ramp here, a 13% spread — which is the
/// property such a sheet is tuned to hold; but the crest-to-trough RATIO
/// under it falls from 0.514 at
/// the ramp's dark end to 0.369 at its bright one, a 28% decline, and with the
/// fresh view's bloom on it is a 35% one. A moving texture is read by that
/// ratio rather than by the difference, which is why the sheet reads weaker on
/// the ramp's bright half however uniform the light it adds. An exposure makes
/// the ratio the constant instead and lets the difference vary — the trade
/// taken deliberately, and the reason `SHIMMER_EXPOSURE` is a gain rather than
/// an amount.
///
/// The bound is a tenth where an added light's could bear no better than a
/// quarter, because this is the property the model HOLDS rather than one it
/// approximates: a multiply
/// is one ratio by construction, and what is left to vary is the layers under
/// the rings that the sheet does not touch. Measured at 3% over the ramp with
/// bloom off and 7% with it on.
///
/// The two colors are the ramp's own ends, injected as the node's ring colors.
/// The table the SHADER samples is the fixture's synthetic ramp and is not what
/// the rings wear — which is exactly why `lifted_pixels` has to drop the pixels
/// that draw the same in both shots. Bloom is off (`parity_scene`'s own
/// setting): a halo is a wide blur added over a fine texture, which raises a
/// pixel's mean without raising its swing, and it lands unevenly along the ramp
/// because the threshold's knee is steepest where the dark end sits. That is a
/// real cost of the post pass and worth its own reading; it is not what the
/// sheet does, which is what this asks.
#[test]
fn the_sweep_is_worth_the_same_contrast_on_a_dark_color_as_on_a_bright_one() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let lut = harmonigraph_scene::pitch_ramp_lut(
        harmonigraph_scene::ViewConfig::default().pitch_gradient,
    );
    let (dark, bright) = (lut[0], lut[harmonigraph_scene::PITCH_LUT_N - 1]);

    let dim = sweep_over_color(&mut gpu, dark);
    let lit = sweep_over_color(&mut gpu, bright);
    let shared = lifted_pixels(&dim, &lit);
    assert!(
        shared.len() > 200,
        "only {} pixels shimmered in both shots — the fixture stopped sweeping the \
         rings and the reading below would be noise",
        shared.len(),
    );

    // Michelson contrast over one cycle, in the light a pixel actually carries:
    // `L*` back through its own curve, so the ratio is a ratio of luminance
    // rather than of a perceptual coordinate that is already a cube root of it.
    let luminance = |l_star: f64| ((l_star + 16.0) / 116.0).powi(3);
    let contrast = |(steady, swept): &(Vec<u8>, Vec<Vec<u8>>)| -> f64 {
        let sum: f64 = shared
            .iter()
            .map(|&i| {
                let base = lightness(&steady[i * 4..i * 4 + 4]);
                let ls = swept.iter().map(|f| lightness(&f[i * 4..i * 4 + 4]));
                let (mut hi, mut lo) = (base, base);
                for l in ls {
                    hi = hi.max(l);
                    lo = lo.min(l);
                }
                let (hi, lo) = (luminance(hi), luminance(lo));
                (hi - lo) / (hi + lo).max(1e-9)
            })
            .sum();
        sum / shared.len() as f64
    };
    let (dim_c, lit_c) = (contrast(&dim), contrast(&lit));
    eprintln!(
        "one cycle is worth contrast {dim_c:.3} on the ramp's dark end, {lit_c:.3} on its \
         bright end"
    );
    // A tenth, against a reading of 3% over the whole ramp. Wider than the
    // measurement by enough for a rasteriser to disagree about a ring edge, and
    // nowhere near wide enough to admit the additive model: an added light
    // reads 28% apart on these same two colors, so the bound separates them
    // several times over, which is what a bound here is for.
    let spread = (dim_c - lit_c).abs() / dim_c.max(lit_c);
    assert!(
        spread < 0.10,
        "one cycle was worth contrast {dim_c:.3} on the ramp's dark end but {lit_c:.3} on \
         its bright end ({:.0}% apart): the sheet is a different size depending on which \
         note it is passing over, which is what one exposure everywhere exists to hold down",
        spread * 100.0,
    );
}

/// Between peaks the layer sits at its own color wherever the ceiling covers
/// the swing: a sweep's trough IS the steady picture rather than a dimmed copy
/// of it, on every color whose luma the swing still fits under
/// `SHIMMER_CEILING` — and where it stops fitting, the standing shade the
/// slide buys is bounded, and grows with how bright the color is.
///
/// This is the half of the model nothing else reads. The contrast test above
/// is indifferent to it — a slid swing is the same crest-to-trough ratio, so
/// that reading passes whether the troughs hold still or the whole ramp rides
/// a standing dimmer — and the chroma and hue test below reads only the crest.
/// A ceiling of 0.5 puts 9 `L*` of standing shade under even the ramp's dark
/// end with every other test in this file green; this is the one that goes
/// red.
///
/// The budgets are the measured shape of that trade, with room for a
/// rasteriser to disagree over ring edges and none for a regression. The slide
/// engages where a color's luma — the shader's dot over the STORED values,
/// not over their decoded light — clears `SHIMMER_CEILING / e^swing`, about
/// 0.40 at this fixture's Intensity of 1. The default ramp crosses that in
/// its upper half: the dark end (luma 0.33) pays nothing and is held to
/// rounding, mid-ramp (0.45) measures 3.7 `L*`, and the bright end (0.64)
/// measures 15 — the encoded-domain slide compounded through the display
/// transfer, and several times what a calibration in decoded light predicts.
/// `SHIMMER_CEILING`'s comment carries the trade; this pins its measured cost
/// so a retune moves a number here rather than a picture only.
#[test]
fn between_peaks_the_layer_sits_at_its_own_color() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let lut = harmonigraph_scene::pitch_ramp_lut(
        harmonigraph_scene::ViewConfig::default().pitch_gradient,
    );
    let ends = [
        ("dark", lut[0]),
        ("mid", lut[harmonigraph_scene::PITCH_LUT_N / 2]),
        ("bright", lut[harmonigraph_scene::PITCH_LUT_N - 1]),
    ];
    let shots = ends.map(|(end, color)| (end, sweep_over_color(&mut gpu, color)));
    // The rings alone. The slice under them wears the fixture's own synthetic
    // ramp, whose brightest entries sit high enough to buy a little slide of
    // their own; the claim here is about the color under test, and the steady
    // difference between the two end colors is what confines a reading to the
    // pixels that wear it — the same fence `lifted_pixels` builds.
    let ring: Vec<usize> = (0..shots[0].1 .0.len() / 4)
        .filter(|&i| shots[0].1 .0[i * 4..i * 4 + 3] != shots[2].1 .0[i * 4..i * 4 + 3])
        .collect();
    for (end, shot) in &shots {
        let moved: Vec<usize> = ring.iter().copied().filter(|&i| swept(shot, i)).collect();
        assert!(
            moved.len() > 200,
            "only {} ring pixels shimmered at the ramp's {end} end — the sweep is not \
             reaching the rings and the trough reading below would be noise",
            moved.len(),
        );
        // How far below its steady self the sweep ever takes a pixel, at the
        // pixel's own darkest moment of the cycle, averaged over the rings.
        let (mut dip, mut base) = (0.0, 0.0);
        for &i in &moved {
            let steady = lightness(&shot.0[i * 4..i * 4 + 4]);
            let low = shot
                .1
                .iter()
                .map(|f| lightness(&f[i * 4..i * 4 + 4]))
                .fold(steady, f64::min);
            dip += steady - low;
            base += steady;
        }
        let (dip, base) = (dip / moved.len() as f64, base / moved.len() as f64);
        eprintln!(
            "the {end} ramp color draws its rings at L* {base:.1}; the sweep's troughs sit \
             {dip:.2} under that"
        );
        let allowed = match *end {
            "dark" => 1.0,
            "mid" => 6.0,
            _ => 17.0,
        };
        assert!(
            dip < allowed,
            "at the ramp's {end} end the sweep holds {dip:.1} L* of standing shade between \
             its peaks (the budget there is {allowed}): the troughs are not the steady \
             layer, which is the promise SHIMMER_CEILING exists to keep",
        );
    }
}

/// A ring keeps its color under a peak — the sweep lights it rather than
/// bleaching it, and lights it rather than turning it some other color.
///
/// HUE is what the sheet holds and chroma is what it spends, and the two bounds
/// below are that trade written down rather than one property measured twice.
///
/// A crest that overflows a channel is desaturated toward the grey of its own
/// light, not clipped. Mixing all three channels toward one value moves them
/// together, so their order survives and the color pales along its own hue; a
/// per-channel clip stops the full channel and lets the others climb past it,
/// which turns the color as it brightens. At Intensity 1 that is the whole
/// difference: 0.7 and 5.0 degrees here against the addition's 0.5 and 15.3.
///
/// The chroma goes the other way and is meant to. The addition keeps 99.6% and
/// 73%; this keeps 88% and 57%, because a uniform sheet wants the ramp's bright
/// end near `L*` 90 and the gamut has almost no chroma to offer that hue up
/// there. The bound is what the trade is allowed to cost, and it sits well
/// clear of the mix toward white this is not — that leaves 15% at every point
/// on the ramp, which is a bleach rather than a highlight.
///
/// BOTH ends, because they spend differently. The dark end has light to give
/// and pays little. The bright end has none and pays most of what is paid,
/// which is where a bound set on the dark end alone would measure nothing.
///
/// Hue as well as chroma, because chroma that survives a rotated hue is a ring
/// that has changed color rather than one that has lit up, and a max-minus-min
/// reading cannot tell those apart. The chroma proxy is the spread between a
/// pixel's channels, which is not a perceptual chroma and does not need to be —
/// it is zero exactly when the color is grey, it moves monotonically with how
/// far from grey the color is, and every shot is read the same way.
#[test]
fn a_ring_keeps_its_color_under_a_sweep_peak() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let lut = harmonigraph_scene::pitch_ramp_lut(
        harmonigraph_scene::ViewConfig::default().pitch_gradient,
    );
    // One pair of bounds for both ends rather than a number dialled per end:
    // the bright end is what they are set from, since it is the end with no
    // room above it, and a per-end figure would let a retune that started
    // bleaching the dark end pass by being compared against itself.
    //
    // Half the chroma, against a reading of 57% at the end that pays; and 8
    // degrees of hue, against 5.0 there, where an addition needs
    // 20 to pass at all. The hue bound is the one doing the work — it sits three
    // times inside the addition's 15.3, so a model that went back to clipping
    // the channels separately fails it on the first peak. The chroma bound is a
    // BUDGET rather than a guarantee: it says the sheet may pale a bright crest
    // and may not bleach one, with the mix toward white's 15% at every point on
    // the ramp as the far side of that line.
    const KEEPS_CHROMA: f64 = 0.5;
    const HUE_SWING: f64 = 8.0;

    let ends = [("dark", lut[0]), ("bright", lut[harmonigraph_scene::PITCH_LUT_N - 1])];
    for (end, color) in ends {
        let shot = sweep_over_color(&mut gpu, color);
        let lit = swept_pixels(&shot);
        assert!(lit.len() > 200, "only {} pixels shimmered at the {end} end", lit.len());

        let chroma = |px: &[u8]| {
            let (r, g, b) = (f64::from(px[0]), f64::from(px[1]), f64::from(px[2]));
            (r.max(g).max(b) - r.min(g).min(b)) / 255.0
        };
        let (steady, frames) = &shot;
        let (mut base_sum, mut peak_sum, mut swing_sum, mut swing_n) = (0.0, 0.0, 0.0, 0usize);
        for &i in &lit {
            let px = &steady[i * 4..i * 4 + 4];
            // The color at the pixel's OWN brightest moment, which is the moment
            // the claim is about — the chroma of some other frame would be a
            // reading of a peak that was somewhere else.
            let at = frames
                .iter()
                .map(|f| &f[i * 4..i * 4 + 4])
                .max_by(|a, b| lightness(a).total_cmp(&lightness(b)))
                .expect("eight frames");
            base_sum += chroma(px);
            peak_sum += chroma(at);
            // Only where both readings have a hue to compare. A pixel the peak
            // drives to grey has no angle, and counting the arbitrary one it
            // reports would read a bleach as a rotation — the chroma bound
            // above is what catches that pixel.
            if let (Some(was), Some(now)) = (hue_degrees(px), hue_degrees(at)) {
                let d = (now - was).abs();
                swing_sum += d.min(360.0 - d);
                swing_n += 1;
            }
        }
        let n = lit.len() as f64;
        let (base, peak) = (base_sum / n, peak_sum / n);
        let swing = swing_sum / swing_n.max(1) as f64;
        eprintln!(
            "{end} end: chroma {base:.3} steady, {peak:.3} at the peak; hue moves {swing:.1} deg"
        );
        assert!(
            peak >= base * KEEPS_CHROMA,
            "at the ramp's {end} end a peak left {peak:.3} of the ring's {base:.3} chroma: \
             the sheet is bleaching the color out rather than paling a crest of it, and \
             the budget for that is half",
        );
        assert!(
            swing < HUE_SWING,
            "at the ramp's {end} end a peak swung the ring's hue by {swing:.1} degrees: \
             the light is turning the color rather than lighting it, which is what \
             lifting the channels that have headroom and not the one that does not does",
        );
    }
}

/// Bloom must add light (halo energy over the bloom-off output) —
/// and only when asked: strength 0 keeps the parity test above valid.
/// One big centered node, sounding, with one octave slot lit: a clean
/// backdrop for measuring how much of the picture a mark actually
/// covers. parity_scene deliberately overlaps its nodes, which hides
/// most of a mark behind whatever draws in front of it.
fn single_marked_node(melody_slots: u32, bass_slots: u32) -> Scene {
    use glam::{Vec3, Vec4};
    use harmonigraph_core::LatticePos;

    let mut octaves = [0.0f32; harmonigraph_scene::OCTAVE_SLOTS];
    // Middle C's slot: the marks link back to a sector, so the note has to
    // sound in one the view actually shows.
    octaves[harmonigraph_scene::MIDDLE_C_SLOT] = 1.0;
    let mut scene = parity_scene();
    scene.nodes = vec![harmonigraph_scene::NodeInstance {
        lattice_pos: LatticePos::ORIGIN,
        world_pos: Vec3::ZERO,
        color: Vec4::new(0.35, 0.55, 0.85, 1.0),
        activation: 1.0,
        // Held at full, so neither end of the envelope is running.
        departing: false,
        octaves,
        hovered: false,
        on_home: true,
        scale: 1.0,
        gutter: 0.0,
        comma: 0.0,
        cents: 0.0,
        melody_slots,
        bass_slots,
        // A mark draws at the level its own note is at; these stand in for
        // a freshly-held note.
        melody_level: f32::from(melody_slots != 0),
        bass_level: f32::from(bass_slots != 0),
        // Distinct hues so the both-ends check below can tell the two
        // marks apart; in the app these are the marked SECTORS' colors,
        // which are the same color wherever the two name one sector.
        melody_color: Vec4::new(1.0, 0.85, 0.4, 1.0),
        bass_color: Vec4::new(0.45, 0.8, 1.0, 1.0),
        trail: 0.0,
    }];
    scene.grid.clear();
    // Fill a good share of the frame, so the measurements below are
    // about the mark's design rather than about pixel quantization.
    scene.node_radius = 1.1;
    scene
}

/// The slot beside middle C's, as a mask — a second sector for the two ends to
/// land on separately.
///
/// Taken off the layout the fixture actually draws rather than named: a mark on
/// a slot outside the drawn ring has no sector to extend and no angle to be
/// drawn at, so it would read as the mark having vanished.
fn slot_beside_middle_c() -> u32 {
    let (low, high) = harmonigraph_scene::OctaveLayout::default().slots(0.0);
    let c = harmonigraph_scene::MIDDLE_C_SLOT as i32;
    let beside = if c < high { c + 1 } else { c - 1 };
    assert!(
        (low..=high).contains(&beside) && beside != c,
        "the fresh wheel draws {low}..={high}, which has no second slot beside {c}",
    );
    1 << beside
}

#[test]
fn a_melody_bass_mark_extends_the_slice_it_names() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    let unmarked = gpu.shot(&single_marked_node(0, 0));
    let changed_px = |other: &[u8]| -> usize {
        unmarked
            .chunks(4)
            .zip(other.chunks(4))
            .filter(|(a, b)| a != b)
            .count()
    };

    // Measure against the node's OWN footprint, not an absolute pixel
    // count: what matters is that the mark claims a real share of the
    // thing it is marking, at whatever size it happens to be drawn.
    let node_px = unmarked.chunks(4).filter(|px| px[..3] != [0, 0, 0]).count();
    let melody = gpu.shot(&single_marked_node(MIDDLE_C, 0));
    let melody_px = changed_px(&melody);
    eprintln!("node {node_px} px; mark {melody_px}");
    // A floor, not a target, measured against the node's whole lit
    // footprint (glow included). A mark is ONE octave's slice continued
    // outward, so it claims a wedge rather than a ring — a fifth of the turn
    // on the fresh five-octave wheel. The floor exists because an early
    // version drew a sub-pixel arc that read as nothing at all in the DAW
    // (well under 1%), which is what this catches. Current: ~8%.
    assert!(
        melody_px * 32 > node_px,
        "the mark covers too little of the node to find: \
         {melody_px} px of {node_px}"
    );

    // Nothing marked draws no mark at all.
    let off = gpu.shot(&single_marked_node(0, 0));
    assert_eq!(changed_px(&off), 0, "an unmarked node must draw no mark");

    // A note claimed by BOTH ends on ONE octave -- a lone held note, or a
    // chord whose top and bottom share a pitch class -- must not be blanked:
    // that vanishes the mark exactly when two things are true at once. The
    // two name one slice, so what draws is that slice extended ONCE, over
    // exactly the pixels either end alone would have covered.
    let shared = gpu.shot(&single_marked_node(MIDDLE_C, MIDDLE_C));
    let shared_px = changed_px(&shared);
    eprintln!("both ends on one slice: {shared_px} px against {melody_px}");
    assert_eq!(
        shared_px, melody_px,
        "both ends on one octave drew a different shape from one end alone",
    );

    // Both ends on DIFFERENT octaves is the case only the shape can say: two
    // slices, each extended, so the picture covers more than either end alone
    // and matches neither.
    let beside = slot_beside_middle_c();
    let apart = gpu.shot(&single_marked_node(MIDDLE_C, beside));
    let apart_px = changed_px(&apart);
    let bass_only = gpu.shot(&single_marked_node(0, beside));
    eprintln!("two slices marked: {apart_px} px");
    assert!(
        apart_px > melody_px && apart_px > changed_px(&bass_only),
        "two marked octaves drew no more than one: {apart_px} px",
    );
    assert!(
        differing_pixels(&apart, &melody) > 0 && differing_pixels(&apart, &bass_only) > 0,
        "a two-octave mark is indistinguishable from a single-ended one",
    );
}

/// How far either side of a pitch the fixture's synthetic partial reaches, in
/// cents.
///
/// A BAND rather than a single bucket, and 40¢ rather than the 3.125¢ one
/// bucket spans, because the measurement below is made in pixels: the ring is
/// an annulus about 20 px from the node's centre in a 256 px shot, so a wedge
/// of the fresh five-octave wheel is some 25 px of arc, and one bucket at the
/// fresh 200¢ Range would be an eighth of a pixel of it. Symmetric, so the
/// lit arc's centroid is the band's own centre pitch whatever the Range.
const PARTIAL_HALF_CENTS: f32 = 40.0;

/// The padding `ringing_node` stands its layers off each other by — see there.
const PROBE_GAP: f32 = 0.12;

/// One node with both rings at the widths a FRESH view gives them: `held`
/// lighting an octave of the band, and a synthetic partial at absolute MIDI
/// `sounding` in the analyzer's grid for the audio ring to find.
///
/// The fresh widths rather than the fixture's, because the claim below is about
/// the shipped picture: that the ring is its own layer inside the octave band,
/// on a node nobody has dialled. The core is the one thing turned off — its
/// glow is light at every radius the measurements read, and what is being
/// measured is where two annuli are.
///
/// The PADDING is the fixture's own, and it is the one number here that has to
/// be: one Gap separates every layer of a node, and the fresh 0.052 of it is
/// under three pixels on the 52-px node this renders, where the two annuli's
/// anti-aliased edges meet inside it. A wider gap measures the geometry
/// rather than the edge softness. It is the same picture a fresh node draws,
/// read at a size a person looks at one from.
///
/// The ramp is a plain black-to-white one rather than a gradient, so a pixel's
/// brightness IS the level the shader read out of the grid and the differences
/// below measure the reading rather than a hue.
fn ringing_node(held: Option<usize>, sounding: Option<f32>, range: f32) -> Scene {
    let fresh = harmonigraph_scene::ViewConfig::default();
    let mut scene = single_marked_node(0, 0);
    let node = &mut scene.nodes[0];
    node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
    // Fully present whatever is lit, so the band's ghost ring is the same in
    // every shot and drops out of the differences below.
    node.activation = 1.0;
    if let Some(slot) = held {
        node.octaves[slot] = 1.0;
    }
    // The fresh stack at the probe's own padding, less its core: the two rings
    // land where a fresh node draws them, spaced far enough apart to be
    // measured in pixels.
    let rings =
        harmonigraph_scene::ViewConfig { ring_gap: PROBE_GAP, ..fresh.clone() }.rings();
    scene.core_radius = 0.0;
    scene.outer_inner = rings.band.0;
    scene.outer_outer = rings.band.1;
    scene.rings_outer = rings.outer;
    scene.mark_inner = rings.mark_inner;
    scene.ring_gap = rings.gap;

    let mut paint = harmonigraph_scene::SpectralPaint::silent();
    paint.lut = std::array::from_fn(|k| {
        let t = k as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
        glam::Vec4::new(t, t, t, 1.0)
    });
    (paint.inner, paint.outer) = rings.audio;
    paint.range = range;
    if let Some(pitch) = sounding {
        for (bucket, level) in paint.levels.iter_mut().enumerate() {
            let cents = (harmonigraph_scene::bucket_pitch(bucket) - pitch) * 100.0;
            if cents.abs() <= PARTIAL_HALF_CENTS {
                *level = 255;
            }
        }
    }
    scene.spectral = paint;
    scene
}

/// Per-pixel brightness of one shot less another's — how a wedge is separated
/// from the ghost ring it is drawn over, both shots carrying the same ghosts.
fn light_over(shot: &[u8], base: &[u8]) -> Vec<f64> {
    shot.chunks(4)
        .zip(base.chunks(4))
        .map(|(a, b)| (brightness(a) - brightness(b)).max(0) as f64)
        .collect()
}

/// Where a shot's light sits about the center of the FRAME, which is about
/// where a node at the world origin draws.
///
/// "About" is all it has to be. Every claim below compares two of these, and
/// both are read from the same place with the node in the same spot, so a
/// center a few pixels out moves them together and cancels — which is what
/// keeps this from needing the camera's projection written out a second time.
struct Light {
    /// Total brightness; 0 when nothing drew.
    weight: f64,
    /// The nearest and furthest a pixel worth seeing sits from that center, in
    /// pixels.
    near: f64,
    far: f64,
    /// The direction of the brightness-weighted centroid, in radians on the
    /// image's own axes — every claim compares two of these, so which way the
    /// screen's y runs never has to be settled.
    angle: f64,
}

/// How bright a pixel must be to count toward [`Light::near`]/[`Light::far`]:
/// past a wedge's antialiased fringe, which trails off over a couple of levels
/// and would otherwise put the extent a pixel either way.
const RING_LIT: f64 = 24.0;

fn light_about_center(weights: &[f64], size: [u32; 2]) -> Light {
    let (cx, cy) = ((size[0] - 1) as f64 / 2.0, (size[1] - 1) as f64 / 2.0);
    let (mut weight, mut near, mut far) = (0.0, f64::INFINITY, 0.0f64);
    let (mut sx, mut sy) = (0.0, 0.0);
    for (i, &w) in weights.iter().enumerate() {
        if w <= 0.0 {
            continue;
        }
        let x = (i % size[0] as usize) as f64 - cx;
        let y = (i / size[0] as usize) as f64 - cy;
        weight += w;
        sx += w * x;
        sy += w * y;
        if w >= RING_LIT {
            let r = x.hypot(y);
            near = near.min(r);
            far = far.max(r);
        }
    }
    Light { weight, near, far, angle: sy.atan2(sx) }
}

/// The short way round from `b` to `a`, in degrees, SIGNED on the image's own
/// axes.
///
/// Which sign means "clockwise on screen" is deliberately never settled here.
/// Every claim that needs a direction calibrates it from the picture itself —
/// the octave band's own wedges, an octave apart, are a known rise in pitch —
/// so nothing below depends on which way the shot's y runs.
fn signed_apart(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(std::f64::consts::TAU);
    if d > std::f64::consts::PI { d - std::f64::consts::TAU } else { d }.to_degrees()
}

/// The short way round between two angles, in degrees.
fn angle_apart(a: f64, b: f64) -> f64 {
    signed_apart(a, b).abs()
}

/// The audio ring reads the spectrum AROUND each octave: it draws inside the
/// octave band on the band's own angles, a partial dead on the node paints
/// down the middle of the wedge, and a detuned one paints off-centre in the
/// direction pitch rises — further off the narrower the Range is dialled.
///
/// Pixels rather than a reading of the shader's arithmetic, because every
/// claim here is geometric. Both rings walk `oct_sector` off one `OctRing`,
/// and the failures this catches all compile, validate, and read as a picture
/// that is subtly lying: a second ring drawn on its own idea of where a slot
/// is, a pitch window mapped backwards across the wedge, a Range that scales
/// the wrong way.
#[test]
fn the_audio_ring_reads_the_spectrum_around_each_octave() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // Octaves the fresh wheel draws on a C node — five slices centered on
    // middle C, so slots 3..=7. Middle C's own, the one above it (a known
    // rise in pitch, which is what calibrates the shot's handedness), and one
    // two below, far enough that a wedge at 72 degrees an octave is well clear
    // of any fringe.
    const UP: usize = harmonigraph_scene::MIDDLE_C_SLOT;
    const OVER: usize = harmonigraph_scene::MIDDLE_C_SLOT + 1;
    const DOWN: usize = harmonigraph_scene::MIDDLE_C_SLOT - 2;
    // The fixture's node is a C (`cents` 0), so slot s names MIDI 12 * s.
    let slot_pitch = |slot: usize| slot as f32 * 12.0;
    // The fresh Range, which every angle below is measured at unless it says
    // otherwise.
    let fresh_range = harmonigraph_scene::ViewConfig::default().spectral_ring_range;

    let base = gpu.shot(&ringing_node(None, None, fresh_range));
    let mut wedge = |held, sounding, range| {
        let shot = gpu.shot(&ringing_node(held, sounding, range));
        light_about_center(&light_over(&shot, &base), SIZE)
    };

    let band = wedge(Some(UP), None, fresh_range);
    let ring = wedge(None, Some(slot_pitch(UP)), fresh_range);
    assert!(ring.weight > 0.0, "the audio ring drew nothing at all");
    assert!(band.weight > 0.0, "the octave band drew nothing, so there is nothing to compare");
    eprintln!(
        "band {:.1}..{:.1} px at {:.1}°; ring {:.1}..{:.1} px at {:.1}°",
        band.near,
        band.far,
        band.angle.to_degrees(),
        ring.near,
        ring.far,
        ring.angle.to_degrees(),
    );
    // Inside, and clear of it: the ring's outermost lit pixel is nearer the
    // center than the band's innermost. A gap of at least a couple of pixels
    // at this size, so a ring that merely failed to overlap by a fraction of
    // one does not read as the design's "visible gap either side".
    assert!(
        ring.far + 2.0 < band.near,
        "the ring reaches {:.1} px against a band starting at {:.1}",
        ring.far,
        band.near,
    );
    // A partial exactly on the octave stands where that octave's own wedge
    // stands: the middle of it, which is the wheel's rule that an angle means
    // an absolute pitch, holding across both rings.
    let apart = angle_apart(ring.angle, band.angle);
    assert!(apart < 6.0, "a partial on the octave sits {apart:.1}° off the wedge that names it");

    // A different octave is a different angle in both — or the check above
    // would pass just as well for a ring pinned to one place on the node.
    let band_down = wedge(Some(DOWN), None, fresh_range);
    let ring_down = wedge(None, Some(slot_pitch(DOWN)), fresh_range);
    let moved = angle_apart(ring_down.angle, ring.angle);
    assert!(moved > 60.0, "two octaves apart moved the ring's wedge only {moved:.1}°");
    let apart = angle_apart(ring_down.angle, band_down.angle);
    assert!(apart < 6.0, "the lower octave sits {apart:.1}° off the wedge that names it");

    // Which way is UP on this shot, taken from the band itself: an octave
    // higher is a known rise in pitch, and the wheel turns clockwise with it.
    let rising = signed_apart(wedge(Some(OVER), None, fresh_range).angle, band.angle);
    assert!(
        rising.abs() > 30.0,
        "an octave moved the band only {rising:.1}°, so it cannot calibrate a direction",
    );

    // A partial a QUARTER of the window sharp lands a quarter of the wedge
    // clockwise of centre — the whole of what the segment is for, and the
    // reading a folded number per octave cannot give.
    let sharp = fresh_range / 4.0;
    let detuned = wedge(None, Some(slot_pitch(UP) + sharp / 100.0), fresh_range);
    let shift = signed_apart(detuned.angle, ring.angle);
    eprintln!(
        "{sharp:.0}¢ sharp moved the wedge {shift:.1}°, an octave of band {rising:.1}°",
    );
    assert!(
        shift * rising > 0.0,
        "{sharp:.0}¢ SHARP moved the wedge {shift:.1}° where rising pitch moves it \
         {rising:.1}°: the pitch window is mapped backwards across the wedge",
    );
    // A quarter of the window across a 72° wedge is 18°, and the lit arc is
    // 80¢ of a 200¢ window wide, so its centroid moves with its centre. Well
    // inside the wedge either way — a shift that ran off the end would clamp
    // and read as a smaller one, which is the other way this can fail.
    assert!(
        (shift.abs() - 18.0).abs() < 5.0,
        "a quarter-window detune moved the wedge {:.1}°, not the 18° a quarter of it is",
        shift.abs(),
    );

    // ...and the Range is a ZOOM: the same detuning, read over twice the
    // window, moves the wedge half as far.
    let wide = wedge(None, Some(slot_pitch(UP) + sharp / 100.0), fresh_range * 2.0);
    let wide_shift = signed_apart(wide.angle, ring.angle);
    eprintln!("the same {sharp:.0}¢ over twice the Range moved it {wide_shift:.1}°");
    assert!(
        (wide_shift.abs() * 2.0 - shift.abs()).abs() < 5.0,
        "twice the Range moved the same detune {:.1}° against {:.1}° at the fresh one, \
         which is not half",
        wide_shift.abs(),
        shift.abs(),
    );

    // The ring OFF draws nothing, whatever the grid holds: the empty annulus
    // is how the toggle reaches the shader, so this is the "exactly today's
    // picture" claim in its smallest form.
    let mut off = ringing_node(None, Some(slot_pitch(UP)), fresh_range);
    off.spectral.inner = 0.0;
    off.spectral.outer = 0.0;
    let quiet = {
        let mut quiet = ringing_node(None, None, fresh_range);
        quiet.spectral.inner = 0.0;
        quiet.spectral.outer = 0.0;
        gpu.shot(&quiet)
    };
    assert_eq!(
        differing_pixels(&gpu.shot(&off), &quiet),
        0,
        "a sounding partial drew something with the ring switched off",
    );
}

/// A melody/bass mark stands off the OUTERMOST RING the node draws, which on a
/// node with no octave band is the audio ring rather than the node's center.
///
/// The mark's inner edge is the one radius the shader is handed rather than
/// deriving. Deriving it from the BAND's outer edge is the same answer whenever
/// a band draws and the wrong one the moment that layer's width bar reaches 0:
/// the strip jumps inward across the whole node and lands against the core,
/// marking a slice of nothing.
///
/// Measured off the picture and not the uniform, because the two ways this
/// fails — the wrong radius packed, or the shader reading a different slot —
/// look identical from the Rust side.
#[test]
fn a_mark_stands_off_the_outermost_ring_the_node_draws() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    // The fresh stack at the probe's padding, so the layers are pixels apart:
    // core, audio ring, band, and the mark outside all three.
    let fresh = harmonigraph_scene::ViewConfig::default();
    let rings =
        harmonigraph_scene::ViewConfig { ring_gap: PROBE_GAP, ..fresh.clone() }.rings();
    let staged = |band: bool| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, 0);
        scene.core_radius = 0.0;
        scene.ring_gap = rings.gap;
        scene.mark_thickness = rings.mark_thickness;
        // The audio ring is drawn from an all-zero grid, which paints the
        // ramp's floor colour across the annulus — light at a known radius,
        // which is all this needs of it.
        let mut paint = harmonigraph_scene::SpectralPaint::silent();
        paint.lut = std::array::from_fn(|_| glam::Vec4::new(1.0, 1.0, 1.0, 1.0));
        (paint.inner, paint.outer) = rings.audio;
        scene.spectral = paint;
        (scene.outer_inner, scene.outer_outer) = if band { rings.band } else { (0.0, 0.0) };
        // A ring is on either way here, so the strip is owed its padding in
        // both — the case where it is not is
        // `a_mark_with_no_ring_under_it_reaches_the_nodes_centre`.
        scene.rings_outer = if band { rings.band.1 } else { rings.audio.1 };
        scene.mark_inner = scene.rings_outer + rings.gap;
        scene
    };

    // The mark alone, over the same node with the marks off: what is left is
    // the strip, wherever it landed.
    let mark_light = |gpu: &mut Shooter, band: bool| -> Light {
        let mut bare = staged(band);
        bare.nodes[0].melody_slots = 0;
        bare.nodes[0].melody_level = 0.0;
        let bare = gpu.shot(&bare);
        light_about_center(&light_over(&gpu.shot(&staged(band)), &bare), SIZE)
    };

    let with_band = mark_light(&mut gpu, true);
    let without = mark_light(&mut gpu, false);
    assert!(with_band.weight > 0.0 && without.weight > 0.0, "the mark drew nothing to measure");
    eprintln!(
        "mark {:.1}..{:.1} px with the band, {:.1}..{:.1} px without",
        with_band.near, with_band.far, without.near, without.far,
    );
    // The band is the wider stack, so its mark is further out — and by about
    // the band's own width, which is the slot the layer gave back.
    let band_px = with_band.near - without.near;
    let scale = with_band.far / (rings.band.1 + rings.gap + rings.mark_thickness) as f64;
    let want = (rings.band.1 - rings.audio.1) as f64 * scale;
    assert!(
        (band_px - want).abs() < 4.0,
        "dropping the band moved the mark in {band_px:.1} px, not the {want:.1} px \
         of band and gap it gave back",
    );
    // And the mark did NOT fall back to the node's center, which is what
    // anchoring it to a band that is not there would do.
    assert!(
        without.near > rings.audio.1 as f64 * scale - 4.0,
        "with the band off the mark starts at {:.1} px, inside the audio ring's own edge",
        without.near,
    );
}

/// With the core, the audio ring and the octave band ALL dialled off, the
/// melody/bass mark is the only layer the node has left — and it reaches the
/// node's CENTRE, rather than standing a padding off nothing.
///
/// [`stacked`](harmonigraph_scene::ViewConfig::rings) writes that rule down
/// for every layer it owns: the gap is skipped at a cursor of 0, where there
/// is nothing to stand off, so the innermost layer closes into a disc instead
/// of opening a hole the size of a padding around nothing. The mark is the one
/// layer it does NOT own — the strip's inner edge is re-derived in WGSL off
/// `rings_outer`, which is handed the cursor and not the rule — so the two
/// answers part company at exactly the one cursor the rule is about.
///
/// The state is a reduction the Lattice page's own bars reach: Core radius,
/// Ring width and Band width all have 0 as their off position, and reading the
/// lattice as melody/bass marks alone is what taking all three there is for.
/// Every other fixture in this file leaves a ring under the mark, where the
/// gap is owed and both answers agree.
#[test]
fn a_mark_with_no_ring_under_it_reaches_the_nodes_centre() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    // Two stacks off one view at the probe's wide padding: the band alone, and
    // nothing at all.
    //
    // The strip is dialled to its deepest on purpose. A sector's gap is a
    // constant EUCLIDEAN thickness at every radius (`outer_glyph`), so the two
    // edge lines blank a disc of half a padding about the node's centre — and
    // a strip no deeper than that disc would have nothing left to measure once
    // it reached the centre, which is the very state under test.
    let fresh = harmonigraph_scene::ViewConfig {
        ring_gap: PROBE_GAP,
        core_radius: 0.0,
        spectral_ring_width: 0.0,
        mark_thickness: harmonigraph_scene::MARK_THICKNESS_MAX,
        ..harmonigraph_scene::ViewConfig::default()
    };
    let band_only = fresh.rings();
    let empty = harmonigraph_scene::ViewConfig { band_width: 0.0, ..fresh.clone() }.rings();
    assert!(band_only.outer > 0.0, "the reference stack must draw a ring");
    assert_eq!(empty.outer, 0.0, "the fixture must empty the stack to test anything");

    let staged = |rings: &harmonigraph_scene::RingStack, mark: bool| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, 0);
        scene.core_radius = rings.core_radius;
        scene.ring_gap = rings.gap;
        scene.mark_thickness = rings.mark_thickness;
        // Silent paint carries the empty pair, so the audio ring is off the
        // way the bar leaves it rather than merely unlit.
        scene.spectral = harmonigraph_scene::SpectralPaint::silent();
        (scene.outer_inner, scene.outer_outer) = rings.band;
        scene.rings_outer = rings.outer;
        scene.mark_inner = rings.mark_inner;
        if !mark {
            scene.nodes[0].melody_slots = 0;
            scene.nodes[0].melody_level = 0.0;
        }
        scene
    };

    // The mark alone, read off the same node with the marks off, so what the
    // difference holds is the strip and nothing under it.
    let mark_light = |gpu: &mut Shooter, rings: &harmonigraph_scene::RingStack| -> Light {
        let bare = gpu.shot(&staged(rings, false));
        light_about_center(&light_over(&gpu.shot(&staged(rings, true)), &bare), SIZE)
    };

    let reference = mark_light(&mut gpu, &band_only);
    let stripped = mark_light(&mut gpu, &empty);
    assert!(reference.weight > 0.0 && stripped.weight > 0.0, "the mark drew nothing");

    // The strip's OUTER edge is what both readings are taken from: it is the
    // one end the octave gap does not eat into, since a sector is wider than
    // the padding out there and narrower than it near the node's centre.
    // Calibrated on the reference, where a ring IS under the strip and the
    // padding is genuinely owed.
    let want_ref = (band_only.mark_inner + band_only.mark_thickness) as f64;
    let scale = reference.far / want_ref;
    let far_uv = stripped.far / scale;
    eprintln!(
        "band under it: {:.1} px = {want_ref:.4} uv ({:.1} px/uv); \
         nothing under it: {:.1} px = {far_uv:.4} uv, thickness {:.4}, gap {:.4}",
        reference.far, scale, stripped.far, empty.mark_thickness, empty.gap,
    );
    assert!(
        (far_uv - empty.mark_thickness as f64).abs() < empty.gap as f64 / 2.0,
        "with every ring off the strip reaches {far_uv:.4} uv, not the {:.4} it is deep — \
         it is standing a padding off nothing, with a hole at the node's centre. \
         `stacked` skips the gap at a cursor of 0 and the strip has to skip it too",
        empty.mark_thickness,
    );
}

/// An octave band the stack switched off — the empty pair (0, 0) — paints
/// NOTHING, rather than a dot at the node's centre.
///
/// `glyph_band` is two soft edges multiplied together, and at inner == outer
/// they cross instead of cancelling: a pixel at the node's centre is half inside
/// each, so the layer answers a quarter coverage where the whole point of the
/// pair was that there is no layer. It is the one radius pair whose arithmetic
/// draws a shape, which is why the shader gates the band on `band_out > band_in`
/// rather than trusting the geometry to say off by drawing nothing.
///
/// Measured against a frame with NO node in it, because the artifact is what a
/// node with every layer off still paints. Differencing two shots that both
/// carry the band — how `a_mark_stands_off_the_outermost_ring_the_node_draws`
/// reads the mark out — is exactly what cannot see this: the dot is in both and
/// cancels.
#[test]
fn a_band_dialled_off_paints_no_dot_at_the_nodes_centre() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    // Every layer of the node off: no core, no audio ring, no marks, and the
    // octave band at the empty pair the stack hands over when it cannot fit the
    // layer. What is left for the node to draw is nothing — and with `node`
    // false, the same frame with no node in it at all, which is the ground.
    //
    // The padding is 0 because the sector gaps all CONVERGE on the node's
    // centre, which is where the artifact is: at the fresh gap they eat most of
    // it, and the dot this is looking for shows at its own size.
    let collapsed = |node: bool| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.core_radius = 0.0;
        scene.mark_thickness = 0.0;
        scene.rings_outer = 0.0;
        scene.ring_gap = 0.0;
        (scene.outer_inner, scene.outer_outer) = (0.0, 0.0);
        scene.spectral = harmonigraph_scene::SpectralPaint::silent();
        if !node {
            scene.nodes.clear();
        }
        scene
    };

    // Any deviation at all, rather than a threshold: what the node is allowed
    // to paint here is nothing, so the ground IS the answer, pixel for pixel.
    // The dot is dim as well as small — six pixels at up to 5/255 over black,
    // which is under a fifth of what `light_about_center` calls lit — so a
    // brightness floor would read it as absent.
    let ground = gpu.shot(&collapsed(false));
    let painted = gpu.shot(&collapsed(true));
    let px = differing_pixels(&painted, &ground);
    let light = light_about_center(&light_over(&painted, &ground), SIZE);
    eprintln!("a node with every layer off: {px} px, {:.0} of light", light.weight);
    assert_eq!(
        px, 0,
        "a node with every layer off painted {px} px, {:.0} of light",
        light.weight,
    );

    // Not a vacuous fixture: hand the same node a real annulus and the same
    // measurement finds it. Without this, a node that drew nothing for some
    // other reason — discarded, off screen, black on black — would read as the
    // empty pair being honoured.
    let mut drawn = collapsed(true);
    (drawn.outer_inner, drawn.outer_outer) = (0.4, 0.7);
    drawn.rings_outer = 0.7;
    let band = light_about_center(&light_over(&gpu.shot(&drawn), &ground), SIZE);
    assert!(band.weight > 0.0, "the fixture paints no band even when it is given one");
}

/// The FOLD reading fills the same wedges of the same annulus, and reads each
/// of them at its own octave's PITCH: a wedge is flat, so nothing about the
/// picture depends on Range and a detuned partial dims rather than moving.
///
/// The pair of claims that make the two readings one control over one
/// indicator rather than two features. Both are things the raw reading does
/// the other way — its wedge is a window, so Range zooms it and a detuning
/// slides across it — and a fold that quietly kept the window would pass
/// every geometric claim above while drawing the wrong picture.
#[test]
fn the_folded_ring_reads_each_wedge_at_its_own_octave() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    const UP: usize = harmonigraph_scene::MIDDLE_C_SLOT;
    let slot_pitch = |slot: usize| slot as f32 * 12.0;
    let fresh_range = harmonigraph_scene::ViewConfig::default().spectral_ring_range;
    // The same fixture as the raw reading's, with the wedges read at their own
    // pitch instead of across a window.
    let folded = |sounding: Option<f32>, range: f32| {
        let mut scene = ringing_node(None, sounding, range);
        scene.spectral.folded = true;
        scene
    };

    let base = gpu.shot(&folded(None, fresh_range));
    let raw_base = gpu.shot(&ringing_node(None, None, fresh_range));

    // It draws, in the same annulus and at the same angle the octave's own
    // wedge stands at — the raw reading's own claim, so the two are one ring.
    let on = {
        let shot = gpu.shot(&folded(Some(slot_pitch(UP)), fresh_range));
        light_about_center(&light_over(&shot, &base), SIZE)
    };
    assert!(on.weight > 0.0, "the folded ring drew nothing at all");
    let raw = {
        let shot = gpu.shot(&ringing_node(None, Some(slot_pitch(UP)), fresh_range));
        light_about_center(&light_over(&shot, &raw_base), SIZE)
    };
    let apart = angle_apart(on.angle, raw.angle);
    assert!(apart < 6.0, "the folded wedge sits {apart:.1}° off the raw one for the same octave");
    assert!(
        (on.near - raw.near).abs() < 3.0 && (on.far - raw.far).abs() < 3.0,
        "the folded ring runs {:.1}..{:.1} px against the raw one's {:.1}..{:.1}",
        on.near,
        on.far,
        raw.near,
        raw.far,
    );

    // Range is inert: a wedge is one reading, so there is no window for it to
    // zoom. Pixel-exact, since the shader does not read the setting at all
    // down this branch.
    let narrow = gpu.shot(&folded(Some(slot_pitch(UP)), 50.0));
    let wide = gpu.shot(&folded(Some(slot_pitch(UP)), 1200.0));
    assert_eq!(
        differing_pixels(&narrow, &wide),
        0,
        "Range changed the folded ring, which has no window to size",
    );

    // ...and a detuned partial DIMS where the raw reading would slide it
    // across the wedge: half the fresh window sharp is a quarter-wedge move
    // there and no move at all here. The fixture's partial is a rectangle
    // PARTIAL_HALF_CENTS wide, so at half a window off it has left the
    // octave's own pitch entirely and the wedge goes dark.
    let off_pitch = {
        let shot = gpu.shot(&folded(Some(slot_pitch(UP) + fresh_range / 200.0), fresh_range));
        light_about_center(&light_over(&shot, &base), SIZE)
    };
    eprintln!("on {:.0}, half a window off {:.0}", on.weight, off_pitch.weight);
    assert!(
        off_pitch.weight < 0.25 * on.weight,
        "a partial half a window off still lit the wedge at {:.0} against {:.0} on pitch",
        off_pitch.weight,
        on.weight,
    );
}

/// A sheet must draw differently from Off and must move with the clock.
///
/// Off must ALSO be steady across the clock, which is the half that keeps the
/// rest honest: a picture that moved with time in every mode would pass the
/// two "it changed" claims below without the sheet doing anything. It is
/// checked on a node with NO mark at all, which is also the containment claim
/// `the_mark_shimmer_reaches_the_octave_slice_it_points_at` makes in full:
/// nothing about an unmarked node depends on the clock.
///
/// The instants are picked without reference to the fixture's own speed or
/// width: the claim is that the clock reaches the layer, not that a
/// particular phase does, so retuning the sweep cannot make this pass by
/// accident. (Which is also why this one does NOT read
/// [`PARITY_SHIMMER_WIDTH`] the way `SHIMMER_PROBE_STEP` has to — a probe
/// that asks WHERE the bands are needs sizing against them; one that asks
/// whether they move at all does not.)
#[test]
fn the_mark_shimmer_sweeps_the_rings_and_moves_with_time() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    let mut off = single_marked_node(0, 0);
    off.now = 0.4;
    let off_a = gpu.shot(&off);
    off.now = 1.1;
    let off_b = gpu.shot(&off);
    assert_eq!(differing_pixels(&off_a, &off_b), 0, "Pulse::Off must not depend on scene.time");

    // The rings need a mark to exist at all -- that is the ring, not the
    // shimmer -- so this marks one end, and the steady shot of the same
    // fixture is what isolates what `pulse_marks` did.
    let mut ring_off = single_marked_node(MIDDLE_C, 0);
    ring_off.now = 0.4;
    let ring_off_a = gpu.shot(&ring_off);

    let mut marks = single_marked_node(MIDDLE_C, 0);
    marks.pulse_marks = harmonigraph_scene::Pulse::Bands;
    marks.now = 0.4;
    let marks_a = gpu.shot(&marks);
    assert!(
        differing_pixels(&ring_off_a, &marks_a) > 0,
        "the mark rings' sheet drew the steady picture"
    );
    marks.now = 1.1;
    let marks_b = gpu.shot(&marks);
    assert!(
        differing_pixels(&marks_a, &marks_b) > 0,
        "the mark rings' sheet did not change between two different \
         times; it is not reading the clock"
    );
}

/// The sweep's two settings reach the picture, and the clock reaches it only
/// THROUGH the speed.
///
/// The last part is what makes this more than two "something changed"
/// probes. Speed and width both scale the same phase (`travel / period`,
/// with the clock inside `travel`), so a width that had quietly taken the
/// clock's term with it, or a speed read as a frequency in the band count,
/// would still move the picture on both bars and still animate — and would
/// have made the two knobs one. At speed 0 the bands must stand still while
/// the clock runs, whatever the width is set to.
#[test]
fn the_shimmers_speed_and_width_reach_the_picture_and_only_speed_carries_the_clock() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // Both ends marked, so the sheet has as much of the picture to fall
    // across as the fixture can give it: this is about the sheet's own shape
    // and pace, not about what it crosses.
    let sweep = |speed: f32, width: f32, time: f64| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, MIDDLE_C);
        scene.pulse_marks = harmonigraph_scene::Pulse::Bands;
        scene.shimmer_speed = speed;
        scene.shimmer_width = width;
        scene.now = time;
        scene
    };

    // A time well off zero, or the speed would have nothing to multiply.
    let base = gpu.shot(&sweep(1.6, 5.0, 0.4));
    assert!(
        differing_pixels(&base, &gpu.shot(&sweep(3.2, 5.0, 0.4))) > 0,
        "the Speed bar did not move the bands: at a fixed instant it is what \
         says how far along their normal they have travelled",
    );
    assert!(
        differing_pixels(&base, &gpu.shot(&sweep(1.6, 2.5, 0.4))) > 0,
        "the Spacing bar did not resize the bands",
    );

    // Held still: the sheet is where it started at every instant, and stays
    // there through three widths spanning the resolvable half of the bar.
    for width in [2.5, 5.0, 12.0] {
        let (early, late) = (gpu.shot(&sweep(0.0, width, 0.4)), gpu.shot(&sweep(0.0, width, 9.7)));
        assert_eq!(
            differing_pixels(&early, &late),
            0,
            "at speed 0 the bands still moved between two instants at width \
             {width}; the clock is reaching the sweep by some route other than \
             the speed, and the two bars are not the independent pair they read as",
        );
    }
}

/// The mark rings' sweep reaches the slice WHOLE — both halves of a band's
/// swing, not just the bright one.
///
/// A band is an exposure around the layer's own color (`shimmer_light`): the
/// whole swing runs upward where the ceiling leaves it room, and slides below
/// the layer's color where the swing outruns that room — which this fixture's
/// ring colors do, so the sheet here has a dark half, and the dark half is
/// what gives it a body to travel through. The ring takes both. The slice that
/// ring names has to take both as well, or one mark is lit by two different
/// lights: the annulus dipping between bands while the wedge it points at
/// only ever brightens.
///
/// The dip is the half a plausible wiring drops, which is why it is measured
/// rather than assumed. The slice takes the sheet through a SWING scaled by how
/// much of the pixel is a wedge some ring points at, and a wiring that scaled
/// the band's shape instead would leave the slice sitting at a phase rather
/// than at rest — brightening perfectly well while never dipping, and looking
/// right everywhere except in the half nobody checks.
#[test]
fn the_mark_sheet_reaches_the_slice_whole() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // Tight enough that a band and the gap after it both fall across the
    // node, so one instant of the cycle has the slice in a trough rather
    // than the whole node riding one shoulder of a band five nodes wide.
    const WIDTH: f32 = 1.2;
    const SPEED: f32 = 1.6;
    let scene = |melody: u32, marks: harmonigraph_scene::Pulse, time: f64| -> Scene {
        let mut scene = single_marked_node(melody, 0);
        scene.pulse_marks = marks;
        scene.shimmer_width = WIDTH;
        scene.shimmer_speed = SPEED;
        scene.now = time;
        scene
    };
    let off = harmonigraph_scene::Pulse::Off;
    let shimmer = harmonigraph_scene::Pulse::Bands;

    // One cycle, walked: the trough has to pass over the slice somewhere in
    // it whatever phase the fixture happens to start at.
    let (mut dimmed_slice, mut dimmed_ring) = (0usize, 0usize);
    for step in 0..8 {
        let time = 0.2 + step as f64 * (WIDTH / SPEED) as f64 / 8.0;
        // The rings, from the node that wears none — the same mask
        // `the_mark_shimmer_reaches_the_octave_slice_it_points_at` takes, so
        // what is left is the glyph layer.
        let bare = gpu.shot(&scene(0, off, time));
        let steady = gpu.shot(&scene(MIDDLE_C, off, time));
        let swept = gpu.shot(&scene(MIDDLE_C, shimmer, time));
        for i in 0..swept.len() / 4 {
            let px = i * 4..i * 4 + 4;
            let on_ring = bare[px.clone()] != steady[px.clone()];
            let (before, after) = (brightness(&steady[px.clone()]), brightness(&swept[px]));
            // One count of rounding per channel: the two shots run the same
            // arithmetic to a different answer only where the sheet falls, and
            // a term that lands on a channel boundary can round down.
            if after < before - 3 {
                if on_ring {
                    dimmed_ring += 1;
                } else {
                    dimmed_slice += 1;
                }
            }
        }
    }
    eprintln!("mark sheet dimmed {dimmed_ring} px of ring and {dimmed_slice} px of slice");
    // The control: the sheet HAS a trough at these instants, and it reaches
    // the rings it is laid over. Without this the slice figure below could be
    // zero because nothing was sweeping at all.
    assert!(
        dimmed_ring > 0,
        "the mark rings' shimmer never dimmed a ring pixel across a whole cycle; \
         the sweep has no trough at these instants and the slice claim below is \
         measuring nothing",
    );
    // A floor rather than a share of the ring: how much of the band the
    // fixture shows is a setting, and the slice is one wedge of it against
    // two full annuli. Measured 625 px of slice against 5781 of ring; a
    // wiring that scaled the band's shape rather than its swing reads 0 of
    // slice against that same ring count, brightening at every phase and
    // never dipping.
    assert!(
        dimmed_slice > 200,
        "the mark rings' shimmer dimmed {dimmed_ring} px of ring but only \
         {dimmed_slice} px of the slice those rings point at: the sheet's trough \
         stops at the ring's edge, so the wedge only ever brightens and the one \
         mark is lit by two different lights",
    );
}

/// Intensity is the DEPTH of the sweep, and its bottom end is the identity:
/// at 0 the layer draws exactly as it does with the mode Off, byte for byte.
///
/// That last claim is the one worth pinning: `shimmer_light` is applied
/// unconditionally rather than behind the mode switch, so the bar's bottom
/// has to be the exact identity and not nearly one. A layer coming back a
/// rounding under itself would be a steady dimming with no shimmer in it —
/// on every frame, at a setting that reads as "off".
#[test]
fn shimmer_intensity_scales_the_sweep_and_bottoms_out_at_the_steady_layer() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // An instant with a band actually over the node: an intensity bar cannot
    // be read at a moment when there is nothing to scale.
    let at = |intensity: f32, pulse: harmonigraph_scene::Pulse| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, MIDDLE_C);
        scene.pulse_marks = pulse;
        scene.shimmer_intensity = intensity;
        scene.shimmer_speed = 0.0;
        scene.now = 0.4;
        scene
    };
    let off = harmonigraph_scene::Pulse::Off;
    let shimmer = harmonigraph_scene::Pulse::Bands;

    let steady = gpu.shot(&at(1.0, off));
    assert_eq!(
        differing_pixels(&steady, &gpu.shot(&at(0.0, shimmer))),
        0,
        "intensity 0 did not draw the steady layer: the sweep still has one of \
         its two terms running, which at the bottom of the bar is a standing \
         dimming rather than a shimmer",
    );

    let full = gpu.shot(&at(1.0, shimmer));
    let half = gpu.shot(&at(0.5, shimmer));
    assert!(differing_pixels(&steady, &full) > 0, "intensity 1 drew the steady layer");
    assert!(
        differing_pixels(&half, &full) > 0 && differing_pixels(&half, &steady) > 0,
        "half intensity is indistinguishable from full or from off; the bar is \
         a switch rather than a depth",
    );
    // ...and it is a depth in the direction the name says: a deeper setting
    // takes the picture FURTHER from its steady self.
    //
    // Departure rather than added light, because an exposure fitted to the
    // layer's headroom does not promise light in both directions and should not
    // be asked for it. Where a color has room the sheet adds light; where it has
    // none — this fixture's ring colors sit at the top of a channel, the ramp's
    // do not — the same setting is the same RATIO taken as shade instead, so
    // total light falls with intensity there while the sweep gets deeper. That
    // is the bar working, and a total-light reading would call it broken.
    let departure = |shot: &[u8]| -> f64 {
        shot.chunks(4)
            .zip(steady.chunks(4))
            .map(|(a, b)| (lightness(a) - lightness(b)).abs())
            .sum()
    };
    let (dim, mid, deep) = (departure(&steady), departure(&half), departure(&full));
    eprintln!("the sweep departs from steady by {dim:.0}/{mid:.0}/{deep:.0} at intensity 0/0.5/1");
    assert!(
        dim < mid && mid < deep,
        "the sweep did not deepen with intensity ({dim:.0}, {mid:.0}, {deep:.0}): a band \
         over the node has to move it further the deeper the sweep is",
    );
}

/// The tight end of the Spacing bar puts SEVERAL bands across one node at once
/// — a texture on the node rather than a sheet passing between nodes — which
/// is a different picture from the wide end and not just a smaller number.
///
/// Counted rather than eyeballed, from a profile taken along the bands' own
/// normal. Each pixel's shimmer is read as the RATIO of its light to the same
/// pixel's light with the mode Off, which cancels everything the node draws —
/// the gaps between sectors, the rings, the glow falling off — and leaves the
/// sweep alone. A band edge is where that ratio crosses the profile's OWN mean,
/// so counting crossings counts band edges, with no threshold picked to suit
/// the answer.
///
/// The mean and not 1. The sheet is an exposure fitted to each layer's
/// headroom, so where a color has no room above it — this fixture's ring
/// colors sit at the top of a channel —
/// the whole sweep is shade and the ratio never reaches 1 at all. Counting
/// crossings of 1 there finds a profile entirely on one side of the line and
/// reports no bands in a picture full of them. The mean rides wherever the
/// sweep put the profile and asks the question the test is actually about,
/// which is how many bands fit across one node.
///
/// A deadband either side of the mean keeps a bin that is merely quiet from
/// reading as a crossing; bins with little paint in them are dropped outright.
#[test]
fn a_tight_width_puts_several_bands_across_one_node() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let sweeping = |width: f32| -> Scene {
        // Both ends marked: the rings are two full annuli spanning the node,
        // so a profile taken across them samples the whole diameter the bands
        // have to fit into rather than one wedge of it.
        let mut scene = single_marked_node(MIDDLE_C, MIDDLE_C);
        scene.pulse_marks = harmonigraph_scene::Pulse::Bands;
        scene.shimmer_width = width;
        // Held still, so the profile is one instant of the sheet and not a
        // smear of where it was going.
        scene.shimmer_speed = 0.0;
        scene.now = 0.4;
        scene
    };
    let steady = {
        let mut scene = sweeping(5.0);
        scene.pulse_marks = harmonigraph_scene::Pulse::Off;
        gpu.shot(&scene)
    };

    // The bands run SHIMMER_ANGLE_TURNS (three eighths of a turn) off the
    // camera's right axis, toward its up axis — which is up the screen,
    // against the row index running down it. That direction is left-and-up, so
    // a fragment's place along the bands' normal is x + y, and binning by it
    // is binning by band phase. Four pixels to a bin: fine enough to resolve
    // the tightest width the bar reaches at this node size, coarse enough that
    // a bin holds a real sample.
    let mut bands_crossing = |width: f32| -> usize {
        let swept = gpu.shot(&sweeping(width));
        let bins = 2 * SIZE[0] as usize / 4;
        let mut lit = vec![0i64; bins];
        let mut here = vec![0i64; bins];
        for (i, (a, b)) in steady.chunks(4).zip(swept.chunks(4)).enumerate() {
            let (x, y) = (i % SIZE[0] as usize, i / SIZE[0] as usize);
            let bin = (x + y) / 4;
            lit[bin] += a[0] as i64 + a[1] as i64 + a[2] as i64;
            here[bin] += b[0] as i64 + b[1] as i64 + b[2] as i64;
        }
        // A bin needs real paint in it before its ratio means anything; the
        // node covers a fraction of the frame, and the empty ground either
        // side of it would otherwise contribute a ratio of 0/0.
        let floor = lit.iter().max().copied().unwrap_or(0) / 8;
        let ratios: Vec<f64> = lit
            .iter()
            .zip(&here)
            .filter(|(l, _)| **l >= floor.max(1))
            .map(|(l, h)| *h as f64 / *l as f64)
            .collect();
        let mean = ratios.iter().sum::<f64>() / ratios.len().max(1) as f64;
        let mut crossings = 0;
        let mut above: Option<bool> = None;
        for (l, h) in lit.iter().zip(&here) {
            if *l < floor.max(1) {
                continue;
            }
            let ratio = *h as f64 / *l as f64;
            let now = if ratio > mean * 1.01 {
                Some(true)
            } else if ratio < mean * 0.99 {
                Some(false)
            } else {
                None
            };
            if let (Some(was), Some(is)) = (above, now) {
                if was != is {
                    crossings += 1;
                }
            }
            above = now.or(above);
        }
        crossings
    };

    // At the WIDE end of the bar the node is a fraction of one band, so its
    // whole paint sits on one side of the sweep or slides across at most one
    // edge of it. (Not the fresh view's width, which sits down near the tight
    // end — see `PARITY_SHIMMER_WIDTH`.)
    let wide = bands_crossing(PARITY_SHIMMER_WIDTH);
    // At a tight one the node is several bands across. The fixture's node is
    // 1.1 world units in radius against a width of 0.35, so the octave band
    // alone spans about five. Measured 1 edge wide against 21 tight.
    let tight = bands_crossing(0.35);
    eprintln!("band edges across the node: {wide} wide, {tight} tight");
    assert!(
        wide <= 2,
        "the wide end already puts {wide} band edges across one node; it is \
         supposed to be a sheet crossing the lattice, and the two ends of the \
         bar are then the same picture",
    );
    assert!(
        tight >= 6,
        "a width of 0.35 put only {tight} band edges across the node (the wide \
         end puts {wide}); the tight end of the bar is not reaching the \
         several-bands-per-node look it exists for",
    );
}

/// Past the tight end the sheet runs out of PIXELS to be drawn in, and what
/// it does there is fade out rather than alias.
///
/// A sine sampled once per fragment stops meaning anything at half a period
/// to the pixel: past that the pattern does not get finer, it turns into a
/// moire of the sampling grid, which crawls as the camera moves and lands
/// differently at every render size — the one thing the sweep's world units
/// exist to avoid. `shimmer_terms` fades the depth out over
/// SHIMMER_RESOLVE_FULL..GONE instead, so the layer settles back onto exactly
/// the picture Off draws.
///
/// The two claims are a pair and neither means much alone: a width the frame
/// CAN resolve has to still sweep (or the fade is just a broken sheet), and
/// the finest width the bar reaches has to be pixel-identical to Off (or the
/// fade stops somewhere short of the identity and leaves a haze that no
/// setting can clear).
#[test]
fn a_width_finer_than_the_pixels_fades_out_instead_of_aliasing() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    let at = |width: f32| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, MIDDLE_C);
        scene.pulse_marks = harmonigraph_scene::Pulse::Bands;
        scene.shimmer_width = width;
        // Held still: a fade measured across two instants would be measuring
        // the travel as well.
        scene.shimmer_speed = 0.0;
        scene.now = 0.4;
        scene
    };
    let steady = {
        let mut scene = at(0.35);
        scene.pulse_marks = harmonigraph_scene::Pulse::Off;
        gpu.shot(&scene)
    };

    // The same width the test above counts fifteen band edges at, which this
    // node's pixels carry comfortably.
    let resolvable = differing_pixels(&steady, &gpu.shot(&at(0.35)));
    // The floor `derive_scene` clamps the Spacing bar to.
    let finest = differing_pixels(&steady, &gpu.shot(&at(0.02)));
    eprintln!("pixels swept: {resolvable} at a resolvable width, {finest} at the floor");
    assert!(
        resolvable > 0,
        "a width the frame can resolve swept nothing; the fade is eating the sheet \
         well before the pixels run out",
    );
    assert_eq!(
        finest, 0,
        "the finest width the bar reaches still moved {finest} px against the steady \
         layer, so what it is drawing there is a moire of the pixel grid rather \
         than the picture Off draws",
    );
}

/// The mark rings' shimmer also sweeps the octave SLICE each ring points at,
/// which is drawn by the glyph layer — a mark is the ring together with the
/// octave it names, and light crossing the one has to cross the other or it
/// cuts the mark in half at the gap between them.
///
/// The claim is about paint OUTSIDE the rings, so the rings are masked off
/// rather than switched off: `mark_thickness = 0` would take the rings and
/// the slice sweep with them (`the_mark_pulse_folds_off_when_the_rings_are_off`
/// in harmonigraph-scene folds the mode there, and a fixture the app cannot
/// build is not a reading of what it draws). The mask is measured instead —
/// an unmarked node wears no rings, so the pixels a marked one differs from
/// it at ARE the rings, fringe and all, whatever radii the band setting put
/// them at. What is left is the rest of the node, where only the glyph layer
/// draws.
#[test]
fn the_mark_shimmer_reaches_the_octave_slice_it_points_at() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // One instant, held across all four shots: the sweep moves, so anything
    // compared across two clocks would differ whatever it drew.
    let at = |melody: u32, pulse: harmonigraph_scene::Pulse| -> Scene {
        let mut scene = single_marked_node(melody, 0);
        scene.now = 0.4;
        scene.pulse_marks = pulse;
        scene
    };
    let off = harmonigraph_scene::Pulse::Off;
    let shimmer = harmonigraph_scene::Pulse::Bands;

    // No mark: no ring to sweep and no slice to reach, so the mode changes
    // nothing at all. This is the containment half of the claim — the mark
    // layer's sweep must not have become a second octave-layer sweep.
    let bare = gpu.shot(&at(0, off));
    let bare_shimmer = gpu.shot(&at(0, shimmer));
    assert_eq!(
        differing_pixels(&bare, &bare_shimmer),
        0,
        "an unmarked node changed under the mark rings' shimmer; the sweep has \
         escaped the slices a ring points at and is crossing the whole octave layer",
    );

    let steady = gpu.shot(&at(MIDDLE_C, off));
    let swept = gpu.shot(&at(MIDDLE_C, shimmer));
    // Where the rings draw, from the node that wears none.
    let ring = |i: usize| bare[i * 4..i * 4 + 4] != steady[i * 4..i * 4 + 4];
    let (mut on_ring, mut past_ring) = (0usize, 0usize);
    for i in 0..steady.len() / 4 {
        if steady[i * 4..i * 4 + 4] == swept[i * 4..i * 4 + 4] {
            continue;
        }
        if ring(i) {
            on_ring += 1;
        } else {
            past_ring += 1;
        }
    }
    eprintln!("mark shimmer moved {on_ring} px of ring and {past_ring} px past it");
    // A floor rather than a share of the rings: the slice is one wedge of the
    // band against two full annuli, and how much of the band the fixture
    // shows is a setting. Measured 599 px past the ring, against 940 on it.
    assert!(
        past_ring > 200,
        "the mark rings' shimmer moved only {past_ring} px outside the rings \
         ({on_ring} on them): it is sweeping the annulus alone and stopping at \
         the gap, leaving the octave slice the mark names unlit",
    );
}

/// Mirrors `SHIMMER_ANGLE` in lattice.wgsl, as a fraction of a turn from the
/// camera's right axis toward its up axis — the direction the bands travel.
///
/// Held to the shader's own literal by `the_probe_moves_along_the_angle_the_shader_sweeps`
/// rather than by a comment asking for it: the probe below moves a node across
/// this direction and along it and reads how much each move costs, so an angle
/// that drifted from the shader's would leave the test comparing two arbitrary
/// directions — passing on its margin while measuring nothing about the sheet.
const SHIMMER_ANGLE_TURNS: f32 = 0.375;

/// The mirror above, enforced. `SHIMMER_ANGLE` is a tuning knob for the look —
/// which diagonal the light rakes across — and retuning it is exactly the edit
/// that would strand the probe.
#[test]
fn the_probe_moves_along_the_angle_the_shader_sweeps() {
    let needle = format!("const SHIMMER_ANGLE: f32 = {SHIMMER_ANGLE_TURNS} * TAU;");
    assert!(
        SHADER_SRC.contains(&needle),
        "lattice.wgsl must declare `{needle}` to match SHIMMER_ANGLE_TURNS; the probe in \
         the_shimmer_is_one_field_across_the_lattice moves nodes across that angle and \
         along it, and against a different one it measures neither",
    );
}

/// How far that probe moves the node, in world units: half the band width
/// the fixtures sweep at, so a move ACROSS the bands lands it on a very
/// different part of the sweep rather than back where it started.
///
/// Derived from [`PARITY_SHIMMER_WIDTH`] rather than written as 2.5, because
/// the width is a SETTING now and the fixture picks it. Retuned by hand the
/// two would drift apart silently, and a step that came out a whole number of
/// widths would move the node back onto the phase it started at — the probe
/// would then report a shimmer defect for a shimmer that is working, which is
/// the same trap `the_probe_moves_along_the_angle_the_shader_sweeps` keeps
/// the ANGLE out of.
const SHIMMER_PROBE_STEP: f32 = PARITY_SHIMMER_WIDTH * 0.5;

/// `scene`'s only node, moved [`SHIMMER_PROBE_STEP`] world units along the
/// camera-plane direction `turns` of a turn from the camera's right axis.
fn move_node_across_the_view(scene: &mut Scene, turns: f32) {
    let (right, up) = scene.camera.right_up();
    let a = turns * std::f32::consts::TAU;
    scene.nodes[0].world_pos = (right * a.cos() + up * a.sin()) * SHIMMER_PROBE_STEP;
}

/// The sheet is ONE field across the lattice, not a copy per node — the claim
/// that is the whole point of the shimmer, and that the tests above would pass
/// without.
///
/// The field is the fragment's place on the plane the billboards face, so a
/// node MOVED across that plane meets the bands at a different phase and draws
/// with a different amount of light in it. Read off a per-node coordinate
/// (`in.uv`, say) every node would run an identical private copy, moving one
/// would change nothing but where it landed, and the "across" measurement
/// below would collapse into its control.
///
/// The control for that is the SAME move made along the bands instead —
/// perpendicular to the direction they travel, which slides the node down a
/// line the field is constant on and so leaves the picture the one it was.
/// The two directions are mirror images across the camera's up axis, so the
/// two moves put the node in exactly mirrored places: whatever the move costs
/// in rasterization and perspective, it costs both equally, and what is left
/// between them is the shimmer.
#[test]
fn the_shimmer_is_one_field_across_the_lattice() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // All the light in the frame. A total rather than a pixel-by-pixel
    // count, because a moved node lands in different pixels by design: what
    // is being compared is how much of it the shimmer let through, not
    // where it went.
    // How much the picture's total light changes when `make`'s node moves
    // `turns` of a turn off the camera's right axis, against leaving it at
    // the origin.
    let mut move_cost = |make: &dyn Fn() -> Scene, turns: f32| -> i64 {
        let still = total_light(&gpu.shot(&make()));
        let mut moved = make();
        move_node_across_the_view(&mut moved, turns);
        (total_light(&gpu.shot(&moved)) - still).abs()
    };

    let across_the_bands = SHIMMER_ANGLE_TURNS;
    let along_the_bands = SHIMMER_ANGLE_TURNS + 0.25;

    // The control: with nothing shimmering, a move costs only what moving
    // costs — a node landing on its own pixel grid, and the perspective at
    // a place that is not the middle of the frame.
    let steady = || {
        let mut scene = single_marked_node(MIDDLE_C, MIDDLE_C);
        scene.now = 0.4;
        scene
    };
    let steady_across = move_cost(&steady, across_the_bands);
    let steady_along = move_cost(&steady, along_the_bands);

    let marks = || {
        let mut scene = steady();
        scene.pulse_marks = harmonigraph_scene::Pulse::Bands;
        scene
    };
    let mark_across = move_cost(&marks, across_the_bands);
    let mark_along = move_cost(&marks, along_the_bands);

    eprintln!(
        "steady {steady_across}/{steady_along}, marks {mark_across}/{mark_along} (across/along)"
    );
    // The control has to STAY small, or the ratio below stops being about the
    // shimmer. Should a bare node move ever get expensive or lopsided — a new
    // depth-dependent layer, a cull edge inside the probe's reach, anything
    // keyed on world position — both figures would inflate off the same base,
    // the ratio would collapse, and the failure would be reported as a shimmer
    // defect it is not. Measured 83/110 against the sheet's 9930.
    let steady = steady_across.max(steady_along);
    assert!(
        steady * 10 < mark_across,
        "moving a node costs {steady} even with nothing shimmering, which is too near \
         what the shimmering layer costs ({mark_across}) for the difference between \
         them to be the shimmer's"
    );
    // A multiple, not a threshold: the claim is that crossing the bands
    // dominates sliding along them, and the along-figure is the same move
    // mirrored, so it carries the layer's own share of the control above.
    assert!(
        mark_across > mark_along * 4,
        "moving a node across the bands ({mark_across}) barely beat moving it \
         along them ({mark_along}; the steady control costs \
         {steady_across}/{steady_along}) -- either the field is per-node rather \
         than one sheet over the lattice, or the bands are not running the way \
         SHIMMER_ANGLE says"
    );
}

#[test]
fn a_real_held_chord_shows_its_melody_and_bass_marks() {
    // End to end, exactly how the app runs it: a held chord through
    // derive_scene, NOT a Scene assembled by hand. The by-hand test
    // above pins the shader down but would happily pass while the
    // tracker -> view -> node-mask path was broken, which is the half
    // that actually reaches a user.
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    use harmonigraph_core::{NoteEvent, NoteTracker, Tuning};
    use harmonigraph_scene::{derive_scene, Camera, FrameParams, ViewConfig};

    const SIZE: [u32; 2] = [256, 256];

    let mut tracker = NoteTracker::new();
    for note in [60u8, 64, 67] {
        tracker.handle_event(NoteEvent::on(0.0, 0, note, 1.0));
    }
    // A small window so the nodes draw big enough to measure.
    let base = ViewConfig {
        extent_threes: 2,
        extent_fives: 2,
        extent_sevens: 0,
        ..ViewConfig::default()
    };
    let scene_for = |marks: bool| {
        derive_scene(
            &tracker,
            &Tuning::default(),
            &ViewConfig { mark_melody: marks, mark_bass: marks, ..base.clone() },
            &base.reach(),
            // No envelope: every layer of a node eases in from its note-on
            // over the Fade, so under a real one t=0 is the instant nothing
            // is drawn yet and any later sample is a fraction. What is
            // compared below is a lit node against a lit node.
            &FrameParams { fade_time: 0.0, ..FrameParams::default() },
            Camera::default(),
            None,
            0.5,
        )
    };

    // The masks must survive derive_scene in the first place.
    let marked = scene_for(true);
    let melody_nodes = marked.nodes.iter().filter(|n| n.melody_slots != 0).count();
    let bass_nodes = marked.nodes.iter().filter(|n| n.bass_slots != 0).count();
    assert!(
        melody_nodes > 0 && bass_nodes > 0,
        "derive_scene marked nothing: {melody_nodes} melody, {bass_nodes} bass nodes"
    );

    let off = gpu.shot(&scene_for(false));
    let on = gpu.shot(&marked);
    let lit = off.chunks(4).filter(|px| px[..3] != [0, 0, 0]).count();
    let changed = off
        .chunks(4)
        .zip(on.chunks(4))
        .filter(|(a, b)| a != b)
        .count();
    eprintln!("chord: {lit} lit px, {changed} changed by the marks");
    // Same reasoning as the by-hand test above; at this node density the
    // ring's screen-space minimum (MARK_RING_MIN_AA) is what keeps it from
    // going sub-pixel.
    assert!(
        changed * 20 > lit,
        "turning the marks on barely changed a real chord: \
         {changed} px of {lit} lit"
    );
}

/// A node showing every octave it can, for reading the wheel's geometry off
/// the picture: no core and no mark rings, so the only thing drawn is the
/// band, and a wide gap so the seams between indicators are several pixels
/// across at the size this renders at.
fn octave_wheel_scene(layout: harmonigraph_scene::OctaveLayout, cents: f32) -> Scene {
    let mut scene = single_marked_node(0, 0);
    scene.octave_layout = layout;
    scene.core_radius = 0.0;
    scene.outer_inner = 0.30;
    scene.outer_outer = 0.95;
    scene.rings_outer = 0.95;
    scene.ring_gap = 0.10;
    scene.mark_thickness = 0.0;
    // Every octave the wheel draws for THIS pitch class, and only those: a
    // level on a slot no sector draws is a state `derive_scene` cannot reach,
    // and the glow would still take a color from it. Slots outside the packing
    // are what a ring near the pitch limits reaches for, and no note can light
    // one.
    let (low, high) = layout.slots(cents);
    let node = &mut scene.nodes[0];
    node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
    for slot in low.max(0)..=high.min(harmonigraph_scene::OCTAVE_SLOTS as i32 - 1) {
        node.octaves[slot as usize] = 1.0;
    }
    node.cents = cents;
    scene
}

/// Reads mean colors out of one wedge of a rendered node's octave band.
/// Self-calibrating — it finds the band's radii from a picture that has it
/// lit, rather than reproducing the camera's arithmetic, which would only
/// re-assert it.
struct BandProbe {
    size: [u32; 2],
    inner: f32,
    outer: f32,
}

impl BandProbe {
    /// Calibrated along `angle`, on a shot with the band drawn: the node is
    /// alone at the world origin and the camera looks at it, so the frame's
    /// center is its center.
    fn new(px: &[u8], size: [u32; 2], angle: f32) -> BandProbe {
        let mut probe = BandProbe { size, inner: 0.0, outer: 0.0 };
        let on_band: Vec<f32> = (4..size[0] / 2)
            .map(|r| r as f32)
            .filter(|&r| probe.at(px, r, angle).iter().sum::<f32>() > 24.0)
            .collect();
        assert!(!on_band.is_empty(), "nothing lit along the ray at {angle} rad");
        probe.inner = on_band[0];
        probe.outer = on_band[on_band.len() - 1];
        assert!(
            probe.outer - probe.inner > 8.0,
            "no band to sample: {}..{}",
            probe.inner,
            probe.outer
        );
        probe
    }

    /// One pixel, `r` from the center at `a` radians.
    fn at(&self, px: &[u8], r: f32, a: f32) -> [f32; 3] {
        let c = self.size[0] as f32 / 2.0;
        // Screen y grows downward, so the sample angle is negated.
        let (x, y) = (c + r * a.cos(), c - r * a.sin());
        let i = (y as usize * self.size[0] as usize + x as usize) * 4;
        [px[i] as f32, px[i + 1] as f32, px[i + 2] as f32]
    }

    /// Mean color well inside the wedge `width` wide centered on `angle`, in
    /// both directions, so neither the slice's antialiased edges nor the
    /// band's enter the reading.
    fn mean(&self, px: &[u8], angle: f32, width: f32) -> [f32; 3] {
        let (mut sum, mut n) = ([0f32; 3], 0f32);
        let margin = 0.2 * (self.outer - self.inner);
        let mut r = self.inner + margin;
        while r <= self.outer - margin {
            for k in -6..=6 {
                let sample = self.at(px, r, angle + 0.03 * k as f32 * width);
                for j in 0..3 {
                    sum[j] += sample[j];
                }
                n += 1.0;
            }
            r += 1.0;
        }
        sum.map(|s| s / n)
    }
}

/// The middle of slot `slot`'s wedge on a node whose pitch class is `cents`,
/// and how wide it is — the pair [`BandProbe::mean`] samples inside.
fn wedge_of(layout: harmonigraph_scene::OctaveLayout, slot: usize, cents: f32) -> (f32, f32) {
    let (e0, e1) = layout.sector(slot as i32, cents);
    (0.5 * (e0 + e1), e0 - e1)
}

/// The band's lit/unlit profile around a rendered node: index `i` is the
/// angle `360 * i / STEPS` counter-clockwise from screen right.
/// Self-calibrating — it finds the node's center and the band's radius from
/// the image rather than reproducing the camera's arithmetic, which would
/// only re-assert it.
const PROFILE_STEPS: usize = 720;
fn band_profile(px: &[u8], size: u32) -> Vec<bool> {
    let w = size as usize;
    let lit = |x: f32, y: f32| -> bool {
        if x < 0.0 || y < 0.0 || x >= size as f32 || y >= size as f32 {
            return false;
        }
        let i = (y as usize * w + x as usize) * 4;
        px[i] as u32 + px[i + 1] as u32 + px[i + 2] as u32 > 24
    };
    // The node is alone at the world origin and the camera looks at it, so
    // the frame's center is its center. Not the lit pixels' centroid: a
    // fringed band is heavier on the side its wide octaves fall, which would
    // pull a centroid off-center by roughly what the measurement below is
    // trying to see.
    let drawn = (0..size * size)
        .filter(|k| lit((k % size) as f32, (k / size) as f32))
        .count();
    assert!(drawn > 100, "nothing drawn to measure ({drawn} lit px)");
    let (cx, cy) = (size as f32 / 2.0, size as f32 / 2.0);

    // Sample at whichever radius has the most band on it: picking one by
    // arithmetic would land in a seam or off the band as the settings move.
    // Screen y grows downward, so the sample angle is negated.
    let ring = |r: f32| -> Vec<bool> {
        (0..PROFILE_STEPS)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / PROFILE_STEPS as f32;
                lit(cx + r * a.cos(), cy - r * a.sin())
            })
            .collect()
    };
    let best = (4..size / 2)
        .map(|r| (ring(r as f32).iter().filter(|b| **b).count(), r))
        .max()
        .expect("a band to sample")
        .1;
    ring(best as f32)
}

/// Width in degrees of the unlit run containing `at_degrees`, or 0 if that
/// direction is lit.
fn gap_at(profile: &[bool], at_degrees: f32) -> f32 {
    let step = 360.0 / PROFILE_STEPS as f32;
    let start = (at_degrees / step).round() as usize % PROFILE_STEPS;
    if profile[start] {
        return 0.0;
    }
    let mut run = 1;
    let mut k = 1;
    while k < PROFILE_STEPS && !profile[(start + k) % PROFILE_STEPS] {
        run += 1;
        k += 1;
    }
    let mut k = 1;
    while k < PROFILE_STEPS && !profile[(start + PROFILE_STEPS - k) % PROFILE_STEPS] {
        run += 1;
        k += 1;
    }
    run as f32 * step
}

/// Every unlit run around the profile, in degrees. On a closed ring of
/// indicators the only unlit stretches are the Gap setting's slits, one per
/// boundary between neighbours — so counting these counts the indicators,
/// and a missing one shows as two slits merged into a wider hole.
fn unlit_runs(profile: &[bool]) -> Vec<f32> {
    let step = 360.0 / PROFILE_STEPS as f32;
    // Start from a lit sample so the walk cannot begin mid-run and count one
    // run as two.
    let from = profile.iter().position(|b| *b).expect("something lit to measure from");
    let mut runs = Vec::new();
    let mut run = 0;
    for k in 0..PROFILE_STEPS {
        if profile[(from + k) % PROFILE_STEPS] {
            if run > 0 {
                runs.push(run as f32 * step);
                run = 0;
            }
        } else {
            run += 1;
        }
    }
    if run > 0 {
        runs.push(run as f32 * step);
    }
    runs
}

/// The invariant the wheel is built around, checked on the picture rather
/// than on the layout that feeds it: every octave of the span gets an
/// indicator and together they close the ring — whatever the counts, the
/// center, the fringe or the node's pitch class. So the only unlit stretches
/// are the Gap setting's slits, one per boundary, and the seam is one of them
/// on every node, wherever that node's turn has carried it.
///
/// Reading it off rendered pixels is the point. The layout's own tests pin
/// the angles down; this one says the shader draws the axis the table
/// describes, in the right direction, anchored where it claims — and in
/// particular that it draws the end indicators out to the seam, which is what
/// closes the ring when the window is not a whole number of octaves.
#[test]
fn every_octave_in_the_range_is_drawn_and_they_close_the_ring() {
    use harmonigraph_scene::octave_layout;

    const SIZE: [u32; 2] = [512, 512];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    // The widest wheel, the default, an even count — where the ring reaches an
    // octave further on one side — a center that is neither a C nor near the
    // middle of the keyboard, where the ring names octaves the packing has no
    // room for, and three fringed wheels: the narrowest there is (where the
    // one full-size octave takes a sector widest and the union branch of the
    // wedge test is stressed hardest), a plain register with a pair either
    // side, and a deep fringe filling the budget. Each at a C node (whose
    // octaves land flush on the center) and at three pitch classes that do
    // not, one of them the tritone that turns furthest.
    //
    // An even wheel, a flat fringe, a graded one, and then a fringe thin
    // enough to be eaten by the Gap.
    const FRINGES: [(f32, f32); 4] = [(1.0, 0.0), (0.6, 0.0), (0.6, 1.0), (0.15, 0.0)];
    for (count, extras, center) in [
        (11u32, 0u32, 60.0f32),
        (5, 0, 60.0),
        (4, 0, 60.0),
        (5, 0, 103.0),
        (1, 1, 60.0),
        (5, 2, 60.0),
        (3, 4, 60.0),
    ] {
        for (i, &(size, blend)) in FRINGES.iter().enumerate() {
            // Both fringe settings are inert without extras, so the other
            // three would render the same picture at four times the cost.
            if extras == 0 && i > 0 {
                continue;
            }
            for cents in [0.0, 350.0, 600.0, 1150.0] {
                let layout = octave_layout(count, center, extras, size, blend);
                let px = gpu.shot(&octave_wheel_scene(layout, cents));
                let profile = band_profile(&px, SIZE[0]);
                let case = format!(
                    "{count}+2x{extras} at {center}, size {size} blend {blend}, {cents}c"
                );

                // One indicator per octave of the wheel, closing the ring:
                // that is one slit per boundary and no other break. A missing
                // indicator merges two slits into one hole, so the count is
                // what says all of them are there — including the ones drawn
                // for octaves no note can reach.
                let want = layout.span as usize;
                let runs = unlit_runs(&profile);
                // Except under a thin fringe, and that is the settings talking
                // rather than a missing indicator: the Gap is cut out of every
                // sector from both sides at full width, so an extra thinner
                // than twice that padding has its two slits meet and reads as
                // no indicator at all. At 0.6 of an even slice they still
                // resolve; 0.15 is where they go, and only the extras are ever
                // that thin. `octaves.rs` pins the count exactly, on angles,
                // where no padding is involved.
                if size >= 0.4 {
                    assert_eq!(runs.len(), want, "{case}: unlit runs {runs:?} for {want} sectors");
                } else {
                    let lost = 2 * extras as usize;
                    assert!(
                        runs.len() + lost >= want && runs.len() <= want,
                        "{case}: unlit runs {runs:?} for {want} sectors — at most the \
                         {lost} extras can be lost to the Gap"
                    );
                }

                // The seam TURNS with the node: it is the bottom only for the
                // center's own pitch class, and every other class carries it
                // round by however far its octaves sit from the center. Read
                // off the layout rather than at a fixed 270 degrees, which is
                // the whole difference between this wheel and a window.
                let seam = layout.ring(cents).seam.to_degrees().rem_euclid(360.0);
                assert!(gap_at(&profile, seam) > 0.0, "{case}: no seam at {seam:.1} deg");

                // The CENTER pitch is straight up on every node, so a slice
                // covers the top — except on the node exactly a tritone from
                // it, where the center is a boundary and a slit there is the
                // axis being read rather than a hole.
                let tritone = (cents - 600.0).abs() < 1e-3;
                if !tritone {
                    assert!(gap_at(&profile, 90.0) == 0.0, "{case}: nothing covers the top");
                }
            }
        }
    }
}

/// Which PITCH each indicator is drawn at — the whole of what "positioned by
/// absolute pitch" means, and the part a seam test cannot see. One octave
/// sounds; the bright arc has to land where the layout puts that pitch, and
/// a node's pitch class has to move it.
#[test]
fn an_indicator_is_drawn_at_its_own_pitchs_angle() {
    use harmonigraph_scene::octave_layout;

    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [512, 512];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vec_size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };

    let mut pane = 100;
    for (count, extras, center, size, blend) in
        [(5u32, 0u32, 60.0f32, 1.0, 0.0), (8, 1, 66.0, 0.3, 0.0)]
    {
        let layout = octave_layout(count, center, extras, size, blend);
        // A C node and a node a fifth up: same slot, pitches 7 semitones
        // apart, so the bright arc must move by exactly that much of the axis.
        // The octave holding the center pitch, and one further round the
        // wheel, where a wrong anchor or a wrong direction shows.
        //
        // Both held INSIDE the ring rather than at its edges: a thin fringe
        // leaves the extras narrower than the Gap's slits, and a centroid
        // needs an arc to measure. That the edges reach the seam at all is
        // `every_octave_in_the_range_is_drawn_and_they_close_the_ring`.
        for (cents, offset) in [(0.0f32, 0i32), (700.0, 0), (0.0, 2), (700.0, 2)] {
            let (first, last) = layout.slots(cents);
            let slot =
                (harmonigraph_scene::MIDDLE_C_SLOT as i32 + offset).clamp(first + 1, last - 1);
            let mut scene = octave_wheel_scene(layout, cents);
            // One octave sounding. The silent slots still ghost in behind it
            // at GHOST_LEVEL, which the brightness threshold below sorts out.
            scene.nodes[0].octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
            scene.nodes[0].octaves[slot as usize] = 1.0;

            let cb = LatticeCallback::from_scene(
                &scene,
                LatticeLabels::default(),
                vec_size,
                format,
                pane,
                None,
            );
            pane += 1;
            let mut encoder = device.create_command_encoder(&Default::default());
            let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
            queue.submit(bufs.into_iter().chain([encoder.finish()]));
            let tex =
                render_to_texture(&device, &queue, SIZE, format, wgpu::Color::BLACK, |pass| {
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
            let px = readback(&device, &queue, &tex, SIZE);

            // Where the BRIGHT pixels are, as a mean direction. The lit
            // indicator runs several times the ghosts' level, so half the
            // maximum separates them cleanly whatever the node color is.
            let bright = |i: usize| px[i] as u32 + px[i + 1] as u32 + px[i + 2] as u32;
            let peak = (0..px.len() / 4).map(|k| bright(k * 4)).max().unwrap_or(0);
            assert!(peak > 60, "{count}+2x{extras} at {center}: nothing bright enough");
            let (mut vx, mut vy) = (0f64, 0f64);
            let c = SIZE[0] as f64 / 2.0;
            for y in 0..SIZE[1] {
                for x in 0..SIZE[0] {
                    let i = ((y * SIZE[0] + x) * 4) as usize;
                    if bright(i) > peak / 2 {
                        // Screen y grows downward; flip it for an ordinary angle.
                        vx += x as f64 - c;
                        vy += c - y as f64;
                    }
                }
            }
            let drawn = vy.atan2(vx).to_degrees() as f32;
            // The indicator's own middle, from the layout: the pitch halfway
            // between its two edges in ANGLE, which a fringe can shift off the
            // pitch itself.
            let (e0, e1) = layout.sector(slot, cents);
            let expected = (0.5 * (e0 + e1)).to_degrees().rem_euclid(360.0);
            let off = (drawn.rem_euclid(360.0) - expected).rem_euclid(360.0);
            let off = off.min(360.0 - off);
            assert!(
                off < 6.0,
                "{count}+2x{extras} at {center}, {cents}c, slot {slot}: indicator drawn \
                 at {drawn:.1} deg, the axis puts its pitch at {expected:.1}"
            );
        }
    }
}

/// How a release ENDS: the fading indicator arrives at the grey its silent
/// neighbours are drawn in, and arrives there continuously.
///
/// It only shows where the node's PRESENCE outlives this slot's level, which
/// is another instance of the pitch class still held. A lone note drives the
/// backdrop and the lit glyph off one envelope, so both land at nothing
/// together and any discontinuity between them has no coverage left to show
/// in; here the held octave pins the backdrop at full strength while the
/// released one runs out against it.
///
/// A slot painted in PLACE of its ghost — opacity by a max(), color by a
/// `level > 0` switch — fails the first two checks below, and which one goes
/// first is worth knowing. The ghost is the WHITENED node color, so the final
/// frame's switch is a step up in light and the never-brightens loop is what
/// actually fires; the tail-spread check catches the same fault, and is the
/// one that would still hold if a ghost ever came out darker than the pitch it
/// takes over from. The last check is neither: at level 0 both shaders run the
/// same line, so it can only say the finished ring is one backdrop, not how it
/// got there.
#[test]
fn a_released_octave_lands_on_its_ghost_without_a_step() {
    use harmonigraph_scene::octave_layout;

    const SIZE: [u32; 2] = [384, 384];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // An even five-octave wheel on a C node: a slice is 72 degrees, which is
    // room to sample well inside one and well inside its neighbour.
    let layout = octave_layout(5, 60.0, 0, 1.0, 0.0);
    let held = harmonigraph_scene::MIDDLE_C_SLOT;
    let (releasing, silent) = (held + 1, held + 2);
    // All three inside the ring this wheel draws. `sector` CLAMPS a slot
    // outside it rather than refusing, so a wheel that stopped reaching them
    // would leave the neighbour reading below comparing one slice against
    // itself — passing for a reason that has nothing to do with the fade.
    let (low, high) = layout.slots(0.0);
    for slot in [held, releasing, silent] {
        assert!((low..=high).contains(&(slot as i32)), "slot {slot} is outside {low}..={high}");
    }
    let scene = |level: f32| {
        let mut scene = octave_wheel_scene(layout, 0.0);
        let node = &mut scene.nodes[0];
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.octaves[held] = 1.0;
        node.octaves[releasing] = level;
        // The instance still down. This is what holds the whole backdrop up
        // while the other one fades, and the reason the handoff is legible.
        node.activation = 1.0;
        scene
    };

    let (mid, wedge) = wedge_of(layout, releasing, 0.0);
    // Down the envelope past the ghost's own level, to the smallest the 8-bit
    // packing carries, and then off it. The first of these is also the shot
    // the radii are calibrated from, taken once and read twice so the two can
    // never drift onto different pictures.
    const TAIL: [f32; 9] = [1.0, 0.5, 0.25, 0.16, 0.12, 0.08, 0.04, 0.02, 1.0 / 255.0];
    let full = gpu.shot(&scene(TAIL[0]));
    let probe = BandProbe::new(&full, SIZE, mid);

    let mut steps: Vec<[f32; 3]> = vec![probe.mean(&full, mid, wedge)];
    steps.extend(TAIL[1..].iter().map(|&level| probe.mean(&gpu.shot(&scene(level)), mid, wedge)));
    let ended = gpu.shot(&scene(0.0));
    steps.push(probe.mean(&ended, mid, wedge));

    let light = |c: &[f32; 3]| c[0] + c[1] + c[2];
    let apart = |a: &[f32; 3], b: &[f32; 3]| {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    // The envelope only ever takes light out of the slice.
    for pair in steps.windows(2) {
        assert!(
            light(&pair[1]) <= light(&pair[0]) + 0.5,
            "the fade brightens at {:?} then {:?}",
            pair[0],
            pair[1]
        );
    }
    // And the last stretch of it — from the ghost's own level down to
    // nothing, which is where an opacity that floors and a color that
    // switches part company — is SPREAD across the frames rather than spent
    // in one. Painted in place of its ghost instead, the slice sits still for
    // that whole stretch and then makes the entire journey in one frame.
    //
    // A share of the travel rather than a run of strict decreases: a ramp
    // this shallow moves the last few frames by less than an 8-bit channel,
    // and the pair either side of zero reads identically here BECAUSE the
    // handoff is smooth. The cut is GHOST_LEVEL in lattice.wgsl; a stale
    // value only widens or narrows the stretch measured, so this reads the
    // sweep rather than asserting the constant.
    let tail = &steps[TAIL.iter().position(|&level| level <= 0.16).expect("a tail to measure")..];
    let travel = apart(&tail[0], &tail[tail.len() - 1]);
    assert!(travel > 10.0, "the tail hardly moves at all ({travel:.1}), so its shape says little");
    for pair in tail.windows(2) {
        let step = apart(&pair[0], &pair[1]);
        assert!(
            step < 0.4 * travel,
            "the indicator spends {step:.1} of its {travel:.1} tail in one step, at {:?}",
            pair[1]
        );
    }
    // Landing on the ghost the silent slices are drawn in — the same grey at
    // the same coverage, so the finished ring is one backdrop rather than a
    // backdrop with one slice a shade off it.
    let (quiet, quiet_wedge) = wedge_of(layout, silent, 0.0);
    let neighbour = probe.mean(&ended, quiet, quiet_wedge);
    assert!(
        apart(&steps[steps.len() - 1], &neighbour) < 3.0,
        "a spent indicator reads {:?} against its neighbours' {neighbour:?}",
        steps[steps.len() - 1]
    );
}

/// The OTHER release: a node going out together with its octave, which is a
/// lone note let go, or the last instance of one. Here `level` and `presence`
/// are the same envelope and there is no backdrop to hand off to — it is
/// leaving too — so the indicator keeps its own pitch the whole way down and
/// its opacity runs to nothing in a STRAIGHT line.
///
/// That is the half a ghost scaled by `1 - level` gets wrong. It is the same
/// arithmetic wherever presence is 1, so the handoff above cannot see it, but
/// with the two on one envelope it counts the note's presence twice: the
/// opacity bulges to `1.16e - 0.16e²`, four points over the line at the middle
/// of the fade, and the slice picks up a whitening from a backdrop that is not
/// there. Taking the ghost as what is LEFT of the presence after this slot's
/// own level is what makes both releases straight.
#[test]
fn a_lone_notes_octave_fades_in_a_straight_line() {
    use harmonigraph_scene::octave_layout;

    const SIZE: [u32; 2] = [384, 384];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let layout = octave_layout(5, 60.0, 0, 1.0, 0.0);
    let slot = harmonigraph_scene::MIDDLE_C_SLOT;
    let scene = |envelope: f32| {
        let mut scene = octave_wheel_scene(layout, 0.0);
        let node = &mut scene.nodes[0];
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.octaves[slot] = envelope;
        // ONE envelope for both: nothing else on this node is sounding, so
        // its presence is this octave's own.
        node.activation = envelope;
        scene
    };

    let (mid, wedge) = wedge_of(layout, slot, 0.0);
    let full = gpu.shot(&scene(1.0));
    let probe = BandProbe::new(&full, SIZE, mid);
    let lit = probe.mean(&full, mid, wedge);

    // Proportional, channel by channel: the wedge is nothing but this glyph
    // (no core and no rings, and an idle node paints nothing at all), so a
    // straight-line fade is the reading at `e` being `e` of the reading at
    // full. The tolerance is the 8-bit packing of the level plus the target's
    // own rounding, well under the 5-to-9 the bulge would add here.
    for envelope in [0.75f32, 0.5, 0.25] {
        let got = probe.mean(&gpu.shot(&scene(envelope)), mid, wedge);
        for j in 0..3 {
            let want = envelope * lit[j];
            assert!(
                (got[j] - want).abs() < 2.5,
                "at {envelope} of the envelope the slice reads {got:?}, not {want:.1} in \
                 channel {j} — {lit:?} at full"
            );
        }
    }
    // And it ends at nothing rather than on a ghost: with the node gone there
    // is no backdrop left for the indicator to sit in.
    let spent = probe.mean(&gpu.shot(&scene(0.0)), mid, wedge);
    assert!(spent.iter().sum::<f32>() < 3.0, "a spent lone note leaves {spent:?} behind");
}

/// The seams between a chord's colors run at ONE width from the rim to the
/// centre. They are laid down as lobes of fixed ANGULAR width, so the arc each
/// spans shrinks with the radius and they would otherwise converge to a cusp at
/// the node's centre — sharpest exactly where the node has the fewest pixels to
/// say it with.
///
/// Both halves of the bargain, because either alone has a trivial cheat: the
/// centre has to lose its seam, AND the rim has to keep its colors, which is
/// what stops the cure from being "average the whole node".
///
/// Taken at both ends of the solidity axis and at the shipped default, because
/// the claim of this shape is that the width does NOT depend on solidity — the
/// cusp belongs to the kernel, and the glow skirt that carries the same blend
/// has no solidity of its own. So the three readings have to AGREE, not merely
/// each pass.
///
/// Measured as how far the colors around a ring point APART as directions, not
/// as how much they differ: a soft core is also a dimmer one, and any measure
/// of magnitude would read that dimming as a blur and pass on it.
#[test]
fn the_color_seams_run_at_one_width_from_the_rim_to_the_centre() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [512, 512];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vec_size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };

    let mut pane = 300;
    let mut shot = |solidity: f32, core_radius: f32| -> Vec<u8> {
        let mut scene = single_marked_node(0, 0);
        scene.core_solidity = solidity;
        // The disc held at one size on screen whatever fraction of the quad it
        // is, so both cases below are measured at the same pixel scale. What
        // the seam floor asks for is a fraction of the core radius, so the
        // profile it produces is the same shape at any size; keeping the disc
        // big just leaves the `aa` floor — the one absolute term — too small to
        // be what these readings are about.
        scene.node_radius *= 0.46 / core_radius;
        scene.core_radius = core_radius;
        // Every octave the wheel draws for this pitch class. A single sounding
        // voice takes the node's own color everywhere (octave_glow_color's solo
        // fallback), which leaves no seam to measure at all.
        let layout = scene.octave_layout;
        let node = &mut scene.nodes[0];
        let (low, high) = layout.slots(node.cents);
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        for slot in low.max(0)..=high.min(harmonigraph_scene::OCTAVE_SLOTS as i32 - 1) {
            node.octaves[slot as usize] = 1.0;
        }
        let labels = LatticeLabels::default();
        let cb = LatticeCallback::from_scene(&scene, labels, vec_size, format, pane, None);
        pane += 1;
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let tex = render_to_texture(&device, &queue, SIZE, format, wgpu::Color::BLACK, |pass| {
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
        readback(&device, &queue, &tex, SIZE)
    };

    // The node is alone at the world origin and the camera looks at it, so the
    // frame's centre is its centre.
    let c = (SIZE[0] / 2) as i32;
    let rgb = |px: &[u8], x: i32, y: i32| -> glam::Vec3 {
        let i = ((y as u32 * SIZE[0] + x as u32) * 4) as usize;
        glam::Vec3::new(px[i] as f32, px[i + 1] as f32, px[i + 2] as f32) / 255.0
    };
    // How far apart, in degrees, the most divergent pair of colors around a
    // ring of radius `r` point. Zero is one flat color all the way round.
    let spread = |px: &[u8], r: f32| -> f32 {
        let dirs: Vec<glam::Vec3> = (0..64)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / 64.0;
                // Screen y grows downward; the sample angle is negated.
                rgb(px, c + (r * a.cos()).round() as i32, c - (r * a.sin()).round() as i32)
            })
            .filter(|v| v.length() > 0.02)
            .map(|v| v.normalize())
            .collect();
        let lit = dirs.len();
        assert!(lit > 56, "the ring at r={r:.0} is not on the node: {lit} lit samples of 64");
        let mut worst = 0.0f32;
        for (i, a) in dirs.iter().enumerate() {
            for b in &dirs[i + 1..] {
                worst = worst.max(a.dot(*b).clamp(-1.0, 1.0).acos().to_degrees());
            }
        }
        worst
    };

    // The disc's radius in pixels, read off the SOLID end, where its edge is
    // the first sharp fall from the centre; at the soft end it has dissolved
    // into the glow and there is no edge to find. It is the same disc either
    // way, so one reading sizes every ring. Taken as the median of eight rays
    // rather than one: a single ray's answer depends on which octave's hue
    // happens to lie along it and how bright that one is, so a fixture whose
    // colors moved would quietly resize the rings instead of failing.
    let disc_radius = |px: &[u8]| -> f32 {
        let ray = |a: f32| -> f32 {
            let at = |r: f32| rgb(px, c + (r * a.cos()) as i32, c - (r * a.sin()) as i32).length();
            let centre = at(0.0);
            (2..c).map(|r| r as f32).find(|r| at(*r) < centre * 0.4).unwrap_or(c as f32)
        };
        let mut rs: Vec<f32> = (0..8)
            .map(|i| ray(std::f32::consts::TAU * i as f32 / 8.0))
            .collect();
        rs.sort_by(f32::total_cmp);
        rs[4]
    };

    for (name, core_radius) in [("the default core", 0.2f32), ("the classic radius", 0.46)] {
        // The disc's edge is a matter of COVERAGE, which none of this touches,
        // so the solid end still has one to find whatever the colors inside it
        // are doing.
        let r_disc = disc_radius(&shot(1.0, core_radius));
        assert!(r_disc > 20.0, "{name}: the disc is too small to sample rings in ({r_disc} px)");
        let (inner, outer) = (r_disc * 0.2, r_disc * 0.75);

        let mut centres = Vec::new();
        for solidity in [1.0f32, 0.4, 0.25] {
            let px = shot(solidity, core_radius);
            let at_centre = spread(&px, inner);
            let at_rim = spread(&px, outer);
            // No cusp: the middle is a blend rather than the point where every
            // seam meets. This is what fails if the lobes go back to one fixed
            // concentration — the centre then reads as separated as the rim.
            assert!(
                at_centre < at_rim * 0.5,
                "{name} at solidity {solidity}: the seams still converge — {at_centre:.0} deg \
                 across the centre against {at_rim:.0} at the rim"
            );
            // And what stops the cure being "average the node": the seams are
            // never held wider than the arc they already span where the disc
            // ends, so the node still shows its notes as distinct colors.
            assert!(
                at_rim > 30.0,
                "{name} at solidity {solidity}: the colors washed out instead of \
                 their seams widening — only {at_rim:.0} deg across the rim"
            );
            centres.push(at_centre);
        }
        // The point of this shape over hanging the cure off the solidity axis:
        // the seam width is the same whatever solidity is dialed. A reading
        // that tracked the axis would spread these three apart.
        let lo = centres.iter().cloned().fold(f32::MAX, f32::min);
        let hi = centres.iter().cloned().fold(0.0f32, f32::max);
        assert!(
            hi - lo < 6.0,
            "{name}: the seam width moved with solidity — the centre reads {lo:.0}..{hi:.0} deg \
             across the axis, and this shape is meant to be independent of it"
        );
    }
}

#[test]
fn bloom_adds_light_over_the_plain_composite() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [256, 256];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let vec_size = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, vec_size);
    let clear = wgpu::Color::BLACK;

    let scene_off = parity_scene();
    let mut scene_on = parity_scene();
    scene_on.bloom_strength = 1.0;

    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor {
        size_in_pixels: SIZE,
        pixels_per_point: 1.0,
    };
    let mut total = |scene: &Scene, pane_id: u64| -> u64 {
        let cb = LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            vec_size,
            format,
            pane_id,
            None,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let tex = render_to_texture(&device, &queue, SIZE, format, clear, |pass| {
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
        readback(&device, &queue, &tex, SIZE)
            .chunks(4)
            .map(|px| u64::from(px[0]) + u64::from(px[1]) + u64::from(px[2]))
            .sum()
    };

    let plain = total(&scene_off, 31);
    let bloomed = total(&scene_on, 32);
    assert!(
        bloomed > plain + plain / 20,
        "bloom at strength 1.0 should add clearly visible light: \
         plain {plain} vs bloomed {bloomed}"
    );
}

/// A sheet in FRONT of the home sheet is drawn over it; a sheet BEHIND it
/// is drawn under. Both directions matter, and only one of them is obvious:
/// forcing the home sheet to the bottom (so an off-sheet note could never be
/// hidden by it) inverts the far half of the axis, and since the knockout
/// gutter clears whatever was drawn before it, the sheet behind then takes a
/// bite out of the home sheet in front of it.
#[test]
fn sheets_draw_back_to_front_along_the_sevens_axis() {
    use harmonigraph_scene::{Camera, FrameParams, Projection, ViewConfig};

    let view = ViewConfig {
        extent_threes: 1,
        extent_fives: 1,
        extent_sevens: 2,
        ..ViewConfig::default()
    };
    for projection in [Projection::Cabinet, Projection::Perspective, Projection::Orthographic]
    {
        let mut scene = harmonigraph_scene::derive_scene(
            &harmonigraph_core::NoteTracker::new(),
            &harmonigraph_core::Tuning::default(),
            &view,
            &view.reach(),
            &FrameParams::default(),
            // Orbited, deliberately: this is the case a plain depth sort
            // gets wrong, because two nodes on one sheet then sit at
            // different depths and the sheets interleave.
            Camera { projection, ..Camera::default() },
            None,
            0.0,
        );
        // Every position SOUNDING. Set here rather than played in, because
        // what this test needs is one node per position on every sheet, and
        // which nodes a tracker lights is a question about tuning.
        //
        // Sounding is the only way to get one: an idle node paints nothing at
        // all now, so the cull drops it, and a scene of idle nodes would leave
        // the order below comparing an empty list with itself.
        for node in &mut scene.nodes {
            node.activation = 1.0;
        }
        let call = LatticeCallback::from_scene(
            &scene,
            LatticeLabels::default(),
            egui::vec2(800.0, 600.0),
            wgpu::TextureFormat::Bgra8Unorm,
            0,
            // No stats slot: this is about draw ORDER, not about timing.
            None,
        );
        // World z IS the sevens axis (see lattice_to_world), so the draw
        // order must run from the most negative sheet to the most positive
        // — and it has to hold under EVERY projection, not only the face-on
        // one. When it doesn't, the sheets interleave, the grid lands in
        // the wrong place in the order, and the home sheet's clearings have
        // nothing drawn before them left to clear.
        let depths: Vec<f32> = call.instances.iter().map(|i| i.world_pos[2]).collect();
        // Several SHEETS, not several nodes. A node count passes on one
        // sheet's worth of identical depths, where every pair below holds
        // whatever the sort did — which is what culling the off-sheet nodes
        // reduced this to, silently, while it went on reading as coverage.
        let (lo, hi) = depths.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &d| {
            (lo.min(d), hi.max(d))
        });
        assert!(
            hi - lo > 1e-6,
            "{projection:?}: every node drawn is at one depth ({lo}), so the order \
             below compares a sheet with itself: {depths:?}",
        );
        for pair in depths.windows(2) {
            assert!(
                pair[1] >= pair[0] - 1e-6,
                "{projection:?}: a sheet behind is drawn after one in front: {pair:?}"
            );
        }
    }
}

/// Every node idle: no note, no marks, no octaves — the state most of a
/// lattice is in most of the time, and the state in which a node paints
/// NOTHING. There is no idle marker and no trail mark to draw one; what
/// says a position is there is the grid's gap around it, which belongs to
/// the edge pass. So every test below expects these nodes to be culled, and
/// the fixture exists to make "nothing to draw" easy to ask for.
///
/// The AUDIO RING has to be off for that to hold, and it is:
/// [`parity_scene`] carries a silent [`harmonigraph_scene::SpectralPaint`].
/// The ring is a window onto one shared spectrum rather than a level a node
/// carries, so with it on every node in the window paints one and there is no
/// such thing as an idle node to cull.
///
/// `on_home` and `trail` are still set, on different cycles, and neither is
/// read by anything in THIS crate: no `GpuInstance` carries them, and the
/// grid and the labels both arrive already built. They stay because a
/// fixture whose two sets coincide cannot tell a predicate that reads one
/// from a predicate that reads the other, and the cull is where such a
/// predicate would go — but nothing here separates them today, so treat the
/// cycles as room left rather than as coverage.
fn idle_scene() -> Scene {
    let mut scene = parity_scene();
    for (i, node) in scene.nodes.iter_mut().enumerate() {
        node.activation = 0.0;
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.melody_slots = 0;
        node.bass_slots = 0;
        node.melody_level = 0.0;
        node.bass_level = 0.0;
        node.hovered = false;
        node.on_home = i % 2 == 0;
        node.trail = if i % 3 == 0 { 0.8 } else { 0.0 };
    }
    scene
}

/// The fragment shader's early-outs — skipping the fragments outside
/// anything a node can paint, and the whole note path for an idle node —
/// must be exactly that: an optimization. Same scene through the real
/// shader and through one compiled with `EARLY_OUT` off, pixel for pixel.
///
/// Worth a GPU test rather than a reading of the code, because the bound in
/// `paint_reach` is a claim about EVERY layer's falloff at once: add a layer
/// that reaches further, or widen one's soft edge past its radius, and the
/// only symptom is a quietly clipped halo somewhere off the node.
#[test]
fn the_fragment_early_outs_do_not_change_a_pixel() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [256, 256];
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let reference_src = SHADER_SRC.replace(
        "const EARLY_OUT: bool = true;",
        "const EARLY_OUT: bool = false;",
    );
    assert_ne!(
        reference_src, SHADER_SRC,
        "the EARLY_OUT switch was renamed; this test is no longer comparing anything",
    );

    // A core dialed near the top of its bar, with the solidity axis in the
    // middle. Every other fixture here runs the default 0.46 radius at full
    // solidity, where `core_reach` is GLOW_LIMIT and the radius arm of it is
    // never the larger — so collapsing that bound to the constant passes.
    // At 0.9 the disc still carries alpha where the glow's window has closed,
    // and clipping it there is a hard circular cut across a soft core.
    let fat_core = || {
        let mut scene = parity_scene();
        scene.core_radius = 0.9;
        scene.core_solidity = 0.5;
        scene
    };
    // The sheet running, at one fixed instant. `paint_reach` is where the
    // claim that shimmer keeps every bound exact has to be checked, and it can
    // only be checked here: the sheet relights the rings and the marked
    // slice inside their own coverage — `shimmer_terms` never touches
    // coverage — and a sheet that widened what a layer paints would push it
    // past the reach the early-out proved it could not cross, visible as a
    // ring clipped flat in the fast pipeline alone, which no other fixture
    // would catch because every other one leaves the pulse Off.
    let shimmering = || {
        let mut scene = parity_scene();
        scene.pulse_marks = harmonigraph_scene::Pulse::Bands;
        scene
    };
    // The audio ring on, which has two early-outs of its own: the annulus
    // skip inside `spectral_ring`, and the idle branch's radial exception,
    // which keeps an otherwise idle node's fragments only where the ring is.
    // Both are answers about a layer NO other fixture here draws — the ring is
    // off in `parity_scene` — so without this the two switches would be
    // compiled and never compared.
    let ringing = || {
        let mut scene = parity_scene();
        let paint = &mut scene.spectral;
        paint.inner = 0.20;
        paint.outer = 0.38;
        paint.range = 300.0;
        paint.lut = std::array::from_fn(|k| {
            let t = k as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
            glam::Vec4::new(t, 0.6 * t, 1.0 - t, 1.0)
        });
        // A comb across the grid rather than a flat level, so a wedge carries
        // an EDGE: a fragment either side of one reads a different bucket, and
        // an early-out that shifted the sampled pitch by so much as a bucket
        // shows up as a moved edge rather than being averaged away.
        for (bucket, level) in paint.levels.iter_mut().enumerate() {
            *level = if (bucket / 7) % 3 == 0 { 220 } else { 20 };
        }
        // ...and a node with a ring and nothing else, which is the idle
        // branch's new case: no activation, no marks, and an annulus to draw.
        let mut silent = scene.nodes[0];
        silent.activation = 0.0;
        silent.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        silent.melody_slots = 0;
        silent.bass_slots = 0;
        silent.melody_level = 0.0;
        silent.bass_level = 0.0;
        silent.world_pos.x += 0.9;
        scene.nodes.push(silent);
        scene
    };
    // The ring's OTHER reading, which is the shader's second branch inside
    // `spectral_ring`: every fragment of a wedge reads the octave's own pitch
    // instead of walking a window across it, so the whole `wedge_fraction`
    // path is skipped and a wedge comes out flat.
    let folded = || {
        let mut scene = ringing();
        scene.spectral.folded = true;
        scene
    };
    // No all-idle fixture: an idle node paints nothing, so the cull ships
    // none of them and the comparison would be two empty images. What the
    // idle branch does is now pinned by
    // `a_silent_lattice_ships_no_nodes_and_still_draws_its_grid` instead,
    // on the CPU side where the decision actually lives.
    for (name, scene) in [
        ("lit", parity_scene()),
        ("fat core", fat_core()),
        ("shimmering", shimmering()),
        ("ringing", ringing()),
        ("folded", folded()),
    ] {
        let cb = LatticeCallback::from_scene(
            &scene,
            LatticeLabels::default(),
            egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
            format,
            11,
            None,
        );
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));

        let res: &LatticeResources = resources.get().expect("prepare created resources");
        let (fast, _) = create_pipelines(&device, SHADER_SRC, format, &res.bind_group_layout, false);
        let (slow, _) =
            create_pipelines(&device, &reference_src, format, &res.bind_group_layout, false);
        let pane = res.panes.get(&11).expect("prepare created the pane");

        let clear = wgpu::Color { r: 0.07, g: 0.08, b: 0.09, a: 1.0 };
        let draw = |pipeline: &wgpu::RenderPipeline| {
            let texture = render_to_texture(&device, &queue, SIZE, format, clear, |pass| {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                pass.draw(0..4, 0..pane.instance_count);
            });
            readback(&device, &queue, &texture, SIZE)
        };
        let with_early_out = draw(&fast);
        let without = draw(&slow);

        // Guard against a vacuous pass: the nodes must have drawn something
        // over the clear color.
        let bg = [18u8, 20, 23, 255];
        assert!(
            without.chunks(4).any(|px| px.iter().zip(bg).any(|(&c, b)| c.abs_diff(b) > 8)),
            "the {name} scene drew nothing; the comparison is vacuous",
        );

        let differing = with_early_out
            .iter()
            .zip(&without)
            .enumerate()
            .find(|(_, (&a, &b))| a != b);
        assert!(
            differing.is_none(),
            "the {name} scene changed when the early-outs were enabled: byte {:?}",
            differing.map(|(i, (a, b))| (i, *a, *b)),
        );
    }
}

/// A node that can paint nothing is not shipped at all — and the grid it
/// sits in still is.
///
/// The billboard is deliberately bigger than the node, so a node the shader
/// discards every fragment of still costs a quad's worth of rasterizing; on
/// an unplayed lattice that is EVERY node, there being no idle marker and no
/// trail mark for one to draw. So the frame drops to a grid and nothing
/// else, and the callback has to keep drawing that grid — which is why
/// neither `prepare` nor `paint` may read "no instances" as "nothing to
/// draw": that test takes the grid down with the nodes.
#[test]
fn a_silent_lattice_ships_no_nodes_and_still_draws_its_grid() {
    let scene = idle_scene();
    assert!(!scene.grid.is_empty(), "the fixture has to carry a grid");
    let cb = LatticeCallback::from_scene(
        &scene,
        LatticeLabels::default(),
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        31,
        None,
    );
    assert!(
        cb.instances.is_empty(),
        "every node is idle with nothing to draw at one, so none should ship",
    );
    assert!(!cb.edges.is_empty(), "the grid is not a node and must survive");

    let Some((device, queue)) = headless_device() else {
        return;
    };
    const SIZE: [u32; 2] = [256, 256];
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    let mut encoder = device.create_command_encoder(&Default::default());
    let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
    queue.submit(bufs.into_iter().chain([encoder.finish()]));
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(256.0, 256.0));
    let clear = wgpu::Color { r: 0.07, g: 0.08, b: 0.09, a: 1.0 };
    let texture = render_to_texture(&device, &queue, SIZE, format_of(&cb), clear, |pass| {
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
    let px = readback(&device, &queue, &texture, SIZE);
    let bg = [18u8, 20, 23, 255];
    assert!(
        px.chunks(4).any(|p| p.iter().zip(bg).any(|(&c, b)| c.abs_diff(b) > 4)),
        "the grid vanished with the nodes",
    );
}

/// The ring's floor colour is a picture on every node, so the ring being on
/// is itself a reason to ship an instance: the cull that drops idle nodes
/// keeps all of them the moment the annulus is real. Nothing else reaches
/// that term — the pixel tests above light their nodes, which passes the
/// cull on activation instead.
#[test]
fn an_open_ring_ships_every_idle_node() {
    let fresh = harmonigraph_scene::ViewConfig::default();
    let mut scene = idle_scene();
    assert!(!scene.nodes.is_empty(), "the fixture has to carry idle nodes");
    (scene.spectral.inner, scene.spectral.outer) = fresh.rings().audio;
    let cb = LatticeCallback::from_scene(
        &scene,
        LatticeLabels::default(),
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        32,
        None,
    );
    assert_eq!(
        cb.instances.len(),
        scene.nodes.len(),
        "with the ring on, a node with nothing else to draw still wears the floor colour",
    );
}

/// A ring wedge takes its colour from the analyzer's ramp at ITS OWN level —
/// not from the node's pitch colour scaled by that level, which is the octave
/// band's MIDI logic and the exact scheme confusion the two tables exist to
/// prevent. The ramp here switches hue at half: a level on one side paints
/// blue, on the other red, and no colour-times-level path can flip a hue, so
/// the flip is the shader indexing the ramp.
///
/// Read on the FOLDED branch, where a wedge is one flat level: the raw one
/// spreads a window of the grid across the wedge, and a shot of it would be
/// two hues at once by design.
#[test]
fn a_ring_wedge_wears_its_own_levels_ramp_entry() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    let shot_at = |gpu: &mut Shooter, level: u8| -> Vec<u8> {
        let fresh = harmonigraph_scene::ViewConfig::default();
        let mut scene = single_marked_node(0, 0);
        let node = &mut scene.nodes[0];
        // Nothing held, so what is on screen is the ring alone: the band's own
        // wedges are the MIDI picture and would sum into the channels below.
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.activation = 1.0;
        // The core off for the same reason ringing_node turns it off: its
        // glow is light at every radius, and the reading below is the ring's.
        scene.core_radius = 0.0;
        let rings = fresh.rings();
        scene.outer_inner = rings.band.0;
        scene.outer_outer = rings.band.1;
        scene.rings_outer = rings.outer;
        scene.ring_gap = rings.gap;
        let mut paint = harmonigraph_scene::SpectralPaint::silent();
        (paint.inner, paint.outer) = rings.audio;
        paint.folded = true;
        // The whole grid at one level, so every wedge whose octave the
        // analyzer's axis reaches reads the same entry. Off the axis
        // `spectrum_at` answers 0 whatever the grid holds, which this wheel
        // stays clear of.
        paint.levels = Box::new([level; harmonigraph_scene::SPECTRAL_BUCKETS]);
        paint.lut = std::array::from_fn(|k| {
            let t = k as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
            if t < 0.5 {
                glam::Vec4::new(0.0, 0.0, 1.0, 1.0)
            } else {
                glam::Vec4::new(1.0, 0.0, 0.0, 1.0)
            }
        });
        scene.spectral = paint;
        gpu.shot(&scene)
    };

    let low = shot_at(&mut gpu, 90);
    let high = shot_at(&mut gpu, 217);
    let sum = |px: &[u8], ch: usize| -> i64 { px.chunks(4).map(|p| p[ch] as i64).sum() };
    let (blue_low, blue_high) = (sum(&low, 2), sum(&high, 2));
    let (red_low, red_high) = (sum(&low, 0), sum(&high, 0));
    eprintln!("low: red {red_low} blue {blue_low}; high: red {red_high} blue {blue_high}");
    // The margin is a wedge's worth of one channel against antialiasing
    // fringes; the ring itself sums in the tens of thousands.
    const HUE_FLIP: i64 = 5_000;
    assert!(
        blue_low > blue_high + HUE_FLIP && red_high > red_low + HUE_FLIP,
        "crossing the ramp's half did not flip the wedge's hue: the ring is not \
         indexing the analyzer's ramp at the wedge's own level",
    );
}

/// The target format the callback was built for, so the test above renders
/// into the one its composite pipeline expects.
fn format_of(cb: &LatticeCallback) -> wgpu::TextureFormat {
    cb.target_format
}

/// One way of making a node sound, for the table in the test below.
type LightUp = fn(&mut harmonigraph_scene::NodeInstance);

/// Each of the four things that make a node sounding keeps it on its own.
///
/// The cull's first question is whether anything is lit — an envelope, a
/// melody or bass ring's level, or a lit octave — and it is a disjunction
/// over four terms that ordinarily move together: `derive_scene` rides all
/// of them on one envelope, so a scene built by playing notes has either
/// none of them or several, and no single term ever decides whether a node
/// ships. That is not a hypothetical gap. Inverting any one of the four,
/// or turning any `||` between them into `&&`, passed the whole suite.
///
/// They come apart in practice, which is why each gets a node here. An
/// octave's level is `envelope * attack(...)` and is packed to a byte, so
/// for the first frames after a note-on a node has a full activation and an
/// octave word of exactly zero — `params[0]` alone is holding it in the
/// buffer, and a cull that stopped reading it would drop the first frame of
/// a note. The mark levels ride their own ease-in (`melody_attack`) rather
/// than the node's, so they part company the same way.
///
/// Built by hand rather than played in: the point is one term at a time,
/// and a tracker cannot be asked for that.
#[test]
fn each_thing_that_makes_a_node_sounding_keeps_it_alone() {
    let bare = || {
        let mut scene = idle_scene();
        // An idle node draws nothing whatever its memory, so the only reason
        // to keep one is the term under test.
        for node in &mut scene.nodes {
            node.trail = 0.0;
        }
        scene
    };
    let ships = |scene: &Scene| {
        LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            egui::vec2(256.0, 256.0),
            wgpu::TextureFormat::Rgba8Unorm,
            33,
            None,
        )
        .instances
        .len()
    };
    assert_eq!(ships(&bare()), 0, "the fixture has to start with nothing drawn");

    // One node per term, each the ONLY lit thing about it.
    let cases: [(&str, LightUp); 4] = [
        ("activation", |n| n.activation = 1.0),
        ("melody level", |n| n.melody_level = 1.0),
        ("bass level", |n| n.bass_level = 1.0),
        ("a lit octave", |n| n.octaves[harmonigraph_scene::MIDDLE_C_SLOT] = 1.0),
    ];
    for (what, set) in cases {
        let mut scene = bare();
        set(&mut scene.nodes[0]);
        assert_eq!(ships(&scene), 1, "{what} alone has to keep its node");
    }

    // The octave levels pack into three u32s, four slots to a word, and the
    // three are OR'd. Two octaves of one pitch class held at the same level
    // is an ordinary voicing, and it puts the SAME byte in two words — where
    // anything but an OR cancels them against each other and reads the node
    // as unlit. Every pairing of words is needed, because which two cancel
    // depends on where the operator lands and how it then binds: `^` binds
    // tighter than `|`, so swapping either one regroups the whole expression
    // as well as changing it.
    for (a, b) in [(0usize, 4usize), (0, 8), (4, 8)] {
        let mut spread = bare();
        spread.nodes[0].octaves[a] = 1.0;
        spread.nodes[0].octaves[b] = 1.0;
        assert_eq!(
            ships(&spread),
            1,
            "octaves {a} and {b} held at one level keep their node",
        );
    }
}

/// The grid's place in the draw order is counted over the nodes actually
/// shipped, not over the ones the scene held.
///
/// `grid_at` is the seam between the sheets BEHIND the home sheet and the
/// home sheet itself, and the whole argument for it is in `from_scene`: put
/// the grid under everything and a node on a sheet behind the home one
/// punches its clearing through the home grid. Culling breaks the old
/// expression silently, because `split` indexes the list before the cull —
/// with the sheets behind home mostly idle, `split` runs past the end of the
/// kept run, `prepare`'s `.min(instance_count)` pins the grid to the very
/// end, and it draws over every node instead of under the home sheet.
///
/// One lit node behind home and one on it, with an idle node behind home
/// between them: the seam has to land at 1, and it is `split` (2) that says
/// otherwise.
#[test]
fn the_grid_seam_counts_the_nodes_that_ship() {
    let mut scene = idle_scene();
    scene.nodes.truncate(3);
    for node in &mut scene.nodes {
        node.trail = 0.0;
        node.activation = 0.0;
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
    }
    // Which world z is "behind" is the camera's to say — the sort keys on
    // `world_pos.z * forward.z` — and the default camera's eye sits at +z, so
    // the sheets behind the home one are the negative side.
    scene.nodes[0].world_pos.z = -2.0;
    scene.nodes[0].activation = 1.0; // behind home, lit: ships, before the grid
    scene.nodes[1].world_pos.z = -1.0; // behind home, idle: culled
    scene.nodes[2].world_pos.z = 0.0;
    scene.nodes[2].activation = 1.0; // the home sheet: ships, after the grid
    scene.nodes[2].on_home = true;

    let call = LatticeCallback::from_scene(
        &scene,
        LatticeLabels::default(),
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        34,
        None,
    );
    assert_eq!(call.instances.len(), 2, "the idle node behind home is culled");
    assert_eq!(
        call.grid_at, 1,
        "the grid draws after the one sheet-behind node that ships, not after \
         the two the scene held",
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
/// The scene is built empty here rather than dialled empty, and that is the
/// honest way round: a silent lattice ships no node already, but its grid
/// survives every setting the panes offer — the line alpha is a constant now
/// and the extent bars stop at 1, so no window a user can ask for is without
/// an adjacent pair. This pins the guard against the day one is.
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
    empty.grid.clear();
    let blank = LatticeCallback::from_scene(
        &empty,
        LatticeLabels::default(),
        size,
        format,
        40,
        Some(stats.clone()),
    );
    assert!(blank.instances.is_empty() && blank.edges.is_empty(), "nothing to draw");
    frame(&blank);
    assert_eq!(
        stats.gpu_ms.load(std::sync::atomic::Ordering::Relaxed),
        GPU_TIME_PENDING,
        "a pane that encodes no pass must not keep reporting the time it took \
         when it last drew",
    );
}

/// Where the pair of nodes below stands, in world units. Off-center in both
/// axes on purpose: a label is drawn into the pane's own pass, and a mapping
/// that flipped or transposed that pane would still land on the right pixel
/// in the middle of the picture.
const STACK_AT: glam::Vec2 = glam::Vec2::new(0.7, -0.5);

/// The pane this scene is drawn into. Bigger than the text fixtures, because
/// this one is about a node's DISC rather than about a glyph: at the real
/// ratio of node radius to lattice spacing, 64 points across puts the whole
/// disc inside a couple of pixels.
const SCENE_SIZE: [u32; 2] = [256, 256];

/// Two nodes on the same pixels, one sevens step apart, and nothing else.
/// Face-on and orthographic, which is the arrangement that puts one node
/// squarely behind another; a sheared or orbited view only spreads the same
/// overlap out.
///
/// Node 0 is the NEARER of the two, so it is the one drawn last.
fn one_node_behind_another() -> Scene {
    let mut scene = parity_scene();
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    scene.node_radius = 0.25;
    let mut near = scene.nodes[0];
    near.world_pos = STACK_AT.extend(0.0);
    near.activation = 1.0;
    near.scale = 1.0;
    near.gutter = 0.0;
    near.hovered = false;
    near.on_home = true;
    let mut far = near;
    far.world_pos = STACK_AT.extend(-1.0);
    far.on_home = false;
    scene.nodes = vec![near, far];
    // The grid would draw across the same pixels, and whether IT covers a
    // label is a separate question that this fixture cannot answer twice.
    scene.grid.clear();
    scene
}

/// A node in front covers the label of the node behind it, the way it covers
/// that node itself — the LETTERS of that label and its drawn marks alike.
///
/// This is the whole feature in one picture, and it is end to end on purpose:
/// the name is drawn inside the lattice's own scene pass, at its node's place
/// in the back-to-front order, so what covers a name is whatever the pass
/// draws after it. Nothing short of running that pass checks it.
///
/// Read as three renders of one scene — the picture with no label, with the
/// FAR node's name, and with the same glyph on the NEAR node, which puts it
/// after everything. The third is what makes the first assertion mean
/// something: a mapping that put the glyph anywhere but on the node would
/// leave the "covered" picture looking exactly right and this one looking
/// blank.
///
/// Run twice over: once for a letter off egui's atlas, once for a drawn mark
/// off the marks' own sheet. The second is #207 — a mark that reaches this
/// pass is covered by the arithmetic that covers everything else in it, so
/// there is nothing mark-shaped in the assertions, and that is the claim. A
/// mark on the painter instead passes none of them: it is drawn over the
/// finished picture, so the disc in front of it never gets a chance.
#[test]
fn a_nearer_node_covers_the_label_of_the_node_behind() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let scene = one_node_behind_another();
    let points = egui::vec2(SCENE_SIZE[0] as f32, SCENE_SIZE[1] as f32);
    let projector = scene.projector(glam::Vec2::new(points.x, points.y));

    // The pixel both discs are painted on, which is where the labels go and
    // where they are read back.
    let on = projector
        .project(scene.nodes[0].world_pos)
        .expect("the stack is in front of the camera");
    let (x, y) = (on.x.round(), on.y.round());
    assert!(
        (x - points.x / 2.0).abs() > 8.0 && (y - points.y / 2.0).abs() > 8.0,
        "the fixture's nodes must sit off-center, at ({x}, {y}) of {points:?}",
    );

    // One glyph, `off` points to the right of that pixel, named by `node`. No
    // rim: the fill alone answers the question, and a rim would spread the
    // reading over pixels nothing is being asked about.
    let bare = [TextRing::default(); 2];
    let picture = |instance: GlyphInstance, off: f32, label: Option<u32>| -> Vec<u8> {
        let (glyphs, labels) = match label {
            Some(node) => (
                vec![GlyphInstance { rect: [x + off - 4.0, y - 4.0, 8.0, 8.0], ..instance }],
                vec![Label { node, glyphs: 1 }],
            ),
            None => (Vec::new(), Vec::new()),
        };
        let cb = LatticeCallback::from_scene(
            &scene,
            LatticeLabels {
                glyphs,
                labels,
                rings: bare,
                atlas: Some(crate::text::tests::atlas()),
                marks: Some(crate::text::tests::mark_sheet()),
                slide: SlideAxis::default(),
            },
            points,
            format,
            9,
            None,
        );
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SCENE_SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));

        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
        let texture = render_to_texture(
            &device,
            &queue,
            SCENE_SIZE,
            format,
            wgpu::Color::TRANSPARENT,
            |pass| {
                cb.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: 1.0,
                        screen_size_px: SCENE_SIZE,
                    },
                    pass,
                    &resources,
                );
            },
        );
        readback(&device, &queue, &texture, SCENE_SIZE)
    };
    // What the probe pixel reads. The glyph is white and everything under it
    // is not, so one channel is the whole of what "how much name is here"
    // means.
    const NEAR: u32 = 0;
    const FAR: u32 = 1;

    for (what, instance) in
        [("a letter", crate::text::tests::glyph()), ("a drawn mark", crate::text::tests::mark())]
    {
        let at = |off: f32, label: Option<u32>| -> u8 {
            let frame = picture(instance, off, label);
            let i = (((y as u32) * SCENE_SIZE[0] + (x + off) as u32) * 4) as usize;
            frame[i + 1]
        };

        // On the disc, which is opaque: the far node's name is gone, exactly
        // gone — this is compositing, not a mask, so "under an opaque disc" is
        // the picture with no label in it at all.
        let (bare_disc, under, over) = (at(0.0, None), at(0.0, Some(FAR)), at(0.0, Some(NEAR)));
        assert_eq!(
            under, bare_disc,
            "{what} under an opaque disc must leave no trace of itself",
        );
        assert!(
            over.abs_diff(bare_disc) > 32,
            "{what} drawn after the disc must be plainly visible on it, \
             got {over} against a bare disc's {bare_disc} — if these agree the \
             glyph is not landing on the node and the assertion above is vacuous",
        );

        // Across the disc's own fading edge, where the difference between
        // covering and cutting shows: the name dims by exactly what the disc
        // took, rather than being taken out whole or left alone.
        let (bare_edge, under_edge, over_edge) =
            (at(6.0, None), at(6.0, Some(FAR)), at(6.0, Some(NEAR)));
        assert!(
            under_edge > bare_edge && under_edge < over_edge,
            "over the disc's fading edge {what} must dim rather than vanish: \
             {under_edge} against {bare_edge} bare and {over_edge} drawn on top",
        );

        // And out in the glow — inside the node's quad, a percent or two of
        // opacity, nothing a reader can see — a name is left alone.
        let (bare_halo, under_halo, over_halo) =
            (at(16.0, None), at(16.0, Some(FAR)), at(16.0, Some(NEAR)));
        assert!(
            over_halo.abs_diff(bare_halo) > 32,
            "the halo probe must be somewhere {what} shows at all: {over_halo} \
             against {bare_halo}",
        );
        assert!(
            under_halo.abs_diff(over_halo) <= 3,
            "out in the invisible glow {what} must be left alone: {under_halo} \
             against {over_halo} drawn on top",
        );
    }
}

/// A label is drawn immediately after the node it names, counted over the
/// instances that actually SHIP.
///
/// Two things that are easy to get right by accident and wrong in the
/// picture. The cull drops a node that can paint nothing, and such a node can
/// still carry a name — a hovered idle one draws no disc and is named all the
/// same — so a label's place is not its node's index in the sorted list. And
/// the labels arrive in the scene's order, which is not the order they are
/// drawn in.
#[test]
fn a_label_takes_its_own_nodes_place_in_the_order() {
    let mut scene = parity_scene();
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    scene.grid.clear();
    let node = |z: f32, activation: f32| harmonigraph_scene::NodeInstance {
        world_pos: glam::Vec3::new(0.0, 0.0, z),
        activation,
        octaves: [0.0; harmonigraph_scene::OCTAVE_SLOTS],
        melody_slots: 0,
        bass_slots: 0,
        melody_level: 0.0,
        bass_level: 0.0,
        trail: 0.0,
        on_home: z == 0.0,
        ..scene.nodes[0]
    };
    // In the scene's own order: the near sheet first, then two silent nodes
    // behind everything, then the home sheet. Drawn back to front that is
    // hush, hush, home, near — so nothing here is in the order it is drawn,
    // and the two silent ones ship no instance at all.
    scene.nodes = vec![node(1.0, 1.0), node(-1.0, 0.0), node(-1.0, 0.0), node(0.0, 1.0)];
    let (near, hush_a, hush_b, home) = (0u32, 1u32, 2u32, 3u32);

    // A glyph per label, told apart by where it claims to be.
    let glyph =
        |at: f32| GlyphInstance { rect: [at, 0.0, 1.0, 1.0], ..crate::text::tests::glyph() };
    let call = LatticeCallback::from_scene(
        &scene,
        LatticeLabels {
            glyphs: vec![glyph(0.0), glyph(1.0), glyph(2.0), glyph(3.0)],
            labels: [near, hush_a, hush_b, home]
                .map(|node| Label { node, glyphs: 1 })
                .to_vec(),
            rings: [TextRing::default(); 2],
            atlas: Some(crate::text::tests::atlas()),
            marks: None,
            slide: SlideAxis::default(),
        },
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        11,
        None,
    );

    assert_eq!(call.instances.len(), 2, "only the two sounding nodes ship an instance");
    assert_eq!(
        call.seams,
        vec![
            // Both silent nodes: nothing has been drawn yet, and two labels
            // at one seam are one uninterrupted draw. They are behind the
            // home sheet, so they are also behind the grid.
            GlyphSeam { at: 0, start: 0, count: 2, after_grid: false },
            // The home sheet's own name, after its disc.
            GlyphSeam { at: 1, start: 2, count: 1, after_grid: true },
            // And the near sheet's, after everything.
            GlyphSeam { at: 2, start: 3, count: 1, after_grid: true },
        ],
        "a label goes after its own node, over the instances that ship",
    );
    assert_eq!(
        call.glyphs.iter().map(|g| g.rect[0]).collect::<Vec<_>>(),
        vec![1.0, 2.0, 3.0, 0.0],
        "the glyphs are regrouped into the order they are drawn in",
    );
}

/// A name on a node that ships no disc still draws over the grid, not under
/// it — the case where the two runs meet at the same number.
///
/// `grid_at` is where the grid goes, counted over the instances that SHIP.
/// The sheets behind the home one draw before it and the home sheet after,
/// so the boundary between the two runs is exactly `grid_at`. A node that
/// paints nothing ships nothing, which leaves its seam sitting on the
/// boundary rather than past it: with every node on the home sheet — the
/// stock `extent_sevens: 0` — the far run is empty, `grid_at` is 0, and the
/// first home node to be culled takes seam 0 as well. Reading the side off
/// `at > grid_at` then files that node's name with the sheets BEHIND the
/// grid, and the grid is painted over the name.
///
/// The state is the plugin's resting one, which is what makes it worth a
/// test of its own: stock view, nothing played, hover any node. An idle node
/// draws nothing at all, and a hovered node is named whether or not it
/// draws.
#[test]
fn a_culled_home_nodes_name_draws_over_the_grid_it_shares_a_seam_with() {
    let mut scene = parity_scene();
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    // The stock resting state: one sheet, and an untouched node painting
    // nothing at all, which is every idle node.
    let node = |activation: f32| harmonigraph_scene::NodeInstance {
        world_pos: glam::Vec3::new(0.0, 0.0, 0.0),
        activation,
        octaves: [0.0; harmonigraph_scene::OCTAVE_SLOTS],
        melody_slots: 0,
        bass_slots: 0,
        melody_level: 0.0,
        bass_level: 0.0,
        trail: 0.0,
        on_home: true,
        ..scene.nodes[0]
    };
    // The hovered one is silent and first, so it is culled before anything
    // has shipped and its seam is 0 — the same number as `grid_at`.
    scene.nodes = vec![node(0.0), node(1.0)];
    let glyph = GlyphInstance { rect: [0.0, 0.0, 1.0, 1.0], ..crate::text::tests::glyph() };
    let call = LatticeCallback::from_scene(
        &scene,
        LatticeLabels {
            glyphs: vec![glyph],
            labels: vec![Label { node: 0, glyphs: 1 }],
            rings: [TextRing::default(); 2],
            atlas: Some(crate::text::tests::atlas()),
            marks: None,
            slide: SlideAxis::default(),
        },
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        13,
        None,
    );

    assert!(!scene.grid.is_empty(), "the fixture needs a grid for the name to be covered BY");
    assert_eq!(call.grid_at, 0, "with one sheet there is nothing to draw before the grid");
    assert_eq!(
        call.seams,
        vec![GlyphSeam { at: 0, start: 0, count: 1, after_grid: true }],
        "a home node's name draws after the grid even when the cull leaves it on grid_at",
    );
}

/// A label is not in the bloom: the light the bloom adds to a frame is the
/// same whether the names are drawn or not.
///
/// Two halves, and the second is the one that is easy to miss. A name in the
/// bloom input GLOWS, which is the obvious half — white type well over the
/// bright pass's threshold, haloed like a lit node. It also takes a BITE out
/// of the halo of the node it covers, by standing where that node's own
/// bright pixels were, so a node dims as a name crosses it. Both are what a
/// second colour attachment buys, and both show up here as the same
/// difference.
///
/// Measured as bloom LIGHT rather than as pixels, since that is the thing
/// that must not change: the composite adds `bloom * strength` on top of the
/// scene, so rendering each arrangement with the bloom on and off and
/// subtracting isolates exactly the term under test. The scene itself does
/// change with a label in it — that is the feature — so comparing frames
/// directly would be comparing the label to itself.
#[test]
fn a_label_adds_no_light_through_the_bloom() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let scene = one_node_behind_another();
    let points = egui::vec2(SCENE_SIZE[0] as f32, SCENE_SIZE[1] as f32);
    let on = scene
        .projector(glam::Vec2::new(points.x, points.y))
        .project(scene.nodes[0].world_pos)
        .expect("the stack is in front of the camera");
    let (x, y) = (on.x.round(), on.y.round());

    // The near node's name, right on the node — over its brightest pixels,
    // which is where a label robs a halo of the most light.
    let label = |on: bool| -> (Vec<GlyphInstance>, Vec<Label>) {
        if !on {
            return (Vec::new(), Vec::new());
        }
        (
            vec![GlyphInstance {
                rect: [x - 6.0, y - 6.0, 12.0, 12.0],
                ..crate::text::tests::glyph()
            }],
            vec![Label { node: 0, glyphs: 1 }],
        )
    };
    let frame = |labelled: bool, bloom: f32| -> Vec<u8> {
        let (glyphs, labels) = label(labelled);
        // A fresh fixture per frame: `Scene` is not `Clone`, and the only
        // thing that varies is the strength the composite reads.
        let mut scene = one_node_behind_another();
        scene.bloom_strength = bloom;
        let cb = LatticeCallback::from_scene(
            &scene,
            LatticeLabels {
                glyphs,
                labels,
                rings: [TextRing::default(); 2],
                atlas: Some(crate::text::tests::atlas()),
                marks: None,
                slide: SlideAxis::default(),
            },
            points,
            format,
            13,
            None,
        );
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SCENE_SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
        let clear = wgpu::Color::TRANSPARENT;
        let texture =
            render_to_texture(&device, &queue, SCENE_SIZE, format, clear, |pass| {
                cb.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: 1.0,
                        screen_size_px: SCENE_SIZE,
                    },
                    pass,
                    &resources,
                );
            });
        readback(&device, &queue, &texture, SCENE_SIZE)
    };
    let (lit, plain) = (frame(true, 1.0), frame(true, 0.0));
    let (bare_lit, bare_plain) = (frame(false, 1.0), frame(false, 0.0));

    // Non-vacuous first: there has to BE bloom, and it has to reach the
    // pixels the label covers — otherwise this passes on a frame with no
    // light in it to move.
    let light = |on: &[u8], off: &[u8], i: usize| on[i] as i32 - off[i] as i32;
    let near_label = |i: usize| {
        let row = SCENE_SIZE[0] as usize;
        let (px, py) = (((i / 4) % row) as f32, ((i / 4) / row) as f32);
        (px - x).abs() <= 10.0 && (py - y).abs() <= 10.0
    };
    let peak = (0..lit.len())
        .filter(|i| i % 4 != 3 && near_label(*i))
        .map(|i| light(&bare_lit, &bare_plain, i))
        .max()
        .unwrap_or(0);
    assert!(peak > 8, "the fixture must bloom where the label sits, peak {peak}");

    // And the label changes none of it. Saturated channels are skipped: the
    // glyph is white, so where it lands the composite is already at 255 with
    // the bloom off and the added light has nowhere to go — a clamp, not a
    // reading.
    let (mut worst, mut at) = (0i32, 0usize);
    for i in (0..lit.len()).filter(|i| i % 4 != 3) {
        if lit[i] == 255 || bare_lit[i] == 255 {
            continue;
        }
        let d = light(&lit, &plain, i) - light(&bare_lit, &bare_plain, i);
        if d.abs() > worst.abs() {
            (worst, at) = (d, i);
        }
    }
    assert!(
        worst.abs() <= 1,
        "a label changed the bloom by {worst}/255 at pixel ({}, {}) — it is in the bright \
         pass's input, so it glows and eats the halo of the node it covers",
        (at / 4) % SCENE_SIZE[0] as usize,
        (at / 4) / SCENE_SIZE[0] as usize,
    );
}
