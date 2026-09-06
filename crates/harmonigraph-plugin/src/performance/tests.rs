//! Production exported-factory fixtures. Host acceptance, setup/state vtables,
//! real endpoint capacities and recorder fanout are used without opening editors.
use super::*;
use clap_sys::{
    audio_buffer::clap_audio_buffer,
    events::*,
    ext::{
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
use clock::Calibration;
use event::Event;
use nice_plug::plugin::{ParamValue, PluginState};
use routing::{HubSetup, SavedUuid, SourceSetup};
use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "attachment_tests.rs"]
mod attachment_tests;
#[path = "publication_tests.rs"]
mod publication_tests;

#[derive(Default)]
struct Host {
    callbacks: AtomicUsize,
}
unsafe extern "C" fn extension(_: *const clap_host, _: *const c_char) -> *const c_void {
    ptr::null()
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
    if sink.reject_kind == Some(header.type_)
        || sink.reject_kind == Some(u16::MAX)
        || sink.reject_attempt == Some(sink.attempts)
    {
        return false;
    }
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
        _ => return true,
    };
    assert!(sink.values.len() < sink.values.capacity());
    sink.values.push((header.time, value));
    true
}
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
            request_restart: Some(request),
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
        assert!(unsafe { (*plugin).init.unwrap()(plugin) });
        Self { plugin, _host: host, _stats: stats, tuner, active: false }
    }
    fn activate(&mut self) {
        self.activate_format(48000.0, 64);
    }
    fn activate_format(&mut self, rate: f64, frames: u32) {
        assert!(unsafe { (*self.plugin).activate.unwrap()(self.plugin, rate, 1, frames) });
        assert!(unsafe { (*self.plugin).start_processing.unwrap()(self.plugin) });
        self.active = true;
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
    fn participation(&self, value: bool, time: u32) -> Input {
        assert!(self.tuner);
        let mut info = unsafe { std::mem::zeroed() };
        assert!(unsafe { self.params().get_info.unwrap()(self.plugin, 0, &mut info) });
        Input::Param(clap_event_param_value {
            header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, time),
            param_id: info.id,
            cookie: ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value: f64::from(value),
        })
    }
    fn source_snapshot(&self) -> source::Snapshot {
        assert!(self.tuner);
        let wrapper = unsafe {
            &*((*self.plugin)
                .plugin_data
                .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
        };
        wrapper.test_inspect_plugin(|plugin| plugin.source.as_ref().unwrap().test_snapshot())
    }
    fn shared(&self) -> std::sync::Arc<setup::Shared> {
        if self.tuner {
            let wrapper = unsafe {
                &*((*self.plugin)
                    .plugin_data
                    .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
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
    fn configure(&self, uuid: SavedUuid, participating: bool) {
        self.configure_offset(uuid, participating, 0);
    }
    fn configure_offset(&self, uuid: SavedUuid, participating: bool, offset: i64) {
        self.configure_format(
            uuid,
            participating,
            Calibration { offset, sample_rate: 48000.0, max_frames: 64, validated: true },
        );
    }
    fn configure_format(&self, uuid: SavedUuid, participating: bool, calibration: Calibration) {
        let mut state = self.save();
        if self.tuner {
            state.params.insert(setup::PARTICIPATING.into(), ParamValue::Bool(participating));
            state.fields.insert(
                setup::SOURCE_FIELD.into(),
                serde_json::to_string(&SourceSetup { selected: Some(uuid), calibration }).unwrap(),
            );
        } else {
            state.fields.insert(
                setup::HUB_FIELD.into(),
                serde_json::to_string(&HubSetup { uuid, calibration }).unwrap(),
            );
        }
        assert!(self.load(&state));
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

#[test]
fn production_factory_exports_two_clap_classes_and_lightweight_tune_ports() {
    let _scope = crate::test_scope::enter();
    assert_eq!(unsafe { factory().get_plugin_count.unwrap()(factory()) }, 2);
    let descriptor = unsafe { &*factory().get_plugin_descriptor.unwrap()(factory(), 1) };
    assert_eq!(unsafe { CStr::from_ptr(descriptor.name) }, c"Harmonigraph Tune");
    let mut source = Device::new(true);
    assert_eq!(unsafe { source.params().count.unwrap()(source.plugin) }, 1);
    let voices = unsafe {
        (*source.plugin).get_extension.unwrap()(
            source.plugin,
            clap_sys::ext::voice_info::CLAP_EXT_VOICE_INFO.as_ptr(),
        )
    };
    assert!(voices.is_null());
    let audio = unsafe {
        &*((*source.plugin).get_extension.unwrap()(
            source.plugin,
            clap_sys::ext::audio_ports::CLAP_EXT_AUDIO_PORTS.as_ptr(),
        )
        .cast::<clap_sys::ext::audio_ports::clap_plugin_audio_ports>())
    };
    assert_eq!(unsafe { audio.count.unwrap()(source.plugin, true) }, 0);
    assert_eq!(unsafe { audio.count.unwrap()(source.plugin, false) }, 0);
    source.activate();
    assert!(source.run(0, vec![], None).values.is_empty());
}

#[test]
fn tuner_before_hub_retains_a_phrase_and_preserves_spacing_after_real_admission() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    assert!(source
        .run(
            0,
            vec![
                note(71, 2, 60, 7, true),
                expression(71, 0.1234567890123, 19),
                note(71, 2, 60, 39, false)
            ],
            None
        )
        .values
        .is_empty());
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    hub.run(0, vec![], None);
    source.run(64, vec![], None);
    hub.run(64, vec![], None);
    let accepted = source.run(128, vec![], None);
    assert_eq!(
        accepted.values.len(),
        3,
        "one ordinary phrase after genuine empty baseline/coverage enrollment"
    );
    assert_eq!(accepted.values.iter().map(|(time, _)| *time).collect::<Vec<_>>(), [0, 12, 32]);
    assert!(
        matches!(accepted.values[1].1, Event::Expression { id: 71, channel: 2, key: 60, value, .. } if value == 0.1234567890123)
    );
    hub.run(128, vec![], None);
    source.run(192, vec![], None);
    hub.run(192, vec![], None);
    source.main();
    hub.main();
}

#[test]
fn repeated_stopped_callbacks_preserve_new_live_input_but_a_real_stop_edge_cancels_older_input() {
    let _scope = crate::test_scope::enter();
    let observation = |playing: bool, time| {
        let Input::Transport(mut value) = transport(time, 120.0) else { unreachable!() };
        if !playing {
            value.flags &= !CLAP_TRANSPORT_IS_PLAYING;
        }
        value
    };
    for real_stop in [false, true] {
        let uuid = SavedUuid::default();
        let mut source = Device::new(true);
        source.configure(uuid, true);
        source.activate();
        source.run_callback(
            0,
            if real_stop {
                vec![note(1, 2, 60, 7, true), note(1, 2, 60, 39, false)]
            } else {
                vec![]
            },
            (None, None),
            (64, false),
            Some(observation(real_stop, 0)),
        );
        let mut events = Vec::new();
        if real_stop {
            events.push(Input::Transport(observation(false, 16)));
        }
        events.extend([
            note(2, 2, 64, 20, true),
            expression(2, 0.34567890123, 32),
            note(2, 2, 64, 52, false),
        ]);
        assert!(source
            .run_callback(64, events, (None, None), (64, false), Some(observation(real_stop, 0)))
            .values
            .is_empty());
        for block in 2..=3 {
            assert!(source
                .run_callback(
                    block * 64,
                    vec![],
                    (None, None),
                    (64, false),
                    Some(observation(false, 0))
                )
                .values
                .is_empty());
        }
        assert_eq!(source.source_snapshot().pending, 3, "new stopped-live input survives repeated stopped observations; only a real earlier Stop edge cancels the old phrase");
        let mut hub = Device::new(false);
        hub.configure(uuid, true);
        hub.activate();
        hub.run(192, vec![], None);
        source.run_callback(256, vec![], (None, None), (64, false), Some(observation(false, 0)));
        hub.run(256, vec![], None);
        let played = source.run_callback(
            320,
            vec![],
            (None, None),
            (64, false),
            Some(observation(false, 0)),
        );
        assert_eq!(played.values.iter().map(|(time, _)| *time).collect::<Vec<_>>(), [0, 12, 32]);
        assert!(matches!(played.values[0].1, Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 2, .. }));
        assert!(
            matches!(played.values[1].1, Event::Expression { id: 2, value, .. } if value == 0.34567890123)
        );
        assert!(matches!(played.values[2].1, Event::Note { kind: CLAP_EVENT_NOTE_OFF, id: 2, .. }));
        hub.run(320, vec![], None);
        source.run_callback(384, vec![], (None, None), (64, false), Some(observation(false, 0)));
        hub.run(384, vec![], None);
        assert_eq!(
            registry::global().lock().unwrap().test_session(uuid).credits.load(Ordering::Acquire),
            0
        );
    }
}

#[test]
fn a_stop_edge_retries_only_old_release_debt_and_preserves_new_stopped_live_notes() {
    let _scope = crate::test_scope::enter();
    let observation = |playing: bool, time| {
        let Input::Transport(mut value) = transport(time, 120.0) else { unreachable!() };
        if !playing {
            value.flags &= !CLAP_TRANSPORT_IS_PLAYING;
        }
        value
    };
    for reject_release in [false, true] {
        let uuid = SavedUuid::default();
        let (mut hub, mut capture) = Device::recorded_hub();
        hub.configure(uuid, true);
        hub.activate();
        let mut source = Device::new(true);
        source.configure(uuid, true);
        source.activate();
        let session = registry::global().lock().unwrap().test_session(uuid);
        source.run_callback(0, vec![], (None, None), (64, false), Some(observation(true, 0)));
        hub.run(0, vec![], None);
        capture.drain_canonical();
        let pedal = Input::Midi(clap_event_midi {
            header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 1),
            port_index: 0,
            data: [0xb0, 64, 127],
        });
        assert_eq!(
            source
                .run_callback(
                    64,
                    vec![pedal, note(1, 0, 60, 5, true)],
                    (None, None),
                    (64, false),
                    Some(observation(true, 0))
                )
                .values
                .len(),
            2
        );
        hub.run(64, vec![], None);
        let events = vec![
            note(3, 0, 62, 4, true),
            expression(1, 0.123456789, 8),
            Input::Midi(clap_event_midi {
                header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 10),
                port_index: 0,
                data: [0xb0, 11, 90],
            }),
            Input::Transport(observation(false, 16)),
            note(2, 0, 64, 20, true),
            expression(2, 0.4567890123, 32),
            note(2, 0, 64, 52, false),
        ];
        let first = source.run_callback(
            128,
            events,
            (reject_release.then_some(CLAP_EVENT_NOTE_CHOKE), None),
            (64, false),
            Some(observation(true, 0)),
        );
        assert!(first.values.iter().any(|(time, event)| {
            *time == 8 && matches!(event, Event::Expression { id: 1, value, .. } if *value == 0.123456789)
        }), "the established expression precedes the actual Stop boundary");
        assert!(first.values.iter().any(|(time, event)| *time == 4
            && matches!(event, Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 3, .. })));
        assert!(first.values.iter().any(|(time, event)| *time == 10
            && matches!(event, Event::Midi { data: [0xb0, 11, 90], .. })));
        let pre_stop_voice: Vec<_> = first
            .values
            .iter()
            .filter(|(_, event)| {
                matches!(event, Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 3, .. })
            })
            .collect();
        assert_eq!(pre_stop_voice.len(), usize::from(!reject_release));
        assert!(pre_stop_voice.iter().all(|(time, _)| *time == 16));
        let neutral: Vec<_> = first
            .values
            .iter()
            .filter_map(|(time, event)| match event {
                Event::Midi { data: [0xb0, controller, 0], .. }
                    if [64, 66, 69].contains(controller) =>
                {
                    Some((*time, *controller))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            neutral,
            [(16, 64), (16, 66), (16, 69)],
            "Stop pedal neutralization is never early"
        );
        assert_eq!(
            first.values.iter().filter(|(_, event)| event.release()).count(),
            if reject_release { 0 } else { 3 },
            "one Stop terminates the old actual voice without waiting for a host Note-Off"
        );
        assert_eq!(
            source.source_snapshot().faults,
            0,
            "a transport Stop is not a permanent output fault"
        );
        assert!(!source.source_snapshot().pedals_held);
        if reject_release {
            assert_eq!(source.source_snapshot().pending, 3);
        }
        hub.run(128, vec![], None); // hub need not have observed the source's Stop yet
        let second = source.run_callback(
            192,
            vec![],
            (None, None),
            (64, false),
            Some(observation(false, 0)),
        );
        let all: Vec<_> = first
            .values
            .into_iter()
            .map(|(time, event)| (128 + i64::from(time), event))
            .chain(second.values.into_iter().map(|(time, event)| (192 + i64::from(time), event)))
            .collect();
        let old: Vec<_> = all
            .iter()
            .filter(|(_, event)| {
                matches!(event, Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 1, .. })
            })
            .collect();
        assert_eq!(old.len(), 1);
        assert_eq!(
            old[0].0,
            if reject_release { 192 } else { 144 },
            "actual Stop or first legal retry sample"
        );
        let new: Vec<_> = all
            .iter()
            .filter(|(_, event)| {
                matches!(event, Event::Note { id: 2, .. } | Event::Expression { id: 2, .. })
            })
            .collect();
        let onset = if reject_release { 192 } else { 148 };
        assert_eq!(
            new.iter().map(|(time, _)| *time).collect::<Vec<_>>(),
            [onset, onset + 12, onset + 32]
        );
        assert!(old[0].0 <= onset);
        hub.run_callback(192, vec![], (None, None), (64, false), Some(observation(false, 0)));
        source.run_callback(256, vec![], (None, None), (64, false), Some(observation(false, 0)));
        hub.run(256, vec![], None);
        assert_eq!(source.source_snapshot().faults, 0);
        assert_eq!(session.credits.load(Ordering::Acquire), 0);
        let records = capture.drain_canonical();
        assert_eq!(records.iter().filter(|record| matches!(record, harmonigraph_take::CanonicalRecord::Delta(delta) if matches!(delta.event.kind, harmonigraph_take::NoteKind::On {..}))).count(),3);
        assert_eq!(records.iter().filter(|record| matches!(record, harmonigraph_take::CanonicalRecord::Delta(delta) if matches!(delta.event.kind, harmonigraph_take::NoteKind::Off))).count(),3);
    }
}

#[test]
fn stop_cut_inhibits_older_controllers_until_all_cancellation_acknowledgements_arrive() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    let observation = |playing: bool, time| {
        let Input::Transport(mut value) = transport(time, 120.0) else { unreachable!() };
        if !playing {
            value.flags &= !CLAP_TRANSPORT_IS_PLAYING;
        }
        value
    };
    for block in 0..2 {
        source.run_callback(
            block * 64,
            vec![],
            (None, None),
            (64, false),
            Some(observation(true, 0)),
        );
        hub.run(block * 64, vec![], None);
    }
    let midi = |data, time| {
        Input::Midi(clap_event_midi {
            header: header::<clap_event_midi>(CLAP_EVENT_MIDI, time),
            port_index: 0,
            data,
        })
    };
    let mut events: Vec<_> = (0..641).map(|_| midi([0xf8, 0, 0], 0)).collect();
    events.extend([
        midi([0xb0, 64, 127], 8),
        Input::Transport(observation(false, 16)),
        midi([0xb0, 64, 100], 20),
    ]);
    let first =
        source.run_callback(128, events, (None, None), (64, false), Some(observation(true, 0)));
    assert_eq!(first.values.len(), 512);
    let mut output = first.values;
    let blocked = source.source_snapshot();
    assert_eq!(blocked.manifest, 64, "the Stop cut reaches the full acknowledgement window");
    assert!(blocked.pending > blocked.manifest, "older work remains outside that window");
    assert_eq!(blocked.faults, 0);
    hub.run(128, vec![], None);
    for block in 3..=12 {
        let next = source.run_callback(
            block * 64,
            vec![],
            (None, None),
            (64, false),
            Some(observation(false, 0)),
        );
        output.extend(next.values);
        hub.run(block * 64, vec![], None);
    }
    assert!(
        !output.iter().any(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 64, 127], .. })),
        "no pre-Stop pedal may escape a partly acknowledged cancellation cut"
    );
    assert_eq!(
        output
            .iter()
            .filter(|(_, event)| matches!(event, Event::Midi { data: [0xf8, 0, 0], .. }))
            .count(),
        512,
        "only the prefix actually accepted before Stop survives"
    );
    assert_eq!(
        output
            .iter()
            .filter(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 64, 100], .. }))
            .count(),
        1,
        "new stopped-live input survives the old cut"
    );
    assert_eq!(source.source_snapshot().pending, 0);
    assert_eq!(source.source_snapshot().faults, 0);
    source.run_callback(
        13 * 64,
        vec![midi([0xb0, 64, 0], 0)],
        (None, None),
        (64, false),
        Some(observation(false, 0)),
    );
    hub.run(13 * 64, vec![], None);
}

#[test]
fn unpaired_reset_settles_the_local_cut_before_recovery_and_preserves_new_input() {
    let _scope = crate::test_scope::enter();
    if std::env::var_os("HARMONIGRAPH_RESET_CUT_CHILD").is_none() {
        // The final unpaired controller has actual history with no receiving
        // lease. Its intentionally retained owner belongs to this process only.
        assert!(std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "performance::tests::unpaired_reset_settles_the_local_cut_before_recovery_and_preserves_new_input", "--nocapture", "--test-threads=1"])
            .env("HARMONIGRAPH_RESET_CUT_CHILD", "1").status().unwrap().success());
        return;
    }
    let uuid = SavedUuid::default();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    let malformed = Input::Midi(clap_event_midi {
        header: clap_event_header {
            size: std::mem::size_of::<clap_event_header>() as u32,
            ..header::<clap_event_midi>(CLAP_EVENT_MIDI, 0)
        },
        port_index: 0,
        data: [0xf8, 0, 0],
    });
    source.run_status(0, vec![malformed], None, None, 64, true);
    let pedal = |value, time| {
        Input::Midi(clap_event_midi {
            header: header::<clap_event_midi>(CLAP_EVENT_MIDI, time),
            port_index: 0,
            data: [0xb0, 64, value],
        })
    };
    assert!(source.run(64, (0..1024).map(|_| pedal(127, 5)).collect(), None).values.is_empty());
    assert_eq!(source.source_snapshot().pending, 1024);
    assert_eq!(source.source_snapshot().input_cut, 1024);
    assert_eq!(source.source_snapshot().faults, source::INPUT_FAULT);
    let shared = source.shared();
    shared.apply(shared.value().routing, true).unwrap();
    let reset = source.run(128, vec![], None);
    assert!(reset.values.is_empty(), "Reset must not forward a pre-Reset buffered pedal");
    assert!(
        source.source_snapshot().pending > 0,
        "the fixture reaches a still-unsettled local cut"
    );
    assert_eq!(source.source_snapshot().input_cut, 1024);
    assert!(shared.applied.load(Ordering::Acquire) < shared.value().generation);
    // The setup hook cuts future input after the observation callback's input
    // batch. This is a genuinely later callback while that cut is still pending.
    assert!(source.run(192, vec![pedal(100, 7)], None).values.is_empty());
    assert!(source.source_snapshot().pending > 1, "new input arrives before the old cut settles");
    assert_eq!(source.source_snapshot().input_cut, 1025);
    assert!(shared.applied.load(Ordering::Acquire) < shared.value().generation);
    let mut accepted = Vec::new();
    for block in 4..=12 {
        let output = source.run(block * 64, vec![], None);
        if shared.applied.load(Ordering::Acquire) < shared.value().generation {
            assert!(output.values.is_empty(), "recovery is inhibited while pre-cut work remains");
            assert!(source.source_snapshot().pending > 0, "the new controller remains owned");
        }
        accepted.extend(output.values);
    }
    assert_eq!(source.source_snapshot().faults, 0);
    assert_eq!(shared.applied.load(Ordering::Acquire), shared.value().generation);
    assert_eq!(accepted, [(0, Event::Midi { port: 0, data: [0xb0, 64, 100], flags: 0 })]);
    assert_eq!(source.source_snapshot().pending, 0);
    assert!(source.run(13 * 64, vec![], None).values.is_empty());
    source.run(14 * 64, vec![pedal(0, 0)], None);
}

#[test]
fn attached_off_source_transfers_more_than_a_journal_of_actual_output() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let mut source = Device::new(true);
    source.configure(uuid, false);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    for block in 1..=2100 {
        let events = vec![note(block, 0, 60, 3, true), note(block, 0, 60, 27, false)];
        let accepted = source.run(i64::from(block) * 64, events, None);
        assert_eq!(
            accepted.values.len(),
            2,
            "Off callback {block} must forward past 4096 cumulative records"
        );
        assert_eq!((accepted.values[0].0, accepted.values[1].0), (3, 27));
        hub.run(i64::from(block) * 64, vec![], None);
    }
    source.run(2101 * 64, vec![], None);
    hub.run(2101 * 64, vec![], None);
}

#[test]
fn off_restore_with_missing_parameter_keeps_actual_get_value_and_saved_value_off() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut source = Device::new(true);
    source.configure(uuid, false);
    source.activate();
    source.run(0, vec![], None);
    let mut state = source.save();
    state.params.remove(setup::PARTICIPATING);
    assert!(source.load(&state));
    assert!(matches!(source.save().params[setup::PARTICIPATING], ParamValue::Bool(false)));
    let mut info = unsafe { std::mem::zeroed() };
    assert!(unsafe { source.params().get_info.unwrap()(source.plugin, 0, &mut info) });
    let mut value = 1.0;
    assert!(unsafe { source.params().get_value.unwrap()(source.plugin, info.id, &mut value) });
    assert_eq!(value, 0.0);
    source.run(64, vec![], None);
    assert!(matches!(source.save().params[setup::PARTICIPATING], ParamValue::Bool(false)));
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    source.run(128, vec![], None);
    hub.run(128, vec![], None);
    assert_eq!(
        source
            .run(192, vec![note(8, 1, 64, 2, true), note(8, 1, 64, 10, false)], None)
            .values
            .len(),
        2
    );
    hub.run(192, vec![], None);
    source.run(256, vec![], None);
    hub.run(256, vec![], None);
    let records = capture.drain_canonical();
    let frames: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            harmonigraph_take::CanonicalRecord::Baseline(frame)
                if frame.baseline().unwrap().source != harmonigraph_core::SourceId::DIRECT =>
            {
                Some(frame.baseline().unwrap())
            }
            _ => None,
        })
        .collect();
    assert!(!frames.is_empty(), "fixture must reach audio-owned remote adoption");
    assert!(
        frames.iter().all(|frame| !frame.participating),
        "missing parameter cannot turn an Off source's actual adopted baseline on"
    );
}

#[test]
fn all_sixteen_tuners_adopt_before_any_registry_ack_and_share_256_real_reservations() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut sources: Vec<_> = (0..16)
        .map(|_| {
            let mut source = Device::new(true);
            source.configure(uuid, true);
            source.activate();
            source
        })
        .collect();
    let mut seventeenth = Device::new(true);
    seventeenth.configure(uuid, true);
    seventeenth.activate();
    assert_eq!(
        seventeenth.shared().source.as_ref().unwrap().status.load(Ordering::Acquire),
        registry::OVERCAPACITY
    );
    for source in &sources {
        source.run(0, vec![], None);
    }
    hub.run(0, vec![], None);
    assert!(seventeenth
        .run(0, vec![note(700, 0, 60, 1, true), note(700, 0, 60, 11, false)], None)
        .values
        .is_empty());
    assert_eq!(seventeenth.source_snapshot().pending, 2);
    // No on_main_thread and therefore no Adopted return-slot drainer.
    for (index, source) in sources.iter().enumerate() {
        let count = if index < 4 { 64 } else { 0 };
        let events = (0..count).map(|key| note(key + 1, 0, key as i16, 0, true)).collect();
        assert_eq!(source.run(64, events, None).values.len(), count as usize);
    }
    assert_eq!(session.credits.load(Ordering::Acquire), 256);
    hub.run(64, vec![], None);
    for (index, source) in sources.iter().enumerate() {
        let count = if index < 4 { 64 } else { 0 };
        assert_eq!(
            source
                .run(
                    128,
                    (0..count).map(|key| note(key + 1, 0, key as i16, 0, false)).collect(),
                    None
                )
                .values
                .len(),
            count as usize
        );
    }
    assert_eq!(
        session.credits.load(Ordering::Acquire),
        256,
        "accepted releases retain credits before the complete output frontier ack"
    );
    hub.run(128, vec![], None);
    for source in &sources {
        source.run(192, vec![], None);
    }
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    hub.run(192, vec![], None);
    // One Adopted return stays queued per instance through all sixteen joins;
    // now service them before exercising ordinary off-thread destruction.
    for source in &sources {
        source.main();
    }
    hub.main();
    sources.clear();
    hub.run(256, vec![], None);
    hub.main();
}

#[test]
fn actual_host_rejection_retains_release_debt_and_never_returns_credit_early() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    assert_eq!(source.run(64, vec![note(90, 4, 60, 8, true)], None).values.len(), 1);
    hub.run(64, vec![], None);
    let rejected = source.run(128, vec![note(90, 4, 60, 7, false)], Some(CLAP_EVENT_NOTE_OFF));
    assert!(
        rejected
            .values
            .iter()
            .any(|(_, e)| matches!(e, Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 90, .. })),
        "dedicated emergency choke settles the rejected ordinary release"
    );
    assert_eq!(session.credits.load(Ordering::Acquire), 1);
    hub.run(128, vec![], None);
    source.run(192, vec![], None);
    hub.run(192, vec![], None);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
}

#[test]
fn duplicate_saved_uuid_before_adoption_keeps_pending_phrase_until_unique_again() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    let duplicate = Device::new(false);
    duplicate.configure(uuid, true);
    assert!(source
        .run(0, vec![note(11, 0, 60, 3, true), note(11, 0, 60, 23, false)], None)
        .values
        .is_empty());
    hub.run(0, vec![], None);
    drop(duplicate);
    source.run(64, vec![], None);
    hub.run(64, vec![], None);
    let accepted = source.run(128, vec![], None);
    assert_eq!(accepted.values.iter().map(|(t, _)| *t).collect::<Vec<_>>(), [0, 20]);
    hub.run(128, vec![], None);
    source.run(192, vec![], None);
    hub.run(192, vec![], None);
}

#[test]
fn duplicate_after_adoption_retains_old_release_then_adopts_new_incarnation() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    assert_eq!(source.run(64, vec![note(21, 1, 64, 5, true)], None).values.len(), 1);
    hub.run(64, vec![], None);
    let duplicate = Device::new(false);
    duplicate.configure(uuid, true);
    let release = source.run(128, vec![note(21, 1, 64, 17, false)], None);
    assert_eq!(release.values.len(), 1, "ambiguity cannot strand an established release");
    hub.run(128, vec![], None);
    // After the original-input cut, a new phrase belongs to the next lease.
    assert!(source
        .run(192, vec![note(22, 1, 64, 3, true), note(22, 1, 64, 23, false)], None)
        .values
        .is_empty());
    hub.run(192, vec![], None);
    drop(duplicate);
    let mut new_phrase = Vec::new();
    for block in 4..=10 {
        source.main();
        hub.main();
        new_phrase.extend(source.run(block * 64, vec![], None).values);
        hub.run(block * 64, vec![], None);
    }
    assert_eq!(new_phrase.len(), 2, "new-side phrase survives two-owner detach and rematch");
    assert_eq!(new_phrase[1].0 - new_phrase[0].0, 20);
    let records = capture.drain_canonical();
    let identities: std::collections::BTreeSet<_> = records
        .iter()
        .filter_map(|record| match record {
            harmonigraph_take::CanonicalRecord::Delta(delta)
                if matches!(delta.event.kind, harmonigraph_take::NoteKind::On { .. }) =>
            {
                Some(delta.event.source)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        identities.len(),
        2,
        "old replies and ring entries cannot cross the new source identity"
    );
}

#[test]
fn pairing_changes_retain_pre_cut_unsounded_ownership_without_an_explicit_reset() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    source.run(64, vec![note(1, 0, 60, 0, true)], None);
    hub.run(64, vec![], None);
    let mut events: Vec<_> = (0..512).map(|_| expression(1, 0.25, 0)).collect();
    events.push(note(2, 0, 62, 63, true));
    let full = source.run(128, events, None);
    assert_eq!(full.attempts, 512);
    assert!(full.values.iter().all(|(_, event)| matches!(event, Event::Expression { .. })));
    let before = source.source_snapshot();
    assert_eq!(before.pending, 1, "the old lease owns an onset awaiting output budget");
    assert_eq!(before.old_obligations, 1);
    hub.run(128, vec![], None);
    let duplicate = Device::new(false);
    duplicate.configure(uuid, true);
    for block in 3..=6 {
        let output = source.run(block * 64, vec![], None);
        assert!(output.values.is_empty());
        hub.run(block * 64, vec![], None);
        let retained = source.source_snapshot();
        assert_eq!(retained.faults, 0);
        assert_eq!(retained.pending, 1, "ambiguity is no cancellation authority");
        assert_eq!(retained.old_obligations, 1);
        assert_eq!(retained.lives, 2);
    }
    // Explicit recovery is a real cancellation boundary and can retire the
    // old lease once its actual held release and disposition are retained.
    let shared = source.shared();
    shared.apply(shared.value().routing, true).unwrap();
    drop(duplicate);
    for block in 7..=18 {
        source.run(block * 64, vec![], None);
        hub.run(block * 64, vec![], None);
        source.main();
        hub.main();
    }
    assert_eq!(source.source_snapshot().pending, 0);
    assert_eq!(source.source_snapshot().held, 0);
}

#[test]
fn ordinary_hub_adoption_preserves_fault_inhibition_until_explicit_settled_reset() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    let malformed = Input::Midi(clap_event_midi {
        header: clap_event_header {
            size: std::mem::size_of::<clap_event_header>() as u32,
            ..header::<clap_event_midi>(CLAP_EVENT_MIDI, 0)
        },
        port_index: 0,
        data: [0xf8, 0, 0],
    });
    source.run_status(0, vec![malformed], None, None, 64, true);
    source.run(64, vec![], None);
    assert_eq!(source.source_snapshot().faults, source::INPUT_FAULT);
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    source.run(128, vec![], None);
    assert_eq!(source.source_snapshot().faults, source::INPUT_FAULT);
    hub.run(128, vec![], None);
    let output = source.run(192, vec![note(1, 0, 60, 2, true)], None);
    assert!(output.values.is_empty(), "joining a hub does not authorize recovery");
    hub.run(192, vec![], None);
    let shared = source.shared();
    assert_eq!(shared.status.load(Ordering::Acquire), source::INPUT_FAULT);
    shared.apply(shared.value().routing, true).unwrap();
    for block in 4..=16 {
        source.run(block * 64, vec![], None);
        hub.run(block * 64, vec![], None);
        source.main();
        hub.main();
    }
    assert_eq!(source.source_snapshot().faults, 0);
    assert_eq!(shared.status.load(Ordering::Acquire), 0);
    let recovered =
        source.run(1088, vec![note(2, 0, 62, 3, true), note(2, 0, 62, 23, false)], None);
    assert_eq!(recovered.values.len(), 2, "explicit settled reset restores forwarding");
    assert_eq!(recovered.values[1].0 - recovered.values[0].0, 20);
}

#[test]
fn held_baseline_precedes_later_release_even_when_hub_drains_after_both_callbacks() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    source.run(64, vec![note(31, 2, 60, 5, true)], None);
    hub.run(64, vec![], None);
    source.run(128, vec![source.participation(false, 0), expression(31, 0.3333333333333, 7)], None);
    source.run(192, vec![note(31, 2, 60, 9, false)], None);
    hub.run(128, vec![], None);
    source.run(256, vec![], None);
    hub.run(192, vec![], None);
    source.run(320, vec![], None);
    hub.run(256, vec![], None);
    let records = capture.drain_canonical();
    let snapshot = records
        .iter()
        .position(|record| matches!(record, CanonicalRecord::Baseline(b) if !b.participating))
        .unwrap();
    let released = records.iter().position(|record| matches!(record, CanonicalRecord::Delta(d) if matches!(d.event.kind, NoteKind::Off))).unwrap();
    assert!(snapshot < released, "the <=C snapshot must not resurrect state after a later release");
    let CanonicalRecord::Baseline(frame) = &records[snapshot] else { unreachable!() };
    let frame = frame.baseline().unwrap();
    assert_eq!(frame.voices().len(), 1);
    assert_eq!(frame.voices()[0].player_tuning, 0.3333333333333);
    assert_eq!(frame.voices()[0].onset.unwrap().sample, 69);
    let mut tracker = harmonigraph_core::NoteTracker::default();
    for record in records {
        record.apply(&mut tracker).unwrap();
    }
    assert_eq!(tracker.held_count(), 0);
}

#[test]
fn full_unpaired_pending_pool_is_retained_and_retired_without_a_peer_wakeup() {
    let _scope = crate::test_scope::enter();
    let mut source = Device::new(true);
    source.configure(SavedUuid::default(), true);
    source.activate();
    for block in 0..4 {
        let events: Vec<_> = (0..1024)
            .flat_map(|id| [note(id, 0, 60, 0, true), note(id, 0, 60, 0, false)])
            .collect();
        assert!(source.run(block * 64, events, None).values.is_empty());
    }
    let state = source.source_snapshot();
    assert_eq!((state.pending, state.lives, state.held, state.faults), (8192, 4096, 0, 0));
    let shared = source.shared();
    let registration = shared.registration().unwrap();
    drop(source);
    assert!(!registry::global().lock().unwrap().test_has_source(registration), "all 8192 never-emitted values can finish their bounded local disposition off audio without a live hub");
}

#[test]
fn repeated_emergency_rejection_keeps_one_exact_release_and_its_credit() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    source.run(64, vec![note(51, 3, 67, 1, true)], None);
    hub.run(64, vec![], None);
    let first = source.run(128, vec![note(51, 3, 67, 5, false)], Some(u16::MAX));
    assert!(first.values.is_empty());
    assert!(first.attempts <= 640);
    hub.run(128, vec![], None);
    let again = source.run(192, vec![], Some(u16::MAX));
    assert!(again.values.is_empty());
    assert!(again.attempts <= 640);
    assert_eq!(session.credits.load(Ordering::Acquire), 1);
    assert_eq!(source.source_snapshot().held, 1);
    hub.run(192, vec![], None);
    let accepted = source.run(256, vec![], None);
    assert_eq!(accepted.values.len(), 4);
    assert!(matches!(
        accepted.values[0].1,
        Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, id: 51, channel: 3, key: 67, .. }
    ));
    assert_eq!(
        accepted.values[1..]
            .iter()
            .map(|(_, event)| match event {
                Event::Midi { data: [0xb3, controller, 0], .. } => *controller,
                _ => panic!("expected accepted channel3 neutralization"),
            })
            .collect::<Vec<_>>(),
        [64, 66, 69]
    );
    assert_eq!(session.credits.load(Ordering::Acquire), 1);
    hub.run(256, vec![], None);
    source.run(320, vec![], None);
    hub.run(320, vec![], None);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    let state = source.source_snapshot();
    assert_eq!((state.held, state.lives, state.pending, state.emergency), (0, 0, 0, 0));
}

#[test]
fn active_restore_cannot_reuse_either_retained_setup_slot_or_mutate_on_refusal() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    source.run(64, vec![note(61, 0, 60, 0, true)], None);
    hub.run(64, vec![], None);
    let shared = source.shared();
    let adopted = shared.adopted().unwrap();
    assert!(adopted.valid);
    assert_eq!(adopted.calibration.offset, 0);
    shared.apply(shared.value().routing, true).unwrap();
    let mut state = source.save();
    state.params.insert(setup::PARTICIPATING.into(), ParamValue::Bool(false));
    assert!(source.load(&state));
    source.run(128, vec![], None); // both slots now remain Reading behind the actual old credit
    assert_eq!(shared.adopted().unwrap(), adopted, "accepted setup cannot replace the displayed audio adoption while old obligations retain it");
    assert!(shared.value().generation > adopted.generation);
    assert_eq!(source.source_snapshot().held, 1);
    state.params.insert(setup::PARTICIPATING.into(), ParamValue::Bool(true));
    assert!(!source.load(&state));
    assert!(matches!(source.save().params[setup::PARTICIPATING], ParamValue::Bool(false)));
    hub.run(128, vec![], None);
    for block in 3..=7 {
        source.run(block * 64, vec![], None);
        hub.run(block * 64, vec![], None);
        source.main();
        hub.main();
    }
    assert!(shared.adopted().unwrap().generation > adopted.generation);
    assert_eq!(shared.adopted().unwrap().generation, shared.applied.load(Ordering::Acquire));
    assert!(source.load(&state), "the same prepared restore can succeed once both old obligations and its retained slot have settled");
    source.run(512, vec![], None);
    hub.run(512, vec![], None);
}

#[test]
fn overlapping_setup_preparation_refuses_the_actual_restore_before_parameter_or_pairing_mutation() {
    let _scope = crate::test_scope::enter();
    use nice_plug::wrapper::clap::setup::Setup;
    let uuid = SavedUuid::default();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    let shared = source.shared();
    let original = source.save();
    let original_generation = shared.value().generation;
    let mut changed = original.clone();
    changed.params.insert(setup::PARTICIPATING.into(), ParamValue::Bool(false));
    changed.fields.insert(
        setup::SOURCE_FIELD.into(),
        serde_json::to_string(&SourceSetup {
            selected: Some(SavedUuid::default()),
            calibration: Calibration {
                offset: 17,
                sample_rate: 48000.0,
                max_frames: 64,
                validated: true,
            },
        })
        .unwrap(),
    );
    let (ready, prepared) = std::sync::mpsc::sync_channel(0);
    let (release, finish) = std::sync::mpsc::sync_channel(0);
    let other = shared.clone();
    let candidate = changed.clone();
    let worker = std::thread::spawn(move || {
        let pending = setup::Adapter(other).prepare(&candidate).unwrap();
        ready.send(()).unwrap();
        finish.recv().unwrap();
        drop(pending); // abandoning the first transaction releases its real slot
    });
    prepared.recv().unwrap();
    assert!(
        !source.load(&changed),
        "the other setup slot is empty, so the in-flight preparer claim must refuse this restore"
    );
    assert!(
        shared.apply(shared.value().routing, false).is_err(),
        "same-thread reentry is refused by the same claim"
    );
    let refused = source.save();
    assert_eq!(refused.fields, original.fields);
    assert!(matches!(refused.params[setup::PARTICIPATING], ParamValue::Bool(true)));
    assert_eq!(shared.value().generation, original_generation);
    release.send(()).unwrap();
    worker.join().unwrap();
    assert!(source.load(&changed));
    assert!(matches!(source.save().params[setup::PARTICIPATING], ParamValue::Bool(false)));
    assert_eq!(shared.value().routing.calibration().offset, 17);
}

#[test]
fn sixty_four_unresolved_retired_sources_refuse_next_registration_without_eviction() {
    let _scope = crate::test_scope::enter();
    const CHILD: &str = "HARMONIGRAPH_RETIRED_CAPACITY_CHILD";
    if std::env::var_os(CHILD).is_none() {
        // This fixture intentionally leaves all counted retired owners pinned.
        // A separate actual test process preserves normal suite independence.
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("performance::tests::sixty_four_unresolved_retired_sources_refuse_next_registration_without_eviction")
            .arg("--exact").arg("--nocapture").env(CHILD, "1").output().unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return;
    }
    let mut hubs = Vec::new();
    let mut sessions = Vec::new();
    for _ in 0..4 {
        let uuid = SavedUuid::default();
        let mut hub = Device::new(false);
        hub.configure(uuid, true);
        hub.activate();
        let session = registry::global().lock().unwrap().test_session(uuid);
        let mut sources = Vec::new();
        for _ in 0..16 {
            let mut source = Device::new(true);
            source.configure(uuid, true);
            source.activate();
            source.run(0, vec![], None);
            sources.push(source);
        }
        hub.run(0, vec![], None);
        for source in &sources {
            assert_eq!(source.run(64, vec![note(1, 0, 60, 0, true)], None).values.len(), 1);
        }
        hub.run(64, vec![], None);
        for source in &sources {
            source.main();
        }
        drop(sources); // callback join does not prove these 16 downstream notes terminated
        hub.run(128, vec![], None);
        hub.main();
        assert_eq!(session.credits.load(Ordering::Acquire), 16);
        sessions.push(session);
        hubs.push(hub);
    }
    assert_eq!(registry::global().lock().unwrap().test_counts(), (4, 64, 64));
    let refused = Device::new(true);
    assert!(refused.shared().registration().is_none());
    assert_eq!(
        refused.shared().source.as_ref().unwrap().status.load(Ordering::Acquire),
        registry::OVERCAPACITY
    );
    assert_eq!(registry::global().lock().unwrap().test_counts(), (4, 64, 64));
    assert!(sessions.iter().all(|session| session.credits.load(Ordering::Acquire) == 16));
}

#[test]
fn hub_reinitialize_waits_for_differently_timed_source_seals_and_preserves_the_take() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut a = Device::new(true);
    a.configure(uuid, true);
    a.activate();
    let mut b = Device::new(true);
    b.configure(uuid, false);
    b.activate();
    capture.arm();
    a.run(0, vec![], None);
    b.run(0, vec![], None);
    hub.run(0, vec![], None);
    let dir = std::env::temp_dir()
        .join(format!("harmonigraph-session-transition-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("session.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    writer.drain(&mut capture);
    a.run(64, vec![note(80, 0, 60, 3, true)], None);
    b.run(64, vec![note(80, 0, 60, 7, true)], None);
    hub.run(64, vec![], None);
    writer.drain(&mut capture);
    let epoch = session.epoch.load(Ordering::Acquire);
    let shared = hub.shared();
    shared.apply(shared.value().routing, true).unwrap();
    hub.run(128, vec![], None);
    assert_ne!(session.closing.load(Ordering::Acquire), 0);
    assert_eq!(a.run(128, vec![], None).values.len(), 4);
    b.run(128, vec![], Some(u16::MAX));
    hub.run(192, vec![], None);
    writer.drain(&mut capture);
    assert_eq!(session.epoch.load(Ordering::Acquire), epoch);
    a.run(192, vec![], None);
    b.run(192, vec![], Some(u16::MAX));
    hub.run(256, vec![], None);
    writer.drain(&mut capture);
    assert_eq!(
        session.epoch.load(Ordering::Acquire),
        epoch,
        "unaccepted late release forbids clock commit"
    );
    a.run(256, vec![], None);
    assert_eq!(b.run(256, vec![], None).values.len(), 4);
    for block in 5..=14 {
        hub.run(block * 64, vec![], None);
        writer.drain(&mut capture);
        a.run(block * 64, vec![], None);
        b.run(block * 64, vec![], None);
        a.main();
        b.main();
        hub.main();
    }
    assert_eq!(session.epoch.load(Ordering::Acquire), epoch + 1);
    assert_eq!(session.closing.load(Ordering::Acquire), 0);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    capture.stop();
    writer.stop();
    a.run(15 * 64, vec![], None);
    b.run(15 * 64, vec![], None);
    hub.run(15 * 64, vec![], None);
    writer.drain(&mut capture);
    assert!(
        writer.finished.is_some(),
        "a clock transition must preserve the actual open take/pass closure"
    );
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(take.incomplete.is_none());
    let notes: Vec<_> = take
        .events
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta) => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(notes.iter().filter(|d| matches!(d.event.kind, NoteKind::On { .. })).count(), 2);
    assert_eq!(notes.iter().filter(|d| matches!(d.event.kind, NoteKind::Off)).count(), 2);
    let mut tracker = harmonigraph_core::NoteTracker::default();
    for record in &take.events {
        record.apply(&mut tracker).unwrap();
    }
    assert_eq!(tracker.held_count(), 0);
    drop(writer);
    drop(hub);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn attached_reset_disposes_more_than_one_manifest_window_without_baseline_substitution() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    let mut accepted = 0;
    for block in 1..=4 {
        let events = (0..1024)
            .flat_map(|id| [note(id, 0, 60, 0, true), note(id, 0, 60, 0, false)])
            .collect();
        accepted += source.run(block * 64, events, None).values.len();
    }
    assert_eq!(
        accepted, 128,
        "the source's 64 terminal-but-unacknowledged reservations remain charged"
    );
    assert_eq!(source.source_snapshot().pending, 8064);
    let shared = source.shared();
    shared.apply(shared.value().routing, true).unwrap();
    for block in 1..=4 {
        hub.run(block * 64, vec![], None);
    }
    let mut largest_manifest = 0;
    for block in 5..=142 {
        assert!(
            source.run(block * 64, vec![], None).values.is_empty(),
            "explicit Reset must not emit a retained unsounded onset"
        );
        largest_manifest = largest_manifest.max(source.source_snapshot().manifest);
        hub.run(block * 64, vec![], None);
        source.main();
        hub.main();
    }
    let state = source.source_snapshot();
    assert_eq!(largest_manifest, 64, "fixture reaches the actual separate disposition window");
    assert_eq!(
        (state.pending, state.lives, state.held, state.journal, state.manifest),
        (0, 0, 0, 0, 0)
    );
}

fn wait_until(mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(ready(), "actual factory/worker did not reach its bounded fixture boundary");
}

#[test]
fn retired_hub_keeps_original_routes_for_actual_output_paused_before_transfer() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    if std::env::var_os("HARMONIGRAPH_JOINED_RECORDING_CHILD").is_none() {
        for mode in 0..5 {
            assert!(std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "performance::tests::retired_hub_keeps_original_routes_for_actual_output_paused_before_transfer", "--nocapture", "--test-threads=1"])
                .env("HARMONIGRAPH_JOINED_RECORDING_CHILD", mode.to_string()).status().unwrap().success());
        }
        return;
    }
    // Unknown wire state intentionally pins its musical owner after the actual
    // recording stream closes. Each case owns a separate process.
    let mode: usize =
        std::env::var("HARMONIGRAPH_JOINED_RECORDING_CHILD").unwrap().parse().unwrap();
    let blocked_configuration = matches!(mode, 1 | 2);
    let unknown_held = mode == 2;
    let owed_off = mode == 3;
    let pedal_only = mode == 4;
    let unknown_wire = unknown_held || owed_off || pedal_only;
    let uuid = SavedUuid::default();
    let directory = std::env::temp_dir()
        .join(format!("harmonigraph-paused-retirement-{}-{mode}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let (recorder, control) = harmonigraph_record::channel();
    let writer = harmonigraph_record::testing::worker_probe(&control, directory.clone());
    struct ResumeWriter<'a>(&'a harmonigraph_record::testing::WorkerProbe);
    impl Drop for ResumeWriter<'_> {
        fn drop(&mut self) {
            self.0.resume_retirement_check();
        }
    }
    let _resume_writer = ResumeWriter(&writer);
    crate::configuration::inject_recorder(recorder);
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    source.main();
    hub.main();
    control.start(48000.0, String::new(), false);
    let shared = source.shared();
    shared.before_transfer.enabled.store(true, Ordering::Release);
    std::thread::scope(|scope| {
        struct Resume<'a>(&'a setup::TestPause);
        impl Drop for Resume<'_> {
            fn drop(&mut self) {
                self.0.enabled.store(false, Ordering::Release);
            }
        }
        let _resume = Resume(&shared.before_transfer);
        let address = (&source as *const Device) as usize;
        let callback = scope.spawn(move || {
            // The original factory Device and host outlive this joined callback.
            // Main operates only the independent hub while the source is paused.
            let source = unsafe { &*(address as *const Device) };
            let cc = |controller, value, time| {
                Input::Midi(clap_event_midi {
                    header: header::<clap_event_midi>(CLAP_EVENT_MIDI, time),
                    port_index: 0,
                    data: [0xb5, controller, value],
                })
            };
            let notes = if pedal_only {
                vec![cc(64, 127, 3)]
            } else if owed_off {
                vec![note(91, 5, 60, 3, true), cc(120, 0, 23)]
            } else if unknown_held {
                vec![note(91, 5, 60, 3, true)]
            } else {
                vec![note(91, 5, 60, 3, true), note(91, 5, 60, 23, false)]
            };
            source.run(64, notes, None)
        });
        wait_until(|| shared.before_transfer.entered.load(Ordering::Acquire));
        assert_eq!(session.credits.load(Ordering::Acquire), usize::from(!pedal_only));
        hub.run(64, vec![], None); // owns the original recorded span of those accepted events
        let mailbox = if blocked_configuration {
            let wrapper = unsafe {
                &*((*hub.plugin)
                    .plugin_data
                    .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
            };
            let mailbox = wrapper.configuration_handle().unwrap();
            for value in 690..707 {
                mailbox
                    .submit(crate::configuration::packet(
                        harmonigraph_core::configuration::ConfigEdit::axis(1, value * 1_000_000),
                    ))
                    .unwrap();
            }
            hub.run(128, vec![], None);
            Some(mailbox)
        } else {
            None
        };
        control.stop(None);
        hub.run(if blocked_configuration { 192 } else { 128 }, vec![], None); // producer/config closure; actual source history is still paused
        if let Some(mailbox) = &mailbox {
            assert!(mailbox.visible().1);
            assert!(
                mailbox.visible().0.applied_id < mailbox.accepted_command.load(Ordering::Acquire),
                "one real command remains after two callback budgets"
            );
            assert_eq!(mailbox.visible().0.raw[1], 705.0);
        }
        drop(mailbox);
        drop(control);
        drop(hub);
        assert!(registry::global().lock().unwrap().test_retained_hub(session.runtime));
        let visits = writer.empty_visits();
        wait_until(|| writer.empty_visits() > visits + 2);
        assert!(!writer.finished(), "empty queues cannot destroy the writer while the actual source callback owns untransferred history");
        if blocked_configuration {
            // Freeze the actual worker after its drain, just before it reads
            // the retirement hold. Publish the final notes and release the
            // hold while frozen; it must recheck the lanes after Acquire.
            writer.pause_retirement_check();
            wait_until(|| writer.retirement_check_paused());
        }
        let wakes = source._stats.callbacks.load(Ordering::Acquire);
        shared.before_transfer.enabled.store(false, Ordering::Release);
        assert_eq!(
            callback.join().unwrap().values.len(),
            if unknown_held || pedal_only { 1 } else { 2 }
        );
        assert!(
            source._stats.callbacks.load(Ordering::Acquire) > wakes,
            "surviving source requests its own valid main service"
        );
    });
    assert_eq!(source.source_snapshot().note_off_owed, usize::from(owed_off));
    assert_eq!(source.source_snapshot().pedals_held, pedal_only);
    let source = if blocked_configuration || unknown_wire {
        // Callback join, then actual destruction: no rescue audio callback.
        drop(source);
        None
    } else {
        source.main();
        assert!(
            registry::global().lock().unwrap().test_retained_hub(session.runtime),
            "history publication alone does not settle the other endpoint user"
        );
        source.run(128, vec![], None);
        source.main();
        Some(source)
    };
    writer.resume_retirement_check();
    wait_until(|| writer.finished());
    assert_eq!(writer.failed(), blocked_configuration || unknown_wire);
    assert_eq!(session.credits.load(Ordering::Acquire), usize::from(unknown_held));
    let file = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension().is_some_and(|extension| extension == harmonigraph_take::EXTENSION)
        })
        .unwrap();
    let take = harmonigraph_take::Take::read(&file).unwrap();
    assert_eq!(take.incomplete.is_some(), blocked_configuration || unknown_wire);
    let notes: Vec<_> = take
        .events
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta) => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(
        notes.len(),
        if pedal_only {
            0
        } else if unknown_held {
            1
        } else {
            2
        }
    );
    if !pedal_only {
        assert!(matches!(notes[0].event.kind, NoteKind::On { .. }));
        assert_eq!(notes[0].timing.unwrap().sample, 67);
        assert!((notes[0].event.t - 67.0 / 48000.0).abs() < 1e-12);
        if !unknown_held {
            assert!(matches!(notes[1].event.kind, NoteKind::Off));
            assert_eq!(notes[1].timing.unwrap().sample, 87);
            assert!((notes[1].event.t - 87.0 / 48000.0).abs() < 1e-12);
        }
    }
    if blocked_configuration {
        let changes: Vec<_> = take
            .configurations
            .iter()
            .filter(|config| config.t == 128.0 / 48000.0)
            .map(|config| config.axes[1])
            .collect();
        assert_eq!(
            changes,
            (690..706).map(|value| value * 1_000_000).collect::<Vec<_>>(),
            "all16 actually-applied commands retain their original route and time"
        );
        assert!(
            !take.configurations.iter().any(|config| config.axes[1] == 706_000_000),
            "the discarded last command is not fabricated"
        );
    }
    drop(source);
    assert_eq!(
        registry::global().lock().unwrap().test_counts(),
        if unknown_wire { (1, 1, 1) } else { (0, 0, 0) }
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn refused_hub_destruction_releases_its_recording_hold_without_a_registry_owner() {
    let _scope = crate::test_scope::enter();
    let registered: Vec<_> = (0..4).map(|_| Device::new(false)).collect();
    let directory = std::env::temp_dir()
        .join(format!("harmonigraph-refused-retirement-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let (recorder, control) = harmonigraph_record::channel();
    let writer = harmonigraph_record::testing::worker_probe(&control, directory.clone());
    crate::configuration::inject_recorder(recorder);
    let mut refused = Device::new(false);
    refused.activate();
    assert!(refused.shared().registration().is_none());
    control.start(48000.0, String::new(), false);
    let wrapper = unsafe {
        &*((*refused.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    let mailbox = wrapper.configuration_handle().unwrap();
    mailbox
        .submit(crate::configuration::packet(harmonigraph_core::configuration::ConfigEdit {
            learning: Some(true),
            ..Default::default()
        }))
        .unwrap();
    refused.run(0, vec![], None);
    for value in 690..698 {
        mailbox
            .submit(crate::configuration::packet(
                harmonigraph_core::configuration::ConfigEdit::axis(1, value * 1_000_000),
            ))
            .unwrap();
    }
    refused.run(
        64,
        [60, 64, 67].into_iter().map(|key| note(i32::from(key), 0, key, 3, true)).collect(),
        None,
    );
    control.stop(None);
    let pending = wrapper.test_inspect_plugin(|plugin| {
        let owner = plugin.configuration.as_ref().unwrap();
        (
            owner.direct.sequence,
            owner.direct.pending().map(|d| d.timing.unwrap().sample),
            owner.recording.prefix,
        )
    });
    assert_eq!(pending, (3,Some(67),67), "the actual triad is observed, but its required learning commit cannot fit the exhausted callback budget");
    drop(mailbox);
    drop(control);
    drop(refused);
    wait_until(|| writer.finished());
    assert!(writer.failed());
    assert_eq!(registry::global().lock().unwrap().test_counts(), (4, 0, 0));
    let file = std::fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == harmonigraph_take::EXTENSION))
        .unwrap();
    let take = harmonigraph_take::Take::read(&file).unwrap();
    assert!(take.incomplete.is_some());
    let samples: Vec<_> = take
        .events
        .iter()
        .filter_map(|record| match record {
            harmonigraph_take::CanonicalRecord::Delta(delta) => Some(delta.timing.unwrap().sample),
            _ => None,
        })
        .collect();
    assert_eq!(samples, [67, 67, 67]);
    drop(registered);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn joined_producer_fact_is_cleared_when_the_actual_source_row_is_reused() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let rows = || {
        let wrapper = unsafe {
            &*((*hub.plugin)
                .plugin_data
                .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
        };
        wrapper
            .test_inspect_plugin(|plugin| plugin.aggregation.as_ref().unwrap().test_joined_rows())
    };
    let mut old = Device::new(true);
    old.configure(uuid, true);
    old.activate();
    old.run(0, vec![], None);
    hub.run(0, vec![], None);
    assert_eq!(
        old.run(64, vec![note(1, 0, 60, 3, true), note(1, 0, 60, 23, false)], None).values.len(),
        2
    );
    hub.run(64, vec![], None);
    let old_lease = rows().iter().find_map(|row| row.0).unwrap();
    drop(old);
    for block in 2..8 {
        hub.run(block * 64, vec![], None);
        hub.main();
    }
    assert_eq!(registry::global().lock().unwrap().test_counts(), (1, 0, 0));
    let old_row = rows()[usize::from(old_lease.slot - 1)];
    assert_eq!(
        (old_row.1, old_row.3),
        (Some(2), 2),
        "the fixture must store a real joined cut before actual slot reuse"
    );
    let mut new = Device::new(true);
    new.configure(uuid, true);
    new.activate();
    new.run(512, vec![], None);
    hub.run(512, vec![], None);
    assert_eq!(
        new.run(576, vec![note(2, 0, 62, 3, true), note(2, 0, 62, 23, false)], None).values.len(),
        2
    );
    hub.run(576, vec![], None);
    let reused = rows()[usize::from(old_lease.slot - 1)];
    let lease = reused.0.unwrap();
    assert_eq!(lease.slot, old_lease.slot);
    assert_ne!(lease.incarnation, old_lease.incarnation);
    assert_eq!((reused.1, reused.2, reused.3), (None, false, 2), "equal sequence cuts in a new incarnation must not inherit the old producer's terminal proof");
    drop(new);
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn host_rewind_keeps_old_routes_until_sealed_unmapped_termination_and_rejects_old_ack() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    capture.arm();
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-forced-clock-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("rewind.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    let shared = source.shared();
    shared.before_transfer.enabled.store(true, Ordering::Release);
    std::thread::scope(|scope| {
        struct Resume<'a>(&'a setup::TestPause);
        impl Drop for Resume<'_> {
            fn drop(&mut self) {
                self.0.enabled.store(false, Ordering::Release);
            }
        }
        let _resume = Resume(&shared.before_transfer);
        let address = (&source as *const Device) as usize;
        let callback = scope.spawn(move || {
            unsafe { &*(address as *const Device) }.run(64, vec![note(95, 3, 61, 7, true)], None)
        });
        wait_until(|| shared.before_transfer.entered.load(Ordering::Acquire));
        hub.run(64, vec![], None);
        writer.drain(&mut capture);
        unsafe {
            (*hub.plugin).reset.unwrap()(hub.plugin);
        }
        shared.before_transfer.enabled.store(false, Ordering::Release);
        assert_eq!(callback.join().unwrap().values.len(), 1);
    });
    let old_epoch = session.epoch.load(Ordering::Acquire);
    let wrapper = unsafe {
        &*((*source.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
    };
    let old_lease = wrapper.test_inspect_plugin(|plugin| {
        plugin.source.as_ref().unwrap().offer.as_ref().unwrap().lease
    });
    hub.run(0, vec![], None);
    writer.drain(&mut capture);
    let release = source.run(0, vec![], None);
    assert_eq!(release.values.len(), 4);
    assert!(matches!(release.values[0].1, Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, .. }));
    assert_eq!(
        session.credits.load(Ordering::Acquire),
        1,
        "actual unplaceable release still requires a complete sealed-stream acknowledgement"
    );
    let sealed = source.source_snapshot();
    let old_ack = protocol::Reply::SealedStreamRetained {
        incarnation: old_lease.incarnation,
        epoch: sealed.epoch,
        generation: sealed.seal.expect("actual final source seal"),
        cut: sealed.sequence,
    };
    hub.run(64, vec![], None);
    writer.drain(&mut capture);
    let controls = &session.rows[usize::from(old_lease.slot - 1)].to_source;
    controls.publish(old_ack).unwrap();
    controls.publish(old_ack).unwrap();
    source.run(64, vec![], None);
    assert_eq!(
        source.source_snapshot().complete_through,
        sealed.complete_through,
        "sealed acknowledgement has no sample frontier"
    );
    assert_eq!(
        session.credits.load(Ordering::Acquire),
        0,
        "duplicate final acknowledgements settle once"
    );
    source.main();
    hub.main();
    for block in 2..=8 {
        hub.run(block * 64, vec![], None);
        writer.drain(&mut capture);
        source.run(block * 64, vec![], None);
        source.main();
        hub.main();
    }
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert_eq!(
        session.epoch.load(Ordering::Acquire),
        old_epoch,
        "host reset is not validated setup"
    );
    let hub_setup = hub.shared();
    hub_setup.apply(hub_setup.value().routing, true).unwrap();
    shared.apply(shared.value().routing, true).unwrap();
    for block in 9..=18 {
        source.run(block * 64, vec![], None);
        hub.run(block * 64, vec![], None);
        writer.drain(&mut capture);
        source.main();
        hub.main();
    }
    assert_eq!(session.epoch.load(Ordering::Acquire), old_epoch + 1);
    assert_eq!(session.closing.load(Ordering::Acquire), 0);
    assert_eq!(source.run(19 * 64, vec![note(96, 3, 61, 5, true)], None).values.len(), 1);
    session.rows[usize::from(old_lease.slot - 1)].to_source.publish(old_ack).unwrap();
    source.run(20 * 64, vec![], None);
    assert_eq!(
        session.credits.load(Ordering::Acquire),
        1,
        "delayed old acknowledgement cannot retire the new lease"
    );
    capture.stop();
    writer.stop();
    source.run(21 * 64, vec![note(96, 3, 61, 2, false)], None);
    for block in 19..=23 {
        hub.run(block * 64, vec![], None);
        writer.drain(&mut capture);
    }
    source.run(22 * 64, vec![], None);
    hub.run(24 * 64, vec![], None);
    writer.drain(&mut capture);
    // Forced continuity loss must be audible in the actual take; pre-reset
    // history nevertheless retains its original epoch/sample and recording map.
    assert!(
        writer.failed(),
        "the actual failure consumer closes an incomplete file separately from finished"
    );
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(take.incomplete.is_some());
    let old: Vec<_> = take
        .events
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta)
                if delta.timing.is_some_and(|t| t.clock_epoch == old_epoch) =>
            {
                Some(delta)
            }
            _ => None,
        })
        .collect();
    assert_eq!(old.len(), 1);
    assert!(matches!(old[0].event.kind, NoteKind::On { .. }));
    assert_eq!(old[0].timing.unwrap().sample, 71);
    assert!((old[0].event.t - 71.0 / 48000.0).abs() < 1e-12);
    drop(writer);
    drop(source);
    drop(hub);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn display_resync_arriving_during_hub_publication_repairs_every_source_without_poisoning_take() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let mut a = Device::new(true);
    a.configure(uuid, true);
    a.activate();
    let mut b = Device::new(true);
    b.configure(uuid, false);
    b.activate();
    a.run(0, vec![], None);
    b.run(0, vec![], None);
    hub.run(0, vec![], None);
    capture.arm();
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-session-resync-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("display.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    a.run(64, vec![note(10, 0, 60, 0, true)], None);
    b.run(64, vec![note(20, 0, 60, 0, true)], None);
    hub.run(64, vec![note(30, 0, 60, 0, true)], None);
    writer.drain(&mut capture);
    for block in 2..=67 {
        a.run(
            block * 64,
            (0..64).map(|time| expression(10, 0.1234567890123, time)).collect(),
            None,
        );
        b.run(block * 64, vec![], None);
        hub.run(block * 64, vec![], None);
        writer.drain(&mut capture);
    }
    let mut tracker = harmonigraph_core::NoteTracker::default();
    for record in writer.display_events() {
        record.apply(&mut tracker).unwrap();
    }
    assert!(!tracker.publication_gaps().is_empty());
    assert!(!writer.failed());
    a.run(68 * 64, vec![], None);
    b.run(68 * 64, vec![], None);
    let shared = hub.shared();
    shared.before_direct_repair.enabled.store(true, Ordering::Release);
    std::thread::scope(|scope| {
        struct Resume<'a>(&'a setup::TestPause);
        impl Drop for Resume<'_> {
            fn drop(&mut self) {
                self.0.enabled.store(false, Ordering::Release);
            }
        }
        let _resume = Resume(&shared.before_direct_repair);
        let address = (&hub as *const Device) as usize;
        let callback =
            scope.spawn(move || unsafe { &*(address as *const Device) }.run(68 * 64, vec![], None));
        wait_until(|| shared.before_direct_repair.entered.load(Ordering::Acquire));
        writer.drain(&mut capture); // actual fanout requests all uncertain sources now that display has capacity
        shared.before_direct_repair.enabled.store(false, Ordering::Release);
        callback.join().unwrap();
    });
    writer.drain(&mut capture);
    a.run(69 * 64, vec![], None);
    b.run(69 * 64, vec![], None);
    hub.run(69 * 64, vec![], None);
    writer.drain(&mut capture);
    let repaired = writer.display_events();
    let frames: Vec<_> = repaired
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Baseline(frame) => Some(frame.baseline().unwrap()),
            _ => None,
        })
        .collect();
    assert_eq!(
        frames.len(),
        3,
        "a hint arriving after Hub dispatch must remain pending for all three sources"
    );
    assert!(frames.iter().all(|frame| frame.voices().len() == 1));
    assert_eq!(frames.iter().filter(|frame| frame.participating).count(), 2);
    assert_eq!(
        frames
            .iter()
            .flat_map(|frame| frame.voices().iter().map(|voice| voice.host_note_id))
            .collect::<std::collections::BTreeSet<_>>(),
        [10, 20, 30].into_iter().collect()
    );
    assert!(!repaired.iter().any(|record| matches!(record, CanonicalRecord::Delta(delta) if matches!(delta.event.kind, NoteKind::On { .. }))));
    for record in repaired {
        record.apply(&mut tracker).unwrap();
    }
    assert_eq!(
        tracker.held_count(),
        2,
        "Off retains its truthful baseline but is excluded from participating context"
    );
    assert!(!writer.failed());
    capture.stop();
    writer.stop();
    a.run(70 * 64, vec![note(10, 0, 60, 0, false)], None);
    b.run(70 * 64, vec![note(20, 0, 60, 0, false)], None);
    hub.run(70 * 64, vec![note(30, 0, 60, 0, false)], None);
    writer.drain(&mut capture);
    assert!(writer.finished.is_some());
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(take.incomplete.is_none());
    assert_eq!(take.events.iter().filter(|record| matches!(record, CanonicalRecord::Delta(delta) if matches!(delta.event.kind, NoteKind::On { .. }))).count(), 3);
    a.run(71 * 64, vec![], None);
    b.run(71 * 64, vec![], None);
    hub.run(71 * 64, vec![], None);
    drop(writer);
    drop(a);
    drop(b);
    drop(hub);
    std::fs::remove_dir_all(directory).unwrap();
}

#[cfg(debug_assertions)]
#[test]
fn measured_ordinary_storage_and_actual_factory_allocation_increments() {
    let _scope = crate::test_scope::enter();
    use nice_plug::wrapper::allocation_probe::measure_allocations;
    if std::env::var_os("HARMONIGRAPH_MEMORY_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "performance::tests::measured_ordinary_storage_and_actual_factory_allocation_increments", "--nocapture", "--test-threads=1"])
            .env("HARMONIGRAPH_MEMORY_CHILD", "1").status().unwrap();
        assert!(status.success());
        return;
    }
    factory(); // one-time logger/factory setup is not per-instance storage
    let (tune_owner, tune_storage) = measure_allocations(tune::HarmonigraphTune::default);
    let (hub_owner, hub_storage) = measure_allocations(hub::Hub::new);
    let mut registry = registry::Registry::default();
    let bridge = std::sync::Arc::new(registry::HubBridge::default());
    let (_, bank_storage) =
        measure_allocations(|| registry.register_hub(SavedUuid::default(), bridge).unwrap());
    tune_owner.source.as_ref().unwrap().print_test_memory_layout();
    hub_owner.print_test_memory_layout();
    use super::{protocol as wire, slots::Slots};
    use std::mem::size_of;
    println!("LEDGER protocol [intent,reply,output,control,baseline,source_control,session_control,hub_bank] {:?}",
        [size_of::<wire::Intent>(), size_of::<wire::Reply>(), size_of::<wire::OutputDelta>(),
         size_of::<wire::Control>(), size_of::<wire::Baseline>(), size_of::<wire::SourceControl>(),
         size_of::<wire::SessionControl>(), size_of::<wire::HubBank>()]);
    println!("LEDGER setup [shared,update,update_slots,source_bridge,hub_bridge,registry,global_once_mutex] {:?}",
        [size_of::<setup::Shared>(), size_of::<setup::Update>(), size_of::<Slots<setup::Update>>(),
         size_of::<registry::SourceBridge>(), size_of::<registry::HubBridge>(), size_of::<registry::Registry>(),
         size_of::<std::sync::OnceLock<std::sync::Mutex<registry::Registry>>>()]);
    let params = crate::HarmonigraphParams::default();
    let (configuration_owner, configuration_storage) =
        measure_allocations(|| Box::new(crate::configuration::Owner::new(&params)));
    configuration_owner.print_test_memory_layout();
    println!("LEDGER isolated boxed configuration Owner allocation {:?}", configuration_storage);
    let (publication_channels, publication_storage) = measure_allocations(|| {
        [harmonigraph_record::publication::channel(), harmonigraph_record::publication::channel()]
    });
    harmonigraph_record::publication::print_test_memory_layout();
    println!("LEDGER actual two publication channels allocation {:?}", publication_storage);
    nice_plug::wrapper::clap::configuration::print_test_memory_layout();
    let (mailbox, mailbox_storage) =
        measure_allocations(nice_plug::wrapper::clap::configuration::test_memory_mailbox);
    println!("LEDGER isolated configuration mailbox allocation {:?}", mailbox_storage);
    println!(
        "LEDGER wrapper [input_option,group_option,input_count,output_count] {:?}",
        [
            size_of::<Option<nice_plug::wrapper::clap::configuration::OwnedInput>>(),
            size_of::<Option<nice_plug::wrapper::clap::performance::Group>>(),
            2048,
            640
        ]
    );
    println!(
        "LEDGER full HG wrapper and existing take Entry bytes {:?}",
        [
            size_of::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>(),
            size_of::<harmonigraph_record::Entry>()
        ]
    );
    drop(mailbox);
    drop(publication_channels);
    drop(configuration_owner);
    let ordinary = 16 * tune_storage.retained + hub_storage.retained + bank_storage.retained;
    println!(
        "MEASURE source cells [pending,life,delta,manifest,owner] {:?}",
        source::Source::test_cell_sizes()
    );
    println!("MEASURE Tune owner {:?}; Hub including full DIRECT owner {:?}; actual16 ring triples+controls {:?}", tune_storage, hub_storage, bank_storage);
    println!("MEASURE current ordinary session bytes {ordinary}; four sessions {}", 4 * ordinary);
    assert!(ordinary > 0 && ordinary < 144 * 1024 * 1024);
    drop(tune_owner);
    drop(hub_owner);
    drop(registry);
    let (mut hub, hub_factory) = measure_allocations(|| Device::new(false));
    let (_, hub_activation) = measure_allocations(|| hub.activate());
    let mut tuners = Vec::with_capacity(16);
    let (_, tune_factory) = measure_allocations(|| {
        for _ in 0..16 {
            let mut tuner = Device::new(true);
            tuner.activate();
            tuners.push(tuner);
        }
    });
    println!(
        "MEASURE actual full-HG factory main-thread increment {:?}; activation {:?}",
        hub_factory, hub_activation
    );
    println!("MEASURE actual16 Tune factories+activation main-thread increment {:?}", tune_factory);
    println!("MEASURE fixture host overhead per instance {} bytes; background-thread requests and allocator-private overhead excluded", std::mem::size_of::<clap_host>() + std::mem::size_of::<Host>());
    assert!(tune_factory.retained > 16 * tune_storage.retained);
    // A refused instance still has its own wrapper/owner today: report that
    // measured increment explicitly instead of calling registration the heap cap.
    for _ in 16..64 {
        tuners.push(Device::new(true));
    }
    let (refused, refused_storage) = measure_allocations(|| Device::new(true));
    assert_eq!(
        refused.shared().source.as_ref().unwrap().status.load(Ordering::Acquire),
        registry::OVERCAPACITY
    );
    println!("MEASURE refused65th Tune factory increment {:?}", refused_storage);
    drop(refused);
    drop(tuners);
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
    let mut hubs = Vec::with_capacity(4);
    let mut tuners = Vec::with_capacity(64);
    let (_, four_sessions) = measure_allocations(|| {
        for _ in 0..4 {
            let uuid = SavedUuid::default();
            let mut hub = Device::new(false);
            hub.configure(uuid, true);
            hub.activate();
            for _ in 0..16 {
                let mut tuner = Device::new(true);
                tuner.configure(uuid, true);
                tuner.activate();
                tuner.run(0, vec![], None);
                tuners.push(tuner);
            }
            hub.run(0, vec![], None);
            hubs.push(hub);
        }
    });
    assert_eq!(registry::global().lock().unwrap().test_counts(), (4, 64, 0));
    println!(
        "MEASURE actually constructed/activated/enrolled4HG+64Tune main-thread increment {:?}",
        four_sessions
    );
    let (_, retirement) = measure_allocations(|| {
        drop(tuners);
        drop(hubs);
    });
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
    println!("MEASURE actual4-session quiesced retirement main-thread changes {:?}", retirement);
}

#[test]
fn sealed_source_does_not_rearm_neutral_pedals_when_retirement_adds_a_stronger_fault() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    let pedal = Input::Midi(clap_event_midi {
        header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
        port_index: 0,
        data: [0xb0, 64, 127],
    });
    assert_eq!(source.run(64, vec![pedal, note(57, 0, 60, 1, true)], None).values.len(), 2);
    hub.run(64, vec![], None);
    let shared = source.shared();
    let setup::Routing::Source(mut next) = shared.value().routing else { unreachable!() };
    next.selected = Some(SavedUuid::default());
    shared.apply(setup::Routing::Source(next), false).unwrap();
    // Raw rewind creates CLOCK_FAULT. The host accepts the Choke but refuses
    // every neutral pedal CC, so no final seal is yet honest.
    let release = source.run(0, vec![], Some(CLAP_EVENT_MIDI));
    assert_eq!(release.values.len(), 1);
    assert!(release.values[0].1.release());
    assert!(source.source_snapshot().seal.is_none());
    assert_eq!(session.credits.load(Ordering::Acquire), 1);
    let pedals = source.run(64, vec![], None);
    assert_eq!(pedals.values.len(), 3);
    assert!(pedals
        .values
        .iter()
        .all(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 64 | 66 | 69, 0], .. })));
    let sealed = source.source_snapshot();
    assert!(sealed.seal.is_some());
    assert_eq!(sealed.faults, source::CLOCK_FAULT);
    assert_eq!(
        session.credits.load(Ordering::Acquire),
        1,
        "termination awaits final history acknowledgement"
    );
    drop(source); // Stop adds OUTPUT_FAULT to the already final CLOCK_FAULT cut.
    assert_eq!(shared.status.load(Ordering::Acquire), source::CLOCK_FAULT | source::OUTPUT_FAULT);
    drop(hub); // both owners are quiesced: there is no future callback to emit new debt
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert!(!registry::global().lock().unwrap().test_retained_hub(session.runtime));
}

#[test]
fn all_retired_peers_drain_a_full_actual_reply_window_without_a_live_callback() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    let mut state = source.save();
    state.fields.insert(
        setup::SOURCE_FIELD.into(),
        serde_json::to_string(&SourceSetup {
            selected: Some(uuid),
            calibration: Calibration {
                offset: 65536,
                sample_rate: 48000.0,
                max_frames: 64,
                validated: true,
            },
        })
        .unwrap(),
    );
    assert!(source.load(&state));
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    assert_eq!(
        source
            .run(64, vec![note(73, 0, 60, 3, true), note(73, 0, 60, 23, false)], None)
            .values
            .len(),
        2
    );
    // Its calibrated report is far ahead. The hub retains it while actual
    // callbacks advance, publishing1024 genuine distinct continuous acks.
    for block in 1..=1025 {
        hub.run(block * 64, vec![], None);
    }
    let wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    assert_eq!(
        wrapper.test_inspect_plugin(|plugin| plugin
            .aggregation
            .as_ref()
            .unwrap()
            .offer
            .as_ref()
            .unwrap()
            .bank
            .rows[0]
            .replies
            .slots()),
        0
    );
    assert_eq!(session.credits.load(Ordering::Acquire), 1);
    drop(hub);
    assert!(registry::global().lock().unwrap().test_retained_hub(session.runtime));
    drop(source);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert!(!registry::global().lock().unwrap().test_retained_hub(session.runtime));
}

#[test]
fn observed_callback_cost_at_empty_and_full_session_state() {
    let _scope = crate::test_scope::enter();
    fn report(name: &str, mut times: Vec<u128>) {
        times.sort_unstable();
        let mean = times.iter().sum::<u128>() as f64 / times.len() as f64;
        println!("CALLBACK {name} n={} mean_ns={mean:.0} p50_ns={} p95_ns={} max_ns={} debug_assertions={}",
            times.len(), times[times.len()/2], times[times.len()*95/100], times[times.len()-1], cfg!(debug_assertions));
    }
    let calibration =
        Calibration { offset: 0, sample_rate: 44100.0, max_frames: 512, validated: true };
    let run = |device: &Device, block: i64, events: Vec<Input>| {
        device.run_format(block * 512, events, None, None, 512)
    };
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, calibration);
    hub.activate_format(44100.0, 512);
    let mut sources: Vec<_> = (0..16)
        .map(|_| {
            let mut source = Device::new(true);
            source.configure_format(uuid, true, calibration);
            source.activate_format(44100.0, 512);
            source
        })
        .collect();
    for source in &sources {
        run(source, 0, vec![]);
    }
    run(&hub, 0, vec![]);
    let mut empty_hub = Vec::with_capacity(256);
    let mut empty_source = Vec::with_capacity(256);
    for block in 1..=256 {
        for (index, source) in sources.iter().enumerate() {
            let value = run(source, block, vec![]);
            if index == 0 {
                empty_source.push(value.callback_nanos);
            }
        }
        empty_hub.push(run(&hub, block, vec![]).callback_nanos);
    }
    for (index, source) in sources.iter().enumerate() {
        let events = if index < 4 {
            (0..64).map(|key| note(key + 1, 0, key as i16, 0, true)).collect()
        } else {
            vec![]
        };
        assert_eq!(run(source, 257, events).values.len(), if index < 4 { 64 } else { 0 });
    }
    let burst = run(&hub, 257, vec![]).callback_nanos;
    let session = registry::global().lock().unwrap().test_session(uuid);
    assert_eq!(session.credits.load(Ordering::Acquire), 256);
    let mut full_hub = Vec::with_capacity(256);
    let mut full_source = Vec::with_capacity(256);
    for block in 258..=513 {
        for (index, source) in sources.iter().enumerate() {
            let value = run(source, block, vec![]);
            if index == 0 {
                full_source.push(value.callback_nanos);
            }
        }
        full_hub.push(run(&hub, block, vec![]).callback_nanos);
    }
    report("empty-source", empty_source);
    report("empty-hub16enrolled", empty_hub);
    report("held64-source", full_source);
    report("held256-hub16enrolled", full_hub);
    println!("CALLBACK actual256-onset hub merge_ns={burst};44100Hz/512frames; observations include whole wrapper/guard and are not WCET");
    for (index, source) in sources.iter().enumerate() {
        run(
            source,
            514,
            if index < 4 {
                (0..64).map(|key| note(key + 1, 0, key as i16, 0, false)).collect()
            } else {
                vec![]
            },
        );
    }
    run(&hub, 514, vec![]);
    for source in &sources {
        run(source, 515, vec![]);
    }
    run(&hub, 515, vec![]);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    sources.clear();
    drop(hub);
}

#[test]
fn session_reservation_257_waits_for_real_retention_including_direct_and_off() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut sources: Vec<_> = (0..4)
        .map(|index| {
            let mut source = Device::new(true);
            source.configure(uuid, index != 2);
            source.activate();
            source
        })
        .collect();
    for source in &sources {
        source.run(0, vec![], None);
    }
    hub.run(0, vec![], None);
    for (index, source) in sources.iter().enumerate() {
        let notes = if index < 3 {
            (0..64).map(|key| note(key + 1, 0, key as i16, 0, true)).collect()
        } else {
            vec![]
        };
        assert_eq!(source.run(64, notes, None).values.len(), if index < 3 { 64 } else { 0 });
    }
    assert_eq!(
        hub.run(64, (0..64).map(|key| note(key + 1, 0, key as i16, 0, true)).collect(), None)
            .values
            .len(),
        64
    );
    assert_eq!(session.credits.load(Ordering::Acquire), 256);
    for source in &sources[..3] {
        source.run(128, vec![], None);
    }
    assert!(sources[3]
        .run(128, vec![note(77, 1, 67, 3, true), note(77, 1, 67, 19, false)], None)
        .values
        .is_empty());
    assert_eq!(sources[3].source_snapshot().pending, 2);
    assert_eq!(
        sources[3].source_snapshot().faults,
        0,
        "session admission backpressure is not fabricated output failure"
    );
    assert_eq!(session.credits.load(Ordering::Acquire), 256);
    // DIRECT's accepted Off64 and actual merged complete frontier release its
    // own64 reservations. The Off companion remains charged alongside the others.
    hub.run(128, (0..64).map(|key| note(key + 1, 0, key as i16, 0, false)).collect(), None);
    assert_eq!(session.credits.load(Ordering::Acquire), 192);
    for source in &sources[..3] {
        source.run(192, vec![], None);
    }
    let resumed = sources[3].run(192, vec![], None);
    assert_eq!(resumed.values.len(), 2);
    assert_eq!((resumed.values[0].0, resumed.values[1].0), (0, 16));
    assert_eq!(
        session.credits.load(Ordering::Acquire),
        193,
        "accepted release is still charged until output retention"
    );
    hub.run(192, vec![], None);
    for source in &sources[..3] {
        source.run(256, (0..64).map(|key| note(key + 1, 0, key as i16, 0, false)).collect(), None);
    }
    sources[3].run(256, vec![], None);
    assert_eq!(session.credits.load(Ordering::Acquire), 192);
    hub.run(256, vec![], None);
    for source in &sources {
        source.run(320, vec![], None);
    }
    hub.run(320, vec![], None);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    sources.clear();
    drop(hub);
}

#[test]
fn source_65th_held_attack_is_contained_without_releasing_unacknowledged_credits() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, false);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    assert_eq!(
        source
            .run(64, (0..64).map(|key| note(key + 1, 0, key as i16, 0, true)).collect(), None)
            .values
            .len(),
        64
    );
    hub.run(64, vec![], None);
    let contained = source.run(128, vec![note(65, 0, 64, 0, true)], None);
    assert_eq!(contained.values.len(), 67);
    assert_eq!(
        contained
            .values
            .iter()
            .filter(|(_, event)| matches!(event, Event::Note { kind: CLAP_EVENT_NOTE_CHOKE, .. }))
            .count(),
        64
    );
    assert!(contained.values[64..]
        .iter()
        .all(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 64 | 66 | 69, 0], .. })));
    assert_ne!(source.source_snapshot().faults & source::STORAGE_FAULT, 0);
    assert_eq!(session.credits.load(Ordering::Acquire), 64);
    hub.run(128, vec![], None);
    source.run(192, vec![], None);
    hub.run(192, vec![], None);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert_eq!(source.source_snapshot().held, 0);
}

#[test]
fn blocked_older_attack_does_not_hold_completed_nonhead_cells_past_8192_events() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut target = Device::new(true);
    target.configure(uuid, false);
    target.activate();
    let fillers: Vec<_> = (0..3)
        .map(|_| {
            let mut source = Device::new(true);
            source.configure(uuid, true);
            source.activate();
            source
        })
        .collect();
    target.run(0, vec![], None);
    for source in &fillers {
        source.run(0, vec![], None);
    }
    hub.run(0, vec![], None);
    target.run(64, vec![note(1, 0, 60, 0, true)], None);
    for source in &fillers {
        source.run(64, (0..64).map(|key| note(key + 1, 0, key as i16, 0, true)).collect(), None);
    }
    hub.run(64, (0..63).map(|key| note(key + 1, 0, key as i16, 0, true)).collect(), None);
    assert_eq!(session.credits.load(Ordering::Acquire), 256);
    assert!(target.run(128, vec![note(2, 0, 61, 1, true)], None).values.is_empty());
    for source in &fillers {
        source.run(128, vec![], None);
    }
    hub.run(128, vec![], None);
    for block in 3..=50 {
        let accepted = target.run(
            block * 64,
            (0..200).map(|_| expression(1, 0.1234567890123, 0)).collect(),
            None,
        );
        assert_eq!(accepted.values.len(), 200);
        assert!(accepted
            .values
            .iter()
            .all(|(_, event)| matches!(event, Event::Expression { id: 1, .. })));
        assert_eq!(
            target.source_snapshot().pending,
            1,
            "completed later cells are reusable while the old attack remains blocked"
        );
        for source in &fillers {
            source.run(block * 64, vec![], None);
        }
        hub.run(block * 64, vec![], None);
    }
    let release = target.run(51 * 64, vec![note(1, 0, 60, 7, false)], None);
    assert_eq!(release.values.len(), 1);
    assert_eq!(release.values[0].0, 7);
    assert!(release.values[0].1.release());
    assert_eq!(target.source_snapshot().pending, 1);
    for source in &fillers {
        source.run(51 * 64, vec![], None);
    }
    hub.run(51 * 64, vec![], None);
    let shared = target.shared();
    shared.apply(shared.value().routing, true).unwrap();
    for block in 52..=58 {
        assert!(target.run(block * 64, vec![], None).values.is_empty());
        for source in &fillers {
            source.run(block * 64, vec![], None);
        }
        hub.run(block * 64, vec![], None);
        target.main();
        hub.main();
    }
    assert_eq!(target.source_snapshot().pending, 0);
    assert_eq!(target.source_snapshot().lives, 0);
    for source in &fillers {
        source.run(
            59 * 64,
            (0..64).map(|key| note(key + 1, 0, key as i16, 0, false)).collect(),
            None,
        );
    }
    target.run(59 * 64, vec![], None);
    hub.run(59 * 64, (0..63).map(|key| note(key + 1, 0, key as i16, 0, false)).collect(), None);
    for source in &fillers {
        source.run(60 * 64, vec![], None);
    }
    target.run(60 * 64, vec![], None);
    hub.run(60 * 64, vec![], None);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    drop(fillers);
    drop(target);
    drop(hub);
}

#[test]
fn one_actual_all_notes_off_targets_original_lifetimes_before_same_key_retrigger() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    capture.drain_canonical();
    let cc = Input::Midi(clap_event_midi {
        header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 20),
        port_index: 0,
        data: [0xb0, 123, 0],
    });
    let accepted = source.run(
        64,
        vec![
            note(1, 0, 60, 3, true),
            note(2, 0, 64, 5, true),
            cc,
            note(3, 0, 60, 30, true),
            note(4, 1, 60, 35, true),
        ],
        None,
    );
    assert_eq!(accepted.values.len(), 5);
    assert_eq!(
        accepted.values.iter().map(|(time, _)| *time).collect::<Vec<_>>(),
        [3, 5, 20, 30, 35]
    );
    assert_eq!(
        accepted
            .values
            .iter()
            .filter(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 123, 0], .. }))
            .count(),
        1
    );
    assert_eq!(session.credits.load(Ordering::Acquire), 4);
    hub.run(64, vec![], None);
    let records = capture.drain_canonical();
    let deltas: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta) => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(deltas.len(), 6);
    assert_eq!(deltas.iter().filter(|delta| matches!(delta.event.kind, NoteKind::Off)).count(), 2);
    assert!(deltas
        .iter()
        .filter(|delta| matches!(delta.event.kind, NoteKind::Off))
        .all(|delta| delta.timing.unwrap().sample == 84));
    assert_ne!(deltas[0].lifetime, deltas[4].lifetime);
    let mut tracker = harmonigraph_core::NoteTracker::default();
    for record in records {
        record.apply(&mut tracker).unwrap();
    }
    assert_eq!(tracker.held_count(), 2);
    let offs = source.run(128, vec![note(3, 0, 60, 1, false), note(4, 1, 60, 2, false)], None);
    assert_eq!(offs.values.len(), 2);
    assert_eq!(session.credits.load(Ordering::Acquire), 2);
    hub.run(128, vec![], None);
    source.run(192, vec![], None);
    hub.run(192, vec![], None);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
}

#[test]
fn channel_references_leave_all_8192_original_event_slots_available() {
    let _scope = crate::test_scope::enter();
    let mut source = Device::new(true);
    source.configure(SavedUuid::default(), true);
    source.activate();
    source.run(0, (0..64).map(|key| note(key + 1, 0, key as i16, 0, true)).collect(), None);
    for block in 1..=511 {
        let cc = Input::Midi(clap_event_midi {
            header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
            port_index: 0,
            data: [0xb0, 1, (block % 128) as u8],
        });
        assert!(source.run(block * 64, vec![cc], None).values.is_empty());
    }
    let mut remaining = 7617;
    let mut block = 512;
    while remaining != 0 {
        let count = remaining.min(1024);
        let events = (0..count)
            .map(|_| {
                Input::Midi(clap_event_midi {
                    header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
                    port_index: 0,
                    data: [0xf8, 0, 0],
                })
            })
            .collect();
        assert!(source.run(block * 64, events, None).values.is_empty());
        remaining -= count;
        block += 1;
    }
    let snapshot = source.source_snapshot();
    assert_eq!(
        (
            snapshot.pending,
            snapshot.input_cut,
            snapshot.references,
            snapshot.reference_high_water,
            snapshot.faults
        ),
        (8192, 8192, 511 * 64, 511 * 64, 0)
    );
    let registration = source.shared().registration().unwrap();
    drop(source);
    assert!(!registry::global().lock().unwrap().test_has_source(registration));
}

#[test]
fn channel_reference_exhaustion_preserves_the_original_unconsumed_event() {
    let _scope = crate::test_scope::enter();
    let mut source = Device::new(true);
    source.configure(SavedUuid::default(), true);
    source.activate();
    source.run(0, (0..64).map(|key| note(key + 1, 0, key as i16, 0, true)).collect(), None);
    for block in 1..=512 {
        let cc = Input::Midi(clap_event_midi {
            header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
            port_index: 0,
            data: [0xb0, 1, (block % 128) as u8],
        });
        assert!(source.run(block * 64, vec![cc], None).values.is_empty());
    }
    let before = source.source_snapshot();
    assert_eq!(
        (before.pending, before.references, before.input_cut, before.faults),
        (576, 32768, 576, 0)
    );
    let cc = Input::Midi(clap_event_midi {
        header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
        port_index: 0,
        data: [0xb0, 7, 23],
    });
    assert!(source.run(513 * 64, vec![cc], None).values.is_empty());
    let after = source.source_snapshot();
    assert_eq!((after.pending, after.references, after.input_cut), (576, 32768, 576));
    assert_ne!(after.faults & source::REFERENCE_FAULT, 0);
    let registration = source.shared().registration().unwrap();
    drop(source);
    assert!(!registry::global().lock().unwrap().test_has_source(registration));
}

#[test]
fn partial_wildcard_acceptance_keeps_one_input_until_remaining_child_disposition() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    capture.drain_canonical();
    source.run(64, vec![note(1, 0, 60, 3, true), note(2, 0, 64, 5, true)], None);
    hub.run(64, vec![], None);
    let value = 0.1234567890123456;
    let accepted = source.run_select(128, vec![expression(-1, value, 11)], None, Some(2));
    assert_eq!(
        accepted
            .values
            .iter()
            .filter(|(_, event)| matches!(event, Event::Expression { .. }))
            .count(),
        1
    );
    assert!(
        matches!(accepted.values[0], (11, Event::Expression { id: 1, value: actual, .. }) if actual == value)
    );
    assert_eq!(accepted.values.iter().filter(|(_, event)| event.release()).count(), 2);
    assert_eq!(session.credits.load(Ordering::Acquire), 2);
    let partial = source.source_snapshot();
    assert_eq!((partial.pending, partial.references, partial.input_cut), (1, 2, 3));
    source.run(192, vec![], None);
    let waiting = source.source_snapshot();
    assert_eq!((waiting.pending, waiting.references, waiting.manifest), (1, 2, 1));
    assert_eq!(
        session.credits.load(Ordering::Acquire),
        2,
        "no output-retention acknowledgement has arrived"
    );
    hub.run(128, vec![], None);
    hub.run(192, vec![], None);
    source.run(256, vec![], None);
    hub.run(256, vec![], None);
    let settled = source.source_snapshot();
    assert_eq!(
        (settled.pending, settled.references, settled.manifest, settled.lives),
        (0, 0, 0, 0)
    );
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    let records = capture.drain_canonical();
    let notes: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta) => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(
        notes.iter().filter(|delta| matches!(delta.event.kind, NoteKind::Tuning { .. })).count(),
        1
    );
    assert_eq!(notes.iter().filter(|delta| matches!(delta.event.kind, NoteKind::Off)).count(), 2);
    let mut tracker = harmonigraph_core::NoteTracker::default();
    for record in records {
        record.apply(&mut tracker).unwrap();
    }
    assert_eq!(tracker.held_count(), 0);
}

#[test]
fn sixty_four_channel_terminals_keep_pedals_and_original_sound_off_wire_obligations() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    for controller in [120, 123] {
        let uuid = SavedUuid::default();
        let (mut hub, mut capture) = Device::recorded_hub();
        hub.configure(uuid, true);
        hub.activate();
        let session = registry::global().lock().unwrap().test_session(uuid);
        let mut source = Device::new(true);
        source.configure(uuid, true);
        source.activate();
        source.run(0, vec![], None);
        hub.run(0, vec![], None);
        capture.drain_canonical();
        assert_eq!(
            source
                .run(64, (0..64).map(|key| note(key + 1, 0, key as i16, 0, true)).collect(), None)
                .values
                .len(),
            64
        );
        hub.run(64, vec![], None);
        let cc = |controller, value, time| {
            Input::Midi(clap_event_midi {
                header: header::<clap_event_midi>(CLAP_EVENT_MIDI, time),
                port_index: 0,
                data: [0xb0, controller, value],
            })
        };
        let wire = source.run(128, vec![cc(64, 127, 1), cc(controller, 0, 9)], None);
        assert_eq!(
            wire.values.len(),
            2,
            "64 captured terminals derive from one controller acceptance"
        );
        assert!(wire.values.iter().all(|(_, event)| matches!(event, Event::Midi { .. })));
        assert_eq!(session.credits.load(Ordering::Acquire), 64);
        assert!(source.source_snapshot().pedals_held);
        hub.run(128, vec![], None);
        source.run(192, vec![], None);
        hub.run(192, vec![], None);
        assert_eq!(session.credits.load(Ordering::Acquire), 0);
        assert_eq!(source.source_snapshot().note_off_owed, if controller == 120 { 64 } else { 0 });
        let original_offs = source.run(
            256,
            (0..64).map(|key| note(key + 1, 0, key as i16, 7, false)).collect(),
            None,
        );
        assert_eq!(original_offs.values.len(), if controller == 120 { 64 } else { 0 });
        assert!(original_offs.values.iter().all(|(time, event)| *time == 7
            && matches!(event, Event::Note { kind: CLAP_EVENT_NOTE_OFF, .. })));
        assert!(source.source_snapshot().pedals_held);
        hub.run(256, vec![], None);
        assert_eq!(source.run(320, vec![cc(64, 0, 13)], None).values.len(), 1);
        hub.run(320, vec![], None);
        source.run(384, vec![], None);
        hub.run(384, vec![], None);
        let state = source.source_snapshot();
        assert_eq!(
            (
                state.pending,
                state.references,
                state.lives,
                state.held,
                state.note_off_owed,
                state.faults
            ),
            (0, 0, 0, 0, 0, 0)
        );
        assert!(!state.pedals_held);
        let records = capture.drain_canonical();
        let off_count = records.iter().filter(|record| matches!(record, CanonicalRecord::Delta(delta) if matches!(delta.event.kind, NoteKind::Off))).count();
        assert_eq!(
            off_count, 64,
            "the later owed wire releases do not fabricate second logical note endings"
        );
        let mut tracker = harmonigraph_core::NoteTracker::default();
        for record in records {
            record.apply(&mut tracker).unwrap();
        }
        assert_eq!(tracker.held_count(), 0);
    }
}

#[test]
fn direct_channel_termination_observes_original_lifetimes_independently_of_forwarding() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(SavedUuid::default(), true);
    hub.activate();
    hub.run(0, vec![], None);
    capture.drain_canonical();
    let cc = Input::Midi(clap_event_midi {
        header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 20),
        port_index: 0,
        data: [0xb0, 123, 0],
    });
    let wire = hub.run(
        64,
        vec![note(1, 0, 60, 3, true), note(2, 0, 64, 5, true), cc, note(3, 0, 60, 30, true)],
        None,
    );
    assert_eq!(wire.values.len(), 4);
    let records = capture.drain_canonical();
    let deltas: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta) => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(deltas.len(), 5);
    assert!(deltas.iter().all(|delta| delta.event.source == 0
        && delta.provenance == harmonigraph_take::canonical::ProvenanceRecord::ObservedDirect));
    assert_eq!(deltas.iter().filter(|delta| matches!(delta.event.kind, NoteKind::Off)).count(), 2);
    assert_ne!(deltas[0].lifetime, deltas[4].lifetime);
    let mut tracker = harmonigraph_core::NoteTracker::default();
    for record in records {
        record.apply(&mut tracker).unwrap();
    }
    assert_eq!(tracker.held_count(), 1);
    hub.run(128, vec![note(3, 0, 60, 0, false)], None);
}

#[test]
fn all_sixteen_retired_sources_dispose_full_event_reference_and_intent_owners_without_callbacks() {
    let _scope = crate::test_scope::enter();
    if std::env::var_os("HARMONIGRAPH_RETIRED_REFERENCES_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "performance::tests::all_sixteen_retired_sources_dispose_full_event_reference_and_intent_owners_without_callbacks", "--nocapture", "--test-threads=1"])
            .env("HARMONIGRAPH_RETIRED_REFERENCES_CHILD", "1").status().unwrap();
        assert!(status.success());
        return;
    }
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let sources: Vec<_> = (0..16)
        .map(|_| {
            let mut source = Device::new(true);
            source.configure(uuid, true);
            source.activate();
            source
        })
        .collect();
    for source in &sources {
        source.run(0, vec![], None);
    }
    hub.run(0, vec![], None);
    // Actual completed lifetimes in four sources occupy all256 reservations.
    // Hub callbacks then stop; no pending attack can become audible.
    for (index, source) in sources.iter().enumerate() {
        let events = if index < 4 {
            (0..64)
                .flat_map(|key| {
                    [note(key + 1, 0, key as i16, 1, true), note(key + 1, 0, key as i16, 1, false)]
                })
                .collect()
        } else {
            vec![]
        };
        assert_eq!(source.run(64, events, None).values.len(), if index < 4 { 128 } else { 0 });
    }
    assert_eq!(session.credits.load(Ordering::Acquire), 256);
    for source in &sources {
        assert!(source
            .run(128, (0..64).map(|key| note(key + 100, 0, key as i16, 0, true)).collect(), None)
            .values
            .is_empty());
        for block in 3..515 {
            let cc = Input::Midi(clap_event_midi {
                header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
                port_index: 0,
                data: [0xb0, 1, (block % 128) as u8],
            });
            assert!(source.run(block * 64, vec![cc], None).values.is_empty());
        }
        let mut remaining = 7616;
        let mut block = 515;
        while remaining != 0 {
            let count = remaining.min(1024);
            let events = (0..count)
                .map(|_| {
                    Input::Midi(clap_event_midi {
                        header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
                        port_index: 0,
                        data: [0xf8, 0, 0],
                    })
                })
                .collect();
            assert!(source.run(block * 64, events, None).values.is_empty());
            block += 1;
            remaining -= count;
        }
        for _ in 0..1024 {
            assert!(source.run(block * 64, vec![], None).values.is_empty());
            block += 1;
        }
        let snapshot = source.source_snapshot();
        assert_eq!(
            (snapshot.pending, snapshot.references, snapshot.intent_slots, snapshot.faults),
            (8192, 32768, 0, 0)
        );
    }
    let registrations: Vec<_> =
        sources.iter().map(|source| source.shared().registration().unwrap()).collect();
    drop(sources);
    // No hub callback runs between destruction of these16 sources and the hub.
    // Their real disposition messages are blocked behind the full intent rings.
    for registration in &registrations {
        assert!(registry::global().lock().unwrap().test_has_source(*registration));
    }
    drop(hub);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn future_progress_does_not_hide_an_earlier_complete_output_interval() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    source.run(64, vec![note(1, 0, 60, 0, true)], None);
    hub.run(64, vec![], None);
    capture.drain_canonical();
    assert_eq!(
        source.run(128, (0..400).map(|_| expression(1, 0.1, 0)).collect(), None).values.len(),
        400
    );
    hub.run(128, vec![], None);
    assert!(
        capture.drain_canonical().is_empty(),
        "the first256 collected records do not complete the400-event same-sample group"
    );
    assert_eq!(
        source.run(192, (0..400).map(|_| expression(1, 0.2, 0)).collect(), None).values.len(),
        400
    );
    hub.run(192, vec![], None);
    let first = capture.drain_canonical();
    let tuning: Vec<_> = first
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta)
                if matches!(delta.event.kind, NoteKind::Tuning { .. }) =>
            {
                Some(delta)
            }
            _ => None,
        })
        .collect();
    assert_eq!(tuning.len(), 400, "the future801 cut cannot erase the now-retained401 cut");
    assert!(tuning.iter().all(|delta| delta.timing.unwrap().sample == 128));
    source.run(256, vec![], None);
    hub.run(256, vec![], None);
    source.run(320, vec![], None);
    hub.run(320, vec![], None);
    let second = capture.drain_canonical();
    let tuning: Vec<_> = second
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta)
                if matches!(delta.event.kind, NoteKind::Tuning { .. }) =>
            {
                Some(delta)
            }
            _ => None,
        })
        .collect();
    assert_eq!(tuning.len(), 400);
    assert!(tuning.iter().all(|delta| delta.timing.unwrap().sample == 192));
    source.run(384, vec![note(1, 0, 60, 0, false)], None);
    hub.run(384, vec![], None);
    source.run(448, vec![], None);
    hub.run(448, vec![], None);
}

#[test]
fn destroyed_frozen_configuration_drains_more_than_a_full_source_output_window() {
    let _scope = crate::test_scope::enter();
    if std::env::var_os("HARMONIGRAPH_FROZEN_WINDOW_CHILD").is_none() {
        assert!(std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "performance::tests::destroyed_frozen_configuration_drains_more_than_a_full_source_output_window", "--nocapture", "--test-threads=1"])
            .env("HARMONIGRAPH_FROZEN_WINDOW_CHILD", "1").status().unwrap().success());
        return;
    }
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    capture.arm();
    assert_eq!(source.run(64, vec![note(1, 0, 60, 0, true)], None).values.len(), 1);
    hub.run(64, vec![], None);
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-frozen-window-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("record.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    for block in 0..9 {
        assert_eq!(
            source
                .run(128 + block * 64, (0..400).map(|_| expression(1, 0.125, 0)).collect(), None)
                .values
                .len(),
            400
        );
    }
    assert_eq!(source.run(704, vec![note(1, 0, 60, 0, false)], None).values.len(), 1);
    let snapshot = source.source_snapshot();
    assert_eq!((snapshot.sequence, snapshot.journal), (3602, 3601));
    let wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    let mailbox = wrapper.configuration_handle().unwrap();
    for value in 690..707 {
        mailbox
            .submit(crate::configuration::packet(
                harmonigraph_core::configuration::ConfigEdit::axis(1, value * 1_000_000),
            ))
            .unwrap();
    }
    hub.run(128, vec![], None);
    assert!(mailbox.visible().1);
    assert_eq!(
        wrapper.test_inspect_plugin(|plugin| plugin
            .configuration
            .as_ref()
            .unwrap()
            .recording
            .prefix),
        128
    );
    drop(mailbox);
    drop(hub);
    drop(source);
    writer.drain(&mut capture);
    let counts = registry::global().lock().unwrap().test_counts();
    assert_eq!(counts, (0,0,0), "joined final output beyond the frozen prefix must drain through the bounded window without a rescue callback");
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert!(writer.current_pass().is_none());
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(take.incomplete.is_some());
    let deltas: Vec<_> = take
        .events
        .iter()
        .filter_map(|record| match record {
            harmonigraph_take::CanonicalRecord::Delta(delta) => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(deltas.len(), 401, "only the On at64 and400 expressions at128 have an original Hub recording route; later source-only spans must be explicitly incomplete");
    assert_eq!(deltas[0].timing.unwrap().sample, 64);
    assert!(deltas[1..].iter().all(|delta| delta.timing.unwrap().sample == 128));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn destroyed_frozen_configuration_disposes_a_later_baseline_without_output() {
    let _scope = crate::test_scope::enter();
    if std::env::var_os("HARMONIGRAPH_FROZEN_BASELINE_CHILD").is_none() {
        assert!(std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "performance::tests::destroyed_frozen_configuration_disposes_a_later_baseline_without_output", "--nocapture", "--test-threads=1"])
            .env("HARMONIGRAPH_FROZEN_BASELINE_CHILD", "1").status().unwrap().success());
        return;
    }
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    capture.arm();
    assert_eq!(
        source.run(64, vec![note(1, 0, 60, 0, true), note(1, 0, 60, 23, false)], None).values.len(),
        2
    );
    hub.run(64, vec![], None);
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-frozen-baseline-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("record.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    assert!(source.run(128, vec![source.participation(false, 0)], None).values.is_empty());
    assert_eq!(source.source_snapshot().sequence, 2);
    let wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    let mailbox = wrapper.configuration_handle().unwrap();
    for value in 690..707 {
        mailbox
            .submit(crate::configuration::packet(
                harmonigraph_core::configuration::ConfigEdit::axis(1, value * 1_000_000),
            ))
            .unwrap();
    }
    hub.run(128, vec![], None);
    assert!(mailbox.visible().1);
    assert_eq!(
        wrapper.test_inspect_plugin(|plugin| plugin
            .configuration
            .as_ref()
            .unwrap()
            .recording
            .prefix),
        128
    );
    let row = wrapper
        .test_inspect_plugin(|plugin| plugin.aggregation.as_ref().unwrap().test_row_retirement(0));
    assert_eq!(row, (2,2,0,Some((2,191))), "the actual snapshot is later than frozen configuration and has no output delta to extend the drain extent");
    drop(mailbox);
    drop(hub);
    drop(source);
    writer.drain(&mut capture);
    let counts = registry::global().lock().unwrap().test_counts();
    assert_eq!(counts, (0, 0, 0));
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert!(writer.current_pass().is_none());
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(take.incomplete.is_some());
    assert_eq!(
        take.events
            .iter()
            .filter(|record| matches!(record, harmonigraph_take::CanonicalRecord::Delta(_)))
            .count(),
        2
    );
    assert!(take.events.iter().any(|record| matches!(record, harmonigraph_take::CanonicalRecord::Baseline(frame) if !frame.participating && (frame.t - 191.0 / 48000.0).abs() < 1e-12)));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn destroyed_frozen_configuration_drains_a_baseline_between_large_output_prefixes() {
    let _scope = crate::test_scope::enter();
    if std::env::var_os("HARMONIGRAPH_FROZEN_MIXED_CHILD").is_none() {
        assert!(std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "performance::tests::destroyed_frozen_configuration_drains_a_baseline_between_large_output_prefixes", "--nocapture", "--test-threads=1"])
            .env("HARMONIGRAPH_FROZEN_MIXED_CHILD", "1").status().unwrap().success());
        return;
    }
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    capture.arm();
    assert_eq!(source.run(64, vec![note(1, 0, 60, 0, true)], None).values.len(), 1);
    hub.run(64, vec![], None);
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-frozen-mixed-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("record.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    for block in 0..9 {
        let mut events: Vec<_> = (0..400).map(|_| expression(1, 0.125, 0)).collect();
        if block == 8 {
            events.insert(0, source.participation(false, 0));
        }
        assert_eq!(source.run(128 + block * 64, events, None).values.len(), 400);
    }
    assert_eq!(
        source.run(704, (0..400).map(|_| expression(1, 0.25, 0)).collect(), None).values.len(),
        400
    );
    assert_eq!(source.run(768, vec![note(1, 0, 60, 0, false)], None).values.len(), 1);
    assert_eq!(source.source_snapshot().baseline_cut, Some(3601));
    let snapshot = source.source_snapshot();
    assert_eq!((snapshot.sequence, snapshot.journal), (4002, 4001));
    let wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    let mailbox = wrapper.configuration_handle().unwrap();
    for value in 690..707 {
        mailbox
            .submit(crate::configuration::packet(
                harmonigraph_core::configuration::ConfigEdit::axis(1, value * 1_000_000),
            ))
            .unwrap();
    }
    hub.run(128, vec![], None);
    assert!(mailbox.visible().1);
    assert_eq!(
        wrapper.test_inspect_plugin(|plugin| plugin
            .configuration
            .as_ref()
            .unwrap()
            .recording
            .prefix),
        128
    );
    let row = wrapper
        .test_inspect_plugin(|plugin| plugin.aggregation.as_ref().unwrap().test_row_retirement(0));
    assert_eq!(row.3, Some((3601, 703)));
    drop(mailbox);
    drop(hub);
    drop(source);
    writer.drain(&mut capture);
    let counts = registry::global().lock().unwrap().test_counts();
    assert_eq!(counts, (0,0,0), "joined final output beyond the frozen prefix must drain through the bounded window without a rescue callback");
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert!(writer.current_pass().is_none());
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(take.incomplete.is_some());
    let deltas: Vec<_> = take
        .events
        .iter()
        .filter_map(|record| match record {
            harmonigraph_take::CanonicalRecord::Delta(delta) => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(deltas.len(), 401, "only the On at64 and400 expressions at128 have an original Hub recording route; later source-only spans must be explicitly incomplete");
    assert_eq!(deltas[0].timing.unwrap().sample, 64);
    assert!(deltas[1..].iter().all(|delta| delta.timing.unwrap().sample == 128));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn destroyed_frozen_configuration_drains_full_ordinary_and_emergency_journals() {
    let _scope = crate::test_scope::enter();
    if std::env::var_os("HARMONIGRAPH_FROZEN_EMERGENCY_CHILD").is_none() {
        for order in ["baseline", "withdrawn"] {
            assert!(std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "performance::tests::destroyed_frozen_configuration_drains_full_ordinary_and_emergency_journals", "--nocapture", "--test-threads=1"])
            .env("HARMONIGRAPH_FROZEN_EMERGENCY_CHILD", order).status().unwrap().success());
        }
        return;
    }
    let withdrawn = std::env::var("HARMONIGRAPH_FROZEN_EMERGENCY_CHILD").unwrap() == "withdrawn";
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    capture.arm();
    assert_eq!(
        source
            .run(64, (0..32).map(|key| note(key + 1, 0, key as i16, 0, true)).collect(), None)
            .values
            .len(),
        32
    );
    hub.run(64, vec![], None);
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-frozen-emergency-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("record.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    let midi = |data| {
        Input::Midi(clap_event_midi {
            header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
            port_index: 0,
            data,
        })
    };
    let mut first = vec![midi([0xb0, 120, 0])];
    first.extend((0..511).map(|_| midi([0xf8, 0, 0])));
    assert_eq!(source.run(128, first, None).values.len(), 512);
    assert_eq!(source.source_snapshot().journal, 544);
    assert_eq!(source.source_snapshot().note_off_owed, 32);
    for raw in (192..=512).step_by(64) {
        assert_eq!(
            source.run(raw, (0..512).map(|_| midi([0xf8, 0, 0])).collect(), None).values.len(),
            512
        );
    }
    assert_eq!(
        source.run(576, (0..480).map(|_| midi([0xf8, 0, 0])).collect(), None).values.len(),
        480
    );
    assert_eq!(source.source_snapshot().journal, 4096);
    let freeze = |hub: &mut Device, expected_baseline| {
        let wrapper = unsafe {
            &*((*hub.plugin)
                .plugin_data
                .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
        };
        let mailbox = wrapper.configuration_handle().unwrap();
        for value in 690..707 {
            mailbox
                .submit(crate::configuration::packet(
                    harmonigraph_core::configuration::ConfigEdit::axis(1, value * 1_000_000),
                ))
                .unwrap();
        }
        hub.run(128, vec![], None);
        assert_eq!(
            wrapper.test_inspect_plugin(|plugin| plugin
                .configuration
                .as_ref()
                .unwrap()
                .recording
                .prefix),
            128
        );
        let row = wrapper.test_inspect_plugin(|plugin| {
            plugin.aggregation.as_ref().unwrap().test_row_retirement(0)
        });
        assert_eq!(row.3, expected_baseline);
    };
    let mut hub = Some(hub);
    if withdrawn {
        freeze(hub.as_mut().unwrap(), None);
        drop(hub.take());
    }
    let emergency = source.run(640, vec![midi([0xf8, 0, 0])], None);
    assert_eq!(emergency.values.len(), 35);
    assert_eq!(emergency.values.iter().filter(|(_, e)| e.release()).count(), 32);
    let snapshot = source.source_snapshot();
    assert_eq!(
        (snapshot.sequence, snapshot.journal, snapshot.emergency, snapshot.baseline_cut),
        (4163, if withdrawn { 3584 } else { 4096 }, 35, (!withdrawn).then_some(4163))
    );
    assert!(snapshot.transfer_cut < snapshot.sequence);
    assert_ne!(
        snapshot.faults & if withdrawn { source::OUTPUT_FAULT } else { source::STORAGE_FAULT },
        0
    );
    if let Some(hub) = hub.as_mut() {
        freeze(hub, Some((4163, 703)));
    }
    drop(hub);
    drop(source);
    writer.drain(&mut capture);
    let counts = registry::global().lock().unwrap().test_counts();
    assert_eq!(counts, (0, 0, 0));
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert!(writer.current_pass().is_none());
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(take.incomplete.is_some());
    let deltas: Vec<_> = take
        .events
        .iter()
        .filter_map(|record| match record {
            harmonigraph_take::CanonicalRecord::Delta(delta) => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(deltas.len(), 64);
    assert!(deltas[..32].iter().all(|delta| delta.timing.unwrap().sample == 64
        && matches!(delta.event.kind, harmonigraph_take::NoteKind::On { .. })));
    assert!(deltas[32..].iter().all(|delta| delta.timing.unwrap().sample == 128
        && matches!(delta.event.kind, harmonigraph_take::NoteKind::Off)));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn progress_spanning_more_than_the_output_window_releases_only_complete_timestamp_prefixes() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    source.run(64, vec![note(1, 0, 60, 0, true)], None);
    hub.run(64, vec![], None);
    capture.drain_canonical();
    let mut source_raw = 128;
    let mut hub_raw = 128;
    for _ in 0..9 {
        assert_eq!(
            source
                .run(source_raw, (0..400).map(|_| expression(1, 0.125, 0)).collect(), None)
                .values
                .len(),
            400
        );
        source_raw += 64;
    }
    assert_eq!(source.source_snapshot().journal, 3600);
    for _ in 0..40 {
        source.run(source_raw, vec![], None);
        source_raw += 64;
        hub.run(hub_raw, vec![], None);
        hub_raw += 64;
    }
    let wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    assert!(wrapper.test_inspect_plugin(|plugin| plugin.aggregation.as_ref().unwrap().window_report_seen), "fixture must reach an incomplete retained report more than2048 output records beyond the applied prefix");
    let records = capture.drain_canonical();
    let tuning: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta)
                if matches!(delta.event.kind, NoteKind::Tuning { .. }) =>
            {
                Some(delta)
            }
            _ => None,
        })
        .collect();
    assert_eq!(tuning.len(), 3600);
    for (block, records) in tuning.chunks_exact(400).enumerate() {
        assert!(records
            .iter()
            .all(|delta| delta.timing.unwrap().sample == 128 + block as i64 * 64));
    }
    assert_eq!(source.source_snapshot().journal, 0);
    assert_eq!(source.source_snapshot().faults, 0);
    let off_at = source_raw;
    source.run(source_raw, vec![note(1, 0, 60, 0, false)], None);
    source_raw += 64;
    while hub_raw <= off_at + 64 {
        source.run(source_raw, vec![], None);
        source_raw += 64;
        hub.run(hub_raw, vec![], None);
        hub_raw += 64;
    }
    source.run(source_raw, vec![], None);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
}

#[test]
fn clock_calibration_preserves_enclosing_wire_offsets_across_transport_subblocks() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure_offset(uuid, true, 32);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure_offset(uuid, true, 64);
    source.activate();
    source.run(0, vec![], None);
    hub.run(32, vec![], None);
    capture.drain_canonical();
    let epoch = session.epoch.load(Ordering::Acquire);
    let accepted = source.run(
        64,
        vec![
            note(7, 2, 60, 5, true),
            transport(16, 120.0),
            expression(7, 0.1234567890123, 19),
            transport(40, 90.0),
            note(7, 2, 60, 43, false),
        ],
        None,
    );
    assert_eq!(accepted.values.iter().map(|(time, _)| *time).collect::<Vec<_>>(), [5, 19, 43]);
    hub.run(96, vec![transport(16, 120.0), transport(40, 90.0)], None);
    let records = capture.drain_canonical();
    let deltas: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta) => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(deltas.len(), 3, "{deltas:?}");
    assert_eq!(
        deltas.iter().map(|delta| delta.timing.unwrap().sample).collect::<Vec<_>>(),
        [133, 147, 171]
    );
    assert!(matches!(deltas[1].event.kind, NoteKind::Tuning { .. }));
    assert_eq!(session.epoch.load(Ordering::Acquire), epoch);
    assert_eq!(session.closing.load(Ordering::Acquire), 0);
    assert_eq!(source.source_snapshot().faults, 0);
    source.run(128, vec![], None);
    hub.run(160, vec![], None);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
}

#[test]
fn clock_missing_silent_member_blocks_canonical_output_until_actual_coverage_arrives() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::CanonicalRecord;
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut a = Device::new(true);
    a.configure(uuid, true);
    a.activate();
    let mut b = Device::new(true);
    b.configure(uuid, true);
    b.activate();
    a.run(0, vec![], None);
    b.run(0, vec![], None);
    hub.run(0, vec![], None);
    capture.drain_canonical();
    assert_eq!(
        a.run(64, vec![note(1, 0, 60, 0, true), note(1, 0, 60, 20, false)], None).values.len(),
        2
    );
    hub.run(64, vec![], None);
    assert!(capture.drain_canonical().is_empty());
    assert_eq!(a.source_snapshot().journal, 2);
    assert_eq!(session.credits.load(Ordering::Acquire), 1);
    // A callback that actually inspected the missing interval supplies proof.
    // Advancing the hub or observing an empty held baseline did not supply it.
    b.run(64, vec![], None);
    a.run(128, vec![], None);
    hub.run(128, vec![], None);
    let records = capture.drain_canonical();
    let samples: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta) => Some(delta.timing.unwrap().sample),
            _ => None,
        })
        .collect();
    assert_eq!(samples, [64, 84]);
    a.run(192, vec![], None);
    b.run(128, vec![], None);
    hub.run(192, vec![], None);
    assert!(capture
        .drain_canonical()
        .iter()
        .all(|record| !matches!(record, CanonicalRecord::Delta(_))));
    assert_eq!(a.source_snapshot().journal, 0);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
}

#[test]
fn clock_reinitialize_keeps_an_absent_held_member_until_it_resumes_contiguous_callbacks() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut a = Device::new(true);
    a.configure(uuid, true);
    a.activate();
    let mut b = Device::new(true);
    b.configure(uuid, false);
    b.activate();
    a.run(0, vec![], None);
    b.run(0, vec![], None);
    hub.run(0, vec![], None);
    a.run(64, vec![note(1, 0, 60, 0, true)], None);
    b.run(64, vec![note(1, 0, 60, 0, true)], None);
    hub.run(64, vec![], None);
    let epoch = session.epoch.load(Ordering::Acquire);
    let shared = hub.shared();
    shared.apply(shared.value().routing, true).unwrap();
    hub.run(128, vec![], None);
    for block in 2..=10 {
        a.run(block * 64, vec![], None);
        hub.run((block + 1) * 64, vec![], None);
        a.main();
        b.main();
        hub.main();
        assert_eq!(session.epoch.load(Ordering::Acquire), epoch);
        assert_ne!(session.closing.load(Ordering::Acquire), 0);
        assert!(session.credits.load(Ordering::Acquire) >= 1);
        assert_eq!(b.source_snapshot().held, 1);
    }
    // B was absent, not silently processed, rejected, or removed. Its next raw
    // interval is still128, and only that actual callback can accept its release.
    let released = b.run(128, vec![], None);
    assert_eq!(released.values.len(), 4);
    assert!(released.values[0].1.release());
    for block in 11..=30 {
        a.run(block * 64, vec![], None);
        b.run((block - 8) * 64, vec![], None);
        hub.run((block + 1) * 64, vec![], None);
        a.main();
        b.main();
        hub.main();
    }
    assert_eq!(session.epoch.load(Ordering::Acquire), epoch + 1);
    assert_eq!(session.closing.load(Ordering::Acquire), 0);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert_eq!(b.source_snapshot().held, 0);
}

#[test]
fn channel_terminal_journal_preflight_reserves_only_the_facts_the_wire_can_create() {
    let _scope = crate::test_scope::enter();
    for (already_terminated, free) in [(true, 64), (false, 64), (false, 65)] {
        let uuid = SavedUuid::default();
        let mut hub = Device::new(false);
        hub.configure(uuid, true);
        hub.activate();
        let session = registry::global().lock().unwrap().test_session(uuid);
        let mut source = Device::new(true);
        source.configure(uuid, true);
        source.activate();
        source.run(0, vec![], None);
        hub.run(0, vec![], None);
        assert_eq!(
            source
                .run(64, (0..64).map(|key| note(key + 1, 0, key as i16, 0, true)).collect(), None)
                .values
                .len(),
            64
        );
        hub.run(64, vec![], None);
        let cc = || {
            Input::Midi(clap_event_midi {
                header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
                port_index: 0,
                data: [0xb0, 120, 0],
            })
        };
        source.run(128, if already_terminated { vec![cc()] } else { vec![] }, None);
        let initial = source.source_snapshot();
        assert_eq!(initial.journal, if already_terminated { 65 } else { 0 });
        assert_eq!(initial.note_off_owed, if already_terminated { 64 } else { 0 });
        let before = 4096 - free;
        let mut remaining = before - initial.journal;
        let mut source_raw = 192;
        while remaining != 0 {
            let count = remaining.min(512);
            let input = (0..count)
                .map(|_| {
                    Input::Midi(clap_event_midi {
                        header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
                        port_index: 0,
                        data: [0xf8, 0, 0],
                    })
                })
                .collect();
            assert_eq!(source.run(source_raw, input, None).values.len(), count);
            source_raw += 64;
            remaining -= count;
        }
        assert_eq!(
            source.source_snapshot().journal,
            before,
            "exactly{free} ordinary journal cells remain"
        );
        let wire = source.run(source_raw, vec![cc()], None);
        source_raw += 64;
        let accepted = already_terminated || free == 65;
        if accepted {
            assert_eq!(
                wire.values.len(),
                1,
                "64 original bindings create terminal facts only once"
            );
            assert!(matches!(wire.values[0].1, Event::Midi { data: [0xb0, 120, 0], .. }));
            assert_eq!(
                source.source_snapshot().journal,
                before + if already_terminated { 1 } else { 65 }
            );
            assert_eq!(source.source_snapshot().faults, 0);
        } else {
            assert_eq!(source.source_snapshot().faults, source::STORAGE_FAULT);
            assert_eq!(source.source_snapshot().journal, 4032);
            assert_eq!(wire.values.len(), 67, "the unreservable65-fact channel group cannot attempt its wire; dedicated emergency still releases64 voices and neutralizes3 unknownpedals");
            assert_eq!(wire.values.iter().filter(|(_, event)| event.release()).count(), 64);
        }
        let mut hub_raw = 128;
        for _ in 0..32 {
            source.run(source_raw, vec![], None);
            source_raw += 64;
            hub.run(hub_raw, vec![], None);
            hub_raw += 64;
        }
        if accepted {
            assert_eq!(
                source
                    .run(
                        source_raw,
                        (0..64).map(|key| note(key + 1, 0, key as i16, 0, false)).collect(),
                        None
                    )
                    .values
                    .len(),
                64
            );
            source_raw += 64;
        }
        while hub_raw < source_raw + 64 {
            hub.run(hub_raw, vec![], None);
            hub_raw += 64;
        }
        source.run(source_raw, vec![], None);
        assert_eq!(session.credits.load(Ordering::Acquire), 0);
        assert_eq!(source.source_snapshot().note_off_owed, 0);
    }
}

#[test]
fn channel_terminal_sequence_preflight_checks_the_whole_actual_outcome_group() {
    let _scope = crate::test_scope::enter();
    if std::env::var_os("HARMONIGRAPH_SEQUENCE_LIMIT_CHILD").is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "performance::tests::channel_terminal_sequence_preflight_checks_the_whole_actual_outcome_group", "--nocapture", "--test-threads=1"])
            .env("HARMONIGRAPH_SEQUENCE_LIMIT_CHILD", "1").status().unwrap();
        assert!(status.success());
        return;
    }
    for (already_terminated, headroom) in [(false, 65), (false, 64), (true, 1), (true, 0)] {
        let uuid = SavedUuid::default();
        let mut hub = Device::new(false);
        hub.configure(uuid, true);
        hub.activate();
        let mut source = Device::new(true);
        source.configure(uuid, true);
        source.activate();
        source.run(0, vec![], None);
        hub.run(0, vec![], None);
        source.run(64, (0..64).map(|key| note(key + 1, 0, key as i16, 0, true)).collect(), None);
        hub.run(64, vec![], None);
        let cc = || {
            Input::Midi(clap_event_midi {
                header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
                port_index: 0,
                data: [0xb0, 120, 0],
            })
        };
        source.run(128, if already_terminated { vec![cc()] } else { vec![] }, None);
        hub.run(128, vec![], None);
        source.run(192, vec![], None);
        hub.run(192, vec![], None);
        assert_eq!(source.source_snapshot().journal, 0);
        let wrapper = unsafe {
            &*((*source.plugin)
                .plugin_data
                .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
        };
        // Only the counter is preset. Actual lifetimes, captured64-target header,
        // wrapper prepare/host acceptance and both factual journals remain real.
        wrapper.test_with_plugin(|plugin| {
            plugin.source.as_mut().unwrap().sequence = u64::MAX - 128 - headroom
        });
        let wire = source.run(256, vec![cc()], None);
        let snapshot = source.source_snapshot();
        if headroom == 64 || headroom == 0 {
            assert_eq!(snapshot.faults, source::STORAGE_FAULT);
            assert_eq!(wire.values.iter().filter(|(_, event)| event.release()).count(), 64);
            assert_eq!(wire.values.len(), 67);
            assert_eq!(snapshot.journal, 0);
            assert_eq!(snapshot.emergency, wire.values.len());
            assert!(snapshot.sequence < u64::MAX);
        } else {
            assert_eq!(snapshot.faults, 0);
            assert_eq!(wire.values.len(), 1);
            assert!(matches!(wire.values[0].1, Event::Midi { data: [0xb0, 120, 0], .. }));
            assert_eq!(snapshot.journal, headroom as usize);
            assert_eq!(snapshot.sequence, u64::MAX - 128);
            let overflow = Input::Midi(clap_event_midi {
                header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
                port_index: 0,
                data: [0xf8, 0, 0],
            });
            let emergency = source.run(320, vec![overflow], None);
            assert_eq!(emergency.values.len(), 67, "the next ordinary event cannot consume numbers reserved for64 owed Note-Offs and3 neutral pedals");
            assert_eq!(emergency.values.iter().filter(|(_, event)| event.release()).count(), 64);
            assert_eq!(source.source_snapshot().faults, source::STORAGE_FAULT);
            assert_eq!(source.source_snapshot().note_off_owed, 0);
            assert_eq!(source.source_snapshot().sequence, u64::MAX - 61);
        }
        // The artificial counter jump cannot supply the missing preceding
        // history to the hub. This isolated fixture intentionally leaves those
        // exhausted owners pinned; it makes no retirement/continuity claim.
    }
}

#[test]
fn sequence_reserve_preserves_real_emergency_history_and_acknowledged_credits() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    let mut initial: Vec<_> = (0..16)
        .map(|channel| {
            Input::Midi(clap_event_midi {
                header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
                port_index: 0,
                data: [0xb0 | channel, 64, 127],
            })
        })
        .collect();
    initial.extend((0..64).map(|id| note(id + 1, (id / 4) as i16, (60 + id % 4) as i16, 1, true)));
    assert_eq!(source.run(64, initial, None).values.len(), 80);
    hub.run(64, vec![], None);
    source.run(128, vec![], None);
    hub.run(128, vec![], None);
    capture.drain_canonical();
    let source_wrapper = unsafe {
        &*((*source.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
    };
    let hub_wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    // The fixture declares a consistent previously retained prefix on both
    // actual owners; it does not pretend to execute2^64 preceding wire events.
    let prefix = u64::MAX - 129;
    let lease = source_wrapper.test_with_plugin(|plugin| {
        plugin.source.as_mut().unwrap().test_rebase_output_prefix(prefix)
    });
    hub_wrapper.test_with_plugin(|plugin| {
        plugin.aggregation.as_mut().unwrap().test_rebase_output_prefix(lease, prefix)
    });
    assert_eq!(source.run(192, vec![expression(1, 0.125, 0)], None).values.len(), 1);
    assert_eq!(source.source_snapshot().sequence, u64::MAX - 128);
    let blocked = Input::Midi(clap_event_midi {
        header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
        port_index: 0,
        data: [0xf8, 0, 0],
    });
    let released = source.run(256, vec![blocked], None);
    assert_eq!(released.attempts, 112);
    assert_eq!(released.values.iter().filter(|(_, event)| event.release()).count(), 64);
    assert_eq!(source.source_snapshot().sequence, u64::MAX - 16);
    assert_eq!(session.credits.load(Ordering::Acquire), 64);
    let malformed = Input::Midi(clap_event_midi {
        header: clap_event_header {
            size: std::mem::size_of::<clap_event_header>() as u32,
            ..header::<clap_event_midi>(CLAP_EVENT_MIDI, 0)
        },
        port_index: 0,
        data: [0xf8, 0, 0],
    });
    assert_eq!(source.run_status(320, vec![malformed], None, None, 64, true).attempts, 0);
    assert_eq!(source.source_snapshot().sequence, u64::MAX - 16);
    for block in 3..=12 {
        hub.run(block * 64, vec![], None);
        source.run((block + 3) * 64, vec![], None);
    }
    let records = capture.drain_canonical();
    let deltas: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta) => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(
        deltas.iter().filter(|delta| matches!(delta.event.kind, NoteKind::Tuning { .. })).count(),
        1
    );
    assert_eq!(deltas.iter().filter(|delta| matches!(delta.event.kind, NoteKind::Off)).count(), 64);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert_eq!((source.source_snapshot().journal, source.source_snapshot().emergency), (0, 0));
}

#[test]
fn final_sequence_terminal_is_retained_once_and_acknowledged_in_mapped_and_sealed_streams() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    for mapped in [true, false] {
        let uuid = SavedUuid::default();
        let (mut hub, mut capture) = Device::recorded_hub();
        hub.configure(uuid, true);
        hub.activate();
        let session = registry::global().lock().unwrap().test_session(uuid);
        let mut source = Device::new(true);
        source.configure(uuid, true);
        source.activate();
        capture.arm();
        source.run(0, vec![], None);
        hub.run(0, vec![], None);
        let directory = std::env::temp_dir()
            .join(format!("harmonigraph-last-sequence-{}-{mapped}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("last.take");
        let mut writer =
            harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
        writer.drain(&mut capture);
        let mut neutral: Vec<_> = [64, 66, 69]
            .into_iter()
            .map(|controller| {
                Input::Midi(clap_event_midi {
                    header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
                    port_index: 0,
                    data: [0xb0, controller, 0],
                })
            })
            .collect();
        neutral.push(note(1, 0, 60, 1, true));
        source.run(64, neutral, None);
        hub.run(64, vec![], None);
        writer.drain(&mut capture);
        source.run(128, vec![], None);
        hub.run(128, vec![], None);
        writer.drain(&mut capture);
        let source_wrapper = unsafe {
            &*((*source.plugin)
                .plugin_data
                .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
        };
        let hub_wrapper = unsafe {
            &*((*hub.plugin)
                .plugin_data
                .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
        };
        let lease = source_wrapper.test_with_plugin(|plugin| {
            plugin.source.as_mut().unwrap().test_rebase_output_prefix(u64::MAX - 1)
        });
        hub_wrapper.test_with_plugin(|plugin| {
            plugin.aggregation.as_mut().unwrap().test_rebase_output_prefix(lease, u64::MAX - 1)
        });
        if mapped {
            let shared = source.shared();
            shared.apply(shared.value().routing, true).unwrap();
        }
        let raw = if mapped { 192 } else { 0 };
        let output = source.run(raw, vec![], None);
        assert_eq!(output.values.len(), 1);
        assert!(output.values[0].1.release());
        assert_eq!(source.source_snapshot().sequence, u64::MAX);
        assert_eq!(session.credits.load(Ordering::Acquire), 1);
        source_wrapper.test_with_plugin(|plugin| {
            plugin.source.as_mut().unwrap().test_repeat_emergency_output()
        });
        hub.run(192, vec![], None);
        writer.drain(&mut capture);
        for block in 4..=16 {
            source.run(if mapped { block * 64 } else { (block - 3) * 64 }, vec![], None);
            hub.run(block * 64, vec![], None);
            writer.drain(&mut capture);
            source.main();
            hub.main();
        }
        assert_eq!(
            session.credits.load(Ordering::Acquire),
            0,
            "finalMAX must be acknowledged without requiringMAX+1: {:?}",
            source.source_snapshot()
        );
        assert_eq!((source.source_snapshot().journal, source.source_snapshot().emergency), (0, 0));
        capture.stop();
        writer.stop();
        source.run(if mapped { 17 * 64 } else { 14 * 64 }, vec![], None);
        hub.run(17 * 64, vec![], None);
        writer.drain(&mut capture);
        if mapped {
            assert!(writer.finished.is_some());
        } else {
            assert!(writer.failed());
        }
        let take = harmonigraph_take::Take::read(&path).unwrap();
        let final_records: Vec<_> = take
            .events
            .iter()
            .filter_map(|record| match record {
                CanonicalRecord::Delta(delta) if delta.sequence == u64::MAX => Some(delta),
                _ => None,
            })
            .collect();
        if mapped {
            assert!(take.incomplete.is_none());
            assert_eq!(
                final_records.len(),
                1,
                "the repeated actual-ring fact is not a second output"
            );
            assert!(matches!(final_records[0].event.kind, NoteKind::Off));
            assert_eq!(final_records[0].timing.unwrap().sample, 192);
        } else {
            assert!(take.incomplete.is_some());
            assert!(
                final_records.is_empty(),
                "an unmapped actual termination cannot acquire a fabricated historical timestamp"
            );
        }
        drop(writer);
        drop(source);
        drop(hub);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn full_normal_attempt_lane_keeps_all_voice_and_pedal_emergency_attempts_available() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    let mut initial: Vec<_> = (0..16)
        .map(|channel| {
            Input::Midi(clap_event_midi {
                header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
                port_index: 0,
                data: [0xb0 | channel, 64, 127],
            })
        })
        .collect();
    initial.extend((0..64).map(|id| note(id + 1, (id / 4) as i16, (60 + id % 4) as i16, 1, true)));
    assert_eq!(source.run(64, initial, None).values.len(), 80);
    hub.run(64, vec![], None);
    assert_eq!(session.credits.load(Ordering::Acquire), 64);
    assert!(source.source_snapshot().pedals_held);
    let events = (0..512).map(|index| expression(index % 64 + 1, 0.125, 0)).collect();
    let wire = source.run_select(128, events, None, Some(512));
    assert_eq!(
        wire.attempts, 624,
        "512 actual normal attempts plus64 voice and48 pedal emergency attempts"
    );
    assert_eq!(wire.values.len(), 623);
    assert_eq!(
        wire.values.iter().filter(|(_, event)| matches!(event, Event::Expression { .. })).count(),
        511
    );
    assert_eq!(wire.values.iter().filter(|(_, event)| event.release()).count(), 64);
    let resets: std::collections::BTreeSet<_> = wire
        .values
        .iter()
        .filter_map(|(_, event)| match event {
            Event::Midi { data: [status, controller, 0], .. }
                if [64, 66, 69].contains(controller) =>
            {
                Some((*status & 15, *controller))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        resets,
        (0..16).flat_map(|channel| [64, 66, 69].map(|controller| (channel, controller))).collect()
    );
    let snapshot = source.source_snapshot();
    assert_eq!(snapshot.faults, source::OUTPUT_FAULT);
    assert_eq!(snapshot.journal, 511);
    assert_eq!(
        snapshot.emergency, 112,
        "actual accepted emergency facts use their separate retained journal"
    );
    assert!(!snapshot.pedals_held);
    assert_eq!(
        session.credits.load(Ordering::Acquire),
        64,
        "accepted termination still awaits canonical retention"
    );
    // A genuinely malformed later host input raises a distinct recovery cause.
    // It cannot recreate the already accepted neutralization or consume more
    // reserved factual sequence numbers for the same settled channel debt.
    let malformed = Input::Midi(clap_event_midi {
        header: clap_event_header {
            size: std::mem::size_of::<clap_event_header>() as u32,
            ..header::<clap_event_midi>(CLAP_EVENT_MIDI, 0)
        },
        port_index: 0,
        data: [0xf8, 0, 0],
    });
    let extra = source.run_status(192, vec![malformed], None, None, 64, true);
    assert_eq!(extra.attempts, 0);
    assert_eq!(source.source_snapshot().emergency, 112);
    assert_eq!(source.source_snapshot().faults, source::OUTPUT_FAULT | source::INPUT_FAULT);
    assert_eq!(source.run(256, vec![], None).attempts, 0);
    assert_eq!(source.source_snapshot().emergency, 112);
    let mut remaining_resets = 0;
    for block in 2..=16 {
        hub.run(block * 64, vec![], None);
        remaining_resets += source.run((block + 3) * 64, vec![], None).values.len();
    }
    assert_eq!(remaining_resets, 0);
    assert_eq!(session.credits.load(Ordering::Acquire), 0, "{:?}", source.source_snapshot());
    assert_eq!(source.source_snapshot().emergency, 0);
}

#[test]
fn mixed_generation_wildcard_parent_retains_only_the_old_childs_acknowledgement_obligation() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    assert_eq!(source.run(64, vec![note(1, 0, 60, 0, true)], None).values.len(), 1);
    hub.run(64, vec![], None);
    let shared = source.shared();
    let setup::Routing::Source(mut next) = shared.value().routing else { unreachable!() };
    next.selected = Some(SavedUuid::default());
    shared.apply(setup::Routing::Source(next), false).unwrap();
    source.run(128, vec![], None);
    assert!(source
        .run(192, vec![note(2, 0, 64, 0, true), expression(-1, 0.234567890123, 1)], None)
        .values
        .is_empty());
    let captured = source.source_snapshot();
    assert_eq!((captured.pending, captured.references, captured.input_cut), (2, 2, 3));
    assert_eq!(
        (captured.obligations, captured.old_obligations),
        (3, 1),
        "one original wildcard captured both lease generations"
    );
    shared.apply(shared.value().routing, true).unwrap();
    let released = source.run(256, vec![], None);
    assert_eq!(released.values.iter().filter(|(_, event)| event.release()).count(), 1);
    assert!(released.values.iter().all(|(_, event)| event.attack().is_none()));
    assert!(source.run(320, vec![], None).values.is_empty());
    let waiting = source.source_snapshot();
    assert_eq!((waiting.pending, waiting.references, waiting.manifest), (1, 2, 1));
    assert_eq!((waiting.obligations,waiting.old_obligations),(1,1),"the new-generation child settled locally; only the original lease's disposition still owns the parent");
    assert_eq!(
        session.credits.load(Ordering::Acquire),
        1,
        "current empty state cannot acknowledge actual old output"
    );
    for block in 2..=12 {
        hub.run(block * 64, vec![], None);
        assert!(source
            .run((block + 4) * 64, vec![], None)
            .values
            .iter()
            .all(|(_, event)| event.attack().is_none()));
        source.main();
        hub.main();
    }
    let settled = source.source_snapshot();
    assert_eq!(
        (
            settled.pending,
            settled.references,
            settled.manifest,
            settled.obligations,
            settled.old_obligations
        ),
        (0, 0, 0, 0, 0)
    );
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
}

#[cfg(debug_assertions)]
#[test]
fn defensive_old_child_completion_cannot_consume_the_reused_parents_live_permit() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    let uuid = SavedUuid::default();
    let (mut hub, mut capture) = Device::recorded_hub();
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    capture.drain_canonical();
    assert_eq!(source.run(64, vec![note(1, 0, 60, 0, true)], None).values.len(), 1);
    hub.run(64, vec![], None);
    let wrapper = unsafe {
        &*((*source.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
    };
    wrapper.test_with_plugin(|plugin| plugin.source.as_ref().unwrap().test_start_replay());
    for (block, id) in [(2, 2), (3, 3)] {
        let wire = source.run(block * 64, vec![note(id, 0, 60, 0, true)], None);
        assert_eq!(wire.values.len(), 2);
        assert!(
            matches!(wire.values[0].1,Event::Note {kind:CLAP_EVENT_NOTE_CHOKE,id:old,..} if old==id-1)
        );
        assert!(
            matches!(wire.values[1].1,Event::Note {kind:CLAP_EVENT_NOTE_ON,id:new,..} if new==id)
        );
        assert_eq!((source.source_snapshot().pending, source.source_snapshot().references), (0, 0));
        hub.run(block * 64, vec![], None);
    }
    wrapper.test_with_plugin(|plugin| plugin.source.as_ref().unwrap().test_finish_replay());
    assert_eq!(source.run(256, vec![note(3, 0, 60, 0, false)], None).values.len(), 1);
    hub.run(256, vec![], None);
    source.run(320, vec![], None);
    hub.run(320, vec![], None);
    let records = capture.drain_canonical();
    let deltas: Vec<_> = records
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta) => Some(delta),
            _ => None,
        })
        .collect();
    assert_eq!(deltas.len(), 6);
    assert_eq!(
        deltas.iter().filter(|delta| matches!(delta.event.kind, NoteKind::On { .. })).count(),
        3
    );
    assert_eq!(deltas.iter().filter(|delta| matches!(delta.event.kind, NoteKind::Off)).count(), 3);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
}
