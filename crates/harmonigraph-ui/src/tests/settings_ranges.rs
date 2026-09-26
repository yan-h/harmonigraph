//! Loaded-state range guard, additional to settings layout/interaction tests.
//!
//! This matrix is MANUAL: new pages, conditional controls and state fields need
//! a scenario/fixture and a visit expectation here. SETTINGS_PANES only checks
//! our page inventory, not future conditional coverage. Disabled bars are
//! recorded, while hidden bars need an enabled scenario that makes them draw.
//! Gradients, StackBar, OctaveStrip, LayerStrip, choices, text fields, RangeBar
//! ordering/minimum spans and gestures are outside this guard, as are renderer
//! clamps and semantic unit mappings.

use super::harness::SETTINGS_PANES;
use super::probe::{fresh, themed};
use crate::widgets::range_probe::collect;
use crate::*;
use harmonigraph_core::configuration::{ConfigMutation, ConfigReducer, PolicyConfig, TuningModes};
use harmonigraph_scene::{
    Projection, ShadowKernel, SpectralAtmosphere, SpectralReading, SEVENS_LAYER_LIMIT,
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Edge {
    Low,
    High,
    /// Nothing poisoned at all: the FRESH values cross the same load door and
    /// are held to the same bars.
    ///
    /// The one case the two edges above can never see, because each overwrites
    /// every field it then checks. A default outside its own bar is the same
    /// defect a poisoned value is — a number the file holds that the control
    /// cannot show — and it arrives the way `64f7d41e` arrived, by a capture
    /// from the DAW moving a default rather than by anyone hand-editing a
    /// blob.
    Fresh,
}

/// Poison all stored fields feeding the recorded appearance/editor bars,
/// including the reach/fade pairs whose bar displays a subtraction. Values are
/// written BEFORE serialization, never clamped by a fixture beside the
/// production door.
fn poison(saved: &mut SharedState, edge: Edge) {
    let (v, n) = match edge {
        Edge::Low => (-1.0e6, i32::MIN),
        Edge::High => (1.0e6, i32::MAX),
        Edge::Fresh => return,
    };
    let a = &mut saved.picture.appearance;
    macro_rules! poison { ($owner:expr; $($field:ident),+ $(,)?) => { $( $owner.$field = v; )+ }; }
    poison!(a.view; render_scale, spiral_bloom, sevens_size, label_scale,
        octave_center, octave_extra_size, octave_extra_blend, mark_delay, fade_shape,
        spectral_ring_gate, spectral_ring_hysteresis, spectral_ring_attack, spectral_ring_release,
        spectral_width, spectral_ring_range, spectral_ring_width, ring_gap,
        ring_inner, band_width, mark_thickness, lattice_ground, marker_ink,
        plus_arm, plus_taper, glow_reach, glow_strength, glow_accumulation,
        glow_blend, glow_wash, glow_attack, glow_release);
    a.view.glow_curve.shape = v;
    poison!(a.view.note_animation; radial_start, stagger_spread);
    poison!(a.view.intensity; gain_range, glow_base, opacity_rest, thickness_max);
    poison!(a.view.intensity.velocity; weight);
    poison!(a.view.intensity.gain; weight);
    poison!(a.view.intensity.pressure; weight);
    poison!(a.view.intensity.timbre; weight);
    poison!(a.view.atmosphere; nebula_depth, nebula_scale, nebula_speed,
        breath_amount, breath_speed);
    a.view.min_sevens = n;
    a.view.max_sevens = n;
    a.view.center_sevens = n;
    for shadow in a.view.shadow.groups_mut() {
        poison!(shadow; width, depth, falloff);
    }
    // `distance` is the zoom and has no bar of its own — navigation rather
    // than a dial — but it crosses this same door and `Camera::sanitize`
    // clamps it there, so it is poisoned like the other no-bar owners below
    // and checked directly in `loaded`.
    poison!(a.camera; yaw, pitch, distance, cabinet_angle, cabinet_scale);
    poison!(a.spectrum; low_midi, high_midi, marking_scale, floor_db, ceiling_db,
        attack, release, keyline_lift, roll_seconds, roll_thickness, roll_opacity, roll_lead,
        roll_lead_fade, roll_lead_release, note_name_scale, volume_floor_db, volume_ceiling_db,
        backdrop_strength, backdrop_height, backdrop_gap);
    // Two more that cross this door without a bar of their own. `tilt` is a
    // CHOICE that happens to be spelled as a float — its repair snaps to the
    // nearest offered step rather than clamping — and `roll_fraction` is set
    // by dragging the divider. Both are checked directly in `loaded`.
    poison!(a.spectrum; tilt, roll_fraction);
    // Every dialled float on the atmosphere, not just the eight that predate
    // the cloud. #888, #909, #913, #918, #928 and #933 each added, retired or
    // re-ranged bars here and each walked past this list, so the cloud dials
    // loaded at their fresh values and the `range.contains` check below passed
    // over them vacuously -- bars reported green by a guard that could not
    // reach them. #933's contract is that a size the bar can offer is a size
    // the blob keeps; this is what holds the bar and the clamp to one pair of
    // numbers.
    poison!(a.spectrum.atmosphere; pitch_softness, time_softness, spread, blur_time_step, contour_strength, contours, contour_softness,
        cloud_depth, cloud_speed, cloud_direction, scale_size, scale_variety, scale_refract,
        wash_size, wash_fuzz, wash_lobe, wash_refract, wash_layers,
        star_density, star_randomness, star_fringe,
        star_far_speed, star_defocus, star_size_min, star_size_max,
        star_size_curve, star_speed_curve, star_speed_spread, star_lifetime);
    saved.workspace.interaction.ui_scale = v;
    poison!(saved.workspace.interaction.skin_dials; lightness, tint_hue, tint, accent_hue, accent_saturation);
    // These owners have NO ValueBar/RangeBar today. Still pass through their
    // real shared load boundary; zero Video visits below explicitly records
    // that its text/choice/divider controls are not range-guard coverage.
    a.spiral.zoom = v;
    a.spiral.look = glam::Vec2::splat(v);
    a.render.stop_bar = v as f64;
    a.render.frame.split = v;
}

/// The poisoned state through its real shared load boundary, with the owners
/// that have no recorded bar checked on the way out.
fn loaded(edge: Edge) -> SharedState {
    let mut saved = fresh();
    poison(&mut saved, edge);
    let serialized = saved.save_persist();
    let mut state = fresh();
    assert!(state.load_persist(&serialized));
    // Spiral and take-render settings share the load boundary but currently
    // have no recorded bar. Check their own normalization directly so adding
    // zero-visit panes to the matrix does not pretend the bar guard covers them.
    assert!((1.0..=8.0).contains(&state.picture.appearance.spiral.zoom));
    assert!(state.picture.appearance.spiral.look.length() <= 1.0);
    assert!(
        (STOP_BAR_RANGE.0..=STOP_BAR_RANGE.1).contains(&state.picture.appearance.render.stop_bar)
    );
    assert!((0.05..=0.95).contains(&state.picture.appearance.render.frame.split));
    // The zoom, for the same reason: no bar, but `Camera::sanitize` is the one
    // door holding it to the range `zoom`/`zoom_by` hold a drag to.
    let camera = &state.picture.appearance.camera;
    assert!(
        (harmonigraph_scene::Camera::MIN_DISTANCE..=harmonigraph_scene::Camera::MAX_DISTANCE)
            .contains(&camera.distance),
        "a poisoned zoom loaded at {}",
        camera.distance,
    );
    // The spectrum's own two: a choice spelled as a float, which must come
    // back as one of the settings actually offered rather than merely inside
    // their span, and the divider's split.
    let spectrum = &state.picture.appearance.spectrum;
    assert!(
        TILT_STEPS.contains(&spectrum.tilt),
        "a poisoned tilt loaded at {}, which is no step the control offers",
        spectrum.tilt,
    );
    assert!((0.0..=1.0).contains(&spectrum.roll_fraction));
    // The sevens stack is a LayerStrip rather than a pair of bars, so it left
    // the recorded set when the two merged. Its three integers still cross this
    // door, and the invariant is the strip's own: both ends on the axis, home
    // between them and so on a sheet the picture draws.
    let view = &state.picture.appearance.view;
    let axis = -SEVENS_LAYER_LIMIT..=SEVENS_LAYER_LIMIT;
    assert!(axis.contains(&view.min_sevens) && axis.contains(&view.max_sevens));
    assert!(
        view.min_sevens <= view.center_sevens && view.center_sevens <= view.max_sevens,
        "home {} is outside the stack {}..{}",
        view.center_sevens,
        view.min_sevens,
        view.max_sevens,
    );
    // Export parses the same document through the other production entry.
    let exported = AppearanceDocument::parse(&saved.picture.appearance.serialize()).unwrap();
    assert_eq!(state.picture.appearance.serialize(), exported.serialize());
    state
}

struct Backend {
    config: params::ConfigurationView,
}
impl ParamBackend for Backend {
    // Host-owned automatable params are not in UiPersist or normalized by UI.
    // Use valid host defaults here; this does not test the host's restore code.
    fn get(&self, key: params::ParamKey) -> f32 {
        key.default_value()
    }
    fn set(&self, _: params::ParamKey, _: f32) {
        panic!("drawing wrote a host parameter");
    }
    fn configuration(&self) -> Option<params::ConfigurationView> {
        Some(self.config)
    }
}
fn backend(edge: Edge, meantone: bool, marvel: bool) -> Backend {
    let mut policy = PolicyConfig::default();
    // `Fresh` leaves the host's own defaults standing, for the reason `poison`
    // leaves the appearance standing: what it asks is whether a first-ever
    // load reads out values its bars can show.
    if edge != Edge::Fresh {
        let high = edge == Edge::High;
        macro_rules! poison { ($($field:ident),+ $(,)?) => { $( policy.$field = if high { std::primitive::u16::MAX as _ } else { 0 }; )+ }; }
        poison!(pitch_flexibility, half_life_ms, register);
        policy.radius = if high { u8::MAX } else { 0 };
        policy.tolerance = if high { u32::MAX } else { 0 };
        policy.silence_ms = if high { u32::MAX } else { 0 };
        policy.keyboard = [if high { i32::MAX } else { i32::MIN }; 3];
    }
    let mut reducer = ConfigReducer::default();
    let modes = TuningModes {
        tempered: harmonigraph_core::Tempered { syntonic: meantone, septimal_kleisma: marvel },
        // Auto detection would immediately overwrite the branch this scenario
        // selected when the restored default tuning is judged.
        auto: [false; 2],
        learning: false,
    };
    assert!(reducer.apply(ConfigMutation::Restore {
        raw: harmonigraph_core::Tuning::default(),
        modes,
        policy
    }));
    Backend {
        config: params::ConfigurationView {
            resolved: reducer.resolved(),
            status: 0,
            pending: false,
        },
    }
}

#[derive(Clone, Copy, Debug)]
struct Scenario {
    pane: panes::Tab,
    expanded: bool,
    projection: Projection,
    enabled: bool,
    meantone: bool,
    marvel: bool,
    /// Which texture's dials the Spectrogram page draws. Three constructions
    /// sharing three bars, so the page has three inventories and only one of
    /// them is the fresh state's.
    style: harmonigraph_scene::CloudStyle,
    visits: usize,
}

fn scenarios() -> Vec<Scenario> {
    let base = Scenario {
        pane: SETTINGS_PANES[0],
        expanded: false,
        projection: Projection::Cabinet,
        enabled: false,
        meantone: false,
        marvel: false,
        style: harmonigraph_scene::CloudStyle::Mosaic,
        visits: 0,
    };
    let mut cases = Vec::new();
    for &pane in SETTINGS_PANES {
        let visits = match pane {
            panes::Tab::Tuning => 7,
            // The pitch colors, then Note intensity: four weights, Gain range,
            // Bloom base, Opacity base and Thickness max.
            panes::Tab::Colors => 2 + 8,
            // The picture, then the background glow (8) with its texture
            // switched off, then two shadow groups of two bars each.
            panes::Tab::LatticeSettings => 16 + 8 + 4,
            // The analyzer's view and axes (5) and analysis (3) with the
            // spectrogram and ribbons switched off, the Spiral's bloom, two
            // shadow groups.
            panes::Tab::AnalyzerSettings => 5 + 3 + 1 + 4,
            panes::Tab::System => 8,
            panes::Tab::Video | panes::Tab::Console => 0,
            _ => panic!("add the new settings page's range scenario"),
        };
        cases.push(Scenario { pane, visits, ..base });
        // Exercise the conditional groups too: labels, fringe, marks, audio
        // reading, sevens, the glow texture, roll/note names, the spectrogram,
        // backdrop, glow and Contour shadow falloff (one bar in each of a
        // page's two groups).
        let visits = match pane {
            panes::Tab::LatticeSettings => visits + 5 + 6 + 2,
            // ...the spectrogram's twelve, the ribbons' five, and the
            // backdrop's height and stripe spacing.
            panes::Tab::AnalyzerSettings => visits + 12 + 5 + 2 + 2,
            _ => visits,
        };
        cases.push(Scenario { pane, visits, enabled: true, ..base });
    }
    // The wash's own inventory: it takes the three scale bars off the Spectrogram
    // section and puts five of its own there, and nothing else on the page moves.
    // Its own scenario rather than a flag on the loop above because the fresh
    // state selects the scales, so without this the five are drawn by no case
    // here at all.
    cases.push(Scenario {
        pane: panes::Tab::AnalyzerSettings,
        style: harmonigraph_scene::CloudStyle::Watercolor,
        enabled: true,
        visits: 13 + 12 + 5 + 2 + 2 - 3 + 5,
        ..base
    });
    // The starfield's, on the same terms: the three scale bars off, ten of
    // its own on.
    cases.push(Scenario {
        pane: panes::Tab::AnalyzerSettings,
        style: harmonigraph_scene::CloudStyle::Stars,
        enabled: true,
        visits: 13 + 12 + 5 + 2 + 2 - 3 + 10,
        ..base
    });
    for projection in [Projection::Perspective, Projection::Orthographic] {
        cases.push(Scenario {
            pane: panes::Tab::LatticeSettings,
            projection,
            enabled: true,
            visits: 22 + 13 + 6,
            ..base
        });
    }
    // The collapsed Keyboard and Context groups hide three and four bars.
    // Cover both open for each comma derivation branch, including both modes.
    for (meantone, marvel) in [(false, false), (true, false), (false, true), (true, true)] {
        cases.push(Scenario { expanded: true, meantone, marvel, visits: 14, ..base });
    }
    cases
}

fn check(edge: Edge) {
    let mut violations = Vec::new();
    for scenario in scenarios() {
        let mut state = loaded(edge);
        let backend = backend(edge, scenario.meantone, scenario.marvel);
        state.picture.runtime.observe_configuration(&mut state.picture.appearance, &backend);
        assert_eq!(state.picture.appearance.view.meantone, scenario.meantone);
        assert_eq!(state.picture.appearance.view.marvel, scenario.marvel);
        let a = &mut state.picture.appearance;
        a.camera.projection = scenario.projection;
        a.view.spectral_reading =
            if scenario.enabled { SpectralReading::Spectrum } else { SpectralReading::Fold };
        a.view.spectral_ring_width = if scenario.enabled { 0.1 } else { 0.0 };
        a.spectrum.show_roll = scenario.enabled;
        a.spectrum.show_spectrogram = scenario.enabled;
        a.spectrum.note_names = scenario.enabled;
        // Strength 0 is the backdrop's off. On, a strength the load clamped up
        // to the bar's top stays there for the bar to be held to.
        a.spectrum.backdrop_strength =
            if scenario.enabled { a.spectrum.backdrop_strength.max(0.85) } else { 0.0 };
        a.spectrum.atmosphere.cloud_style = scenario.style;
        a.view.show_perf = scenario.enabled;
        a.view.atmosphere.enabled = scenario.enabled;
        for style in a.view.shadow.groups_mut() {
            style.kernel =
                if scenario.enabled { ShadowKernel::Distance } else { ShadowKernel::Gaussian };
        }
        state.workspace.interaction.take.supported = scenario.enabled;
        state.workspace.interaction.take.last_ready = scenario.enabled;
        let mut tab = scenario.pane;
        let ctx = themed();
        // Observe the first load frame once, before any discarded egui pass
        // could mutate a value and make a later pass look normalized.
        ctx.options_mut(|options| options.max_passes = std::num::NonZeroUsize::new(1).unwrap());
        ctx.memory_mut(|memory| memory.set_everything_is_visible(scenario.expanded));
        let visits = collect(|| {
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 4000.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let mut viewer = panes::Viewer {
                        state: &mut state.picture,
                        interaction: &mut state.workspace.interaction,
                        params: &backend,
                        now: 0.0,
                    };
                    viewer.ui(ui, &mut tab);
                },
            );
        });
        assert_eq!(visits.len(), scenario.visits, "{edge:?} {scenario:?}: {visits:?}");
        let saw = |label: &str| visits.iter().any(|visit| visit.label == label);
        if scenario.pane == panes::Tab::LatticeSettings {
            match scenario.projection {
                Projection::Cabinet => {
                    assert!(saw("Depth angle") && saw("Depth step scale"));
                    assert!(!saw("Horizontal angle") && !saw("Vertical angle"));
                }
                Projection::Perspective | Projection::Orthographic => {
                    assert!(saw("Horizontal angle") && saw("Vertical angle"));
                    assert!(!saw("Depth angle") && !saw("Depth step scale"));
                }
            }
        }
        if scenario.pane == panes::Tab::Tuning {
            assert!(saw("Pitch flexibility"));
            assert_eq!(saw("Fifth") && saw("Half-life"), scenario.expanded);
        }
        // The lattice and the ribbons share one bloom on the Mappings page; the
        // Spiral keeps its own.
        let blooms: &[&str] = match scenario.pane {
            panes::Tab::Colors => &["Bloom base"],
            panes::Tab::AnalyzerSettings => &["Spiral bloom"],
            _ => &[],
        };
        for &label in blooms {
            assert!(saw(label), "{scenario:?} drew no {label:?} bar");
        }
        assert!(!saw("Bloom") && !saw("Ribbon bloom"), "{scenario:?} drew a retired bloom bar");
        for visit in visits {
            if visit.label == "Contour levels" {
                assert_eq!(
                    visit.values,
                    vec![match edge {
                        Edge::Low => *visit.range.start(),
                        Edge::High => *visit.range.end(),
                        Edge::Fresh => SpectralAtmosphere::default().contours,
                    }]
                );
            }
            for value in &visit.values {
                if !visit.range.contains(value) {
                    violations.push(format!("{edge:?} {scenario:?}: {visit:?}"));
                }
            }
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn loaded_settings_low_fit_real_bars() {
    check(Edge::Low);
}
#[test]
fn loaded_settings_high_fit_real_bars() {
    check(Edge::High);
}
#[test]
fn fresh_settings_fit_real_bars() {
    check(Edge::Fresh);
}

/// A fresh appearance is a fixed point of the door it loads through: nothing
/// `normalize` does moves a default.
///
/// Five owners, because that is what `AppearanceDocument::normalize` calls,
/// and one assertion because a document that survived its own sanitizer
/// serializes back to itself. A default that did NOT survive would be repaired
/// on the way in with nothing on screen saying so — the picture a fresh
/// install draws would not be the picture `impl Default` names — and the
/// existing edge guards cannot see it, each of them overwriting every field it
/// checks.
///
/// The way that arrives here is not a hand-edited blob: it is a capture from
/// the DAW writing a dialled value in as a default (`64f7d41e`, PRs #717/#834/
/// #836), past a range only `sanitize` knows about.
#[test]
fn a_fresh_appearance_is_a_fixed_point_of_its_own_normalize() {
    let opened = AppearanceDocument::default();
    let normalized = opened.clone().normalize().expect("the fresh appearance normalizes");
    assert_eq!(
        opened.serialize(),
        normalized.serialize(),
        "the load door repairs a value the fresh appearance ships with",
    );
}

/// Every `key:value` at the top level of one serialized struct, in order. The
/// depth count is what keeps a nested struct's own commas out of the split.
///
/// A narrower walk than `tests::persist`'s `top_level_pairs`, which takes a
/// whole blob apart and is that module's own.
fn top_level(serialized: &str) -> Vec<(&str, &str)> {
    let inner = serialized
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .expect("a serialized struct is parenthesized");
    let (mut fields, mut depth, mut start) = (Vec::new(), 0i32, 0usize);
    for (at, c) in inner.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                fields.push(&inner[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    fields.push(&inner[start..]);
    fields.into_iter().map(|field| field.split_once(':').expect("a pair has a colon")).collect()
}

/// Compare one owner's direct float fields against its own serialized shape,
/// so the hand-written poison inventory cannot silently leave one at default.
/// RON decimal spelling is a heuristic, not type reflection: integers, choices,
/// nested values and floats without a decimal point are not recognized here.
fn assert_poisoned_float_fields<T: serde::Serialize>(label: &str, opened: &T, poisoned: &T) {
    let opened = ron::to_string(opened).expect("settings serialize");
    let poisoned = ron::to_string(poisoned).expect("settings serialize");
    let (before, after) = (top_level(&opened), top_level(&poisoned));
    assert_eq!(before.len(), after.len(), "two serializations of {label} differ in length");
    let (mut dialled, mut missed) = (0, Vec::new());
    for ((key, was), (again, now)) in before.into_iter().zip(after) {
        assert_eq!(key, again, "two serializations of {label} named different fields");
        if !was.contains('.') || was.parse::<f32>().is_err() {
            continue;
        }
        dialled += 1;
        if was == now {
            missed.push(key);
        }
    }
    assert!(missed.is_empty(), "the loaded-state guard never poisons {label} {missed:?}");
    // Detect an owner going wholly unrecognized if RON's spelling changes.
    // This does not prove that every float spelling was recognized.
    assert!(dialled > 0, "none of {label}'s fields read as dialled floats");
}

/// Every direct float in these owners belongs in the poison fixture. The
/// nested owners also contain choices/booleans; only their floats are checked.
/// Owners mixing dialled and non-dialled floats (such as gradients) need their
/// own treatment, and future nested owners must be added explicitly. This is
/// fixture completeness, not discovery of controls: the conditional scenarios,
/// real bar probes and visit assertions above still establish UI coverage.
#[test]
fn the_loaded_state_guard_poisons_every_dialled_view_float() {
    let opened = fresh();
    let mut saved = fresh();
    poison(&mut saved, Edge::High);
    let old = &opened.picture.appearance;
    let new = &saved.picture.appearance;
    assert_poisoned_float_fields("view", &old.view, &new.view);
    // The two owners still hand-listed in `poison` whose OWN completeness
    // nothing checked. Neither is short a field today; both are where the next
    // added float would have gone missing quietly, this range alone having
    // removed one from `camera` and reshaped `spectrum`'s surroundings.
    assert_poisoned_float_fields("camera", &old.camera, &new.camera);
    assert_poisoned_float_fields("spectrum", &old.spectrum, &new.spectrum);
    assert_poisoned_float_fields(
        "view.note_animation",
        &old.view.note_animation,
        &new.view.note_animation,
    );
    assert_poisoned_float_fields("view.intensity", &old.view.intensity, &new.view.intensity);
    let sources = |i: &harmonigraph_scene::IntensitySettings| {
        [("velocity", i.velocity), ("gain", i.gain), ("pressure", i.pressure), ("timbre", i.timbre)]
    };
    for ((name, before), (_, after)) in
        sources(&old.view.intensity).into_iter().zip(sources(&new.view.intensity))
    {
        assert_poisoned_float_fields(&format!("view.intensity.{name}"), &before, &after);
    }
    assert_poisoned_float_fields("view.glow_curve", &old.view.glow_curve, &new.view.glow_curve);
    assert_poisoned_float_fields("view.atmosphere", &old.view.atmosphere, &new.view.atmosphere);
    assert_poisoned_float_fields(
        "spectrum.atmosphere",
        &old.spectrum.atmosphere,
        &new.spectrum.atmosphere,
    );
    for (index, (before, after)) in
        old.view.shadow.groups().into_iter().zip(new.view.shadow.groups()).enumerate()
    {
        assert_poisoned_float_fields(&format!("view.shadow[{index}]"), &before, &after);
    }
}
