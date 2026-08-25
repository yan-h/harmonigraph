//! The Display tab's System page: the plugin's own knobs rather than the
//! lattice's — how sharply and how hard it renders, and how the panes are
//! arranged. None of this is part of a picture; it's what surrounds them, which
//! is why it is a page of its own rather than the foot of [`super::view`],
//! where it would sit under the camera looking like a view setting.
//!
//! Called System because the picker is where a name has to do its work.
//! "Panel" names the thing being looked AT rather than anything the page
//! changes, and sits one letter from "pane", which is what every tab in the
//! dock is.

use super::section;
use crate::widgets::{button_row, choice_row, ValueBar};
use crate::SharedState;

/// Render quality/cost, then the workspace layout.
pub(super) fn system_pane(ui: &mut egui::Ui, state: &mut SharedState) {
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
    ValueBar::new(&mut state.view.render_scale, 0.5..=2.0, "Render scale").show(ui).on_hover_text(
        "How many pixels the lattice renders at: 1.0 native, 0.5 a quarter \
             as many. A cost dial, not a look dial — turn it down if the plugin \
             works the machine hard.",
    );
    // The other half of the cost dial: render scale sets what each frame
    // costs, this sets how many of them there are. Presented as a ceiling
    // rather than a target — the shell decides the actual cadence, and a
    // ceiling above what it can offer simply doesn't bind.
    choice_row(
        ui,
        "Frame rate",
        &mut state.fps_cap,
        &[
            (None, "Uncapped", "Repaint as often as the host window allows."),
            (Some(30.0), "30", "Halves the GUI's frame cost. The lattice and the roll still move smoothly; fast transients in the spectrum get chunkier."),
            (Some(60.0), "60", "A middle setting for a loaded project."),
            (Some(120.0), "120", "For a high-refresh display."),
            (Some(144.0), "144", "For a high-refresh display."),
        ],
    );
    ui.checkbox(&mut state.view.show_perf, "Performance overlay").on_hover_text(
        "A draggable HUD: frame rate, worst recent frame, memory, and the \
         voice/node workload.",
    );
    if state.view.show_perf {
        ui.checkbox(&mut state.view.show_perf_detail, "Frame breakdown").on_hover_text(
            "Expands the overlay into every stage of the frame, to see which \
             one is costing you.",
        );
    }

    // Layout: how big the chrome draws, then the pane arrangement itself.
    section(ui, "Layout");
    // Sizes the panel, not the picture. Everything the lattice, the roll and
    // the spectrogram draw is measured off the pane it lands in, so this moves
    // the knobs and the tab bars out of the way and leaves what they are
    // pointed at exactly as it was — which is the point of it on a laptop,
    // where the settings column costs more of the screen than the picture can
    // spare. A render is unaffected for the same reason, and deliberately: the
    // offline renderer draws the picture panes and never this.
    ValueBar::new(&mut state.ui_scale, crate::theme::UI_SCALE_RANGE, "UI scale")
        .display(|v| format!("{:.0}%", v * 100.0))
        .show(ui)
        .on_hover_text(
            "Size of the panel's own text and controls. The pictures are drawn \
             from their panes' space and don't change — neither does a render.",
        );
    ui.checkbox(&mut state.view.frameless, "Frameless (Tab)").on_hover_text(
        "Hide the tab bars, so adjacent panes record as one seamless surface. \
         Tab toggles it from anywhere except a text field.",
    );
    button_row(ui, |ui| {
        // Escape hatch for the persisted dock arrangement (it survives
        // every reopen, so a new default layout is otherwise unreachable).
        if ui.button("Reset layout").on_hover_text("Restore the default pane arrangement").clicked()
        {
            state.workspace.reset_dock_layout();
        }
    });
}
