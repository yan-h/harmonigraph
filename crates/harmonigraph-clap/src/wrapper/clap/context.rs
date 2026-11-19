use crate::context::PluginApi;
use crate::context::gui::GuiContext;
use crate::plugin::{Plugin, PluginState};
use std::sync::Arc;

use super::wrapper::Wrapper;

/// A [`GuiContext`] implementation for the wrapper. This is passed to the plugin in
/// [`Editor::spawn()`][crate::prelude::Editor::spawn()] so it can interact with the rest of the plugin and
/// with the host for things like setting parameters.
pub(crate) struct WrapperGuiContext<P: Plugin> {
    pub(super) wrapper: Arc<Wrapper<P>>,
    #[cfg(debug_assertions)]
    pub(super) param_gesture_checker:
        atomic_refcell::AtomicRefCell<crate::wrapper::util::context_checks::ParamGestureChecker>,
}

impl<P: Plugin> GuiContext for WrapperGuiContext<P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    fn request_resize(&self) -> bool {
        self.wrapper.request_resize()
    }

    // All of these functions are supposed to be called from the main thread, so we'll put some
    // trust in the caller and assume that this is indeed the case. Each writes the table at
    // once and asks the host to flush, where the shell tells it what changed (#1814 Q1).
    fn begin_set_parameter(&self, row: usize) {
        self.wrapper.params.begin_edit(row);
        self.wrapper.request_flush();

        #[cfg(debug_assertions)]
        self.param_gesture_checker
            .borrow_mut()
            .begin_set_parameter(self.wrapper.params.spec(row).id);
    }

    fn set_parameter(&self, row: usize, plain: f32) {
        self.wrapper.params.edit(row, plain);
        self.wrapper.request_flush();

        #[cfg(debug_assertions)]
        self.param_gesture_checker.borrow_mut().set_parameter(self.wrapper.params.spec(row).id);
    }

    fn end_set_parameter(&self, row: usize) {
        self.wrapper.params.end_edit(row);
        self.wrapper.request_flush();

        #[cfg(debug_assertions)]
        self.param_gesture_checker.borrow_mut().end_set_parameter(self.wrapper.params.spec(row).id);
    }

    fn get_state(&self) -> PluginState {
        self.wrapper.get_state_object()
    }

    fn set_state(&self, state: PluginState) {
        self.wrapper.set_state_object_from_gui(state)
    }
}
