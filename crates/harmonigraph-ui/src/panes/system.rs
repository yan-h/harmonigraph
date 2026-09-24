//! Rendering cost and editor workspace controls. Quality settings also reach
//! exports; frame cadence and interface controls apply only to the editor.

use super::section;
use crate::widgets::{button_row, choice_row, ValueBar};
use crate::AppearanceDocument;

/// Render quality/cost, then the workspace layout.
pub(super) fn system_pane(
    ui: &mut egui::Ui,
    appearance: &mut AppearanceDocument,
    interaction: &mut crate::Interaction,
) {
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
    section(ui, "Performance", |ui| {
        ValueBar::new(&mut appearance.view.render_scale, 0.5..=2.0, "Lattice resolution")
            .percent()
            .show(ui)
            .on_hover_text(
                "Lattice render dimensions relative to native resolution. \
                     100% is native; \
                     50% uses a quarter of the pixels; \
                     200% uses four times as many. \
                     Lower values reduce GPU load and detail in the editor, preview and exported lattice. Video output size is set separately.",
            );
        let atmosphere = &mut appearance.spectrum.atmosphere;
        ui.add_enabled_ui(atmosphere.effects().light(), |ui| {
            ValueBar::new(
                &mut atmosphere.blur_time_step,
                0.0..=harmonigraph_scene::BLUR_TIME_STEP_MAX,
                "Spectrogram time step",
            )
            .unit(1.0, "×")
            .decimals(1)
            .step(0.5)
            .show(ui)
            .on_hover_text(
                "Sample spacing along time for spectrogram softness and cloud effects, as a multiple of one displayed history column. \
                 Higher values reduce GPU cost at long history durations, with extra softening along time. \
                 0 disables this additional reduction; 1 is the default. Pitch detail and MIDI ribbons are unaffected. \
                 Requires spectrogram softness or cloud texture.",
            );
        });
        // The other half of the cost dial: render scale sets what each frame
        // costs, this sets how many of them there are. Presented as a ceiling
        // rather than a target — the shell decides the actual cadence, and a
        // ceiling above what it can offer simply doesn't bind.
        choice_row(
            ui,
            "Editor frame limit (fps)",
            &mut interaction.fps_cap,
            &[
                (None, "Uncapped", "Repaint as often as the host window allows."),
                (
                    Some(30.0),
                    "30",
                    "Limit redraws to 30 frames per second to reduce display processing cost.",
                ),
                (
                    Some(60.0),
                    "60",
                    "Limit editor redraws to 60 frames per second. Video exports always use 60 fps.",
                ),
                (Some(120.0), "120", "For a high-refresh display."),
                (Some(144.0), "144", "For a high-refresh display."),
            ],
        );
        crate::widgets::checkbox(ui, &mut appearance.view.show_perf, "Performance overlay")
            .on_hover_text(
                "A draggable HUD: frame rate, worst recent frame, memory, and the \
             voice/node workload.",
            );
        if appearance.view.show_perf {
            crate::widgets::checkbox(ui, &mut appearance.view.show_perf_detail, "Frame breakdown")
                .on_hover_text(
                    "Expands the overlay into every stage of the frame, to see which \
                 one is costing you.",
                );
        }
    });

    // Layout: how big the chrome draws, then the pane arrangement itself.
    section(ui, "Interface and layout", |ui| {
        // Sizes the panel, not the picture. Everything the lattice, the roll and
        // the spectrogram draw is measured off the pane it lands in, so this moves
        // the knobs and the tab bars out of the way and leaves what they are
        // pointed at exactly as it was — which is the point of it on a laptop,
        // where the settings column costs more of the screen than the picture can
        // spare. A render is unaffected for the same reason, and deliberately: the
        // offline renderer draws the picture panes and never this.
        ValueBar::new(&mut interaction.ui_scale, crate::theme::UI_SCALE_RANGE, "Interface scale")
            .percent()
            .show(ui)
            .on_hover_text(
                "Size of interface text, controls and tab bars. 100% is the reference size. Picture scale and exported videos are unaffected.",
            );
        crate::widgets::checkbox(ui, &mut appearance.view.frameless, "Hide tab bars (Tab)").on_hover_text(
            "Hide dock tab bars for a continuous picture. Press Tab to toggle while not editing text.",
        );
        button_row(ui, |ui| {
            // Escape hatch for the persisted dock arrangement (it survives
            // every reopen, so a new default layout is otherwise unreachable).
            if ui
                .button("Reset layout")
                .on_hover_text("Restore the default pane arrangement")
                .clicked()
            {
                interaction.reset_layout = true;
            }
        });
    });
}
