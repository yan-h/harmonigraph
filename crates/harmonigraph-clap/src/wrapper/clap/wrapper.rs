//! The CLAP plugin vtable and its extensions, over one [`Plugin`].
//!
//! A process callback is one call into the plugin, in the order [`Plugin`]
//! states: `begin`, the shell's write of the callback's parameter events into
//! the table, `process`, `end`. Nothing splits the block: the transport is the
//! callback's own, read once at its start, and a transport event inside the
//! host's list reaches the plugin as an event at its sample. Bitwig sends
//! none (#1814); one that arrives is logged once, from the main thread.
//!
//! The main thread is reached by one host callback request, deferred past any
//! process callback in flight, and serves four flags: the setup's service,
//! a latency the running activation was asked to change, that log line, and
//! the line for a restart of the host's steady counter (#1881).

use crate::audio_setup::{Audio, BufferConfig, ProcessMode, Silence, Slices, audio};
use crate::editor::{Editor, EditorHost};
use crate::events::{self, InputValue, List, Output};
use crate::params::{Report, Table};
use crate::plugin::{Block, Boundary, Plugin, PluginState};
use atomic_refcell::AtomicRefCell;
use clap_sys::events::{clap_input_events, clap_output_events};
use clap_sys::ext::audio_ports::{
    CLAP_AUDIO_PORT_IS_MAIN, CLAP_EXT_AUDIO_PORTS, CLAP_PORT_MONO, CLAP_PORT_STEREO,
    clap_audio_port_info, clap_plugin_audio_ports,
};
use clap_sys::ext::audio_ports_config::{
    CLAP_EXT_AUDIO_PORTS_CONFIG, clap_audio_ports_config, clap_plugin_audio_ports_config,
};
use clap_sys::ext::gui::{
    CLAP_EXT_GUI, CLAP_WINDOW_API_COCOA, clap_gui_resize_hints, clap_host_gui, clap_plugin_gui,
    clap_window,
};
use clap_sys::ext::latency::{CLAP_EXT_LATENCY, clap_host_latency, clap_plugin_latency};
use clap_sys::ext::note_ports::{
    CLAP_EXT_NOTE_PORTS, CLAP_NOTE_DIALECT_CLAP, CLAP_NOTE_DIALECT_MIDI, clap_note_port_info,
    clap_plugin_note_ports,
};
use clap_sys::ext::params::{
    CLAP_EXT_PARAMS, CLAP_PARAM_IS_AUTOMATABLE, CLAP_PARAM_IS_MODULATABLE, CLAP_PARAM_IS_STEPPED,
    CLAP_PARAM_RESCAN_TEXT, CLAP_PARAM_RESCAN_VALUES, clap_host_params, clap_param_info,
    clap_plugin_params,
};
use clap_sys::ext::remote_controls::{
    CLAP_EXT_REMOTE_CONTROLS, clap_plugin_remote_controls, clap_remote_controls_page,
};
use clap_sys::ext::render::{
    CLAP_EXT_RENDER, CLAP_RENDER_OFFLINE, CLAP_RENDER_REALTIME, clap_plugin_render,
    clap_plugin_render_mode,
};
use clap_sys::ext::state::{CLAP_EXT_STATE, clap_host_state, clap_plugin_state};
use clap_sys::ext::tail::{CLAP_EXT_TAIL, clap_plugin_tail};
use clap_sys::host::clap_host;
use clap_sys::id::{CLAP_INVALID_ID, clap_id};
use clap_sys::plugin::clap_plugin;
use clap_sys::process::{
    CLAP_PROCESS_CONTINUE, CLAP_PROCESS_ERROR, clap_process, clap_process_status,
};
use clap_sys::stream::{clap_istream, clap_ostream};
use crossbeam::atomic::AtomicCell;
use parking_lot::Mutex;

#[path = "track_info_adapter.rs"]
mod track_info_adapter;
use clap_sys::ext::track_info::{
    CLAP_EXT_TRACK_INFO, CLAP_EXT_TRACK_INFO_COMPAT, clap_plugin_track_info,
};
use std::any::Any;
use std::ffi::{CStr, c_void};
use std::mem;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use super::descriptor::PluginDescriptor;
use super::setup::Setup;
use super::util::ClapPtr;
use crate::wrapper::clap::util::{read_stream, write_stream};
use crate::wrapper::state::{self};
use crate::wrapper::util::{process_wrapper, strlcpy};

/// The latency the host reads, and the one the plugin last wanted.
#[derive(Default)]
struct LatencyState {
    /// What the host reads: only an activation moves it.
    current: AtomicU32,
    pending: AtomicU32,
    /// A running activation's plugin wants a latency it does not report.
    changed: AtomicBool,
}

/// The line a transport event in the host's list leaves, once per instance.
const TRANSPORT_EVENT: &str = "The host sent a transport event inside a block. Harmonigraph reads \
     the transport once, at each block's start, so the tape, a take's timing and the analyzer may \
     lag that change by up to one block. Please report the host (#1814).";

/// The steady clock the plugin reads: the host's counter, run on across a
/// restart.
///
/// CLAP lets a host restart its steady counter at a reset. A valid callback
/// whose counter falls below where the last valid one ended starts at that
/// end instead, and the offset that takes stays on every later callback, so
/// [`Boundary::steady_time`] and every event's sample run on within an
/// activation: nothing keyed on them sees time go back (#1881). A counter that
/// runs on is never offset, and a forward jump passes through as it comes. An
/// invalid callback with a counter takes the offset too, so every boundary is
/// on one clock, but moves nothing.
///
/// A forward jump is left as it comes, so every instance stays on the host's
/// counter: Harmonigraph's presentation clock leaves the samples nobody
/// processed out of picture time itself (#1885).
///
/// Each activation starts the clock over, so across one it may go back. What a
/// plugin keys on steady time relies on `start_processing`, which calls
/// [`Plugin::reset`], to clear it before the next callback; in Harmonigraph
/// that clears the configuration history (`Owner::reset`, then
/// `AudioMaps::reset`).
///
/// Audio thread only, apart from the activation that resets it.
#[derive(Default)]
struct SteadyClock {
    /// What a restart added to the host's counter.
    offset: i64,
    /// Where the last valid callback ended, on this clock.
    next: Option<i64>,
    /// This activation has reported a restart.
    reported: bool,
}

impl SteadyClock {
    /// A valid callback's start on this clock, and, for the activation's
    /// first restart, where the host's counter should have been and where it
    /// was.
    fn run_on(&mut self, host: i64, frames: u32) -> (i64, Option<(i64, i64)>) {
        let mut start = host.saturating_add(self.offset);
        let mut restart = None;
        if let Some(next) = self.next
            && start < next
        {
            if !std::mem::replace(&mut self.reported, true) {
                restart = Some((next - self.offset, host));
            }
            self.offset += next - start;
            start = next;
        }
        self.next = Some(start.saturating_add(i64::from(frames)));
        (start, restart)
    }

    /// An invalid callback's counter, on this clock without moving it, or as
    /// given when it has none.
    fn invalid(&self, host: i64) -> i64 {
        if host >= 0 { host.saturating_add(self.offset) } else { host }
    }
}

/// A restart for the main thread's line, with where the counter should have
/// been and where it was.
#[derive(Default)]
struct RestartNotice {
    due: AtomicBool,
    expected: AtomicI64,
    given: AtomicI64,
}

pub struct Wrapper<P: Plugin> {
    setup: Option<Arc<dyn Setup>>,
    setup_pending: AtomicBool,
    /// A flush's input list failed validation: the next process callback's
    /// input is lost, as its own invalid list would be.
    flush_fault: AtomicBool,
    /// A process callback saw a transport event in the host's list.
    transport_event: AtomicBool,
    /// [`TRANSPORT_EVENT`] has been logged.
    transport_logged: AtomicBool,
    steady: AtomicRefCell<SteadyClock>,
    /// A process callback found the host's steady counter restarted, for the
    /// main thread's line: once an activation.
    steady_restart: RestartNotice,
    /// A process callback is running, so a main-thread request waits for its
    /// end rather than calling the host from inside it.
    audio: AtomicBool,
    deferred_host_callback: AtomicBool,
    host_state: AtomicRefCell<Option<ClapPtr<clap_host_state>>>,

    /// The wrapped plugin instance.
    plugin: Mutex<P>,
    /// The plugin's host parameters, fetched once at creation.
    pub(crate) params: Arc<Table>,
    /// The plugin's editor, if it has one, made once with the plugin so the
    /// host's `gui` calls never take the plugin's lock.
    editor: OnceLock<Box<dyn Editor>>,
    /// The attached editor, which closes when dropped. Locked only to read or
    /// swap it, never across the editor's open or close: either can call the
    /// host, or draw a frame on the spot.
    editor_handle: Mutex<Option<Box<dyn Any + Send>>>,

    is_activated: AtomicBool,
    /// The current buffer configuration, containing the sample rate and the maximum block size.
    /// Will be set in `clap_plugin::activate()`.
    current_buffer_config: AtomicCell<Option<BufferConfig>>,
    /// The current audio processing mode. Set through the render extension. Defaults to realtime.
    pub current_process_mode: AtomicCell<ProcessMode>,
    /// A callback has been processed since processing started. The plugin
    /// keeps the host running whatever its input, so its tail is then
    /// infinite; before, there is none.
    processed: AtomicBool,
    /// The latency the host reads, and the one the plugin asked for. CLAP
    /// lets the reported latency change only across an activation, so a
    /// request an active plugin makes waits there, and the host keeps reading
    /// the latency this activation runs, until the next one adopts it (#615).
    latency: LatencyState,
    /// What a callback reads for an auxiliary port or channel the host left
    /// out, sized at activation.
    aux_silence: AtomicRefCell<Silence>,

    // We'll query all of the host's extensions upfront
    host_callback: ClapPtr<clap_host>,

    clap_plugin_audio_ports_config: clap_plugin_audio_ports_config,

    // The main `clap_plugin` vtable. A pointer to this `Wrapper<P>` instance is stored in the
    // `plugin_data` field. This pointer is set after creating the `Arc<Wrapper<P>>`.
    pub clap_plugin: AtomicRefCell<clap_plugin>,
    /// Needs to be boxed because the plugin object is supposed to contain a static reference to
    /// this.
    _plugin_descriptor: Box<PluginDescriptor>,

    clap_plugin_audio_ports: clap_plugin_audio_ports,

    clap_plugin_gui: clap_plugin_gui,
    host_gui: AtomicRefCell<Option<ClapPtr<clap_host_gui>>>,

    clap_plugin_latency: clap_plugin_latency,
    host_latency: AtomicRefCell<Option<ClapPtr<clap_host_latency>>>,

    clap_plugin_track_info: clap_plugin_track_info,
    clap_plugin_note_ports: clap_plugin_note_ports,

    clap_plugin_params: clap_plugin_params,
    host_params: AtomicRefCell<Option<ClapPtr<clap_host_params>>>,

    clap_plugin_remote_controls: clap_plugin_remote_controls,

    clap_plugin_render: clap_plugin_render,

    clap_plugin_state: clap_plugin_state,

    clap_plugin_tail: clap_plugin_tail,
}

impl<P: Plugin> Wrapper<P> {
    #[cfg(feature = "clap-boundary-tests")]
    #[doc(hidden)]
    pub fn test_inspect_plugin<R>(&self, inspect: impl FnOnce(&P) -> R) -> R {
        // Fixtures call this outside process, after the serialized callback
        // joined. It observes the production owner without replacing its path.
        inspect(&self.plugin.lock())
    }

    /// # Safety
    ///
    /// `host_callback` needs to outlive the returned object.
    pub unsafe fn new(host_callback: *const clap_host) -> Arc<Self> {
        P::PORTS.check();
        let plugin = P::default();
        let plugin_descriptor: Box<PluginDescriptor> =
            Box::new(PluginDescriptor::for_plugin::<P>());

        // We're not allowed to query any extensions until the init function has been called, so we
        // need a bunch of AtomicRefCells instead
        assert!(!host_callback.is_null());
        let host_callback = unsafe { ClapPtr::new(host_callback) };

        let wrapper = Self {
            setup: plugin.setup(),
            setup_pending: AtomicBool::new(false),
            flush_fault: AtomicBool::new(false),
            transport_event: AtomicBool::new(false),
            transport_logged: AtomicBool::new(false),
            steady: AtomicRefCell::new(SteadyClock::default()),
            steady_restart: Default::default(),
            audio: AtomicBool::new(false),
            deferred_host_callback: AtomicBool::new(false),
            host_state: AtomicRefCell::new(None),

            params: plugin.params(),
            plugin: Mutex::new(plugin),
            // Set below, once the plugin has its `EditorHost`, which needs
            // the wrapper's `Arc`.
            editor: OnceLock::new(),
            editor_handle: Mutex::new(None),

            is_activated: AtomicBool::new(false),
            current_buffer_config: AtomicCell::new(None),
            current_process_mode: AtomicCell::new(ProcessMode::Realtime),
            processed: AtomicBool::new(false),
            latency: LatencyState::default(),
            // Sized at activation, so a callback allocates nothing.
            aux_silence: AtomicRefCell::new(Silence::new(0)),

            host_callback,

            clap_plugin: AtomicRefCell::new(clap_plugin {
                // This needs to live on the heap because the plugin object contains a direct
                // reference to the manifest as a value. We could share this between instances of
                // the plugin using an `Arc`, but this doesn't consume a lot of memory so it's not a
                // huge deal.
                desc: plugin_descriptor.clap_plugin_descriptor(),
                // This pointer will be set to point at our wrapper instance later
                plugin_data: std::ptr::null_mut(),
                init: Some(Self::init),
                destroy: Some(Self::destroy),
                activate: Some(Self::activate),
                deactivate: Some(Self::deactivate),
                start_processing: Some(Self::start_processing),
                stop_processing: Some(Self::stop_processing),
                reset: Some(Self::reset),
                process: Some(Self::process),
                get_extension: Some(Self::get_extension),
                on_main_thread: Some(Self::on_main_thread),
            }),
            _plugin_descriptor: plugin_descriptor,

            clap_plugin_audio_ports_config: clap_plugin_audio_ports_config {
                count: Some(Self::ext_audio_ports_config_count),
                get: Some(Self::ext_audio_ports_config_get),
                select: Some(Self::ext_audio_ports_config_select),
            },

            clap_plugin_audio_ports: clap_plugin_audio_ports {
                count: Some(Self::ext_audio_ports_count),
                get: Some(Self::ext_audio_ports_get),
            },

            clap_plugin_gui: clap_plugin_gui {
                is_api_supported: Some(Self::ext_gui_is_api_supported),
                get_preferred_api: Some(Self::ext_gui_get_preferred_api),
                create: Some(Self::ext_gui_create),
                destroy: Some(Self::ext_gui_destroy),
                set_scale: Some(Self::ext_gui_set_scale),
                get_size: Some(Self::ext_gui_get_size),
                can_resize: Some(Self::ext_gui_can_resize),
                get_resize_hints: Some(Self::ext_gui_get_resize_hints),
                adjust_size: Some(Self::ext_gui_adjust_size),
                set_size: Some(Self::ext_gui_set_size),
                set_parent: Some(Self::ext_gui_set_parent),
                set_transient: Some(Self::ext_gui_set_transient),
                suggest_title: Some(Self::ext_gui_suggest_title),
                show: Some(Self::ext_gui_show),
                hide: Some(Self::ext_gui_hide),
            },
            host_gui: AtomicRefCell::new(None),

            clap_plugin_latency: clap_plugin_latency { get: Some(Self::ext_latency_get) },
            host_latency: AtomicRefCell::new(None),

            clap_plugin_track_info: clap_plugin_track_info {
                changed: Some(Self::ext_track_info_changed),
            },
            clap_plugin_note_ports: clap_plugin_note_ports {
                count: Some(Self::ext_note_ports_count),
                get: Some(Self::ext_note_ports_get),
            },

            clap_plugin_params: clap_plugin_params {
                count: Some(Self::ext_params_count),
                get_info: Some(Self::ext_params_get_info),
                get_value: Some(Self::ext_params_get_value),
                value_to_text: Some(Self::ext_params_value_to_text),
                text_to_value: Some(Self::ext_params_text_to_value),
                flush: Some(Self::ext_params_flush),
            },
            host_params: AtomicRefCell::new(None),

            clap_plugin_remote_controls: clap_plugin_remote_controls {
                count: Some(Self::ext_remote_controls_count),
                get: Some(Self::ext_remote_controls_get),
            },

            clap_plugin_render: clap_plugin_render {
                has_hard_realtime_requirement: Some(Self::ext_render_has_hard_realtime_requirement),
                set: Some(Self::ext_render_set),
            },

            clap_plugin_state: clap_plugin_state {
                save: Some(Self::ext_state_save),
                load: Some(Self::ext_state_load),
            },

            clap_plugin_tail: clap_plugin_tail { get: Some(Self::ext_tail_get) },
        };

        let wrapper = Arc::new(wrapper);
        if let Some(setup) = &wrapper.setup {
            let weak = Arc::downgrade(&wrapper);
            setup.install_wakeup(Box::new(move || {
                if let Some(wrapper) = weak.upgrade() {
                    wrapper.wake_setup();
                }
            }));
        }

        // The `clap_plugin::plugin_data` field needs to point to this wrapper so we can access it
        // from the vtable functions
        wrapper.clap_plugin.borrow_mut().plugin_data = Arc::as_ptr(&wrapper) as *mut _;

        let mut plugin = wrapper.plugin.lock();
        plugin.editor_host(Arc::new(HostGui(Arc::downgrade(&wrapper))));
        if let Some(editor) = plugin.editor() {
            let _ = wrapper.editor.set(editor);
        }
        drop(plugin);

        wrapper
    }

    /// Ask the host for a flush, for it to hear what the editor did. Safe
    /// from any thread but the audio one, where a flush is no use.
    fn request_flush(&self) {
        match &*self.host_params.borrow() {
            Some(host_params) => {
                unsafe_clap_call! { host_params=>request_flush(&*self.host_callback) }
            }
            None => {
                crate::shell_debug_assert_failure!("The host does not support parameters? What?")
            }
        }
    }

    /// The state a save stores: the parameters, then whatever the setup writes.
    pub fn get_state_object(&self) -> PluginState {
        let mut state = PluginState {
            version: String::from(P::VERSION),
            params: self.params.save(),
            fields: Default::default(),
        };
        if let Some(setup) = &self.setup {
            setup.save(&mut state);
        }
        state
    }

    /// Restore `state` as a load from the host does, without its framing:
    /// whether the setup took it.
    #[cfg(feature = "clap-boundary-tests")]
    #[doc(hidden)]
    pub fn test_restore(&self, state: &PluginState) -> bool {
        self.restore(state)
    }

    /// Ask the host for a main-thread call. Inside a process callback the
    /// request waits for the callback's end, where the audio thread makes it;
    /// a request racing that end either lands before the final swap, which
    /// then makes it, or sees the completed callback and makes it itself.
    fn wake_setup(&self) {
        self.setup_pending.store(true, Ordering::Release);
        if self.audio.load(Ordering::Acquire) {
            self.deferred_host_callback.swap(true, Ordering::AcqRel);
            if self.audio.load(Ordering::Acquire) {
                return;
            }
        }
        // No plugin/runtime borrow or setup/registry mutex is held by callers.
        let host = &self.host_callback;
        unsafe_clap_call! { host=>request_callback(&**host) };
    }

    /// The one restore ([`setup::restore`](super::setup::restore)), from the
    /// main thread, even while audio runs.
    fn restore(&self, state: &PluginState) -> bool {
        super::setup::restore(self.setup.as_deref(), &self.params, state)
            .inspect_err(|reason| crate::shell_error!("Prepared setup restore refused: {reason}"))
            .is_ok()
    }

    /// Move the number the host reads to the one the plugin asked for, and
    /// tell the host whenever that moves it. CLAP accepts `changed()` during
    /// `activate` and nowhere else, so an activation is the only caller and
    /// the only writer of what the host reads: a deactivated plugin keeps
    /// reporting the delay its host is still compensating for until the next
    /// activation adopts a replacement (#615).
    fn publish_pending_latency(&self) {
        let pending = self.latency.pending.load(Ordering::SeqCst);
        if self.latency.current.swap(pending, Ordering::SeqCst) != pending {
            if let Some(host_latency) = &*self.host_latency.borrow() {
                unsafe_clap_call! { host_latency=>changed(&*self.host_callback) };
            }
        }
    }

    /// Tell the host what the editor did, at offset 0, stopping at the
    /// first refusal.
    ///
    /// # Safety
    /// `out` must be null or a valid host output list.
    unsafe fn report(&self, out: *const clap_output_events) {
        let mut output = unsafe { Output::new(out, true) };
        self.params.report(|id, report| match report {
            Report::Begin => output.push_gesture(id, true, 0),
            Report::Value(value) => {
                output.push(InputValue::Parameter { id, value, modulation: false }, 0)
            }
            Report::End => output.push_gesture(id, false, 0),
        });
    }

    /// Apply every parameter event of a validated list to the table.
    fn write_parameters(&self, list: List<'_>) {
        for (_, value) in list.values() {
            if let InputValue::Parameter { id, value, modulation } = value {
                self.params.apply(id, value, modulation);
            }
        }
    }

    unsafe extern "C" fn init(plugin: *const clap_plugin) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        // We weren't allowed to query these in the constructor, so we need to do it now instead.
        unsafe {
            *wrapper.host_gui.borrow_mut() =
                query_host_extension::<clap_host_gui>(&wrapper.host_callback, CLAP_EXT_GUI);
            *wrapper.host_latency.borrow_mut() =
                query_host_extension::<clap_host_latency>(&wrapper.host_callback, CLAP_EXT_LATENCY);
            *wrapper.host_params.borrow_mut() =
                query_host_extension::<clap_host_params>(&wrapper.host_callback, CLAP_EXT_PARAMS);
            if wrapper.setup.is_some() {
                *wrapper.host_state.borrow_mut() =
                    query_host_extension::<clap_host_state>(&wrapper.host_callback, CLAP_EXT_STATE);
            }
        }

        wrapper.refresh_track_name();
        wrapper.plugin.lock().init()
    }

    unsafe extern "C" fn destroy(plugin: *const clap_plugin) {
        assert!(!plugin.is_null() && unsafe { !(*plugin).plugin_data.is_null() });
        let this = unsafe { Arc::from_raw((*plugin).plugin_data as *mut Self) };
        crate::shell_debug_assert_eq!(Arc::strong_count(&this), 1);

        this.plugin.lock().destroy();
        drop(this);
    }

    unsafe extern "C" fn activate(
        plugin: *const clap_plugin,
        sample_rate: f64,
        _min_frames_count: u32,
        max_frames_count: u32,
    ) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        let buffer_config = BufferConfig {
            sample_rate: sample_rate as f32,
            max_buffer_size: max_frames_count,
            process_mode: wrapper.current_process_mode.load(),
        };

        let mut plugin = wrapper.plugin.lock();
        plugin.inactive();
        if !plugin.activate(&buffer_config) {
            return false;
        }
        // Not active yet, so the latency wanted now waits for nothing: it is
        // what this activation publishes below, while the host still has the
        // activation in progress, and it asks for no restart (#615).
        wrapper.latency.pending.store(plugin.latency(), Ordering::SeqCst);
        // Release the plugin lock before host callbacks.
        drop(plugin);

        *wrapper.aux_silence.borrow_mut() = Silence::new(max_frames_count);
        *wrapper.steady.borrow_mut() = SteadyClock::default();
        wrapper.current_buffer_config.store(Some(buffer_config));
        // The one place the reported latency moves and the one place the host
        // is told it has. Whatever this activation adopts -- requested during
        // it or while the previous activation ran -- becomes the number the
        // host reads, here.
        wrapper.publish_pending_latency();
        wrapper.is_activated.store(true, Ordering::SeqCst);
        true
    }

    unsafe extern "C" fn deactivate(plugin: *const clap_plugin) {
        check_null_ptr!((), plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        wrapper.is_activated.store(false, Ordering::SeqCst);
        wrapper.plugin.lock().inactive();
    }

    unsafe extern "C" fn start_processing(plugin: *const clap_plugin) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        // A new processing run starts without a tail from the previous run.
        wrapper.processed.store(false, Ordering::Release);
        process_wrapper(|| {
            let mut plugin = wrapper.plugin.lock();
            plugin.reset();
        });
        true
    }

    unsafe extern "C" fn stop_processing(plugin: *const clap_plugin) {
        check_null_ptr!((), plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        process_wrapper(|| wrapper.plugin.lock().stop());
    }

    unsafe extern "C" fn reset(plugin: *const clap_plugin) {
        check_null_ptr!((), plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        process_wrapper(|| wrapper.plugin.lock().reset());
    }

    unsafe extern "C" fn process(
        plugin: *const clap_plugin,
        process: *const clap_process,
    ) -> clap_process_status {
        check_null_ptr!(CLAP_PROCESS_ERROR, plugin, unsafe { (*plugin).plugin_data }, process);
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        // Panic on allocations if the `assert_process_allocs` feature has been enabled, and make
        // sure that FTZ is set up correctly
        process_wrapper(|| {
            wrapper.audio.store(true, Ordering::Release);
            let status = unsafe { wrapper.process_callback(&*process) };
            wrapper.audio.store(false, Ordering::Release);
            if wrapper.deferred_host_callback.swap(false, Ordering::AcqRel) {
                let host = &wrapper.host_callback;
                unsafe_clap_call! { host=>request_callback(&**host) };
            }
            status
        })
    }

    /// One process callback, in [`Plugin`]'s order.
    ///
    /// # Safety
    /// `process` and everything it points to must be valid for the call.
    unsafe fn process_callback(&self, process: &clap_process) -> clap_process_status {
        let frames = process.frames_count;
        let config = self.current_buffer_config.load();
        let valid = process.steady_time >= 0
            && frames > 0
            && process.steady_time.checked_add(i64::from(frames)).is_some()
            && config.is_some_and(|config| frames <= config.max_buffer_size);
        let checked = if valid {
            unsafe { events::validate(process.in_events, Some(frames)) }
        } else {
            Err(())
        };
        // A flush fault is the next callback's, whatever that callback brings.
        let flush_fault = self.flush_fault.swap(false, Ordering::AcqRel);
        let input_lost = checked.is_err() || flush_fault;
        let list = match checked {
            Ok(checked) if !input_lost => {
                if checked.transport && !self.transport_logged.load(Ordering::Relaxed) {
                    self.transport_event.store(true, Ordering::Release);
                    self.deferred_host_callback.store(true, Ordering::Release);
                }
                checked.list
            }
            _ => List::EMPTY,
        };
        let steady_time = if valid {
            let (start, restart) = self.steady.borrow_mut().run_on(process.steady_time, frames);
            if let Some((expected, given)) = restart {
                let notice = &self.steady_restart;
                notice.expected.store(expected, Ordering::Relaxed);
                notice.given.store(given, Ordering::Relaxed);
                notice.due.store(true, Ordering::Release);
                self.deferred_host_callback.store(true, Ordering::Release);
            }
            start
        } else {
            self.steady.borrow().invalid(process.steady_time)
        };
        let boundary = Boundary {
            steady_time,
            frames,
            transport: unsafe { process.transport.as_ref() }.map(|&transport| transport.into()),
            valid,
            input_lost,
        };

        // What the editor did, at offset 0, ahead of anything the plugin
        // pushes. On the real list even when the input is lost: a host that
        // discards the audio still keeps its parameter values.
        unsafe { self.report(process.out_events) };

        let mut plugin = self.plugin.lock();
        plugin.begin(&boundary);
        // After `begin`: see `Plugin`'s ordering rule.
        self.write_parameters(list);
        if valid && input_lost {
            // A lost callback's audio is left as the host gave it.
            plugin.process(&mut unsafe { self.block(boundary, list, process, None) });
        } else if valid {
            let silence = self.aux_silence.borrow();
            let mut slices = Slices::default();
            let audio = unsafe { audio(&P::PORTS, process, &silence, &mut slices) };
            plugin.process(&mut unsafe { self.block(boundary, list, process, audio) });
            self.poll_latency(&plugin);
        }
        plugin.end();
        drop(plugin);

        if input_lost {
            CLAP_PROCESS_ERROR
        } else {
            self.processed.store(true, Ordering::Release);
            CLAP_PROCESS_CONTINUE
        }
    }

    /// The plugin's view of one callback around `audio`. Its output refuses
    /// everything when the input is lost.
    ///
    /// # Safety
    /// `process`'s output list must be valid for `'a`.
    unsafe fn block<'a>(
        &'a self,
        boundary: Boundary,
        events: List<'a>,
        process: &'a clap_process,
        audio: Option<Audio<'a>>,
    ) -> Block<'a> {
        Block {
            boundary,
            events,
            output: unsafe { Output::new(process.out_events, !boundary.input_lost) },
            audio,
        }
    }

    /// Read the latency an active plugin wants after its `process`: a change
    /// waits for a restart, which the main thread asks for (#615).
    fn poll_latency(&self, plugin: &P) {
        let wanted = plugin.latency();
        if self.latency.pending.swap(wanted, Ordering::SeqCst) != wanted {
            self.latency.changed.store(true, Ordering::Release);
            self.deferred_host_callback.store(true, Ordering::Release);
        }
    }

    unsafe extern "C" fn get_extension(
        plugin: *const clap_plugin,
        id: *const c_char,
    ) -> *const c_void {
        check_null_ptr!(std::ptr::null(), plugin, unsafe { (*plugin).plugin_data }, id);
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        let id = unsafe { CStr::from_ptr(id) };

        if id == CLAP_EXT_AUDIO_PORTS_CONFIG {
            &wrapper.clap_plugin_audio_ports_config as *const _ as *const c_void
        } else if id == CLAP_EXT_AUDIO_PORTS {
            &wrapper.clap_plugin_audio_ports as *const _ as *const c_void
        } else if id == CLAP_EXT_GUI && wrapper.editor.get().is_some() {
            // Only report that we support this extension if the plugin has an editor
            &wrapper.clap_plugin_gui as *const _ as *const c_void
        } else if id == CLAP_EXT_LATENCY {
            &wrapper.clap_plugin_latency as *const _ as *const c_void
        } else if id == CLAP_EXT_NOTE_PORTS && P::NOTE_PORTS {
            &wrapper.clap_plugin_note_ports as *const _ as *const c_void
        } else if (id == CLAP_EXT_TRACK_INFO || id == CLAP_EXT_TRACK_INFO_COMPAT)
            && wrapper.setup.is_some()
        {
            &wrapper.clap_plugin_track_info as *const _ as *const c_void
        } else if id == CLAP_EXT_PARAMS {
            &wrapper.clap_plugin_params as *const _ as *const c_void
        } else if id == CLAP_EXT_REMOTE_CONTROLS {
            &wrapper.clap_plugin_remote_controls as *const _ as *const c_void
        } else if id == CLAP_EXT_RENDER {
            &wrapper.clap_plugin_render as *const _ as *const c_void
        } else if id == CLAP_EXT_STATE {
            &wrapper.clap_plugin_state as *const _ as *const c_void
        } else if id == CLAP_EXT_TAIL {
            &wrapper.clap_plugin_tail as *const _ as *const c_void
        } else {
            crate::shell_trace!("Host tried to query unknown extension {:?}", id);
            std::ptr::null()
        }
    }

    unsafe extern "C" fn on_main_thread(plugin: *const clap_plugin) {
        check_null_ptr!((), plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        let activated = wrapper.is_activated.load(Ordering::SeqCst);
        if !activated {
            wrapper.plugin.lock().inactive();
        }
        if wrapper.setup_pending.swap(false, Ordering::AcqRel) {
            let serviced = wrapper.setup.as_ref().map(|setup| setup.service()).unwrap_or_default();
            let rescan = if serviced.values { CLAP_PARAM_RESCAN_VALUES } else { 0 }
                | if serviced.text { CLAP_PARAM_RESCAN_TEXT } else { 0 };
            if rescan != 0 {
                if let Some(params) = wrapper.host_params.borrow().as_ref() {
                    unsafe_clap_call! { params=>rescan(&*wrapper.host_callback, rescan) };
                }
            }
            if serviced.modified {
                if let Some(state) = wrapper.host_state.borrow().as_ref() {
                    unsafe_clap_call! { state=>mark_dirty(&*wrapper.host_callback) };
                }
            }
        }
        // The latency may only change across an activation, so all this can
        // do is ask for one. A deactivated plugin is between activations
        // already and has nothing to restart; an active one whose request an
        // activation has since adopted has nothing left to ask for either.
        if wrapper.latency.changed.swap(false, Ordering::AcqRel)
            && activated
            && wrapper.latency.pending.load(Ordering::SeqCst)
                != wrapper.latency.current.load(Ordering::SeqCst)
        {
            if wrapper.host_latency.borrow().is_some() {
                unsafe_clap_call! { &*wrapper.host_callback=>request_restart(&*wrapper.host_callback) };
            } else {
                crate::shell_debug_assert_failure!("Host does not support the latency extension");
            }
        }
        if wrapper.transport_event.swap(false, Ordering::AcqRel)
            && !wrapper.transport_logged.swap(true, Ordering::AcqRel)
        {
            crate::shell_warn!("{TRANSPORT_EVENT}");
            if let Some(setup) = &wrapper.setup {
                setup.notice(TRANSPORT_EVENT);
            }
        }
        let notice = &wrapper.steady_restart;
        if notice.due.swap(false, Ordering::AcqRel) {
            let (given, expected) =
                (notice.given.load(Ordering::Relaxed), notice.expected.load(Ordering::Relaxed));
            let line = format!(
                "The host restarted its steady sample counter: a block began at {given} where \
                 {expected} was due. This instance runs its clock on from where it was. Where \
                 another Harmonigraph or Tune did not see the same restart, note timing across \
                 them may be off until the plugin is reloaded. Said once per activation (#1881)."
            );
            crate::shell_warn!("{line}");
            if let Some(setup) = &wrapper.setup {
                setup.notice(&line);
            }
        }
    }

    /// One layout, which the host cannot change; the extension stays
    /// answered, since whether Bitwig reads its absence differently is
    /// unmeasured.
    unsafe extern "C" fn ext_audio_ports_config_count(plugin: *const clap_plugin) -> u32 {
        check_null_ptr!(0, plugin, unsafe { (*plugin).plugin_data });

        1
    }

    unsafe extern "C" fn ext_audio_ports_config_get(
        plugin: *const clap_plugin,
        index: u32,
        config: *mut clap_audio_ports_config,
    ) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data }, config);
        if index != 0 {
            crate::shell_debug_assert_failure!(
                "Host tried to query out of bounds audio port config {}",
                index
            );
            return false;
        }

        let ports = P::PORTS;
        unsafe { *config = std::mem::zeroed() };
        let config = unsafe { &mut *config };
        config.id = 0;
        strlcpy(&mut config.name, ports.name());
        config.input_port_count = ports.inputs();
        config.output_port_count = ports.outputs();
        config.has_main_input = ports.main_input > 0;
        config.main_input_channel_count = ports.main_input;
        config.main_input_port_type = port_type(ports.main_input);
        config.has_main_output = ports.main_output > 0;
        config.main_output_channel_count = ports.main_output;
        config.main_output_port_type = port_type(ports.main_output);
        true
    }

    unsafe extern "C" fn ext_audio_ports_config_select(
        plugin: *const clap_plugin,
        config_id: clap_id,
    ) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data });

        config_id == 0
    }

    unsafe extern "C" fn ext_audio_ports_count(plugin: *const clap_plugin, is_input: bool) -> u32 {
        check_null_ptr!(0, plugin, unsafe { (*plugin).plugin_data });

        if is_input { P::PORTS.inputs() } else { P::PORTS.outputs() }
    }

    /// Inputs are numbered from 0, the main one first, and outputs after
    /// them; the main input and output are an in-place pair.
    unsafe extern "C" fn ext_audio_ports_get(
        plugin: *const clap_plugin,
        index: u32,
        is_input: bool,
        info: *mut clap_audio_port_info,
    ) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data }, info);

        let ports = P::PORTS;
        let (has_main_input, has_main_output) = (ports.main_input > 0, ports.main_output > 0);
        let main = index == 0 && if is_input { has_main_input } else { has_main_output };
        let (name, channels) = match (is_input, main) {
            (true, true) => ("Input", ports.main_input),
            (false, true) => ("Output", ports.main_output),
            (true, false) => {
                match ports.aux_inputs.get((index - u32::from(has_main_input)) as usize) {
                    Some(&(channels, name)) => (name, channels),
                    None => return false,
                }
            }
            (false, false) => return false,
        };
        unsafe { *info = std::mem::zeroed() };
        let info = unsafe { &mut *info };
        info.id = if is_input { index } else { index + ports.inputs() };
        strlcpy(&mut info.name, name);
        info.flags = if main { CLAP_AUDIO_PORT_IS_MAIN } else { 0 };
        info.channel_count = channels;
        info.port_type = port_type(channels);
        info.in_place_pair = match (is_input, main) {
            (true, true) if has_main_output => ports.inputs(),
            (false, true) if has_main_input => 0,
            _ => CLAP_INVALID_ID,
        };
        true
    }

    unsafe extern "C" fn ext_gui_is_api_supported(
        _plugin: *const clap_plugin,
        api: *const c_char,
        is_floating: bool,
    ) -> bool {
        // Embedded in an AppKit view, on every platform the shell builds on,
        // so a host can only create the editor where it can attach it.
        !is_floating && unsafe { CStr::from_ptr(api) } == CLAP_WINDOW_API_COCOA
    }

    unsafe extern "C" fn ext_gui_get_preferred_api(
        _plugin: *const clap_plugin,
        api: *mut *const c_char,
        is_floating: *mut bool,
    ) -> bool {
        check_null_ptr!(false, api, is_floating);

        unsafe {
            *api = CLAP_WINDOW_API_COCOA.as_ptr();
            *is_floating = false;
        }

        true
    }

    unsafe extern "C" fn ext_gui_create(
        plugin: *const clap_plugin,
        api: *const c_char,
        is_floating: bool,
    ) -> bool {
        // Double check this in case the host didn't
        if unsafe { !Self::ext_gui_is_api_supported(plugin, api, is_floating) } {
            return false;
        }

        // In CLAP creating the editor window and embedding it in another window are separate, and
        // those things are one and the same in our framework. So we'll just pretend we did
        // something here.
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        let editor_handle = wrapper.editor_handle.lock();
        if editor_handle.is_none() {
            true
        } else {
            crate::shell_debug_assert_failure!(
                "Tried creating editor while the editor was already active"
            );
            false
        }
    }

    unsafe extern "C" fn ext_gui_destroy(plugin: *const clap_plugin) {
        check_null_ptr!((), plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        // Out of the lock before it closes: the editor's close ends its
        // gestures with the host.
        let editor_handle = wrapper.editor_handle.lock().take();
        if editor_handle.is_none() {
            crate::shell_debug_assert_failure!(
                "Tried destroying editor while the editor was not active"
            );
        }
        drop(editor_handle);
    }

    /// Refused: the editor's sizes are logical points, which the OS scales.
    unsafe extern "C" fn ext_gui_set_scale(_plugin: *const clap_plugin, _scale: f64) -> bool {
        false
    }

    unsafe extern "C" fn ext_gui_get_size(
        plugin: *const clap_plugin,
        width: *mut u32,
        height: *mut u32,
    ) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data }, width, height);
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        let Some(editor) = wrapper.editor.get() else { return false };
        unsafe { (*width, *height) = editor.size() };
        true
    }

    unsafe extern "C" fn ext_gui_can_resize(plugin: *const clap_plugin) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        wrapper.editor.get().is_some_and(|editor| editor.resizable())
    }

    /// A resizable editor resizes freely on both axes.
    unsafe extern "C" fn ext_gui_get_resize_hints(
        plugin: *const clap_plugin,
        hints: *mut clap_gui_resize_hints,
    ) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data }, hints);
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        if !wrapper.editor.get().is_some_and(|editor| editor.resizable()) {
            return false;
        }
        unsafe {
            *hints = clap_gui_resize_hints {
                can_resize_horizontally: true,
                can_resize_vertically: true,
                preserve_aspect_ratio: false,
                aspect_ratio_width: 1,
                aspect_ratio_height: 1,
            };
        }
        true
    }

    /// The size the editor would take nearest the one the host proposes,
    /// written back in place: a border drag stops at the editor's limits.
    unsafe extern "C" fn ext_gui_adjust_size(
        plugin: *const clap_plugin,
        width: *mut u32,
        height: *mut u32,
    ) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data }, width, height);
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        let Some(editor) = wrapper.editor.get() else { return false };
        unsafe { (*width, *height) = editor.adjust_size(*width, *height) };
        true
    }

    unsafe extern "C" fn ext_gui_set_size(
        plugin: *const clap_plugin,
        width: u32,
        height: u32,
    ) -> bool {
        // The host calls this after honoring an earlier `request_resize()`, when
        // the user drags a host-drawn resize handle, and (on Linux) if an
        // asynchronous resize request fails.
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        wrapper.editor.get().is_some_and(|editor| editor.set_size(width, height))
    }

    /// Opens the editor under an AppKit view, the one window API the shell
    /// offers; any other is refused.
    unsafe extern "C" fn ext_gui_set_parent(
        plugin: *const clap_plugin,
        window: *const clap_window,
    ) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data }, window);
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };
        let window = unsafe { &*window };

        if unsafe { CStr::from_ptr(window.api) } != CLAP_WINDOW_API_COCOA {
            return false;
        }
        // This extension is only exposed when there is an editor.
        let Some(editor) = wrapper.editor.get() else { return false };
        if wrapper.editor_handle.lock().is_some() {
            crate::shell_debug_assert_failure!(
                "Host tried to attach editor while the editor is already attached"
            );
            return false;
        }
        // Out of the lock: opening can draw a frame on the spot, which can
        // ask the host for a resize.
        // SAFETY: CLAP calls `set_parent` on the main thread, and a `cocoa`
        // window carries the host's `NSView`, alive while the editor is
        // attached to it.
        let handle = unsafe { editor.spawn(window.specific.cocoa) };
        *wrapper.editor_handle.lock() = Some(handle);
        true
    }

    unsafe extern "C" fn ext_gui_set_transient(
        _plugin: *const clap_plugin,
        _window: *const clap_window,
    ) -> bool {
        // This is only relevant for floating windows
        false
    }

    unsafe extern "C" fn ext_gui_suggest_title(_plugin: *const clap_plugin, _title: *const c_char) {
        // This is only relevant for floating windows
    }

    /// Refused, as is [`Self::ext_gui_hide`]. Bitwig calls the two for an
    /// embedded editor on every switch of the frontmost app: `hide` within
    /// milliseconds of Bitwig losing focus, `show` as it regains it
    /// (measured in Bitwig 6.1.6). An editor that answered them by hiding a
    /// window of its own would vanish whenever another app came forward,
    /// which is what the editor's own window exists to survive (#1580);
    /// #1607's answering did exactly that.
    unsafe extern "C" fn ext_gui_show(_plugin: *const clap_plugin) -> bool {
        false
    }

    unsafe extern "C" fn ext_gui_hide(_plugin: *const clap_plugin) -> bool {
        false
    }

    unsafe extern "C" fn ext_latency_get(plugin: *const clap_plugin) -> u32 {
        check_null_ptr!(0, plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        wrapper.latency.current.load(Ordering::SeqCst)
    }

    unsafe extern "C" fn ext_note_ports_count(_plugin: *const clap_plugin, is_input: bool) -> u32 {
        match is_input {
            _ if P::NOTE_PORTS => 1,
            _ => 0,
        }
    }

    unsafe extern "C" fn ext_note_ports_get(
        _plugin: *const clap_plugin,
        index: u32,
        is_input: bool,
        info: *mut clap_note_port_info,
    ) -> bool {
        match (index, is_input) {
            (0, true) if P::NOTE_PORTS => {
                unsafe {
                    *info = std::mem::zeroed();
                }

                let info = unsafe { &mut *info };
                info.id = 0;
                // NOTE: REAPER won't send us SysEx if we don't support the MIDI dialect
                // TODO: Implement MPE (would just be a toggle for the plugin to expose it) and MIDI2
                info.supported_dialects = CLAP_NOTE_DIALECT_CLAP | CLAP_NOTE_DIALECT_MIDI;
                info.preferred_dialect = CLAP_NOTE_DIALECT_CLAP;
                strlcpy(&mut info.name, "Note Input");

                true
            }
            (0, false) if P::NOTE_PORTS => {
                unsafe { *info = std::mem::zeroed() };

                let info = unsafe { &mut *info };
                info.id = 0;
                info.supported_dialects = CLAP_NOTE_DIALECT_CLAP | CLAP_NOTE_DIALECT_MIDI;
                info.preferred_dialect = CLAP_NOTE_DIALECT_CLAP;
                strlcpy(&mut info.name, "Note Output");

                true
            }
            _ => false,
        }
    }

    unsafe extern "C" fn ext_params_count(plugin: *const clap_plugin) -> u32 {
        check_null_ptr!(0, plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        wrapper.params.len() as u32
    }

    unsafe extern "C" fn ext_params_get_info(
        plugin: *const clap_plugin,
        param_index: u32,
        param_info: *mut clap_param_info,
    ) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data }, param_info);
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        let row = param_index as usize;
        if row >= wrapper.params.len() {
            return false;
        }
        let spec = wrapper.params.spec(row);
        let steps = spec.steps();

        unsafe {
            *param_info = std::mem::zeroed();
        }
        let param_info = unsafe { &mut *param_info };
        param_info.id = wrapper.params.hash(row);
        param_info.flags = 0;
        if spec.automatable {
            param_info.flags |= CLAP_PARAM_IS_AUTOMATABLE;
        }
        if spec.modulatable {
            param_info.flags |= CLAP_PARAM_IS_MODULATABLE;
        }
        if steps.is_some() {
            param_info.flags |= CLAP_PARAM_IS_STEPPED
        }
        param_info.cookie = std::ptr::null_mut();
        strlcpy(&mut param_info.name, spec.name);
        // Every row is at the top level: the module path stays empty.
        // The host sees each row from 0 to its step count (0 to 1 for a float): host values.
        param_info.min_value = 0.0;
        param_info.max_value = steps.unwrap_or(1) as f64;
        param_info.default_value = spec.to_host(spec.default);

        true
    }

    unsafe extern "C" fn ext_params_get_value(
        plugin: *const clap_plugin,
        param_id: clap_id,
        value: *mut f64,
    ) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data }, value);
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        match wrapper.params.row(param_id) {
            Some(row) => {
                unsafe {
                    *value = wrapper.params.host_value(row);
                }
                true
            }
            None => false,
        }
    }

    unsafe extern "C" fn ext_params_value_to_text(
        plugin: *const clap_plugin,
        param_id: clap_id,
        value: f64,
        display: *mut c_char,
        size: u32,
    ) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data }, display);
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        let dest = unsafe { std::slice::from_raw_parts_mut(display, size as usize) };
        match wrapper.params.row(param_id) {
            Some(row) => {
                strlcpy(dest, &wrapper.params.spec(row).text(value));
                true
            }
            None => false,
        }
    }

    unsafe extern "C" fn ext_params_text_to_value(
        plugin: *const clap_plugin,
        param_id: clap_id,
        display: *const c_char,
        value: *mut f64,
    ) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data }, display, value);
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        let Ok(display) = (unsafe { CStr::from_ptr(display).to_str() }) else {
            return false;
        };
        let parsed =
            wrapper.params.row(param_id).and_then(|row| wrapper.params.spec(row).parse(display));
        match parsed {
            Some(parsed) => {
                unsafe {
                    *value = parsed;
                }
                true
            }
            None => false,
        }
    }

    /// The host's values apply to the table at once, before the flush
    /// returns, so `get_value` and a save read them straight away (#1814 Q1);
    /// the host never flushes beside a process callback. A flush's events
    /// have no time. A list that fails validation applies nothing and loses
    /// the next process callback's input, which tells the plugin.
    unsafe extern "C" fn ext_params_flush(
        plugin: *const clap_plugin,
        in_: *const clap_input_events,
        out: *const clap_output_events,
    ) {
        check_null_ptr!((), plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        process_wrapper(|| {
            match unsafe { events::validate(in_, None) } {
                Ok(checked) => wrapper.write_parameters(checked.list),
                Err(()) => wrapper.flush_fault.store(true, Ordering::Release),
            }
            unsafe { wrapper.report(out) };
        });
    }

    /// The plugin declares no remote-control pages; the extension stays answered,
    /// since whether Bitwig reads its absence differently is unmeasured.
    unsafe extern "C" fn ext_remote_controls_count(_plugin: *const clap_plugin) -> u32 {
        0
    }

    unsafe extern "C" fn ext_remote_controls_get(
        _plugin: *const clap_plugin,
        _page_index: u32,
        _page: *mut clap_remote_controls_page,
    ) -> bool {
        false
    }

    /// Both plugins render offline as they do live.
    unsafe extern "C" fn ext_render_has_hard_realtime_requirement(
        _plugin: *const clap_plugin,
    ) -> bool {
        false
    }

    unsafe extern "C" fn ext_render_set(
        plugin: *const clap_plugin,
        mode: clap_plugin_render_mode,
    ) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        let mode = match mode {
            CLAP_RENDER_REALTIME => ProcessMode::Realtime,
            // Even if the plugin has a hard realtime requirement, we'll still honor this
            CLAP_RENDER_OFFLINE => ProcessMode::Offline,
            n => {
                crate::shell_debug_assert_failure!(
                    "Unknown rendering mode '{}', defaulting to realtime",
                    n
                );
                ProcessMode::Realtime
            }
        };

        let previous_mode = wrapper.current_process_mode.swap(mode);
        let activated = wrapper.is_activated.load(Ordering::SeqCst);
        if previous_mode != mode && activated {
            // We may change process mode while activated. In that case, restart the audio processor
            // so the plugin can react to the process mode change in `Plugin::activate`.
            unsafe_clap_call! { &*wrapper.host_callback=>request_restart(&*wrapper.host_callback) };
        }

        true
    }

    unsafe extern "C" fn ext_state_save(
        plugin: *const clap_plugin,
        stream: *const clap_ostream,
    ) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data }, stream);
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        match state::serialize_state_json(&wrapper.get_state_object()) {
            Ok(serialized) => {
                // CLAP does not provide a way to tell how much data there is left in a stream, so
                // we need to prepend it to our actual state data.
                let length_bytes = (serialized.len() as u64).to_le_bytes();
                if !write_stream(unsafe { &*stream }, &length_bytes) {
                    crate::shell_debug_assert_failure!(
                        "Error or end of stream while writing the state length to the stream."
                    );
                    return false;
                }
                if !write_stream(unsafe { &*stream }, &serialized) {
                    crate::shell_debug_assert_failure!(
                        "Error or end of stream while writing the state buffer to the stream."
                    );
                    return false;
                }

                crate::shell_trace!("Saved state ({} bytes)", serialized.len());

                true
            }
            Err(err) => {
                crate::shell_debug_assert_failure!("Could not save state: {:#}", err);
                false
            }
        }
    }

    unsafe extern "C" fn ext_state_load(
        plugin: *const clap_plugin,
        stream: *const clap_istream,
    ) -> bool {
        check_null_ptr!(false, plugin, unsafe { (*plugin).plugin_data }, stream);
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        // CLAP does not have a way to tell how much data there is left in a stream, so we've
        // prepended the size in front of our JSON state
        let mut length_bytes = [0u8; 8];
        if !read_stream(unsafe { &*stream }, length_bytes.as_mut_slice()) {
            crate::shell_debug_assert_failure!(
                "Error or end of stream while reading the state length from the stream."
            );
            return false;
        }
        let length = u64::from_le_bytes(length_bytes);

        let mut read_buffer: Vec<u8> = Vec::with_capacity(length as usize);
        if !read_stream(unsafe { &*stream }, read_buffer.spare_capacity_mut()) {
            crate::shell_debug_assert_failure!(
                "Error or end of stream while reading the state buffer from the stream."
            );
            return false;
        }
        unsafe {
            read_buffer.set_len(length as usize);
        }

        match state::deserialize_json(&read_buffer) {
            Some(state) => {
                let success = wrapper.restore(&state);
                if success {
                    crate::shell_trace!("Loaded state ({} bytes)", read_buffer.len());
                }

                success
            }
            None => false,
        }
    }

    unsafe extern "C" fn ext_tail_get(plugin: *const clap_plugin) -> u32 {
        check_null_ptr!(0, plugin, unsafe { (*plugin).plugin_data });
        let wrapper = unsafe { &*((*plugin).plugin_data as *const Self) };

        if wrapper.processed.load(Ordering::Acquire) { u32::MAX } else { 0 }
    }
}

/// The CLAP port type for `channels`: mono, stereo, or none named.
fn port_type(channels: u32) -> *const c_char {
    match channels {
        1 => CLAP_PORT_MONO.as_ptr(),
        2 => CLAP_PORT_STEREO.as_ptr(),
        _ => std::ptr::null(),
    }
}

/// Convenience function to query an extension from the host.
///
/// # Safety
///
/// The extension type `T` must match the extension's name `name`.
unsafe fn query_host_extension<T>(
    host_callback: &ClapPtr<clap_host>,
    name: &CStr,
) -> Option<ClapPtr<T>> {
    let extension_ptr = unsafe {
        clap_call! { host_callback=>get_extension(&**host_callback, name.as_ptr()) }
    };
    if !extension_ptr.is_null() {
        unsafe { Some(ClapPtr::new(extension_ptr as *const T)) }
    } else {
        None
    }
}

/// One wrapper's [`EditorHost`], held weakly: the plugin keeps it, and the
/// wrapper keeps the plugin.
struct HostGui<P: Plugin>(Weak<Wrapper<P>>);

impl<P: Plugin> EditorHost for HostGui<P> {
    fn closed(&self) -> bool {
        let Some(wrapper) = self.0.upgrade() else { return false };
        let host_gui = wrapper.host_gui.borrow();
        let Some(host_gui) = host_gui.as_ref() else { return false };
        unsafe_clap_call! { host_gui=>closed(&*wrapper.host_callback, true) };
        true
    }

    fn request_resize(&self, (width, height): (u32, u32)) -> bool {
        let Some(wrapper) = self.0.upgrade() else { return false };
        let host_gui = wrapper.host_gui.borrow();
        let Some(host_gui) = host_gui.as_ref() else { return false };
        unsafe_clap_call! { host_gui=>request_resize(&*wrapper.host_callback, width, height) }
    }

    // Each writes the table at once and asks the host to flush, where the
    // shell tells it what changed (#1814 Q1).
    fn begin_set_parameter(&self, row: usize) {
        let Some(wrapper) = self.0.upgrade() else { return };
        wrapper.params.begin_edit(row);
        wrapper.request_flush();
    }

    fn set_parameter(&self, row: usize, plain: f32) {
        let Some(wrapper) = self.0.upgrade() else { return };
        wrapper.params.edit(row, plain);
        wrapper.request_flush();
    }

    fn end_set_parameter(&self, row: usize) {
        let Some(wrapper) = self.0.upgrade() else { return };
        wrapper.params.end_edit(row);
        wrapper.request_flush();
    }
}
