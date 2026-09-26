//! One claim over the whole picture: whatever a [`ViewConfig`] holds, the
//! [`Scene`] derived from it is made of numbers a shader can draw.
//!
//! Held here as ONE assertion rather than one per clamp, because that is the
//! shape the defect has. A bare `clamp` is no guard against a NaN — NaN
//! answers no to every comparison a clamp makes, so it passes straight
//! through — and the site that acquires one next is covered by an assertion
//! over the scene rather than by a test somebody remembers to add beside it.
//!
//! The door ([`ViewConfig::sanitize`]) repairs a loaded blob, so nothing
//! arriving through the DAW reaches these sites broken. What this is for is
//! the shells that never cross it: the offline layout, take replay and the
//! harness each build a view in code, and there the failure is a blank or
//! garbled frame with nothing on screen saying why.

use super::harness::*;
use crate::*;
use glam::{Vec3, Vec4};
use harmonigraph_core::Tuning;

/// A [`ViewConfig`] with every float poisoned, and every other field set to
/// whatever it takes to reach the picture: a window with nodes in it, a
/// sounding note's marks, and the resting marker field switched on.
///
/// Written WHOLE rather than over `..ViewConfig::default()` on purpose. A new
/// dial then breaks this fixture instead of slipping past it, which is the one
/// mechanism that keeps the assertion below covering the next float as well as
/// today's.
fn poisoned_view() -> ViewConfig {
    let nan = f32::NAN;
    let base = ViewConfig::default();
    // Written out for the same reason the view is, and it has to be its own
    // literal: taken from `base` the six knobs arrive CLEAN, and the 256-entry
    // `pitch_lut` the sweep walks would be asserted over a gradient nothing
    // ever poisoned — the one float the "written WHOLE" mechanism above cannot
    // catch by itself, since the field is one name here and seven there.
    //
    // Green today because every consumer funnels through `color::with_lut`,
    // which calls `Gradient::sanitized` before keying its memo. That is the
    // claim this pins; a knob added to `Gradient` and left out of that repair
    // is what it goes red for.
    let pitch_gradient = Gradient {
        hue_start: nan,
        hue_span: nan,
        lightness: nan,
        lightness_ramp: nan,
        chroma: nan,
        chroma_ramp: nan,
        bend: Bend { at: nan, share: nan, ..Bend::default() },
    };
    let shadow = ShadowStyle {
        kernel: base.shadow.lattice_geometry.kernel,
        width: nan,
        depth: nan,
        falloff: nan,
    };
    ViewConfig {
        extent_threes: 3,
        extent_fives: 3,
        min_sevens: -1,
        max_sevens: 1,
        center_threes: 0,
        center_fives: 0,
        center_sevens: 0,
        sevens_size: nan,
        sevens_label: base.sevens_label,
        label_scale: nan,
        show_cents: true,
        note_names: NoteNames::Played,
        show_map_indicators: base.show_map_indicators,
        pitch_gradient,
        band_width: nan,
        ring_inner: nan,
        ring_gap: nan,
        lattice_ground: nan,
        marker_ink: nan,
        octave_count: base.octave_count,
        octave_center: nan,
        octave_extras: base.octave_extras,
        octave_extra_size: nan,
        octave_extra_blend: nan,
        spectral_reading: base.spectral_reading,
        spectral_width: nan,
        spectral_ring_width: nan,
        spectral_ring_range: nan,
        spectral_ring_gate: nan,
        spectral_ring_hysteresis: nan,
        spectral_ring_attack: nan,
        spectral_ring_release: nan,
        fade_shape: nan,
        note_animation: NoteAnimationConfig {
            order: base.note_animation.order,
            stagger_spread: nan,
            radial_start: nan,
        },
        intensity: {
            let source = |target| crate::IntensitySource { target, weight: nan };
            crate::IntensitySettings {
                velocity: source(crate::IntensityTarget::Opacity),
                gain: source(crate::IntensityTarget::Glow),
                pressure: source(crate::IntensityTarget::Thickness),
                timbre: source(crate::IntensityTarget::Opacity),
                gain_range: nan,
                glow_base: nan,
                opacity_rest: nan,
                thickness_max: nan,
            }
        },
        mark_thickness: nan,
        mark_delay: nan,
        plus_arm: nan,
        plus_width: nan,
        plus_taper: nan,
        meantone: base.meantone,
        meantone_auto: base.meantone_auto,
        marvel: base.marvel,
        marvel_auto: base.marvel_auto,
        frameless: base.frameless,
        show_perf: base.show_perf,
        show_perf_detail: base.show_perf_detail,
        render_scale: nan,
        spiral_bloom: nan,
        glow_reach: nan,
        atmosphere: AtmosphereSettings {
            enabled: true,
            nebula_depth: nan,
            nebula_scale: nan,
            nebula_speed: nan,
            breath_amount: nan,
            breath_speed: nan,
        },
        glow_strength: nan,
        glow_curve: GlowCurve { shape: nan },
        shadow: ShadowSettings {
            lattice_geometry: shadow,
            lattice_text: shadow,
            spectral_geometry: shadow,
            spectral_text: shadow,
        },
        glow_wash: nan,
        glow_blend: nan,
        glow_accumulation: nan,
        glow_attack: nan,
        glow_release: nan,
    }
}

/// Every float the scene ships, under the name a failure has to print to be
/// worth anything: "something is NaN" costs a bisect, "`marker_unit` is NaN"
/// costs a read.
#[derive(Default)]
struct Floats(Vec<(String, f32)>);

impl Floats {
    fn one(&mut self, name: impl Into<String>, value: f32) {
        self.0.push((name.into(), value));
    }

    fn many(&mut self, name: &str, values: impl IntoIterator<Item = f32>) {
        for (i, value) in values.into_iter().enumerate() {
            self.one(format!("{name}[{i}]"), value);
        }
    }

    fn vec3(&mut self, name: &str, value: Vec3) {
        self.many(name, value.to_array());
    }

    fn vec4(&mut self, name: &str, value: Vec4) {
        self.many(name, value.to_array());
    }

    fn luts(&mut self, name: &str, lut: &[Vec4]) {
        for (i, color) in lut.iter().enumerate() {
            self.vec4(&format!("{name}[{i}]"), *color);
        }
    }

    fn shadow(&mut self, name: &str, style: &ShadowStyle) {
        let ShadowStyle { kernel: _, width, depth, falloff } = style;
        self.one(format!("{name}.width"), *width);
        self.one(format!("{name}.depth"), *depth);
        self.one(format!("{name}.falloff"), *falloff);
    }
}

/// The whole scene, flattened.
///
/// Every struct is destructured EXHAUSTIVELY — no `..` anywhere below — so a
/// field added to the scene, to a node or to a marker breaks this walk rather
/// than going unwatched. That is the half of the claim a test cannot assert:
/// the assertion says the floats are real numbers, and this says which floats
/// there are.
fn scene_floats(scene: &Scene) -> Floats {
    let mut f = Floats::default();
    let Scene {
        nodes,
        camera,
        node_radius,
        note_animation,
        outer_inner,
        outer_outer,
        rings_outer,
        mark_inner,
        octave_gap,
        lattice_ground,
        spectral,
        octave_layout,
        pluses,
        plus_half_width,
        plus_taper_start,
        background,
        mark_thickness,
        pitch_lut,
        pitch_lut_spacing,
        darkest_pitch,
        brightest_pitch,
        render_scale,
        bloom_strength,
        glow_reach,
        glow_strength,
        glow_curve,
        shadow,
        glow_wash,
        marker_unit,
        glow_blend,
        glow_accumulation,
        glow_rows: _,
        glow_timing,
        atmosphere,
    } = scene;

    for (i, node) in nodes.iter().enumerate() {
        let NodeInstance {
            lattice_pos: _,
            world_pos,
            activation,
            departing: _,
            slice_progress,
            octaves,
            thickness,
            hovered: _,
            on_home: _,
            scale,
            comma,
            cents,
            melody_slots: _,
            bass_slots: _,
            melody_level,
            bass_level,
            melody_color,
            bass_color,
            audio_ring,
            glow,
            bloom,
            trail,
        } = node;
        f.vec3(&format!("nodes[{i}].world_pos"), *world_pos);
        f.one(format!("nodes[{i}].activation"), *activation);
        f.many(&format!("nodes[{i}].slice_progress"), *slice_progress);
        f.many(&format!("nodes[{i}].octaves"), *octaves);
        f.one(format!("nodes[{i}].bloom"), *bloom);
        f.many(&format!("nodes[{i}].thickness"), *thickness);
        f.one(format!("nodes[{i}].scale"), *scale);
        f.one(format!("nodes[{i}].comma"), *comma);
        f.one(format!("nodes[{i}].cents"), *cents);
        f.one(format!("nodes[{i}].melody_level"), *melody_level);
        f.one(format!("nodes[{i}].bass_level"), *bass_level);
        f.vec4(&format!("nodes[{i}].melody_color"), *melody_color);
        f.vec4(&format!("nodes[{i}].bass_color"), *bass_color);
        f.one(format!("nodes[{i}].audio_ring"), *audio_ring);
        let GlowStep { incarnation: _, level, row: _ } = glow;
        f.one(format!("nodes[{i}].glow.level"), *level);
        f.one(format!("nodes[{i}].trail"), *trail);
    }

    let Camera { target, yaw, pitch, distance, projection: _, cabinet_angle, cabinet_scale } =
        camera;
    f.vec3("camera.target", *target);
    f.one("camera.yaw", *yaw);
    f.one("camera.pitch", *pitch);
    f.one("camera.distance", *distance);
    f.one("camera.cabinet_angle", *cabinet_angle);
    f.one("camera.cabinet_scale", *cabinet_scale);

    f.one("node_radius", *node_radius);

    let NoteAnimationConfig { order: _, stagger_spread, radial_start } = note_animation;
    f.one("note_animation.stagger_spread", *stagger_spread);
    f.one("note_animation.radial_start", *radial_start);

    f.one("outer_inner", *outer_inner);
    f.one("outer_outer", *outer_outer);
    f.one("rings_outer", *rings_outer);
    f.one("mark_inner", *mark_inner);
    f.one("octave_gap", *octave_gap);
    f.vec4("lattice_ground", *lattice_ground);

    let SpectralPaint {
        lut,
        lut_spacing,
        folded: _,
        inner,
        outer,
        range,
        gate,
        hysteresis,
        levels: _,
        color_levels: _,
    } = spectral;
    f.luts("spectral.lut", lut);
    f.many("spectral.lut_spacing", [lut_spacing.at, lut_spacing.share]);
    f.one("spectral.inner", *inner);
    f.one("spectral.outer", *outer);
    f.one("spectral.range", *range);
    f.one("spectral.gate", *gate);
    f.one("spectral.hysteresis", *hysteresis);

    let OctaveLayout { center, span: _, count: _, extras: _, bounds } = octave_layout;
    f.one("octave_layout.center", *center);
    f.many("octave_layout.bounds", *bounds);

    for (i, plus) in pluses.iter().enumerate() {
        let PlusInstance { node: _, pos, radius, color, strength } = plus;
        f.vec3(&format!("pluses[{i}].pos"), *pos);
        f.one(format!("pluses[{i}].radius"), *radius);
        f.vec4(&format!("pluses[{i}].color"), *color);
        f.one(format!("pluses[{i}].strength"), *strength);
    }

    f.one("plus_half_width", *plus_half_width);
    f.one("plus_taper_start", *plus_taper_start);
    f.vec4("background", *background);
    f.one("mark_thickness", *mark_thickness);
    f.luts("pitch_lut", pitch_lut);
    f.many("pitch_lut_spacing", [pitch_lut_spacing.at, pitch_lut_spacing.share]);
    f.one("darkest_pitch", *darkest_pitch);
    f.one("brightest_pitch", *brightest_pitch);
    f.one("render_scale", *render_scale);
    f.one("bloom_strength", *bloom_strength);
    f.one("glow_reach", *glow_reach);
    f.one("glow_strength", *glow_strength);

    let GlowCurve { shape } = glow_curve;
    f.one("glow_curve.shape", *shape);

    let ShadowSettings { lattice_geometry, lattice_text, spectral_geometry, spectral_text } =
        shadow;
    f.shadow("shadow.lattice_geometry", lattice_geometry);
    f.shadow("shadow.lattice_text", lattice_text);
    f.shadow("shadow.spectral_geometry", spectral_geometry);
    f.shadow("shadow.spectral_text", spectral_text);

    f.one("glow_wash", *glow_wash);
    f.one("marker_unit", *marker_unit);
    f.one("glow_blend", *glow_blend);
    f.one("glow_accumulation", *glow_accumulation);

    // Nothing to walk here, and that is worth saying rather than leaving as a
    // silent pass: `derive_scene` ships `glow_timing: None` unconditionally,
    // the ballistics being the SHELL's glow pass to fill, so the view's
    // `glow_attack` and `glow_release` reach the picture there and not through
    // this function. Read as coverage of a `GlowTiming` the arm below would be
    // a green light with nothing behind it. It stays for the one thing it does
    // buy — a field added to `GlowTiming` breaks this build — and the test
    // asserts the emptiness, so the day `derive_scene` starts filling it this
    // comment goes red instead of going stale.
    if let Some(GlowTiming { now, attack, release }) = glow_timing {
        f.one("glow_timing.now", *now as f32);
        f.one("glow_timing.attack", *attack);
        f.one("glow_timing.release", *release);
    }

    let AtmosphereSettings {
        enabled: _,
        nebula_depth,
        nebula_scale,
        nebula_speed,
        breath_amount,
        breath_speed,
    } = atmosphere;
    f.one("atmosphere.nebula_depth", *nebula_depth);
    f.one("atmosphere.nebula_scale", *nebula_scale);
    f.one("atmosphere.nebula_speed", *nebula_speed);
    f.one("atmosphere.breath_amount", *breath_amount);
    f.one("atmosphere.breath_speed", *breath_speed);

    f
}

/// Every float the scene ships, by name, that is not a real number.
fn broken(scene: &Scene) -> Vec<String> {
    scene_floats(scene)
        .0
        .into_iter()
        .filter(|(_, value)| !value.is_finite())
        .map(|(name, value)| format!("{name} = {value}"))
        .collect()
}

#[test]
fn a_view_of_nothing_but_nan_still_derives_a_scene_of_real_numbers() {
    let scene = scene_of(&sounding(), &Tuning::default(), &poisoned_view(), &plain_frame(), 0.0);

    // The fixture has to ARRIVE, or the sweep below passes over an empty
    // picture and reads as coverage while measuring nothing.
    assert!(!scene.nodes.is_empty(), "a poisoned view left no nodes to measure");
    assert!(
        scene.nodes.iter().any(|n| n.on_home),
        "a poisoned view left no home position, where the marker field and the ring stack stand",
    );
    let off_sheet = scene
        .nodes
        .iter()
        .find(|n| !n.on_home)
        .expect("the fixture has sevens layers, so the sevens scale below has something to size")
        .scale;
    // The emptiness the walk's `glow_timing` arm rests on, stated where it can
    // go red: the ballistics are the shell's to fill, and nothing here covers
    // a `GlowTiming`.
    assert!(
        scene.glow_timing.is_none(),
        "`derive_scene` filled the glow timing, so the walk's empty arm is no longer honest",
    );

    // Each site's chosen fallback, pinned — the VALUE, and no more than that.
    // Several of these fallbacks are the fresh value as well (all three poses,
    // and `render_scale`), so the line cannot tell a repair from a field the
    // poison never reached, and none of them is offered as evidence that it
    // did. What says the poison arrives is that this test was written and run
    // against the unrepaired tree first, where it failed naming every site
    // below and 147 nodes besides.
    let step = crate::NODE_RADIUS_FACTOR;
    let shadow = scene.shadow;
    for (site, got, want) in [
        ("node_radius", scene.node_radius, step),
        ("marker_unit", scene.marker_unit, step * 1.8),
        ("nodes[off the home sheet].scale", off_sheet, 0.15),
        ("glow_reach", scene.glow_reach, 0.0),
        ("glow_strength", scene.glow_strength, 0.0),
        ("glow_wash", scene.glow_wash, 0.0),
        ("glow_blend", scene.glow_blend, 0.0),
        ("glow_accumulation", scene.glow_accumulation, 0.0),
        ("render_scale", scene.render_scale, 1.0),
        // Gain is routed to Glow above, so the pass runs at its floor.
        ("bloom_strength", scene.bloom_strength, crate::BLOOM_REFERENCE_FLOOR),
        ("shadow.lattice_geometry.width", shadow.lattice_geometry.width, 0.0),
        ("shadow.lattice_geometry.depth", shadow.lattice_geometry.depth, 0.0),
        ("shadow.lattice_geometry.falloff", shadow.lattice_geometry.falloff, SHADOW_FALLOFF_MIN),
        ("shadow.spectral_text.falloff", shadow.spectral_text.falloff, SHADOW_FALLOFF_MIN),
        ("note_animation.starting_scale", scene.note_animation.starting_scale(), 1.0),
        ("note_animation.radial_start", scene.note_animation.radial_start, 0.0),
    ] {
        assert_eq!(got, want, "{site} came out {got}, not the fallback this pass chose");
    }

    let names = broken(&scene);
    assert!(names.is_empty(), "a NaN view reached the scene at: {}", names.join(", "));

    // ...and the same sweep over a picture that still has MARKERS in it, which
    // the pass above cannot have. A NaN arm reads as 0 through `size`, so
    // `derive_pluses` ships an empty field and the walk's `pluses` loop runs zero times —
    // leaving a marker's position, colour and strength unmeasured by the one
    // test that claims the whole scene. One real arm is what it takes to
    // get a marker drawn at all; everything the marker's own geometry and ink
    // are derived from stays poisoned around it.
    let drawable = ViewConfig { plus_arm: 0.5, ..poisoned_view() };
    let scene = scene_of(&sounding(), &Tuning::default(), &drawable, &plain_frame(), 0.0);
    assert!(!scene.pluses.is_empty(), "a real arm still shipped no marker field");
    let names = broken(&scene);
    assert!(names.is_empty(), "a NaN view reached the marked scene at: {}", names.join(", "));
}
