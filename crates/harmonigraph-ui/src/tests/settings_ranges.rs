//! Loaded-state range guard, additional to settings layout/interaction tests.
//!
//! The poison is DERIVED: every float the saved appearance and preferences
//! serialize is overwritten in the blob itself, found by the same walk the
//! persistence guards use, so a new dialled float is poisoned without an edit
//! here. Only integers and empty options are poisoned by hand, since a blob
//! names neither's type, and so are the host-owned tuning policy fields in
//! `backend`, which never cross the editor's blob.
//!
//! The bars are FOUND rather than listed, and so is which input each one reads
//! (#1752): every input path is observed. Each numeric path of the saved blob
//! is nudged alone (a float to the edges below, an integer by one) and every
//! scenario drawn through the real load door; the tuning axes and adaptive
//! policy fields, plugin settings the blob does not carry, are nudged the same
//! way through the test backend.
//! Three claims follow, and none needs a list of what a page draws:
//!
//! - every path moves some bar's raw value or changes which bars draw, unless
//!   [`EXEMPT`] names it with its reason;
//! - every bar some scenario draws is moved by some path;
//! - every bar found fits its own range after a poisoned load, and sits at its
//!   own reset target on a fresh one.
//!
//! A conditional bar no scenario reaches leaves its path unseen, which is what
//! a hand-kept inventory used to catch by failing on every intended change
//! (#1714). Disabled bars are recorded; hidden ones need a scenario that draws
//! them. The camera's host parameters reach the pages through the appearance's
//! camera, whose blob paths are nudged like any other.
//! Gradients, StackBar, LayerStrip, choices, text fields, RangeBar
//! ordering/minimum spans and gestures are outside this guard, as are renderer
//! clamps and semantic unit mappings.

use super::harness::{materials, projections, set_optional_sections, textures, SETTINGS_PANES};
use super::persist::{is_float, nested_pairs, nested_scalars};
use super::probe::{fresh, themed};
use crate::widgets::range_probe::{collect, Visit};
use crate::*;
use harmonigraph_core::configuration::{ConfigMutation, ConfigReducer, PolicyConfig, TuningModes};
use harmonigraph_scene::{Projection, SEVENS_LAYER_LIMIT};
use std::collections::{BTreeMap, BTreeSet};

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
    /// Every float not a number: each must come back on its bar, whatever
    /// value its repair picks. The integers stay fresh, since no integer is
    /// one.
    NaN,
}

/// The saved blob of a fresh state with every dialled value poisoned to the
/// edge: each float under `appearance` and `prefs` bar the
/// [`Held::Unpoisoned`] ones, including the reach/fade pairs whose bar displays a subtraction and
/// the owners that have no bar at all. Values are written BEFORE the load door,
/// never clamped by a fixture beside it.
///
/// Floats are found in the serialized blob rather than listed: a hand list
/// here was edited every time a dial was added, and each time one was missed
/// its bar passed this guard vacuously at the fresh value (#888, #909, #913,
/// #918, #928 and #933 all walked past it). The walk is [`nested_scalars`],
/// the persistence guards' own, and not the code that draws the bars, so a
/// field no bar shows is poisoned all the same.
fn poisoned(edge: Edge) -> String {
    let mut saved = fresh();
    let (v, n) = match edge {
        Edge::Low => (-1.0e6f32, Some(i32::MIN)),
        Edge::High => (1.0e6, Some(i32::MAX)),
        Edge::NaN => (f32::NAN, None),
        Edge::Fresh => return saved.save_persist(),
    };
    // What a blob cannot say is a float: an integer, whose type sets the edge
    // it can hold, and an option still empty, which holds no number to find.
    let a = &mut saved.picture.appearance;
    for source in [
        &mut a.view.intensity.velocity,
        &mut a.view.intensity.gain,
        &mut a.view.intensity.pressure,
        &mut a.view.intensity.timbre,
    ] {
        for target in harmonigraph_scene::IntensityTarget::ALL {
            *source.weight_mut(target) = Some(v);
        }
    }
    if let Some(n) = n {
        a.view.min_sevens = n;
        a.view.max_sevens = n;
        a.view.center_sevens = n;
        a.view.octave_count = if matches!(edge, Edge::Low) { 0 } else { u32::MAX };
    }
    let mut blob = saved.save_persist();
    let mut floats: Vec<_> = nested_scalars(&blob)
        .into_iter()
        .filter(|(path, range)| {
            dialled(path) && !unpoisoned(path) && is_float(&blob[range.clone()])
        })
        .map(|(_, range)| range)
        .collect();
    assert!(!floats.is_empty(), "the walk found no float in the saved blob");
    // Back to front, so each replacement leaves the ranges before it valid.
    floats.sort_by_key(|range| std::cmp::Reverse(range.start));
    let text = format!("{v:.1}");
    for range in floats {
        blob.replace_range(range, &text);
    }
    blob
}

/// The poisoned state through its real shared load boundary, with the owners
/// that have no recorded bar checked on the way out.
fn loaded(edge: Edge) -> SharedState {
    let serialized = poisoned(edge);
    let mut state = fresh();
    assert!(state.install_document(&serialized));
    // Plugin and standalone editor shells share this load door. Check the
    // complete view invariant, including fields that have no value bar.
    let mut normalized = state.picture.appearance.view.clone();
    normalized.sanitize();
    assert_eq!(state.picture.appearance.view, normalized);
    // Not a number anywhere in what the next save stores, the floats no bar
    // reads included.
    let saved = state.save_persist();
    let left: Vec<_> = nested_scalars(&saved)
        .into_iter()
        .filter(|(_, at)| ["NaN", "inf", "-inf"].contains(&&saved[at.clone()]))
        .map(|(path, _)| path)
        .collect();
    assert!(left.is_empty(), "{edge:?}: the load left {left:?} non-finite");
    // The floats no bar shows, held to the range [`EXEMPT`] gives each.
    for (path, range) in nested_scalars(&saved) {
        if let Some((_, Held::Range(low, high), _)) = exemption(&path) {
            // In the f32 every one of these is stored in, so an edge reads as
            // the edge it was clamped to.
            let value: f32 = saved[range].parse().expect("an exempt range holds a number");
            let (low, high) = (*low as f32, *high as f32);
            assert!((low..=high).contains(&value), "{edge:?}: {path} loaded at {value}");
        }
    }
    // The two the ranges cannot say: a choice spelled as a float, which must
    // come back as one of the settings actually offered rather than merely
    // inside their span, and a direction held to the unit disc.
    let spectrum = &state.picture.appearance.spectrum;
    assert!(
        TILT_STEPS.contains(&spectrum.tilt),
        "a poisoned tilt loaded at {}, which is no step the control offers",
        spectrum.tilt,
    );
    assert!(state.workspace.interaction.prefs.spiral.look.length() <= 1.0);
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
    let (_, appearance) = nested_pairs(&serialized)
        .into_iter()
        .find(|(path, _)| path == "appearance")
        .expect("the blob holds an appearance");
    let appearance = &serialized[appearance.start + "appearance:".len()..appearance.end];
    let exported = AppearanceDocument::parse(appearance).unwrap();
    assert_eq!(state.picture.appearance.serialize(), exported.serialize());
    state
}

/// The host's side of a draw: its camera parameters, and the configuration
/// the tuning axes and the adaptive policy bars read.
struct Backend {
    config: params::ConfigurationView,
    host: [f32; params::ParamKey::ALL.len()],
}
impl ParamBackend for Backend {
    // Host-owned automatable params are not in UiPersist or normalized by UI.
    // Valid host values here; this does not test the host's restore code.
    fn get(&self, key: params::ParamKey) -> f32 {
        self.host[key_index(key)]
    }
    fn set(&self, _: params::ParamKey, _: f32) {
        panic!("drawing wrote a host parameter");
    }
    fn configuration(&self) -> Option<params::ConfigurationView> {
        Some(self.config)
    }
    // Adaptive, so the pages draw the adaptive policy's bars.
    fn lattice_maps(&self) -> Option<crate::lattice_maps::MapView> {
        Some(super::harness::engine_view(harmonigraph_core::lattice_map::TuningEngine::Adaptive))
    }
}

fn key_index(key: params::ParamKey) -> usize {
    params::ParamKey::ALL.iter().position(|&k| k == key).unwrap()
}

/// Each adaptive policy field the host owns, and how it is pushed to its low
/// or high edge: all at once for the poison, one at a time for the paths.
type PolicyEdge = (&'static str, fn(&mut PolicyConfig, bool));
const POLICY: &[PolicyEdge] = &[
    ("pitch_flexibility", |p, high| p.pitch_flexibility = if high { u16::MAX } else { 0 }),
    ("half_life_ms", |p, high| p.half_life_ms = if high { u16::MAX } else { 0 }),
    ("register", |p, high| p.register = if high { u16::MAX } else { 0 }),
    ("radius", |p, high| p.radius = if high { u8::MAX } else { 0 }),
    ("tolerance", |p, high| p.tolerance = if high { u32::MAX } else { 0 }),
    ("silence_ms", |p, high| p.silence_ms = if high { u32::MAX } else { 0 }),
    ("keyboard[0]", |p, high| p.keyboard[0] = if high { i32::MAX } else { i32::MIN }),
    ("keyboard[1]", |p, high| p.keyboard[1] = if high { i32::MAX } else { i32::MIN }),
    ("keyboard[2]", |p, high| p.keyboard[2] = if high { i32::MAX } else { i32::MIN }),
    ("decoded_fifth", |p, high| p.decoded_fifth = if high { i32::MAX } else { i32::MIN }),
];

/// What a host holds before anything is nudged: every parameter at its default.
fn host_defaults() -> [f32; params::ParamKey::ALL.len()] {
    params::ParamKey::ALL.map(|key| key.default_value())
}

/// A backend whose configuration holds the raw tuning `raw`.
fn backend_with(
    policy: PolicyConfig,
    raw: harmonigraph_core::Tuning,
    meantone: bool,
    marvel: bool,
) -> Backend {
    let host = host_defaults();
    let mut reducer = ConfigReducer::default();
    let modes = TuningModes {
        tempered: harmonigraph_core::Tempered { syntonic: meantone, septimal_kleisma: marvel },
        learning: false,
    };
    reducer.apply(ConfigMutation::Restore { raw, modes, policy });
    Backend {
        config: params::ConfigurationView {
            board: false,
            resolved: reducer.resolved(),
            input_lost: false,
            unadopted: None,
        },
        host,
    }
}

fn backend(edge: Edge, meantone: bool, marvel: bool) -> Backend {
    let mut policy = PolicyConfig::default();
    // `Fresh` leaves the host's own defaults standing, for the reason `poison`
    // leaves the appearance standing: what it asks is whether a first-ever
    // load reads out values its bars can show. `NaN` does too, since every
    // policy field is an integer.
    if matches!(edge, Edge::Low | Edge::High) {
        for (_, push) in POLICY {
            push(&mut policy, edge == Edge::High);
        }
    }
    backend_with(policy, harmonigraph_core::Tuning::default(), meantone, marvel)
}

#[derive(Clone, Copy, Debug)]
struct Scenario {
    pane: panes::Tab,
    expanded: bool,
    projection: Projection,
    enabled: bool,
    meantone: bool,
    marvel: bool,
    /// Which texture's dials the Spectrogram page draws. Each texture has its
    /// own bars, so the page has one inventory per texture, and the base
    /// scenario draws the wash's.
    style: harmonigraph_scene::CloudStyle,
    material: harmonigraph_scene::LatticeMaterial,
}

fn scenarios() -> Vec<Scenario> {
    // The fresh modes, Cabinet named; every other one is a scenario of its
    // own below, read off the editor's option tables, so a mode added to a
    // choice row is drawn here too.
    let base = Scenario {
        pane: SETTINGS_PANES[0],
        expanded: false,
        projection: Projection::Cabinet,
        enabled: false,
        meantone: false,
        marvel: false,
        style: crate::panes::fresh_spectrum().atmosphere.cloud_style,
        material: harmonigraph_scene::LatticeMaterial::default(),
    };
    let mut cases = Vec::new();
    for &pane in SETTINGS_PANES {
        cases.push(Scenario { pane, ..base });
        // Draw conditional audio, mapping, spectrogram and ribbon bars.
        cases.push(Scenario { pane, enabled: true, ..base });
    }
    // Each lattice material's own dials (amount, speed and direction beside
    // the geometry it shares with the spectrogram), and each texture's, with
    // every fold open for the Stars layer editor.
    for material in materials().filter(|&material| material != base.material) {
        let pane = panes::Tab::LatticeSettings;
        cases.push(Scenario { pane, material, expanded: true, enabled: true, ..base });
    }
    for style in textures().filter(|&style| style != base.style) {
        let pane = panes::Tab::AnalyzerSettings;
        cases.push(Scenario { pane, style, expanded: true, enabled: true, ..base });
    }
    for projection in projections().filter(|&projection| projection != base.projection) {
        let pane = panes::Tab::LatticeSettings;
        cases.push(Scenario { pane, projection, enabled: true, ..base });
    }
    // Cover each comma derivation branch, including both modes, with every
    // fold open.
    for (meantone, marvel) in [(false, false), (true, false), (false, true), (true, true)] {
        cases.push(Scenario { expanded: true, meantone, marvel, ..base });
    }
    cases
}

/// The bars `scenario` draws from a loaded state, as they read on its first
/// frame.
fn draw(mut state: SharedState, scenario: Scenario, backend: &Backend) -> Vec<Visit> {
    state.picture.runtime.observe_configuration(backend);
    assert_eq!(state.picture.runtime.tempered.syntonic, scenario.meantone);
    assert_eq!(state.picture.runtime.tempered.septimal_kleisma, scenario.marvel);
    set_optional_sections(&mut state, scenario.enabled);
    // Every section open, though a fresh editor folds the lattice-board's.
    state.workspace.interaction.prefs.folded_sections.clear();
    let a = &mut state.picture.appearance;
    a.camera.projection = scenario.projection;
    if scenario.pane == panes::Tab::Mappings {
        for source in [
            &mut a.view.intensity.velocity,
            &mut a.view.intensity.gain,
            &mut a.view.intensity.pressure,
            &mut a.view.intensity.timbre,
        ] {
            for target in harmonigraph_scene::IntensityTarget::ALL {
                let weight = source.weight_mut(target);
                *weight = scenario.enabled.then_some(weight.unwrap_or(1.0));
            }
        }
    }
    a.spectrum.atmosphere.cloud_style = scenario.style;
    a.view.atmosphere.material_style = scenario.material;
    let mut tab = scenario.pane;
    let ctx = themed();
    // Observe the first load frame once, before any discarded egui pass
    // could mutate a value and make a later pass look normalized.
    ctx.options_mut(|options| options.max_passes = std::num::NonZeroUsize::new(1).unwrap());
    ctx.memory_mut(|memory| memory.set_everything_is_visible(scenario.expanded));
    collect(|| {
        let _ = crate::tests::probe::run(
            &ctx,
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
                    params: backend,
                    now: 0.0,
                };
                viewer.ui(ui, &mut tab);
            },
        );
    })
}

fn check(edge: Edge) {
    let mut violations = Vec::new();
    for scenario in scenarios() {
        let backend = backend(edge, scenario.meantone, scenario.marvel);
        let visits = draw(loaded(edge), scenario, &backend);
        for visit in visits {
            for value in &visit.values {
                if !visit.range.contains(value) {
                    violations.push(format!("{edge:?} {scenario:?}: {visit:?}"));
                }
            }
            // A double-click puts back what a fresh state opens on, so on the
            // fresh state every value or range bar already sits at its own
            // target. A bar reading anything else resets to some other
            // setting's number. The one exempt is a switch
            // `set_optional_sections` flips, which moves it off fresh on
            // purpose. Star size has no reset by design: its pair is read off
            // the layers (`widgets/profile.rs`) and has no fresh value of its
            // own.
            let switched = visit.label == "Pattern contrast";
            let derived = visit.label == "Star size";
            if edge == Edge::Fresh {
                match &visit.default {
                    Some(None) if !derived => {
                        violations.push(format!("{scenario:?}: no reset: {visit:?}"))
                    }
                    Some(Some(default))
                        if !switched
                            && default
                                .iter()
                                .zip(&visit.values)
                                .any(|(default, value)| (default - value).abs() > 1e-4) =>
                    {
                        violations.push(format!("{scenario:?}: resets elsewhere: {visit:?}"))
                    }
                    _ => {}
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
#[test]
fn loaded_settings_nan_fit_real_bars() {
    check(Edge::NaN);
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

/// Walk a fresh value against its poisoned copy as RON values, through
/// every struct, sequence, `Some` and enum payload, collecting the path of each
/// float the poison left as it was. An option still empty after the poison
/// counts as left as it was, since what it would hold is unclassified; a value
/// whose shape the poison changed fails, naming its path, rather than being
/// skipped with everything under it.
fn unpoisoned_floats(
    path: &str,
    opened: &ron::Value,
    poisoned: &ron::Value,
    dialled: &mut usize,
    missed: &mut Vec<String>,
) {
    use ron::{value::Number, Value};
    match (opened, poisoned) {
        (Value::Map(opened), Value::Map(poisoned)) => {
            for (key, value) in opened.iter() {
                let Value::String(name) = key else { panic!("{path} has an unnamed key") };
                let path = if path.is_empty() { name.clone() } else { format!("{path}.{name}") };
                let again =
                    poisoned.get(key).unwrap_or_else(|| panic!("{path} left the poisoned copy"));
                unpoisoned_floats(&path, value, again, dialled, missed);
            }
        }
        (Value::Seq(opened), Value::Seq(poisoned)) => {
            assert_eq!(
                opened.len(),
                poisoned.len(),
                "two serializations of {path} differ in length"
            );
            for (index, (value, again)) in opened.iter().zip(poisoned).enumerate() {
                unpoisoned_floats(&format!("{path}[{index}]"), value, again, dialled, missed);
            }
        }
        (Value::Option(Some(value)), Value::Option(Some(again))) => {
            unpoisoned_floats(path, value, again, dialled, missed);
        }
        // An option the poison filled is dialled; one it left empty holds a
        // float nobody classified, unless it is listed.
        (Value::Option(None), Value::Option(Some(_))) => *dialled += 1,
        (Value::Option(None), Value::Option(None)) => missed.push(path.to_string()),
        (Value::Number(Number::F32(_) | Number::F64(_)), Value::Number(_)) => {
            *dialled += 1;
            if opened == poisoned {
                missed.push(path.to_string());
            }
        }
        (Value::Option(Some(_)), Value::Option(None)) => {
            panic!("the poison emptied {path}, so nothing under it is checked")
        }
        _ if std::mem::discriminant(opened) == std::mem::discriminant(poisoned) => {}
        _ => panic!("{path} changed shape under the poison: {opened:?} became {poisoned:?}"),
    }
}

/// What holds an input path no bar reads, in place of the bar.
#[derive(Debug)]
enum Held {
    /// Left at its fresh value by [`poisoned`]: no range to hold it to.
    Unpoisoned,
    /// No bar; a poisoned load must land inside this range ([`loaded`]).
    Range(f64, f64),
    /// No bar this guard records, and whatever does hold it is named in the
    /// reason.
    Elsewhere,
}

/// The input paths no bar reads, grouped by owner, each group under one
/// reason. An entry names a path or a prefix of paths (`a.b` covers `a.b.c`
/// and `a.b[2]`). A new path beside them is not covered by its neighbours:
/// unless it falls under a prefix, it is nudged and must move a bar.
const EXEMPT: &[(&[&str], Held, &str)] = &[
    (
        &["appearance.camera.target"],
        Held::Unpoisoned,
        "where the orbit looks, set by panning and framing: no bar and no range, so \
         its load repair only refuses a non-finite one",
    ),
    (
        &["prefs.perf_pos"],
        Held::Unpoisoned,
        "where the performance overlay sits, set by dragging it and empty until then: \
         no bar and no range, so its load repair only drops a non-finite one",
    ),
    (
        &["appearance.view.pitch_gradient", "appearance.spectrum.spectrogram_gradient"],
        Held::Unpoisoned,
        "gradients are outside this guard (module header)",
    ),
    (
        &["appearance.camera.distance"],
        Held::Range(
            harmonigraph_scene::Camera::MIN_DISTANCE as f64,
            harmonigraph_scene::Camera::MAX_DISTANCE as f64,
        ),
        "the zoom: set by the wheel and pinch, which `Camera::sanitize` and \
         `zoom_by` hold to one range",
    ),
    (
        &["appearance.spectrum.roll_fraction"],
        Held::Range(0.0, 1.0),
        "the Analyzer's divider, set by dragging it",
    ),
    (
        &["appearance.render.stop_bar"],
        Held::Range(STOP_BAR_RANGE.0, STOP_BAR_RANGE.1),
        "a drag value rather than a bar: its range runs to 100,000",
    ),
    (
        &["appearance.render.frame.split"],
        Held::Range(RenderFrame::SPLIT_MIN as f64, RenderFrame::SPLIT_MAX as f64),
        "the video frame's lattice/spectral split, set by the Video page's divider",
    ),
    (
        &["prefs.spiral.zoom"],
        Held::Range(crate::panes::spiral::ZOOM.0 as f64, crate::panes::spiral::ZOOM.1 as f64),
        "the Spiral's framing, set on the picture itself",
    ),
    (
        &["appearance.spectrum.tilt", "prefs.spiral.look"],
        Held::Elsewhere,
        "a choice spelled as a float and a direction: `loaded` checks each against \
         its own steps and the unit disc",
    ),
    (
        &[
            "appearance.view.center_threes",
            "appearance.view.center_fives",
            "appearance.view.min_sevens",
            "appearance.view.max_sevens",
            "appearance.view.center_sevens",
        ],
        Held::Elsewhere,
        "the drawn window and the sevens LayerStrip: integers no bar shows, the \
         strip's invariant `loaded`'s own check",
    ),
    (
        &[
            "appearance.view.band_width",
            "appearance.view.ring_inner",
            "appearance.view.spectral_ring_width",
            "appearance.view.mark_thickness",
        ],
        Held::Elsewhere,
        "the layer widths: the Layers StackBar, outside this guard",
    ),
    (
        &[
            "appearance.view.atmosphere.stars.star_spacing_ratio",
            "appearance.view.atmosphere.stars.star_speed",
            "appearance.view.atmosphere.stars.star_solid",
            "appearance.spectrum.atmosphere.stars.star_spacing_ratio",
            "appearance.spectrum.atmosphere.stars.star_speed",
            "appearance.spectrum.atmosphere.stars.star_solid",
            "appearance.view.atmosphere.stars.star_size[4]",
        ],
        Held::Elsewhere,
        "the Stars editor's Per layer plot shows one property at a time, chosen in \
         egui's memory, and opens on Size; and a layer past the lattice's four fresh \
         layers has no handle",
    ),
    (
        &[
            "appearance.render.frame.aspect_w",
            "appearance.render.frame.aspect_h",
            "appearance.render.short_edge",
        ],
        Held::Elsewhere,
        "the Video page's Aspect and Output size choices",
    ),
    (
        &["appearance.version"],
        Held::Elsewhere,
        "the document's version: any other value is refused at the door",
    ),
];

/// Whether `entry` names `path`: the path itself, or a prefix of it at a field
/// or an index.
fn covers(entry: &str, path: &str) -> bool {
    path.strip_prefix(entry).is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '[']))
}

fn exemption(path: &str) -> Option<&'static (&'static [&'static str], Held, &'static str)> {
    EXEMPT.iter().find(|(entries, _, _)| entries.iter().any(|entry| covers(entry, path)))
}

fn unpoisoned(path: &str) -> bool {
    matches!(exemption(path), Some((_, Held::Unpoisoned, _)))
}

/// The two owners of the editor blob whose values the settings pages show.
fn dialled(path: &str) -> bool {
    path.starts_with("appearance.") || path.starts_with("prefs.")
}

/// One input a bar may read, as a name and the copy of the fresh inputs with
/// that one nudged to its low or high edge: the blob for a saved path, the
/// backend for a host's.
struct Input {
    name: String,
    nudge: Box<dyn Fn(bool, Scenario) -> (String, Backend) + Sync>,
}

/// Every input path: the blob's numeric scalars under [`dialled`] owners
/// bar the [`Held::Unpoisoned`] ones, the tuning axes and the adaptive policy
/// fields.
fn inputs(blob: &str) -> Vec<Input> {
    let mut inputs = Vec::new();
    for (path, range) in nested_scalars(blob) {
        let text = &blob[range.clone()];
        if !dialled(&path) || unpoisoned(&path) || text.parse::<f64>().is_err() {
            continue;
        }
        // The #1714 edges for a float; one step either way for an integer,
        // whose type the text does not say and whose edge it does not need.
        let (low, high) = if is_float(text) {
            ("-1000000.0".to_owned(), "1000000.0".to_owned())
        } else {
            let n: i64 = text.parse().unwrap();
            ((n - 1).to_string(), (n + 1).to_string())
        };
        let blob = blob.to_owned();
        inputs.push(Input {
            name: path,
            nudge: Box::new(move |high_edge, scenario| {
                let mut nudged = blob.clone();
                nudged.replace_range(range.clone(), if high_edge { &high } else { &low });
                (nudged, backend(Edge::Fresh, scenario.meantone, scenario.marvel))
            }),
        });
    }
    for axis in harmonigraph_core::configuration::TuningAxis::ALL {
        let blob = blob.to_owned();
        inputs.push(Input {
            name: format!("tuning.{axis:?}"),
            nudge: Box::new(move |high, scenario| {
                let mut axes = harmonigraph_core::Tuning::default().axes();
                let range = axis.range();
                let edge = if high { *range.end() } else { *range.start() };
                axes[axis.index()] = harmonigraph_core::tuning::microcents(edge);
                let raw = harmonigraph_core::Tuning::from_axes(axes);
                let backend =
                    backend_with(PolicyConfig::default(), raw, scenario.meantone, scenario.marvel);
                (blob.clone(), backend)
            }),
        });
    }
    for &(field, push) in POLICY {
        let blob = blob.to_owned();
        inputs.push(Input {
            name: format!("host.policy.{field}"),
            nudge: Box::new(move |high, scenario| {
                let mut policy = PolicyConfig::default();
                push(&mut policy, high);
                let backend =
                    backend_with(policy, Default::default(), scenario.meantone, scenario.marvel);
                (blob.clone(), backend)
            }),
        });
    }
    inputs
}

/// Each bar a draw recorded, by page, label and which of that label's bars on
/// the page it is, with the raw values it read.
fn bars(scenario: Scenario, visits: &[Visit]) -> BTreeMap<String, Vec<f32>> {
    let mut seen = BTreeMap::<&str, usize>::new();
    visits
        .iter()
        .map(|visit| {
            let n = seen.entry(&visit.label).or_default();
            *n += 1;
            (format!("{:?}: {} #{n}", scenario.pane, visit.label), visit.values.clone())
        })
        .collect()
}

/// The bars `scenario` draws from `blob` under `backend`, or `None` when the
/// door refuses the blob.
fn bars_from(
    blob: &str,
    scenario: Scenario,
    backend: &Backend,
) -> Option<BTreeMap<String, Vec<f32>>> {
    let mut state = fresh();
    state.install_document(blob).then(|| bars(scenario, &draw(state, scenario, backend)))
}

/// Every input path reaches some bar, and every bar is reached by some input
/// path: the map is found by nudging each input alone and drawing every
/// scenario (module header).
///
/// The fresh state's intensity weights are filled first, so that the Mappings
/// page's weight bars read blob values rather than the 1.0 its enabled
/// scenario fills an empty one with.
#[test]
fn every_input_path_moves_a_bar_and_every_bar_is_moved() {
    let started = std::time::Instant::now();
    let mut state = fresh();
    let view = &mut state.picture.appearance.view;
    for source in [
        &mut view.intensity.velocity,
        &mut view.intensity.gain,
        &mut view.intensity.pressure,
        &mut view.intensity.timbre,
    ] {
        for target in harmonigraph_scene::IntensityTarget::ALL {
            *source.weight_mut(target) = Some(1.0);
        }
    }
    let blob = state.save_persist();
    let scenarios = scenarios();
    let base: Vec<_> = scenarios
        .iter()
        .map(|&s| bars_from(&blob, s, &backend(Edge::Fresh, s.meantone, s.marvel)).unwrap())
        .collect();
    let inputs = inputs(&blob);
    let draws = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    // Each input on its own, one thread per share of them: a draw is a whole
    // settings page through the load door, and there are a few thousand.
    let observed: Vec<(Vec<String>, BTreeSet<String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = inputs
            .chunks(inputs.len().div_ceil(threads))
            .map(|share| {
                let (scenarios, base, draws) = (&scenarios, &base, &draws);
                scope.spawn(move || {
                    let (mut seen, mut moved) = (Vec::new(), BTreeSet::new());
                    for input in share {
                        for high in [true, false] {
                            let mut reached = false;
                            for (s, &scenario) in scenarios.iter().enumerate() {
                                let (blob, backend) = (input.nudge)(high, scenario);
                                draws.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                let Some(nudged) = bars_from(&blob, scenario, &backend) else {
                                    break;
                                };
                                // A bar drawn or hidden by the nudge counts as
                                // the path being read, though by no one bar.
                                reached |= nudged.keys().ne(base[s].keys());
                                for (bar, values) in &nudged {
                                    if base[s].get(bar).is_some_and(|was| was != values) {
                                        reached = true;
                                        moved.insert(bar.clone());
                                    }
                                }
                            }
                            if reached {
                                seen.push(input.name.clone());
                                break;
                            }
                        }
                    }
                    (seen, moved)
                })
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().unwrap()).collect()
    });
    let (mut seen, mut moved) = (BTreeSet::new(), BTreeSet::new());
    for (s, m) in observed {
        seen.extend(s);
        moved.extend(m);
    }
    eprintln!(
        "{} input paths, {} scenarios, {} draws on {threads} threads in {:?}",
        inputs.len(),
        scenarios.len(),
        draws.load(std::sync::atomic::Ordering::Relaxed),
        started.elapsed(),
    );

    let names: Vec<&str> = inputs.iter().map(|input| input.name.as_str()).collect();
    let unseen: Vec<_> =
        names.iter().filter(|name| !seen.contains(**name) && exemption(name).is_none()).collect();
    assert!(unseen.is_empty(), "no bar reads these input paths: {unseen:#?}");
    // An entry no unseen path answers to: gone from the blob, or read by a bar
    // now, which makes it a bar's to hold.
    let stale: Vec<_> = EXEMPT
        .iter()
        .filter(|(_, held, _)| !matches!(held, Held::Unpoisoned))
        .flat_map(|(entries, _, _)| entries.iter())
        .filter(|entry| !names.iter().any(|name| covers(entry, name) && !seen.contains(*name)))
        .collect();
    assert!(stale.is_empty(), "EXEMPT names paths a bar reads or that are gone: {stale:?}");
    let unmoved: Vec<_> =
        base.iter().flat_map(|bars| bars.keys()).filter(|bar| !moved.contains(*bar)).collect();
    assert!(unmoved.is_empty(), "no input path moves these bars: {unmoved:#?}");
}

/// Every float the fresh appearance and preferences hold is poisoned, unless
/// [`EXEMPT`] holds it [`Held::Unpoisoned`]. This is fixture completeness, not discovery of
/// controls: the conditional scenarios, real bar probes and visit assertions
/// above still establish UI coverage.
///
/// The poison finds floats by walking the blob's text; this reads both blobs
/// through RON's own parser instead, so a float the text walk does not
/// recognize, or an empty option nobody filled by hand, fails here.
#[test]
fn the_loaded_state_guard_poisons_every_dialled_view_float() {
    let value = |text: &str| ron::from_str::<ron::Value>(text).expect("a persist blob is RON");
    let (before, after) = (value(&fresh().save_persist()), value(&poisoned(Edge::High)));
    let (ron::Value::Map(before), ron::Value::Map(after)) = (before, after) else {
        panic!("a persist blob is a struct");
    };
    let (mut dialled, mut missed) = (0, Vec::new());
    for owner in ["appearance", "prefs"] {
        let key = ron::Value::String(owner.to_owned());
        let (old, new) = (before.get(&key), after.get(&key));
        let (Some(old), Some(new)) = (old, new) else { panic!("the blob has no {owner}") };
        unpoisoned_floats(owner, old, new, &mut dialled, &mut missed);
    }
    // Detect the blob going wholly unrecognized if RON's value model changes.
    assert!(dialled > 0, "none of the blob's fields read as floats");
    let stale: Vec<_> = EXEMPT
        .iter()
        .filter(|(_, held, _)| matches!(held, Held::Unpoisoned))
        .flat_map(|(entries, _, _)| entries.iter())
        .filter(|entry| !missed.iter().any(|path| covers(entry, path)))
        .collect();
    missed.retain(|path| !unpoisoned(path));
    assert!(missed.is_empty(), "the loaded-state guard never poisons {missed:?}");
    assert!(
        stale.is_empty(),
        "EXEMPT leaves unpoisoned floats the guard poisons or that are gone: {stale:?}"
    );
}
