//! Loaded-state range guard, additional to settings layout/interaction tests.
//!
//! This matrix is MANUAL: new pages, conditional controls and state fields need
//! a scenario/fixture and a visit expectation here. SETTINGS_PANES only checks
//! our page inventory, not future conditional coverage. Disabled bars are still
//! recorded. Gradients, StackBar, OctaveStrip, LayerStrip, choices, text
//! fields, RangeBar ordering/minimum spans and gestures are outside this guard,
//! as are renderer clamps and semantic unit mappings.

use super::harness::{DisplayPage, SettingsPane, SETTINGS_PANES};
use super::probe::{fresh, themed};
use crate::widgets::range_probe::collect;
use crate::*;
use harmonigraph_core::configuration::{ConfigMutation, ConfigReducer, PolicyConfig, TuningModes};
use harmonigraph_scene::{
    Projection, ShadowKernel, SpectralAtmosphere, SpectralReading, ViewConfig, SEVENS_LAYER_LIMIT,
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
    poison!(a.view; spacing, render_scale, bloom_strength, sevens_size, label_scale, sounding_ink,
        octave_center, octave_extra_size, octave_extra_blend, mark_delay, fade_shape,
        spectral_ring_gate, spectral_ring_hysteresis, spectral_ring_attack, spectral_ring_release,
        spectral_width, spectral_ring_range, spectral_ring_width, ring_gap, octave_gap,
        ring_inner, band_width, mark_thickness, lattice_ground, marker_ink,
        plus_arm, plus_taper, plus_width, glow_reach, glow_strength, glow_accumulation,
        glow_blend, glow_wash, glow_attack, glow_release);
    a.view.glow_curve.shape = v;
    poison!(a.view.note_animation; radial_start, start_size, stagger_spread);
    poison!(a.view.atmosphere; nebula_depth, nebula_scale, nebula_speed,
        breath_amount, breath_speed);
    a.view.min_sevens = n;
    a.view.max_sevens = n;
    a.view.center_sevens = n;
    for shadow in a.view.shadow.groups_mut() {
        poison!(shadow; width, depth, falloff);
    }
    poison!(a.camera; yaw, pitch, cabinet_angle, cabinet_scale);
    poison!(a.spectrum; low_midi, high_midi, marking_scale, floor_db, ceiling_db,
        attack, release, keyline, roll_seconds, roll_thickness, roll_opacity, roll_lead,
        roll_lead_fade, roll_lead_release, note_name_scale, volume_floor_db, volume_ceiling_db);
    // Every dialled float on the atmosphere, not just the eight that predate
    // the cloud. #888, #909, #913, #918, #928 and #933 each added, retired or
    // re-ranged bars here and each walked past this list, so the sixteen cloud
    // dials loaded at their fresh values and the `range.contains` check below
    // passed over them vacuously -- sixteen bars reported green by a guard that
    // could not reach them. #933's contract is that a size the bar can offer is
    // a size the blob keeps; this is what holds the bar and the clamp to one
    // pair of numbers.
    poison!(a.spectrum.atmosphere; pitch_softness, time_softness, spread, contour_strength, contours, contour_softness, analyzer_softness, note_glow,
        cloud_depth, cloud_speed, scale_size, scale_variety, scale_refract, scale_relief, scale_rock,
        wash_size, wash_fuzz, wash_ragged, wash_lobe, wash_refract, wash_pool, wash_grain, wash_layers, wash_black);
    saved.workspace.interaction.ui_scale = v;
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
    //
    // `view.spacing` joins them for the same reason and one more: it is in the
    // `poison!` block above because the completeness guard below demands every
    // dialled float be poisoned, and until #912 nothing read the result — the
    // one field in that block whose value was written and then never looked
    // at. This is what reads it, and it is the only place that can: no bar in
    // the recorded set shows a spacing.
    assert!(
        (harmonigraph_scene::SPACING_MIN..=harmonigraph_scene::SPACING_MAX)
            .contains(&state.picture.appearance.view.spacing),
        "a poisoned spacing loaded as {}",
        state.picture.appearance.view.spacing,
    );
    assert!((1.0..=8.0).contains(&state.picture.appearance.spiral.zoom));
    assert!(state.picture.appearance.spiral.look.length() <= 1.0);
    assert!(
        (STOP_BAR_RANGE.0..=STOP_BAR_RANGE.1).contains(&state.picture.appearance.render.stop_bar)
    );
    assert!((0.05..=0.95).contains(&state.picture.appearance.render.frame.split));
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
    pane: SettingsPane,
    expanded: bool,
    projection: Projection,
    enabled: bool,
    meantone: bool,
    marvel: bool,
    /// Draw the Analyzer page's cloud dials for the watercolour wash rather
    /// than for the refracting scales. Two constructions sharing three bars, so
    /// the page has two inventories and only one of them is the fresh state's.
    wash: bool,
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
        wash: false,
        visits: 0,
    };
    let mut cases = Vec::new();
    for pane in SETTINGS_PANES {
        let visits = match pane {
            SettingsPane::Tab(panes::Tab::Tuning) => 7,
            SettingsPane::Page(DisplayPage::Colors) => 2,
            SettingsPane::Page(DisplayPage::Lattice) => 26,
            SettingsPane::Page(DisplayPage::Analyzer) => 27,
            SettingsPane::Page(DisplayPage::Lighting) => 26,
            SettingsPane::Page(DisplayPage::System) => 2,
            SettingsPane::Tab(panes::Tab::Video | panes::Tab::Console | panes::Tab::Notes) => 0,
            _ => panic!("add the new settings page's range scenario"),
        };
        cases.push(Scenario { pane, visits, ..base });
        // Enabled/disabled sections still draw their bars: labels, fringe,
        // marks, audio reading, sevens, roll/note names, glow and shadow falloff.
        cases.push(Scenario { pane, visits, enabled: true, ..base });
    }
    // The wash's own inventory: it takes the five scale bars off the Analyzer
    // page and puts nine of its own there, and nothing else on the page moves.
    // Its own scenario rather than a flag on the loop above because the fresh
    // state selects the scales, so without this the nine are drawn by no case
    // here at all.
    cases.push(Scenario {
        pane: SettingsPane::Page(DisplayPage::Analyzer),
        wash: true,
        visits: 31,
        ..base
    });
    for projection in [Projection::Perspective, Projection::Orthographic] {
        cases.push(Scenario {
            pane: SettingsPane::Page(DisplayPage::Lattice),
            projection,
            enabled: true,
            visits: 26,
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
        a.view.show_labels = scenario.enabled;
        a.view.mark_melody = scenario.enabled;
        a.view.mark_bass = scenario.enabled;
        a.view.spectral_reading =
            if scenario.enabled { SpectralReading::Spectrum } else { SpectralReading::Fold };
        a.spectrum.show_roll = scenario.enabled;
        a.spectrum.show_spectrogram = scenario.enabled;
        a.spectrum.note_names = scenario.enabled;
        a.spectrum.atmosphere.cloud_style = if scenario.wash {
            harmonigraph_scene::CloudStyle::Watercolor
        } else {
            harmonigraph_scene::CloudStyle::Mosaic
        };
        a.view.show_perf = scenario.enabled;
        a.view.atmosphere.enabled = scenario.enabled;
        for style in a.view.shadow.groups_mut() {
            style.kernel =
                if scenario.enabled { ShadowKernel::Distance } else { ShadowKernel::Gaussian };
        }
        state.workspace.interaction.take.supported = scenario.enabled;
        state.workspace.interaction.take.last_ready = scenario.enabled;
        let mut tab = scenario.pane.install(&mut state);
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
                    egui_dock::TabViewer::ui(&mut viewer, ui, &mut tab);
                },
            );
        });
        assert_eq!(visits.len(), scenario.visits, "{edge:?} {scenario:?}: {visits:?}");
        let saw = |label: &str| visits.iter().any(|visit| visit.label == label);
        if scenario.pane == SettingsPane::Page(DisplayPage::Lattice) {
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
        if scenario.pane == SettingsPane::Tab(panes::Tab::Tuning) {
            assert!(saw("Pitch flexibility"));
            assert_eq!(saw("Fifth") && saw("Half-life"), scenario.expanded);
        }
        for visit in visits {
            if visit.label == "Contours" {
                assert_eq!(visit.range, 2.0..=64.0);
                assert_eq!(
                    visit.values,
                    vec![match edge {
                        Edge::Low => 2.0,
                        Edge::High => 64.0,
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

/// The `poison!` block above is hand-written, so a float added to `ViewConfig`
/// joins this guard only if somebody remembers to name it there — `spacing`
/// and `spectral_ring_width` were both simply missing, and the second of those
/// is a handle on the Layers bar.
///
/// So the list is checked against the struct's OWN serialized form rather than
/// against a second list: every top-level key whose fresh value is a decimal
/// number is a dialled float, and the guard has to have moved it. Integers
/// (the sevens stack, the naming reach, the octave wheel) and choices carry no
/// decimal point, and a nested struct is not a number — the curve, the note
/// animation, the atmosphere, the gradient and the four shadow groups are each
/// poisoned or excluded on their own terms above.
#[test]
fn the_loaded_state_guard_poisons_every_dialled_view_float() {
    let mut saved = fresh();
    poison(&mut saved, Edge::High);
    let opened = ron::to_string(&ViewConfig::default()).expect("a view serializes");
    let poisoned = ron::to_string(&saved.picture.appearance.view).expect("a view serializes");
    let (mut dialled, mut missed) = (0, Vec::new());
    for ((key, was), (again, now)) in top_level(&opened).into_iter().zip(top_level(&poisoned)) {
        assert_eq!(key, again, "two serializations of one struct named different fields");
        if !was.contains('.') || was.parse::<f32>().is_err() {
            continue;
        }
        dialled += 1;
        if was == now {
            missed.push(key);
        }
    }
    assert!(missed.is_empty(), "the loaded-state guard never poisons {missed:?}");
    // What counts as a dialled float is read off the spelling, so a change in
    // RON's would otherwise leave this passing over nothing at all.
    assert!(dialled > 30, "only {dialled} of the view's fields read as dialled floats");
}
