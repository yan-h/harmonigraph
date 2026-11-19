//! What a saved state loads as, for the `read-plugin-state` example (#1867).
//!
//! The restore a host makes, run on a fresh instance's parameters and
//! settings with no plugin around them, and read back the way the plugin
//! saves and draws it: the setup's prepare, the parameter table's restore and
//! the commit, in the wrapper's order; the editor's settings installed as its
//! lock installs a restore; and the host's camera over them, as each editor
//! frame lays it. Nothing here parses a field itself, so a change to a saved
//! shape reaches this reading with the restore it changes.

use std::collections::BTreeMap;
use std::sync::Arc;

use harmonigraph_clap::plugin::PluginState;
use harmonigraph_clap::prelude::Table;
use harmonigraph_clap::wrapper::clap::setup::Setup;
use harmonigraph_ui::params::{ParamBackend, ParamKey};
use harmonigraph_ui::SharedState;

use crate::configuration::{HubSetup, MUSICAL_SETTINGS};
use crate::saved_fields::{LATTICE_MAPS, UI_STATE};
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
    /// Why the restore refused the state whole. The instance then keeps what
    /// it held, a fresh one's defaults here, and nothing below was read.
    pub refused: Option<&'static str>,
    /// Each host parameter as restored, by id: its value, with what the host
    /// shows for it where that is a name rather than the number.
    pub params: Vec<(&'static str, String)>,
    /// The ids the state saved that no parameter of this build has, which a
    /// restore skips.
    pub ignored: Vec<String>,
    /// Every saved field but `ui-state`, as the plugin saves it after this
    /// restore.
    pub fields: BTreeMap<String, String>,
    /// The editor's settings: `None` for a Tune.
    pub look: Option<Look>,
    /// The Console lines the restore leaves, oldest first.
    pub notes: Vec<String>,
}

/// The `ui-state` an editor nobody opened saves: the empty document, as JSON.
const EMPTY_FIELD: &str = r#""""#;

/// What the editor's settings load as.
pub enum Look {
    /// The state holds no `ui-state`, which every save of a current build
    /// writes: an older build's.
    Missing,
    /// The state holds `""`, which an editor nobody opened saves (#1757): it
    /// opens at the fresh look of whichever build loads it.
    Unopened,
    /// The editor installed the settings, whole or in part (the notes say
    /// which), with the host's camera laid over them.
    Installed(Box<SharedState>),
    /// The editor refused the settings and opens at the fresh look; the
    /// notes say why.
    Refused,
}

/// Read `state` as the plugin that saved it loads it, or `None` for a state
/// neither plugin saved. A state is Harmonigraph's when it holds one of its
/// parameters or a field only its editor saves, and a Tune's when all it
/// holds of ours is the instance settings every instance saves.
/// `editor-state` marks nothing: it is nice-plug's usual name for a window
/// size, which other plugins save too.
pub fn read(state: &PluginState) -> Option<Reading> {
    let ours = |id: &str| Row::ALL.iter().any(|row| row.id() == id);
    let hub = state.params.keys().any(|id| ours(id))
        || [UI_STATE, LATTICE_MAPS, MUSICAL_SETTINGS]
            .iter()
            .any(|key| state.fields.contains_key(*key));
    if hub {
        Some(read_hub(state))
    } else if state.fields.contains_key(INSTANCE_FIELD) {
        Some(read_tune(state))
    } else {
        None
    }
}

fn read_hub(state: &PluginState) -> Reading {
    let params = Arc::new(HarmonigraphParams::default());
    let setup = HubSetup { instance: Adapter(Shared::hub()), params: params.clone() };
    let mut reading = restore(Plugin::Harmonigraph, &setup, &params.table, state);
    if reading.refused.is_some() {
        return reading;
    }
    while let Some(line) = params.settings.take_notice() {
        reading.notes.push(line);
    }
    // With no editor attached, the setup saves the document the restore
    // delivered rather than what the editor installs of it, which is the
    // look below.
    reading.fields.remove(UI_STATE);
    reading.look = Some(match state.fields.get(UI_STATE) {
        None => Look::Missing,
        Some(field) => {
            // What the restore delivered, for the editor lock to install at
            // its next holder.
            let document = params.ui_state.inbox().fallback();
            if document.is_empty() {
                // A field that did not parse delivers nothing either, and the
                // notes name it.
                if field == EMPTY_FIELD {
                    Look::Unopened
                } else {
                    Look::Refused
                }
            } else {
                let mut editor = SharedState::new(crate::editor::ASSUMED_SURFACE_FORMAT);
                let installed = editor.install_document(&document);
                reading.notes.extend(editor.picture.runtime.console.lines().map(str::to_owned));
                if installed {
                    // As each editor frame lays the host's camera over it.
                    editor.picture.appearance.sync_camera(&HostCamera(&params));
                    Look::Installed(Box::new(editor))
                } else {
                    Look::Refused
                }
            }
        }
    });
    reading
}

fn read_tune(state: &PluginState) -> Reading {
    // A Tune exports no host parameter.
    restore(Plugin::Tune, &Adapter(Shared::source()), &Table::empty(), state)
}

/// Prepare, restore `table` and commit, as the wrapper's restore does, then
/// read the parameters back and save the fields.
fn restore(plugin: Plugin, setup: &dyn Setup, table: &Table, state: &PluginState) -> Reading {
    let declared = |id: &str| (0..table.len()).any(|row| table.spec(row).id == id);
    let mut reading = Reading {
        plugin,
        refused: None,
        params: Vec::new(),
        ignored: state.params.keys().filter(|id| !declared(id)).cloned().collect(),
        fields: BTreeMap::new(),
        look: None,
        notes: Vec::new(),
    };
    let prepared = match setup.prepare(state) {
        Ok(prepared) => prepared,
        Err(reason) => {
            reading.refused = Some(reason);
            return reading;
        }
    };
    table.restore(&state.params);
    prepared.commit();
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
    setup.save(&mut saved);
    reading.fields = saved.fields;
    reading
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
    use harmonigraph_clap::plugin::ParamValue;

    /// A Harmonigraph state saved by a real project in November 2026, or a
    /// Tune's saved beside one before #1795, which still holds the retired
    /// `tuning_delay` parameter.
    fn fixture(name: &str) -> PluginState {
        let path = format!("{}/fixtures/saved-state/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
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
        let mut state = fixture("harmonigraph");
        // Away from the fixture's snapshot, so a reading that skipped the
        // host camera shows the snapshot's values instead.
        for (key, value) in
            [("yaw", 1.0), ("pitch", -0.5), ("distance", 6.0), ("pan-x", 2.5), ("pan-y", -3.5)]
        {
            state.params.insert(format!("camera-{key}"), ParamValue::F32(value));
        }
        let reading = read(&state).unwrap();
        let appearance = &installed(&reading).picture.appearance;
        let camera = &appearance.camera;
        assert_eq!((camera.yaw, camera.pitch, camera.distance), (1.0, -0.5, 6.0));
        // Absolute pan is a whole lattice center and a residual target, its
        // halves rounded away from zero.
        assert_eq!((appearance.view.center_fives, appearance.view.center_threes), (3, -4));
        assert_eq!((camera.target.x, camera.target.y, camera.target.z), (-0.5, 0.5, 0.0));
        // What the host does not own stays the saved look's.
        assert_eq!(camera.cabinet_scale, 0.6);
    }

    /// Every field the plugin saves is read, the two the Python reader
    /// missed among them, and a parameter this build no longer has is named
    /// rather than shown as a value the plugin holds.
    #[test]
    fn every_saved_field_is_read_and_retired_parameters_are_named() {
        let mut state = fixture("harmonigraph");
        state.params.insert("tuning-three".into(), ParamValue::I32(7));
        let reading = read(&state).unwrap();
        assert_eq!(reading.plugin, Plugin::Harmonigraph);
        assert_eq!(
            reading.fields.keys().map(String::as_str).collect::<Vec<_>>(),
            ["editor-state", "lattice-maps", "musical-settings", "tuning-instance"],
        );
        assert!(reading.fields["editor-state"].contains("[1147,687]"));
        assert!(reading.fields["lattice-maps"].contains("C · 3×4"));
        assert_eq!(reading.ignored, ["tuning-three"]);
        assert!(reading.notes.iter().any(|note| note.contains("tuning-three")));
        assert!(!reading.params.iter().any(|(id, ..)| *id == "tuning-three"));
        installed(&reading);
    }

    /// A Tune reads as one, with the Tuning delay its restore gives it.
    #[test]
    fn a_tune_reads_as_a_tune() {
        let reading = read(&fixture("tune")).unwrap();
        assert_eq!(reading.plugin, Plugin::Tune);
        assert!(reading.look.is_none());
        assert_eq!(reading.ignored, ["tuning_delay"]);
        assert_eq!(
            reading.fields[INSTANCE_FIELD],
            r#"{"retune":true,"show":true,"name":"","delay":1}"#
        );
    }
}
