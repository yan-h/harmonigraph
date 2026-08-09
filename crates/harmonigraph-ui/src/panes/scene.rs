//! The Scene pane: everything drawn *around* the sounding notes — the home
//! grid that carries the lattice's shape at rest, the note-name labels, and
//! the trail of where the music has been. The played note's own look lives in
//! [`super::nodes`], and Bloom went with it: a halo around bright notes is a
//! property of the notes, and it was the only thing an "Effects" heading here
//! ever had to hold.

use super::section;
use crate::widgets::{button_row, ValueBar};
use crate::SharedState;
use harmonigraph_core::NoteTracker;
use harmonigraph_scene::ViewConfig;

/// The scene-wide look, top to bottom: the always-drawn Home grid, the Labels
/// on nodes, then the Trail of visited nodes. Scrolls so the full list is
/// reachable in a short pane.
pub(super) fn scene_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            home_grid_section(ui, &mut state.view);
            labels_section(ui, &mut state.view);
            trail_section(ui, &mut state.view, &mut state.tracker);
        });
}

/// Home grid: the whole of what the lattice draws at rest. Idle positions
/// draw nothing at all -- no disc, no marker -- so these faint lines between
/// them, and the gap each one leaves around the position it runs to, are what
/// carry the lattice's shape when nothing is playing.
///
/// Two settings, and its COLOR is deliberately not one of them: the grid
/// draws in the skin's hairline grey, the same one this pane's own rules are
/// drawn in. The structural layer is chrome that happens to be in the
/// picture, and every color control in the panel is for the music.
fn home_grid_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    // First section of the pane: a plain heading, no leading rule (matches
    // the Nodes pane's Color section, which leads that pane the same way).
    ui.heading("Home grid");
    ValueBar::new(&mut view.grid_thickness, 0.0..=4.0, "Line width")
        .show(ui)
        .on_hover_text(
            "Line width, as a multiple of the classic hairline. 0 takes \
             the lines away, and with them everything a resting lattice \
             draws",
        );
    ValueBar::new(&mut view.grid_inset, 0.0..=3.0, "Line gap")
        .show(ui)
        .on_hover_text(
            "How far each line stops short of the node it runs to, as \
             a multiple of the node radius; 0 runs it to the center. \
             The gap is what a node position looks like at rest -- the \
             lines say one is there by stopping short of it",
        );
}

/// Labels: the note text drawn on hovered and sounding nodes.
fn labels_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Labels");
    ui.checkbox(&mut view.show_labels, "Note names");
    // Cents ride on the labels, so the toggle grays out with them off.
    ui.add_enabled(
        view.show_labels,
        egui::Checkbox::new(&mut view.show_cents, "Cents"),
    );
    ui.add_enabled_ui(view.show_labels, |ui| {
        ValueBar::new(&mut view.label_scale, crate::SCALE_BAR_RANGE, "Name size")
            .show(ui)
            .on_hover_text(
                "Overall size of a label -- the name, the marks beside it and \
                 the cents line under it together, so it keeps its \
                 proportions.\n\nLabels already follow the camera: they grow \
                 and shrink with the lattice as you zoom, so a name stays the \
                 same size ON its node whatever the framing. This sets what \
                 that size is",
            );
    });
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
            "Leave the note name and cents on a visited node, so the \
             harmonic space reads off the screen by name with its \
             tuning. Needs Note names on",
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
