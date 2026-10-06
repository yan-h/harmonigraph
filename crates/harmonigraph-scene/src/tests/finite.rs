//! One claim over the whole picture: whatever a [`ViewConfig`] holds, the
//! [`Scene`] derived from it is made of numbers a shader can draw.
//!
//! Held here as ONE assertion rather than one per clamp, because that is the
//! shape the defect has. A bare `clamp` is no guard against a NaN — NaN
//! answers no to every comparison a clamp makes, so it passes straight
//! through — and the site that acquires one next is covered by an assertion
//! over the scene rather than by a test somebody remembers to add beside it.
//!
//! Production loads normalize their view. This intentionally bypasses loading
//! to hold scene entry to the same repair when handed damaged values.

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
        spread: nan,
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
        spectral_reading: base.spectral_reading,
        spectral_width: nan,
        spectral_ring_width: nan,
        spectral_ring_range: nan,
        spectral_ring_gate: nan,
        spectral_ring_hysteresis: nan,
        spectral_ring_attack: nan,
        spectral_ring_release: nan,
        fade_shape: nan,
        note_bloom: nan,
        note_animation: NoteAnimationConfig {
            order: base.note_animation.order,
            lit_first: base.note_animation.lit_first,
            stagger_spread: nan,
            radial_start: nan,
        },
        intensity: {
            let source = || crate::IntensitySource { opacity: Some(nan), thickness: Some(nan) };
            crate::IntensitySettings {
                velocity: source(),
                gain: source(),
                pressure: source(),
                timbre: source(),
                opacity_rest: nan,
                thickness_base: nan,
                thickness_max: nan,
            }
        },
        mark_thickness: nan,
        mark_delay: nan,
        plus_arm: nan,
        plus_taper: nan,
        meantone: base.meantone,
        marvel: base.marvel,
        render_scale: nan,
        spiral_bloom: nan,
        glow_reach: nan,
        atmosphere: AtmosphereSettings {
            material_style: base.atmosphere.material_style,
            material_amount: nan,
            material_shadow_pickup: nan,
            material_color_pickup: nan,
            pigment_reach: nan,
            stars: crate::StarSettings::default(),
            material_settings: crate::MaterialSettings {
                velvet_size: nan,
                velvet_variety: nan,
                velvet_edge: nan,
                velvet_irregularity: nan,
                velvet_shape: nan,
                velvet_square: nan,
                velvet_tilt: nan,
                wash_size: nan,
                wash_fuzz: nan,
                wash_lobe: nan,
                wash_refract: nan,
                wash_layers: nan,
                wash_randomness: nan,
            },
            material_speed: nan,
            material_direction: nan,
            texture_depth: nan,
            texture_scale: nan,
            texture_speed: nan,
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
        let ShadowStyle { kernel: _, width, spread, depth, falloff } = style;
        self.one(format!("{name}.width"), *width);
        self.one(format!("{name}.spread"), *spread);
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
        view,
        edge_softness_points,
        nodes,
        camera,
        node_radius,
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
        pitch_lut,
        pitch_lut_spacing,
        darkest_pitch,
        brightest_pitch,
        marker_unit,
        glow_rows: _,
        glow_timing,
    } = scene;
    let ViewConfig {
        note_animation,
        render_scale,
        note_bloom: bloom_strength,
        glow_reach,
        glow_strength,
        glow_curve,
        shadow,
        glow_wash,
        glow_blend,
        glow_accumulation,
        mark_thickness,
        atmosphere,
        ..
    } = view;
    f.one("edge_softness_points", *edge_softness_points);

    for (i, node) in nodes.iter().enumerate() {
        let NodeInstance {
            lattice_pos: _,
            world_pos,
            activation,
            envelope,
            departing: _,
            slice_progress,
            octaves,
            thickness,
            hovered: _,
            on_home: _,
            scale,
            cents,
            melody_slots: _,
            bass_slots: _,
            melody_level,
            bass_level,
            melody_color,
            bass_color,
            audio_ring,
            glow,
            trail,
        } = node;
        f.vec3(&format!("nodes[{i}].world_pos"), *world_pos);
        f.one(format!("nodes[{i}].activation"), *activation);
        f.one(format!("nodes[{i}].envelope"), *envelope);
        f.many(&format!("nodes[{i}].slice_progress"), *slice_progress);
        f.many(&format!("nodes[{i}].octaves"), *octaves);
        f.many(&format!("nodes[{i}].thickness"), *thickness);
        f.one(format!("nodes[{i}].scale"), *scale);
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

    let NoteAnimationConfig { order: _, lit_first: _, stagger_spread, radial_start } =
        note_animation;
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

    let OctaveLayout { center, span: _ } = octave_layout;
    f.one("octave_layout.center", *center);

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
        material_style: _,
        material_amount,
        material_shadow_pickup,
        material_color_pickup,
        pigment_reach,
        material_settings,
        stars,
        material_speed,
        material_direction,
        texture_depth,
        texture_scale,
        texture_speed,
        breath_amount,
        breath_speed,
    } = atmosphere;
    f.one("atmosphere.stars.star_randomness", stars.star_randomness);
    f.one("atmosphere.stars.star_size_variation", stars.star_size_variation);
    f.one("atmosphere.stars.star_jitter", stars.star_jitter);
    f.many("atmosphere.stars.star_spacing_ratio", stars.star_spacing_ratio);
    f.many("atmosphere.stars.star_size", stars.star_size);
    f.many("atmosphere.stars.star_speed", stars.star_speed);
    f.one("atmosphere.stars.star_lifetime", stars.star_lifetime);
    f.many("atmosphere.stars.star_twinkle", stars.star_twinkle);
    f.many("atmosphere.stars.star_solid", stars.star_solid);
    f.one("atmosphere.stars.star_glow_falloff", stars.star_glow_falloff);
    f.one("atmosphere.stars.star_resolution", stars.star_resolution);
    f.one("atmosphere.material_amount", *material_amount);
    f.one("atmosphere.material_shadow_pickup", *material_shadow_pickup);
    f.one("atmosphere.material_color_pickup", *material_color_pickup);
    f.one("atmosphere.pigment_reach", *pigment_reach);
    let crate::MaterialSettings {
        velvet_size,
        velvet_variety,
        velvet_edge,
        velvet_irregularity,
        velvet_shape,
        velvet_square,
        velvet_tilt,
        wash_size,
        wash_fuzz,
        wash_lobe,
        wash_refract,
        wash_layers,
        wash_randomness,
    } = material_settings;
    f.one("atmosphere.material_settings.velvet_size", *velvet_size);
    f.one("atmosphere.material_settings.velvet_variety", *velvet_variety);
    f.one("atmosphere.material_settings.velvet_edge", *velvet_edge);
    f.one("atmosphere.material_settings.velvet_irregularity", *velvet_irregularity);
    f.one("atmosphere.material_settings.velvet_shape", *velvet_shape);
    f.one("atmosphere.material_settings.velvet_square", *velvet_square);
    f.one("atmosphere.material_settings.velvet_tilt", *velvet_tilt);
    f.one("atmosphere.material_settings.wash_size", *wash_size);
    f.one("atmosphere.material_settings.wash_fuzz", *wash_fuzz);
    f.one("atmosphere.material_settings.wash_lobe", *wash_lobe);
    f.one("atmosphere.material_settings.wash_refract", *wash_refract);
    f.one("atmosphere.material_settings.wash_layers", *wash_layers);
    f.one("atmosphere.material_settings.wash_randomness", *wash_randomness);

    f.one("atmosphere.material_speed", *material_speed);
    f.one("atmosphere.material_direction", *material_direction);
    f.one("atmosphere.texture_depth", *texture_depth);
    f.one("atmosphere.texture_scale", *texture_scale);
    f.one("atmosphere.texture_speed", *texture_speed);
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

    let mut normalized = poisoned_view();
    normalized.sanitize();
    // The requested window is independent of the view's repaired dials.
    let mut expected = derive_scene(
        &sounding(),
        &Tuning::default(),
        &normalized,
        &poisoned_view().reach(),
        &plain_frame(),
        Camera::default(),
        None,
    );
    NodeMotion::default().step(
        &mut expected,
        &sounding(),
        &Tuning::default(),
        &normalized,
        &normalized.envelope(&plain_frame()),
        &RingFade::default(),
        0.0,
    );
    assert_eq!(scene.view, expected.view);
    assert_eq!(scene_floats(&scene).0, scene_floats(&expected).0);
    assert_eq!(off_sheet, normalized.sevens_size);

    let names = broken(&scene);
    assert!(names.is_empty(), "a NaN view reached the scene at: {}", names.join(", "));

    // A nonzero arm makes the marker walk reach its geometry and ink too.
    let drawable = ViewConfig { plus_arm: 0.5, ..poisoned_view() };
    let scene = scene_of(&sounding(), &Tuning::default(), &drawable, &plain_frame(), 0.0);
    assert!(!scene.pluses.is_empty(), "a real arm still shipped no marker field");
    let names = broken(&scene);
    assert!(names.is_empty(), "a NaN view reached the marked scene at: {}", names.join(", "));
}

/// An angle a hair below zero is stored as 0, not as 360: a second sanitize
/// must leave the view alone, or the live view and its sanitized copy disagree
/// (`panes/lattice.rs` asserts they agree in debug builds).
#[test]
fn angles_just_below_zero_sanitize_once() {
    let below = -1e-6_f32;
    assert_eq!(below.rem_euclid(360.0), 360.0, "the fixture must reach the rounding");
    let mut view = ViewConfig::default();
    view.atmosphere.material_direction = below;
    view.pitch_gradient.hue_start = below;
    view.sanitize();
    assert_eq!(view.atmosphere.material_direction, 0.0);
    assert_eq!(view.pitch_gradient.hue_start, 0.0);
    let mut again = view.clone();
    again.sanitize();
    assert_eq!(again, view);
    let spectral = SpectralAtmosphere { cloud_direction: below, ..Default::default() }.sanitized();
    assert_eq!(spectral.cloud_direction, 0.0);
}
