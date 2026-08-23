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

/// The ink strip is as wide as the texture it is drawn into.
///
/// Two constants, one number: [`INK_STRIP_N`] sizes the texture and every
/// index into it on the Rust side, and lattice.wgsl's own `INK_STRIP_N` is
/// what the shader walks, samples and wraps at. Nothing checks them against
/// each other — the strip is a colour attachment, so no binding size is
/// validated, and both halves are well-formed at any value.
///
/// A one-sided bump is silent and disfigures every node. Raise the shader's
/// alone and the read pass writes columns past the target's edge, which are
/// dropped, so the blur averages the ink of a partial turn and calls it the
/// whole; raise the Rust one alone and the columns past the shader's idea of a
/// turn are never written, and the light takes its colour from a cleared texel
/// wherever the fragment's angle lands in them.
#[test]
fn the_shaders_ink_strip_is_as_wide_as_the_texture_it_is_drawn_into() {
    let needle = format!("const INK_STRIP_N: u32 = {INK_STRIP_N}u;");
    assert!(
        SHADER_SRC.contains(&needle),
        "lattice.wgsl must declare `{needle}` to match INK_STRIP_N here ({INK_STRIP_N}); the \
         strip is allocated that wide and the shader would walk a different turn",
    );
}

/// The Feather bar draws the light's falloff on itself, and the line it draws
/// is a COPY of the shader's skirt (`harmonigraph_scene::glow_skirt`) rather
/// than the skirt itself — the shader is the only place the light is
/// computed, and there is nothing on the CPU to hand the bar. So the copy is
/// held to the shader's text: the two rates the bar mixes between and the two
/// lines that spend them. A preview that drifted from the picture would be
/// worse than none, and nothing on screen would show the drift.
#[test]
fn the_feather_bars_preview_is_the_skirt_the_shader_draws() {
    let needles = [
        format!("const GLOW_FALLOFF_TIGHT: f32 = {:?};", harmonigraph_scene::GLOW_FALLOFF_TIGHT),
        format!("const GLOW_FALLOFF_FLAT: f32 = {:?};", harmonigraph_scene::GLOW_FALLOFF_FLAT),
        "let rate = mix(GLOW_FALLOFF_TIGHT, GLOW_FALLOFF_FLAT, glow_feather());".to_owned(),
        "let window = 1.0 - smoothstep(span * 0.5, span, d);".to_owned(),
        "let skirt = GLOW_BASE * exp(-rate * d / span) * window;".to_owned(),
    ];
    for needle in &needles {
        assert!(
            SHADER_SRC.contains(needle),
            "lattice.wgsl must contain `{needle}`: harmonigraph_scene::glow_skirt mirrors the \
             skirt line for line to draw the Feather bar's preview, so a change to either \
             is a change to both",
        );
    }
}

/// The same contract for the Gap curve bar, whose preview is a copy of the
/// standoff's ramp and the exponent it is raised to
/// (`harmonigraph_scene::standoff_recovery`).
#[test]
fn the_gap_curve_bars_preview_is_the_ramp_the_shader_runs() {
    let needles = [
        format!("const GAP_SHAPE_TRAIL: f32 = {:?};", harmonigraph_scene::GAP_SHAPE_TRAIL),
        format!("const GAP_SHAPE_HOLD: f32 = {:?};", harmonigraph_scene::GAP_SHAPE_HOLD),
        "return GAP_SHAPE_TRAIL * pow(GAP_SHAPE_HOLD / GAP_SHAPE_TRAIL, t);".to_owned(),
        "let ramp = smoothstep(inner, edge, sd);".to_owned(),
        "return 1.0 - pow(ramp, glow_gap_shape());".to_owned(),
    ];
    for needle in &needles {
        assert!(
            SHADER_SRC.contains(needle),
            "lattice.wgsl must contain `{needle}`: harmonigraph_scene::standoff_recovery mirrors \
             the standoff's ramp to draw the Gap curve bar's preview, so a change to either is \
             a change to both",
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
/// octave indicators, and resting markers under them, all overlapping so blend
/// order matters.
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
            // The lattice pass draws the ring on every node it ships; the
            // gate is the fold's answer and there is no fold here.
            audio_ring: 1.0,
            // Drawn at full and READING full: a fixture with a dialled-in
            // annulus is the ungated picture, and the light asks this one
            // (see `NodeInstance::ring_peak`).
            ring_peak: 1.0,
            // A row per node in the order they are built, settled: the light's
            // own clock is the shell's pass and no shell has run here, so this
            // fixture is the picture with nothing carried — which is exactly
            // what a still image of the draw paths wants. The mark the light
            // is sized against is settled on this node's own, for the same
            // reason.
            glow: harmonigraph_scene::GlowStep {
                level: 1.0,
                row: i,
                mix: 1.0,
                marked: f32::from(i == 0 || i == 2 || i == 4),
            },
            trail: 0.0,
        });
    }
    // Two markers, one under a node and one clear of every node, so the pass is
    // exercised both where the nodes composite over it and where it stands
    // alone. Different radii, because the size is per instance.
    let pluses = vec![
        harmonigraph_scene::PlusInstance {
            pos: Vec3::new(-1.8, -0.6, -0.3),
            radius: 0.22,
            color: Vec4::new(0.16, 0.17, 0.20, 1.0),
            strength: 0.55,
        },
        harmonigraph_scene::PlusInstance {
            pos: Vec3::new(0.0, 0.0, 0.0),
            radius: 0.13,
            color: Vec4::new(0.16, 0.17, 0.20, 1.0),
            strength: 0.4,
        },
    ];
    let glow_rows = nodes.len() as u32;
    Scene {
        nodes,
        camera: harmonigraph_scene::Camera::default(),
        now: 1.25,
        // The ground the sevens knockout clears to; the half of this
        // scene's nodes that carry a gutter exercise it.
        background: harmonigraph_scene::skin::well_color(),
        // The grey the octave band's unsounding slices draw, at the fresh
        // view's own Ground — most of every node's band in this fixture.
        lattice_ground: harmonigraph_scene::grey_of_lightness(
            harmonigraph_scene::ViewConfig::default().lattice_ground,
        ),
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
        outer_inner: 0.545,
        outer_outer: 0.795,
        // The band is the outermost ring here, as it is on a fresh node, so it
        // is what the marks stand off — a gap out from its edge.
        rings_outer: 0.795,
        mark_inner: 0.795 + 0.12,
        // The same 0.12 the radii above are spaced by, the fixture standing its
        // layers off each other and cutting its sectors by one padding.
        octave_gap: 0.12,
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
        pluses,
        // Arms a little over half their length across, the proportion a fresh
        // view draws, so a marker here is the shape every width test is a
        // departure from.
        plus_half_width: 0.275,
        // Square-ended arms, so the fixture's markers are the shape the taper
        // is a departure from.
        plus_taper_start: 1.0,
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
        // And the node glow with it: it is a pass of its own into a target of
        // its own, composited into the scene pass, which the
        // single-attachment reference path has no pass to composite into.
        glow_reach: 0.0,
        glow_strength: 1.0,
        // The accent's own falloff, which is what every glow test here that
        // does not say otherwise is measuring.
        glow_feather: 0.0,
        // Melded whole, the light two overlapping halos have always made, so a
        // test that says nothing about the Meld is measuring the screen.
        glow_meld: 1.0,
        // The fresh standoff and the shares that shape the light inside it and
        // over the node's own ink, inert at reach 0 and here to say so.
        glow_gap: 0.16,
        glow_gap_soft: 0.16,
        glow_gap_shape: 0.5,
        glow_gap_depth: 0.85,
        glow_wash: 0.15,
        glow_blend: 0.5,
        // A row per node, which is what the nodes above are built with.
        glow_rows,
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
        self.draw(scene, labels)
    }

    /// The frame AFTER the last shot, on the same pane rather than a picture
    /// of its own.
    ///
    /// One thing survives a frame here and it is a node's light: the ink strip
    /// keeps each row's colour, and a row's next reading is mixed into what it
    /// already held (`harmonigraph_scene::GlowStep::mix`). That is exactly what
    /// every other shot's fresh pane exists to keep out — a fixture is settled
    /// unless it says otherwise — so a test that wants the carrying has to ask
    /// for it.
    fn shot_again(&mut self, scene: &Scene) -> Vec<u8> {
        self.draw(scene, LatticeLabels::default())
    }

    fn draw(&mut self, scene: &Scene, labels: LatticeLabels) -> Vec<u8> {
        // A lit node's ink-strip row is read back by IDENTITY, out of a strip
        // `glow_rows` tall, so a row past the end is not a small error in a
        // fixture: what an out-of-range `textureLoad` returns is the BACKEND's
        // choice. Metal clamps, so the node silently reads row 0 and wears
        // another node's colour while the test goes on passing; a backend that
        // returns zero instead makes the same test vacuous, passing with the
        // draw it is about deleted. Either way the fixture measures something
        // other than what it says, and says nothing when it stops measuring at
        // all — so the bookkeeping is asserted here, once, for every shot.
        //
        // `rows_per_node` is the helper that gets this right; a fixture that
        // pushes a node by hand has to call it.
        for (i, node) in scene.nodes.iter().enumerate() {
            assert!(
                node.glow.level <= 0.0 || node.glow.row < scene.glow_rows,
                "node {i} claims ink-strip row {} of a strip {} tall — see `rows_per_node`",
                node.glow.row,
                scene.glow_rows,
            );
        }
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
    let layouts =
        SceneLayouts { uniforms: &res.bind_group_layout, glow: &res.glow_layout };
    let (node_pipeline, plus_pipeline) =
        create_pipelines(&device, SHADER_SRC, format, layouts, false);
    // The stand-in light at group 1: this path has no glow pass to composite,
    // and the fixture asks for none (`parity_scene` holds the reach at 0), so
    // the offscreen path is reading the same transparent nothing.
    let light = &res.glow_dummy_bind_group;
    let pane = res.panes.get(&7).expect("prepare created the pane");
    let direct_tex = render_to_texture(&device, &queue, SIZE, format, clear, |pass| {
        // The markers sit at the home sheet's depth, so they are drawn INSIDE
        // the node run, at `pluses_at` — mirror that or the two paths differ
        // by draw order rather than by the thing under test.
        let nodes = |pass: &mut wgpu::RenderPass<'static>, range: std::ops::Range<u32>| {
            if !range.is_empty() {
                pass.set_pipeline(&node_pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_bind_group(1, light, &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                pass.draw(0..4, range);
            }
        };
        nodes(pass, 0..pane.pluses_at);
        if pane.plus_count > 0 {
            pass.set_pipeline(&plus_pipeline);
            pass.set_bind_group(0, &pane.bind_group, &[]);
            pass.set_bind_group(1, light, &[]);
            pass.set_vertex_buffer(0, pane.plus_buffer.slice(..));
            pass.draw(0..4, 0..pane.plus_count);
        }
        nodes(pass, pane.pluses_at..pane.instance_count);
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
        // The lattice pass draws the ring on every node it ships; the
        // gate is the fold's answer and there is no fold here.
        audio_ring: 1.0,
        // Drawn at full and READING full: a fixture with a dialled-in
        // annulus is the ungated picture, and the light asks this one
        // (see `NodeInstance::ring_peak`).
        ring_peak: 1.0,
        // Lit and settled on the strip's first row: one node, nothing carried,
        // and the light sized against whichever ends this fixture is wearing.
        glow: harmonigraph_scene::GlowStep {
            level: 1.0,
            row: 0,
            mix: 1.0,
            marked: f32::from((melody_slots | bass_slots) != 0),
        },
        trail: 0.0,
    }];
    // One node, so one row — the strip follows the scene it is handed.
    scene.glow_rows = 1;
    scene.pluses.clear();
    // Fill a good share of the frame, so the measurements below are
    // about the mark's design rather than about pixel quantization.
    scene.node_radius = 1.1;
    scene
}

/// Hand every node in a scene its own row of the ink strip, and size the strip
/// to them.
///
/// What the shell's own pass does with a map from node to row
/// (`panes::glow_fade` in harmonigraph-ui), reduced to what a fixture needs: a
/// scene assembled by hand is one frame with nothing carried, so a row per node
/// in the list's own order is both unique and stable. It is [`derive_scene`]'s
/// own answer, and the reason a fixture has to restate it is that replacing
/// `scene.nodes` replaces the rows with copies of one.
///
/// A row is read back by identity, so two nodes sharing one is not a subtle
/// wrong: both write it and both read whichever won.
fn rows_per_node(scene: &mut Scene) {
    for (row, node) in scene.nodes.iter_mut().enumerate() {
        node.glow.row = row as u32;
    }
    scene.glow_rows = scene.nodes.len() as u32;
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
/// probe's 200¢ Range would be an eighth of a pixel of it. Symmetric, so the
/// lit arc's centroid is the band's own centre pitch whatever the Range.
const PARTIAL_HALF_CENTS: f32 = 40.0;

/// The padding `ringing_node` stands its layers off each other by — see there.
/// Spent on BOTH of the node's axes in these fixtures, radially between the
/// layers and angularly between the sectors, which is what the view's two gap
/// bars are free to dial apart: a probe reading a radius wants the layers
/// pixels apart, and one reading a sector wants the seams pixels wide.
const PROBE_GAP: f32 = 0.12;

/// Where the probe stacks BEGIN, and it is the node's own centre: a radius read
/// off one of these pictures is then a width, or a sum of widths and gaps, with
/// no offset under it. Stated rather than inherited for the same reason every
/// other size here is — a fresh view stands its stack well out from the centre
/// (see [`ViewConfig::ring_inner`](harmonigraph_scene::ViewConfig)), and the
/// probe widths, deliberately wide so a pixel reading can tell one layer's edge
/// from the next, do not fit in what that leaves.
const PROBE_INNER: f32 = 0.0;

/// The octave band's width for the probes below, standing in for the fresh
/// view's own (see [`ViewConfig::band_width`](harmonigraph_scene::ViewConfig))
/// the same way [`PROBE_GAP`] stands in for the gap: the band is the outermost
/// ring the stack has to fit, so it is the layer a retune of anything INSIDE
/// it pushes off the quad edge, and a band the stack has refused draws nothing
/// for a pixel reading to find.
const PROBE_BAND_WIDTH: f32 = 0.163_084_63;

/// The angular padding the clearing probe slices its wedges at, standing in
/// for the fresh view's own (see
/// [`ViewConfig::octave_gap`](harmonigraph_scene::ViewConfig)): the clearing
/// is read across a wedge's own arc, and a slicing dialled wide enough eats
/// the arc the reading is taken over.
const PROBE_OCTAVE_GAP: f32 = 0.05;

/// The Range these fixtures read their partials against, standing in for the
/// fresh view's own (see
/// [`ViewConfig::spectral_ring_range`](harmonigraph_scene::ViewConfig)) the
/// same way [`PROBE_GAP`] stands in for the gap: a window dialled narrow
/// enough leaves a detune too small for a 256 px shot to resolve, which is a
/// property of the shot's resolution rather than of the Range being tested.
const PROBE_RANGE: f32 = 200.0;

/// One node wearing both rings at the probe's own widths: `held` lighting an
/// octave of the band, and a synthetic partial at absolute MIDI `sounding` in
/// the analyzer's grid for the audio ring to find.
///
/// The probe's stack rather than the fresh view's, because what the claims
/// below need is two annuli far enough apart for a 256 px shot to tell one
/// from the other — a proportion, not the shipped one. Inheriting the fresh
/// widths would tie every reading here to an aesthetic that moves whenever a
/// dialled-in look is captured, and it moves the readings the wrong way:
/// [`PROBE_BAND_WIDTH`] is the outermost ring, so a retune of the audio ring
/// INSIDE it can push the band off the quad edge, and a refused
/// band leaves these measurements nothing to find. That the shipped stack
/// draws three visible layers inside the quad is held where it belongs, on the
/// fresh view itself, by `harmonigraph_scene`'s
/// `the_fresh_node_stacks_three_visible_layers_inside_the_quad`.
///
/// The PADDING is the probe's for a second reason: the Ring gap is what
/// separates every layer of a node, and a gap of the order the fresh view
/// carries is under three pixels on the 52-px node this renders, where the two
/// annuli's anti-aliased edges meet inside it. A wider gap measures the
/// geometry rather than the edge softness.
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
    // The probe's stack: the two rings land far enough apart to be measured in
    // pixels, at radii no capture of a dialled-in look moves.
    let rings = harmonigraph_scene::ViewConfig {
        ring_inner: PROBE_INNER,
        ring_gap: PROBE_GAP,
        spectral_ring_width: PROBE_RING_WIDTH,
        band_width: PROBE_BAND_WIDTH,
        ..fresh.clone()
    }
    .rings();
    scene.outer_inner = rings.band.0;
    scene.outer_outer = rings.band.1;
    scene.rings_outer = rings.outer;
    scene.mark_inner = rings.mark_inner;
    scene.octave_gap = PROBE_GAP;

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
    // The probe Range, which every angle below is measured at unless it says
    // otherwise.
    let fresh_range = PROBE_RANGE;

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

/// A node the gate holds back draws exactly the picture it would draw with the
/// ring layer OFF: the annulus goes, and the octave band, the marks and the
/// node's own body stay pixel for pixel.
///
/// Two claims in one comparison, and the second is the one worth the GPU. That
/// the gate removes the ring is arithmetic anyone can read off the shader; that
/// it removes NOTHING ELSE is a property of where the test sits in the fragment
/// program, and the ways it can fail all draw a plausible picture — a gate
/// applied before the wedge walk instead of inside it, or one that fell through
/// to the layer under it, would take the band's ghost or the glyph's edge with
/// it and read as a node that changed shape when the music went quiet.
#[test]
fn a_gated_node_loses_its_ring_and_nothing_else() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let fresh_range = PROBE_RANGE;
    // A node with an octave held and a partial sounding at that same octave, so
    // both rings have something to draw and the two can be told apart in the
    // shot.
    let slot = harmonigraph_scene::MIDDLE_C_SLOT;
    let sounding = slot as f32 * 12.0;
    let lit = ringing_node(Some(slot), Some(sounding), fresh_range);
    let ringing = gpu.shot(&lit);

    // The same node held back by the gate...
    let mut gated = ringing_node(Some(slot), Some(sounding), fresh_range);
    gated.nodes[0].audio_ring = 0.0;
    // ...against the same node with the LAYER off, which is the picture a gated
    // node has to come out as.
    let mut layer_off = ringing_node(Some(slot), Some(sounding), fresh_range);
    layer_off.spectral.inner = 0.0;
    layer_off.spectral.outer = 0.0;

    let dark = gpu.shot(&gated);
    assert!(
        differing_pixels(&ringing, &dark) > 0,
        "the ungated node drew no ring, so there is nothing for the gate to take",
    );
    assert_eq!(
        differing_pixels(&dark, &gpu.shot(&layer_off)),
        0,
        "a gated node is not the picture the ring layer being off draws",
    );
    // And the ring is what went: the light that differs sits in the audio
    // ring's own annulus, well inside the band. (`light_over` is the ungated
    // shot less the gated one, so what it holds is exactly the ring.)
    let ring = light_about_center(&light_over(&ringing, &dark), SIZE);
    let bare = gpu.shot(&ringing_node(None, None, fresh_range));
    let band = light_about_center(&light_over(&ringing, &bare), SIZE);
    assert!(ring.weight > 0.0, "nothing at all was taken away");
    assert!(
        ring.far + 2.0 < band.far,
        "what the gate took reaches {:.1} px, past the node's own band at {:.1}",
        ring.far,
        band.far,
    );
}

/// A ring part way through its fade is the ring drawn OVER the picture without
/// it, at a fraction of its coverage — every pixel of the node between the two
/// pictures the ends of the fade draw, and no pixel outside the annulus moved.
///
/// What the fade has to be if it is to read as a ring arriving rather than as
/// the node changing: the level scales the RING's coverage, so what shows
/// through is the octave layer under it. The ways it can fail all draw a
/// plausible picture and none of them is this — a level mixed into the wedge's
/// COLOUR would draw a reading of a quieter spectrum, and one applied to the
/// composite would fade the band and the marks with it.
///
/// A quarter and not a half, so that a shot which merely picked one END of the
/// fade cannot pass by landing between the two.
#[test]
fn a_ring_part_way_through_its_fade_sits_between_the_two() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let fresh_range = PROBE_RANGE;
    let slot = harmonigraph_scene::MIDDLE_C_SLOT;
    let sounding = slot as f32 * 12.0;
    let full = gpu.shot(&ringing_node(Some(slot), Some(sounding), fresh_range));

    let mut none = ringing_node(Some(slot), Some(sounding), fresh_range);
    none.nodes[0].audio_ring = 0.0;
    let none = gpu.shot(&none);

    let mut part = ringing_node(Some(slot), Some(sounding), fresh_range);
    part.nodes[0].audio_ring = 0.25;
    let part = gpu.shot(&part);

    assert!(differing_pixels(&full, &none) > 0, "the ring drew nothing to fade");
    assert!(differing_pixels(&part, &none) > 0, "a quarter of a ring drew nothing at all");
    assert!(differing_pixels(&part, &full) > 0, "a quarter of a ring is the whole of one");

    // Between the two, channel by channel. The slack is the compositing's own
    // rounding — the ring is blended in 8-bit twice over — and not a tolerance
    // on the claim: a level that reached the colour instead would leave the
    // wedges the same coverage and paint them a different colour, which lands
    // outside the pair wherever the ramp is not monotone in the channel.
    let mut moved = 0;
    for ((p, a), b) in part.chunks(4).zip(full.chunks(4)).zip(none.chunks(4)) {
        for c in 0..3 {
            let (low, high) = (a[c].min(b[c]), a[c].max(b[c]));
            assert!(
                i32::from(p[c]) >= i32::from(low) - 2 && i32::from(p[c]) <= i32::from(high) + 2,
                "a quarter-faded pixel reads {} where the ends read {low} and {high}",
                p[c],
            );
        }
        if a != b {
            moved += 1;
        }
    }
    assert!(moved > 0, "the two ends of the fade drew one picture");
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

    // The probe's stack, so the layers are pixels apart: audio ring, band, and
    // the mark outside both. The claim is that the mark finds whichever ring is
    // outermost, which wants a stack that draws all of them rather than the one
    // the fresh view happens to be dialled to.
    let fresh = harmonigraph_scene::ViewConfig::default();
    let rings = harmonigraph_scene::ViewConfig {
        ring_inner: PROBE_INNER,
        ring_gap: PROBE_GAP,
        spectral_ring_width: PROBE_RING_WIDTH,
        band_width: PROBE_BAND_WIDTH,
        ..fresh.clone()
    }
    .rings();
    let staged = |band: bool| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, 0);
        scene.octave_gap = PROBE_GAP;
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
/// The stack ([`ViewConfig::rings`](harmonigraph_scene::ViewConfig::rings))
/// writes that rule down for every layer it owns: the gap is skipped at a
/// cursor of 0, where there
/// is nothing to stand off, so the innermost layer closes into a disc instead
/// of opening a hole the size of a padding around nothing. The mark is the one
/// layer it does NOT own — the strip's inner edge is re-derived in WGSL off
/// `rings_outer`, which is handed the cursor and not the rule — so the two
/// answers part company at exactly the one cursor the rule is about.
///
/// The state is a reduction the Lattice page's own Layers bar reaches: the
/// core, the audio ring and the octave band all have 0 as their off position,
/// which is their handle dragged home, and reading the lattice as melody/bass
/// marks alone is what taking all three there is for.
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
        ring_inner: PROBE_INNER,
        ring_gap: PROBE_GAP,
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
        scene.octave_gap = PROBE_GAP;
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

/// How far past its own body the clearing in the tests below reaches, in the uv
/// of a full-size node, and — through `sevens_soft` at 0 — how gradually it
/// gets there: not at all. A hard rim lands on a pixel or two, where the fade
/// the app ships spreads it over the dozen a reading would then have to pick a
/// level out of.
const CLEAR_REACH: f32 = 0.30;

/// The audio ring's width for the clearing probe, standing in for the fresh
/// view's own (see [`ViewConfig::spectral_ring_width`](harmonigraph_scene::ViewConfig))
/// the same way [`PROBE_GAP`] stands in for its gap: CLEAR_REACH is a fixed
/// reach in uv, and a ring dialled thinner than that leaves too few pixels
/// past it for the clearing-fraction reading below to hold its tolerance.
const PROBE_RING_WIDTH: f32 = 0.3;

/// The stack the clearing tests are measured against: three layers at the
/// probe's own widths and wider padding, so a pixel reading can tell one
/// layer's edge from the next.
///
/// Every width the stack is built from is stated rather than inherited. What
/// the clearing tests need is a node wearing all three layers with room between
/// them; a capture of a dialled-in look is free to dial any of them to a
/// hairline, and each one left inheriting is a way for these readings to fail
/// on a change that has nothing to do with clearing.
fn clearing_rings() -> harmonigraph_scene::RingStack {
    let fresh = harmonigraph_scene::ViewConfig::default();
    harmonigraph_scene::ViewConfig {
        ring_inner: PROBE_INNER,
        ring_gap: PROBE_GAP,
        spectral_ring_width: PROBE_RING_WIDTH,
        band_width: PROBE_BAND_WIDTH,
        octave_gap: PROBE_OCTAVE_GAP,
        ..fresh
    }
    .rings()
}

/// One node sitting in its own clearing at [`clearing_rings`]'s radii: `melody`
/// names the slot its mark extends (0 for no mark), `ring` how much of its audio
/// ring the view's Gate leaves it, `band` whether the octave band is on, and
/// `gutter` the clearing's reach (0 for none).
///
/// The ground is WHITE where the app clears to the pane's own panel: every
/// reading below is "what changed when the gutter was turned on", and a bright
/// ground makes that the whole range of a channel at a cleared pixel instead of
/// a few levels over black.
///
/// The node is drawn small enough for its clearing to fit in the frame — the
/// hole reaches a third of a node past a mark that already stands outside every
/// ring.
fn clearing_node(melody: u32, ring: f32, band: bool, gutter: f32) -> Scene {
    let rings = clearing_rings();
    let mut scene = single_marked_node(melody, 0);
    scene.background = glam::Vec4::ONE;
    scene.node_radius = 1.4;
    scene.octave_gap = PROBE_GAP;
    scene.mark_thickness = rings.mark_thickness;
    // The audio ring drawn off an all-zero grid: the ramp's floor across the
    // whole annulus, which is ink at a known radius and all this asks of it.
    let mut paint = harmonigraph_scene::SpectralPaint::silent();
    paint.lut = std::array::from_fn(|_| glam::Vec4::new(1.0, 1.0, 1.0, 1.0));
    (paint.inner, paint.outer) = rings.audio;
    scene.spectral = paint;
    (scene.outer_inner, scene.outer_outer) = if band { rings.band } else { (0.0, 0.0) };
    // The view's own cursor either way: which layer it landed on is the band
    // with one and the audio ring without, and `ring` is a per-NODE answer that
    // leaves it where it is.
    scene.rings_outer = if band { rings.band.1 } else { rings.audio.1 };
    scene.mark_inner = scene.rings_outer + rings.gap;
    scene.sevens_soft = 0.0;
    let node = &mut scene.nodes[0];
    node.gutter = gutter;
    node.audio_ring = ring;
    scene
}

/// How far the light in `weights` reaches from the centre of the frame within
/// `cone` degrees of `toward`, in pixels.
///
/// One direction's radius, where [`Light::far`] is the largest over every
/// direction at once — which is the whole question about a shape that is no
/// longer a circle.
fn far_toward(weights: &[f64], size: [u32; 2], toward: f64, cone: f64) -> f64 {
    let (cx, cy) = ((size[0] - 1) as f64 / 2.0, (size[1] - 1) as f64 / 2.0);
    let mut far = 0.0f64;
    for (i, &w) in weights.iter().enumerate() {
        if w < RING_LIT {
            continue;
        }
        let x = (i % size[0] as usize) as f64 - cx;
        let y = (i / size[0] as usize) as f64 - cy;
        if angle_apart(y.atan2(x), toward) <= cone {
            far = far.max(x.hypot(y));
        }
    }
    far
}

/// A node's clearing is the node's own SHAPE one reach out, so a melody mark
/// pushes the hole out over the wedge it extends and nowhere else.
///
/// The circle this replaces is sized to hold the node whichever direction it
/// reaches furthest in, and a mark reaches a whole strip further than the rings
/// do: a marked node cleared a gap wider than itself all the way round, so a
/// hole that says "this node is in front of that one" said it about a ring of
/// empty lattice too. That is visible exactly where the clearing is for — over
/// the resting markers and the sheets behind — and invisible in the node's own
/// picture, which is why every reading here is off the difference the gutter
/// makes rather than off the node.
#[test]
fn a_clearing_bulges_over_the_mark_and_hugs_the_rings_everywhere_else() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let rings = clearing_rings();
    let mark_out = rings.mark_inner + rings.mark_thickness;

    let bare_plain = gpu.shot(&clearing_node(0, 1.0, true, 0.0));
    let holed_plain = gpu.shot(&clearing_node(0, 1.0, true, CLEAR_REACH));
    let bare_marked = gpu.shot(&clearing_node(MIDDLE_C, 1.0, true, 0.0));
    let holed_marked = gpu.shot(&clearing_node(MIDDLE_C, 1.0, true, CLEAR_REACH));

    // Which way the mark points, taken off the picture: the marked node over the
    // same node with its mark off.
    let mark = light_about_center(&light_over(&bare_marked, &bare_plain), SIZE);
    assert!(mark.weight > 0.0, "the mark drew nothing to aim at");
    let away = mark.angle + std::f64::consts::PI;

    let plain = light_over(&holed_plain, &bare_plain);
    let marked = light_over(&holed_marked, &bare_marked);
    // A cone inside the wedge the mark extends — a full-size slice of the fresh
    // wheel is 55 degrees, so ±27 — and wide enough to hold the rounding the
    // dilation puts on its corners.
    const CONE: f64 = 15.0;
    let plain_far = far_toward(&plain, SIZE, mark.angle, CONE);
    let marked_far = far_toward(&marked, SIZE, mark.angle, CONE);
    let plain_back = far_toward(&plain, SIZE, away, CONE);
    let marked_back = far_toward(&marked, SIZE, away, CONE);
    assert!(plain_far > 0.0 && marked_far > 0.0, "no clearing to measure");

    // Every length below is in pixels, so the picture calibrates itself: the
    // unmarked hole IS the rings' edge one reach out, and that is a uv the
    // stack states.
    let scale = plain_far / (rings.outer + CLEAR_REACH) as f64;
    let want = (mark_out - rings.outer) as f64 * scale;
    eprintln!(
        "toward the mark {plain_far:.1} -> {marked_far:.1} px, away {plain_back:.1} -> \
         {marked_back:.1} px; the strip is {want:.1} px at {scale:.1} px/uv",
    );
    assert!(
        (marked_far - plain_far - want).abs() < 2.0,
        "the mark pushed the hole out {:.1} px over its own wedge, not the {want:.1} px \
         its strip stands past the rings",
        marked_far - plain_far,
    );
    // The other half of the claim, and the one the circle fails: a mark on one
    // octave is not a wider node.
    assert!(
        (marked_back - plain_back).abs() < 1.5,
        "the hole is {:.1} px wider away from the mark than the unmarked node's, \
         so the mark widened the clearing all the way round",
        marked_back - plain_back,
    );
}

/// A mark's wedge can run past a HALF turn, and `sector_distance` has to stay
/// exact when it does.
///
/// `MIN_SPAN` rules out a slice that is a whole turn and nothing narrower, so a
/// wheel of one full-size octave between two extras at their minimum cuts a
/// slice most of the way round — a shape the Octaves bar can be dialled into.
/// Past a half-aperture of pi/2 the sector's two edge half-planes stop
/// intersecting in front of the wedge and start intersecting behind it, which
/// is the case a naive `max` of two half-planes gets wrong: it would take the
/// clearing back to the rings across the far side of a wedge that covers it.
///
/// Measured off the wedge's own middle rather than a fixed angle, so it does
/// not matter which slot the wheel made the wide one. Only the covering half of
/// the claim is made here: the extras leave a gap narrower than twice the
/// reach, so a hole this wide legitimately has no direction left in which it
/// hugs the rings. That half is
/// `a_clearing_bulges_over_the_mark_and_hugs_the_rings_everywhere_else`, on a
/// wheel whose slices are 55 degrees.
#[test]
fn a_clearing_over_a_wedge_past_a_half_turn_covers_the_whole_wedge() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let rings = clearing_rings();
    let mark_out = rings.mark_inner + rings.mark_thickness;

    // One full-size octave and two extras at MIN_EXTRA_SIZE, so the full one
    // takes 1/(1 + 2*0.1) of the turn.
    let wheel =
        harmonigraph_scene::octave_layout(1, 60.0, 1, harmonigraph_scene::MIN_EXTRA_SIZE, 1.0);
    let widest = (1..=wheel.span)
        .map(|j| wheel.bounds[j as usize] - wheel.bounds[j as usize - 1])
        .fold(0.0f32, f32::max);
    eprintln!(
        "span {} of {} full + {} extras, widest slice {:.1} deg",
        wheel.span,
        wheel.count,
        wheel.extras,
        widest.to_degrees(),
    );
    assert!(
        widest > std::f32::consts::PI,
        "the fixture has to cut a slice past a half turn to be testing anything; \
         the widest is {:.1} deg",
        widest.to_degrees(),
    );
    let lopsided = |melody: u32, gutter: f32| -> Scene {
        let mut scene = clearing_node(melody, 1.0, true, gutter);
        scene.octave_layout = wheel;
        scene
    };

    let bare_plain = gpu.shot(&lopsided(0, 0.0));
    let holed_plain = gpu.shot(&lopsided(0, CLEAR_REACH));
    let bare_marked = gpu.shot(&lopsided(MIDDLE_C, 0.0));
    let holed_marked = gpu.shot(&lopsided(MIDDLE_C, CLEAR_REACH));

    // Which way the wide wedge points, taken off the picture as the marked
    // node's own extra ink.
    let mark = light_about_center(&light_over(&bare_marked, &bare_plain), SIZE);
    assert!(mark.weight > 0.0, "the mark drew nothing to aim at");

    let plain = light_over(&holed_plain, &bare_plain);
    let marked = light_over(&holed_marked, &bare_marked);
    // Narrow, because the two extras between them hold only what the full
    // slice leaves and the far reading has to stay inside that.
    const CONE: f64 = 8.0;
    let scale = far_toward(&plain, SIZE, mark.angle, CONE) / (rings.outer + CLEAR_REACH) as f64;
    let strip = (mark_out - rings.outer) as f64 * scale;

    // The wedge's middle, then a quarter turn off it, then most of the way out
    // to its edge — the last two past the half-aperture where a wedge stops
    // being an intersection of two half-planes in front of itself.
    for turn in [0.0_f64, 90.0, 150.0] {
        let toward = mark.angle + turn.to_radians();
        let grew = far_toward(&marked, SIZE, toward, CONE) - far_toward(&plain, SIZE, toward, CONE);
        eprintln!(
            "{turn:.0} deg off the wedge's middle: the hole grew {grew:.1} px, want {strip:.1}",
        );
        assert!(
            (grew - strip).abs() < 2.0,
            "{turn:.0} degrees off the middle of a {:.0}-degree wedge the hole grew {grew:.1} px, \
             not the {strip:.1} px its strip stands past the rings — the wedge does not \
             reach its own edge",
            widest.to_degrees(),
        );
    }
}

/// The clearing is one HOLE the node sits in — filled in to its centre, across
/// the gaps between its rings and between one sector and the next — rather than
/// a stencil of the node's ink.
///
/// Both halves matter and they fail differently. A clearing that followed the
/// ink would leave the lattice showing through every gap on the node, which
/// reads as neither a hole nor a node; and the node whose rings leave the widest
/// hole in the middle is the one with no core at all, where the ink is an
/// annulus and its middle is the marker standing under it.
///
/// Read along rays out of the node's centre, which is the one sweep that does
/// not need to know where the rim is in each direction — and the marked node's
/// rim is a different radius over its mark than beside it. Everything from the
/// centre out to whatever the ray last found is the node or the ground; a dark
/// sample with light beyond it is a hole in the hole. (The shape is a union of
/// parts that all reach the centre, and dilating one of those leaves it reaching
/// the centre, so "no gap along any ray" is exactly the claim.)
#[test]
fn a_clearing_is_one_hole_covering_the_centre_and_every_ring() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    let staged = |melody: u32, band: bool, ring: f32, gutter: f32| -> Scene {
        let mut scene = clearing_node(melody, ring, band, gutter);
        // No markers. The ray below reads the picture for anything lit, and a marker
        // sitting just outside the hole is a lit sample past its rim with
        // bare pane between — which reads as a gap in the hole and is nothing
        // of the kind. What the clearing cuts out of the marker field is a
        // separate claim, and `a_node_wearing_only_an_audio_ring_clears_around_it`
        // is where the added light is measured against it.
        scene.pluses.clear();
        scene
    };
    // Everything a node can wear, and then the same node with its audio ring
    // gated shut — which leaves the octave band an annulus with the lattice
    // showing through the middle of it, the case the fill-to-the-centre rule
    // is for.
    for (name, melody, band, ring) in [
        ("every layer", MIDDLE_C, true, 1.0),
        ("the octave band alone", 0, true, 0.0),
    ] {
        let bare = gpu.shot(&staged(melody, band, ring, 0.0));
        let holed = gpu.shot(&staged(melody, band, ring, CLEAR_REACH));
        let reach = light_about_center(&light_over(&holed, &bare), SIZE).far;
        assert!(reach > 8.0, "{name}: no clearing to read, {reach:.1} px");

        let (cx, cy) = ((SIZE[0] - 1) as f64 / 2.0, (SIZE[1] - 1) as f64 / 2.0);
        // Half-pixel steps: a step of a whole one can straddle the rim and read
        // a gap the picture does not have.
        let steps = (reach * 2.0).ceil() as usize;
        let lit = |r: f64, a: f64| -> bool {
            let x = (cx + r * a.cos()).round();
            let y = (cy + r * a.sin()).round();
            let i = (y as usize * SIZE[0] as usize + x as usize) * 4;
            brightness(&holed[i..i + 4]) >= 24
        };
        for turn in 0..360 {
            let a = (turn as f64).to_radians();
            let Some(rim) = (0..=steps).rev().map(|s| s as f64 / 2.0).find(|&r| lit(r, a)) else {
                continue;
            };
            // Stopping two pixels short of the rim, which is the hole's own
            // anti-aliased edge: out there a lobe's angular boundary and the
            // rounding of a sample to a pixel can disagree, so a single lit
            // sample sits past a dark one and reads as a gap the picture does
            // not have. Everything a gap in the hole would actually be is
            // inside this.
            let gap = (0..=((rim - 2.0).max(0.0) * 2.0) as usize)
                .map(|s| s as f64 / 2.0)
                .find(|&r| !lit(r, a));
            assert!(
                gap.is_none(),
                "{name}: at {turn} degrees the picture is dark {:.1} px out and lit \
                 again at {rim:.1} px — the clearing has a hole in it",
                gap.unwrap_or_default(),
            );
        }
    }
}

/// Mean added light over the pixels between `lo` and `hi` from the centre of the
/// frame — how STRONGLY a ring of the clearing is cleared, where
/// [`far_toward`] and [`Light::far`] answer how far it reaches.
fn light_in_band(weights: &[f64], size: [u32; 2], lo: f64, hi: f64) -> f64 {
    let (cx, cy) = ((size[0] - 1) as f64 / 2.0, (size[1] - 1) as f64 / 2.0);
    let (mut sum, mut n) = (0.0, 0usize);
    for (i, &w) in weights.iter().enumerate() {
        let x = (i % size[0] as usize) as f64 - cx;
        let y = (i / size[0] as usize) as f64 - cy;
        let r = x.hypot(y);
        if r >= lo && r <= hi {
            sum += w;
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { sum / n as f64 }
}

/// The audio ring is worn node by node, so its part of the clearing is too: a
/// node the Gate has closed clears only the core it is left with, and one part
/// way through its fade clears the ring's WHOLE hole at part of its strength.
///
/// Width from the layer, level from the layer's own fade — the same division the
/// note's clearing has always run on, now per layer. The two halves are separate
/// claims and they fail differently. A hole sized by the fade would sweep
/// outward across the lattice as a ring arrives, which is the "node retreating"
/// look the reach is deliberately held against; a hole at full strength from the
/// first frame would pop.
#[test]
fn a_clearing_follows_the_audio_ring_its_node_wears() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let rings = clearing_rings();

    // The band off, so the audio ring is the layer the stack's cursor landed on
    // and this node's own gate is what its hole answers to.
    let hole = |gpu: &mut Shooter, ring: f32| -> (Vec<f64>, f64) {
        let bare = gpu.shot(&clearing_node(0, ring, false, 0.0));
        let holed = gpu.shot(&clearing_node(0, ring, false, CLEAR_REACH));
        let cleared = light_over(&holed, &bare);
        let far = light_about_center(&cleared, SIZE).far;
        (cleared, far)
    };
    let (worn, worn_far) = hole(&mut gpu, 1.0);
    let (closed, closed_far) = hole(&mut gpu, 0.0);
    let (half, half_far) = hole(&mut gpu, 0.5);

    let scale = worn_far / (rings.audio.1 + CLEAR_REACH) as f64;
    eprintln!(
        "ring worn {worn_far:.1} px, closed {closed_far:.1}, half {half_far:.1}, \
         at {scale:.1} px/uv",
    );
    // The band is off and the note is silent, so a node the gate closed is
    // wearing nothing at all and has nothing to clear. That is the per-layer
    // split at its limit: a hole sized to what the VIEW has on would still cut
    // a ring-sized gap here, around ink nobody drew.
    assert!(
        closed_far < 2.0,
        "a node the gate closed still clears {closed_far:.1} px, with no layer on it",
    );
    assert!(
        (half_far - worn_far).abs() < 2.0,
        "a ring half way in clears {half_far:.1} px where a whole one clears \
         {worn_far:.1} — the hole is sized by the fade instead of by the layer",
    );

    // Read where only the ring's own clearing lands: outside the ring's ink, so
    // the node paints nothing there and the added light IS the hole, and inside
    // the reach, so a hard-edged clearing covers all of it.
    let (lo, hi) = (rings.audio.1 as f64 * scale + 3.0, worn_far - 3.0);
    let (lit, dim, none) = (
        light_in_band(&worn, SIZE, lo, hi),
        light_in_band(&half, SIZE, lo, hi),
        light_in_band(&closed, SIZE, lo, hi),
    );
    eprintln!("past the ring, {lo:.1}..{hi:.1} px: worn {lit:.0}, half {dim:.0}, closed {none:.0}");
    assert!(
        lit > 0.0 && (dim / lit - 0.5).abs() < 0.05,
        "half a ring cleared {dim:.0} of the {lit:.0} a whole one does",
    );
    assert!(none < lit * 0.02, "a closed gate cleared {none:.0} past a ring it is not wearing");
}

/// A node wearing NOTHING BUT an audio ring clears around it — the case the
/// whole per-layer split is for.
///
/// The ring is a window onto the spectrum rather than a level a node carries, so
/// a node nobody played wears one wherever the view's Gate lets it. That is ink,
/// and ink with no hole under it reads as painted ON the lattice rather than in
/// front of it: the marker under it shows through the ring, and so do the
/// sheets behind.
///
/// The other half is what such a node must NOT clear. Its band and its core are
/// drawn at the note's level, which is nothing, so a hole sized to the layers
/// the VIEW has on would clear a band-sized gap around ring-sized ink — the
/// same "wider than the node" failure the marks had, arrived at from the other
/// direction.
#[test]
fn a_node_wearing_only_an_audio_ring_clears_around_it() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    let rings = clearing_rings();

    // Silent: no note, no octaves, no marks. What is left is the ring the gate
    // hands it, at `ring`, and the band is ON so the "clears what the view draws
    // rather than what this node draws" failure has room to show.
    let silent = |ring: f32, gutter: f32| -> Scene {
        let mut scene = clearing_node(0, ring, true, gutter);
        let node = &mut scene.nodes[0];
        node.activation = 0.0;
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        scene
    };
    let bare = gpu.shot(&silent(1.0, 0.0));
    let holed = gpu.shot(&silent(1.0, CLEAR_REACH));
    let cleared = light_over(&holed, &bare);
    let hole = light_about_center(&cleared, SIZE);
    assert!(hole.weight > 0.0, "a node wearing an audio ring cleared nothing at all");

    // Calibrated on a node that IS played, where the hole is the band's and the
    // band's outer edge is a uv the stack states.
    let played_bare = gpu.shot(&clearing_node(0, 1.0, true, 0.0));
    let played = gpu.shot(&clearing_node(0, 1.0, true, CLEAR_REACH));
    let played_far = light_about_center(&light_over(&played, &played_bare), SIZE).far;
    let scale = played_far / (rings.band.1 + CLEAR_REACH) as f64;
    let want = (rings.audio.1 + CLEAR_REACH) as f64 * scale;
    eprintln!(
        "ring alone clears {:.1} px (want {want:.1}), a played node {played_far:.1} \
         (band {:.1}), at {scale:.1} px/uv",
        hole.far,
        (rings.band.1 + CLEAR_REACH) as f64 * scale,
    );
    assert!(
        (hole.far - want).abs() < 2.0,
        "a node wearing only its ring cleared {:.1} px, not the {want:.1} px that ring \
         reaches — a band nobody is drawing is in the hole",
        hole.far,
    );

    // ...and with the gate closed there is no ink and nothing to clear around.
    // That half is the CULL's, and it is asked of the cull directly: a node
    // with no note, no marks and no ring ships no instance, so its reach never
    // reaches the shader. Two shots would prove nothing here — the node is gone
    // from both, and comparing two empty images passes whatever the shader
    // does. The shader's own half of the answer, a closed gate falling back to
    // the layers that ARE drawn, is measured in
    // `a_clearing_follows_the_audio_ring_its_node_wears`.
    let quiet = LatticeCallback::from_scene(
        &silent(0.0, CLEAR_REACH),
        LatticeLabels::default(),
        egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
        wgpu::TextureFormat::Rgba8Unorm,
        31,
        None,
    );
    assert!(
        quiet.instances.is_empty(),
        "a node with no note and no ring still shipped, carrying a reach that would \
         clear a hole around ink nobody draws",
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
        scene.mark_thickness = 0.0;
        scene.rings_outer = 0.0;
        scene.octave_gap = 0.0;
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
    let fresh_range = PROBE_RANGE;
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
/// the picture: no audio ring and no mark rings, so the only thing drawn is the
/// band, and a wide gap so the seams between indicators are several pixels
/// across at the size this renders at.
fn octave_wheel_scene(layout: harmonigraph_scene::OctaveLayout, cents: f32) -> Scene {
    let mut scene = single_marked_node(0, 0);
    scene.octave_layout = layout;
    scene.outer_inner = 0.30;
    scene.outer_outer = 0.95;
    scene.rings_outer = 0.95;
    scene.octave_gap = 0.10;
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
/// indicators the only unlit stretches are the Octave gap's slits, one per
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
/// are the Octave gap's slits, one per boundary, and the seam is one of them
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
    // enough to be eaten by the Octave gap.
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
                // rather than a missing indicator: the Octave gap is cut out of every
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
                         {lost} extras can be lost to the Octave gap"
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
        // leaves the extras narrower than the Octave gap's slits, and a centroid
        // needs an arc to measure. That the edges reach the seam at all is
        // `every_octave_in_the_range_is_drawn_and_they_close_the_ring`.
        for (cents, offset) in [(0.0f32, 0i32), (700.0, 0), (0.0, 2), (700.0, 2)] {
            let (first, last) = layout.slots(cents);
            let slot =
                (harmonigraph_scene::MIDDLE_C_SLOT as i32 + offset).clamp(first + 1, last - 1);
            let mut scene = octave_wheel_scene(layout, cents);
            // One octave sounding. The silent slots still carry the ring's
            // ground behind it, which the brightness threshold below sorts
            // out.
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
/// `level > 0` switch — is caught below, and knowing WHICH check catches it is
/// worth stating. The ghost is the rings' own grey (`Scene::lattice_ground`),
/// and here it is darker than anything this fixture paints over it: every
/// `pitch_lut` entry adds up to 1.4 across its three channels, against the
/// fresh Ground's grey at 0.57. So the switch's last frame is a step DOWN in
/// light, the never-brightens loop passes the whole way, and the TAIL-SPREAD
/// check is the one that fires — the slice holds its pitch to the last lit
/// frame and then makes the entire journey to the ground in one. The loop
/// takes the fault first only where the ground is the BRIGHTER of the two,
/// which is a Ground bar away rather than a shader change, so the spread check
/// is the one to read this test by. The last check is neither: at level 0 both
/// shaders run the same line, so it can only say the finished ring is one
/// backdrop, not how it got there.
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
    // And the last stretch of it — the bottom sixth of the envelope, where a
    // slice is nearly the ground already — is SPREAD across the frames rather
    // than spent in one. Painted in place of the ground instead of mixed
    // toward it, the slice sits still for that whole stretch and then makes
    // the entire journey in one frame.
    //
    // A share of the travel rather than a run of strict decreases: a ramp
    // this shallow moves the last few frames by less than an 8-bit channel,
    // and the pair either side of zero reads identically here BECAUSE the
    // handoff is smooth. Where the cut falls is not the claim — a different
    // one only widens or narrows the stretch measured, so this reads the
    // sweep rather than asserting a level.
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
    // Landing on the ground the silent slices are drawn in — the same grey at
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

/// The ground reaches the shader as a UNIFORM, and the picture has to track
/// it: a silent slice wears the grey `Scene::lattice_ground` carries, whatever
/// that is, rather than one grey baked into the shader.
///
/// Every other fixture here draws at the fresh Ground alone, and the grey that
/// names is `vec3(0.189)` — near enough a plausible literal that a shader
/// ignoring `u.lattice_ground` entirely, or reading the ground out of the
/// wrong vec4 of the uniform block, would render all of them pixel for pixel.
/// So this one draws one node four times across the bar, the fresh 20 first
/// and then a near-black, a mid grey and a near-white.
///
/// Full presence against a slot at level 0 is where the arithmetic leaves
/// nothing to interpret: a silent slice's opacity IS the node's presence, so
/// at 1.0 the wedge is the ground colour undiluted, and the byte is that
/// colour at 8 bits. The LIT slot beside it is read from the same shot and has
/// to stay put — at full level the ghost is nothing, so a sounding pitch owes
/// the ground no part of its colour, and a ground that moved it would be the
/// mix leaking into the one place it must not reach.
///
/// A channel and a half, tighter than the 2.5 the fade probes above allow, and
/// honestly so: those read points ON an envelope, where the level's own 8-bit
/// packing is inside the measurement, and nothing here fades. One flat colour
/// into an 8-bit target rounds by half a channel and by nothing else, and the
/// closest pair of grounds measured lands 29 channels apart — twenty times the
/// tolerance — so nothing here passes by being loose.
#[test]
fn a_silent_slice_wears_the_ground_the_scene_names() {
    use harmonigraph_scene::{grey_of_lightness, octave_layout};

    const SIZE: [u32; 2] = [384, 384];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };
    // The even five-octave wheel the release tests use: a 72-degree slice is
    // room to sample well inside one and well inside its neighbour.
    let layout = octave_layout(5, 60.0, 0, 1.0, 0.0);
    let lit = harmonigraph_scene::MIDDLE_C_SLOT;
    let quiet = lit + 1;
    // Both inside the ring this wheel draws: `sector` CLAMPS a slot outside it
    // rather than refusing, which would leave the two readings below taken on
    // one wedge and agreeing for a reason that has nothing to do with the
    // ground.
    let (low, high) = layout.slots(0.0);
    for slot in [lit, quiet] {
        assert!((low..=high).contains(&(slot as i32)), "slot {slot} is outside {low}..={high}");
    }
    let scene_at = |ground: f32| {
        let mut scene = octave_wheel_scene(layout, 0.0);
        scene.lattice_ground = grey_of_lightness(ground);
        let node = &mut scene.nodes[0];
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.octaves[lit] = 1.0;
        // The note fully down, so every silent slice on the ring is opaque and
        // the wedge below reads the ground rather than a share of it.
        node.activation = 1.0;
        scene
    };

    let (lit_mid, lit_wedge) = wedge_of(layout, lit, 0.0);
    let (quiet_mid, quiet_wedge) = wedge_of(layout, quiet, 0.0);
    let mut pitch: Option<[f32; 3]> = None;
    for ground in [20.0f32, 6.0, 45.0, 80.0] {
        let px = gpu.shot(&scene_at(ground));
        // Calibrated along the SOUNDING slice, which is the one ray that is
        // bright at every ground: a band drawn in a near-black ground is the
        // dark end of the bar, and the radii are the scene's either way.
        let probe = BandProbe::new(&px, SIZE, lit_mid);
        let got = probe.mean(&px, quiet_mid, quiet_wedge);
        let want = grey_of_lightness(ground).truncate() * 255.0;
        for j in 0..3 {
            assert!(
                (got[j] - want[j]).abs() < 1.5,
                "at Ground {ground} the silent slice reads {got:?}, not {want:?}"
            );
        }
        let sounding = probe.mean(&px, lit_mid, lit_wedge);
        match pitch {
            None => pitch = Some(sounding),
            Some(first) => {
                for j in 0..3 {
                    assert!(
                        (sounding[j] - first[j]).abs() < 1.5,
                        "at Ground {ground} the lit slice reads {sounding:?}, and at the \
                         first ground it read {first:?}"
                    );
                }
            }
        }
    }
}

/// The seams between a chord's colors run at ONE width from the edge of a
/// node's light to its middle. They are laid down as lobes of fixed ANGULAR
/// width, so the arc each spans shrinks with the radius and they would
/// otherwise converge to a cusp at the node's centre — sharpest exactly where
/// the node has the fewest pixels to say it with.
///
/// Both halves of the bargain, because either alone has a trivial cheat: the
/// centre has to lose its seam, AND the outside has to keep its colors, which
/// is what stops the cure from being "average the whole halo".
///
/// The node's light is the only place this can be read. It is the one thing a
/// node draws whose colour is laid in ANGLE at every radius at once — every
/// ring paints its own annulus and nothing inside it — so the cusp is the
/// glow's own to have, and `glow_layer`'s ease toward the strip's mean is the
/// cure being measured.
///
/// Measured as how far the colors around a ring point APART as directions, not
/// as how much they differ: the light dims inward under the Centre dip and
/// outward under its falloff, and any measure of magnitude would read that
/// dimming as a blur and pass on it.
#[test]
fn the_lights_colour_seams_run_at_one_width_from_its_edge_to_the_centre() {
    const SIZE: [u32; 2] = [512, 512];
    // Inside the node's own middle, where the light runs in to the centre with
    // nothing standing it off, and out past every ring the node draws, where
    // the light is all there is. Both are pure light: the node's ink is an
    // annulus between them (the octave band, 80..120 px at this node size), and
    // a reading taken on it would be the band's colour rather than the halo's.
    const INNER: f32 = 20.0;
    const OUTER: f32 = 170.0;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };

    let mut scene = single_marked_node(0, 0);
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
    // The octave band is the ink the light's colour is read off, and it is the
    // only layer on: nothing is drawn inside it, so every pixel sampled below
    // is light and nothing else.
    scene.mark_thickness = 0.0;
    scene.spectral = harmonigraph_scene::SpectralPaint::silent();
    scene.node_radius = 1.6;
    scene.glow_reach = 0.8;
    scene.glow_strength = 1.5;
    // Each octave's hue kept as its own arc rather than averaged round the
    // halo, which is the state a seam exists in at all.
    scene.glow_blend = 0.0;
    let px = shooter.shot(&scene);

    // The node is alone at the world origin and the camera looks at it, so the
    // frame's centre is its centre.
    let c = (SIZE[0] / 2) as i32;
    let rgb = |x: i32, y: i32| -> glam::Vec3 {
        let i = ((y as u32 * SIZE[0] + x as u32) * 4) as usize;
        glam::Vec3::new(px[i] as f32, px[i + 1] as f32, px[i + 2] as f32) / 255.0
    };
    // How far apart, in degrees, the most divergent pair of colors around a
    // ring of radius `r` point. Zero is one flat color all the way round.
    let spread = |r: f32| -> f32 {
        let dirs: Vec<glam::Vec3> = (0..64)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / 64.0;
                // Screen y grows downward; the sample angle is negated.
                rgb(c + (r * a.cos()).round() as i32, c - (r * a.sin()).round() as i32)
            })
            .filter(|v| v.length() > 0.02)
            .map(|v| v.normalize())
            .collect();
        let lit = dirs.len();
        assert!(lit > 56, "the ring at r={r:.0} is not lit: {lit} lit samples of 64");
        let mut worst = 0.0f32;
        for (i, a) in dirs.iter().enumerate() {
            for b in &dirs[i + 1..] {
                worst = worst.max(a.dot(*b).clamp(-1.0, 1.0).acos().to_degrees());
            }
        }
        worst
    };

    let (at_centre, at_edge) = (spread(INNER), spread(OUTER));
    eprintln!("seams: {at_centre:.0} deg at {INNER} px, {at_edge:.0} at {OUTER}");
    // No cusp: the middle is a blend rather than the point where every seam
    // meets. This is what fails if the mix toward the strip's mean goes away
    // and the light is laid at one fixed concentration — the centre then reads
    // as separated as the edge.
    assert!(
        at_centre < at_edge * 0.5,
        "the seams still converge — {at_centre:.0} deg across the centre against \
         {at_edge:.0} further out",
    );
    // And what stops the cure being "average the node": the seams are never
    // held wider than the arc they already span out where the ink is, so the
    // node still shows its notes as distinct colors.
    // The arc is the strip's own — the light is a BLEND of the chord's hues by
    // design, and out where the mix reaches the strip in full it spans the arc
    // GLOW_LOBE_KAPPA gives it. What this rules out is that arc collapsing to
    // the flat mean everywhere, which is what "average the node" looks like.
    assert!(
        at_edge > 15.0,
        "the colors washed out instead of their seams widening — only {at_edge:.0} deg \
         across the outer ring",
    );
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
/// markers and the labels both arrive already built. They stay because a
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
    // which keeps an otherwise idle node's fragments where the ring is and out
    // to the edge of the hole that ring clears.
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
        // ...and a reach, which is the whole point of it: the idle branch keeps
        // an idle node's fragments out to the edge of the hole its ring clears,
        // and `parity_scene` hands its gutters to the off-sheet half — node 0,
        // which this is a copy of, carries none. Without this the branch is
        // compiled on both pipelines and never once decides anything, so the
        // one spelling of `clearing_edge` it shares with the coverage is
        // untested where a second spelling would show.
        silent.gutter = 0.12;
        silent.world_pos.x += 0.9;
        scene.nodes.push(silent);
        scene
    };
    // A clearing around a MARKED node, which is `node_clearing`'s own
    // skip: the clearing's shape is the rings' disc unioned with one wedge per
    // mark, and inside that disc the walk over the wedges is skipped as an
    // answer already arrived at. `parity_scene` hands its gutters and its marks
    // to different halves of its nodes — the off-sheet ones clear, the marked
    // ones are on the home sheet — so without this the two never meet on one
    // node and the skip is never taken.
    let clearing = || {
        let mut scene = parity_scene();
        for node in &mut scene.nodes {
            node.gutter = 0.16;
        }
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
    // The STANDOFF around those same marked nodes, which is `glow_standoff`'s own
    // skip — the same one as `node_clearing`'s, over the same wedges, taken
    // once the band has held the pixel's light off in full. The REACH is what
    // puts it in the comparison at all, and no fixture above has one: the Gap
    // dials ride to the GPU only while the light does (`misc11` is zeroed at
    // reach 0), and there is no light draw to compare without a glow target
    // for it to write into. Both are the same switch, which is why one line
    // buys the fixture.
    let standing_off = || {
        let mut scene = clearing();
        scene.glow_reach = 0.8;
        // One node with INK and no light of its own, which is the other half of
        // `fs_glow`'s early-out: it weighs the standoff's answer as well as the
        // light's, and a fixture whose every node is lit decides it on the
        // light's alone — a relaxation that dropped the standoff's term would
        // then discard this node's band and nothing here would see it. Shipped
        // for its ink (`paints`), it draws its own layers and stands the light
        // off them while emitting none.
        let mut dark = scene.nodes[0].clone();
        dark.glow.level = 0.0;
        dark.glow.row = scene.glow_rows;
        dark.world_pos.x -= 0.9;
        scene.glow_rows += 1;
        scene.nodes.push(dark);
        scene
    };
    // ...and that standoff with the ring CLOSED, which is `slice_gap_distance`'s
    // own skip: with the slices meeting edge to edge there is no gap to walk
    // the boundaries of, and the ring's plain radial distance has to be the
    // answer on both pipelines. Every fixture above carries a gap, so without
    // this the branch is compiled and never once decides anything.
    let closed_ring = || {
        let mut scene = standing_off();
        scene.octave_gap = 0.0;
        scene
    };
    // No all-idle fixture: an idle node paints nothing, so the cull ships
    // none of them and the comparison would be two empty images. What the
    // idle branch does is now pinned by
    // `a_silent_lattice_ships_no_nodes_and_still_draws_its_grid` instead,
    // on the CPU side where the decision actually lives.
    for (name, scene) in [
        ("lit", parity_scene()),
        ("shimmering", shimmering()),
        ("ringing", ringing()),
        ("folded", folded()),
        ("clearing", clearing()),
        ("standing off", standing_off()),
        ("standing off a closed ring", closed_ring()),
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
        let layouts =
            SceneLayouts { uniforms: &res.bind_group_layout, glow: &res.glow_layout };
        let build = |src: &str| create_pipelines(&device, src, format, layouts, false);
        let (fast, _) = build(SHADER_SRC);
        let (slow, _) = build(&reference_src);
        // The light at group 1: one colour over the whole frame, bound to both
        // pipelines, so a clearing reads the same thing back whichever is
        // drawing and what they differ by is the early-outs alone. A constant
        // rather than the 1x1 stand-in because `node_paint` reads its ground
        // and its Wash OUT of this, and a transparent nothing leaves both terms
        // at zero on either pipeline — which takes the whole of what a clearing
        // does with the light out of the comparison. Premultiplied, as the real
        // target is, and well under opaque so the ground still shows through it.
        let light = {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("parity_light"),
                size: wgpu::Extent3d { width: SIZE[0], height: SIZE[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let texel = [96u8, 64, 32, 128];
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &texel.repeat((SIZE[0] * SIZE[1]) as usize),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE[0] * 4),
                    rows_per_image: Some(SIZE[1]),
                },
                wgpu::Extent3d { width: SIZE[0], height: SIZE[1], depth_or_array_layers: 1 },
            );
            let view = texture.create_view(&Default::default());
            // Never written, and RENDER_ATTACHMENT is what lets wgpu zero it:
            // a coverage of zero is the light kept whole. Full size rather than
            // 1x1 because `node_paint` clamps its read against the LIGHT's
            // dimensions, so a smaller layer beside it would be read out of
            // bounds.
            let shade = device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("parity_shade"),
                    size: wgpu::Extent3d {
                        width: SIZE[0],
                        height: SIZE[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: GLOW_SHADE_FORMAT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default());
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("parity_light_bind_group"),
                layout: &res.glow_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&res.sampler),
                    },
                    // The one texture at both of the light's slots, so the
                    // Meld mixes a colour with itself and hands back that
                    // colour at every setting: what this fixture holds still
                    // is the light a clearing reads, and the bar is not what
                    // it is comparing.
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    // A standoff of nothing, so the light above reaches these
                    // pipelines whole. What the standoff DOES is compared
                    // below, in the pass that now writes it; here it would only
                    // dim the constant this fixture is holding still.
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&shade),
                    },
                ],
            })
        };
        let light = &light;
        let pane = res.panes.get(&11).expect("prepare created the pane");

        let clear = wgpu::Color { r: 0.07, g: 0.08, b: 0.09, a: 1.0 };
        let draw = |pipeline: &wgpu::RenderPipeline| {
            let texture = render_to_texture(&device, &queue, SIZE, format, clear, |pass| {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_bind_group(1, light, &[]);
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

        // The LIGHT's own draw, compared the same way and for the same reason.
        // Three of its early-outs are only reachable here: `fs_glow`'s own,
        // which weighs the standoff's answer as well as the light's now that
        // one fragment carries both; `glow_standoff`'s skip inside it; and
        // `slice_gap_distance`'s. `node_paint` reaches none of them — it reads
        // the standoff back off a texture rather than computing it — so without
        // this the three would be compiled over that code and never once
        // compared.
        //
        // The SHADE attachment is what is read back, being the one the standoff
        // writes: a skip that dropped a band would leave the light beside it
        // identical and show only here.
        let Some(glow) = pane.offscreen.as_ref().and_then(|o| o.glow.as_ref()) else {
            continue;
        };
        let glow_draw = |src: &str| {
            let pipeline = create_glow_pipeline(
                &device,
                src,
                format,
                &res.bind_group_layout,
                &res.strip_layout,
            );
            let attachment = |label, format| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: SIZE[0],
                        height: SIZE[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            };
            let targets = [
                attachment("parity_glow", format),
                attachment("parity_glow_max", format),
                attachment("parity_glow_shade", GLOW_SHADE_FORMAT),
            ];
            let views: Vec<_> = targets.iter().map(|t| t.create_view(&Default::default())).collect();
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("parity_glow_pass"),
                    color_attachments: &views
                        .iter()
                        .map(|view| {
                            Some(wgpu::RenderPassColorAttachment {
                                view,
                                depth_slice: None,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    store: wgpu::StoreOp::Store,
                                },
                            })
                        })
                        .collect::<Vec<_>>(),
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &pane.bind_group, &[]);
                pass.set_bind_group(1, &glow.strip.blurred_bind_group, &[]);
                pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
                pass.draw(0..4, 0..pane.instance_count);
            }
            queue.submit([encoder.finish()]);
            // BOTH quantities the pass writes, because a skip can drop either
            // one alone: an early-out that went on the light's answer where it
            // should go on both leaves the standoff's layer untouched, and one
            // that went on the standoff's leaves the LIGHT's. The `max`-blended
            // light is left out of the readback and not out of the comparison —
            // one fragment emits it and the screened attachment together, so a
            // dropped fragment shows in this one.
            //
            // `readback`'s copy with its one assumption widened: a 256-wide row
            // is 1024 bytes of light or 512 of coverage, and both are aligned.
            let read = |target: &wgpu::Texture, bytes: u32| {
                let bytes_per_row = SIZE[0] * bytes;
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("parity_glow_readback"),
                    size: (bytes_per_row * SIZE[1]) as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                });
                let mut encoder = device.create_command_encoder(&Default::default());
                encoder.copy_texture_to_buffer(
                    target.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(bytes_per_row),
                            rows_per_image: None,
                        },
                    },
                    wgpu::Extent3d { width: SIZE[0], height: SIZE[1], depth_or_array_layers: 1 },
                );
                queue.submit([encoder.finish()]);
                let slice = buffer.slice(..);
                slice.map_async(wgpu::MapMode::Read, |r| r.expect("map readback buffer"));
                device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
                slice.get_mapped_range().to_vec()
            };
            (read(&targets[0], 4), read(&targets[2], 2))
        };
        let (light_fast, shade_fast) = glow_draw(SHADER_SRC);
        let (light_slow, shade_slow) = glow_draw(&reference_src);

        // Vacuous unless the pass actually wrote each of them: every fixture
        // that reaches here carries a reach and a depth, so a layer of zeroes
        // means the dials stopped arriving rather than that the skips are sound.
        assert!(
            shade_slow.iter().any(|&b| b != 0),
            "the {name} scene held no light off; the standoff comparison is vacuous",
        );
        assert!(
            light_slow.iter().any(|&b| b != 0),
            "the {name} scene lit nothing; the light comparison is vacuous",
        );

        for (layer, fast, slow) in
            [("light", &light_fast, &light_slow), ("standoff", &shade_fast, &shade_slow)]
        {
            let differing = fast.iter().zip(slow.iter()).enumerate().find(|(_, (a, b))| a != b);
            assert!(
                differing.is_none(),
                "the {name} scene's {layer} changed when the early-outs were enabled: \
                 byte {:?}",
                differing.map(|(i, (a, b))| (i, *a, *b)),
            );
        }
    }
}

/// A node that can paint nothing is not shipped at all — and the marker standing
/// at its position still is.
///
/// The billboard is deliberately bigger than the node, so a node the shader
/// discards every fragment of still costs a quad's worth of rasterizing; on
/// an unplayed lattice that is EVERY node, an idle one drawing nothing of its
/// own and carrying no trail mark. So the frame drops to the marker field and
/// nothing else, and the callback has to keep drawing that field — which is
/// why neither `prepare` nor `paint` may read "no instances" as "nothing to
/// draw": that test takes the markers down with the nodes.
#[test]
fn a_silent_lattice_ships_no_nodes_and_still_draws_its_markers() {
    let scene = idle_scene();
    assert!(!scene.pluses.is_empty(), "the fixture has to carry a marker field");
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
    assert!(!cb.pluses.is_empty(), "a marker is not a node and must survive");

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

/// One marker alone on a black field, at the size an area measurement wants:
/// no nodes, so nothing composites over it, and nothing else in the shot can
/// be mistaken for its ink.
fn lone_marker_scene(half_width: f32, taper_start: f32) -> Scene {
    let mut scene = idle_scene();
    scene.nodes.clear();
    scene.pluses = vec![harmonigraph_scene::PlusInstance {
        pos: glam::Vec3::ZERO,
        // Big enough that the screen-constant soft band is a thin rim on it
        // rather than a share of the area — the band is the error term in
        // every ratio below, and a small marker is mostly band.
        radius: 0.5,
        color: glam::Vec4::ONE,
        strength: 1.0,
    }];
    scene.plus_half_width = half_width;
    scene.plus_taper_start = taper_start;
    scene
}

/// The two proportions a marker carries are read by the SHADER as the shapes
/// they name: a filled square at the top of the width bar, and ends that
/// actually run out at the bottom of the taper.
///
/// `the_width_reaches_the_scene_as_a_share_of_the_arm` pins the conversion on
/// the way in, and it cannot see this: the number arriving correct in the
/// uniform says nothing about the distance field spending it. What is measured
/// here is AREA, which is a number rather than a look — a cross of half-width
/// `t` covers `8t - 4t^2` of an arm-squared where a filled square covers 4, so
/// the ratio between two widths is arithmetic the picture either agrees with
/// or does not.
///
/// The premultiplied ink is linear in coverage (`plus_paint` returns
/// `rgb * alpha`), so summing the light IS integrating the area, with the soft
/// band as a proportional rim on both — which is why the marker is drawn big
/// and the tolerance is a tenth rather than a percent.
///
/// What stays a look, and stays with `the_resting_markers_draw_a_picture`:
/// whether a cross reads as a crossing rather than as a glyph, and whether the
/// tapered end arrives at nothing rather than stopping at something.
#[test]
fn the_shader_spends_a_markers_proportions_on_the_shape_they_name() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut gpu) = Shooter::new(SIZE) else {
        return;
    };

    // Square ends throughout, so this half measures the WIDTH alone.
    let ink = |gpu: &mut Shooter, t: f32| total_light(&gpu.shot(&lone_marker_scene(t, 1.0)));
    let cross = ink(&mut gpu, 0.275) as f64;
    let square = ink(&mut gpu, 1.0) as f64;
    assert!(cross > 0.0, "the fixture drew no marker at all");

    let want = 4.0 / (8.0 * 0.275 - 4.0 * 0.275 * 0.275);
    let got = square / cross;
    assert!(
        (got - want).abs() / want < 0.1,
        "a half-width of 1 covers {got:.3}x what 0.275 does, and the shape says {want:.3}x \
         — the box's y-extent is not being read as half the arm's thickness",
    );

    // And the square really is filled rather than a very fat cross: at
    // half-width 1 the box covers the whole octant, so there is nothing left
    // for a wider one to add.
    let over = ink(&mut gpu, 1.0) as f64;
    assert!(
        (over - square).abs() / square < 0.01,
        "past a full half-width the marker is still growing: {square} then {over}",
    );

    // The taper, at one width: an arm solid to its tip, then to half way, then
    // fading the whole way from the crossing. Ink has to fall each time, and by
    // a share the smoothstep can account for — it integrates to half over the
    // span it covers, and the crossing keeps its own.
    let taper = |gpu: &mut Shooter, start: f32| {
        total_light(&gpu.shot(&lone_marker_scene(0.275, start))) as f64
    };
    let (square_end, half, whole) = (taper(&mut gpu, 1.0), taper(&mut gpu, 0.5), taper(&mut gpu, 0.0));
    assert!(
        square_end > half && half > whole,
        "a longer taper has to take more ink, not less: {square_end} {half} {whole}",
    );
    let lost = (square_end - whole) / square_end;
    assert!(
        (0.25..0.65).contains(&lost),
        "tapering the whole arm took {:.0}% of the ink; a smoothstep over the arm \
         with the crossing keeping its own is nearer 40%",
        lost * 100.0,
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

/// ...and the other end of that term: a node whose ring has faded out owes no
/// annulus, so the ring layer being ON is not on its own a reason to ship it.
///
/// The whole cost argument for the gate is here rather than in the picture —
/// a gated-off idle node ships nothing at all, where an ungated ring forces
/// every node in the window onto the bus. Nothing else can see it: a shipped
/// node with no ring and nothing else to draw is transparent at every
/// fragment, so no pixel test can tell it from one that was never sent, and
/// [`an_open_ring_ships_every_idle_node`] only ever drives the term's true
/// side.
///
/// Part way through the fade is the case that says it is a LEVEL and not the
/// gate's own bit: a ring on its way out is drawn, so it is shipped for
/// exactly as long as it is drawn.
#[test]
fn a_faded_out_ring_ships_no_idle_node() {
    let fresh = harmonigraph_scene::ViewConfig::default();
    let mut scene = idle_scene();
    assert!(!scene.nodes.is_empty(), "the fixture has to carry idle nodes");
    (scene.spectral.inner, scene.spectral.outer) = fresh.rings().audio;
    let ships = |scene: &Scene| {
        LatticeCallback::from_scene(
            scene,
            LatticeLabels::default(),
            egui::vec2(256.0, 256.0),
            wgpu::TextureFormat::Rgba8Unorm,
            32,
            None,
        )
        .instances
        .len()
    };
    for node in &mut scene.nodes {
        node.audio_ring = 0.0;
    }
    assert_eq!(ships(&scene), 0, "an idle node with no ring left was still shipped");
    for node in &mut scene.nodes {
        node.audio_ring = 0.5;
    }
    assert_eq!(
        ships(&scene),
        scene.nodes.len(),
        "an idle node part way through its fade draws a ring and has to be shipped",
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
        let rings = fresh.rings();
        scene.outer_inner = rings.band.0;
        scene.outer_outer = rings.band.1;
        scene.rings_outer = rings.outer;
        scene.octave_gap = fresh.octave_gap_width();
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
/// `pluses_at` is the seam between the sheets BEHIND the home sheet and the
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
fn the_marker_seam_counts_the_nodes_that_ship() {
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
        call.pluses_at, 1,
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
    empty.pluses.clear();
    let blank = LatticeCallback::from_scene(
        &empty,
        LatticeLabels::default(),
        size,
        format,
        40,
        Some(stats.clone()),
    );
    assert!(blank.instances.is_empty() && blank.pluses.is_empty(), "nothing to draw");
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
/// this one is about a node's RINGS rather than about a glyph: at the real
/// ratio of node radius to lattice spacing, 64 points across puts the whole
/// node inside a couple of pixels.
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
    // Big enough that the octave band — the one thing this node paints — is
    // several pixels deep and reaches full coverage in the middle of itself.
    // A band's two soft edges are screen-constant, so on a small node they
    // overlap and the annulus never becomes opaque, which is the one property
    // the covering claim below is read off.
    scene.node_radius = 0.5;
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
    // The markers would draw across the same pixels, and whether ONE covers a
    // label is a separate question that this fixture cannot answer twice.
    scene.pluses.clear();
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
    /// Points out from the node's centre to the middle of its octave band,
    /// where the annulus is opaque; to that band's own fading edge; and to
    /// bare quad past every ring, where the node paints nothing at all.
    const ON_RING: f32 = 15.0;
    const ON_EDGE: f32 = 18.0;
    const PAST_RING: f32 = 30.0;

    for (what, instance) in
        [("a letter", crate::text::tests::glyph()), ("a drawn mark", crate::text::tests::mark())]
    {
        let at = |off: f32, label: Option<u32>| -> u8 {
            let frame = picture(instance, off, label);
            let i = (((y as u32) * SCENE_SIZE[0] + (x + off) as u32) * 4) as usize;
            frame[i + 1]
        };

        // On the octave band, which is opaque: the far node's name is gone,
        // exactly gone — this is compositing, not a mask, so "under an opaque
        // ring" is the picture with no label in it at all.
        let (bare_ring, under, over) =
            (at(ON_RING, None), at(ON_RING, Some(FAR)), at(ON_RING, Some(NEAR)));
        assert_eq!(
            under, bare_ring,
            "{what} under an opaque ring must leave no trace of itself",
        );
        assert!(
            over.abs_diff(bare_ring) > 32,
            "{what} drawn after the ring must be plainly visible on it, \
             got {over} against a bare ring's {bare_ring} — if these agree the \
             glyph is not landing on the node and the assertion above is vacuous",
        );

        // Across the band's own fading edge, where the difference between
        // covering and cutting shows: the name dims by exactly what the ring
        // took, rather than being taken out whole or left alone.
        let (bare_edge, under_edge, over_edge) =
            (at(ON_EDGE, None), at(ON_EDGE, Some(FAR)), at(ON_EDGE, Some(NEAR)));
        assert!(
            under_edge > bare_edge && under_edge < over_edge,
            "over the ring's fading edge {what} must dim rather than vanish: \
             {under_edge} against {bare_edge} bare and {over_edge} drawn on top",
        );

        // And past every ring the node draws — still inside its own quad, and
        // painted with nothing at all — a name is left exactly alone. A node
        // covers what it PAINTS and no more of its billboard than that.
        let (bare_out, under_out, over_out) =
            (at(PAST_RING, None), at(PAST_RING, Some(FAR)), at(PAST_RING, Some(NEAR)));
        assert!(
            over_out.abs_diff(bare_out) > 32,
            "the probe past the rings must be somewhere {what} shows at all: {over_out} \
             against {bare_out}",
        );
        assert!(
            under_out.abs_diff(over_out) <= 3,
            "past every ring it draws, a node must leave {what} alone: {under_out} \
             against {over_out} drawn on top",
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
    scene.pluses.clear();
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
            GlyphSeam { at: 0, start: 0, count: 2, after_pluses: false, sheet: 0 },
            // The home sheet's own name, after its disc.
            GlyphSeam { at: 1, start: 2, count: 1, after_pluses: true, sheet: 1 },
            // And the near sheet's, after everything.
            GlyphSeam { at: 2, start: 3, count: 1, after_pluses: true, sheet: 2 },
        ],
        "a label goes after its own node, over the instances that ship",
    );
    assert_eq!(
        call.glyphs.iter().map(|g| g.rect[0]).collect::<Vec<_>>(),
        vec![1.0, 2.0, 3.0, 0.0],
        "the glyphs are regrouped into the order they are drawn in",
    );
}

/// Two names sharing a seam from different sheets draw nearer-last, and as
/// two draws — the one tie `at` and the side of the markers cannot break.
///
/// The last node of one sheet to ship and a node culled at the head of the
/// next sheet land on one `at`, the cull having moved nothing; with both
/// sheets on the same side of the markers, nothing else in the seam tells
/// them apart. The sort's fallback is then the order the labels ARRIVE in,
/// which is the scene's node order — lattice order, fixed whichever way the
/// camera is turned — so the farther sheet's name draws over the nearer's
/// whenever its node happens to come later in the lattice. And the two merge
/// into one rim-then-fill draw, so the nearer name's rim no longer separates
/// its glyphs from the farther name's fill where the two overlap, which on a
/// face-on sevens lattice they do exactly: a sevens node sits on top of its
/// home node.
///
/// The state is an ordinary one: a home node sounding with a silent node on
/// the sheet in front of it hovered. The hovered node draws nothing and is
/// named all the same.
#[test]
fn two_names_on_one_seam_from_different_sheets_draw_the_nearer_last_and_apart() {
    let mut scene = parity_scene();
    scene.camera = harmonigraph_scene::Camera {
        projection: harmonigraph_scene::Projection::Orthographic,
        yaw: 0.0,
        pitch: 0.0,
        ..Default::default()
    };
    scene.pluses.clear();
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
    // The near sheet's node FIRST in the scene, so that lattice order and
    // depth order disagree: drawn back to front it is home, then near; the
    // home node ships and the silent near one does not, so both seams sit at
    // 1, past the markers.
    scene.nodes = vec![node(1.0, 0.0), node(0.0, 1.0)];
    let (near, home) = (0u32, 1u32);
    let glyph =
        |at: f32| GlyphInstance { rect: [at, 0.0, 1.0, 1.0], ..crate::text::tests::glyph() };
    let call = LatticeCallback::from_scene(
        &scene,
        LatticeLabels {
            glyphs: vec![glyph(0.0), glyph(1.0)],
            labels: [near, home].map(|node| Label { node, glyphs: 1 }).to_vec(),
            rings: [TextRing::default(); 2],
            atlas: Some(crate::text::tests::atlas()),
            marks: None,
            slide: SlideAxis::default(),
        },
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        17,
        None,
    );

    assert_eq!(call.instances.len(), 1, "only the sounding home node ships an instance");
    assert!(
        call.seams.iter().all(|s| s.at == 1 && s.after_pluses)
            && call.seams.iter().map(|s| s.count).sum::<u32>() == 2,
        "the two names share a seam, on the same side of the markers — the fixture \
         has to produce the tie for the assertions below to be about anything: {:?}",
        call.seams,
    );
    assert_eq!(
        call.glyphs.iter().map(|g| g.rect[0]).collect::<Vec<_>>(),
        vec![1.0, 0.0],
        "the home sheet's name draws first and the nearer sheet's over it",
    );
    assert_eq!(
        call.seams.iter().map(|s| (s.start, s.count)).collect::<Vec<_>>(),
        vec![(0, 1), (1, 1)],
        "names from different sheets are two draws, the nearer's rim on the other's fill",
    );
}

/// A name on a node that ships no disc still draws over the markers, not
/// under them — the case where the two runs meet at the same number.
///
/// `pluses_at` is where the markers go, counted over the instances that SHIP.
/// The sheets behind the home one draw before them and the home sheet after,
/// so the boundary between the two runs is exactly `pluses_at`. A node that
/// paints nothing ships nothing, which leaves its seam sitting on the
/// boundary rather than past it: with every node on the home sheet — the
/// stock `extent_sevens: 0` — the far run is empty, `pluses_at` is 0, and the
/// first home node to be culled takes seam 0 as well. Reading the side off
/// `at > pluses_at` then files that node's name with the sheets BEHIND the
/// markers, and they are painted over the name.
///
/// The state is the plugin's resting one, which is what makes it worth a
/// test of its own: stock view, nothing played, hover any node. An idle node
/// draws nothing at all, and a hovered node is named whether or not it
/// draws.
#[test]
fn a_culled_home_nodes_name_draws_over_the_markers_it_shares_a_seam_with() {
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
    // has shipped and its seam is 0 — the same number as `pluses_at`.
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

    assert!(!scene.pluses.is_empty(), "the fixture needs a marker for the name to be covered BY");
    assert_eq!(call.pluses_at, 0, "with one sheet there is nothing to draw before the pluses");
    assert_eq!(
        call.seams,
        vec![GlyphSeam { at: 0, start: 0, count: 1, after_pluses: true, sheet: 0 }],
        "a home node's name draws after the pluses even when the cull leaves it on pluses_at",
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

/// The node glow's two claims about geometry: it puts light OUTSIDE the node it
/// comes from, and the Reach bar is what says how far out.
///
/// Measured as the farthest pixel the glow changes, rather than as an amount at
/// a chosen radius, because the reach is where the light STOPS and not how
/// bright it is on the way: the same number is the falloff's domain and the
/// point its window shuts (`glow_layer`), so what moves when the bar moves is
/// the edge.
///
/// One centered node on a cleared grid — [`single_marked_node`]'s fixture,
/// which is the one scene here with a node whose surroundings are empty enough
/// for "outside the node" to mean anything.
#[test]
fn the_glow_reach_says_how_far_a_node_lights_past_its_own_edge() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |reach: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene
    };
    let off = shooter.shot(&at(0.0));
    let near = shooter.shot(&at(0.2));
    let far = shooter.shot(&at(0.8));

    // How far from the node's center — the frame's center, the fixture's one
    // node sitting at the world origin — the picture changed at all.
    let farthest = |a: &[u8], b: &[u8]| -> f32 {
        let row = SIZE[0] as usize;
        let center = (SIZE[0] as f32 / 2.0, SIZE[1] as f32 / 2.0);
        a.chunks(4)
            .zip(b.chunks(4))
            .enumerate()
            .filter(|(_, (x, y))| x != y)
            .map(|(i, _)| {
                let (px, py) = ((i % row) as f32, (i / row) as f32);
                ((px - center.0).powi(2) + (py - center.1).powi(2)).sqrt()
            })
            .fold(0.0f32, f32::max)
    };
    let (near_edge, far_edge) = (farthest(&near, &off), farthest(&far, &off));

    // Non-vacuous first: there has to BE light to measure. Against the reach
    // beside it rather than against the glow OFF, because the glow replaces the
    // core's own skirt rather than joining it — a narrow reach spreads that
    // same light thinner and can leave the frame dimmer overall, where one
    // reach against another is the monotone claim: the lit area grows with the
    // square of the span.
    assert!(
        total_light(&far) > total_light(&near),
        "a wider reach must spread more light: {} against {}",
        total_light(&far),
        total_light(&near),
    );
    assert!(near_edge > 0.0, "a glow at reach 0.2 changed no pixel at all");
    assert!(
        far_edge > near_edge + 4.0,
        "reach 0.8 must light further out than reach 0.2, and it reached {far_edge:.1}px \
         against {near_edge:.1}px",
    );
}

/// The Feather bar's claim: it fills a node's own reach in rather than
/// reaching further. The light's centre of mass moves outward, and the far
/// edge — which the Reach alone decides — stays where it was.
///
/// A light-weighted mean RADIUS, over the pixels the glow changed, rather than
/// an amount at a chosen distance: what the bar moves is where inside one span
/// the light sits, and any single annulus reads that as brightness. One
/// centered node on a cleared grid ([`single_marked_node`]), so the profile
/// under the measurement is the falloff and nothing else.
#[test]
fn the_glow_feather_fills_a_nodes_reach_in_rather_than_reaching_further() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |feather: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.glow_reach = 0.8;
        scene.glow_strength = 1.0;
        scene.glow_feather = feather;
        scene
    };
    let mut off = at(0.0);
    off.glow_reach = 0.0;
    let dark = shooter.shot(&off);
    let tight = shooter.shot(&at(0.0));
    let flat = shooter.shot(&at(1.0));

    // Every pixel the glow changed, as (radius from the node's centre, how much
    // light it gained) — the frame's centre being where the fixture's one node
    // sits.
    let lit = |a: &[u8]| -> Vec<(f32, f64)> {
        let row = SIZE[0] as usize;
        let centre = (SIZE[0] as f32 / 2.0, SIZE[1] as f32 / 2.0);
        a.chunks(4)
            .zip(dark.chunks(4))
            .enumerate()
            .filter_map(|(i, (x, y))| {
                let gained = (brightness(x) - brightness(y)) as f64;
                if gained <= 0.0 {
                    return None;
                }
                let (px, py) = ((i % row) as f32, (i / row) as f32);
                let r = ((px - centre.0).powi(2) + (py - centre.1).powi(2)).sqrt();
                Some((r, gained))
            })
            .collect()
    };
    let mean_radius = |lit: &[(f32, f64)]| -> f64 {
        let weight: f64 = lit.iter().map(|&(_, w)| w).sum();
        assert!(weight > 0.0, "a glow at reach 0.8 added no light at all");
        lit.iter().map(|&(r, w)| r as f64 * w).sum::<f64>() / weight
    };
    let edge = |lit: &[(f32, f64)]| -> f32 { lit.iter().fold(0.0f32, |m, &(r, _)| m.max(r)) };

    let (tight_lit, flat_lit) = (lit(&tight), lit(&flat));
    let (tight_r, flat_r) = (mean_radius(&tight_lit), mean_radius(&flat_lit));
    assert!(
        flat_r > tight_r * 1.2,
        "a feathered light must sit further out in its own span: {flat_r:.1}px against \
         {tight_r:.1}px",
    );
    // And barely further out than the unfeathered one: the window is the same
    // smoothstep across the same span at either setting, so the edge belongs to
    // the Reach. Not "the same to the pixel", and the tenth is not slack —
    // what a picture shows is where the light last cleared 1/255, and the flat
    // profile arrives at the window's own tail with sixteen times the amplitude
    // under it, which buys a few more pixels of a cubic tail before it
    // quantizes away. A tenth of the span is small against the difference the
    // Reach itself makes over that range (`the_glow_reach_says_how_far_a_node_
    // lights_past_its_own_edge`), which is the distinction being drawn.
    let (tight_edge, flat_edge) = (edge(&tight_lit), edge(&flat_lit));
    assert!(
        flat_edge <= tight_edge * 1.1,
        "the Feather bar moved the light's far edge from {tight_edge:.1}px to \
         {flat_edge:.1}px — the span is the Reach's to say",
    );
}

/// The widest the colours round one node's halo get from one another: the
/// annulus `inner..outer` about the frame's centre cut into wedges, each
/// wedge's mean taken, and the largest distance between any two of them.
///
/// A CHROMATICITY — every pixel divided by its own total — because the light
/// falls off across the annulus and a plain mean would read that falloff as a
/// colour difference. The question here is whether two directions are lit in
/// different COLOURS, not in different amounts.
fn halo_hue_spread(px: &[u8], size: [u32; 2], inner: f32, outer: f32) -> f64 {
    const BINS: usize = 16;
    let centre = (size[0] as f32 / 2.0, size[1] as f32 / 2.0);
    let mut sums = [[0.0f64; 3]; BINS];
    let mut counts = [0.0f64; BINS];
    for (i, p) in px.chunks(4).enumerate() {
        let (x, y) = ((i % size[0] as usize) as f32, (i / size[0] as usize) as f32);
        let (dx, dy) = (x - centre.0, y - centre.1);
        let r = (dx * dx + dy * dy).sqrt();
        if r < inner || r > outer {
            continue;
        }
        let total = f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]);
        // Too dark to have a colour at all: the chromaticity of a near-black
        // pixel is quantization noise, and out at the light's own edge there
        // are enough of those to drown the reading.
        if total < 24.0 {
            continue;
        }
        let turn = (dy.atan2(dx).rem_euclid(std::f32::consts::TAU)) / std::f32::consts::TAU;
        let bin = ((turn * BINS as f32) as usize).min(BINS - 1);
        for c in 0..3 {
            sums[bin][c] += f64::from(p[c]) / total;
        }
        counts[bin] += 1.0;
    }
    let means: Vec<[f64; 3]> = (0..BINS)
        .filter(|&b| counts[b] > 0.0)
        .map(|b| [sums[b][0] / counts[b], sums[b][1] / counts[b], sums[b][2] / counts[b]])
        .collect();
    let mut worst = 0.0f64;
    for (i, a) in means.iter().enumerate() {
        for b in &means[i + 1..] {
            let d = (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2);
            worst = worst.max(d.sqrt());
        }
    }
    worst
}

/// The Color blend bar's whole claim: a node lighting two directions in two colours
/// keeps them apart at the bottom of the bar and averages them into one tint at
/// the top.
///
/// Measured out in the HALO — an annulus past everything the node draws — and
/// not over the node, because the ink is drawn over the light there and what
/// would be read is the node rather than its glow. The two ends of the fixture's
/// mark are the two colours: gold one way, cyan another, on octave slices that
/// do not touch.
///
/// This is the reading the bar had no test for while it did nearly nothing. The
/// colour eased toward the strip's flat mean over the light's whole SPAN, and
/// the skirt is an exponential over that same length, so with any real Reach
/// dialled in the halo was that mean nearly everywhere — and the mean is the one
/// average the Color blend bar cannot move, being taken at no concentration at all by
/// definition. Against the node's own rim instead, the bottom of the bar reads
/// 0.11 here where the span ramp read 0.065, and the bar's travel roughly
/// doubles.
#[test]
fn the_glow_blend_says_how_separate_a_node_keeps_its_colours() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let beside = slot_beside_middle_c();
    let at = |blend: f32| -> Scene {
        let mut scene = single_marked_node(MIDDLE_C, beside);
        scene.glow_reach = 0.8;
        scene.glow_strength = 1.5;
        scene.glow_blend = blend;
        scene
    };
    let tight = shooter.shot(&at(0.0));
    let middle = shooter.shot(&at(0.5));
    let broad = shooter.shot(&at(1.0));

    // The annulus, sized off the light itself rather than guessed: the farthest
    // pixel the glow moves is where its window shuts, and the outer half of
    // that is halo and nothing else.
    let mut edge = 0.0f32;
    let mut unlit = at(0.0);
    unlit.glow_reach = 0.0;
    let dark = shooter.shot(&unlit);
    let row = SIZE[0] as usize;
    for (i, (a, b)) in tight.chunks(4).zip(dark.chunks(4)).enumerate() {
        if a != b {
            let (px, py) = ((i % row) as f32, (i / row) as f32);
            let (dx, dy) = (px - SIZE[0] as f32 / 2.0, py - SIZE[1] as f32 / 2.0);
            edge = edge.max((dx * dx + dy * dy).sqrt());
        }
    }
    assert!(
        edge > 16.0,
        "the fixture's light reached only {edge:.1}px and there is nothing to read",
    );
    let (inner, outer) = (edge * 0.6, edge * 0.9);

    let spreads = [
        halo_hue_spread(&tight, SIZE, inner, outer),
        halo_hue_spread(&middle, SIZE, inner, outer),
        halo_hue_spread(&broad, SIZE, inner, outer),
    ];
    eprintln!(
        "halo hue spread over {inner:.0}..{outer:.0}px — tight {:.4}, middle {:.4}, broad {:.4}",
        spreads[0], spreads[1], spreads[2],
    );
    // Non-vacuous first: the bottom of the bar has to draw two colours at all.
    // The fixture's two marks are 0.36 apart in this reading laid down pure, so
    // a tenth of that is a halo carrying both of them and not one tint.
    assert!(
        spreads[0] > 0.035,
        "at the bottom of the Color blend bar a node lighting two colours drew one: {:.4}",
        spreads[0],
    );
    // And monotone: every step up the bar averages further.
    assert!(
        spreads[0] > spreads[1] && spreads[1] > spreads[2],
        "the Color blend bar must average further at every step: {:.4} / {:.4} / {:.4}",
        spreads[0], spreads[1], spreads[2],
    );
    // The top of it is the mean, which has no direction left in it — read as a
    // RATIO against the bottom rather than as an absolute, because the annulus
    // is sized off the light's own edge and the node's outer ink reaches a
    // little way into it. That ink has a direction whatever the bar says, so
    // the top of the bar has a floor it cannot go under and only the two ends
    // compared say what the LIGHT did.
    assert!(
        spreads[0] > spreads[2] * 3.0,
        "the top of the bar must be one tint beside the bottom, and it kept \
         {:.4} against {:.4}",
        spreads[2], spreads[0],
    );
}

/// What the glow is a layer OF is a node, and a lattice drawing none has no
/// light — the resting markers included, which are drawn in the same pass and
/// would glow like nodes if the light were a post-process over the picture
/// rather than a draw off the node instance buffer.
///
/// Byte-identical rather than nearly so: every draw of the glow's is over the
/// instance buffer, which is empty, so the target is cleared to transparent and
/// nothing writes to it, and the composite lays exactly nothing over the
/// picture.
#[test]
fn a_lattice_with_no_node_grows_no_glow() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |reach: f32| -> Scene {
        let mut scene = parity_scene();
        scene.nodes.clear();
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene
    };
    let off = shooter.shot(&at(0.0));
    let on = shooter.shot(&at(0.8));
    // The grid has to be drawing something, or this passes on a blank frame.
    assert!(total_light(&off) > 0, "the fixture must draw its grid");
    assert_eq!(
        differing_pixels(&on, &off),
        0,
        "the glow lit something no node drew",
    );
}

/// A node a nearer sheet's node COVERS cuts nothing out of it: not a ring, not
/// its name, and not a shadow anywhere in the near node's halo. What a hidden
/// node may do is BRIGHTEN, and that is the whole of what it may do.
///
/// The case is a harmonic seventh over its home node, face on: the seventh's
/// disc and knockout hide the home node entirely. Every node draws AFTER the
/// whole light, so a covered node's ink and name are simply covered, and its
/// halo is one term of the melded field the near node stands on rather than a
/// layer laid over the near node's ink. Nothing in that field is subtractive,
/// so there is nothing a hidden node could cut with even if it reached.
///
/// Measured over the near node's own INK — the pixels its paint covers in full
/// — and per CHANNEL, since that is the claim the wash rests on: the light a
/// node's ink takes is a screen (`node_paint`), which can add to a channel and
/// never take from one, so a far sheet's red halo cannot pull the green out of
/// a white name in front of it.
///
/// It rests on the fixture's fresh WASH, that being what puts any light on a
/// node's ink at all: at a wash of 0 the near node's ink is untouchable and
/// there is nothing here to measure. The count of pixels the hidden node
/// brightens is asserted for that reason, and says so when it is 0.
///
/// That set is found rather than described, and found on the GROUND rather than
/// on the light, which reaches the ink too: a pixel the node paints opaquely is
/// a pixel no ground shows through, so it is the pixel that does not move when
/// the ground does. Two shots with the glow off and the darkest
/// and brightest grounds there are agree exactly over that ink and nowhere else
/// the node draws, the clearing painting the ground by definition. Every other
/// pixel of the node is that clearing, which paints the light over the ground
/// on purpose — the halo behind it belongs there, and asking for it back would
/// be asking for the hole this design exists to not have.
///
/// The hidden node is LIT and NAMED, both at the middle of the near node where
/// the light is fullest and an escaped name would be most legible. The name is
/// measured on its OWN footprint and not on the ink: a glyph lands in the empty
/// middle the rings stand around, so no opaque pixel can carry that claim.
#[test]
fn a_node_under_a_nearer_sheets_node_cuts_nothing_out_of_its_light() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let near = |reach: f32| {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        // A clearing, so the near node's knockout covers the far node's ink
        // in the scene pass; what is measured is the light, not the knockout.
        scene.nodes[0].gutter = 0.4;
        scene
    };
    let covering = |wash: f32| -> Scene {
        let mut both = near(0.8);
        both.glow_wash = wash;
        let mut far = both.nodes[0];
        far.world_pos.z = -1.0;
        // Smaller and off centre, so the whole of what it draws falls inside
        // the near node's clearing while its halo reaches where the near node's
        // light is PARTIAL — at the middle the light is full, and full light
        // melded with anything is still full.
        far.scale = 0.5;
        far.world_pos.x += 0.4;
        far.glow = harmonigraph_scene::GlowStep { level: 0.8, row: 1, mix: 1.0, marked: 0.0 };
        far.color = glam::Vec4::new(0.9, 0.2, 0.2, 1.0);
        both.nodes.push(far);
        rows_per_node(&mut both);
        both
    };
    let fresh_wash = single_marked_node(0, 0).glow_wash;
    let both = covering(fresh_wash);
    // The hidden node names itself, at the middle of the near node.
    let name = |node: u32| LatticeLabels {
        glyphs: vec![GlyphInstance {
            rect: [112.0, 112.0, 32.0, 32.0],
            fill: [255, 255, 255, 255],
            rim: [0, 0, 0, 255],
            ..crate::text::tests::glyph()
        }],
        labels: vec![Label { node, glyphs: 1 }],
        rings: [TextRing::default(); 2],
        atlas: Some(crate::text::tests::atlas()),
        marks: None,
        slide: SlideAxis::default(),
    };

    let call = LatticeCallback::from_scene(
        &both,
        name(1),
        egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
        wgpu::TextureFormat::Rgba8Unorm,
        0,
        None,
    );
    assert_eq!(
        call.instances[0].world_pos[2],
        -1.0,
        "the fixture puts the second node BEHIND the first, or it is covering rather than covered",
    );

    let alone_on = shooter.shot(&near(0.8));
    // The two grounds, with the glow off so that the ground is the only thing
    // moving. Black and white rather than two greys: the widest step there is
    // leaves no pixel of the clearing agreeing across it by rounding, and a
    // pixel too faint to differ even here is a pixel the pass discarded and the
    // `drawn` test drops.
    let mut on_ground = |bg: glam::Vec4| {
        let mut scene = near(0.0);
        scene.background = bg;
        shooter.shot(&scene)
    };
    let dark_ground = on_ground(glam::Vec4::new(0.0, 0.0, 0.0, 1.0));
    let pale_ground = on_ground(glam::Vec4::ONE);
    let covered = shooter.shot_with(&both, name(1));
    // The near node's opaque ink: drawn (so not the cleared black the pass
    // starts from) and the same over either ground (so neither its clearing nor
    // a soft edge of its own paint).
    let ink: Vec<usize> = (0..alone_on.len())
        .step_by(4)
        .filter(|&i| {
            pale_ground[i..i + 4] != [0u8, 0, 0, 255]
                && pale_ground[i..i + 4] == dark_ground[i..i + 4]
        })
        .collect();
    assert!(ink.len() > 500, "the near node painted {} opaque pixels", ink.len());
    let moved = |cmp: fn(u8, u8) -> bool| {
        ink.iter().filter(|&&i| (0..3).any(|c| cmp(covered[i + c], alone_on[i + c]))).count()
    };
    let dimmed = moved(|a, b| a < b);
    // Non-vacuous over the INK itself, not merely somewhere in the frame: the
    // hidden node's halo is part of the melded field the near node's own paint
    // is washed with, so it DOES reach that paint, and "never darker" is being
    // asked of pixels a hidden node actually moves. That reach is the tradeoff
    // the melded field is taken for — see `node_paint`, where the alternative
    // is a light pass per sheet.
    assert!(
        moved(|a, b| a > b) > 0,
        "the hidden node moved none of the near node's {} opaque pixels; the comparison is vacuous",
        ink.len(),
    );
    assert_eq!(
        dimmed,
        0,
        "a node hidden behind the near one darkened {dimmed} of its {} opaque pixels",
        ink.len(),
    );
    // The NAME's own claim, which the ink set cannot carry at all: a glyph sits
    // in the node's empty MIDDLE, where the rings stand around nothing and
    // there is no opaque ink to move. So it is measured over the pixels a name
    // actually covers, found by giving the same glyph to the NEAR node — whose
    // name is drawn — and read against a shot of the same two nodes with nobody
    // named. Both carry the hidden node's halo, so what differs between them is
    // the name and only the name.
    let unnamed = shooter.shot(&covering(fresh_wash));
    let named_near = shooter.shot_with(&covering(fresh_wash), name(0));
    let glyph: Vec<usize> = (0..unnamed.len())
        .step_by(4)
        .filter(|&i| named_near[i..i + 4] != unnamed[i..i + 4])
        .collect();
    assert!(
        glyph.len() > 500,
        "the fixture's name covers {} pixels; there is no name here to escape",
        glyph.len(),
    );
    let escaped = glyph.iter().filter(|&&i| covered[i..i + 4] != unnamed[i..i + 4]).count();
    assert_eq!(
        escaped,
        0,
        "a hidden node's name reached {escaped} of the {} pixels it would cover were it drawn",
        glyph.len(),
    );
}

/// The MIDDLE of a node glows, and a SHEET behind it makes it glow more.
///
/// Two halves, and the second is #435. The first: inside the innermost ring
/// there is nothing painted at all — [`parity_scene`]'s octave band is an
/// annulus and the audio ring is off — so what that pixel carries is the light
/// and only the light, which is what makes the glow the note's own light
/// rather than a rim around it. Read against the glow OFF rather than against
/// a neighbouring pixel, because the thing that must not happen is the middle
/// going DARK: nothing else is drawn there to take the light's place.
///
/// The second: a node on a sheet BEHIND adds its halo to the field, and a
/// node's clearing paints that field over the ground (`node_paint`), so the
/// near node's middle comes out brighter than it is with nothing behind it. A
/// nearer node's body taking the light of the sheets behind off itself is what
/// inverts this — its middle then holds its own light alone while the ground a
/// few pixels away holds everyone's, and the node reads as a hole rather than
/// as a lamp. The near node carries a CLEARING here for exactly that reason:
/// its footprint is the surface such a pass would erase.
#[test]
fn the_middle_of_a_node_is_where_its_light_is_fullest() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |reach: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene.nodes[0].gutter = 0.16;
        scene
    };
    // The fixture's one node sits at the origin, which the camera is pointed
    // at, so the frame's centre is the node's.
    let mid = ((SIZE[1] / 2) * SIZE[0] + SIZE[0] / 2) as usize;
    let middle = |px: &[u8]| brightness(&px[mid * 4..mid * 4 + 3]);

    let off = shooter.shot(&at(0.0));
    let on = shooter.shot(&at(0.8));
    assert!(
        middle(&on) > middle(&off),
        "the node's middle is no brighter with the glow on: {} against {}",
        middle(&on),
        middle(&off),
    );

    // A second sheet: one node behind, its light reaching far enough to wash
    // over the near node's whole footprint.
    let mut flat = at(0.8);
    flat.glow_reach = 3.0;
    let mut far = flat.nodes[0];
    far.world_pos.z = -1.0;
    far.world_pos.x += 0.6;
    far.gutter = 0.0;
    far.glow = harmonigraph_scene::GlowStep { level: 1.0, row: 1, mix: 1.0, marked: 0.0 };
    far.color = glam::Vec4::new(0.9, 0.2, 0.2, 1.0);
    let mut sheets = at(0.8);
    sheets.glow_reach = 3.0;
    sheets.nodes.push(far);
    rows_per_node(&mut sheets);
    rows_per_node(&mut flat);

    let one_sheet = shooter.shot(&flat);
    let two_sheets = shooter.shot(&sheets);
    assert!(
        middle(&two_sheets) > middle(&one_sheet),
        "a sheet behind left the near node's middle at {} against {} with nothing behind it: \
         its light is being taken off its own body",
        middle(&two_sheets),
        middle(&one_sheet),
    );
}

/// A node STANDS ITS LIGHT OFF the rings it draws, and the Gap depth is the
/// one switch on it.
///
/// The standoff is a term of the node's own clearing rather than a hole cut in
/// the light: the clearing paints the finished field over the ground
/// (`node_paint`), and around every ring the node draws it paints that field
/// dimmed. So what this measures is a pixel just outside the octave band —
/// inside the clearing, outside every shape the node inks — where the light is
/// otherwise at nearly its fullest, the falloff being measured from the node's
/// centre.
///
/// The GROUND is the whole of what this bar moves, a node's own ink being the
/// Wash bar's — [`a_ring_wears_the_wash_inside_its_own_dark_pool`] holds that
/// boundary from the other side. So the probe sits outside the ink on purpose,
/// and not merely for want of light there: a probe ON the ink would read
/// nothing this bar does at any setting.
///
/// TWO claims, and the second is what makes the bar an A/B rather than a
/// restyle. The depth takes light: the probe is darker at the fresh 85% than
/// at 0, and no pixel anywhere in the frame is brighter, the term the depth
/// scales being a factor on light that was going to be laid down anyway. And a
/// depth of 0 is the whole feature off: the frame is byte for byte the same at
/// any Gap, which is the one place the four dials can be proved not to leak
/// into a picture that is supposed to have no standoff in it.
///
/// A Gap of 0 is deliberately NOT the off position and is not compared here: it
/// is a standoff whose fade has collapsed onto the ring's own annulus, which is
/// a CRISPER one, not an absent one.
///
/// [`the_middle_of_a_node_is_where_its_light_is_fullest`]'s fixture, whose
/// clearing is what the standoff lives in, plus one calibration shot: with the
/// glow and the clearing both off, the outermost pixel the node inks along +x
/// IS the band's outer edge, which is `rings_outer` in the node's own uv. That
/// is the scale everything below is measured in, so the probe follows the
/// fixture instead of naming a pixel.
#[test]
fn the_gap_depth_says_how_much_light_a_ring_stands_off() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |reach: f32, gap: f32, depth: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene.glow_gap = gap;
        // The fade the whole width of the gap, which is the fresh pair.
        scene.glow_gap_soft = gap;
        scene.glow_gap_depth = depth;
        scene.nodes[0].gutter = 0.16;
        scene
    };
    let row = SIZE[0] as usize;
    let centre = (SIZE[1] / 2) as usize * row + (SIZE[0] / 2) as usize;

    // The scale: the node's own ink with nothing around it. The clearing is off
    // for this shot alone — it paints the ground out past the ink, and what is
    // wanted here is where the INK stops.
    let mut bare = at(0.0, 0.0, 0.0);
    bare.nodes[0].gutter = 0.0;
    let plain = shooter.shot(&bare);
    let inked = |px: &[u8], i: usize| px[i * 4..i * 4 + 4] != [0u8, 0, 0, 255];
    let band_px = (1..(SIZE[0] / 2) as usize)
        .rfind(|&x| inked(&plain, centre + x))
        .expect("the fixture's node must ink something along +x");
    assert!(band_px > 20, "the node inked only {band_px}px of radius; there is nothing to read");
    // Half a Gap past the band's outer edge: the standoff is solid there and
    // the node inks nothing, so the whole of the difference below is light.
    let probe = centre
        + (band_px as f32 * (1.0 + 0.08 / bare.rings_outer)).round() as usize;
    assert!(
        !inked(&plain, probe),
        "the probe at {probe} sits on the node's own ink, not outside it",
    );

    let lit = |px: &[u8], i: usize| brightness(&px[i * 4..i * 4 + 3]);
    let stood_off = shooter.shot(&at(0.8, 0.16, 0.85));
    let flat = shooter.shot(&at(0.8, 0.16, 0.0));
    assert!(
        lit(&stood_off, probe) < lit(&flat, probe),
        "the standoff left the pixel outside the ring at {} against {} with the depth at 0",
        lit(&stood_off, probe),
        lit(&flat, probe),
    );
    // Non-vacuous: there has to be light there to stand off in the first place.
    let dark = shooter.shot(&at(0.0, 0.16, 0.85));
    assert!(
        lit(&flat, probe) > lit(&dark, probe),
        "the fixture lights the probe no more than the glow off does; the comparison is vacuous",
    );
    // A factor on light that was going to be laid down anyway, so it can only
    // take: no pixel in the frame comes out brighter for it.
    let brighter = stood_off
        .chunks(4)
        .zip(flat.chunks(4))
        .filter(|(a, b)| brightness(&a[..3]) > brightness(&b[..3]))
        .count();
    assert_eq!(brighter, 0, "the standoff brightened {brighter} pixels");

    // The bar's top is the bare ground: where the standoff is solid, a depth of
    // 1 leaves the pixel exactly what it is with the glow off — not nearly,
    // since the clearing at full coverage replaces what is under it and a
    // factor of 0 on the light is no light. BOTH fades are taken off for this
    // pair of shots: the standoff's, and the clearing's it is floored at,
    // which at the fresh width runs nearly the whole gutter — so the probe
    // sits in a solid band rather than on either ramp.
    let solid = |reach: f32| {
        let mut scene = at(reach, 0.16, 1.0);
        scene.glow_gap_soft = 0.0;
        scene.sevens_soft = 0.0;
        scene
    };
    let bare_ground = shooter.shot(&solid(0.8));
    let no_glow = shooter.shot(&solid(0.0));
    assert_eq!(
        bare_ground[probe * 4..probe * 4 + 4],
        no_glow[probe * 4..probe * 4 + 4],
        "at a depth of 1 the stood-off pixel is not the frame with no glow in it",
    );

    // And the depth is the whole switch: at 0 the Gap and its curve reach the
    // picture nowhere.
    for (name, gap) in [("no gap", 0.0), ("a wide gap", 0.5)] {
        let other = shooter.shot(&at(0.8, gap, 0.0));
        assert_eq!(
            differing_pixels(&other, &flat),
            0,
            "at a depth of 0, {name} drew a different frame from the fresh gap",
        );
    }
}

/// The Gap reaches as far as it says, and the Clearance is not a lid on it.
///
/// The standoff is written into a layer of the LIGHT (`fs_glow`), so it dims
/// the field wherever that field reaches. A standoff carried instead by the
/// ground a node's own clearing paints is bounded at the Clearance's reach —
/// solid inward, where the clearing fills every footprint to the node's centre,
/// and gone a fraction of a node-radius outward, where the clearing has faded
/// out — which makes a Gap wider than the Clearance half a dial: it eats inward
/// and does nothing outward.
///
/// So the probe sits OUTSIDE the clearing altogether — five times further from
/// the ring than the Clearance reaches — and the two claims are what pin the
/// difference. The standoff dims it, which is the light being held off where no
/// node paints. And dialling the Clearance to nothing does not move it: what
/// holds the light off there is the Gap alone, so the pixel is identical with a
/// clearing and with none. Under the bounded shape neither shot moves, both
/// being the undimmed field.
///
/// The Clearance is deliberately not 0 in the first shot. A node that clears
/// nothing is the easy case — there is no lid to prove the standoff has got out
/// from under. The case worth pinning is a node whose clearing exists, ends,
/// and does not take the standoff's reach with it.
#[test]
fn the_gap_reaches_past_the_clearance_the_node_cuts() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // A narrow clearing under a wide gap, which is the pair a standoff bounded
    // by the clearing cannot draw. The fade is left at the full gap, the fresh
    // pairing, so the
    // probe reads the ramp rather than a band edge.
    const CLEARANCE: f32 = 0.02;
    const GAP: f32 = 0.5;
    let at = |reach: f32, depth: f32, gutter: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene.glow_gap = GAP;
        scene.glow_gap_soft = GAP;
        scene.glow_gap_depth = depth;
        scene.nodes[0].gutter = gutter;
        scene
    };
    let row = SIZE[0] as usize;
    let centre = (SIZE[1] / 2) as usize * row + (SIZE[0] / 2) as usize;

    // The scale, as `the_gap_depth_says_how_much_light_a_ring_stands_off` takes
    // it: the node's own ink with no clearing and no light, whose outermost lit
    // pixel along +x is `rings_outer` in the node's uv.
    let bare = at(0.0, 0.0, 0.0);
    let plain = shooter.shot(&bare);
    let inked = |px: &[u8], i: usize| px[i * 4..i * 4 + 4] != [0u8, 0, 0, 255];
    let band_px = (1..(SIZE[0] / 2) as usize)
        .rfind(|&x| inked(&plain, centre + x))
        .expect("the fixture's node must ink something along +x");
    let per_uv = band_px as f32 / bare.rings_outer;

    // Five times the Clearance out from the ink, and well inside the Gap: no
    // clearing of this node reaches here at any level, and the standoff still
    // has most of its own width left to spend.
    const PAST: f32 = CLEARANCE * 5.0;
    const { assert!(PAST < GAP, "the probe has to sit inside the gap it is measuring") };
    let probe = centre + (band_px as f32 + PAST * per_uv).round() as usize;
    assert!(
        !inked(&plain, probe),
        "the probe at {probe} sits on the node's own ink, not outside it",
    );

    let lit = |px: &[u8], i: usize| brightness(&px[i * 4..i * 4 + 3]);
    let stood_off = shooter.shot(&at(0.8, 0.85, CLEARANCE));
    let flat = shooter.shot(&at(0.8, 0.0, CLEARANCE));
    assert!(
        lit(&stood_off, probe) < lit(&flat, probe),
        "outside the clearing the standoff left the pixel at {} against {} with the depth at 0",
        lit(&stood_off, probe),
        lit(&flat, probe),
    );
    // Non-vacuous: there has to be light out there to hold off.
    let dark = shooter.shot(&at(0.0, 0.85, CLEARANCE));
    assert!(
        lit(&flat, probe) > lit(&dark, probe),
        "the fixture lights the probe no more than the glow off does; the comparison is vacuous",
    );

    // ...and it is the Gap's own doing, not the clearing's: take the Clearance
    // away entirely and the pixel does not move.
    let clearless = shooter.shot(&at(0.8, 0.85, 0.0));
    assert_eq!(
        clearless[probe * 4..probe * 4 + 4],
        stood_off[probe * 4..probe * 4 + 4],
        "the standoff outside the clearing changed when the Clearance was dialled off",
    );
}

/// The Gap reaches light this node never lit — a NEIGHBOUR's halo, out past
/// where its own light has shut.
///
/// The standoff is written per node into a layer of the light (`fs_glow`), one
/// quad per node, so what bounds it is that node's own billboard. The billboard
/// is sized to hold the LIGHT — the lit rim plus the Reach — and the Gap is a
/// length of its own with a ceiling of its own (`GLOW_GAP_MAX` against
/// `GLOW_REACH_MAX`), so a Gap dialled past the Reach asks for a standoff out
/// where this node draws no fragment at all. What an unheld bound looks like is
/// not a wrong value but a DISCONTINUITY: the fade stops dead partway down its
/// ramp, on a line that is straight and screen-aligned — `node_vertex` builds
/// the quad from `cam_right`/`cam_up` — so it slides around every node as the
/// camera turns while the lattice under it does not.
///
/// Hence a probe past `QUAD_MARGIN`, the floor the billboard takes at this
/// Reach, and inside `rings_outer + GAP`, where the standoff's own fade still
/// has most of its depth left. The light there is worth measuring only because
/// it is somebody else's: a node's own light shuts at its rim plus the Reach,
/// which is always inside its own quad, so the far side of the bound is lit by
/// the neighbour alone — the same split `fs_glow`'s early-out turns on, where
/// a node with no light of its own still stands its rings off a neighbour's.
#[test]
fn the_gap_reaches_light_the_nodes_own_never_lit() {
    // Wide enough for both nodes and a multiple of 64, so the readback's rows
    // stay aligned.
    const SIZE: [u32; 2] = [1408, 320];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The widest Gap the bar has against the fresh Reach — the pair that puts
    // the standoff outside the quad the light alone would size.
    const GAP: f32 = harmonigraph_scene::GLOW_GAP_MAX;
    const REACH: f32 = 0.35;
    // Where the neighbour stands, and where the light is read, both in the
    // probed node's uv. The probe is past `QUAD_MARGIN` (1.6) and inside the
    // fixture's `rings_outer + GAP`.
    const APART: f32 = 2.25;
    const PROBE: f32 = 1.65;

    // One node, and every bar of the standoff open but the depth.
    let alone = |reach: f32, depth: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = reach;
        scene.glow_strength = 2.0;
        // An even field, so the light is still worth something out at the bound
        // rather than an exponential's tail.
        scene.glow_feather = 1.0;
        scene.glow_gap = GAP;
        scene.glow_gap_soft = GAP;
        // The fade held longest, which is what leaves a measurable share of the
        // standoff this far out.
        scene.glow_gap_shape = 1.0;
        scene.glow_gap_depth = depth;
        scene.nodes[0].gutter = 0.0;
        scene
    };
    // ...and the neighbour that lights it. Dialled almost off, so every term of
    // its OWN standoff — each one scaled by the level of the layer it stands off
    // — is worth nothing at the probe, while its light rides the glow's own
    // clock at full.
    let with_neighbour = |reach: f32, depth: f32| -> Scene {
        let mut scene = alone(reach, depth);
        let mut lamp = scene.nodes[0].clone();
        lamp.world_pos = glam::Vec3::new(APART * scene.node_radius * 1.8, 0.0, 0.0);
        lamp.activation = 0.02;
        lamp.audio_ring = 0.0;
        lamp.ring_peak = 0.0;
        lamp.glow.row = 1;
        lamp.glow.marked = 0.0;
        scene.nodes.push(lamp);
        scene.glow_rows = scene.nodes.len() as u32;
        scene
    };

    let row = SIZE[0] as usize;
    let centre = (SIZE[1] / 2) as usize * row + (SIZE[0] / 2) as usize;
    // The scale, taken off ONE node so the outermost ink along +x is this node's
    // own rings and not the neighbour's: that pixel is `rings_outer` in its uv.
    let solo = alone(0.0, 0.0);
    let plain = shooter.shot(&solo);
    let inked = |px: &[u8], i: usize| px[i * 4..i * 4 + 4] != [0u8, 0, 0, 255];
    let band_px = (1..(SIZE[0] / 2) as usize)
        .rfind(|&x| inked(&plain, centre + x))
        .expect("the fixture's node must ink something along +x");
    let per_uv = band_px as f32 / solo.rings_outer;
    let probe = centre + (PROBE * per_uv).round() as usize;
    assert!(
        !inked(&plain, probe),
        "the probe at {probe} sits on the node's own ink, not outside it",
    );

    let lit = |px: &[u8], i: usize| brightness(&px[i * 4..i * 4 + 3]);
    let flat = shooter.shot(&with_neighbour(REACH, 0.0));
    let gapped = shooter.shot(&with_neighbour(REACH, 1.0));
    // The light at the probe is the NEIGHBOUR's: take the neighbour away and
    // this node's own light does not reach, so what the standoff is holding off
    // out here was never this node's to lay down.
    let lonely = shooter.shot(&alone(REACH, 0.0));
    assert!(
        lit(&lonely, probe) < lit(&flat, probe) / 4,
        "the probe is lit by the node's own light ({} against {} with a neighbour), so it \
         measures nothing about a neighbour's",
        lit(&lonely, probe),
        lit(&flat, probe),
    );

    // And the Gap holds it off. A tenth is far under the share the bars ask for
    // here and far over what the neighbour's own hundredth of a standoff can
    // account for, so the threshold is the bound and not the arithmetic.
    assert!(
        lit(&gapped, probe) * 10 < lit(&flat, probe) * 9,
        "the Gap left a neighbour's light at {} against {} with the depth at 0, so the \
         standoff stopped at this node's own billboard",
        lit(&gapped, probe),
        lit(&flat, probe),
    );
}

/// How much of the light the standoff takes at one radius, angle by angle, at a
/// closed Octave gap and at the widest one — the two rings of shares every claim
/// about the standoff following the slices is stated on.
///
/// The share is `(lit - stood off) / lit` at a Gap depth of 1, which is the
/// standoff's own coverage and nothing else. Three things have to hold for that,
/// and the fixtures below carry all three: the clearing the standoff rides is
/// radial, so it contributes one constant factor around the turn; the light's
/// own falloff is radial too (`glow_layer`), so the only thing varying with
/// angle is what is being measured; and the ground is black, so a pixel's
/// brightness IS its light and the division is exact rather than nearly so.
///
/// `ink_uv` is where the fixture's own ink ends in the node's uv, and `past` how
/// far outside it to read. The scale between the two is taken from a calibration
/// shot rather than assumed: the ink is found at a CLOSED gap, which is the
/// widest it is ever drawn, and that same shot is what proves the probe ring
/// clears it at every angle.
///
/// A ratio between the two rings at ONE pixel is the strongest reading it
/// supports, and the reason is the pixel grid: the ring lands on integer pixels
/// so its radius wobbles by up to half of one, which the fade's slope turns into
/// a couple of hundredths of share. Two shots at one pixel share that wobble
/// exactly, so a ratio has none of it; a claim made across angles has to carry a
/// budget for it.
fn standoff_share_rings(
    shooter: &mut Shooter,
    size: [u32; 2],
    at: &dyn Fn(f32, f32, f32) -> Scene,
    ink_uv: f32,
    past: f32,
    angles: usize,
) -> (Vec<f64>, Vec<f64>) {
    let row = size[0] as usize;
    // The node projects to the frame's exact middle, which is a CORNER of the
    // pixel grid on an even frame and not the middle of any pixel: every radius
    // here is taken from there and floored into the pixel that holds it, so the
    // probe ring is centred on the node rather than half a pixel off it.
    let cx = 0.5 * size[0] as f32;
    let cy = 0.5 * size[1] as f32;
    let centre = (size[1] / 2) as usize * row + (size[0] / 2) as usize;

    let mut bare = at(0.0, 0.0, 0.0);
    bare.nodes[0].gutter = 0.0;
    let plain = shooter.shot(&bare);
    let inked = |px: &[u8], i: usize| px[i * 4..i * 4 + 4] != [0u8, 0, 0, 255];
    let ink_px = (1..(size[0] / 2) as usize)
        .rfind(|&x| inked(&plain, centre + x))
        .expect("the fixture's node must ink something along +x");
    assert!(ink_px > 20, "the node inked only {ink_px}px of radius; there is nothing to read");
    let probe_px = ink_px as f32 * (1.0 + past / ink_uv);
    let probe = |k: usize| -> usize {
        let a = std::f32::consts::TAU * k as f32 / angles as f32;
        // The framebuffer's rows run down where the node's own uv runs up.
        let y = (cy - probe_px * a.sin()).floor() as usize;
        y * row + (cx + probe_px * a.cos()).floor() as usize
    };
    for k in 0..angles {
        assert!(
            !inked(&plain, probe(k)),
            "the probe ring crosses the node's own ink {k} steps round",
        );
    }

    let lit = |px: &[u8], i: usize| brightness(&px[i * 4..i * 4 + 3]);
    let mut shares = |octave_gap: f32| -> Vec<f64> {
        let flat = shooter.shot(&at(octave_gap, 0.8, 0.0));
        let stood = shooter.shot(&at(octave_gap, 0.8, 1.0));
        (0..angles)
            .map(|k| {
                let i = probe(k);
                let light = lit(&flat, i);
                assert!(
                    light > 60,
                    "the fixture lit the probe {k} steps round to only {light}: there is \
                     too little light there to measure a share of",
                );
                (light - lit(&stood, i)) as f64 / light as f64
            })
            .collect()
    };
    (shares(0.0), shares(harmonigraph_scene::GAP_MAX))
}

/// A node stands its light off the ink its rings DRAW, and a ring is slices
/// with gaps between them rather than a closed annulus.
///
/// What this measures is the SHARE of the light one pixel loses to the standoff
/// — `(lit - stood off) / lit` at a depth of 1 — taken at one radius half a Gap
/// outside the band, all the way round the node. That share is the standoff's
/// own coverage and nothing else: the clearing carrying it is a disc
/// (`node_clearing` reads the band as a rim), the light's falloff is radial
/// (`glow_layer`), and whatever the light's colour does around the turn divides
/// out of a ratio taken per pixel.
///
/// TWO claims, and the second is what keeps the first from being a restyle.
/// Against a CLOSED ring's share at the same pixel, a wide Octave gap keeps
/// nearly all of its light in the middle of a gap — where the nearest ink is
/// half a gap away, further off than the Gap reaches — and loses all of the
/// same share as the closed ring over the middle of a slice, the ink there
/// being no further off than the annulus itself. And with the gap closed the
/// share is FLAT around the turn, which is the picture an angular term must not
/// be able to touch: a dark band no setting on the node asked for.
///
/// Per pixel is what makes the tolerance on that flatness a tight one. The
/// probe ring lands on integer pixels, so its radius wobbles by up to half of
/// one, which on the fade's own slope is a couple of hundredths of the share —
/// hence a budget rather than an equality, and the ratio in the first claim,
/// which compares two shots at ONE pixel and so has no wobble in it at all.
///
/// [`the_gap_depth_says_how_much_light_a_ring_stands_off`]'s fixture and its
/// probe radius, with no fade on the clearing so the probe sits in solid
/// coverage and the share is the standoff's whole answer.
#[test]
fn the_standoff_follows_the_gaps_between_the_slices() {
    // A big frame for a small measurement: the Gap's fade is 0.16 of a node's
    // uv, so at 256 it spans some seven pixels and half of one of those is a
    // twentieth of the share below. The node is drawn at the same size in uv
    // whatever the frame, so the pixels are what buys the resolution.
    const SIZE: [u32; 2] = [1024, 1024];
    const GAP: f32 = 0.16;
    // Enough of them that one lands near the middle of a gap and one near the
    // middle of a slice, at every wheel the view can be dialled to.
    const ANGLES: usize = 360;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |octave_gap: f32, reach: f32, depth: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        // The fresh wheel's own FRINGE, which `OctaveLayout::default` leaves
        // off: with no extras every slice is one width and the walk reads a
        // uniform ladder, which a boundary table built by arithmetic rather
        // than read out of the uniform would satisfy just as well.
        //
        // What no wheel can pin is the DIRECTION the walk takes that table in.
        // `octave_layout` mirrors the fringe, so the bounds satisfy
        // `b[k] + b[span-k] = TAU` at every setting, and a min over all of them
        // is the same answer whichever way round the fragment's angle is
        // measured. That is a property of the wheel, not a hole in this fixture.
        scene.octave_layout = harmonigraph_scene::octave_layout(
            harmonigraph_scene::DEFAULT_COUNT,
            harmonigraph_scene::DEFAULT_CENTER,
            2,
            harmonigraph_scene::DEFAULT_EXTRA_SIZE,
            harmonigraph_scene::DEFAULT_EXTRA_BLEND,
        );
        scene.octave_gap = octave_gap;
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene.glow_gap = GAP;
        // The fade the whole width of the gap, which is the fresh pair.
        scene.glow_gap_soft = GAP;
        scene.glow_gap_depth = depth;
        scene.nodes[0].gutter = GAP;
        scene.sevens_soft = 0.0;
        scene
    };
    // The band's own outer edge is where this fixture's ink ends, and the Scene
    // names it.
    let ink_uv = at(0.0, 0.0, 0.0).rings_outer;
    let (closed, wide) =
        standoff_share_rings(&mut shooter, SIZE, &at, ink_uv, 0.5 * GAP, ANGLES);

    // The closed ring first, which is both the reference for the wide gap below
    // and a claim of its own.
    let mean = closed.iter().sum::<f64>() / ANGLES as f64;
    let drift = closed.iter().fold(0.0f64, |w, s| w.max((s - mean).abs()));
    assert!(
        mean > 0.15,
        "a closed ring took only {mean:.3} of the light at the probe; there is no \
         standoff here to find a gap in",
    );
    assert!(
        drift <= 0.05,
        "a closed ring's standoff swung by {drift:.3} around the turn, off a mean of \
         {mean:.3}: it is not standing the light off evenly the whole way round",
    );

    // And the wide gap, against that same pixel's share.
    let ratio: Vec<f64> = wide.iter().zip(&closed).map(|(w, c)| w / c).collect();
    let emptiest = ratio.iter().fold(f64::MAX, |m, r| m.min(*r));
    let deepest = ratio.iter().fold(f64::MIN, |m, r| m.max(*r));
    assert!(
        emptiest < 0.15,
        "the widest Octave gap still took {emptiest:.3} of the closed ring's share at \
         its emptiest angle: the standoff is not following the slices",
    );
    assert!(
        deepest > 0.85,
        "over a slice the widest Octave gap took only {deepest:.3} of the closed ring's \
         share: the standoff is following something narrower than the ink",
    );
}

/// A slice PAST A HALF TURN is ink all the way in to the node's centre down its
/// own middle, and the standoff follows it there.
///
/// The wheel hands one out at the bottom of its own bar: an octave count of 1
/// with the fresh two extras either side leaves the middle slice 259 degrees
/// (`octave_layout`). `outer_glyph` cuts each gap only on the side its edge runs
/// to, so down the middle of a slice that wide NO edge cuts anything, however
/// close the edges' own lines pass on their way through the centre — which is
/// why `oct_arc_coverage` carries a union branch for exactly this wedge.
///
/// A standoff measuring the distance to the nearest boundary RAY has to say the
/// same, and the reading that asks only "how far is the nearest ray" says the
/// opposite: near the centre every ray is close, so it calls the widest slice's
/// middle a gap and hands the light back exactly where the ink is.
///
/// Measured inside HALF the Octave gap, which is where the two readings can
/// disagree at all — further out than that, half a gap is spent before the
/// nearest ray is reached and both call it ink. The reading is the MAXIMUM
/// share around the turn, against a closed ring's share at the same pixel:
/// somewhere on that circle is the wide slice's middle, ink in both pictures, so
/// the two have to stand the light off there by the same amount. A per-pixel
/// ratio is also what makes the probe ring's half-pixel wobble cancel — where
/// the two shots agree on the shape, they agree whatever radius the pixel
/// landed at.
///
/// The AUDIO RING carries it, alone: the octave band is dialled off, so this is
/// also the only test that reaches `glow_standoff`'s ring term at all
/// (`parity_scene` is silent, and a silent ring has no radii to stand off).
#[test]
fn a_slice_past_a_half_turn_is_stood_off_down_its_middle() {
    const SIZE: [u32; 2] = [1024, 1024];
    // Small against the ring's own radius, so the probe sits well inside half
    // the Octave gap with the fade still spending most of itself there.
    const GAP: f32 = 0.05;
    const RING_OUTER: f32 = 0.15;
    const PAST: f32 = 0.01;
    const ANGLES: usize = 360;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |octave_gap: f32, reach: f32, depth: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        // One slice past a half turn, which is the whole fixture: the count at
        // its floor with the fresh fringe either side.
        scene.octave_layout = harmonigraph_scene::octave_layout(
            harmonigraph_scene::MIN_COUNT,
            harmonigraph_scene::DEFAULT_CENTER,
            2,
            harmonigraph_scene::DEFAULT_EXTRA_SIZE,
            harmonigraph_scene::DEFAULT_EXTRA_BLEND,
        );
        // The band off, so the ring is the only thing standing light off and
        // the only term the share below can be reading.
        scene.outer_inner = 0.0;
        scene.outer_outer = 0.0;
        // ...and the ring reaching the node's centre, which is what puts its
        // own footprint where the two readings differ.
        scene.spectral.inner = 0.0;
        scene.spectral.outer = RING_OUTER;
        scene.spectral.lut = std::array::from_fn(|k| {
            let t = k as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
            glam::Vec4::new(t, 0.6 * t, 1.0 - t, 1.0)
        });
        // Loud and FLAT, so every wedge reads the same: the ring lights the
        // halo by what it is measuring, and a comb would put the light's own
        // pattern into the share.
        scene.spectral.levels = Box::new([220; harmonigraph_scene::SPECTRAL_BUCKETS]);
        scene.octave_gap = octave_gap;
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene.glow_gap = GAP;
        scene.glow_gap_soft = GAP;
        scene.glow_gap_depth = depth;
        scene.nodes[0].gutter = 0.16;
        scene.sevens_soft = 0.0;
        scene
    };
    // Inside half the widest gap, which is the only radius at which the two
    // readings can differ at all.
    const {
        assert!(
            RING_OUTER + PAST < 0.5 * harmonigraph_scene::GAP_MAX,
            "the probe sits outside half an Octave gap, where every reading agrees",
        )
    };
    let (closed, wide) =
        standoff_share_rings(&mut shooter, SIZE, &at, RING_OUTER, PAST, ANGLES);

    let least = closed.iter().fold(f64::MAX, |m, s| m.min(*s));
    assert!(
        least > 0.3,
        "a closed ring took only {least:.3} of the light somewhere on the probe ring; \
         there is no standoff there to compare a wide gap against",
    );

    let ratio: Vec<f64> = wide.iter().zip(&closed).map(|(w, c)| w / c).collect();
    let deepest = ratio.iter().fold(f64::MIN, |m, r| m.max(*r));
    let emptiest = ratio.iter().fold(f64::MAX, |m, r| m.min(*r));
    assert!(
        deepest > 0.85,
        "down the middle of a 259-degree slice the widest Octave gap took only \
         {deepest:.3} of the closed ring's share: the standoff is reading ink as gap \
         where no edge cuts",
    );
    // Non-vacuous the other way: the narrow slices really are eaten at this
    // radius, so the ring the ratio is taken over is not simply solid.
    assert!(
        emptiest < 0.15,
        "every angle kept {emptiest:.3} or more of the closed ring's share; the wide \
         gap is not opening anywhere and the claim above is vacuous",
    );
}

/// A MARK's standoff stops where the gap cuts the mark's own sides.
///
/// The wedge a mark is drawn in is not the wedge its slot owns: `outer_glyph`
/// takes half an Octave gap off each of its sides, exactly as it does for the
/// slices of a ring. `sector_distance` measures the slot's wedge, so a standoff
/// reading it alone stands the light off from the BOUNDARY — half a gap wider
/// than the ink, on both sides of every mark.
///
/// The measurement that separates the two is the middle of a gap between two
/// marks: the nearest ink there is half an Octave gap away, and at the widest
/// gap that is 0.2 against a Gap of 0.16, so the light has to be fully back.
/// Read off the boundary the two wedges share, which is where the un-eroded
/// wedge puts its own edge and so reads as a distance of nothing.
///
/// Every slot marked, and the band and the ring both dialled off: the strip is
/// then a full ring of wedges cut by the one gap, which is what lets the same
/// probe ring read it, and the marks are the only term `glow_standoff` has.
#[test]
fn a_marks_standoff_stops_where_the_gap_cuts_its_sides() {
    const SIZE: [u32; 2] = [1024, 1024];
    const GAP: f32 = 0.16;
    const STRIP_IN: f32 = 0.5;
    const STRIP_THICK: f32 = 0.12;
    const ANGLES: usize = 360;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |octave_gap: f32, reach: f32, depth: f32| -> Scene {
        // Every slot the wheel can show, so the strip closes into a ring.
        let mut scene = single_marked_node((1 << harmonigraph_scene::OCTAVE_SLOTS) - 1, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.octave_layout = harmonigraph_scene::octave_layout(
            harmonigraph_scene::DEFAULT_COUNT,
            harmonigraph_scene::DEFAULT_CENTER,
            2,
            harmonigraph_scene::DEFAULT_EXTRA_SIZE,
            harmonigraph_scene::DEFAULT_EXTRA_BLEND,
        );
        // The band and the ring off, so the marks are the only thing standing
        // any light off and the only term the share can be reading.
        scene.outer_inner = 0.0;
        scene.outer_outer = 0.0;
        scene.mark_inner = STRIP_IN;
        scene.mark_thickness = STRIP_THICK;
        scene.octave_gap = octave_gap;
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene.glow_gap = GAP;
        scene.glow_gap_soft = GAP;
        scene.glow_gap_depth = depth;
        scene.nodes[0].gutter = GAP;
        scene.sevens_soft = 0.0;
        scene
    };
    // Half an Octave gap has to outreach the Gap, or the light is held off in
    // the middle of a gap whatever the sides do.
    const {
        assert!(
            0.5 * harmonigraph_scene::GAP_MAX > GAP,
            "the widest gap is too narrow for its middle to be clear of the ink",
        )
    };
    let (closed, wide) = standoff_share_rings(
        &mut shooter,
        SIZE,
        &at,
        STRIP_IN + STRIP_THICK,
        0.5 * GAP,
        ANGLES,
    );

    let least = closed.iter().fold(f64::MAX, |m, s| m.min(*s));
    assert!(
        least > 0.3,
        "a closed strip took only {least:.3} of the light somewhere on the probe ring; \
         there is no standoff there for a gap to open",
    );
    let ratio: Vec<f64> = wide.iter().zip(&closed).map(|(w, c)| w / c).collect();
    let emptiest = ratio.iter().fold(f64::MAX, |m, r| m.min(*r));
    let deepest = ratio.iter().fold(f64::MIN, |m, r| m.max(*r));
    assert!(
        emptiest < 0.15,
        "between two marks the widest Octave gap still took {emptiest:.3} of the closed \
         strip's share: the standoff is measuring the slot's wedge rather than the ink \
         drawn in it",
    );
    assert!(
        deepest > 0.85,
        "over a mark the widest Octave gap took only {deepest:.3} of the closed strip's \
         share: the standoff is following something narrower than the mark",
    );
}

/// A ring WEARS THE WASH inside a pool the Gap depth has cleared to the bare
/// ground: the two are one field asked for twice, and the answers are free of
/// each other.
///
/// The look the bar exists for. On one coupled dial a dark pool and a tinted
/// ring are mutually exclusive — the light the ink would wear is exactly the
/// light the standoff takes — so the first two claims below are measured at a
/// DEPTH OF 1, where the ground around the ring is the frame with no glow in it
/// at all and there is nothing left of the pool's light to tint anything with.
///
/// Three claims, and the third is the decoupling itself:
///
/// - A wash of 0 leaves the ink byte for byte what it is with the glow off,
///   whatever light is standing at it. Byte-identical rather than nearly so
///   because a factor of 0 on the light is no light: nothing is left to round.
/// - A wash of 1 lifts it, and lifts more than half of it.
/// - Moving the DEPTH moves the ink not at all, the wash reading the field
///   before the standoff's factor reaches it. A wash carried on the standoff's
///   remainder instead cannot say this at all, and that is the reason there is
///   a second bar.
///
/// Every lift is measured as a lift and never a loss, which is the wash's own
/// arithmetic (`node_paint`): the ink takes the light as a screen, so every
/// channel it moves it moves up.
///
/// [`the_gap_depth_says_how_much_light_a_ring_stands_off`]'s fixture, whose
/// probe is the other side of this boundary — that pixel is outside the node's
/// ink and these are inside it, and neither bar answers for both.
///
/// The ink is found on the GROUND, as in
/// [`a_node_under_a_nearer_sheets_node_cuts_nothing_out_of_its_light`]: a pixel
/// the node paints opaquely is the pixel that does not move when the ground
/// does.
///
/// That set is not exact, and the third claim is stated to survive it: a pixel
/// the node paints at an alpha a hair under 1 carries a SUB-LSB sliver of
/// ground, which a black-and-white probe rounds away and the depth still moves.
/// The BOUND follows from how the set is chosen rather than from tuning —
/// agreeing over both grounds forces that sliver's coefficient under 1/255, and
/// the sliver is the only term the depth touches on such a pixel, so one byte
/// is the most it can carry. A wash reading the standoff's remainder would move
/// the ink by the light's own size instead, which is the scale the shot beside
/// it supplies.
#[test]
fn a_ring_wears_the_wash_inside_its_own_dark_pool() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |reach: f32, depth: f32, wash: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.camera = harmonigraph_scene::Camera {
            projection: harmonigraph_scene::Projection::Orthographic,
            yaw: 0.0,
            pitch: 0.0,
            ..Default::default()
        };
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene.glow_gap = 0.16;
        // The fade the whole width of the gap, which is the fresh pair.
        scene.glow_gap_soft = 0.16;
        scene.glow_gap_depth = depth;
        scene.glow_wash = wash;
        scene.nodes[0].gutter = 0.16;
        scene
    };
    let mut on_ground = |bg: glam::Vec4| {
        let mut scene = at(0.0, 1.0, 0.0);
        scene.background = bg;
        shooter.shot(&scene)
    };
    let dark_ground = on_ground(glam::Vec4::new(0.0, 0.0, 0.0, 1.0));
    let pale_ground = on_ground(glam::Vec4::ONE);
    let ink: Vec<usize> = (0..pale_ground.len())
        .step_by(4)
        .filter(|&i| {
            pale_ground[i..i + 4] != [0u8, 0, 0, 255]
                && pale_ground[i..i + 4] == dark_ground[i..i + 4]
        })
        .collect();
    assert!(ink.len() > 500, "the node painted {} opaque pixels", ink.len());

    let off = shooter.shot(&at(0.0, 1.0, 1.0));
    let dry = shooter.shot(&at(0.8, 1.0, 0.0));
    let worn = shooter.shot(&at(0.8, 1.0, 1.0));
    let open = shooter.shot(&at(0.8, 0.0, 1.0));
    let moved = ink.iter().filter(|&&i| dry[i..i + 4] != off[i..i + 4]).count();
    assert_eq!(
        moved,
        0,
        "with no wash the glow reached {moved} of the ring's {} opaque pixels",
        ink.len(),
    );
    let lifted = ink
        .iter()
        .filter(|&&i| brightness(&worn[i..i + 3]) > brightness(&off[i..i + 3]))
        .count();
    assert!(
        lifted * 2 > ink.len(),
        "inside a pool cleared to the bare ground, a full wash lifted {lifted} of the ring's {} \
         opaque pixels",
        ink.len(),
    );
    let dimmed =
        ink.iter().filter(|&&i| (0..3).any(|c| worn[i + c] < off[i + c])).count();
    assert_eq!(
        dimmed,
        0,
        "the wash took light off {dimmed} of the ring's {} opaque pixels",
        ink.len(),
    );
    // The furthest any one channel of the ink moves between two shots, which is
    // what both halves of the last claim are read in.
    let spread = |a: &[u8], b: &[u8]| {
        ink.iter()
            .map(|&i| (0..3).map(|c| a[i + c].abs_diff(b[i + c])).max().unwrap())
            .max()
            .unwrap()
    };
    let by_wash = spread(&worn, &dry);
    let by_depth = spread(&worn, &open);
    assert!(
        by_wash > 20,
        "the fixture's wash moves the ink by {by_wash}; there is nothing here to be free of",
    );
    assert!(
        by_depth <= 1,
        "dropping the depth moved the ink by {by_depth} against the wash's own {by_wash}: the \
         wash is reading the standoff's remainder rather than the field",
    );
}

/// A node wearing NOTHING BUT AN AUDIO RING glows.
///
/// Two halves, and each is a thing the first cut of this got wrong. The LEVEL
/// is the largest of everything that draws ink, not the note's activation — a
/// ring is the analyzer's reading rather than a voice's, so a node with no key
/// down carries an activation of 0 and a ring at full. And the COLOUR falls
/// back to the node's own pitch, because the octave word a chord's light is
/// blended out of is empty here: there is no voice to take a hue from.
///
/// The ring is the one layer an idle node paints, so this is also the case that
/// says the glow follows every layer that LIGHTS the node rather than the keys
/// alone. What it does not say is that drawing a layer is enough — see
/// [`a_ring_reading_nothing_gives_off_no_light`], where the same ring with
/// nothing in it gives off nothing.
#[test]
fn a_node_wearing_only_an_audio_ring_glows() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |reach: f32| -> Scene {
        // [`ringing_node`]'s fixture: a live analyzer, a partial sounding into
        // one wedge, and no core at all — so the only thing on screen is the
        // ring, and the only light around it is the glow's.
        let mut scene =
            ringing_node(None, Some(harmonigraph_scene::MIDDLE_C_SLOT as f32 * 12.0), PROBE_RANGE);
        let node = &mut scene.nodes[0];
        // Silence at the node, and the analyzer still reading: no key down, no
        // octave sounding, no mark at either end — and the ring at full, which
        // is the view's Gate answered for this node.
        node.activation = 0.0;
        node.melody_level = 0.0;
        node.bass_level = 0.0;
        node.melody_slots = 0;
        node.bass_slots = 0;
        node.audio_ring = 1.0;
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene
    };
    let off = shooter.shot(&at(0.0));
    let on = shooter.shot(&at(0.8));
    // The fixture draws its ring with no note at all, or "the ring alone glows"
    // is being asked of a frame with no ring in it.
    assert!(total_light(&off) > 0, "the fixture must draw its audio ring");
    assert!(
        total_light(&on) > total_light(&off),
        "a ringing node with no note gave off no light: {} against {}",
        total_light(&on),
        total_light(&off),
    );
    // And OUTSIDE the ring, which is where a halo is: the ring's own annulus is
    // drawn over the light, so what shows there is the ring rather than a glow.
    let row = (SIZE[1] / 2) as usize;
    let outside = (SIZE[0] as usize / 2..SIZE[0] as usize)
        .filter(|&x| {
            let i = row * SIZE[0] as usize + x;
            brightness(&on[i * 4..i * 4 + 3]) > brightness(&off[i * 4..i * 4 + 3])
        })
        .count();
    assert!(
        outside >= 8,
        "the ring's light reached {outside} pixels along the row out from the node",
    );
}

/// Two nodes' halos MELD where they overlap: brighter than either alone, and
/// bounded — screen (`src + dst * (1 - src)`), not a sum.
///
/// The two claims have to be made together, and the second is the one that
/// says which blend is in the pipeline: any blend that adds light at all
/// passes the first, and `a + b` passes it too while blowing a chord's middle
/// out to white. Screen is strictly under the sum wherever both sides are lit,
/// which is the discriminator this measures.
///
/// At the Meld bar's TOP, which is where the fixture leaves it: that bar dials
/// this blend against the max beside it, and what a melded overlap is is this
/// end of it (`the_meld_says_how_much_two_nodes_overlapping_light_adds_up`).
#[test]
fn two_nodes_light_melds_rather_than_summing() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The fixture's node, moved off center, and its mirror image — so the pair
    // straddles the origin the camera is pointed at and their halos cross in
    // between. Far enough apart that neither node's own layers reach the other
    // (the rim is well under one uv unit, which is 1.8 world units here).
    let scene_of = |xs: &[f32]| -> Scene {
        let mut scene = single_marked_node(0, 0);
        let node = scene.nodes[0];
        scene.nodes = xs
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let mut n = node;
                n.world_pos = glam::Vec3::new(*x, 0.0, 0.0);
                n.lattice_pos = harmonigraph_core::LatticePos::new(i as i32, 0, 0);
                n
            })
            .collect();
        rows_per_node(&mut scene);
        scene.glow_reach = 0.8;
        // High on its bar, so the overlap is a reading with room in it rather
        // than three quantization steps: the claim is about the SHAPE of the
        // blend, and at four levels of grey any blend passes.
        scene.glow_strength = 2.0;
        scene
    };
    const APART: f32 = 1.8;
    let left = shooter.shot(&scene_of(&[-APART]));
    let right = shooter.shot(&scene_of(&[APART]));
    let both = shooter.shot(&scene_of(&[-APART, APART]));

    // Where the two overlap most: the pixel whose dimmer half is brightest.
    // Found rather than named, so the probe follows the camera instead of a
    // pixel coordinate that a change of fixture would quietly move off the
    // overlap.
    let at = |shot: &[u8], i: usize| -> [u8; 3] {
        std::array::from_fn(|c| shot[i * 4 + c])
    };
    let probe = (0..(SIZE[0] * SIZE[1]) as usize)
        .max_by_key(|&i| {
            let (l, r) = (at(&left, i), at(&right, i));
            brightness(&l).min(brightness(&r))
        })
        .expect("a non-empty frame");
    let (l, r, b) = (at(&left, probe), at(&right, probe), at(&both, probe));
    assert!(
        brightness(&l) > 24 && brightness(&r) > 24,
        "the two halos never overlapped: {l:?} and {r:?} at pixel {probe}",
    );
    assert!(
        brightness(&b) > brightness(&l).max(brightness(&r)),
        "the overlap {b:?} is no brighter than the brighter half ({l:?}, {r:?})",
    );
    for c in 0..3 {
        let (l, r, b) = (i32::from(l[c]), i32::from(r[c]), i32::from(b[c]));
        assert!(b <= 255, "a channel left the range: {b}");
        // Screen is strictly under the sum wherever both sides carry light;
        // the slack is 8-bit rounding on three composited values.
        if l > 25 && r > 25 {
            assert!(
                b < l + r - 1,
                "channel {c} summed rather than melded: {b} against {l} + {r}",
            );
        }
    }
}

/// One node lit in TWO colours no mixing of the other's table can reach: the
/// pitch ramp flat RED, so every slice of the octave band is red however its
/// octave is voiced, and the analyzer's ramp flat GREEN, so every wedge of the
/// audio ring is green whatever the grid holds.
///
/// The suite's usual ramps — a blue-to-red pitch sweep and a grey spectral one —
/// overlap in every channel, so a halo drawn out of both would answer "somewhere
/// between" and say nothing about which layer coloured it. With one channel
/// apiece the halo's red against its green IS the two layers' share of it, which
/// is what every claim below reads.
///
/// The core and the marks are off: two layers under test and nothing else
/// putting ink on the node. Every octave sounds, so the band is one red ring
/// rather than a lit slice among ghosts, and the node wears its ring in full.
fn two_colour_node(band_width: f32, ring_width: f32) -> Scene {
    let fresh = harmonigraph_scene::ViewConfig::default();
    let mut scene = single_marked_node(0, 0);
    // The probe's wide padding, exactly as [`ringing_node`] uses it: the angular
    // gap is a constant chord, so a ring packed against the node's centre has
    // its wedges eaten by that gap and paints almost nothing — which would make
    // the ring's share of the light a reading of the padding rather than of its
    // width.
    let rings = harmonigraph_scene::ViewConfig {
        ring_inner: PROBE_INNER,
        ring_gap: PROBE_GAP,
        spectral_ring_width: ring_width,
        band_width,
        mark_thickness: 0.0,
        ..fresh.clone()
    }
    .rings();
    scene.mark_thickness = 0.0;
    scene.outer_inner = rings.band.0;
    scene.outer_outer = rings.band.1;
    scene.rings_outer = rings.outer;
    scene.mark_inner = rings.mark_inner;
    // A NARROW angular gap, where the radial one is the probe's wide one: the
    // sector gap is a constant Euclidean chord, so at the radii the innermost
    // ring occupies — it reaches the node's centre — the probe's own 0.12 would
    // blank the ring's wedges outright and the light would carry none of its
    // colour.
    scene.octave_gap = 0.03;
    scene.pitch_lut = [glam::Vec4::new(1.0, 0.0, 0.0, 1.0); harmonigraph_scene::PITCH_LUT_N];

    let mut paint = harmonigraph_scene::SpectralPaint::silent();
    // FLAT rather than a ramp, so the ring's colour is the ring's colour at
    // whatever the analyzer reads: what is being measured here is which layer
    // the light took its hue from, not what the grid held.
    paint.lut = [glam::Vec4::new(0.0, 1.0, 0.0, 1.0); harmonigraph_scene::PITCH_LUT_N];
    // ...and a grid that is LOUD everywhere, which is what makes the wedges
    // light at all: the halo weighs a wedge by the reading behind it (`ink_at`),
    // so a silent grid draws a full green ring that gives off nothing. Loud
    // across the whole axis rather than a partial at one pitch, so every wedge
    // carries the same weight and the ring's share of the light is its WIDTH,
    // which is what these claims read.
    paint.levels.fill(255);
    (paint.inner, paint.outer) = rings.audio;
    paint.range = PROBE_RANGE;
    scene.spectral = paint;

    let node = &mut scene.nodes[0];
    node.octaves = [1.0; harmonigraph_scene::OCTAVE_SLOTS];
    node.activation = 1.0;
    node.audio_ring = 1.0;
    (node.melody_slots, node.bass_slots) = (0, 0);
    (node.melody_level, node.bass_level) = (0.0, 0.0);

    scene.glow_reach = 0.8;
    scene.glow_strength = 1.5;
    scene
}

/// The light one shot ADDED over the same frame with the glow off, summed per
/// channel over the whole frame.
///
/// Everything the node itself draws stands in both shots and cancels, so what
/// is left is the halo's own colour — which is what a claim about where the
/// light took its hue from has to read. Clamped at 0 per channel because the
/// glow also takes the core's skirt away, and a channel that went DOWN is that
/// subtraction rather than any hue.
fn added_light(on: &[u8], off: &[u8]) -> [i64; 3] {
    let mut sum = [0i64; 3];
    for (a, b) in on.chunks(4).zip(off.chunks(4)) {
        for (c, s) in sum.iter_mut().enumerate() {
            *s += (i64::from(a[c]) - i64::from(b[c])).max(0);
        }
    }
    sum
}

/// The glow's colour is the node's own INK, whatever layer laid it down.
///
/// Three nodes, one code path: a node wearing nothing but its audio ring lights
/// in the ring's colour, a node wearing nothing but its octave band lights in
/// the band's, and a node wearing both lights in a mixture that is greener than
/// the one and redder than the other. Nothing here names a layer — the light is
/// `ink_at` read round the node — so the ring's hue reaching the halo and the
/// band's reaching it are the same mechanism, and a layer added to a node is
/// lit without a line of its own.
///
/// See [`two_colour_node`] for why the two ramps are one channel apiece.
#[test]
fn a_nodes_light_takes_the_colour_of_whichever_layer_is_drawing() {
    const SIZE: [u32; 2] = [256, 256];
    const BAND: f32 = 0.16;
    const RING: f32 = 0.16;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let dark = |mut scene: Scene| -> Scene {
        scene.glow_reach = 0.0;
        scene
    };
    // The ring alone: no key down and no octave sounding, so the band draws
    // nothing at all and the node's whole picture is the analyzer's.
    let ring_only = || -> Scene {
        let mut scene = two_colour_node(BAND, RING);
        let node = &mut scene.nodes[0];
        node.activation = 0.0;
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        scene
    };
    // The band alone: the ring's width dialled to nothing, which is the layer's
    // own off switch.
    let band_only = || two_colour_node(BAND, 0.0);

    let ring = added_light(&shooter.shot(&ring_only()), &shooter.shot(&dark(ring_only())));
    let band = added_light(&shooter.shot(&band_only()), &shooter.shot(&dark(band_only())));
    let both = added_light(
        &shooter.shot(&two_colour_node(BAND, RING)),
        &shooter.shot(&dark(two_colour_node(BAND, RING))),
    );

    assert!(ring[1] > 0 && band[0] > 0, "neither fixture lit anything: {ring:?}, {band:?}");
    assert!(
        ring[1] > ring[0] * 4,
        "a node wearing only its audio ring lit {ring:?} — its light is not the ring's green",
    );
    assert!(
        band[0] > band[1] * 4,
        "a node wearing only its octave band lit {band:?} — its light is not the band's red",
    );
    // And the two together are a mixture rather than either one winning: the
    // SHARE is what moves, the three halos not being the same size.
    let share = |c: [i64; 3]| c[0] as f64 / (c[0] + c[1]).max(1) as f64;
    assert!(
        share(ring) < share(both) && share(both) < share(band),
        "a node wearing both lit {both:?}, which is not between {ring:?} and {band:?}",
    );
}

/// A slice the node is not sounding puts NO ground in its light.
///
/// A note voiced in a single octave lights one slice of the octave band and
/// leaves the rest of the wheel ghosts — eight of them at the fresh view's
/// span of nine, four at this fixture's five — and a ghost is
/// `Scene::lattice_ground` flat and opaque. Weighing the band by its INK
/// therefore hands that note a halo that is mostly grey, with its own pitch a
/// lobe inside it. The light weighs each slice by its LEVEL instead (`ink_at`,
/// through `oct_slot_lit`), so the halo is the octave's own colour and the
/// ghosts are a thing drawn rather than a thing shining.
///
/// The ground is set to pure BLUE here, which is a colour the view cannot
/// actually hold — `grey_of_lightness` is what fills that field in the app —
/// and that is the point: the pitch ramp is flat red, so a blue channel in the
/// halo can only have come from the ghosts, and no mixture of the one is
/// reachable from the other. The ghosts themselves are checked to be on
/// screen, or the claim is being made about a node that has none.
#[test]
fn a_silent_slice_puts_none_of_its_ground_in_the_light() {
    const SIZE: [u32; 2] = [256, 256];
    const BAND: f32 = 0.18;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The band alone — the ring dialled to nothing — voiced in ONE octave, so
    // every other slice on the wheel is a ghost.
    let one_octave = || -> Scene {
        let mut scene = two_colour_node(BAND, 0.0);
        scene.lattice_ground = glam::Vec4::new(0.0, 0.0, 1.0, 1.0);
        let node = &mut scene.nodes[0];
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.octaves[harmonigraph_scene::MIDDLE_C_SLOT] = 1.0;
        scene
    };
    let dark = || -> Scene {
        let mut scene = one_octave();
        scene.glow_reach = 0.0;
        scene
    };
    let off = shooter.shot(&dark());
    // Non-vacuous: the ghosts have to be ON the node, or "the light carries
    // none of them" is a claim about a ring that is not there.
    let ghosts: i64 = off.chunks(4).map(|px| i64::from(px[2])).sum();
    let red: i64 = off.chunks(4).map(|px| i64::from(px[0])).sum();
    assert!(ghosts > red, "the fixture drew no blue ghosts: {ghosts} against {red} of red");

    let lit = added_light(&shooter.shot(&one_octave()), &off);
    assert!(lit[0] > 0, "the node's one lit octave lit nothing: {lit:?}");
    // Four slices of five are the ground here, so weighing the band by its ink
    // puts four times as much of that grey in the halo as of the pitch, and a
    // factor of eight is well clear of either reading.
    assert!(
        lit[0] > lit[2] * 8,
        "a note voiced in one octave lit {lit:?} — the halo is carrying its ghosts",
    );
}

/// A slice part way through its envelope carries that much of the light, and
/// no more.
///
/// The one place the weight is neither what it was nor zero, and the reason
/// the drawn ink cannot be asked for it: a slice's OPACITY is the node's
/// presence, with the ghost filling in whatever its own level does not
/// account for. So a pitch class held in one octave while another octave
/// releases draws both slices fully opaque, and weighing the band by its ink
/// hands the releasing one a full share of the light for the whole of its
/// release — in a colour that is itself part ghost by then.
///
/// THREE channels, one per thing that could be in the halo: the ground pure
/// green, and a pitch ramp that is pure blue under the two slices' midpoint
/// and pure red over it, so the held slice is red, the releasing one blue, and
/// any ghost that reaches the light is green. None of the three is reachable
/// from the others.
///
/// Green is the discriminator — a ghost weighs nothing at any level, so the
/// halo has none of it while the slice is half out. The blue share falling
/// with the slice's own level is the positive claim, over three levels rather
/// than two so that it is the ENVELOPE being followed and not a switch at 0.
#[test]
fn a_slice_part_way_out_carries_that_much_of_the_light() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let beside = slot_beside_middle_c().trailing_zeros() as usize;
    let lit = harmonigraph_scene::MIDDLE_C_SLOT;
    assert_ne!(beside, lit, "the two slices have to be different slices");
    // The node fully PRESENT throughout — that is what makes both slices
    // opaque and the ink weight blind to which of them is sounding.
    let at = |releasing: f32, reach: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.lattice_ground = glam::Vec4::new(0.0, 1.0, 0.0, 1.0);
        // Split at the two slices' midpoint: one octave either side of it, so
        // each wears one end of the ramp whole and neither is a blend.
        let (dark, bright) = (scene.darkest_pitch, scene.brightest_pitch);
        let mid = (harmonigraph_scene::MIDDLE_C_SLOT + beside) as f32 * 6.0;
        let split = (mid - dark) / (bright - dark);
        scene.pitch_lut = std::array::from_fn(|k| {
            let t = k as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
            if t < split {
                glam::Vec4::new(0.0, 0.0, 1.0, 1.0)
            } else {
                glam::Vec4::new(1.0, 0.0, 0.0, 1.0)
            }
        });
        let node = &mut scene.nodes[0];
        node.activation = 1.0;
        node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        node.octaves[lit] = 1.0;
        node.octaves[beside] = releasing;
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene
    };
    // The two slices sit either side of the split, so which of them is red and
    // which blue follows from their order rather than being assumed.
    let (held_ch, going_ch) = if beside > lit { (2, 0) } else { (0, 2) };
    let mut halo = |releasing: f32| -> [i64; 3] {
        added_light(&shooter.shot(&at(releasing, 0.8)), &shooter.shot(&at(releasing, 0.0)))
    };
    let (held, half, gone) = (halo(1.0), halo(0.5), halo(0.0));
    eprintln!("halo r/g/b — releasing 1.0 {held:?}, 0.5 {half:?}, 0.0 {gone:?}");
    // Non-vacuous: the two slices do light the halo in their own colours.
    assert!(
        held[held_ch] > 0 && held[going_ch] > 0,
        "both slices sounding lit {held:?}, which is not two colours",
    );
    // The ghost never reaches the light — not while the slice is HALF out,
    // which is where the drawn ink is half ground and fully weighed.
    for (what, c) in [("both sounding", held), ("half out", half), ("gone", gone)] {
        let colour = c[0] + c[2];
        assert!(
            c[1] * 8 < colour,
            "with the slice {what} the halo carried {} of ground against {colour} of pitch: {c:?}",
            c[1],
        );
    }
    // And the share follows the envelope.
    let share = |c: [i64; 3]| c[going_ch] as f64 / (c[0] + c[2]).max(1) as f64;
    let (a, b, d) = (share(held), share(half), share(gone));
    assert!(
        a > b && b > d,
        "the light did not follow the releasing slice's level: {a:.4} / {b:.4} / {d:.4}",
    );
}

/// A node the audio ring is showing on with NOTHING sounding in it gives off no
/// light at all.
///
/// The ring's colour ramp is pinned to the ground at its silent end, so a wedge
/// reading nothing is that same grey — and a lattice whose Gate admits every
/// node is hundreds of those. They are worth DRAWING (the ring says a node is
/// there) and worth no light, so the analyzer's share of the halo is weighed by
/// the reading behind each wedge (`ink_at`) and a ring of empty ones sums to
/// nothing. `glow_layer` stops there rather than lighting a grey halo.
///
/// Byte-identical, which is the whole claim: the light's draw runs over this
/// node, writes nothing into its target, and the composite lays exactly
/// nothing over the picture. The same fixture with a partial in it is shot
/// beside it, or "no light" would pass on a ring that never drew.
#[test]
fn a_ring_reading_nothing_gives_off_no_light() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // [`ringing_node`] with no key down and no octave sounding: the band draws
    // nothing at all and the node's whole picture is the analyzer's.
    let at = |sounding: Option<f32>, reach: f32| -> Scene {
        let mut scene = ringing_node(None, sounding, PROBE_RANGE);
        // The app's ramp rather than the fixture's: its silent end PINNED to
        // the ground, which is what makes an empty wedge a grey the eye reads
        // as a ring rather than the black the probe's own ramp starts at
        // (`harmonigraph_scene::ring_gradient`). The whole point here is a ring
        // that is on screen and gives off nothing, so a silent end nobody can
        // see would make the claim vacuous.
        let ground = scene.lattice_ground;
        scene.spectral.lut = std::array::from_fn(|k| {
            let t = k as f32 / (harmonigraph_scene::PITCH_LUT_N - 1) as f32;
            ground.lerp(glam::Vec4::ONE, t)
        });
        let node = &mut scene.nodes[0];
        node.activation = 0.0;
        node.melody_level = 0.0;
        node.bass_level = 0.0;
        node.melody_slots = 0;
        node.bass_slots = 0;
        node.audio_ring = 1.0;
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene
    };
    const PARTIAL: f32 = harmonigraph_scene::MIDDLE_C_SLOT as f32 * 12.0;
    let quiet_off = shooter.shot(&at(None, 0.0));
    let quiet_on = shooter.shot(&at(None, 0.8));
    // Non-vacuous: the silent ring is ON SCREEN. Against the layer's own off
    // switch rather than against `total_light`, which the node's clearing
    // alone would satisfy — and safe as a one-layer diff only because this
    // node draws nothing else: no key, no octave, no mark, so there is no
    // layer outside the ring for its width to slide inward (the stack packs
    // outward from the centre).
    let mut ringless = at(None, 0.0);
    (ringless.spectral.inner, ringless.spectral.outer) = (0.0, 0.0);
    assert!(
        quiet_off != shooter.shot(&ringless),
        "the fixture drew no audio ring, so it cannot say a silent one gives off nothing",
    );
    let loud_off = shooter.shot(&at(Some(PARTIAL), 0.0));
    let loud_on = shooter.shot(&at(Some(PARTIAL), 0.8));
    assert!(
        total_light(&loud_on) > total_light(&loud_off),
        "a partial sounding into the ring gave off no light: {} against {}",
        total_light(&loud_on),
        total_light(&loud_off),
    );
    assert!(
        quiet_on == quiet_off,
        "a ring reading silence lit {} against {} with the glow off",
        total_light(&quiet_on),
        total_light(&quiet_off),
    );
}

/// How much of the light a layer's colour owns is how much of the NODE that
/// layer occupies: the same node with its octave band twice as wide glows
/// redder — the band's colour — than it does at the narrower width.
///
/// No knob of its own. The weight in `ink_at` is the radial width the ring
/// stack handed the layer, so this follows the Layers bar directly: widen a
/// ring and its colour takes more of the halo, dial it to nothing and it takes
/// none.
///
/// The audio ring is held at one width and sits INSIDE the band, so widening
/// the band leaves the ring's own radii exactly where they were — what changes
/// is the share, not the other layer.
#[test]
fn widening_a_layer_gives_its_colour_more_of_the_light() {
    const SIZE: [u32; 2] = [256, 256];
    const RING: f32 = 0.16;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |band: f32| -> Scene { two_colour_node(band, RING) };
    let dark = |band: f32| -> Scene {
        let mut scene = at(band);
        scene.glow_reach = 0.0;
        scene
    };
    let narrow = added_light(&shooter.shot(&at(0.11)), &shooter.shot(&dark(0.11)));
    let wide = added_light(&shooter.shot(&at(0.22)), &shooter.shot(&dark(0.22)));
    // Both layers are drawing in both shots, or "the share moved" is one of
    // them arriving rather than the widths being read.
    for (what, c) in [("narrow", narrow), ("wide", wide)] {
        assert!(c[0] > 0 && c[1] > 0, "the {what} shot lit {c:?} — one layer is missing");
    }
    let share = |c: [i64; 3]| c[0] as f64 / (c[0] + c[1]).max(1) as f64;
    assert!(
        share(wide) > share(narrow) + 0.05,
        "doubling the band's width moved the light's red share from {:.3} to {:.3}",
        share(narrow),
        share(wide),
    );
}

/// The light carries no ripple the ink it is read from cannot hold.
///
/// This is the artefact the strip exists to remove, and it has a name: the
/// colour used to be averaged at twelve FIXED angles per fragment, so anything
/// the ink held near that rate came through the average intact, and every node
/// wore a fan of dark spokes converging on its middle. Nothing in the picture
/// has twelve-fold symmetry — the wheel is cut into at most eleven — so the
/// ripple could only be the sampling.
///
/// Measured as ANGULAR HARMONICS of the light's brightness round a circle,
/// because that is what a spoke is: a ripple that goes round the node a whole
/// number of times. The band under test starts above what a blurred ink can
/// hold — the tightest lobe the Color blend bar reaches is GLOW_LOBE_KAPPA, whose
/// von Mises coefficients are already under a thousandth of the mean by the
/// eighth harmonic — so anything found there is the machinery and not the node.
/// At fbc6cd5 the twelfth carried 12% to 17% of the mean at every radius inside
/// the node; the bound below is a quarter of that.
///
/// A node wearing NOTHING BUT ITS AUDIO RING, which is the case the spokes were
/// worst in, and the ink is the reason: the ring is cut into a wedge per
/// octave, each reading the analyzer at its own pitch, which is the sharpest
/// angular structure a node draws. The Spread is at the bottom of its bar,
/// where the blur is tightest and a sampled one has the least room to hide.
#[test]
fn a_nodes_light_has_no_ripple_the_ink_does_not() {
    const SIZE: [u32; 2] = [512, 512];
    // Every radius here is out past the ring's own annulus (0..45 px at this
    // node size), where the light is all there is and a spoke has nothing to
    // hide behind. The ring reaches the node's centre, being the innermost
    // layer of the stack, so there is no middle to read inside it.
    const RADII: [f32; 5] = [55.0, 70.0, 85.0, 100.0, 115.0];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |reach: f32| -> Scene {
        // [`ringing_node`]'s fixture with the note taken out of it: a live
        // analyzer, a partial sounding into one wedge, and no key down, so the
        // only ink on the node is the ring's and the only light is the glow's.
        let mut scene =
            ringing_node(None, Some(harmonigraph_scene::MIDDLE_C_SLOT as f32 * 12.0), PROBE_RANGE);
        let node = &mut scene.nodes[0];
        node.activation = 0.0;
        node.melody_level = 0.0;
        node.bass_level = 0.0;
        node.melody_slots = 0;
        node.bass_slots = 0;
        node.audio_ring = 1.0;
        scene.glow_reach = reach;
        scene.glow_strength = 1.5;
        scene.glow_blend = 0.0;
        // Big enough that a circle inside the node is hundreds of pixels round,
        // which is what resolving a ripple at these rates takes.
        scene.node_radius = 1.6;
        scene
    };
    let off = shooter.shot(&at(0.0));
    let on = shooter.shot(&at(0.8));

    for radius in RADII {
        let lit = ring_profile(&on, SIZE, radius);
        let dark = ring_profile(&off, SIZE, radius);
        let mean = |p: &[f64]| p.iter().sum::<f64>() / p.len() as f64;
        // Non-vacuous first: there has to BE light on this circle, or a dark
        // frame passes every bound below.
        let (bright, unlit) = (mean(&lit), mean(&dark));
        assert!(
            bright > unlit + 4.0,
            "the fixture puts no light at {radius} px: {bright:.1} against {unlit:.1} unlit",
        );
        // The ripple, band by band. Against the mean because that is what a
        // spoke reads as — a dip against the light around it.
        for k in 8..=32 {
            let ripple = harmonic(&lit, k) / bright;
            assert!(
                ripple < 0.03,
                "the light ripples {:.1}% of its own brightness {k} times round the node at \
                 {radius} px — nothing the node draws is cut that fine, so it is the sampling",
                ripple * 100.0,
            );
        }
        // ...and the same thing said the plain way: neighbouring samples round
        // the circle are within a step of each other. Cruder, since 8-bit
        // rounding is most of what is left in it, and here because a spoke is
        // something you SEE rather than a coefficient — at fbc6cd5 this reads
        // 2 to 3 at every radius above.
        let step = lit.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f64, f64::max);
        assert!(
            step < 0.75,
            "the light steps {step:.2}/255 between neighbouring samples round the node at \
             {radius} px",
        );
    }
}

/// The ink strip is as tall as the scene says it is, however that changes —
/// asked across a frame that ADDS a node, on the pane that drew the frame
/// before it.
///
/// A node's row is handed out by the light's own clock and the scene carries
/// the height that goes with it (`Scene::glow_rows`), so a strip left at the
/// previous frame's height is a node writing past the end of the texture and
/// reading zeros back — which looks like a node that has stopped glowing rather
/// than like a bug. The fixture here settles for a row per node, which is what
/// a scene assembled by hand has (`rows_per_node`).
///
/// The same pane through every frame, which is the whole point — a fresh pane
/// allocates a fresh strip and could not be wrong about this. What is under
/// test is the resize, so each frame has to find the last one's.
#[test]
fn the_ink_strip_has_a_row_for_every_node() {
    const SIZE: [u32; 2] = [256, 256];
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let points = egui::vec2(SIZE[0] as f32, SIZE[1] as f32);
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    let mut resources = CallbackResources::default();

    // The fixture's node, copied along a row far enough apart that no node's
    // own layers reach another's.
    let scene_of = |n: usize| -> Scene {
        let mut scene = single_marked_node(0, 0);
        let node = scene.nodes[0];
        scene.nodes = (0..n)
            .map(|i| {
                let mut nd = node;
                nd.world_pos = glam::Vec3::new(i as f32 * 1.8 - 1.8, 0.0, 0.0);
                nd.lattice_pos = harmonigraph_core::LatticePos::new(i as i32, 0, 0);
                nd
            })
            .collect();
        rows_per_node(&mut scene);
        scene.glow_reach = 0.8;
        scene.glow_strength = 1.5;
        scene
    };
    let frame = |resources: &mut CallbackResources, n: usize| -> (u32, u32) {
        let cb = LatticeCallback::from_scene(
            &scene_of(n),
            LatticeLabels::default(),
            points,
            format,
            9,
            None,
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(&device, &queue, &screen, &mut encoder, resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));
        let pane = resources
            .get::<LatticeResources>()
            .expect("prepare built the resources")
            .panes
            .get(&9)
            .expect("...and this pane's buffers");
        let strip = &pane
            .offscreen
            .as_ref()
            .expect("the pane drew something")
            .glow
            .as_ref()
            .expect("the view asks for a glow")
            .strip;
        (pane.instance_count, strip.rows)
    };
    // Up and back down: a strip that only ever grew would pass a rising
    // sequence while leaving rows no node writes into.
    for n in [2usize, 3, 5, 4] {
        let (instances, rows) = frame(&mut resources, n);
        // The instance count rather than `n`: a node that can paint nothing is
        // dropped before it reaches the buffer, and the strip follows what is
        // IN the buffer.
        assert_eq!(instances, n as u32, "the fixture's {n} nodes must all be drawn");
        assert_eq!(
            rows, instances,
            "a frame of {instances} nodes drew into a strip {rows} rows tall",
        );
    }
}

/// The light of a node ADDED to a pane mid-session is that node's own.
///
/// The behavioural half of [`the_ink_strip_has_a_row_for_every_node`], and the
/// half that says the rows are the right way round: a strip that grew but was
/// read at the wrong offset would still be one row per node.
///
/// Two nodes lit in colours no mixture of the other's could be mistaken for —
/// [`two_colour_node`]'s two flat ramps, the band's red and the analyzer's
/// green. The one already on screen wears the band alone and the one arriving
/// wears the ring alone, so the added node's light is green exactly where the
/// other's is red.
#[test]
fn a_node_added_to_a_pane_lights_in_its_own_colour() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let scene_of = |arrived: bool| -> Scene {
        let mut scene = two_colour_node(0.16, 0.16);
        let node = scene.nodes[0];
        let band = {
            let mut node = node;
            node.audio_ring = 0.0;
            node.world_pos = glam::Vec3::new(-1.8, 0.0, 0.0);
            node.lattice_pos = harmonigraph_core::LatticePos::new(-1, 0, 0);
            node
        };
        let ring = {
            let mut node = node;
            node.activation = 0.0;
            node.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
            node.audio_ring = 1.0;
            node.world_pos = glam::Vec3::new(1.8, 0.0, 0.0);
            node.lattice_pos = harmonigraph_core::LatticePos::new(1, 0, 0);
            node
        };
        scene.nodes = if arrived { vec![band, ring] } else { vec![band] };
        rows_per_node(&mut scene);
        scene
    };
    let one = shooter.shot(&scene_of(false));
    let two = shooter.shot(&scene_of(true));

    // What the second frame added, per channel, over the right-hand half of the
    // picture — where the added node is, and where the first frame has nothing.
    let mut added = [0i64; 3];
    let row = SIZE[0] as usize;
    for i in 0..(SIZE[0] * SIZE[1]) as usize {
        if i % row < row / 2 {
            continue;
        }
        for (c, sum) in added.iter_mut().enumerate() {
            *sum += (i64::from(two[i * 4 + c]) - i64::from(one[i * 4 + c])).max(0);
        }
    }
    assert!(added[1] > 0, "the added node lit nothing at all: {added:?}");
    assert!(
        added[1] > added[0] * 4,
        "a node wearing only its audio ring lit {added:?} — that is the BAND's red, which is \
         the other node's ink and so the other node's row of the strip",
    );
}

/// One profile of a shot's brightness round a circle of `radius` about the
/// frame's centre, sampled bilinearly so the reading follows the circle rather
/// than the pixel grid.
///
/// The step between samples is well under a pixel, which is what the claims
/// above need: a ripple is measured against the light beside it, and a profile
/// that skipped pixels would read the grid's own steps as one.
fn ring_profile(shot: &[u8], size: [u32; 2], radius: f32) -> Vec<f64> {
    let at = |x: f32, y: f32| -> f64 {
        let (x0, y0) = (x.floor(), y.floor());
        let px = |ix: f32, iy: f32| -> f64 {
            let ix = (ix as i32).clamp(0, size[0] as i32 - 1) as usize;
            let iy = (iy as i32).clamp(0, size[1] as i32 - 1) as usize;
            let i = (iy * size[0] as usize + ix) * 4;
            brightness(&shot[i..i + 3]) as f64 / 3.0
        };
        let (fx, fy) = (f64::from(x - x0), f64::from(y - y0));
        let top = px(x0, y0) + (px(x0 + 1.0, y0) - px(x0, y0)) * fx;
        let bot = px(x0, y0 + 1.0) + (px(x0 + 1.0, y0 + 1.0) - px(x0, y0 + 1.0)) * fx;
        top + (bot - top) * fy
    };
    let (cx, cy) = (size[0] as f32 / 2.0, size[1] as f32 / 2.0);
    // Four samples per pixel of circumference, and never so few that a whole
    // turn is under a reading.
    let n = ((radius * std::f32::consts::TAU * 4.0) as usize).max(64);
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            at(cx + radius * t.cos(), cy + radius * t.sin())
        })
        .collect()
}

/// The amplitude of the `k`th angular harmonic of a profile — how much of it is
/// a ripple that goes round the turn exactly `k` times.
fn harmonic(profile: &[f64], k: usize) -> f64 {
    let n = profile.len() as f64;
    let (mut re, mut im) = (0.0, 0.0);
    for (i, v) in profile.iter().enumerate() {
        let a = std::f64::consts::TAU * k as f64 * i as f64 / n;
        re += v * a.cos();
        im += v * a.sin();
    }
    2.0 * (re * re + im * im).sqrt() / n
}

/// A node with no light of its own writes into no other node's colour.
///
/// `GlowFade` hands a strip row only to a node that has a light. Everything
/// else is given `GlowStep::default()` — and that default's row is 0, its mix
/// 1.0. A node with no light is still SHIPPED whenever it draws anything at
/// all (`paints`: an audio ring is enough), and `fs_ink_strip` draws for every
/// instance without looking at the level, so such a node settles its own ink
/// into row 0 at full weight and takes the colour of whichever node actually
/// owns that row.
///
/// Ordinary material reaches this, not a stress test: turn the audio ring on
/// and every ringing node that is not itself lit — most of them, with the Gate
/// low — writes over row 0. The node holding row 0 is the first node to have
/// lit in the session, so what a listener sees is one node's halo wearing the
/// wrong hue and flickering between wrong hues, since which of the several
/// writers lands last is the rasteriser's business and not stable frame to
/// frame.
///
/// Two nodes, two layers, one colour each: the lit node draws the RED octave
/// band and no ring, the unlit one draws the GREEN audio ring and nothing
/// else. The lit node's halo has to stay red.
#[test]
fn a_node_with_no_light_writes_into_no_other_nodes_colour() {
    const SIZE: [u32; 2] = [256, 256];
    const WIDTH: f32 = 0.18;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The band red and the ring green, with the LIT node wearing only the
    // band — so every green pixel of light in the frame came off the other
    // node's ink rather than out of this one's own ring.
    let scene = || -> Scene {
        let mut scene = two_colour_node(WIDTH, WIDTH);
        scene.nodes[0].audio_ring = 0.0;
        scene.nodes[0].glow.mix = 1.0;
        let mut idle = scene.nodes[0];
        idle.world_pos.x += 1.2;
        idle.octaves = [0.0; harmonigraph_scene::OCTAVE_SLOTS];
        idle.activation = 0.0;
        idle.audio_ring = 1.0;
        // What `GlowFade` hands a node it gave no row to.
        idle.glow = harmonigraph_scene::GlowStep::default();
        scene.nodes.push(idle);
        scene
    };
    let unlit = |mut scene: Scene| -> Scene {
        scene.glow_reach = 0.0;
        scene
    };

    let ground = shooter.shot(&unlit(scene()));
    let light = added_light(&shooter.shot(&scene()), &ground);

    // Non-vacuous first, on the WHOLE halo rather than on its red: the defect
    // takes the red to nothing, so a red-only check here reports "nothing is
    // lit" for a frame that is brightly lit in the wrong colour.
    assert!(
        light.iter().sum::<i64>() > 64,
        "the lit node lit nothing at all: {light:?}",
    );
    assert!(
        light[0] > light[1] * 4,
        "the lit node's halo came out {light:?}: it is drawing the RED band and no ring, so \
         the green is the idle node's ink settling into the row it was never given",
    );
}

/// A light already in its RELEASE survives the pane changing size.
///
/// The colour half of a node's light lives only in the ink strip, and a node
/// whose note fade has run out draws no layer at all — `ink_at` gates the band
/// on `params.x`, the ring on `in.ring` and the marks on `params.y`/`z`, and
/// every one of them is 0. That is the designed state, not an edge: a level can
/// stand above zero on a node whose every layer has gone silent, and such a
/// node is shipped for exactly that reason. Its halo's colour is therefore
/// entirely what the strip already HELD.
///
/// So the strip is the one thing that must not be dropped underneath it. Any
/// change to the pane's pixel size rebuilds the offscreen targets, and a strip
/// rebuilt from nothing hands a releasing node `held = 0` with no ink to seed
/// from — `glow_layer` reads `ink.w <= 0` and returns nothing, on that frame
/// and every frame after. The halo does not fade, it disappears.
///
/// What that looks like: hold a chord, let go, and while the light is still
/// running out drag the window's edge, drag the dock separator over the
/// lattice, or drag the window between a Retina display and an external
/// monitor — that last one moves `pixels_per_point`, so the pixel size changes
/// at an unchanged point size. Every lingering halo snaps off in one frame,
/// while halos on nodes still holding keys are untouched (they have ink of
/// their own). It reads as a bug in the release rather than in the resize.
///
/// Measured against the SAME node one ordinary frame on, so the claim is
/// "a resize is not different from a frame" rather than a number.
#[test]
fn a_light_in_its_release_survives_the_pane_changing_size() {
    const SIZE: [u32; 2] = [256, 256];
    const GROWN: [u32; 2] = [256, 260];
    const WIDTH: f32 = 0.18;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // Sounding: the octave band alone, which is what puts a colour in the row.
    let sounding = || -> Scene {
        let mut scene = two_colour_node(WIDTH, 0.0);
        scene.nodes[0].glow.mix = 1.0;
        scene
    };
    // Releasing: the note fade has run out, so the node draws no layer at all
    // and takes none of this frame's ink — only its light is left, and only
    // the strip knows what colour it is.
    let releasing = || -> Scene {
        let mut scene = two_colour_node(0.0, 0.0);
        scene.nodes[0].glow.mix = 0.0;
        scene
    };
    let unlit = |mut scene: Scene| -> Scene {
        scene.glow_reach = 0.0;
        scene
    };

    // The two grounds first, each on a pane of its own, so that the carrying
    // sequence below runs without a shot in the middle of it taking the pane.
    let ground = shooter.shot(&unlit(releasing()));
    shooter.size = GROWN;
    let ground_grown = shooter.shot(&unlit(releasing()));
    shooter.size = SIZE;

    // A sounding frame to put a colour in the row, then the release.
    let _ = shooter.shot(&sounding());
    let kept = added_light(&shooter.shot_again(&releasing()), &ground);
    // And the same release again, with the pane one pixel wider.
    shooter.size = GROWN;
    let resized = added_light(&shooter.shot_again(&releasing()), &ground_grown);

    // Non-vacuous first: the release has to light the halo at all, or the
    // comparison below is between two nothings.
    assert!(
        kept[0] > 64,
        "a releasing node lit nothing to begin with: {kept:?}",
    );
    // The claim. Half is generous — the frame is a little larger, and the
    // light is stepped once more — where the failure takes it to zero.
    assert!(
        resized[0] > kept[0] / 2,
        "the light went from {kept:?} to {resized:?} when the pane changed size: \
         a node drawing no ink of its own has only the strip's held colour, and \
         a strip rebuilt with the offscreen targets hands it none",
    );
}

/// A node's light takes its colour from the frame before, not from this frame's
/// ink alone.
///
/// The COLOUR half of the glow's own clock. A node's ink is read in WGSL and
/// kept in a strip on the GPU, so this is where it is carried: the reading is
/// mixed into the row that node already had
/// (`harmonigraph_scene::GlowStep::mix`), on the same coefficient the level
/// took on the CPU. What that buys is a hue that MORPHS when the chord under it
/// changes, rather than one that cuts.
///
/// [`two_colour_node`]'s two layers, which is the fixture built for exactly
/// this reading: the octave band flat RED and the audio ring flat GREEN, so a
/// halo's red against its green is which layer coloured it, with no mixture of
/// one able to be mistaken for the other. The node keeps its identity across
/// the two frames — same position, same row — and swaps which of the two layers
/// it is drawing, which is as sharp a change of hue as a node can make.
#[test]
fn a_nodes_light_takes_its_colour_from_the_frame_before() {
    const SIZE: [u32; 2] = [256, 256];
    const WIDTH: f32 = 0.18;
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The band alone, and the ring alone, each at the same width so neither
    // layer carries more of the node than the other.
    let red = |mix: f32| -> Scene {
        let mut scene = two_colour_node(WIDTH, 0.0);
        scene.nodes[0].glow.mix = mix;
        scene
    };
    let green = |mix: f32| -> Scene {
        let mut scene = two_colour_node(0.0, WIDTH);
        scene.nodes[0].glow.mix = mix;
        scene
    };
    let unlit = |mut scene: Scene| -> Scene {
        scene.glow_reach = 0.0;
        scene
    };
    // The two ends, each settled on a pane of its own.
    let red_off = shooter.shot(&unlit(red(1.0)));
    let green_off = shooter.shot(&unlit(green(1.0)));
    let all_red = added_light(&shooter.shot(&red(1.0)), &red_off);
    let all_green = added_light(&shooter.shot(&green(1.0)), &green_off);
    // And the frame after a red one, on the same pane, taking a tenth of the
    // new reading — a Glow attack long against the frame it is stepped over.
    let _ = shooter.shot(&red(1.0));
    let carried = added_light(&shooter.shot_again(&green(0.1)), &green_off);

    // Non-vacuous first: each layer alone has to light the halo in its own
    // colour, or the reading below is measuring nothing.
    assert!(
        all_red[0] > all_red[1] * 4,
        "the band alone must light the halo red: {all_red:?}",
    );
    assert!(
        all_green[1] > all_green[0] * 4,
        "the ring alone must light the halo green: {all_green:?}",
    );
    // The claim: one frame in, the light is still mostly the colour it was,
    // though the node is drawing nothing but the ring.
    assert!(
        carried[0] > carried[1],
        "a light that took a tenth of the new reading came out {carried:?} — that is the \
         ring's green, so the row was written rather than mixed into",
    );
}

/// The Meld bar's claim: it says how much two nodes' overlapping light adds
/// up, and nothing else. At its top an overlap is the screen the light has
/// always been (`two_nodes_light_melds_rather_than_summing`); at its bottom it
/// is exactly as bright as the brighter of the nodes making it.
///
/// Measured at a FLAT feather, which is the setting the bar exists for. A
/// falloff spread across its whole reach is still near its peak halfway to a
/// neighbour, so screening two of them puts more light in the GAP between two
/// nodes than either node has of its own — the count of overlapping nodes
/// becomes the brightest thing on screen, which is the failure the screen was
/// picked over a sum to avoid. Under a max there is no count to read.
///
/// Against the two nodes' own single shots at the SAME pixel rather than
/// against a chosen brightness: what the bar changes is how two lots of light
/// combine, so the reading that says which operator is in the pipeline is the
/// overlap against its own two halves.
#[test]
fn the_meld_says_how_much_two_nodes_overlapping_light_adds_up() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // The mirrored pair `two_nodes_light_melds_rather_than_summing` measures,
    // at a flat feather: far enough apart that neither node's own layers reach
    // the other and only their light crosses.
    let scene_of = |xs: &[f32], meld: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        let node = scene.nodes[0];
        scene.nodes = xs
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let mut n = node;
                n.world_pos = glam::Vec3::new(*x, 0.0, 0.0);
                n.lattice_pos = harmonigraph_core::LatticePos::new(i as i32, 0, 0);
                n
            })
            .collect();
        rows_per_node(&mut scene);
        scene.glow_reach = 0.8;
        scene.glow_strength = 2.0;
        scene.glow_feather = 1.0;
        scene.glow_meld = meld;
        scene
    };
    const APART: f32 = 1.8;
    let left = shooter.shot(&scene_of(&[-APART], 1.0));
    let right = shooter.shot(&scene_of(&[APART], 1.0));
    let melded = shooter.shot(&scene_of(&[-APART, APART], 1.0));
    let brightest = shooter.shot(&scene_of(&[-APART, APART], 0.0));

    let at = |shot: &[u8], i: usize| -> [u8; 3] { std::array::from_fn(|c| shot[i * 4 + c]) };
    // Where the two overlap most: the pixel whose dimmer half is brightest,
    // found rather than named so the probe follows the camera.
    let probe = (0..(SIZE[0] * SIZE[1]) as usize)
        .max_by_key(|&i| {
            let (l, r) = (at(&left, i), at(&right, i));
            brightness(&l).min(brightness(&r))
        })
        .expect("a non-empty frame");
    let (l, r) = (at(&left, probe), at(&right, probe));
    let (m, b) = (at(&melded, probe), at(&brightest, probe));
    assert!(
        brightness(&l) > 24 && brightness(&r) > 24,
        "the two halos never overlapped: {l:?} and {r:?} at pixel {probe}",
    );

    let half = brightness(&l).max(brightness(&r));
    assert!(
        brightness(&m) > half,
        "at a full Meld the overlap {m:?} must be brighter than the brighter half \
         ({l:?}, {r:?})",
    );
    // The bar's bottom: no light added at all. A per-channel max of two
    // premultiplied colours, so a pixel whose two halves differ in hue can come
    // out a shade over either — the slack is that, plus a composite's rounding.
    assert!(
        brightness(&b) <= half + 2,
        "at a Meld of 0 the overlap {b:?} must be no brighter than the brighter half \
         ({l:?}, {r:?})",
    );
    assert!(
        brightness(&b) + 8 < brightness(&m),
        "the Meld moved the overlap by {} of 255, which is no picture to dial",
        brightness(&m) - brightness(&b),
    );
}

/// The Meld's other half, and what makes it a bar about OVERLAP rather than a
/// second Strength: a pixel one node lights on its own is the same at every
/// setting of it.
///
/// Exact equality, not a tolerance. The two blends the bar mixes between agree
/// wherever only one node writes — a screen over nothing and a max against
/// nothing are both the source — so mixing them there returns that source
/// whatever the weight, and any drift at all would mean the pair had stopped
/// agreeing.
#[test]
fn the_meld_leaves_a_node_lighting_a_pixel_alone_untouched() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    let at = |meld: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        scene.glow_reach = 0.8;
        scene.glow_strength = 2.0;
        // Flat, so the light fills its whole reach: the widest picture one
        // node's halo has, and the most of a frame this can hold still.
        scene.glow_feather = 1.0;
        scene.glow_meld = meld;
        scene
    };
    let melded = shooter.shot(&at(1.0));
    let brightest = shooter.shot(&at(0.0));
    let differ = melded
        .chunks(4)
        .zip(brightest.chunks(4))
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(differ, 0, "the Meld moved {differ} pixels of a lone node's own light");
    // Non-vacuous: there has to BE light for the equality to be about anything,
    // the fixture's node being the only thing lighting the frame.
    let mut dark = at(1.0);
    dark.glow_reach = 0.0;
    let unlit = shooter.shot(&dark);
    assert!(
        total_light(&melded) > total_light(&unlit),
        "the fixture's node lit nothing, so holding its light still says nothing",
    );
}

/// The Meld reaches what a NODE paints, not just the ground between nodes.
///
/// A node's clearing paints the light standing at its own pixel rather than
/// bare ground, reading the glow target back through `node_paint` — so it mixes
/// the same pair the composite does, and has to mix it the same way. A clearing
/// left on the screen while the ground around it took the max is a node sitting
/// on a plateau with a step at its Clearance, which is a halo drawn round every
/// node: the one failure the light being ONE field under the whole lattice
/// exists to prevent.
///
/// The probe is the brightest pixel of a ONE-node frame — the middle of that
/// node, where its own light is fullest and its ink is what the pass wrote
/// (`the_middle_of_a_node_is_where_its_light_is_fullest`). Bare ground is what
/// `the_meld_says_how_much_two_nodes_overlapping_light_adds_up` measures, and
/// bare ground is written by the composite alone: probing it says nothing about
/// this path. A second node then lights that same pixel from outside, which is
/// what gives the mix two lots of light to combine there.
#[test]
fn the_meld_reaches_the_light_a_node_paints_over_its_own_body() {
    const SIZE: [u32; 2] = [256, 256];
    let Some(mut shooter) = Shooter::new(SIZE) else {
        return;
    };
    // A reach wide enough that each node stands INSIDE its neighbour's light
    // rather than merely touching it at the midpoint: the pixel under test is
    // on a node's own body, so the other node's halo has to reach that far in
    // for there to be two lots of light to mix there at all.
    let scene_of = |xs: &[f32], meld: f32| -> Scene {
        let mut scene = single_marked_node(0, 0);
        let node = scene.nodes[0];
        scene.nodes = xs
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let mut n = node;
                n.world_pos = glam::Vec3::new(*x, 0.0, 0.0);
                n.lattice_pos = harmonigraph_core::LatticePos::new(i as i32, 0, 0);
                n
            })
            .collect();
        rows_per_node(&mut scene);
        scene.glow_reach = 3.0;
        scene.glow_strength = 1.0;
        scene.glow_feather = 1.0;
        scene.glow_meld = meld;
        scene
    };
    const APART: f32 = 1.8;
    let lone = shooter.shot(&scene_of(&[-APART], 1.0));
    let melded = shooter.shot(&scene_of(&[-APART, APART], 1.0));
    let brightest = shooter.shot(&scene_of(&[-APART, APART], 0.0));

    let at = |shot: &[u8], i: usize| -> [u8; 3] { std::array::from_fn(|c| shot[i * 4 + c]) };
    // The one node's own middle: the brightest pixel of the frame it is alone
    // in. Found rather than named, so the probe follows the camera.
    let probe = (0..(SIZE[0] * SIZE[1]) as usize)
        .max_by_key(|&i| brightness(&at(&lone, i)))
        .expect("a non-empty frame");
    let (l, m, b) = (at(&lone, probe), at(&melded, probe), at(&brightest, probe));
    assert!(
        brightness(&l) > 24,
        "the probe {l:?} is not on the node the frame was searched for",
    );
    // The neighbour has to be lighting this pixel for the mix to have anything
    // to do here: without that, both blends see one contribution and agree.
    assert!(
        brightness(&m) > brightness(&l),
        "the second node did not reach the first node's own middle: {m:?} against {l:?}",
    );
    assert!(
        brightness(&b) < brightness(&m),
        "the Meld did not reach what the node paints: its middle is {b:?} at 0 and {m:?} at \
         1, so this pixel took the screen either way while the ground around it did not",
    );
}
