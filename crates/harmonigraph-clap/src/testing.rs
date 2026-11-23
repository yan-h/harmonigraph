//! One CLAP host for every suite that drives a plugin through its exported
//! entry point: the shell's own boundary and auxiliary fixtures, and
//! Harmonigraph's configuration, tuning and host-surface suites (#1868).
//!
//! The host answers only the extensions a suite turns on ([`Host::params`],
//! [`Host::state`], [`Host::gui`], [`Host::latency`],
//! [`Host::track_info`]), since what a host answers is part of what a suite
//! tests, and counts what the plugin asks of it. [`Device`] creates and
//! initializes one plugin on it, and drives activation, process callbacks,
//! flushes and the state stream through the C ABI alone. Anything specific
//! to a plugin, such as which engine it starts with, belongs to that
//! plugin's own fixture.
use crate::wrapper::clap::Wrapper;
use crate::{plugin::Plugin, wrapper::clap::clap_plugin_entry};
use clap_sys::{
    audio_buffer::clap_audio_buffer,
    events::*,
    ext::{
        gui::{CLAP_EXT_GUI, clap_host_gui, clap_plugin_gui},
        latency::{CLAP_EXT_LATENCY, clap_host_latency, clap_plugin_latency},
        params::{CLAP_EXT_PARAMS, clap_host_params, clap_plugin_params},
        state::{CLAP_EXT_STATE, clap_host_state, clap_plugin_state},
        track_info::{
            CLAP_EXT_TRACK_INFO, CLAP_EXT_TRACK_INFO_COMPAT, CLAP_TRACK_INFO_HAS_TRACK_NAME,
            clap_host_track_info, clap_track_info,
        },
    },
    factory::plugin_factory::{CLAP_PLUGIN_FACTORY_ID, clap_plugin_factory},
    host::clap_host,
    plugin::clap_plugin,
    process::*,
    stream::{clap_istream, clap_ostream},
    version::CLAP_VERSION,
};
use std::{
    ffi::{CStr, c_char, c_void},
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicUsize, Ordering},
    },
};

/// A test host: the extensions it answers, and what the plugin asked of it.
/// Every extension is off until a suite turns it on.
#[derive(Default)]
pub struct Host {
    params: bool,
    state: bool,
    gui: bool,
    latency: bool,
    /// The track-info id answered, the current one or the compatibility one.
    track_info: Option<&'static CStr>,
    /// `gui.request_resize` calls the plugin's `gui.get_size` and
    /// `gui.set_size` from inside, as Bitwig echoes a resize.
    reenter_resize: bool,

    /// `request_callback` calls.
    pub callbacks: AtomicUsize,
    /// A `request_callback` made on the thread so named waits while `hold`
    /// is set, with `holding` set while it waits: a host wake in flight.
    pub hold_thread: Mutex<Option<&'static str>>,
    pub hold: std::sync::Arc<AtomicBool>,
    pub holding: std::sync::Arc<AtomicBool>,
    /// `request_restart` calls.
    pub restarts: AtomicUsize,
    /// `state.mark_dirty` calls.
    pub dirty: AtomicUsize,
    /// `params.rescan` calls, and the flags of the last.
    pub rescans: AtomicUsize,
    pub rescan_flags: AtomicU32,
    /// `gui.closed` calls, and whether the last asked for destruction.
    pub closed: AtomicUsize,
    pub closed_destroyed: AtomicBool,
    /// The sizes `gui.request_resize` asked for, and whether it grants them.
    pub resizes: Mutex<Vec<(u32, u32)>>,
    pub grants_resize: AtomicBool,
    /// The last `gui.request_resize` the host re-entered the plugin from.
    pub reentered: Mutex<Option<Resize>>,
    /// The track name `track_info.get` answers, or `None` to answer no.
    pub track_name: Mutex<Option<String>>,
    /// `latency.changed` calls, those outside an activation (which CLAP lets
    /// a host discard), and the latency read back at the last.
    pub latency_changes: AtomicUsize,
    pub out_of_phase: AtomicUsize,
    pub observed_latency: AtomicUsize,
    /// Set only across `clap_plugin::activate`, the one call CLAP lets the
    /// latency change in.
    activating: AtomicBool,
    /// The plugin, for a callback that calls back into it as a host does.
    plugin: AtomicPtr<clap_plugin>,
}

impl Host {
    /// A host that answers no extension.
    pub fn new() -> Self {
        Self::default()
    }
    pub fn params(mut self) -> Self {
        self.params = true;
        self
    }
    pub fn state(mut self) -> Self {
        self.state = true;
        self
    }
    pub fn gui(mut self) -> Self {
        self.gui = true;
        self
    }
    /// A `gui.request_resize` that grants the size after calling back into
    /// the plugin's `gui.get_size` and `gui.set_size`, as Bitwig does.
    pub fn reentering_resize(mut self) -> Self {
        self.reenter_resize = true;
        self.grants_resize = AtomicBool::new(true);
        self
    }
    pub fn latency(mut self) -> Self {
        self.latency = true;
        self
    }
    /// Answer track info under its current id, or under the compatibility
    /// id Bitwig still uses, naming the track `name`.
    pub fn track_info(mut self, compat: bool, name: Option<&str>) -> Self {
        self.track_info =
            Some(if compat { CLAP_EXT_TRACK_INFO_COMPAT } else { CLAP_EXT_TRACK_INFO });
        self.track_name = Mutex::new(name.map(str::to_owned));
        self
    }
}

/// A `gui.request_resize` as the host heard it: the size asked for, and the
/// plugin's answers to `gui.get_size` and `gui.set_size(asked)` made from
/// inside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Resize {
    pub asked: (u32, u32),
    pub reported: (u32, u32),
    pub set: bool,
}

/// # Safety
///
/// `host` is a host a [`Device`] made, whose data is its [`Host`].
unsafe fn stats<'a>(host: *const clap_host) -> &'a Host {
    unsafe { &*((*host).host_data.cast::<Host>()) }
}

unsafe extern "C" fn extension(host: *const clap_host, id: *const c_char) -> *const c_void {
    let stats = unsafe { stats(host) };
    let id = unsafe { CStr::from_ptr(id) };
    if stats.params && id == CLAP_EXT_PARAMS {
        (&HOST_PARAMS as *const clap_host_params).cast()
    } else if stats.state && id == CLAP_EXT_STATE {
        (&HOST_STATE as *const clap_host_state).cast()
    } else if stats.gui && id == CLAP_EXT_GUI {
        (&HOST_GUI as *const clap_host_gui).cast()
    } else if stats.latency && id == CLAP_EXT_LATENCY {
        (&HOST_LATENCY as *const clap_host_latency).cast()
    } else if stats.track_info == Some(id) {
        (&HOST_TRACK_INFO as *const clap_host_track_info).cast()
    } else {
        ptr::null()
    }
}
unsafe extern "C" fn request_callback(host: *const clap_host) {
    let stats = unsafe { stats(host) };
    stats.callbacks.fetch_add(1, Ordering::Relaxed);
    let thread = std::thread::current();
    if thread.name().is_some() && *stats.hold_thread.lock().unwrap() == thread.name() {
        // Clones, so the wait reads nothing of a host freed meanwhile.
        let (hold, holding) = (stats.hold.clone(), stats.holding.clone());
        holding.store(true, Ordering::Release);
        while hold.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        holding.store(false, Ordering::Release);
    }
}
unsafe extern "C" fn request_restart(host: *const clap_host) {
    unsafe { stats(host) }.restarts.fetch_add(1, Ordering::Relaxed);
}
unsafe extern "C" fn request(_: *const clap_host) {}

unsafe extern "C" fn rescan(host: *const clap_host, flags: u32) {
    let stats = unsafe { stats(host) };
    stats.rescans.fetch_add(1, Ordering::Relaxed);
    stats.rescan_flags.store(flags, Ordering::Relaxed);
}
unsafe extern "C" fn clear(_: *const clap_host, _: u32, _: u32) {}
static HOST_PARAMS: clap_host_params =
    clap_host_params { rescan: Some(rescan), clear: Some(clear), request_flush: Some(request) };

unsafe extern "C" fn mark_dirty(host: *const clap_host) {
    unsafe { stats(host) }.dirty.fetch_add(1, Ordering::Relaxed);
}
static HOST_STATE: clap_host_state = clap_host_state { mark_dirty: Some(mark_dirty) };

unsafe extern "C" fn request_resize(host: *const clap_host, width: u32, height: u32) -> bool {
    let stats = unsafe { stats(host) };
    stats.resizes.lock().unwrap_or_else(|e| e.into_inner()).push((width, height));
    if stats.reenter_resize {
        let plugin = stats.plugin.load(Ordering::Acquire).cast_const();
        let gui = unsafe {
            &*((*plugin).get_extension.unwrap())(plugin, CLAP_EXT_GUI.as_ptr())
                .cast::<clap_plugin_gui>()
        };
        let mut reported = (0, 0);
        assert!(unsafe { (gui.get_size.unwrap())(plugin, &mut reported.0, &mut reported.1) });
        let set = unsafe { (gui.set_size.unwrap())(plugin, width, height) };
        let resize = Resize { asked: (width, height), reported, set };
        *stats.reentered.lock().unwrap_or_else(|e| e.into_inner()) = Some(resize);
    }
    stats.grants_resize.load(Ordering::Relaxed)
}
unsafe extern "C" fn resize_hints_changed(_: *const clap_host) {}
unsafe extern "C" fn refuse(_: *const clap_host) -> bool {
    false
}
unsafe extern "C" fn closed(host: *const clap_host, was_destroyed: bool) {
    let stats = unsafe { stats(host) };
    stats.closed.fetch_add(1, Ordering::Relaxed);
    stats.closed_destroyed.store(was_destroyed, Ordering::Relaxed);
}
static HOST_GUI: clap_host_gui = clap_host_gui {
    resize_hints_changed: Some(resize_hints_changed),
    request_resize: Some(request_resize),
    request_show: Some(refuse),
    request_hide: Some(refuse),
    closed: Some(closed),
};

unsafe extern "C" fn latency_changed(host: *const clap_host) {
    let stats = unsafe { stats(host) };
    stats.latency_changes.fetch_add(1, Ordering::Relaxed);
    if !stats.activating.load(Ordering::Relaxed) {
        stats.out_of_phase.fetch_add(1, Ordering::Relaxed);
    }
    // Read the value the moment the host is told there is a new one.
    let plugin = stats.plugin.load(Ordering::Relaxed);
    if !plugin.is_null() {
        stats.observed_latency.store(latency_of(plugin) as usize, Ordering::Relaxed);
    }
}
static HOST_LATENCY: clap_host_latency = clap_host_latency { changed: Some(latency_changed) };
fn latency_of(plugin: *const clap_plugin) -> u32 {
    let latency = unsafe {
        &*((*plugin).get_extension.unwrap()(plugin, CLAP_EXT_LATENCY.as_ptr())
            .cast::<clap_plugin_latency>())
    };
    unsafe { latency.get.unwrap()(plugin) }
}

unsafe extern "C" fn track_info(host: *const clap_host, info: *mut clap_track_info) -> bool {
    let name = unsafe { stats(host) }.track_name.lock().unwrap_or_else(|e| e.into_inner());
    let Some(name) = &*name else {
        return false;
    };
    let info = unsafe { &mut *info };
    info.flags = CLAP_TRACK_INFO_HAS_TRACK_NAME;
    for (target, byte) in info.name.iter_mut().zip(name.bytes()) {
        *target = byte as c_char;
    }
    true
}
static HOST_TRACK_INFO: clap_host_track_info = clap_host_track_info { get: Some(track_info) };

/// A header for an event of type `T`.
pub fn header<T>(kind: u16, time: u32) -> clap_event_header {
    clap_event_header {
        size: std::mem::size_of::<T>() as u32,
        time,
        space_id: CLAP_CORE_EVENT_SPACE_ID,
        type_: kind,
        flags: 0,
    }
}

/// One event, whole, as a host hands it over or a plugin pushes it.
#[derive(Clone, Copy)]
pub enum Event {
    Note(clap_event_note),
    Expression(clap_event_note_expression),
    Midi(clap_event_midi),
    SysEx(clap_event_midi_sysex),
    Param(clap_event_param_value),
    Mod(clap_event_param_mod),
    Gesture(clap_event_param_gesture),
    Transport(clap_event_transport),
    /// Only a header: a type this host does not read.
    Header(clap_event_header),
}

impl Event {
    /// A pointer to the whole event, as a host hands one over: a pointer to
    /// only its header field could not read the rest (Miri).
    pub fn header(&self) -> *const clap_event_header {
        match self {
            Self::Note(e) => (e as *const clap_event_note).cast(),
            Self::Expression(e) => (e as *const clap_event_note_expression).cast(),
            Self::Midi(e) => (e as *const clap_event_midi).cast(),
            Self::SysEx(e) => (e as *const clap_event_midi_sysex).cast(),
            Self::Param(e) => (e as *const clap_event_param_value).cast(),
            Self::Mod(e) => (e as *const clap_event_param_mod).cast(),
            Self::Gesture(e) => (e as *const clap_event_param_gesture).cast(),
            Self::Transport(e) => (e as *const clap_event_transport).cast(),
            Self::Header(e) => e,
        }
    }

    /// Copy the core event at `header`, by its type.
    ///
    /// # Safety
    ///
    /// `header` heads a whole event of the type it names.
    unsafe fn read(header: *const clap_event_header) -> Self {
        let h = unsafe { &*header };
        unsafe {
            match h.type_ {
                _ if h.space_id != CLAP_CORE_EVENT_SPACE_ID => Self::Header(*h),
                CLAP_EVENT_NOTE_ON
                | CLAP_EVENT_NOTE_OFF
                | CLAP_EVENT_NOTE_CHOKE
                | CLAP_EVENT_NOTE_END => Self::Note(*header.cast()),
                CLAP_EVENT_NOTE_EXPRESSION => Self::Expression(*header.cast()),
                CLAP_EVENT_MIDI => Self::Midi(*header.cast()),
                CLAP_EVENT_MIDI_SYSEX => Self::SysEx(*header.cast()),
                CLAP_EVENT_PARAM_VALUE => Self::Param(*header.cast()),
                CLAP_EVENT_PARAM_MOD => Self::Mod(*header.cast()),
                CLAP_EVENT_PARAM_GESTURE_BEGIN | CLAP_EVENT_PARAM_GESTURE_END => {
                    Self::Gesture(*header.cast())
                }
                CLAP_EVENT_TRANSPORT => Self::Transport(*header.cast()),
                _ => Self::Header(*h),
            }
        }
    }
}

unsafe extern "C" fn input_size(list: *const clap_input_events) -> u32 {
    let events: &[Event] = unsafe { *(*list).ctx.cast::<&[Event]>() };
    events.len() as u32
}
unsafe extern "C" fn input_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    let events: &[Event] = unsafe { *(*list).ctx.cast::<&[Event]>() };
    events[index as usize].header()
}
/// An input list over `events`, which must outlive it.
fn input_list(events: &&[Event]) -> clap_input_events {
    clap_input_events {
        ctx: (events as *const &[Event]).cast_mut().cast(),
        size: Some(input_size),
        get: Some(input_get),
    }
}

/// One event the plugin offered the host's output list.
#[derive(Clone, Copy)]
pub struct Offer {
    pub time: u32,
    pub kind: u16,
    pub flags: u32,
    pub event: Event,
    /// Whether the host took it.
    pub taken: bool,
}

impl Offer {
    /// The parameter a value or gesture names, else 0.
    pub fn param_id(&self) -> u32 {
        match self.event {
            Event::Param(e) => e.param_id,
            Event::Gesture(e) => e.param_id,
            _ => 0,
        }
    }
}

/// The host's output list: what the plugin offered, and the host's answers.
///
/// The host takes an offer unless [`Sink::script`] answers it, or it is of
/// [`Sink::refuse_kind`], or it is the [`Sink::refuse_attempt`]th. Taken
/// events must not go back in time within a callback, which is CLAP's
/// contract for an output list.
pub struct Sink {
    /// The answer to each offer in turn; past its end, the rules below.
    pub script: Vec<bool>,
    /// Refuse every offer of this type; [`Sink::ALL`] refuses every offer.
    pub refuse_kind: Option<u16>,
    /// Refuse the offer with this 1-based number.
    pub refuse_attempt: Option<usize>,
    pub offers: Vec<Offer>,
    taken_time: u32,
}

impl Sink {
    /// A [`Sink::refuse_kind`] that refuses every offer.
    pub const ALL: u16 = u16::MAX;

    /// A sink with room for `capacity` offers, so a guarded callback never
    /// allocates in it: a callback that offers more fails here, where the
    /// overflow is reported, rather than in the allocation guard.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            script: Vec::new(),
            refuse_kind: None,
            refuse_attempt: None,
            offers: Vec::with_capacity(capacity),
            taken_time: 0,
        }
    }
    fn output(&mut self) -> clap_output_events {
        self.taken_time = 0;
        clap_output_events { ctx: (self as *mut Self).cast(), try_push: Some(try_push) }
    }
}

unsafe extern "C" fn try_push(
    list: *const clap_output_events,
    header: *const clap_event_header,
) -> bool {
    let sink = unsafe { &mut *((*list).ctx.cast::<Sink>()) };
    let h = unsafe { &*header };
    let number = sink.offers.len() + 1;
    let taken = match sink.script.get(number - 1) {
        Some(&answer) => answer,
        None => {
            sink.refuse_kind != Some(h.type_)
                && sink.refuse_kind != Some(Sink::ALL)
                && sink.refuse_attempt != Some(number)
        }
    };
    if taken {
        assert!(
            h.time >= sink.taken_time,
            "CLAP output must be sorted by time: {} after {}",
            h.time,
            sink.taken_time
        );
        sink.taken_time = h.time;
    }
    assert!(sink.offers.len() < sink.offers.capacity(), "the sink is full");
    let event = unsafe { Event::read(header) };
    sink.offers.push(Offer { time: h.time, kind: h.type_, flags: h.flags, event, taken });
    taken
}

/// The audio a callback carries.
pub enum Audio<'a> {
    /// No audio ports at all.
    None,
    /// A silent stereo main input, processed in place as the main output.
    Main,
    /// As [`Audio::Main`], with a stereo auxiliary input whose channels hold
    /// these samples (silence past their end).
    Sidechain([&'a [f32]; 2]),
}

/// One process callback.
pub struct Call<'a> {
    pub steady: i64,
    pub frames: u32,
    pub events: &'a [Event],
    /// The callback's own transport.
    pub transport: Option<clap_event_transport>,
    pub audio: Audio<'a>,
}

/// One plugin from an exported entry, on a [`Host`] of its own.
///
/// The host's structs are raw pointers the plugin holds on to, freed at the
/// end: a `Box` moved after the plugin took its pointer would end that
/// pointer's reach (Miri).
pub struct Device {
    pub plugin: *const clap_plugin,
    host: *mut Host,
    clap_host: *mut clap_host,
    active: bool,
}

impl Device {
    /// Create plugin `id` from `entry`'s factory on `host`, and initialize
    /// it. The entry is initialized once per process, which holds one entry.
    pub fn new(entry: &clap_plugin_entry, id: &CStr, host: Host) -> Self {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| assert!(unsafe { (entry.init.unwrap())(c"fixture.clap".as_ptr()) }));
        let host = Box::into_raw(Box::new(host));
        let clap_host = Box::into_raw(Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: host.cast(),
            name: c"fixture".as_ptr(),
            vendor: c"fixture".as_ptr(),
            url: c"".as_ptr(),
            version: c"1".as_ptr(),
            get_extension: Some(extension),
            request_restart: Some(request_restart),
            request_process: Some(request),
            request_callback: Some(request_callback),
        }));
        let factory = unsafe { (entry.get_factory.unwrap())(CLAP_PLUGIN_FACTORY_ID.as_ptr()) }
            .cast::<clap_plugin_factory>();
        let plugin =
            unsafe { ((*factory).create_plugin.unwrap())(factory, clap_host, id.as_ptr()) };
        assert!(!plugin.is_null(), "{id:?}");
        unsafe { &*host }.plugin.store(plugin.cast_mut(), Ordering::Release);
        assert!(unsafe { ((*plugin).init.unwrap())(plugin) });
        Self { plugin, host, clap_host, active: false }
    }
    /// What the plugin asked of the host.
    pub fn host(&self) -> &Host {
        unsafe { &*self.host }
    }
    /// The shell wrapping plugin type `P`.
    ///
    /// # Safety
    ///
    /// The device was created from `P`'s id.
    pub unsafe fn wrapper<P: Plugin>(&self) -> &Wrapper<P> {
        unsafe { &*((*self.plugin).plugin_data.cast()) }
    }
    /// The plugin's extension `id`, if it answers one.
    pub fn extension<T>(&self, id: &CStr) -> Option<&T> {
        let found = unsafe { ((*self.plugin).get_extension.unwrap())(self.plugin, id.as_ptr()) };
        (!found.is_null()).then(|| unsafe { &*found.cast::<T>() })
    }

    /// Activate at `rate` with at most `max_frames` per callback, and start
    /// processing.
    pub fn activate(&mut self, rate: f64, max_frames: u32) {
        self.host().activating.store(true, Ordering::Relaxed);
        let activated =
            unsafe { ((*self.plugin).activate.unwrap())(self.plugin, rate, 1, max_frames) };
        self.host().activating.store(false, Ordering::Relaxed);
        assert!(activated);
        assert!(unsafe { ((*self.plugin).start_processing.unwrap())(self.plugin) });
        self.active = true;
    }
    /// Stop processing and deactivate.
    pub fn deactivate(&mut self) {
        assert!(self.active);
        unsafe {
            ((*self.plugin).stop_processing.unwrap())(self.plugin);
            ((*self.plugin).deactivate.unwrap())(self.plugin);
        }
        self.active = false;
    }
    /// Record that a fixture activated (and started processing) or
    /// deactivated the plugin through its vtable, so the drop matches.
    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }
    /// `clap_plugin::reset`.
    pub fn reset(&self) {
        unsafe { ((*self.plugin).reset.unwrap())(self.plugin) };
    }
    /// The host's main-thread callback.
    pub fn main(&self) {
        unsafe { ((*self.plugin).on_main_thread.unwrap())(self.plugin) };
    }
    pub fn latency(&self) -> u32 {
        latency_of(self.plugin)
    }

    /// One process callback, the host's output list `sink`.
    pub fn process(&self, call: &Call<'_>, sink: &mut Sink) -> clap_process_status {
        let length = (call.frames as usize).max(64);
        let mut main = [vec![0.0f32; length], vec![0.0f32; length]];
        let mut side = [vec![0.0f32; length], vec![0.0f32; length]];
        if let Audio::Sidechain(samples) = &call.audio {
            for (channel, samples) in side.iter_mut().zip(samples) {
                let count = samples.len().min(length);
                channel[..count].copy_from_slice(&samples[..count]);
            }
        }
        let mut main_channels = main.each_mut().map(|c| c.as_mut_ptr());
        let mut side_channels = side.each_mut().map(|c| c.as_mut_ptr());
        let buffer = |channels: &mut [*mut f32; 2]| clap_audio_buffer {
            data32: channels.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let inputs = [buffer(&mut main_channels), buffer(&mut side_channels)];
        let mut output = inputs[0];
        let (inputs_count, outputs_count) = match call.audio {
            Audio::None => (0, 0),
            Audio::Main => (1, 1),
            Audio::Sidechain(_) => (2, 1),
        };
        let events = call.events;
        let input = input_list(&events);
        let output_list = sink.output();
        let process = clap_process {
            steady_time: call.steady,
            frames_count: call.frames,
            transport: call.transport.as_ref().map_or(ptr::null(), |t| t),
            audio_inputs: if inputs_count == 0 { ptr::null() } else { inputs.as_ptr() },
            audio_outputs: if outputs_count == 0 { ptr::null_mut() } else { &mut output },
            audio_inputs_count: inputs_count,
            audio_outputs_count: outputs_count,
            in_events: &input,
            out_events: &output_list,
        };
        self.process_raw(&process)
    }
    /// A process callback exactly as `process` has it.
    pub fn process_raw(&self, process: &clap_process) -> clap_process_status {
        unsafe { ((*self.plugin).process.unwrap())(self.plugin, process) }
    }

    pub fn params(&self) -> &clap_plugin_params {
        self.extension(CLAP_EXT_PARAMS).expect("a params extension")
    }
    /// The host value of parameter `id`.
    pub fn value(&self, id: u32) -> f64 {
        let mut value = f64::NAN;
        assert!(unsafe { (self.params().get_value.unwrap())(self.plugin, id, &mut value) });
        value
    }
    /// `params.flush` with `events`, its output `sink` or none.
    pub fn flush(&self, events: &[Event], sink: Option<&mut Sink>) {
        let input = input_list(&events);
        let output = sink.map(Sink::output);
        let output = output.as_ref().map_or(ptr::null(), |o| o as *const clap_output_events);
        unsafe { (self.params().flush.unwrap())(self.plugin, &input, output) };
    }

    fn state(&self) -> &clap_plugin_state {
        self.extension(CLAP_EXT_STATE).expect("a state extension")
    }
    /// The bytes the state extension saves, as the host stores them.
    pub fn save_framed(&self) -> Vec<u8> {
        unsafe extern "C" fn write(
            stream: *const clap_ostream,
            data: *const c_void,
            size: u64,
        ) -> i64 {
            let bytes = unsafe { &mut *((*stream).ctx.cast::<Vec<u8>>()) };
            bytes.extend_from_slice(unsafe {
                std::slice::from_raw_parts(data.cast(), size as usize)
            });
            size as i64
        }
        let mut bytes = Vec::<u8>::new();
        let stream = clap_ostream { ctx: (&mut bytes as *mut Vec<u8>).cast(), write: Some(write) };
        assert!(unsafe { (self.state().save.unwrap())(self.plugin, &stream) });
        bytes
    }
    /// The JSON a save holds, its 8-byte length prefix checked against what
    /// follows it.
    pub fn save(&self) -> Vec<u8> {
        let mut bytes = self.save_framed();
        let length = u64::from_le_bytes(bytes[..8].try_into().unwrap());
        assert_eq!(length as usize, bytes.len() - 8, "the length prefix counts what follows it");
        bytes.drain(..8);
        bytes
    }
    /// Load `bytes` through the state extension as the host reads them:
    /// whether the plugin took them.
    pub fn load_framed(&self, bytes: &[u8]) -> bool {
        unsafe extern "C" fn read(
            stream: *const clap_istream,
            data: *mut c_void,
            size: u64,
        ) -> i64 {
            let bytes = unsafe { &mut *((*stream).ctx.cast::<&[u8]>()) };
            let count = bytes.len().min(size as usize);
            unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast(), count) };
            *bytes = &bytes[count..];
            count as i64
        }
        let mut unread = bytes;
        let stream = clap_istream { ctx: (&mut unread as *mut &[u8]).cast(), read: Some(read) };
        unsafe { (self.state().load.unwrap())(self.plugin, &stream) }
    }
    /// Load `json`, framed as a save is.
    pub fn load(&self, json: &[u8]) -> bool {
        self.load_framed(&[&(json.len() as u64).to_le_bytes()[..], json].concat())
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            if self.active {
                ((*self.plugin).stop_processing.unwrap())(self.plugin);
                ((*self.plugin).deactivate.unwrap())(self.plugin);
            }
            ((*self.plugin).destroy.unwrap())(self.plugin);
            drop(Box::from_raw(self.clap_host));
            drop(Box::from_raw(self.host));
        }
    }
}
