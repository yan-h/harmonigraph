//! The three saved fields that belong to Harmonigraph's editor and maps
//! rather than to its tuning: the Lattice Map document, the window, and the
//! editor's look and layout. The Hub's setup saves and restores them beside
//! its musical settings and instance settings, so one owner writes every
//! field a project stores (#1814).
//!
//! A field that does not parse is skipped with a line in the log and the rest
//! of the restore goes ahead, as the fields did when nice-plug's derive
//! restored them, which skipped one in silence. A field the save lacks keeps
//! what the instance holds.

use std::sync::Arc;

use nice_plug::plugin::PluginState;
use nice_plug::prelude::nice_error;
use nice_plug::wrapper::clap::setup::Prepared;

use crate::editor::EguiState;
use crate::HarmonigraphParams;
use harmonigraph_ui::lattice_maps::MapDocument;

const LATTICE_MAPS: &str = "lattice-maps";
const EDITOR_STATE: &str = "editor-state";
const UI_STATE: &str = "ui-state";

/// Write the three fields into `state`. The maps' lock and the editor's lock
/// are taken one after the other, never nested.
pub(crate) fn save(params: &HarmonigraphParams, state: &mut PluginState) {
    let maps = serde_json::to_string(&*params.maps.read());
    let window = serde_json::to_string(&*params.editor_state);
    let look = serde_json::to_string(&*params.ui_state.save());
    for (key, json) in [(LATTICE_MAPS, maps), (EDITOR_STATE, window), (UI_STATE, look)] {
        match json {
            Ok(json) => {
                state.fields.insert(key.to_owned(), json);
            }
            Err(error) => nice_error!("The saved field {key} could not be written: {error}"),
        }
    }
}

/// Parse the three fields `state` holds, for [`Fields::commit`] to install.
/// Never refuses: a field that does not parse is skipped.
pub(crate) fn prepare(params: &Arc<HarmonigraphParams>, state: &PluginState) -> Fields {
    Fields {
        params: params.clone(),
        maps: parse(state, LATTICE_MAPS),
        window: parse(state, EDITOR_STATE),
        look: parse(state, UI_STATE),
    }
}

fn parse<T: serde::de::DeserializeOwned>(state: &PluginState, key: &str) -> Option<T> {
    let json = state.fields.get(key)?;
    serde_json::from_str(json)
        .inspect_err(|error| nice_error!("The saved field {key} could not be read: {error}"))
        .ok()
}

/// The fields a restore read, waiting for the parameters to be restored.
pub(crate) struct Fields {
    params: Arc<HarmonigraphParams>,
    maps: Option<MapDocument>,
    window: Option<EguiState>,
    look: Option<String>,
}

impl Prepared for Fields {
    /// Install what was read. Every present `ui-state`, `""` included, goes
    /// to the editor's inbox, which reads an empty one as the fresh look
    /// (#1757).
    fn commit(self: Box<Self>) {
        let Fields { params, maps, window, look } = *self;
        if let Some(maps) = maps {
            *params.maps.write() = maps;
        }
        if let Some(window) = window {
            params.editor_state.restore(window);
        }
        if let Some(look) = look {
            params.ui_state.restore(look);
        }
    }
}
