//! Production exported-factory fixtures. The real CLAP entry point, real
//! endpoint capacities and real recorder fanout, without opening an editor.
use super::*;
use clap_sys::{
    audio_buffer::clap_audio_buffer,
    events::*,
    ext::{
        latency::{clap_host_latency, clap_plugin_latency, CLAP_EXT_LATENCY},
        params::{clap_plugin_params, CLAP_EXT_PARAMS},
        state::{clap_plugin_state, CLAP_EXT_STATE},
    },
    factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID},
    host::clap_host,
    plugin::clap_plugin,
    process::*,
    stream::{clap_istream, clap_ostream},
    version::CLAP_VERSION,
};
use event::Event;
use nice_plug::plugin::PluginState;

#[path = "musical_tests.rs"]
mod musical_tests;
use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};

#[derive(Default)]
struct Host {
    callbacks: AtomicUsize,
    restarts: AtomicUsize,
    latency_changes: AtomicUsize,
    /// The plugin, so the notification below can do what a host does with one:
    /// read the value the moment it is told there is a new one.
    plugin: AtomicPtr<clap_plugin>,
    observed_latency: AtomicUsize,
    /// Set only across `clap_plugin::activate`. CLAP lets the latency change
    /// during that call and nowhere else, so a host is entitled to discard a
    /// notification that arrives with this clear.
    activating: AtomicBool,
    /// Notifications this host would have been right to discard.
    out_of_phase: AtomicUsize,
}
unsafe extern "C" fn extension(_: *const clap_host, id: *const c_char) -> *const c_void {
    if unsafe { CStr::from_ptr(id) } == CLAP_EXT_LATENCY {
        &HOST_LATENCY as *const _ as *const c_void
    } else {
        ptr::null()
    }
}
static HOST_LATENCY: clap_host_latency = clap_host_latency { changed: Some(latency_changed) };
unsafe extern "C" fn latency_changed(host: *const clap_host) {
    let stats = unsafe { &*((*host).host_data.cast::<Host>()) };
    stats.latency_changes.fetch_add(1, Ordering::Relaxed);
    if !stats.activating.load(Ordering::Relaxed) {
        stats.out_of_phase.fetch_add(1, Ordering::Relaxed);
    }
    let plugin = stats.plugin.load(Ordering::Relaxed);
    if !plugin.is_null() {
        stats.observed_latency.store(latency_of(plugin) as usize, Ordering::Relaxed);
    }
}
fn latency_of(plugin: *const clap_plugin) -> u32 {
    let latency = unsafe {
        &*((*plugin).get_extension.unwrap()(plugin, CLAP_EXT_LATENCY.as_ptr())
            .cast::<clap_plugin_latency>())
    };
    unsafe { latency.get.unwrap()(plugin) }
}
unsafe extern "C" fn restart(host: *const clap_host) {
    unsafe { &*((*host).host_data.cast::<Host>()) }.restarts.fetch_add(1, Ordering::Relaxed);
}
unsafe extern "C" fn request(_: *const clap_host) {}
unsafe extern "C" fn callback(host: *const clap_host) {
    unsafe { &*((*host).host_data.cast::<Host>()) }.callbacks.fetch_add(1, Ordering::Relaxed);
}
fn factory() -> &'static clap_plugin_factory {
    static ENTRY: std::sync::Once = std::sync::Once::new();
    ENTRY.call_once(|| {
        assert!(unsafe { crate::clap_entry.init.unwrap()(c"fixture.clap".as_ptr()) })
    });
    unsafe { &*(crate::clap_entry.get_factory.unwrap()(CLAP_PLUGIN_FACTORY_ID.as_ptr()).cast()) }
}
#[derive(Clone, Copy)]
enum Input {
    Note(clap_event_note),
    Expression(clap_event_note_expression),
    Midi(clap_event_midi),
    Param(clap_event_param_value),
    Transport(clap_event_transport),
}
impl Input {
    fn header(&self) -> &clap_event_header {
        match self {
            Self::Note(e) => &e.header,
            Self::Expression(e) => &e.header,
            Self::Midi(e) => &e.header,
            Self::Param(e) => &e.header,
            Self::Transport(e) => &e.header,
        }
    }
}
fn header<T>(kind: u16, time: u32) -> clap_event_header {
    clap_event_header {
        size: std::mem::size_of::<T>() as u32,
        time,
        space_id: 0,
        type_: kind,
        flags: 0,
    }
}
fn note(id: i32, channel: i16, key: i16, time: u32, on: bool) -> Input {
    Input::Note(clap_event_note {
        header: header::<clap_event_note>(
            if on { CLAP_EVENT_NOTE_ON } else { CLAP_EVENT_NOTE_OFF },
            time,
        ),
        note_id: id,
        port_index: 0,
        channel,
        key,
        velocity: 0.625,
    })
}
fn expression(id: i32, value: f64, time: u32) -> Input {
    Input::Expression(clap_event_note_expression {
        header: header::<clap_event_note_expression>(CLAP_EVENT_NOTE_EXPRESSION, time),
        expression_id: CLAP_NOTE_EXPRESSION_TUNING,
        note_id: id,
        port_index: -1,
        channel: -1,
        key: -1,
        value,
    })
}
fn raw_midi(data: [u8; 3], time: u32) -> Input {
    Input::Midi(clap_event_midi {
        header: header::<clap_event_midi>(CLAP_EVENT_MIDI, time),
        port_index: 0,
        data,
    })
}
fn hub_wrapper(device: &Device) -> &nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph> {
    assert!(!device.tuner);
    unsafe { &*((*device.plugin).plugin_data.cast()) }
}
fn inspect_hub<R>(device: &Device, f: impl FnOnce(&hub::Hub) -> R) -> R {
    hub_wrapper(device).test_inspect_plugin(|plugin| f(plugin.aggregation.as_ref().unwrap()))
}
fn transport(time: u32, tempo: f64) -> Input {
    let mut event: clap_event_transport = unsafe { std::mem::zeroed() };
    event.header = header::<clap_event_transport>(CLAP_EVENT_TRANSPORT, time);
    event.flags = CLAP_TRANSPORT_HAS_TEMPO | CLAP_TRANSPORT_IS_PLAYING;
    event.tempo = tempo;
    Input::Transport(event)
}
unsafe extern "C" fn size(events: *const clap_input_events) -> u32 {
    unsafe { &*((*events).ctx.cast::<Vec<Input>>()) }.len() as u32
}
unsafe extern "C" fn get(events: *const clap_input_events, index: u32) -> *const clap_event_header {
    (unsafe { &*((*events).ctx.cast::<Vec<Input>>()) })[index as usize].header()
}
struct Sink {
    values: Vec<(u32, Event)>,
    rejected: Vec<(u32, Event)>,
    attempts: usize,
    reject_kind: Option<u16>,
    reject_attempt: Option<usize>,
    callback_nanos: u128,
}
unsafe extern "C" fn push(
    output: *const clap_output_events,
    header: *const clap_event_header,
) -> bool {
    let sink = unsafe { &mut *((*output).ctx.cast::<Sink>()) };
    let header = unsafe { &*header };
    sink.attempts += 1;
    let reject = sink.reject_kind == Some(header.type_)
        || sink.reject_kind == Some(u16::MAX)
        || sink.reject_attempt == Some(sink.attempts);
    let value = match header.type_ {
        CLAP_EVENT_NOTE_ON | CLAP_EVENT_NOTE_OFF | CLAP_EVENT_NOTE_CHOKE | CLAP_EVENT_NOTE_END => {
            let e = unsafe { &*(header as *const clap_event_header).cast::<clap_event_note>() };
            Event::Note {
                kind: header.type_,
                id: e.note_id,
                port: e.port_index,
                channel: e.channel,
                key: e.key,
                velocity: e.velocity,
                flags: header.flags,
            }
        }
        CLAP_EVENT_NOTE_EXPRESSION => {
            let e = unsafe {
                &*(header as *const clap_event_header).cast::<clap_event_note_expression>()
            };
            Event::Expression {
                kind: e.expression_id,
                id: e.note_id,
                port: e.port_index,
                channel: e.channel,
                key: e.key,
                value: e.value,
                flags: header.flags,
            }
        }
        CLAP_EVENT_MIDI => {
            let e = unsafe { &*(header as *const clap_event_header).cast::<clap_event_midi>() };
            Event::Midi { port: e.port_index, data: e.data, flags: header.flags }
        }
        _ => return !reject,
    };
    if reject {
        assert!(sink.rejected.len() < sink.rejected.capacity());
        sink.rejected.push((header.time, value));
        return false;
    }
    assert!(sink.values.len() < sink.values.capacity());
    sink.values.push((header.time, value));
    true
}
/// The one parameter the Tune exports.
const DELAY_PARAM: u32 = 0;
struct Device {
    plugin: *const clap_plugin,
    _host: Box<clap_host>,
    _stats: Box<Host>,
    tuner: bool,
    active: bool,
}
impl Device {
    fn new(tuner: bool) -> Self {
        let mut stats = Box::<Host>::default();
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: (&mut *stats as *mut Host).cast(),
            name: c"Aggregation fixture".as_ptr(),
            vendor: c"test".as_ptr(),
            url: c"".as_ptr(),
            version: c"1".as_ptr(),
            get_extension: Some(extension),
            request_restart: Some(restart),
            request_process: Some(request),
            request_callback: Some(callback),
        });
        let plugin = unsafe {
            factory().create_plugin.unwrap()(
                factory(),
                &*host,
                if tuner { c"com.yan-h.harmonigraph-tune" } else { c"com.yan-h.harmonigraph" }
                    .as_ptr(),
            )
        };
        assert!(!plugin.is_null());
        stats.plugin.store(plugin.cast_mut(), Ordering::Relaxed);
        assert!(unsafe { (*plugin).init.unwrap()(plugin) });
        Self { plugin, _host: host, _stats: stats, tuner, active: false }
    }
    /// D is `multiplier x max_frames`, so the format a fixture activates at is
    /// also the delay it plays against: 512 frames at the default 1x is the
    /// 512 samples this delay used to be fixed at. Callbacks stay whatever
    /// size the fixture drives; this is only the advertised maximum.
    fn activate(&mut self) {
        self.activate_format(48000.0, 512);
    }
    fn activate_format(&mut self, rate: f64, frames: u32) {
        self._stats.activating.store(true, Ordering::Relaxed);
        let activated = unsafe { (*self.plugin).activate.unwrap()(self.plugin, rate, 1, frames) };
        self._stats.activating.store(false, Ordering::Relaxed);
        assert!(activated);
        assert!(unsafe { (*self.plugin).start_processing.unwrap()(self.plugin) });
        self.active = true;
    }
    fn deactivate(&mut self) {
        assert!(self.active);
        unsafe {
            (*self.plugin).stop_processing.unwrap()(self.plugin);
            (*self.plugin).deactivate.unwrap()(self.plugin);
        }
        self.active = false;
    }
    fn reactivate_format(&mut self, rate: f64, frames: u32) {
        self.deactivate();
        self.activate_format(rate, frames);
    }
    fn recorded_hub() -> (Self, harmonigraph_record::testing::Capture) {
        let (recorder, capture) = harmonigraph_record::testing::channel();
        crate::configuration::inject_recorder(recorder);
        (Self::new(false), capture)
    }
    fn main(&self) {
        unsafe {
            (*self.plugin).on_main_thread.unwrap()(self.plugin);
        }
    }
    fn state_api(&self) -> &clap_plugin_state {
        unsafe {
            &*((*self.plugin).get_extension.unwrap()(self.plugin, CLAP_EXT_STATE.as_ptr()).cast())
        }
    }
    fn params(&self) -> &clap_plugin_params {
        unsafe {
            &*((*self.plugin).get_extension.unwrap()(self.plugin, CLAP_EXT_PARAMS.as_ptr()).cast())
        }
    }
    fn latency(&self) -> u32 {
        latency_of(self.plugin)
    }
    fn param_id(&self, index: u32) -> u32 {
        assert!(self.tuner);
        let mut info = unsafe { std::mem::zeroed() };
        assert!(unsafe { self.params().get_info.unwrap()(self.plugin, index, &mut info) });
        info.id
    }
    /// Automation for the Tune's parameter at `index`, carrying the value a
    /// host sends: a stepped parameter's CLAP value is its step index, so
    /// Participating is 0 or 1 and the tuning delay's 1x buffer is a zero.
    fn param_event(&self, index: u32, value: f64, time: u32) -> Input {
        Input::Param(clap_event_param_value {
            header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, time),
            param_id: self.param_id(index),
            cookie: ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value,
        })
    }
    /// What the host reads back for that parameter, in the same units.
    fn param_value(&self, index: u32) -> f64 {
        let mut value = 0.0;
        assert!(unsafe {
            self.params().get_value.unwrap()(self.plugin, self.param_id(index), &mut value)
        });
        value
    }
    fn shared(&self) -> std::sync::Arc<setup::Shared> {
        if self.tuner {
            let wrapper = unsafe {
                &*((*self.plugin)
                    .plugin_data
                    .cast::<nice_plug::wrapper::clap::Wrapper<plugin::HarmonigraphTune>>())
            };
            wrapper.test_inspect_plugin(|plugin| plugin.shared.clone())
        } else {
            let wrapper = unsafe {
                &*((*self.plugin)
                    .plugin_data
                    .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
            };
            wrapper
                .test_inspect_plugin(|plugin| plugin.aggregation.as_ref().unwrap().shared.clone())
        }
    }
    fn load(&self, state: &PluginState) -> bool {
        let bytes = serde_json::to_vec(state).unwrap();
        let mut framed = (bytes.len() as u64).to_le_bytes().to_vec();
        framed.extend(bytes);
        let mut reader = (framed.as_slice(), 0usize);
        let stream =
            clap_istream { ctx: (&mut reader as *mut (&[u8], usize)).cast(), read: Some(read) };
        unsafe { self.state_api().load.unwrap()(self.plugin, &stream) }
    }
    fn save(&self) -> PluginState {
        let mut bytes = Vec::new();
        let stream = clap_ostream { ctx: (&mut bytes as *mut Vec<u8>).cast(), write: Some(write) };
        assert!(unsafe { self.state_api().save.unwrap()(self.plugin, &stream) });
        serde_json::from_slice(&bytes[8..]).unwrap()
    }
    fn run(&self, raw: i64, events: Vec<Input>, reject_kind: Option<u16>) -> Sink {
        self.run_select(raw, events, reject_kind, None)
    }
    fn run_select(
        &self,
        raw: i64,
        events: Vec<Input>,
        reject_kind: Option<u16>,
        reject_attempt: Option<usize>,
    ) -> Sink {
        self.run_format(raw, events, reject_kind, reject_attempt, 64)
    }
    fn run_format(
        &self,
        raw: i64,
        events: Vec<Input>,
        reject_kind: Option<u16>,
        reject_attempt: Option<usize>,
        frames: u32,
    ) -> Sink {
        self.run_status(raw, events, reject_kind, reject_attempt, frames, false)
    }
    fn run_status(
        &self,
        raw: i64,
        events: Vec<Input>,
        reject_kind: Option<u16>,
        reject_attempt: Option<usize>,
        frames: u32,
        expect_error: bool,
    ) -> Sink {
        self.run_callback(raw, events, (reject_kind, reject_attempt), (frames, expect_error), None)
    }
    fn run_callback(
        &self,
        raw: i64,
        events: Vec<Input>,
        rejection: (Option<u16>, Option<usize>),
        format: (u32, bool),
        transport: Option<clap_event_transport>,
    ) -> Sink {
        let (reject_kind, reject_attempt) = rejection;
        let (frames, expect_error) = format;
        assert!(frames > 0 && frames <= 512);
        let input = clap_input_events {
            ctx: (&events as *const Vec<Input>).cast_mut().cast(),
            size: Some(size),
            get: Some(get),
        };
        let mut sink = Sink {
            values: Vec::with_capacity(640),
            rejected: Vec::with_capacity(640),
            attempts: 0,
            reject_kind,
            reject_attempt,
            callback_nanos: 0,
        };
        let output =
            clap_output_events { ctx: (&mut sink as *mut Sink).cast(), try_push: Some(push) };
        let mut left = [0f32; 512];
        let mut right = [0f32; 512];
        let mut side_left = [0f32; 512];
        let mut side_right = [0f32; 512];
        let mut channels = [left.as_mut_ptr(), right.as_mut_ptr()];
        let mut side = [side_left.as_mut_ptr(), side_right.as_mut_ptr()];
        let audio = clap_audio_buffer {
            data32: channels.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let inputs = [audio, clap_audio_buffer { data32: side.as_mut_ptr(), ..audio }];
        let mut out_audio = audio;
        let process = clap_process {
            steady_time: raw,
            frames_count: frames,
            transport: transport.as_ref().map_or(ptr::null(), |value| value as *const _),
            audio_inputs: if self.tuner { ptr::null() } else { inputs.as_ptr() },
            audio_outputs: if self.tuner { ptr::null_mut() } else { &mut out_audio },
            audio_inputs_count: if self.tuner { 0 } else { 2 },
            audio_outputs_count: if self.tuner { 0 } else { 1 },
            in_events: &input,
            out_events: &output,
        };
        // assert_process_allocs guards this entire exported callback, including
        // the wrapper. Nesting a second guard defeats its diagnostic permit.
        let started = std::time::Instant::now();
        let status = unsafe { (*self.plugin).process.unwrap()(self.plugin, &process) };
        sink.callback_nanos = started.elapsed().as_nanos();
        assert_eq!(status == CLAP_PROCESS_ERROR, expect_error);
        sink
    }
}
impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            if self.active {
                (*self.plugin).stop_processing.unwrap()(self.plugin);
                (*self.plugin).deactivate.unwrap()(self.plugin);
            }
            (*self.plugin).destroy.unwrap()(self.plugin);
        }
    }
}
unsafe extern "C" fn read(stream: *const clap_istream, out: *mut c_void, size: u64) -> i64 {
    let reader = unsafe { &mut *((*stream).ctx.cast::<(&[u8], usize)>()) };
    let count = (size as usize).min(reader.0.len() - reader.1);
    unsafe {
        ptr::copy_nonoverlapping(reader.0[reader.1..].as_ptr(), out.cast(), count);
    }
    reader.1 += count;
    count as i64
}
unsafe extern "C" fn write(stream: *const clap_ostream, input: *const c_void, size: u64) -> i64 {
    unsafe { &mut *((*stream).ctx.cast::<Vec<u8>>()) }.extend_from_slice(unsafe {
        std::slice::from_raw_parts(input.cast::<u8>(), size as usize)
    });
    size as i64
}

