//! The Labels section of the Display tab's Lattice page: the text on the
//! lattice as one subject — what a node's label says (its name, its cents),
//! how big it draws, how bright it is while its note sounds, and which nodes
//! carry one at all.
//!
//! Per-node text, and what keeps it out of [`super::nodes`] is that a label
//! rides a hovered node, a sounding one and a remembered one alike: the
//! subject here is the text. The trail is that last case and belongs here for
//! the same reason — it IS labels persisting, which is why it is one option of
//! the Show row rather than a heading of its own.
//!
//! Sounding ink is not the exception to that it looks like. What it sets is
//! how a NAME is drawn; the note under it only chooses which of the label's
//! two ends is in force, and the other end is the marker field's own ink, over
//! in [`super::plus`] — a resting name and the crosses standing around it are
//! one grey (`label_ink` in [`super::lattice`]).

use super::section;
use crate::widgets::{button_row, choice_row, ValueBar};
use crate::SharedState;
use harmonigraph_core::NoteTracker;
use harmonigraph_scene::{NoteNames, ViewConfig};

/// What a label says, which nodes carry one, how big it draws and how bright
/// it is while its note sounds.
pub(super) fn labels_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    section(ui, "Note labels");
    ui.checkbox(&mut state.view.show_labels, "Show note names")
        .on_hover_text("Show note names on lattice nodes.");
    ui.add_enabled_ui(state.view.show_labels, |ui| {
        names_row(ui, &mut state.view);
        // Cents ride on the labels, so the toggle grays out with them.
        ui.checkbox(&mut state.view.show_cents, "Show pitch in cents")
            .on_hover_text("Each node's pitch class in cents, under its name.");
        ValueBar::new(&mut state.view.label_scale, crate::SCALE_BAR_RANGE, "Label scale")
        .unit(1.0, "×")
            .show(ui)
            .on_hover_text(
                "Text size relative to the node. 1× is the reference size; labels also follow lattice zoom.",
            );
        // Last of the bars, because it is the only one here whose other end is
        // somewhere else: what it sets is one end of a pair, and the pair reads
        // as a pair only once the size and the cents under the name are
        // settled.
        //
        // No off position and none to want. Equal to the Marker ink under the
        // At rest heading IS the off position — every label in the resting
        // field's one grey, and the type answering to the music by nothing.
        ValueBar::new(&mut state.view.sounding_ink, 0.0..=100.0, "Active brightness")
        .unit(1.0, "%")
            // Whole points on the L* axis the Ground and Marker ink bars are
            // counted in, which is the point of the units here: this number is
            // only readable against the resting end's, and the two sit in
            // different sections of the page.
            .integer()
            .show(ui)
            .on_hover_text(
                "Brightness of note labels while sounding: 0% is black, 100% is white. \
                 Released labels return to Idle label brightness over the Note fade time.",
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
        "Label visibility",
        &mut view.note_names,
        &[
            (NoteNames::All, "All", "Every node on screen carries its name, played or not"),
            (
                NoteNames::Past,
                "History",
                "Keep labels on nodes that have been played, as well as nodes sounding now.",
            ),
            (NoteNames::Played, "Sounding", "Only the nodes sounding now"),
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
