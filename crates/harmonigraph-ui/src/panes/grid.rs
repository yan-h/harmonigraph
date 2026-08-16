//! The At rest section of the Display tab's Lattice page: what the lattice
//! draws when nothing is playing — how bright that picture is, and the lines
//! between node positions that make up most of it.
//!
//! An idle position draws no disc and no marker of its own, so the lines and
//! the node rings standing at their empty state are the whole of it. The three
//! surfaces involved — the grid, the audio ring where it reads silence, the
//! octave band where an octave is not sounding — are ONE grey
//! ([`ViewConfig::lattice_ground`](harmonigraph_scene::ViewConfig)), which is
//! why the brightness bar is here rather than under the note whose rings it
//! also moves: it is a statement about the resting picture, and the lines are
//! the largest part of that picture.
//!
//! The ring WIDTHS stay with the note ([`super::nodes`]), because a width is a
//! layer's size whether it is lit or not. What is here is only what nothing
//! sounding looks like.
//!
//! Still no HUE among them, and deliberately: the ground is neutral, and every
//! colour control in the panel is for the music — both of those tables are on
//! the Colors page ([`super::color`]).

use super::section;
use crate::widgets::ValueBar;
use crate::SharedState;

/// The resting picture, last on the page: the lattice's own structure, under
/// everything drawn on top of it.
pub(super) fn grid_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    section(ui, "At rest");
    // First, because it is the one setting here that reaches past the lines:
    // the rings a node wears at their empty state are this same grey, so the
    // bar under this heading moves more of the picture than the two below it.
    //
    // No off position, and none is missing: each surface has its own switch —
    // Line width for the lines, a width bar for each ring — and every setting
    // of THIS bar draws something. The bottom of it is black rather than
    // nothing, which against the panel reads as holes punched through the
    // lattice: a picture worth being able to reach, and worth reaching by
    // dragging rather than by falling off the end.
    ValueBar::new(&mut state.view.lattice_ground, 0.0..=100.0, "Ground")
        // L*, the units the gradients' own Brightness is authored in, so a
        // ground and a gradient can be compared by their numbers. Whole
        // points: the axis is 100 wide and a tenth of one is under a
        // quantization step of the grey it names.
        .integer()
        .show(ui)
        .on_hover_text(
            "How bright the lattice is where nothing is sounding: the grid \
             lines, the audio ring where it reads silence, and the octave \
             band where an octave is not. One grey for the three. Around 9 \
             the resting picture sinks into the background and only played \
             notes draw; 20 is the fresh raised grey.",
        );
    ValueBar::new(&mut state.view.grid_thickness, 0.0..=4.0, "Line width")
        .show(ui)
        .on_hover_text(
            "Line width, as a multiple of the classic hairline. 0 takes \
             the lines away, and with them everything a resting lattice \
             draws but the node rings",
        );
    ValueBar::new(&mut state.view.grid_inset, 0.0..=3.0, "Line gap")
        .show(ui)
        .on_hover_text(
            "How far each line stops short of the node it runs to, as \
             a multiple of the node radius; 0 runs it to the center. \
             The gap is what a node position looks like at rest -- the \
             lines say one is there by stopping short of it",
        );
}
