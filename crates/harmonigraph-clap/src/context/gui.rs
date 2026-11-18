//! A context passed to a plugin's editor.

use std::sync::Arc;

use crate::plugin::{Plugin, PluginState};

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

/// An way to run background tasks from the plugin's GUI, equivalent to the
/// [`ProcessContext::execute_background()`][crate::context::process::ProcessContext::execute_background()]
/// and [`ProcessContext::execute_gui()`][crate::context::process::ProcessContext::execute_gui()]
/// functions. This is passed directly to [`Plugin::editor()`] so the plugin can move it into its
/// editor and use it later.
///
/// # Note
///
/// This is only intended to be used from the GUI. Use the methods on
/// [`InitContext`][crate::context::init::InitContext] and
/// [`ProcessContext`][crate::context::process::ProcessContext] to run tasks during the `initialize()`
/// and `process()` functions.
//
// NOTE: This is separate from `GuiContext` because adding a type parameter there would clutter up a
//       lot of structs, and may even be incompatible with the way certain GUI libraries work.
pub struct AsyncExecutor<P: Plugin> {
    pub(crate) execute_background: Arc<dyn Fn(P::BackgroundTask) + Send + Sync>,
    pub(crate) execute_gui: Arc<dyn Fn(P::BackgroundTask) + Send + Sync>,
}

impl<P: Plugin> AsyncExecutor<P> {
    pub fn new(
        execute_background: Arc<dyn Fn(P::BackgroundTask) + Send + Sync>,
        execute_gui: Arc<dyn Fn(P::BackgroundTask) + Send + Sync>,
    ) -> Self {
        Self { execute_background, execute_gui }
    }
}

// Can't derive this since Rust then requires `P` to also be `Clone`able
impl<P: Plugin> Clone for AsyncExecutor<P> {
    fn clone(&self) -> Self {
        Self {
            execute_background: self.execute_background.clone(),
            execute_gui: self.execute_gui.clone(),
        }
    }
}

impl<P: Plugin> AsyncExecutor<P> {
    /// Execute a task on a background thread using `[Plugin::task_executor]`. This allows you to
    /// defer expensive tasks for later without blocking either the process function or the GUI
    /// thread. As long as creating the `task` is realtime-safe, this operation is too.
    ///
    /// # Note
    ///
    /// Scheduling the same task multiple times will cause those duplicate tasks to pile up. Try to
    /// either prevent this from happening, or check whether the task still needs to be completed in
    /// your task executor.
    pub fn execute_background(&self, task: P::BackgroundTask) {
        (self.execute_background)(task);
    }

    /// Execute a task on a background thread using `[Plugin::task_executor]`.
    ///
    /// # Note
    ///
    /// Scheduling the same task multiple times will cause those duplicate tasks to pile up. Try to
    /// either prevent this from happening, or check whether the task still needs to be completed in
    /// your task executor.
    pub fn execute_gui(&self, task: P::BackgroundTask) {
        (self.execute_gui)(task);
    }
}
