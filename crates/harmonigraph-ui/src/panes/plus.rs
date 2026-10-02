//! The Idle lattice section of the Lattice settings page: what the lattice
//! draws when nothing is playing — how bright that picture is, and the cross
//! standing at each node position that makes up most of it.
//!
//! Idle positions draw crosses or names, not empty node rings. Their brightness
//! is independent of the silent slices within a sounding node; that control
//! belongs to Note layers ([`super::nodes`]).
//!
//! Nothing is drawn BETWEEN the positions, and a CROSS is why that costs the
//! picture nothing: it draws exactly what a pair of gridlines draws where they
//! meet, so every junction a mesh would have is still there and no ink is spent
//! getting from one junction to the next. What the eye reads the lattice's rows
//! and columns off is the regularity of the field itself.
//!
//! Under the brightness bar, the marker's LENGTH — how far an arm reaches and how much
//! of it tapers. Its thickness has no bar here: it follows Label size, so a
//! cross weighs what the letters beside it weigh
//! ([`PLUS_WIDTH_PER_LABEL_SCALE`](harmonigraph_scene::PLUS_WIDTH_PER_LABEL_SCALE)).
//! There is no bar for what runs
//! to a neighbour, and none for SOFTNESS: a marker's edge is a ring's edge, the
//! same screen-constant band the audio ring and the octave band carry, so the
//! resting field and the layers that stand on it come to an end the same way.
//!
//! A NAMED position draws no marker
//! ([`is_named`](harmonigraph_scene::NodeInstance::is_named)), which is why
//! the Show row in the Note labels block reaches this picture: both a marker and
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
//! the Mappings page ([`super::color`]).

use super::{edge_bar, section};
use crate::widgets::ValueBar;
use crate::AppearanceDocument;
use harmonigraph_scene::MARKER_INK_RANGE;
use harmonigraph_scene::{ViewConfig, PLUS_SIZE_MAX};

/// The resting picture, last on the page: the lattice's own structure, under
/// everything drawn on top of it.
pub(super) fn plus_pane(ui: &mut egui::Ui, appearance: &mut AppearanceDocument) {
    section(ui, "Idle lattice", |ui| {
        // Black is a visible ink choice, not off; Cross length hides the marks.
        ValueBar::new(
            &mut appearance.view.marker_ink,
            MARKER_INK_RANGE,
            "Idle label/cross brightness",
        )
        .unit(1.0, "%")
        // Whole L* points, matching the gradients' brightness scale.
        .integer()
        .show(ui)
        .on_hover_text(
            "Brightness of idle note labels and crosses: 0% is black, 100% is white. \
                     Raise it to keep the resting lattice easy to navigate.",
        );
        // Stored in the same quad UV a node's ring radii are dialled in, so this
        // pair and the Layers bar's Inner handle are positions on ONE axis. The
        // Layers bar shows no numbers, so whether a cross fits inside the middle
        // a node's rings stand around is still judged by eye on the picture.
        edge_bar(
            ui,
            (&mut appearance.view.plus_arm, &mut appearance.view.plus_taper),
            PLUS_SIZE_MAX,
            "Cross length",
            {
                let fresh = ViewConfig::default();
                (fresh.plus_arm, fresh.plus_taper)
            },
            // In true node radii, as the Gap and the Reach read
            // (`QUAD_UV_PERCENT`); the stored pair stays in quad uv.
            |v| format!("{:.1}%", v * super::QUAD_UV_PERCENT),
        )
        .on_hover_text(
            "Cross-arm length from the center, as a percentage of the node radius. \
                     180% reaches the edge no ring crosses on the Layers bar in Note layers. \
                     Solid to the inner handle, faded out by the outer handle. \
                     0% hides crosses; named nodes draw none. \
                     Double-click resets.",
        );
    });
}
