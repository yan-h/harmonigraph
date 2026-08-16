//! The Labels section of the Display tab's Lattice page: the text on the
//! lattice as one subject — what a node's label says (its name, its cents), how
//! big it draws, and how long it outlives the note that put it there.
//!
//! Per-node text, but not per-note styling, which is what keeps it out of
//! [`super::nodes`]: a label rides a hovered node, a sounding one and a
//! remembered one alike, so it is about the text rather than about any state
//! the note is in. The Trail is that last case and belongs here for the same
//! reason — it IS labels persisting, and its checkbox already needs Note names
//! on to have anything to leave behind.

use super::section;
use crate::widgets::{button_row, ValueBar};
use crate::SharedState;
use harmonigraph_core::NoteTracker;
use harmonigraph_scene::ViewConfig;

/// What a label says and how big it draws, then how long it stays.
///
/// Trail takes a heading of its own below them — it names something narrower
/// than the section, which is the whole of when a heading earns its row here.
pub(super) fn labels_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    section(ui, "Labels");
    ui.checkbox(&mut state.view.show_labels, "Note names")
        .on_hover_text("Name each sounding node. The rest of this section rides on it.");
    // Cents ride on the labels, so the toggle grays out with them off.
    ui.add_enabled(
        state.view.show_labels,
        egui::Checkbox::new(&mut state.view.show_cents, "Cents"),
    )
    .on_hover_text("Each node's pitch class in cents, under its name.");
    ui.add_enabled_ui(state.view.show_labels, |ui| {
        ValueBar::new(&mut state.view.label_scale, crate::SCALE_BAR_RANGE, "Name size")
            .show(ui)
            .on_hover_text(
                "Label size on the node. Labels already zoom with the lattice; \
                 this sets how big a name is on its node.",
            );
    });
    trail_section(ui, &mut state.view, &mut state.tracker);
}

/// Trail: where the music has already been, drawn in TYPE and nothing else --
/// a visited node keeps its note name and cents. No mark is added to the
/// picture at all, which is what lets a whole piece's territory accumulate
/// without ever competing with the notes actually sounding: a memory and a
/// sounding note are not the same kind of thing on screen.
fn trail_section(ui: &mut egui::Ui, view: &mut ViewConfig, tracker: &mut NoteTracker) {
    section(ui, "Trail");
    // The names ARE the trail, so this checkbox is its on/off -- with it
    // clear nothing remembers a visited node, and the two settings under it
    // have nothing to act on.
    ui.checkbox(&mut view.trail_labels, "Keep note names")
        .on_hover_text(
            "A visited node keeps its name and cents, so the piece's territory \
             stays readable. Needs Note names.",
        );
    ui.add_enabled_ui(view.trail_labels, |ui| {
        // 0 = never forget, which is the point of the feature;
        // the bar doubles as the on/off for forgetting at all.
        ValueBar::new(&mut view.trail_memory, 0.0..=600.0, "Memory")
            .show(ui)
            .on_hover_text(
                "Seconds before a note is forgotten, counted from \
                 when it last sounded. 0 = never, so a whole \
                 piece's territory stays",
            );
        button_row(ui, |ui| {
            if ui
                .button("Clear trail")
                .on_hover_text("Forget everything played so far; sounding notes stay")
                .clicked()
            {
                tracker.clear_history();
            }
        });
    });
}
