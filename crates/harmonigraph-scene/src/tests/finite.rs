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
    let shadow = ShadowStyle { width: nan, spread: nan, depth: nan };
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
            stagger_spread: nan,
            radial_start: nan,
            fade_out_delay: nan,
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
        render_scale: nan,
        spiral_bloom: nan,
        glow_reach: nan,
        atmosphere: AtmosphereSettings {
            material_style: base.atmosphere.material_style,
            material_amount: nan,
            stars: crate::StarSettings {
                star_randomness: nan,
                star_size_variation: nan,
                star_jitter: nan,
                star_layers: base.atmosphere.stars.star_layers,
                star_spacing_ratio: [nan; crate::star_plan::STAR_DEPTHS],
                star_size: [nan; crate::star_plan::STAR_DEPTHS],
                star_speed: [nan; crate::star_plan::STAR_DEPTHS],
                star_lifetime: nan,
                star_twinkle_dip: nan,
                star_solid: [nan; crate::star_plan::STAR_DEPTHS],
                star_cover: nan,
                star_glow_falloff: nan,
                star_resolution: nan,
                star_solo: base.atmosphere.stars.star_solo,
            },
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
            spectral_text: shadow,
        },
        hide_behind: nan,
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
}

/// Every float the scene DERIVES, flattened.
///
/// Every struct is destructured EXHAUSTIVELY — no `..` anywhere below — so a
/// field added to the scene, to a node or to a marker breaks this walk rather
/// than going unwatched. That is the half of the claim a test cannot assert:
/// the assertion says the floats are real numbers, and this says which floats
/// there are.
///
/// The view the scene carries is not walked here: [`broken_view`] reads every
/// float in it off its debug dump.
fn scene_floats(scene: &Scene) -> Floats {
    let mut f = Floats::default();
    let Scene {
        view: _,
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
    f.one("edge_softness_points", *edge_softness_points);

    for (i, node) in nodes.iter().enumerate() {
        let NodeInstance {
            lattice_pos: _,
            world_pos,
            presence_with_opacity,
            presence_without_opacity,
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
        f.one(format!("nodes[{i}].presence_with_opacity"), *presence_with_opacity);
        f.one(format!("nodes[{i}].presence_without_opacity"), *presence_without_opacity);
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
    f.luts("pitch_lut", pitch_lut);
    f.many("pitch_lut_spacing", [pitch_lut_spacing.at, pitch_lut_spacing.share]);
    f.one("darkest_pitch", *darkest_pitch);
    f.one("brightest_pitch", *brightest_pitch);
    f.one("marker_unit", *marker_unit);

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

    f
}

/// Every value in the view's `{:#?}` dump that is not a real number, by its
/// path through the dump. Read off the debug output rather than walked by
/// hand, so a field added to the view or anything nested in it is covered
/// without being named here. It sees a value only where derived pretty `Debug`
/// prints it alone on its line, so a `Range<f32>`, a `Duration` or a
/// hand-written `Debug` would go unseen.
fn broken_view(view: &ViewConfig) -> Vec<String> {
    // Each open container's name and how many unnamed items it has shown.
    let (mut path, mut broken) = (Vec::<(String, usize)>::new(), Vec::new());
    for line in format!("{view:#?}").lines() {
        let line = line.trim();
        if line.starts_with(['}', ']', ')']) {
            path.pop();
            continue;
        }
        let (name, value) = match (line.split_once(": "), path.last_mut()) {
            (Some((name, value)), _) => (name.to_string(), value),
            (None, Some(open)) => {
                open.1 += 1;
                (format!("[{}]", open.1 - 1), line)
            }
            // The type name the dump opens with.
            (None, None) => (String::new(), line),
        };
        let value = value.trim_end_matches(',');
        if ["NaN", "inf", "-inf"].contains(&value) {
            let at: Vec<_> = path.iter().map(|(name, _)| name.as_str()).chain([&*name]).collect();
            broken.push(format!("{} = {value}", at.join(".").trim_matches('.')));
        }
        if line.ends_with(['{', '[', '(']) {
            path.push((name, 0));
        }
    }
    broken
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
    // The view's half of the claim, by name, before the comparisons below:
    // a NaN would also fail those, unnamed, and an infinity left on both sides
    // would pass them.
    let names = broken_view(&scene.view);
    assert!(names.is_empty(), "the view's repair left: {}", names.join(", "));
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
