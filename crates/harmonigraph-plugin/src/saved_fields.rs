//! The three saved fields that belong to Harmonigraph's editor and maps
//! rather than to its tuning: the Lattice Map document, the window, and the
//! editor's look and layout. The Hub's setup saves and restores them beside
//! its musical settings and instance settings, so one owner writes every
//! field a project stores (#1814).
//!
//! A field that does not parse is skipped with a line in the log and one in
//! the editor's Console, which opens on it as on every restore line that
//! reports a loss, and the rest of the restore goes ahead, as the fields did
//! when nice-plug's derive restored them, which skipped one in silence. A
//! field the save lacks keeps what the instance holds. The parameter ids a
//! save holds that no row has are named in the Console the same way, and so
//! are those whose value this build cannot read, which open at their defaults.

use harmonigraph_clap::plugin::PluginState;
use harmonigraph_clap::prelude::shell_error;

use crate::HarmonigraphParams;

pub(crate) const LATTICE_MAPS: &str = "lattice-maps";
pub(crate) const EDITOR_STATE: &str = "editor-state";
pub(crate) const UI_STATE: &str = "ui-state";

/// Write the three fields into `state`. The maps' lock and the editor's lock
/// are taken one after the other, never nested.
pub(crate) fn save(params: &HarmonigraphParams, state: &mut PluginState) {
    let maps = params.maps.read(serde_json::to_string);
    let window = serde_json::to_string(&params.editor_state.saved());
    let look = serde_json::to_string(&*params.ui_state.save());
    for (key, json) in [(LATTICE_MAPS, maps), (EDITOR_STATE, window), (UI_STATE, look)] {
        match json {
            Ok(json) => {
                state.fields.insert(key.to_owned(), json);
            }
            Err(error) => shell_error!("The saved field {key} could not be written: {error}"),
        }
    }
}

/// The three fields a state holds, read, and the Console lines the load owes
/// the editor, until [`Self::install`].
pub(crate) struct Fields {
    maps: Option<harmonigraph_ui::lattice_maps::MapDocument>,
    window: Option<crate::editor::SavedWindow>,
    look: Option<String>,
    notes: Vec<String>,
}

/// Read the three fields `state` holds, after the lines `notes` already
/// holds. Never refuses: a field that does not parse is skipped.
pub(crate) fn read(
    params: &HarmonigraphParams,
    state: &PluginState,
    mut notes: Vec<String>,
) -> Fields {
    const KEPT: &str = "it kept what it had";
    let maps = parse(state, LATTICE_MAPS, KEPT, &mut notes);
    let window = parse(state, EDITOR_STATE, KEPT, &mut notes);
    let look = parse(state, UI_STATE, KEPT, &mut notes);
    let unknown = params.table.unknown(&state.params);
    if !unknown.is_empty() {
        notes.push(format!(
            "This project saved parameters this build no longer has, which were not read: \
             {}.",
            unknown.join(", ")
        ));
    }
    let defaulted = params.table.defaulted(&state.params);
    if !defaulted.is_empty() {
        notes.push(format!(
            "This project saved parameter values this build cannot read, which opened at their \
             defaults: {}. Set them again and save the project.",
            defaulted.join(", ")
        ));
    }
    Fields { maps, window, look, notes }
}

impl Fields {
    /// Install what [`read`] found, and owe the editor its Console lines.
    /// Every present `ui-state`, `""` included, goes to the editor's inbox,
    /// which reads an empty one as the fresh look (#1757).
    pub(crate) fn install(self, params: &HarmonigraphParams) {
        let Fields { maps, window, look, notes } = self;
        if let Some(maps) = maps {
            params.maps.write(|document| *document = maps);
        }
        // Every restore is another project, or another state of one: undo
        // entries name its slots by identity and would write old shapes into
        // the new ones.
        params.map_editor.lock().forget();
        if let Some(window) = window {
            params.editor_state.restore(window);
        }
        if let Some(look) = look {
            params.settings.supersede_reveals();
            params.ui_state.restore(look);
        }
        for note in notes {
            params.settings.notify(crate::configuration::Notice::Restore(note));
        }
    }
}

/// The field `key` of `state`, or `None` when the save lacks it or it does
/// not parse. One that does not parse writes a line in the log and pushes
/// one for the Console onto `notes`, saying what the restore did instead
/// (`outcome`). The one reader of every saved field but the parameters.
pub(crate) fn parse<T: serde::de::DeserializeOwned>(
    state: &PluginState,
    key: &str,
    outcome: &str,
    notes: &mut Vec<String>,
) -> Option<T> {
    let json = state.fields.get(key)?;
    serde_json::from_str(json)
        .inspect_err(|error| {
            shell_error!("The saved field {key} could not be read: {error}");
            notes.push(format!(
                "This project's saved {key} could not be read, so {outcome}: {error}."
            ));
        })
        .ok()
}
