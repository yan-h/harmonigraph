//! The Grid section of the Display tab's Lattice page: the faint lines between
//! node positions, and the gap each one leaves around the position it runs to.
//! An idle position draws no disc and no marker of its own, so with the audio
//! ring off these two settings are the whole of what carries the lattice's
//! shape when nothing is playing. With the ring on — which is where a fresh
//! view starts — every position also wears its annulus at the ring's floor
//! colour, and the lines are then the half of that shape which does not move
//! with the sound.
//!
//! Two settings, and the grid's COLOR is deliberately not one of them: the
//! lines draw in the skin's hairline grey, the same one this pane's own rules
//! are drawn in. The structural layer is chrome that happens to be in the
//! picture, and every color control in the panel is for the music — both of
//! those tables are on the Colors page ([`super::color`]).

use super::section;
use crate::widgets::ValueBar;
use crate::SharedState;

/// The two lines settings, last on the page: the lattice's own structure, under
/// everything drawn on top of it.
pub(super) fn grid_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    section(ui, "Grid");
    ValueBar::new(&mut state.view.grid_thickness, 0.0..=4.0, "Line width")
        .show(ui)
        .on_hover_text(
            "Line width, as a multiple of the classic hairline. 0 takes \
             the lines away, and with them everything a resting lattice \
             draws but the audio ring",
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
