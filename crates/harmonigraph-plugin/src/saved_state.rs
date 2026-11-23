//! What a saved state loads as, for the `read-plugin-state` tool (#1867).
//!
//! The load a host makes, run on a fresh instance's parameters and settings
//! with no plugin around them, and read back the way the plugin saves and
//! draws it: the state decoded as a load decodes it, then the shell's one
//! restore ([`setup::restore`]); the editor's settings installed from the
//! inbox as every holder of the editor lock installs them; and the host's
//! camera laid over them, as each editor frame lays it. Nothing here parses
//! a field itself, so a change to a saved shape reaches this reading with the
//! restore it changes.
//!
//! [`setup::restore`]: harmonigraph_clap::wrapper::clap::setup::restore

use std::collections::BTreeMap;
use std::sync::Arc;

use harmonigraph_clap::plugin::{Plugin as _, PluginState};
use harmonigraph_clap::prelude::Table;
use harmonigraph_clap::wrapper::clap::setup::{self, Setup};
use harmonigraph_ui::params::{ParamBackend, ParamKey};
use harmonigraph_ui::SharedState;
use serde_json::Value;

use crate::configuration::{HubSetup, Notice, MUSICAL_SETTINGS};
use crate::saved_fields::{LATTICE_MAPS, UI_STATE};
use crate::tuning::plugin::HarmonigraphTune;
use crate::tuning::setup::{Adapter, Shared, INSTANCE_FIELD};
use crate::{HarmonigraphParams, Row};

/// Which of the two plugins saved a state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plugin {
    Harmonigraph,
    /// A Harmonigraph Tune, which has no parameters and no editor.
    Tune,
}

/// One saved state, as the plugin loads it.
pub struct Reading {
    pub plugin: Plugin,
    /// Why the load refused the state whole: it does not decode. The instance
    /// then keeps what it held, a fresh one's defaults here, and nothing
    /// below was read.
    pub refused: Option<String>,
    /// Each host parameter as restored, by id: its value, with what the host
    /// shows for it where that is a name rather than the number.
    pub params: Vec<(&'static str, String)>,
    /// Every saved field but `ui-state`, as the plugin saves it after this
    /// restore.
    pub fields: BTreeMap<String, String>,
    /// The editor's settings: `None` for a Tune.
    pub look: Option<Look>,
    /// The Console lines the restore leaves, oldest first.
    pub notes: Vec<String>,
}

/// What the editor's settings load as.
pub enum Look {
    /// The state holds no `ui-state`, which every save of a current build
    /// writes: an older build's.
    Missing,
    /// The state holds `""`, which an editor nobody opened saves (#1757): it
    /// opens at the fresh look of whichever build loads it.
    Unopened,
    /// The editor installed the settings, whole or in part (the notes say
    /// which), as it opens them: the host's camera laid over them, and the
    /// Console selected with Settings unfolded where a restore line reports a
    /// loss. The raw dump (`--raw`) is the file as it holds them.
    Installed(Box<SharedState>),
    /// The editor refused the settings, or the field did not parse, and opens
    /// at the fresh look; the notes say why.
    Refused,
}

/// Which plugin saved `state`, judged by its keys alone so that a state
/// either plugin would refuse is still recognised; `None` for a state neither
/// saved. A state is Harmonigraph's when it holds one of its parameters or a
/// field only its editor saves, and a Tune's when all it holds of ours is the
/// instance settings every instance saves. `editor-state` marks nothing: it
/// is nice-plug's usual name for a window size, which other plugins save too.
pub fn plugin_of(state: &Value) -> Option<Plugin> {
    let keys = |name: &str| {
        state.get(name).and_then(Value::as_object).into_iter().flat_map(|map| map.keys())
    };
    let hub = keys("params").any(|id| Row::ALL.iter().any(|row| row.id() == id))
        || keys("fields").any(|key| [UI_STATE, LATTICE_MAPS, MUSICAL_SETTINGS].contains(&&**key));
    if hub {
        Some(Plugin::Harmonigraph)
    } else if keys("fields").any(|key| key == INSTANCE_FIELD) {
        Some(Plugin::Tune)
    } else {
        None
    }
}

/// Load the saved state `json` into a fresh instance of `plugin`, as a host
/// load does, and read what it holds then.
pub fn read(plugin: Plugin, json: &[u8]) -> Reading {
    let mut reading = Reading {
        plugin,
        refused: None,
        params: Vec::new(),
        fields: BTreeMap::new(),
        look: None,
        notes: Vec::new(),
    };
    let state = match harmonigraph_clap::wrapper::state::decode(json) {
        Ok(state) => state,
        Err(error) => {
            reading.refused = Some(format!("the state does not decode: {error}"));
            return reading;
        }
    };
    match plugin {
        Plugin::Harmonigraph => read_hub(&state, &mut reading),
        Plugin::Tune => {
            let tune = HarmonigraphTune::default();
            restore(tune.setup().as_deref(), &tune.params(), &state, &mut reading);
        }
    }
    reading
}

/// The Hub's setup is built as `Harmonigraph::setup` builds it, on a fresh
/// instance's parameters, without the input worker and recorder a whole
/// plugin starts.
fn read_hub(state: &PluginState, reading: &mut Reading) {
    let params = Arc::new(HarmonigraphParams::default());
    let setup = HubSetup { instance: Adapter(Shared::hub()), params: params.clone() };
    restore(Some(&setup), &params.table, state, reading);
    // With no editor attached, the setup saves the document the restore
    // delivered rather than what the editor installs of it, which is the
    // look below.
    reading.fields.remove(UI_STATE);
    if !state.fields.contains_key(UI_STATE) {
        while let Some(Notice::Restore(line) | Notice::Host(line)) = params.settings.next_notice() {
            reading.notes.push(line);
        }
        reading.look = Some(Look::Missing);
        return;
    }
    // As an editor frame takes them: the lock installs the document, then
    // the restore's lines arrive over it, so the look below shows the tab,
    // fold and line order the editor would.
    let mut editor = SharedState::new(crate::editor::ASSUMED_SURFACE_FORMAT);
    let delivered = params.ui_state.inbox().install_pending(&mut editor);
    params.settings.deliver_notices(&mut editor);
    reading.notes.extend(editor.picture.runtime.console.lines().map(str::to_owned));
    reading.look = Some(match delivered {
        Some((document, _)) if document.is_empty() => Look::Unopened,
        Some((_, true)) => {
            // As each editor frame lays the host's camera over it.
            editor.picture.appearance.sync_camera(&HostCamera(&params));
            Look::Installed(Box::new(editor))
        }
        // Refused on install, or not delivered: a field that did not parse,
        // which the notes name.
        Some((_, false)) | None => Look::Refused,
    });
}

/// Restore `state` through the shell's one restore, then read the
/// parameters back and save the fields.
fn restore(setup: Option<&dyn Setup>, table: &Table, state: &PluginState, reading: &mut Reading) {
    setup::restore(setup, table, state);
    reading.params = (0..table.len())
        .map(|row| {
            let spec = table.spec(row);
            // A row without steps is a float row, whose text is its value
            // again. Every other reads whole, and names what it stands for
            // where its text says more than the number.
            let value = match spec.steps() {
                None => format!("{:?}", table.float(row)),
                Some(_) => {
                    let value = table.int(row);
                    let text = spec.text(spec.to_host(value as f32));
                    if text == value.to_string() {
                        text
                    } else {
                        format!("{value} ({text})")
                    }
                }
            };
            (spec.id, value)
        })
        .collect();
    let mut saved =
        PluginState { version: String::new(), params: BTreeMap::new(), fields: BTreeMap::new() };
    if let Some(setup) = setup {
        setup.save(&mut saved);
    }
    reading.fields = saved.fields;
}

/// The restored camera rows, as the editor frame's backend reads them.
struct HostCamera<'a>(&'a HarmonigraphParams);

impl ParamBackend for HostCamera<'_> {
    fn get(&self, key: ParamKey) -> f32 {
        self.0.camera(key)
    }

    fn set(&self, _key: ParamKey, _value: f32) {}

    fn camera_value(&self, key: ParamKey) -> Option<f32> {
        Some(self.0.camera(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Harmonigraph state saved by a real project in November 2026, a
    /// Tune's saved beside one before #1795, which still holds the retired
    /// `tuning_delay` parameter, or the first with its `version` `null`.
    fn fixture(name: &str) -> Vec<u8> {
        let path = format!("{}/fixtures/saved-state/{name}.json", env!("CARGO_MANIFEST_DIR"));
        std::fs::read(path).unwrap()
    }

    /// The Harmonigraph fixture with its `ui-state` RON passed through `edit`.
    fn with_look(edit: impl FnOnce(&str) -> String) -> Vec<u8> {
        let mut state: Value = serde_json::from_slice(&fixture("harmonigraph")).unwrap();
        let field = &mut state["fields"][UI_STATE];
        let look: String = serde_json::from_str(field.as_str().unwrap()).unwrap();
        *field = Value::String(serde_json::to_string(&edit(&look)).unwrap());
        serde_json::to_vec(&state).unwrap()
    }

    fn installed(reading: &Reading) -> &SharedState {
        match &reading.look {
            Some(Look::Installed(editor)) => editor,
            _ => panic!("the saved look did not install"),
        }
    }

    /// The camera the editor draws is the host's, laid out as each frame
    /// lays it, whatever the saved look's own snapshot says: automation can
    /// move the parameters while the editor is shut.
    #[test]
    fn the_host_camera_is_laid_over_the_saved_look() {
        let look = with_look(|look| look.replace("cabinet_scale:0.6", "cabinet_scale:0.71"));
        let mut state: Value = serde_json::from_slice(&look).unwrap();
        // Away from the fixture's snapshot, so a reading that skipped the
        // host camera shows the snapshot's values instead.
        for (key, value) in
            [("yaw", 1.0), ("pitch", -0.5), ("distance", 6.0), ("pan-x", 2.5), ("pan-y", -3.5)]
        {
            state["params"][format!("camera-{key}")] = serde_json::json!({ "f32": value });
        }
        let reading = read(Plugin::Harmonigraph, &serde_json::to_vec(&state).unwrap());
        let appearance = &installed(&reading).picture.appearance;
        let camera = &appearance.camera;
        assert_eq!((camera.yaw, camera.pitch, camera.distance), (1.0, -0.5, 6.0));
        // Absolute pan is a whole lattice center and a residual target, its
        // halves rounded away from zero.
        assert_eq!((appearance.view.center_fives, appearance.view.center_threes), (3, -4));
        assert_eq!((camera.target.x, camera.target.y, camera.target.z), (-0.5, 0.5, 0.0));
        // What the host does not own stays the saved look's.
        assert_eq!(camera.cabinet_scale, 0.71);
    }

    /// Every field the plugin saves is read, the two the Python reader
    /// missed among them, and a parameter this build no longer has is named
    /// in the Console rather than shown as a value the plugin holds.
    #[test]
    fn every_saved_field_is_read_and_retired_parameters_are_named() {
        let mut state: Value = serde_json::from_slice(&fixture("harmonigraph")).unwrap();
        state["params"]["tuning-three"] = serde_json::json!({ "i32": 7 });
        let maps = &mut state["fields"][LATTICE_MAPS];
        *maps = Value::String(maps.as_str().unwrap().replace("C · 3×4", "Fixture map"));
        let reading = read(Plugin::Harmonigraph, &serde_json::to_vec(&state).unwrap());
        assert_eq!(
            reading.fields.keys().map(String::as_str).collect::<Vec<_>>(),
            ["editor-state", "lattice-maps", "musical-settings", "tuning-instance"],
        );
        assert!(reading.fields["editor-state"].contains("[1147,687]"));
        assert!(reading.fields["lattice-maps"].contains("Fixture map"));
        assert!(reading.fields[MUSICAL_SETTINGS]
            .contains(r#""axes":[0,696889465,387553406,1006222534,500000]"#));
        assert!(reading.notes.iter().any(|note| note.contains("tuning-three")));
        assert!(!reading.params.iter().any(|(id, _)| *id == "tuning-three"));
        installed(&reading);
    }

    /// A parameter value this build cannot read, such as Analyzer Audio
    /// Input's id from before it saved an index, opens that row at its
    /// default with a Console line, and the rest of the project loads.
    #[test]
    fn an_unreadable_parameter_value_opens_at_its_default_with_a_console_line() {
        let mut state: Value = serde_json::from_slice(&fixture("harmonigraph")).unwrap();
        state["params"]["analysis-input"] = serde_json::json!({ "string": "sidechain" });
        let reading = read(Plugin::Harmonigraph, &serde_json::to_vec(&state).unwrap());
        assert_eq!(reading.refused, None);
        let param = |id| &reading.params.iter().find(|(row, _)| *row == id).unwrap().1;
        assert_eq!(param("analysis-input"), "0 (Main)");
        assert_eq!(param("tuning-engine"), "2 (Lattice Map)", "the rest of the project");
        assert!(reading
            .notes
            .iter()
            .any(|note| note.contains("cannot read") && note.contains("analysis-input")));
        installed(&reading);
    }

    /// A Tune reads through its own plugin, with the Tuning delay its
    /// restore gives it.
    #[test]
    fn a_tune_reads_as_a_tune() {
        let json = fixture("tune");
        assert_eq!(plugin_of(&serde_json::from_slice(&json).unwrap()), Some(Plugin::Tune));
        let reading = read(Plugin::Tune, &json);
        assert!(reading.look.is_none() && reading.params.is_empty());
        assert_eq!(
            reading.fields[INSTANCE_FIELD],
            r#"{"retune":true,"show":true,"name":"","delay":1}"#
        );
    }

    /// An editor never opened and a look the editor refuses both open at the
    /// fresh look, and are told apart.
    #[test]
    fn an_unopened_editor_is_not_a_refused_look() {
        let unopened = read(Plugin::Harmonigraph, &with_look(|_| String::new()));
        assert!(matches!(unopened.look, Some(Look::Unopened)));
        let below_floor = with_look(|look| look.replacen("version:7", "version:6", 1));
        let refused = read(Plugin::Harmonigraph, &below_floor);
        assert!(matches!(refused.look, Some(Look::Refused)));
        assert!(refused.notes.iter().any(|note| note.contains("below the floor")));
    }

    /// A state of ours that a load refuses is still recognised as ours, so
    /// the reader reports the refusal rather than skipping it.
    #[test]
    fn a_state_that_does_not_decode_is_ours_and_refused() {
        let json = fixture("harmonigraph-rejected");
        let value = serde_json::from_slice(&json).unwrap();
        assert_eq!(plugin_of(&value), Some(Plugin::Harmonigraph));
        let reading = read(Plugin::Harmonigraph, &json);
        assert!(reading.refused.unwrap().contains("does not decode"));
    }

    /// A tuning field that does not parse opens at the default a save without
    /// it opens at, with a Console line naming it, and the rest of the
    /// project loads, its look among it, rather than the restore refusing
    /// whole and the next save writing the fresh look over it (#1927). Here
    /// `musical-settings` is an axis short, as a build with another axis
    /// count would find it, and `tuning-instance` is cut off.
    #[test]
    fn a_tuning_field_that_does_not_parse_opens_at_its_default_and_the_rest_loads() {
        let mut state: Value = serde_json::from_slice(&fixture("harmonigraph")).unwrap();
        let mut absent = state.clone();
        let fields = absent["fields"].as_object_mut().unwrap();
        fields.remove(MUSICAL_SETTINGS);
        fields.remove(INSTANCE_FIELD);
        let field = &mut state["fields"][MUSICAL_SETTINGS];
        let broken = field.as_str().unwrap().replacen(r#""axes":[0,"#, r#""axes":["#, 1);
        assert_ne!(broken, field.as_str().unwrap());
        *field = Value::String(broken);
        state["fields"][INSTANCE_FIELD] = Value::String(r#"{"retune":"#.into());
        let reading = read(Plugin::Harmonigraph, &serde_json::to_vec(&state).unwrap());
        let absent = read(Plugin::Harmonigraph, &serde_json::to_vec(&absent).unwrap());
        assert_eq!(reading.refused, None);
        for key in [MUSICAL_SETTINGS, INSTANCE_FIELD] {
            assert_eq!(reading.fields[key], absent.fields[key], "{key} at its default");
            assert!(
                reading.notes.iter().any(|note| note.contains(key) && note.contains("defaults")),
                "no Console line names {key}: {:?}",
                reading.notes
            );
        }
        assert!(reading.fields["editor-state"].contains("[1147,687]"), "the rest of the project");
        installed(&reading);
    }

    /// A project saved before the main thread owned the tuning (#1938), the
    /// fixture's, opens with the tuning it saved and saves it back unchanged,
    /// key for key: every value away from its default, Learn armed among them.
    #[test]
    fn a_saved_tuning_opens_and_saves_back_unchanged() {
        let mut state: Value = serde_json::from_slice(&fixture("harmonigraph")).unwrap();
        let field = &mut state["fields"][MUSICAL_SETTINGS];
        let mut tuning: Value = serde_json::from_str(field.as_str().unwrap()).unwrap();
        tuning["learning"] = true.into();
        tuning["marvel"] = true.into();
        let adaptive = &mut tuning["adaptive"];
        (adaptive["reset_stop"], adaptive["silence_ms"]) = (true.into(), 1500.into());
        (adaptive["decode_fifths"], adaptive["decoded_fifth"]) = (true.into(), 695_000_000.into());
        *field = Value::String(tuning.to_string());
        let reading = read(Plugin::Harmonigraph, &serde_json::to_vec(&state).unwrap());
        assert_eq!(reading.refused, None);
        let back: Value = serde_json::from_str(&reading.fields[MUSICAL_SETTINGS]).unwrap();
        assert_eq!(back, tuning);
    }

    /// A Tune's `tuning-instance` that does not parse opens at the settings a
    /// save without it opens at, rather than refusing its restore.
    #[test]
    fn a_tunes_instance_settings_that_do_not_parse_open_at_its_defaults() {
        let mut state: Value = serde_json::from_slice(&fixture("tune")).unwrap();
        let mut absent = state.clone();
        absent["fields"].as_object_mut().unwrap().remove(INSTANCE_FIELD);
        let absent = read(Plugin::Tune, &serde_json::to_vec(&absent).unwrap());
        // Readable, and other than the defaults, the test tells the two apart.
        state["fields"][INSTANCE_FIELD] = Value::String(r#"{"name":"Bass","delay":3}"#.into());
        let named = read(Plugin::Tune, &serde_json::to_vec(&state).unwrap());
        assert_ne!(named.fields[INSTANCE_FIELD], absent.fields[INSTANCE_FIELD]);
        state["fields"][INSTANCE_FIELD] = Value::String(r#"{"name":"Bass","delay":"#.into());
        let reading = read(Plugin::Tune, &serde_json::to_vec(&state).unwrap());
        assert_eq!(reading.refused, None);
        assert_eq!(reading.fields[INSTANCE_FIELD], absent.fields[INSTANCE_FIELD]);
    }
}
