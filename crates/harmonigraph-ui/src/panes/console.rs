//! The Console pane: the log scrollback, and the one place a refused saved
//! blob says so where a user can read it.

use crate::theme;
use crate::widgets::button_row;

pub(super) fn console_pane(ui: &mut egui::Ui, runtime: &mut crate::VisualRuntime) {
    button_row(ui, |ui| {
        ui.label(format!("{} held", runtime.tracker.held_count()));
        if ui.button("Clear").clicked() {
            runtime.console.clear();
        }
    });
    // Its own area rather than the dock's, because it sticks to the bottom and
    // the Clear row above it does not scroll — so it needs the bar's lane
    // reserved out of its width (see [`theme::reserve_scroll_gutter`]).
    theme::reserve_scroll_gutter(ui);
    egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        for line in runtime.console.lines() {
            ui.monospace(line);
        }
    });
}
