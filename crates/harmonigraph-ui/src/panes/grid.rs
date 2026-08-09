//! The Display tab's Grid section: the faint lines between node positions, and
//! the gap each one leaves around the position it runs to. Idle positions draw
//! nothing at all -- no disc, no marker -- so these two settings are the whole
//! of what carries the lattice's shape when nothing is playing.
//!
//! Two settings, and the grid's COLOR is deliberately not one of them: the
//! lines draw in the skin's hairline grey, the same one this pane's own rules
//! are drawn in. The structural layer is chrome that happens to be in the
//! picture, and every color control in the panel is for the music — which is
//! [`super::color`], the section that holds all of them.

use crate::widgets::ValueBar;
use crate::SharedState;

/// The two lines settings.
///
/// No leading heading, unlike the Nodes and View bodies: this section is one
/// group, and the only name it could take is the fold-out header's own.
pub(super) fn grid_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    ValueBar::new(&mut state.view.grid_thickness, 0.0..=4.0, "Line width")
        .show(ui)
        .on_hover_text(
            "Line width, as a multiple of the classic hairline. 0 takes \
             the lines away, and with them everything a resting lattice \
             draws",
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
