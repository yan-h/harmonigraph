//! The Labels section of the Lattice settings page: the text on the
//! lattice as one subject — what a node's label says (its name, its cents),
//! how big it draws, and which nodes carry one at all.
//!
//! Per-node text, and what keeps it out of [`super::nodes`] is that a label
//! rides a hovered node, a sounding one and a remembered one alike: the
//! subject here is the text. The trail is that last case and belongs here for
//! the same reason — it IS labels persisting, which is why it is one option of
//! the Show row rather than a heading of its own.
//!
//! Active labels are fixed at white. Resting names use the marker field's ink,
//! over in [`super::plus`], so a resting name and the crosses standing around
//! it are one grey (`label_ink` in [`super::lattice`]).

use super::section;
use crate::widgets::{button_row, choice_row, ValueBar};
use crate::PictureState;
use harmonigraph_core::NoteTracker;
use harmonigraph_scene::{NoteNames, ViewConfig};

/// What a label says, which nodes carry one and how big it draws.
pub(super) fn labels_pane(ui: &mut egui::Ui, state: &mut PictureState) {
    section(ui, "Note labels", |ui| {
        names_row(ui, &mut state.appearance.view);
        crate::widgets::checkbox(ui, &mut state.appearance.view.show_cents, "Show pitch in cents")
            .on_hover_text("Each node's pitch class in cents, under its name.");
        ValueBar::new(&mut state.appearance.view.label_scale, crate::SCALE_BAR_RANGE, "Label scale")
            .unit(1.0, "×")
            .show(ui)
            .on_hover_text(
                "Text size relative to the node. 1× is the reference size; labels also follow lattice zoom.",
            );
        clear_button(ui, &state.appearance.view, &mut state.runtime.tracker);
    });
}

/// Which nodes are named: the whole lattice, everywhere the music has been,
/// or only what is sounding.
///
/// One row rather than a switch and a knob, because the three are one
/// question — how far a name reaches past the note that put it there — and no
/// answer to it is a modifier of another.
fn names_row(ui: &mut egui::Ui, view: &mut ViewConfig) {
    choice_row(
        ui,
        "Label visibility",
        &mut view.note_names,
        &[
            (NoteNames::All, "All", "Every node on screen carries its name, played or not"),
            (
                NoteNames::Past,
                "History",
                "Keep labels on nodes that have been played, as well as nodes sounding now.",
            ),
            (
                NoteNames::Played,
                "Sounding",
                "Sounding and fading nodes, plus the node under the pointer",
            ),
        ],
    );
}

/// Forget the visited nodes, drawn only under [`NoteNames::Past`].
///
/// Absent rather than grayed under the other two: what it clears is the
/// history behind the kept names, and neither of those modes reads one, so
/// pressed there it would take an effect nothing on screen could show. Past
/// is also the only mode where a piece's territory accumulates, and so the
/// only one where ending it is a thing to want.
fn clear_button(ui: &mut egui::Ui, view: &ViewConfig, tracker: &mut NoteTracker) {
    if view.note_names != NoteNames::Past {
        return;
    }
    button_row(ui, |ui| {
        if ui
            .button("Clear label history")
            .on_hover_text(
                "Forget previously visited lattice nodes; sounding notes and Analyzer history stay",
            )
            .clicked()
        {
            tracker.clear_history();
        }
    });
}
