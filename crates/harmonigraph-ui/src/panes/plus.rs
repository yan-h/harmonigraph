//! The At rest section of the Display tab's Lattice page: what the lattice
//! draws when nothing is playing — how bright that picture is, and the cross
//! standing at each node position that makes up most of it.
//!
//! An idle position draws no disc of its own, so the markers and the node
//! rings standing at their empty state are the whole of it. The three surfaces
//! involved — the markers, the audio ring where it reads silence, the octave
//! band where an octave is not sounding — are ONE grey
//! ([`ViewConfig::lattice_ground`](harmonigraph_scene::ViewConfig)), which is
//! why the brightness bar is here rather than under the note whose rings it
//! also moves: it is a statement about the resting picture, and the markers
//! are the largest part of that picture.
//!
//! Nothing is drawn BETWEEN the positions, and a CROSS is why that costs the
//! picture nothing: it draws exactly what a pair of gridlines draws where they
//! meet, so every junction a mesh would have is still there and no ink is spent
//! getting from one junction to the next. What the eye reads the lattice's rows
//! and columns off is the regularity of the field itself.
//!
//! So the two bars here are the marker's two LENGTHS — how far an arm reaches
//! and how thick it is — and they are independent on purpose: a long hairline
//! and a short block are different pictures of the same field, and a shape with
//! one fixed proportion could be neither. There is no third bar for what runs
//! to a neighbour, and none for SOFTNESS: a marker's edge is a ring's edge, the
//! same screen-constant band the audio ring and the octave band carry, so the
//! resting field and the layers that stand on it come to an end the same way.
//!
//! A NAMED position draws no marker
//! ([`is_named`](harmonigraph_scene::NodeInstance::is_named)), which is why
//! the Show row on the Labels section reaches this picture: both a marker and
//! a name say "a position is here" and the name says which one, so under
//! `All` the field is gone entirely and the bars below go quiet. That is not a
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

use super::{edge_bar, section};
use crate::widgets::ValueBar;
use crate::SharedState;
use harmonigraph_scene::{ViewConfig, PLUS_SIZE_MAX};

/// The resting picture, last on the page: the lattice's own structure, under
/// everything drawn on top of it.
pub(super) fn plus_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    section(ui, "At rest");
    // First, because it is the one setting here that reaches past the markers:
    // the rings a node wears at their empty state are this same grey, so the
    // bar under this heading moves more of the picture than the two below it.
    //
    // No off position, and none is missing: each surface has its own switch —
    // Arm length for the markers, a width bar for each ring — and every setting
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
            "How bright the lattice is where nothing is sounding: the markers \
             standing at node positions, the audio ring where it reads \
             silence, and the octave band where an octave is not. One grey \
             for the three. Around 9 the resting picture sinks into the \
             background and only played notes draw; 20 is the fresh raised \
             grey.",
        );
    // Length first, then thickness, in the order the shape is built: an arm
    // reaches, and then it has a width. Both are in the same quad UV a node's
    // ring radii are dialled in, so this pair and Inner on the Layers bar are
    // readings on ONE axis — a marker that fits inside the middle a node's
    // rings stand around can be read off the numbers rather than by eye.
    edge_bar(
        ui,
        (&mut state.view.plus_arm, &mut state.view.plus_taper),
        PLUS_SIZE_MAX,
        "Arm length",
        {
            let fresh = ViewConfig::default();
            (fresh.plus_arm, fresh.plus_taper)
        },
        |v| format!("{v:.2}"),
    )
    .on_hover_text(
        "How far a marker's arms reach from the crossing, and how much of that \
         end fades out, in the same units a node's ring radii are dialled in. \
         Solid to the inner handle, gone by the outer -- the way a line drawn \
         into a node arrives at nothing rather than stopping. Close the pair \
         for square ends; open it fully and an arm fades the whole way from \
         the crossing. 0 takes the markers away, and with them everything a \
         resting lattice draws but the node rings",
    );
    // A length of its own rather than a share of the arm above it. Tied to the
    // arm the marker would have one proportion at every size, and this bar is
    // exactly the freedom that buys: a long hairline crossing, or a short thick
    // one, off the same two numbers.
    //
    // No off position, and it needs none: an arm with no thickness is still cut
    // with the screen-constant band every edge here carries, so the bottom of
    // this bar is the thinnest cross the screen can draw. What takes the field
    // away is the bar above.
    ValueBar::new(&mut state.view.plus_width, 0.0..=PLUS_SIZE_MAX, "Arm width")
        .show(ui)
        .on_hover_text(
            "How thick a marker's arms are, all the way across, in the same \
             units a node's ring radii are dialled in. Independent of their \
             length, so a long crossing can stay a hairline and a short one \
             can be a block. At the bottom of the bar an arm is as thin as the \
             screen can draw it rather than absent; past twice the arm length \
             the cross has filled its own square",
        );
}
