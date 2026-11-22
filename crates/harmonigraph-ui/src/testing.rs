//! What the plugin's tests need of a frame without a window to run one in: a
//! look edit through the frame's undo history, that history's depth (#1740),
//! whether the Console is open, and a frame's end, which reports an edit.
//!
//! Behind the `testing` feature, which only the plugin's dev-dependencies
//! enable.

use crate::appearance_edit::Look;
use crate::SharedState;

/// One look edit, recorded in the active slot's undo history as a frame
/// records a widget's: closed at once, or left open as a drag in progress is.
pub fn edit_look(state: &mut SharedState, dragging: bool) {
    let before = Look::capture(&state.picture.appearance);
    state.picture.appearance.spectrum.attack += 0.01;
    state.workspace.interaction.appearance_editor.observe(
        before,
        &state.picture.appearance,
        dragging,
    );
}

/// How many edits the active slot can undo.
pub fn undo_depth(state: &SharedState) -> usize {
    state.workspace.interaction.appearance_editor.undo_depth()
}

/// Whether the Console is the Settings section's tab and that section is
/// unfolded, as a restore that reports a loss leaves it.
pub fn console_revealed(state: &SharedState) -> bool {
    let layout = &state.workspace.layout;
    layout.settings_tab == crate::panes::Tab::Console
        && !layout.folded[crate::workspace::Section::Settings as usize]
}

/// The end of a frame with no gesture open: what moved since the host was
/// last told is reported through [`SharedState::on_edit`].
pub fn end_frame(state: &mut SharedState) {
    state.report_edit();
}
