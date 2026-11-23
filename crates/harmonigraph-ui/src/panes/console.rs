//! The Console pane: the log scrollback, and the one place a refused saved
//! blob says so where a user can read it.

use crate::theme;

pub(super) fn console_pane(ui: &mut egui::Ui, runtime: &mut crate::VisualRuntime) {
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("{} held", runtime.tracker.held_count()));
        if ui.button("Clear").clicked() {
            runtime.console.clear();
        }
        let gpu = ui.button("GPU memory").on_hover_text(
            "Log every live GPU texture and buffer, largest first, and copy the table.",
        );
        if gpu.clicked() {
            let table = crate::perf::gpu_memory_table(ui.ctx());
            for line in table.lines() {
                runtime.console.log(line.to_owned());
            }
            ui.ctx().copy_text(table);
        }
    });
    // Its own area rather than the dock's, because it sticks to the bottom and
    // the Clear row above it does not scroll — so it needs the bar's lane
    // reserved out of its width (see [`theme::reserve_scroll_gutter`]).
    theme::reserve_scroll_gutter(ui);
    let to_bottom = std::mem::take(&mut runtime.console.to_bottom);
    egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        for line in runtime.console.lines() {
            ui.monospace(line);
        }
        // Sticking holds only a view already at the bottom, so a reveal
        // brings one scrolled up to its newest line.
        if to_bottom {
            ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
        }
    });
}
