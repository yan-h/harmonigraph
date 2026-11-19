//! A context passed to a plugin's editor.

use crate::plugin::PluginState;

use super::PluginApi;

/// Callbacks the plugin can make when the user interacts with its GUI such as updating parameter
/// values. This is passed to the plugin during [`Editor::spawn()`][crate::editor::Editor::spawn()].
/// All of these functions assume they're being called from the main GUI thread.
//
// # Safety
//
// The implementing wrapper can assume that everything is being called from the main thread. Since
// nice-plug doesn't own the GUI event loop, this invariant cannot be part of the interface.
pub trait GuiContext: Send + Sync + 'static {
    /// Get the current plugin API. This may be useful to display in the plugin's GUI as part of an
    /// about screen.
    fn plugin_api(&self) -> PluginApi;

    /// Ask the host to resize the editor window to the size specified by
    /// [`Editor::size()`][crate::editor::Editor::size()]. This will return false if the host
    /// somehow didn't like this and rejected the resize, in which case the window should revert to
    /// its old size. You should only actually resize your embedded window once this returns `true`.
    ///
    /// TODO: Host->Plugin resizing has not been implemented yet
    fn request_resize(&self) -> bool;

    /// Begin a gesture on the parameter table's `row`: the host records the
    /// values set until [`Self::end_set_parameter`] as one edit.
    fn begin_set_parameter(&self, row: usize);

    /// Set the parameter table's `row` to the plain value `plain` (an integer
    /// or a variant's index as a float), and tell the host.
    fn set_parameter(&self, row: usize, plain: f32);

    /// End the gesture [`Self::begin_set_parameter`] began on `row`.
    fn end_set_parameter(&self, row: usize);

    /// Serialize the plugin's current state to a serde-serializable object. Useful for implementing
    /// preset handling within a plugin's GUI.
    fn get_state(&self) -> PluginState;

    /// Restore the state from a previously serialized state object. This will block the GUI thread
    /// until the state has been restored and a parameter value rescan has been requested from the
    /// host. If the plugin is currently processing audio, then the parameter values will be
    /// restored at the end of the current processing cycle.
    fn set_state(&self, state: PluginState);
}
