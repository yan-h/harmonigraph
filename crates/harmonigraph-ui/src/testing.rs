//! What the plugin's tests need of a frame without a window to run one in: a
//! look edit through the frame's undo history, that history's depth (#1740),
//! a Settings section folded off the Console, whether the Console is open, and a
//! frame's end, which reports an edit.
//!
//! Behind the `testing` feature, which only the plugin's dev-dependencies
//! enable.

use crate::appearance_edit::Look;
use crate::SharedState;

/// One look edit, recorded in the undo history as a frame records a
/// widget's: closed at once, or left open as a drag in progress is.
pub fn edit_look(state: &mut SharedState, dragging: bool) {
    let before = Look::capture(&state.picture.appearance);
    state.picture.appearance.spectrum.attack += 0.01;
    state.workspace.interaction.appearance_editor.observe(
        before,
        &state.picture.appearance,
        dragging,
    );
}

/// How many edits the history can undo.
pub fn undo_depth(state: &SharedState) -> usize {
    state.workspace.interaction.appearance_editor.undo_depth()
}

/// Install the current settings with the Settings section folded on the
/// Tuning tab, as a restore would: a save then stores that layout rather than
/// the empty document of an editor never opened.
pub fn fold_settings_on_tuning(state: &mut SharedState) {
    state.workspace.layout.select(crate::panes::Tab::Tuning);
    state.workspace.layout.folded[crate::workspace::Section::Settings as usize] = true;
    let document = state.save_persist();
    assert!(state.install_document(&document), "a fixture's own settings install");
}

/// Whether the Settings section is folded to its rail.
pub fn settings_folded(state: &SharedState) -> bool {
    state.workspace.layout.folded[crate::workspace::Section::Settings as usize]
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
