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
//! rows and columns off is the regularity of the field itself, so what is set
//! here is one marker's shape and size and there is no third setting for what
//! runs to its neighbours. The PLUS is where that shows most: a cross is what
//! two gridlines used to say at the moment they crossed, so choosing it keeps
//! the junction and still draws nothing between one junction and the next.
//!
//! A marker has no SOFTNESS to dial. Both shapes are cut with a ring's edge —
//! the same screen-constant band the audio ring and the octave band carry — so
//! the resting field and the layers that stand on it come to an end the same
//! way, and what is left to say about a marker is which shape it is and how
//! big.
//!
//! A NAMED position draws no dot
//! ([`is_named`](harmonigraph_scene::NodeInstance::is_named)), which is why
//! the Show row on the Labels section reaches this picture: both
//! markers say "a position is here" and the name says which one, so under
//! `All` the field is gone entirely and the two settings below go quiet. That
//! is not a setting to add here — it is one picture with two readings of it, and the
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
use crate::widgets::{choice_row, ValueBar};
use crate::SharedState;
use harmonigraph_scene::DotShape;

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
    // Before the size, because it says what the thing IS and the bar under it
    // is the same number either way: a marker's reach. Two shapes and no knob
    // between them — see `DotShape`.
    choice_row(
        ui,
        "Shape",
        &mut state.view.dot_shape,
        &[
            (
                DotShape::Dot,
                "Dot",
                "A filled circle at each position -- the quieter of the two, \
                 a field that reads as texture for the music to arrive on",
            ),
            (
                DotShape::Plus,
                "Plus",
                "A cross of two bars at each position: what a pair of \
                 gridlines said where they met, without the ink that ran \
                 between the meetings. Its arms reach exactly as far as a \
                 dot's edge would, so the bar below sizes either shape",
            ),
        ],
    );
    // The same quad UV a node's ring radii are dialled in, so this bar and
    // Inner on the Layers bar are two readings on one axis: a dot that fits
    // inside the middle a node's rings stand around can be read off the two
    // numbers rather than by eye.
    ValueBar::new(&mut state.view.dot_size, 0.0..=harmonigraph_scene::DOT_SIZE_MAX, "Dot size")
        .show(ui)
        .on_hover_text(
            "How big the marker at each node position is, in the same units \
             a node's ring radii are dialled in -- a dot's radius, or how \
             far a plus's arms reach. 0 takes the markers away, and with \
             them everything a resting lattice draws but the node rings. A \
             position with a note name over it draws none, so showing every \
             name leaves no field for this to size",
        );
}
