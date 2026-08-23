//! The At rest section of the Display tab's Lattice page: what the lattice
//! draws when nothing is playing — how bright that picture is, and the cross
//! standing at each node position that makes up most of it.
//!
//! An idle position draws no disc of its own, so the markers and the node
//! rings standing at their empty state are the whole of it. Two brightness bars
//! open the section, on one `L*` axis so they are read against each other:
//! Ground is the NODE at rest, one grey under the audio ring where it reads
//! silence and the octave band where an octave is not sounding
//! ([`ViewConfig::lattice_ground`](harmonigraph_scene::ViewConfig)), and Marker
//! ink is the field of crosses
//! ([`ViewConfig::marker_ink`](harmonigraph_scene::ViewConfig)).
//!
//! They are both here rather than one of them living under the note whose rings
//! it moves, because the question they answer is the same one and it is about
//! this picture: how visible is the lattice with nothing playing. Equal numbers
//! are the whole resting lattice in one grey. What wants them apart is the glow
//! — a Ground dark enough for the light behind the notes to read takes the node
//! rings down where it should and the marker field with them, and the field is
//! what says where the positions ARE.
//!
//! Nothing is drawn BETWEEN the positions, and a CROSS is why that costs the
//! picture nothing: it draws exactly what a pair of gridlines draws where they
//! meet, so every junction a mesh would have is still there and no ink is spent
//! getting from one junction to the next. What the eye reads the lattice's rows
//! and columns off is the regularity of the field itself.
//!
//! Under those pair the marker's two LENGTHS — how far an arm reaches and how
//! thick it is — and they are independent on purpose: a long hairline and a
//! short block are different pictures of the same field, and a shape with one
//! fixed proportion could be neither. There is no third bar for what runs
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
    // First, because it is the one setting here that reaches past this
    // section: the rings it moves belong to the note, and every other bar
    // under this heading is the marker field's.
    //
    // No off position, and none is missing: each ring has a width bar, so
    // every setting of THIS one draws something. The bottom of it is black
    // rather than nothing, which against the panel reads as holes punched
    // through the lattice: a picture worth being able to reach, and worth
    // reaching by dragging rather than by falling off the end.
    ValueBar::new(&mut state.view.lattice_ground, 0.0..=100.0, "Ground")
        // L*, the units the gradients' own Brightness is authored in, so a
        // ground and a gradient can be compared by their numbers. Whole
        // points: the axis is 100 wide and a tenth of one is under a
        // quantization step of the grey it names.
        .integer()
        .show(ui)
        .on_hover_text(
            "How bright a node is where nothing is sounding: the audio ring \
             where it reads silence, and the octave band where an octave is \
             not. One grey for the two. Around 9 a quiet node sinks into the \
             background and only played ones draw; 20 is the fresh raised \
             grey. The markers standing between the nodes are on their own \
             bar below, so this can go as dark as the glow behind the notes \
             wants without taking the lattice's structure with it.",
        );
    // Second, and on the same axis, so the pair is dialled by comparing two
    // numbers. Together they are the whole resting picture; apart they are the
    // one thing a person navigates by held above the one that is only ever
    // backdrop.
    //
    // No off position, for the bar above's reason: Arm length at 0 is the
    // marker field's own switch, and every setting of this one draws.
    ValueBar::new(&mut state.view.marker_ink, 0.0..=100.0, "Marker ink")
        // Whole points on the same L* axis as Ground, and that IS the point of
        // the units: two bars a person is meant to read against each other
        // have to be counted in the same thing.
        .integer()
        .show(ui)
        .on_hover_text(
            "How bright the crosses standing at the lattice's resting \
             positions are drawn. The same L* the Ground above is in, so equal \
             numbers put the whole resting lattice in one grey and any \
             difference is read straight off the pair. Hold it above the \
             Ground to keep the structure legible while a node's empty rings \
             sink away — which is what a strong glow wants, the light behind \
             the notes reading best over a dark node. This is the marker's \
             INK; the pool of light it sits in is Marker light, over on the \
             Glow section, and the two are separate ways of finding a position \
             in a dark picture.",
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
