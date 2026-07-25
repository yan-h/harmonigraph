//! The Scene pane: everything drawn *around* the sounding notes — the home
//! grid that carries the lattice's shape at rest, the note-name labels, the
//! trail of where the music has been, and scene-wide effects. The played
//! note's own look lives in [`super::nodes`].

use super::section;
use crate::widgets::{button_row, choice_row, ValueBar};
use crate::SharedState;
use lattice_core::NoteTracker;
use lattice_scene::{IdleMarker, LabelRim, TrailMark, ViewConfig};

/// The scene-wide look, top to bottom: the always-drawn Home grid, the Labels
/// on nodes, the Trail of visited nodes, then Effects layered over everything.
/// Scrolls so the full list is reachable in a short pane.
pub(super) fn scene_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            home_grid_section(ui, &mut state.view);
            labels_section(ui, &mut state.view);
            trail_section(ui, &mut state.view, &mut state.tracker);
            effects_section(ui, &mut state.view);
        });
}

/// Home grid: the always-drawn structural layer -- the faint lines between
/// node positions AND the idle marker sitting at each unlit home-sheet node.
/// Idle positions draw no disc, so together these are what carry the
/// lattice's shape when nothing is playing. They share one color for that
/// reason.
fn home_grid_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    // First section of the pane: a plain heading, no leading rule (matches
    // the Nodes pane's Core section).
    ui.heading("Home grid");
    button_row(ui, |ui| {
        ui.label("Color");
        ui.color_edit_button_rgba_unmultiplied(&mut view.grid_color)
            .on_hover_text(
                "Color of the whole idle structure -- grid lines and \
                 idle node markers alike. The alpha is how faint an \
                 unlit LINE draws; markers keep their own presence. \
                 Lit segments still take their notes' color",
            );
    });
    ValueBar::new(&mut view.grid_thickness, 0.0..=4.0, "Thickness")
        .show(ui)
        .on_hover_text("Line width, as a multiple of the classic hairline");
    ValueBar::new(&mut view.grid_inset, 0.0..=3.0, "Line gap")
        .show(ui)
        .on_hover_text(
            "How far each line stops short of the node it runs to, as \
             a multiple of the node radius; 0 runs it to the center",
        );
    ui.checkbox(&mut view.grid_dashed, "Dashed").on_hover_text(
        "Dash the in-plane lines. The sevens-axis links are always \
         dashed -- that's what marks them as depth links",
    );

    // The idle marker: shown ALWAYS at each unlit home-sheet node,
    // independent of the active appearance and of whether a note
    // plays there (a sounding note just draws over it). Off-sheet
    // positions are marked by the lines alone.
    choice_row(
        ui,
        "Marker",
        &mut view.idle_marker,
        &[
            (IdleMarker::None, "None", "No idle marker"),
            (IdleMarker::Dot, "Dot", "A filled dot at the radius below"),
            (
                IdleMarker::Circle,
                "Circle",
                "A thin outline circle at the radius below",
            ),
        ],
    );
    ui.add_enabled_ui(view.idle_marker != IdleMarker::None, |ui| {
        ValueBar::new(&mut view.idle_radius, 0.0..=0.9, "Marker radius")
            .show(ui)
            .on_hover_text(
                "Size of the idle marker; independent of the active \
                 Core (0.46 is the classic placeholder ring)",
            );
    });
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
    // Also the labels' whole cost, which is why the options say what they
    // cost: the rim is the same text drawn again once per sample, and on a
    // busy lattice the labels are most of the geometry in the frame.
    ui.add_enabled_ui(view.show_labels, |ui| {
        choice_row(
            ui,
            "Rim",
            &mut view.label_rim,
            &[
                (
                    LabelRim::Halo,
                    "Halo",
                    "A soft dark pool around the text. Reads over anything, \
                     including a node at full brightness. 32 stamps a label.",
                ),
                (
                    LabelRim::Outline,
                    "Outline",
                    "A crisp dark line hugging the glyphs, no pool. Keeps the \
                     separation for about a quarter of the cost. 8 stamps.",
                ),
                (
                    LabelRim::Off,
                    "Off",
                    "The glyphs alone — the cheapest labels there are. Fine \
                     wherever what is behind them stays dark.",
                ),
            ],
        );
    });
}

/// Trail: where the music has already been. Rides the IDLE layer only -- a
/// visited node wears a quietly different version of the same small grey mark
/// an unvisited one does -- so it can accumulate over a whole piece without
/// ever competing with the notes actually sounding.
fn trail_section(ui: &mut egui::Ui, view: &mut ViewConfig, tracker: &mut NoteTracker) {
    section(ui, "Trail");
    choice_row(
        ui,
        "Mark",
        &mut view.trail_mark,
        &[
            (TrailMark::Off, "Off", "Show only what is sounding"),
            (
                TrailMark::Lift,
                "Lift",
                "A visited node's idle marker draws a lighter grey. \
                 The quietest option: no new shape and no color, \
                 just a little more presence",
            ),
            (
                TrailMark::Ring,
                "Ring",
                "A pale circle around the node, where its sounding \
                 disc would be -- a ghost of the note that was \
                 there. The only mark that draws with the idle \
                 marker off",
            ),
            (
                TrailMark::Tint,
                "Tint",
                "The idle marker keeps a hint of the color the note \
                 was played in, at idle brightness -- so the trail \
                 says what the music was doing there, not just that \
                 it was",
            ),
        ],
    );
    ui.add_enabled_ui(view.trail_mark != TrailMark::Off, |ui| {
        ValueBar::new(&mut view.trail_strength, 0.0..=1.0, "Strength")
            .show(ui)
            .on_hover_text(
                "How far the mark departs from a plain idle node. \
                 The whole range is quiet -- even 1 stays well short \
                 of reading as a sounding note",
            );
        // Marks that MODIFY the idle marker need one to be showing;
        // say so rather than leaving a setting that does nothing.
        if view.trail_mark.needs_idle_marker() && view.idle_marker == IdleMarker::None {
            ui.weak("Needs a Home grid marker other than None.");
        }
    });
    // Independent of the mark: the text is its own channel, and is
    // useful with the marks off.
    ui.checkbox(&mut view.trail_labels, "Keep note names")
        .on_hover_text(
            "Leave the note name and cents on a visited node, so the \
             harmonic space reads off the screen by name with its \
             tuning. Needs Note names on",
        );
    ui.add_enabled_ui(
        view.trail_mark != TrailMark::Off || view.trail_labels,
        |ui| {
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
                    .on_hover_text(
                        "Forget everything played so far; sounding notes stay",
                    )
                    .clicked()
                {
                    tracker.clear_history();
                }
            });
        },
    );
}

/// Effects: scene-wide extras layered over the notes.
fn effects_section(ui: &mut egui::Ui, view: &mut ViewConfig) {
    section(ui, "Effects");
    // 0 = off (the renderer skips the whole post-process chain), so
    // the bar doubles as the toggle.
    ValueBar::new(&mut view.bloom_strength, 0.0..=1.5, "Bloom")
        .show(ui)
        .on_hover_text("Soft halo around bright notes; 0 turns the post-process off");
}
