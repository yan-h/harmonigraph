//! The Panel pane: the plugin's own knobs rather than the lattice's — how
//! sharply and how hard it renders, and how the panes are arranged. None of
//! this is part of the picture; it's what surrounds it. Pulled out of the
//! Frame pane, where it used to sit at the bottom looking like a view setting.

use super::section;
use crate::widgets::{button_row, ValueBar};
use crate::SharedState;

/// Render quality/cost, then the workspace layout.
pub(super) fn panel_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    // Performance: the cost dial and the meter to judge it by. Render scale is
    // presented as GPU cost rather than as a look setting, because that is what
    // it is: the renderer pins the two things that decide how the lattice LOOKS
    // to the native screen size — the bloom chain is sized off `px_size(1.0)`,
    // and `aa_width` converts its 2px soft band from screen pixels into render
    // pixels — so all the scale can move is resolved detail. Down is the
    // direction that earns its keep (0.5 = a quarter of the lattice's pixels
    // for a near-identical picture); up has little left to sharpen, since every
    // edge is already soft-banded before the extra samples see it. Described as
    // "higher supersamples" it read as a quality knob that did nothing.
    ui.heading("Performance");
    ValueBar::new(&mut state.view.render_scale, 0.5..=2.0, "Render scale")
        .show(ui)
        .on_hover_text(
            "How many pixels the lattice is drawn at: 1.0 = native, 0.5 = a \
             quarter as many. A cost dial, not a look dial — edge softness and \
             the glow are pinned to screen size, so turning it down buys back \
             GPU for very little picture, and turning it up costs a lot for \
             slightly finer detail. Turn it down if the plugin is working the \
             machine hard.",
        );
    ui.checkbox(&mut state.view.show_perf, "Performance overlay").on_hover_text(
        "A corner HUD with frame rate, the GUI's CPU time per frame, \
         process memory, and the voice/node workload — to see if the \
         plugin is working the machine hard. (GPU time isn't measured \
         directly; the node count and render scale stand in for it.)",
    );

    // Layout: the pane arrangement itself.
    section(ui, "Layout");
    ui.checkbox(&mut state.view.frameless, "Frameless").on_hover_text(
        "Hide the tab bars so adjacent panes (lattice over spectrum) \
         record as one seamless surface. Esc restores.",
    );
    button_row(ui, |ui| {
        // Escape hatch for the persisted dock arrangement (it survives
        // every reopen, so a new default layout is otherwise unreachable).
        if ui
            .button("Reset layout")
            .on_hover_text("Restore the default pane arrangement")
            .clicked()
        {
            state.reset_dock_layout();
        }
    });
}
