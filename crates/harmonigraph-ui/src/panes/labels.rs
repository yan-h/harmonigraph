//! The Labels section of the Display tab's Lattice page: the text on the
//! lattice as one subject — what a node's label says (its name, its cents),
//! how big it draws, and which nodes carry one at all.
//!
//! Per-node text, but not per-note styling, which is what keeps it out of
//! [`super::nodes`]: a label rides a hovered node, a sounding one and a
//! remembered one alike, so it is about the text rather than about any state
//! the note is in. The trail is that last case and belongs here for the same
//! reason — it IS labels persisting, which is why it is one option of the
//! Show row rather than a heading of its own.

use super::section;
use crate::widgets::{button_row, choice_row, ValueBar};
use crate::SharedState;
use harmonigraph_core::NoteTracker;
use harmonigraph_scene::{NoteNames, ViewConfig};

/// What a label says, which nodes carry one, and how big it draws.
pub(super) fn labels_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    section(ui, "Labels");
    ui.checkbox(&mut state.view.show_labels, "Note names")
        .on_hover_text("Text on the lattice. The rest of this section rides on it.");
    ui.add_enabled_ui(state.view.show_labels, |ui| {
        names_row(ui, &mut state.view);
        // Cents ride on the labels, so the toggle grays out with them.
        ui.checkbox(&mut state.view.show_cents, "Cents")
            .on_hover_text("Each node's pitch class in cents, under its name.");
        ValueBar::new(&mut state.view.label_scale, crate::SCALE_BAR_RANGE, "Name size")
            .show(ui)
            .on_hover_text(
                "Label size on the node. Labels already zoom with the lattice; \
                 this sets how big a name is on its node.",
            );
        clear_button(ui, &state.view, &mut state.tracker);
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
        "Show",
        &mut view.note_names,
        &[
            (NoteNames::All, "All", "Every node on screen carries its name, played or not"),
            (
                NoteNames::Past,
                "Past",
                "A visited node keeps its name and cents for good, so the piece's \
                 territory stays readable",
            ),
            (NoteNames::Played, "Played", "Only the nodes sounding now"),
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
            .button("Clear note names")
            .on_hover_text("Forget everything played so far; sounding notes stay")
            .clicked()
        {
            tracker.clear_history();
        }
    });
}
