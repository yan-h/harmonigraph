//! The At rest section of the Display tab's Lattice page: what the lattice
//! draws when nothing is playing — how bright that picture is, and the dot
//! standing at each node position that makes up most of it.
//!
//! An idle position draws no disc of its own, so the dots and the node rings
//! standing at their empty state are the whole of it. The three surfaces
//! involved — the dots, the audio ring where it reads silence, the octave band
//! where an octave is not sounding — are ONE grey
//! ([`ViewConfig::lattice_ground`](harmonigraph_scene::ViewConfig)), which is
//! why the brightness bar is here rather than under the note whose rings it
//! also moves: it is a statement about the resting picture, and the dots are
//! the largest part of that picture.
//!
//! Nothing is drawn BETWEEN the positions. What the eye reads the lattice's
//! rows and columns off is the regularity of the field itself, so the two bars
//! below the ground are about one dot's shape and there is no third setting for
//! what runs to its neighbours.
//!
//! A NAMED position draws no dot
//! ([`is_named`](harmonigraph_scene::NodeInstance::is_named)), which is why
//! the Show row on the Labels section reaches this picture: both
//! markers say "a position is here" and the name says which one, so under
//! `All` the field is gone entirely and these two bars go quiet. That is not a
//! setting to add here — it is one picture with two readings of it, and the
//! place to change which is the row that chooses the names.
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
pub(super) fn dots_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    section(ui, "At rest");
    // First, because it is the one setting here that reaches past the dots:
    // the rings a node wears at their empty state are this same grey, so the
    // bar under this heading moves more of the picture than the two below it.
    //
    // No off position, and none is missing: each surface has its own switch —
    // Dot size for the dots, a width bar for each ring — and every setting
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
            "How bright the lattice is where nothing is sounding: the dots \
             standing at node positions, the audio ring where it reads \
             silence, and the octave band where an octave is not. One grey \
             for the three. Around 9 the resting picture sinks into the \
             background and only played notes draw; 20 is the fresh raised \
             grey.",
        );
    // The same quad UV a node's ring radii are dialled in, so this bar and
    // Inner on the Layers bar are two readings on one axis: a dot that fits
    // inside the middle a node's rings stand around can be read off the two
    // numbers rather than by eye.
    ValueBar::new(&mut state.view.dot_size, 0.0..=harmonigraph_scene::DOT_SIZE_MAX, "Dot size")
        .show(ui)
        .on_hover_text(
            "How big the dot at each node position is, in the same units \
             a node's ring radii are dialled in. 0 takes the dots away, and \
             with them everything a resting lattice draws but the node \
             rings. A position with a note name over it draws no dot, so \
             showing every name leaves no field for this to size",
        );
    ValueBar::new(&mut state.view.dot_feather, 0.0..=1.0, "Dot feather")
        .show(ui)
        .on_hover_text(
            "How much of a dot is its soft edge, as a share of its radius. \
             0 is a hard-edged disc, 1 a dot that falls off from its own \
             centre -- a position marked rather than an object drawn. A \
             share rather than a width, so growing a dot keeps its look",
        );
}
