//! Production exported-factory fixtures. The real CLAP entry point, real
//! endpoint capacities and real recorder fanout, without opening an editor.
use super::*;
use clap_sys::{
    audio_buffer::clap_audio_buffer,
    events::*,
    ext::{
        latency::{clap_host_latency, clap_plugin_latency, CLAP_EXT_LATENCY},
        params::{clap_plugin_params, CLAP_EXT_PARAMS},
    },
    factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID},
    host::clap_host,
    plugin::clap_plugin,
    process::*,
    version::CLAP_VERSION,
};
use event::Event;

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
fn inspect_tune<R>(device: &Device, f: impl FnOnce(&tune::Tune) -> R) -> R {
    assert!(device.tuner);
    let wrapper = unsafe {
        &*((*device.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<plugin::HarmonigraphTune>>())
    };
    wrapper.test_inspect_plugin(|plugin| f(plugin.tune.as_ref().unwrap()))
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
    /// `clap_plugin::reset`, which a host calls on one instance when it wants
    /// that instance's state discarded. It reaches this module as
    /// `clap_performance_reset`.
    fn host_reset(&self) {
        unsafe {
            (*self.plugin).reset.unwrap()(self.plugin);
        }
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
        assert_eq!(
            status == CLAP_PROCESS_ERROR,
            expect_error,
            "tuner={} raw={raw} frames={frames}",
            self.tuner
        );
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

/// One Hub and one Tune, at 48 kHz and 512-frame callbacks, so D is 512
/// samples at the default 1x. `step` runs the Tune and then the Hub, which is
/// the order that lets a 1x note make its own deadline.
struct Pair {
    hub: Device,
    tune: Device,
    raw: i64,
}
impl Pair {
    fn new() -> Self {
        let mut hub = Device::new(false);
        hub.activate();
        let mut tune = Device::new(true);
        tune.activate();
        Self { hub, tune, raw: 0 }
    }
    fn step(&mut self, input: Vec<Input>) -> Vec<(u32, Event)> {
        let sink = self.tune.run(self.raw, input, None);
        self.hub.run(self.raw, vec![], None);
        self.raw += 512;
        sink.values
    }
    fn idle(&mut self) -> Vec<(u32, Event)> {
        self.step(vec![])
    }
    fn misses(&self) -> u64 {
        self.tune.shared().misses.load(Ordering::Relaxed)
    }
    fn status(&self) -> u32 {
        self.tune.shared().status()
    }
}
fn tuning_of(output: &[(u32, Event)]) -> Option<f64> {
    output.iter().find_map(|(_, event)| match event {
        Event::Expression { kind: 2, value, .. } => Some(*value),
        _ => None,
    })
}

/// The whole of the delay contract: an input emits at its own sample plus D,
/// with the correction that came back for it.
#[test]
fn an_input_emits_at_its_own_sample_plus_the_delay_with_its_correction() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    assert!(pair.step(vec![note(1, 0, 60, 17, true)]).is_empty(), "nothing emits before D");
    let output = pair.idle();
    assert_eq!(output.len(), 2, "the note and the tuning expression that states it: {output:?}");
    assert_eq!(output[0].0, 17, "input offset 17 plus D lands at offset 17 of the next callback");
    assert!(tuning_of(&output).is_some());
    assert_eq!(pair.misses(), 0);
}

/// An onset whose correction has not arrived by its own deadline emits at raw
/// pitch and counts the miss. Nothing waits, and the next note is unaffected.
#[test]
fn an_onset_without_its_correction_emits_uncorrected_and_counts_the_miss() {
    let _scope = crate::test_scope::enter();
    // No Hub in the process at all: the request has nowhere to go, which is
    // the cheapest way to reach the uncorrected path from outside.
    let mut tune = Device::new(true);
    tune.activate();
    assert!(tune.run(0, vec![note(1, 0, 60, 0, true)], None).values.is_empty());
    let output = tune.run(512, vec![], None).values;
    assert_eq!(output.len(), 1, "the note goes out on time, alone: {output:?}");
    assert!(tuning_of(&output).is_none(), "no correction is stated for a note that has none");
    // An unpaired Tune is not a missed deadline: a larger delay is no remedy
    // for a Hub that is not there, so the status says so instead.
    assert_eq!(tune.shared().misses.load(Ordering::Relaxed), 0);
    assert_ne!(tune.shared().status() & session::NO_HUB, 0);
}

/// #718, retargeted. An unpaired Tune is an ordinary delay line: it forwards
/// everything it is given, at D, and never holds a note back for an answer
/// that is not coming.
#[test]
fn an_unpaired_tune_passes_notes_through_uncorrected() {
    let _scope = crate::test_scope::enter();
    let mut tune = Device::new(true);
    tune.activate();
    let phrase = [60, 64, 67];
    let mut emitted = Vec::new();
    for step in 0..phrase.len() + 2 {
        let input = phrase
            .get(step)
            .map(|key| vec![note(step as i32, 0, *key, 0, true)])
            .unwrap_or_default();
        emitted.extend(tune.run_format(step as i64 * 512, input, None, None, 512).values);
    }
    let keys: Vec<i16> = emitted
        .iter()
        .filter_map(|(_, event)| event.attack().map(|(_, _, key, _)| i16::from(key)))
        .collect();
    assert_eq!(keys, phrase, "every note reached the wire, in order: {emitted:?}");
    assert!(tuning_of(&emitted).is_none());
    assert_ne!(tune.shared().status() & session::NO_HUB, 0);
}

/// A full ring drops that note from the Hub's context, so it sounds
/// uncorrected and counts as a miss. The ring is what overflows; the note is
/// not held back and nothing latches.
#[test]
fn a_full_copy_ring_costs_a_correction_rather_than_a_note() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    let mut input: Vec<_> =
        (0..CAPTURE_RING).map(|index| raw_midi([0xb0, 20, (index % 128) as u8], 0)).collect();
    input.push(note(1, 0, 60, 0, true));
    pair.step(input);
    let mut output = Vec::new();
    for _ in 0..6 {
        output.extend(pair.idle());
    }
    let attacks: Vec<_> = output.iter().filter(|(_, event)| event.attack().is_some()).collect();
    assert_eq!(attacks.len(), 1, "the note itself is never the thing that is dropped");
    assert!(tuning_of(&output).is_none(), "its copy never reached the Hub");
    assert_eq!(pair.misses(), 1);
    assert_ne!(pair.status() & session::RING_FULL, 0);
}

/// A reply reaches the onset with its serial or it reaches nothing. After a
/// cut the epoch has moved, so an answer minted before it is discarded rather
/// than attached to whatever now holds that serial.
#[test]
fn a_reply_from_before_the_cut_is_discarded_rather_than_attached() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    // Ask, then cut before the answer can be drained.
    pair.tune.run(pair.raw, vec![note(1, 0, 60, 0, true)], None);
    pair.hub.run(pair.raw, vec![], None);
    pair.raw += 512;
    pair.tune.shared().request_reset();
    pair.tune.main();
    let output = pair.idle();
    assert!(
        tuning_of(&output).is_none(),
        "the correction was minted under the epoch the cut ended: {output:?}"
    );
    // And the session is playable again immediately afterwards.
    pair.step(vec![note(2, 0, 62, 0, true)]);
    let resumed = pair.idle();
    assert!(tuning_of(&resumed).is_some(), "the next note is corrected: {resumed:?}");
}

/// #787, retargeted. The Apply that used to park a row behind ten wait
/// reasons is now one epoch bump: it cuts what is sounding, and the very next
/// phrase is corrected again. Audio resuming is the whole assertion.
#[test]
fn reset_cuts_and_audio_resumes_within_two_callbacks() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    pair.step(vec![note(1, 0, 60, 0, true)]);
    let sounding = pair.idle();
    assert!(tuning_of(&sounding).is_some(), "the fixture reaches the Reset with a note held");
    pair.tune.shared().request_reset();
    pair.tune.main();
    // The cut owes a Note-Off for the voice it just ended.
    let cut = pair.idle();
    assert!(
        cut.iter().any(|(_, event)| event.release()),
        "the cut releases what it forgot it was holding: {cut:?}"
    );
    for step in 0..2 {
        pair.step(vec![note(10 + step, 0, 64, 0, true)]);
        let output = pair.idle();
        if tuning_of(&output).is_some() {
            return;
        }
    }
    panic!("audio must resume within two callbacks of the cut");
}

/// The cut ends every held voice and neutralises the pedals it left down.
/// A pedal still holding after its note-off would hold the very voices the
/// cut just released.
#[test]
fn the_cut_releases_every_held_voice_and_neutralises_the_pedals() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    pair.step(vec![raw_midi([0xb0, 64, 127], 0), note(1, 0, 60, 1, true), note(2, 0, 64, 2, true)]);
    let sounding = pair.idle();
    assert_eq!(
        sounding.iter().filter(|(_, event)| event.attack().is_some()).count(),
        2,
        "the fixture reaches the cut with two voices and a pedal down: {sounding:?}"
    );
    pair.tune.shared().request_reset();
    pair.tune.main();
    let cut = pair.idle();
    assert_eq!(
        cut.iter().filter(|(_, event)| event.release()).count(),
        2,
        "one Note-Off per held voice: {cut:?}"
    );
    assert!(
        cut.iter().any(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 64, 0], .. })),
        "sustain is put back down to neutral: {cut:?}"
    );
}

/// No coverage wait. A record whose sample the Hub has already sequenced is
/// assigned when it arrives, rather than establishing an interval nobody can
/// complete.
#[test]
fn a_record_arriving_after_its_sample_was_sequenced_is_still_assigned() {
    let _scope = crate::test_scope::enter();
    let mut hub = Device::new(false);
    hub.activate();
    let mut early = Device::new(true);
    early.activate();
    let mut late = Device::new(true);
    late.activate();
    // The early Tune plays and the Hub sequences that sample.
    early.run(0, vec![note(1, 0, 60, 0, true)], None);
    hub.run(0, vec![], None);
    // The late Tune's copy for the SAME sample only reaches the Hub now.
    late.run(0, vec![note(2, 0, 64, 0, true)], None);
    hub.run(512, vec![], None);
    let output = late.run(512, vec![], None).values;
    assert!(
        tuning_of(&output).is_some(),
        "the late record is assigned when it arrives, not refused: {output:?}"
    );
}

/// Membership is the epoch. A Tune attaching cuts every paired track, which
/// is the price of never having a hot-plug reconciliation to get wrong.
#[test]
fn a_tune_attaching_cuts_every_track_already_playing() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    pair.step(vec![note(1, 0, 60, 0, true)]);
    assert!(tuning_of(&pair.idle()).is_some());
    let mut joining = Device::new(true);
    joining.activate();
    let cut = pair.idle();
    assert!(
        cut.iter().any(|(_, event)| event.release()),
        "the attach cut the voice the first Tune was holding: {cut:?}"
    );
    drop(joining);
}

/// #757 and #672, retargeted. Destruction is one main-thread release: the row
/// goes back, and the next Tune takes it. There is nothing to settle, so
/// there is nothing that can fail to.
#[test]
fn destroy_leaves_no_registry_entry() {
    let _scope = crate::test_scope::enter();
    let session = session::session();
    let mut hub = Device::new(false);
    hub.activate();
    assert_ne!(session.hub(), 0);
    for _ in 0..3 {
        let mut tune = Device::new(true);
        tune.activate();
        let held: Vec<_> = (0..TUNERS as u8).filter(|slot| session.row(*slot).held()).collect();
        assert_eq!(held.len(), 1, "exactly one row is claimed at a time");
        tune.run(0, vec![note(1, 0, 60, 0, true)], None);
        drop(tune);
        assert!(
            (0..TUNERS as u8).all(|slot| !session.row(slot).held()),
            "the row goes back at destruction"
        );
    }
    drop(hub);
    assert_eq!(session.hub(), 0, "and so does the Hub's own slot");
    assert_eq!(session.hubs(), 0);
}

/// A second Harmonigraph is a status on both rather than a pairing choice.
/// Neither of them stops passing notes for it.
#[test]
fn a_second_hub_is_a_fault_status_and_never_a_silence() {
    let _scope = crate::test_scope::enter();
    let mut first = Device::new(false);
    first.activate();
    let mut second = Device::new(false);
    second.activate();
    let mut tune = Device::new(true);
    tune.activate();
    tune.run(0, vec![note(1, 0, 60, 0, true)], None);
    first.run(0, vec![], None);
    second.run(0, vec![], None);
    let output = tune.run(512, vec![], None).values;
    assert_eq!(output.len(), 2, "the note is still corrected by the Hub that owns the session");
    assert_ne!(tune.shared().status() & session::SECOND_HUB, 0);
    assert_ne!(first.shared().status() & session::SECOND_HUB, 0);
}

/// The delay is `multiplier x advertised maximum frames`, resolved at
/// activation and reported to the host as latency before any Hub exists.
#[test]
fn the_delay_is_reported_as_latency_from_the_saved_multiplier_alone() {
    let _scope = crate::test_scope::enter();
    let mut tune = Device::new(true);
    assert_eq!(tune.latency(), 0, "no activation has advertised a format yet");
    tune.activate_format(48000.0, 256);
    assert_eq!(tune.latency(), 256);
    // A stepped parameter's CLAP value is its step index, so 3 is 4x buffer.
    tune.run(0, vec![tune.param_event(DELAY_PARAM, 3.0, 0)], None);
    tune.reactivate_format(48000.0, 256);
    assert_eq!(tune.latency(), 4 * 256, "the new multiplier is adopted at the reactivation");
    // The round trip is through a normalized f32, so this is the step index
    // rather than an exact double.
    assert_eq!(tune.param_value(DELAY_PARAM).round(), 3.0, "and the host reads back what it set");
}

/// A host format change is a cut: the notes standing in the line were
/// scheduled against a delay that no longer exists. It is the SESSION's cut,
/// not the reactivating Tune's own — the Note-Offs it owes go to the host and
/// never into the ring, so the Hub learns what ended only from the epoch.
#[test]
fn a_host_format_change_cuts_and_adopts_the_new_delay() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    pair.step(vec![note(1, 0, 60, 0, true)]);
    assert!(tuning_of(&pair.idle()).is_some());
    assert_eq!(
        inspect_hub(&pair.hub, |hub| hub.test_held(0)),
        1,
        "the fixture reaches the format change with the Hub holding that voice"
    );
    pair.tune.reactivate_format(48000.0, 128);
    let cut = pair.idle();
    assert!(
        cut.iter().any(|(_, event)| event.release()),
        "reactivation released the voice it stopped being able to schedule: {cut:?}"
    );
    assert_eq!(
        inspect_hub(&pair.hub, |hub| hub.test_held(0)),
        0,
        "and the Hub let go of it rather than drawing a note that has ended"
    );
    assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_context()), 0);
    pair.step(vec![note(2, 0, 62, 0, true)]);
    let output = pair.idle();
    assert!(tuning_of(&output).is_some(), "and the next note is corrected at the new D");
}

/// A host reset on one Tune is the same cut by another trigger, and owes the
/// Hub the same news. Nothing else in the process is being reset.
#[test]
fn a_host_reset_on_one_tune_cuts_it_out_of_the_hubs_context() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    pair.step(vec![note(1, 0, 60, 0, true)]);
    assert!(tuning_of(&pair.idle()).is_some());
    assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_held(0)), 1, "the Hub is holding the voice");
    pair.tune.host_reset();
    let cut = pair.idle();
    assert!(
        cut.iter().any(|(_, event)| event.release()),
        "the reset released what the Tune was holding: {cut:?}"
    );
    assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_held(0)), 0);
    assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_context()), 0);
    pair.step(vec![note(2, 0, 62, 0, true)]);
    assert!(tuning_of(&pair.idle()).is_some(), "and the session plays on");
}

/// The Reset classification is "has one happened since I adopted", not "is the
/// newest bump a Reset". A Reset and an attach landing between two Hub
/// callbacks is still a Reset, and the memory it was pressed to clear must not
/// ride out on the attach's epoch.
#[test]
fn a_reset_with_an_attach_behind_it_still_clears_released_memory() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    pair.step(vec![note(1, 0, 60, 0, true), note(2, 0, 64, 1, true)]);
    pair.idle();
    pair.step(vec![note(1, 0, 60, 0, false), note(2, 0, 64, 1, false)]);
    for _ in 0..2 {
        pair.idle();
    }
    // Nothing is held any more, so what the Hub still publishes as context is
    // exactly the released memory a Reset is meant to clear.
    let remembered = inspect_hub(&pair.hub, |hub| hub.test_next_context());
    assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_context()), 0, "no voice is still held");
    assert!(
        !remembered.context.is_empty(),
        "the fixture reaches the Reset with released memory to clear: {remembered:?}"
    );
    // The Reset and the attach land in the same window: the Tune turns the
    // request into the session's Reset on its callback, a second Tune joins,
    // and only then does the Hub get a callback to adopt any of it.
    pair.tune.shared().request_reset();
    pair.tune.main();
    pair.tune.run(pair.raw, vec![], None);
    let mut joining = Device::new(true);
    joining.activate();
    pair.hub.run(pair.raw, vec![], None);
    pair.raw += 512;
    let after = inspect_hub(&pair.hub, |hub| hub.test_next_context());
    assert!(
        after.context.is_empty(),
        "the attach behind the Reset did not reclassify it: {after:?}"
    );
    drop(joining);
}

/// The two publication lanes stay independent, and the display shows what the
/// Hub scheduled: input plus that source's D, not the sample it arrived at.
#[test]
fn the_display_shows_the_schedule_rather_than_the_input() {
    let _scope = crate::test_scope::enter();
    let (mut hub, capture) = Device::recorded_hub();
    hub.activate();
    let mut tune = Device::new(true);
    tune.activate();
    tune.run_format(0, vec![note(1, 0, 60, 64, true)], None, None, 512);
    hub.run_format(0, vec![], None, None, 512);
    let mut capture = capture;
    let onset = capture
        .display_events()
        .into_iter()
        .find_map(|record| match record {
            harmonigraph_take::CanonicalRecord::Delta(delta)
                if matches!(delta.event.kind, harmonigraph_take::NoteKind::On { .. }) =>
            {
                delta.timing
            }
            _ => None,
        })
        .expect("the onset reached the display lane with its own timing");
    assert_eq!(onset.input, 64, "the sample it arrived at");
    assert_eq!(onset.sample, 64 + 512, "and the sample it is scheduled to sound at");
    assert_eq!(onset.planned, Some(onset.sample));
}

/// Both a cohort still wholly pending and a sounding set reach the actual
/// 64-cell ceiling. Refused attacks reach neither the wire nor publication;
/// every admitted voice keeps its frozen expression and cut ownership.
#[test]
fn an_onset_past_the_held_ceiling_is_refused_before_copy_and_output() {
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.activate();
    let mut tune = Device::new(true);
    tune.activate();
    let mut pair = Pair { hub, tune, raw: 0 };
    let input: Vec<_> =
        (0..HELD_PER_SOURCE + 1).map(|key| note(key as i32, 0, key as i16, 0, true)).collect();
    assert!(pair.step(input).is_empty());
    assert_eq!(
        inspect_tune(&pair.tune, |tune| (tune.held(), tune.pending(), tune.captured)),
        (0, HELD_PER_SOURCE, HELD_PER_SOURCE as u64)
    );
    let output = pair.idle();
    let keys: Vec<_> =
        output.iter().filter_map(|(_, e)| e.attack().map(|(_, _, key, _)| key)).collect();
    assert_eq!(keys, (0..HELD_PER_SOURCE as u8).collect::<Vec<_>>());
    assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_held(0)), HELD_PER_SOURCE);
    pair.step(vec![note(65, 0, 65, 0, true)]);
    assert!(pair.idle().is_empty(), "a full sounding set refuses the next onset too");
    assert_eq!(inspect_tune(&pair.tune, |tune| tune.captured), HELD_PER_SOURCE as u64);
    assert_ne!(pair.status() & session::DROPPED, 0);
    let published: Vec<_> = capture
        .display_events()
        .into_iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta) if matches!(delta.event.kind, NoteKind::On { .. }) => {
                Some(delta.event.note)
            }
            _ => None,
        })
        .collect();
    assert_eq!(published, keys, "only admitted attacks enter publication");
    let (id, correction) = output
        .iter()
        .find_map(|(_, e)| match e {
            Event::Expression { id, value, .. } if value.abs() > 0.001 => Some((*id, *value)),
            _ => None,
        })
        .expect("the fixture must exercise a nonzero frozen correction");
    pair.step(vec![expression(id, 0.25, 0)]);
    assert!((tuning_of(&pair.idle()).unwrap() - (0.25 + correction)).abs() < 1e-9);
    pair.tune.shared().request_reset();
    pair.tune.main();
    let cut = pair.idle();
    let ended: Vec<_> = cut
        .iter()
        .filter_map(|(_, e)| match e {
            Event::Note { kind: 1, key, .. } => Some(*key as u8),
            _ => None,
        })
        .collect();
    assert_eq!(ended, keys, "every sounded voice is in the cut inventory");
    assert_eq!(inspect_hub(&pair.hub, |hub| (hub.test_held(0), hub.test_context())), (0, 0));
}

/// The admission replay must use the same identity rules as emission, even
/// before any onset has sounded: replacement ids, stale releases, wildcard
/// addressing and both channel-ending controllers all run through real input.
#[test]
fn pending_retriggers_and_endings_reuse_held_capacity_in_wire_order() {
    let _scope = crate::test_scope::enter();
    for controller in [None, Some(120), Some(123)] {
        let mut pair = Pair::new();
        let mut input: Vec<_> =
            (0..HELD_PER_SOURCE).map(|key| note(key as i32, 0, key as i16, 0, true)).collect();
        let Input::Note(mut invalid_release) = note(1, 0, 1, 2, false) else { unreachable!() };
        invalid_release.velocity = f64::NAN;
        input.extend([
            note(100, 0, 0, 1, true),
            Input::Note(invalid_release),
            note(0, 0, 0, 2, false),
            note(200, 0, 64, 3, true),
        ]);
        input.push(match controller {
            Some(cc) => raw_midi([0xb0, cc, 0], 4),
            None => note(100, -1, -1, 4, false),
        });
        input.push(note(201, 0, 64, 5, true));
        let expected_copies = input.len() - 2; // Invalid release and overflow attack.
        pair.step(input);
        assert_eq!(inspect_tune(&pair.tune, |tune| tune.captured), expected_copies as u64);
        let output = pair.idle();
        let ids: Vec<_> =
            output.iter().filter_map(|(_, e)| e.attack().map(|(id, _, _, _)| id)).collect();
        assert_eq!(ids, (0..HELD_PER_SOURCE as i32).chain([100, 201]).collect::<Vec<_>>());
        assert!(output
            .iter()
            .any(|(time, e)| *time == 4 && (e.release() || e.channel_termination().is_some())));
        let held = if controller.is_some() { 1 } else { HELD_PER_SOURCE };
        assert_eq!(inspect_tune(&pair.tune, |tune| tune.held()), held);
        assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_held(0)), held);
        assert_eq!(
            inspect_hub(&pair.hub, |hub| hub.test_voice(0, 0, 64).unwrap().host_note_id),
            201
        );
        pair.tune.host_reset();
        let cut = pair.idle();
        assert_eq!(cut.iter().filter(|(_, e)| e.release()).count(), held);
        assert!(cut.iter().any(|(_, e)| matches!(e, Event::Note { kind: 1, id: 201, .. })));
    }
}

/// Losing a release copy can leave the Hub at capacity after the Tune has
/// room. The Hub must refuse before sending a correction or changing context;
/// the locally admitted note still sounds raw and retains cut ownership.
#[test]
fn a_full_hub_refuses_assignment_before_replying_to_a_locally_admitted_note() {
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.activate();
    let mut tune = Device::new(true);
    tune.activate();
    let mut pair = Pair { hub, tune, raw: 0 };
    pair.step((0..HELD_PER_SOURCE).map(|key| note(key as i32, 0, key as i16, 0, true)).collect());
    assert_eq!(pair.idle().iter().filter(|(_, e)| e.attack().is_some()).count(), HELD_PER_SOURCE);
    let mut input: Vec<_> = (0..CAPTURE_RING).map(|_| raw_midi([0xb0, 20, 0], 0)).collect();
    input.push(note(0, 0, 0, 1, false));
    pair.step(input);
    let mut output = Vec::new();
    for _ in 0..6 {
        output.extend(pair.idle());
    }
    assert!(output.iter().any(|(_, e)| matches!(e, Event::Note { kind: 1, id: 0, .. })));
    assert_eq!(inspect_tune(&pair.tune, |tune| tune.held()), HELD_PER_SOURCE - 1);
    assert_eq!(
        inspect_hub(&pair.hub, |hub| (hub.test_held(0), hub.test_context())),
        (HELD_PER_SOURCE, HELD_PER_SOURCE)
    );
    capture.display_events();
    pair.step(vec![note(64, 0, 64, 0, true)]);
    let output = pair.idle();
    assert_eq!(output.len(), 1, "no correction reply accompanies the locally tracked onset");
    assert_eq!(output[0].1.attack().map(|(id, _, _, _)| id), Some(64));
    assert_eq!(pair.misses(), 1);
    assert_ne!(pair.hub.shared().status() & session::DROPPED, 0);
    assert_eq!(inspect_tune(&pair.tune, |tune| tune.held()), HELD_PER_SOURCE);
    assert_eq!(
        inspect_hub(&pair.hub, |hub| (hub.test_held(0), hub.test_context())),
        (HELD_PER_SOURCE, HELD_PER_SOURCE)
    );
    assert!(capture.display_events().is_empty(), "the Hub does not publish a refused assignment");
    pair.tune.host_reset();
    let cut = pair.idle();
    assert_eq!(cut.iter().filter(|(_, e)| e.release()).count(), HELD_PER_SOURCE);
    assert!(cut.iter().any(|(_, e)| matches!(e, Event::Note { kind: 1, id: 64, .. })));
    assert!(!cut.iter().any(|(_, e)| matches!(e, Event::Note { kind: 1, id: 0, .. })));
}

/// Fill the real 8192-entry line across callbacks while the Hub drains each
/// 1024-entry copy ring. Neither a new onset nor an existing voice's release
/// may leak to Hub state/publication when local retention fails.
#[test]
fn a_full_local_line_refuses_onset_and_release_before_copying() {
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.activate();
    let mut tune = Device::new(true);
    tune.activate();
    tune.run(0, vec![tune.param_event(DELAY_PARAM, 15.0, 0)], None);
    tune.reactivate_format(48000.0, 512);
    assert_eq!(tune.latency(), 16 * 512);
    let mut pair = Pair { hub, tune, raw: 512 };
    pair.step(vec![note(1, 0, 60, 0, true)]);
    let mut sounding = Vec::new();
    for _ in 0..16 {
        sounding.extend(pair.idle());
    }
    assert_eq!(sounding.iter().filter(|(_, e)| e.attack().is_some()).count(), 1);
    capture.display_events();
    assert_eq!(PENDING_EVENTS % CAPTURE_RING, 0);
    for _ in 0..PENDING_EVENTS / CAPTURE_RING {
        assert!(pair
            .step((0..CAPTURE_RING).map(|_| raw_midi([0xb0, 20, 0], 0)).collect())
            .is_empty());
    }
    assert_eq!(inspect_tune(&pair.tune, |tune| tune.pending()), PENDING_EVENTS);
    assert_eq!(pair.status() & session::RING_FULL, 0, "the Hub ring is not the ceiling reached");
    let copied = inspect_tune(&pair.tune, |tune| tune.captured);
    pair.step(vec![note(2, 0, 64, 0, true), note(1, 0, 60, 1, false)]);
    assert_eq!(
        inspect_tune(&pair.tune, |tune| (tune.pending(), tune.captured)),
        (PENDING_EVENTS, copied)
    );
    assert_ne!(pair.status() & session::DROPPED, 0);
    assert!(capture.display_events().is_empty(), "neither refusal publishes an onset or release");
    assert!(inspect_hub(&pair.hub, |hub| hub.test_voice(0, 0, 60)).is_some());
    assert!(inspect_hub(&pair.hub, |hub| hub.test_voice(0, 0, 64)).is_none());
    let mut output = Vec::new();
    for _ in 0..64 {
        output.extend(pair.idle());
    }
    assert_eq!(inspect_tune(&pair.tune, |tune| tune.pending()), 0);
    assert_eq!(output.len(), PENDING_EVENTS, "all locally admitted controllers drain");
    assert!(output.iter().all(|(_, e)| matches!(e, Event::Midi { data: [0xb0, 20, 0], .. })));
    pair.tune.host_reset();
    let cut = pair.idle();
    assert_eq!(cut.iter().filter(|(_, e)| e.release()).count(), 1);
    assert!(cut.iter().any(|(_, e)| matches!(e, Event::Note { kind: 1, id: 1, .. })));
    assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_held(0)), 0);
}

/// Missing steady time is rejected at the exported CLAP process boundary;
/// calling Tune::begin directly would miss the wrapper's input refusal.
#[test]
fn missing_steady_time_rejects_the_callback_before_note_admission() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    let output = pair.tune.run_status(-1, vec![note(1, 0, 60, 0, true)], None, None, 64, true);
    assert!(output.values.is_empty());
    assert_eq!(inspect_tune(&pair.tune, |tune| (tune.pending(), tune.captured)), (0, 0));
    assert_ne!(pair.status() & session::CLOCK, 0);
    pair.idle();
    assert!(pair.idle().is_empty(), "the invalid callback did not retain raw output for later");
    assert_eq!(inspect_hub(&pair.hub, |hub| hub.test_context()), 0);
}
