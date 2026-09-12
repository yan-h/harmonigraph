//! Loaded-state range guard, additional to settings layout/interaction tests.
//!
//! This matrix is MANUAL: new pages, conditional controls and state fields need
//! a scenario/fixture and a visit expectation here. SETTINGS_PANES only checks
//! our page inventory, not future conditional coverage. Disabled bars are still
//! recorded. Gradients, StackBar, OctaveStrip, choices, text fields, RangeBar
//! ordering/minimum spans and gestures are outside this guard, as are renderer
//! clamps and semantic unit mappings.

use super::harness::{DisplayPage, SettingsPane, SETTINGS_PANES};
use super::probe::{fresh, themed};
use crate::widgets::range_probe::collect;
use crate::*;
use harmonigraph_core::configuration::{ConfigMutation, ConfigReducer, PolicyConfig, TuningModes};
use harmonigraph_scene::{Projection, ShadowKernel, SpectralReading};

#[derive(Clone, Copy, Debug)]
enum Edge {
    Low,
    High,
}
impl Edge {
    fn float(self) -> f32 {
        match self {
            Self::Low => -1.0e6,
            Self::High => 1.0e6,
        }
    }
    fn integer(self) -> i32 {
        match self {
            Self::Low => i32::MIN,
            Self::High => i32::MAX,
        }
    }
}

/// All stored fields feeding the recorded appearance/editor bars, including
/// the reach/fade pairs whose bar displays a subtraction. Values are poisoned
/// BEFORE serialization, never clamped by a fixture beside the production door.
fn loaded(edge: Edge) -> SharedState {
    let mut saved = fresh();
    let a = &mut saved.picture.appearance;
    let v = edge.float();
    macro_rules! poison { ($owner:expr; $($field:ident),+ $(,)?) => { $( $owner.$field = v; )+ }; }
    poison!(a.view; render_scale, bloom_strength, sevens_size, label_scale, sounding_ink,
        octave_center, octave_extra_size, octave_extra_blend, mark_delay, fade_shape,
        spectral_ring_gate, spectral_ring_hysteresis, spectral_ring_attack, spectral_ring_release,
        spectral_width, spectral_ring_range, ring_gap, octave_gap, lattice_ground, marker_ink,
        plus_arm, plus_taper, plus_width, glow_reach, glow_strength, glow_accumulation,
        glow_blend, glow_wash, glow_attack, glow_release);
    a.view.glow_curve.shape = v;
    a.view.extent_sevens = edge.integer();
    a.view.center_sevens = edge.integer();
    for shadow in a.view.shadow.groups_mut() {
        poison!(shadow; width, depth, falloff);
    }
    poison!(a.camera; yaw, pitch, cabinet_angle, cabinet_scale);
    poison!(a.spectrum; low_midi, high_midi, marking_scale, floor_db, ceiling_db,
        attack, release, keyline, roll_seconds, roll_thickness, roll_lead, roll_lead_fade,
        roll_lead_release, note_name_scale, volume_floor_db, volume_ceiling_db);
    saved.workspace.interaction.ui_scale = v;
    // These owners have NO ValueBar/RangeBar today. Still pass through their
    // real shared load boundary; zero Video visits below explicitly records
    // that its text/choice/divider controls are not range-guard coverage.
    a.spiral.zoom = v;
    a.spiral.look = glam::Vec2::splat(v);
    a.render.stop_bar = v as f64;
    a.render.frame.split = v;
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
    let high = matches!(edge, Edge::High);
    macro_rules! poison { ($($field:ident),+ $(,)?) => { $( policy.$field = if high { std::primitive::u16::MAX as _ } else { 0 }; )+ }; }
    poison!(harmonic, pitch_scale, half_life_ms, register);
    policy.radius = if high { u8::MAX } else { 0 };
    policy.tolerance = if high { u32::MAX } else { 0 };
    policy.silence_ms = if high { u32::MAX } else { 0 };
    policy.keyboard = [edge.integer(); 3];
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
        visits: 0,
    };
    let mut cases = Vec::new();
    for pane in SETTINGS_PANES {
        let visits = match pane {
            SettingsPane::Tab(panes::Tab::Tuning) => 8,
            SettingsPane::Page(DisplayPage::Colors) => 2,
            SettingsPane::Page(DisplayPage::Lattice) => 25,
            SettingsPane::Page(DisplayPage::Analyzer) => 11,
            SettingsPane::Page(DisplayPage::Lighting) => 21,
            SettingsPane::Page(DisplayPage::System) => 2,
            SettingsPane::Tab(panes::Tab::Video | panes::Tab::Console | panes::Tab::Notes) => 0,
            _ => panic!("add the new settings page's range scenario"),
        };
        cases.push(Scenario { pane, visits, ..base });
        // Enabled/disabled sections still draw their bars: labels, fringe,
        // marks, audio reading, sevens, roll/note names, glow and shadow falloff.
        cases.push(Scenario { pane, visits, enabled: true, ..base });
    }
    for projection in [Projection::Perspective, Projection::Orthographic] {
        cases.push(Scenario {
            pane: SettingsPane::Page(DisplayPage::Lattice),
            projection,
            enabled: true,
            visits: 25,
            ..base
        });
    }
    // The collapsed Keyboard and Context groups hide three and four bars.
    // Cover both open for each comma derivation branch, including both modes.
    for (meantone, marvel) in [(false, false), (true, false), (false, true), (true, true)] {
        cases.push(Scenario { expanded: true, meantone, marvel, visits: 15, ..base });
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
        a.view.show_perf = scenario.enabled;
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
            assert_eq!(saw("Fifth") && saw("Half-life"), scenario.expanded);
        }
        for visit in visits {
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
