//! What the plugin's #1740 test needs of a frame without a window to run one
//! in: a look edit through the frame's undo history, and that history's depth.
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
