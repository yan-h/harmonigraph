use crate::context::PluginApi;
use crate::context::gui::GuiContext;
use crate::context::init::InitContext;
use crate::context::process::{ProcessContext, Transport};
use crate::midi::PluginNoteEvent;
use crate::plugin::PluginState;
use atomic_refcell::AtomicRefMut;
use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::Arc;

use super::wrapper::{OutputParamEvent, Task, Wrapper};
use crate::event_loop::EventLoop;
use crate::wrapper::clap::ClapPlugin;

/// An [`InitContext`] implementation for the wrapper.
///
/// # Note
///
/// Requests to change the latency are only sent when this object is dropped. Otherwise there's the
/// risk that the host will immediately deactivate/reactivate the plugin while still in the init
/// call. Reentrant function calls are difficult to handle in Rust without forcing everything to
/// use interior mutability, so this will have to do for now. This does mean that `Plugin` mutex
/// lock has to be dropped before this object.
pub(crate) struct WrapperInitContext<'a, P: ClapPlugin> {
    pub(super) wrapper: &'a Wrapper<P>,
    pub(super) pending_requests: PendingInitContextRequests,
}

/// Any requests that should be sent out when the [`WrapperInitContext`] is dropped. See that
/// struct's docstring for mroe information.
#[derive(Debug, Default)]
pub(crate) struct PendingInitContextRequests {
    /// The value of the last `.set_latency_samples()` call.
    latency_changed: Cell<Option<u32>>,
}

/// A [`ProcessContext`] implementation for the wrapper. This is a separate object so it can hold on
/// to lock guards for event queues. Otherwise reading these events would require constant
/// unnecessary atomic operations to lock the uncontested `RwLock`s.
pub(crate) struct WrapperProcessContext<'a, P: ClapPlugin> {
    pub(super) wrapper: &'a Wrapper<P>,
    pub(super) input_events_guard: AtomicRefMut<'a, VecDeque<PluginNoteEvent<P>>>,
    pub(super) output_events_guard: AtomicRefMut<'a, VecDeque<PluginNoteEvent<P>>>,
    pub(super) transport: Transport,
}

/// A [`GuiContext`] implementation for the wrapper. This is passed to the plugin in
/// [`Editor::spawn()`][crate::prelude::Editor::spawn()] so it can interact with the rest of the plugin and
/// with the host for things like setting parameters.
pub(crate) struct WrapperGuiContext<P: ClapPlugin> {
    pub(super) wrapper: Arc<Wrapper<P>>,
    #[cfg(debug_assertions)]
    pub(super) param_gesture_checker:
        atomic_refcell::AtomicRefCell<crate::wrapper::util::context_checks::ParamGestureChecker>,
}

impl<P: ClapPlugin> Drop for WrapperInitContext<'_, P> {
    fn drop(&mut self) {
        if let Some(samples) = self.pending_requests.latency_changed.take() {
            self.wrapper.set_latency_samples(samples)
        }
    }
}

impl<P: ClapPlugin> InitContext<P> for WrapperInitContext<'_, P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    fn execute(&self, task: P::BackgroundTask) {
        (self.wrapper.task_executor.lock())(task);
    }

    fn set_latency_samples(&self, samples: u32) {
        // See this struct's docstring
        self.pending_requests.latency_changed.set(Some(samples));
    }

    fn set_current_voice_capacity(&self, capacity: u32) {
        self.wrapper.set_current_voice_capacity(capacity)
    }
}

impl<P: ClapPlugin> ProcessContext<P> for WrapperProcessContext<'_, P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    fn execute_background(&self, task: P::BackgroundTask) {
        let task_posted = self.wrapper.schedule_background(Task::PluginTask(task));
        crate::nice_debug_assert!(task_posted, "The task queue is full, dropping task...");
    }

    fn execute_gui(&self, task: P::BackgroundTask) {
        let task_posted = self.wrapper.schedule_gui(Task::PluginTask(task));
        crate::nice_debug_assert!(task_posted, "The task queue is full, dropping task...");
    }

    #[inline]
    fn transport(&self) -> &Transport {
        &self.transport
    }

    fn next_event(&mut self) -> Option<PluginNoteEvent<P>> {
        self.input_events_guard.pop_front()
    }

    fn send_event(&mut self, event: PluginNoteEvent<P>) {
        if P::CLAP_PERFORMANCE {
            self.wrapper.legacy_send_misuse.store(true, std::sync::atomic::Ordering::Release);
        } else {
            self.output_events_guard.push_back(event);
        }
    }

    fn set_latency_samples(&self, samples: u32) {
        self.wrapper.set_latency_samples(samples)
    }

    fn set_current_voice_capacity(&self, capacity: u32) {
        self.wrapper.set_current_voice_capacity(capacity)
    }
}

impl<P: ClapPlugin> GuiContext for WrapperGuiContext<P> {
    fn plugin_api(&self) -> PluginApi {
        PluginApi::Clap
    }

    fn request_resize(&self) -> bool {
        self.wrapper.request_resize()
    }

    // All of these functions are supposed to be called from the main thread, so we'll put some
    // trust in the caller and assume that this is indeed the case
    fn begin_set_parameter(&self, row: usize) {
        let param_hash = self.wrapper.params.hash(row);
        let success =
            self.wrapper.queue_parameter_event(OutputParamEvent::BeginGesture { param_hash });
        crate::nice_debug_assert!(
            success,
            "Parameter output event queue was full, parameter change will not be sent to the host"
        );

        #[cfg(debug_assertions)]
        self.param_gesture_checker
            .borrow_mut()
            .begin_set_parameter(self.wrapper.params.spec(row).id);
    }

    fn set_parameter(&self, row: usize, plain: f32) {
        // We queue the parameter change event here, and it will be sent to the host either at a
        // processing/flush boundary. The owned performance path admits it at the next enclosing
        // input capture and applies it in input order; host notification retries never mutate
        // the parameter again.
        let params = &self.wrapper.params;
        let success = self.wrapper.queue_parameter_event(OutputParamEvent::SetValue {
            param_hash: params.hash(row),
            clap_plain_value: params.spec(row).to_host(plain),
        });
        crate::nice_debug_assert!(
            success,
            "Parameter output event queue was full, parameter change will not be sent to the host"
        );

        #[cfg(debug_assertions)]
        self.param_gesture_checker.borrow_mut().set_parameter(params.spec(row).id);
    }

    fn end_set_parameter(&self, row: usize) {
        let param_hash = self.wrapper.params.hash(row);
        let success =
            self.wrapper.queue_parameter_event(OutputParamEvent::EndGesture { param_hash });
        crate::nice_debug_assert!(
            success,
            "Parameter output event queue was full, parameter change will not be sent to the host"
        );

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
