//! What the plugin's saved-state protocol tests (#1757) need of a frame
//! without a window to run one in: its plugin frame needs a window's queue.
//!
//! Behind the `testing` feature, which only the plugin's dev-dependencies
//! enable. Each hook stands for real code that a test in this crate runs:
//! [`mark_drawn`] for the mark [`root_ui`](crate::root_ui) sets past the
//! loading screen (`shell.rs`'s frame tests), [`edit_look`] for a widget's
//! look edit through the frame's undo history.

use crate::appearance_edit::Look;
use crate::SharedState;

/// What a frame that draws the editor does to what a save stores.
pub fn mark_drawn(state: &mut SharedState) {
    state.seen();
}

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
