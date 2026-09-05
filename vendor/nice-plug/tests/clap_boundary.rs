//! Real exported factory, declared stereo main and auxiliary input, exact raw
//! hooks and scripted host acceptance. Runs independently of the tuning probe.
use clap_sys::ext::latency::{CLAP_EXT_LATENCY, clap_host_latency, clap_plugin_latency};
use clap_sys::factory::plugin_factory::{CLAP_PLUGIN_FACTORY_ID, clap_plugin_factory};
use clap_sys::{
    audio_buffer::clap_audio_buffer, events::*, host::clap_host, plugin::clap_plugin, process::*,
    version::CLAP_VERSION,
};
use nice_plug::prelude::*;
use nice_plug::wrapper::clap::{ProcessTrace, configuration::*, performance as perf};
use std::{
    ffi::{CStr, c_char, c_void},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

static SERIAL: Mutex<()> = Mutex::new(());
static CONSTRUCTION: Mutex<Option<Arc<Control>>> = Mutex::new(None);
#[derive(Params)]
struct Parameters {
    #[id = "axis"]
    axis: FloatParam,
}
#[derive(Clone, Copy)]
struct Instruction {
    callback: usize,
    block: u32,
    group: perf::Group,
}
struct Control {
    script: Vec<Instruction>,
    observed: Mutex<Observed>,
    pending: AtomicBool,
    closed: AtomicBool,
    busy: AtomicBool,
    fence_on_push: AtomicBool,
    auto_emergency: bool,
    final_emergency: bool,
    process_error: bool,
    misuse: bool,
    learn_at: Option<i64>,
    apply_limit: AtomicUsize,
    restarts: AtomicUsize,
    latency_changes: AtomicUsize,
}
#[derive(Default)]
struct Observed {
    inputs: Vec<OwnedInput>,
    configuration: Vec<OwnedInput>,
    blocks: Vec<(i64, u32, u32)>,
    completions: Vec<perf::Completion>,
    summaries: Vec<perf::Summary>,
    callbacks: Vec<perf::Callback>,
    admissions: Vec<Result<(), perf::StageError>>,
    applies: Vec<i64>,
    legacy: usize,
    finals: usize,
    traces: usize,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            script: vec![],
            observed: Mutex::new(Observed {
                inputs: Vec::with_capacity(5000),
                configuration: Vec::with_capacity(5000),
                blocks: Vec::with_capacity(5000),
                completions: Vec::with_capacity(3000),
                summaries: Vec::with_capacity(100),
                callbacks: Vec::with_capacity(100),
                admissions: Vec::with_capacity(3000),
                applies: Vec::with_capacity(5000),
                ..Default::default()
            }),
            pending: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            fence_on_push: AtomicBool::new(false),
            auto_emergency: false,
            final_emergency: false,
            process_error: false,
            misuse: false,
            learn_at: None,
            apply_limit: AtomicUsize::new(usize::MAX),
            restarts: AtomicUsize::new(0),
            latency_changes: AtomicUsize::new(0),
        }
    }
}
struct Fixture<const CONFIG: bool, const PERFORMANCE: bool> {
    params: Arc<Parameters>,
    control: Arc<Control>,
    callback: usize,
    mailbox: Option<Arc<ConfigurationMailbox>>,
    learned: bool,
}
impl<const C: bool, const P: bool> Default for Fixture<C, P> {
    fn default() -> Self {
        Self {
            params: Arc::new(Parameters {
                axis: FloatParam::new("Axis", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            }),
            control: CONSTRUCTION
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
                .unwrap()
                .clone(),
            callback: 0,
            mailbox: None,
            learned: false,
        }
    }
}
impl<const C: bool, const P: bool> Plugin for Fixture<C, P> {
    const NAME: &'static str = "Boundary fixture";
    const VENDOR: &'static str = "fixture";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = "1";
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: Some(new_nonzero_u32(2)),
        main_output_channels: Some(new_nonzero_u32(2)),
        aux_input_ports: &[new_nonzero_u32(2)],
        ..AudioIOLayout::const_default()
    }];
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;
    type SysExMessage = ();
    type BackgroundTask = ();
    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }
    fn initialize(
        &mut self,
        _: &AudioIOLayout,
        _: &BufferConfig,
        context: &mut impl InitContext<Self>,
    ) -> bool {
        context.set_latency_samples(512);
        true
    }
    fn process(
        &mut self,
        _: &mut Buffer,
        _: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        while let Some(event) = context.next_event() {
            self.control.observed.lock().unwrap_or_else(|e| e.into_inner()).legacy += 1;
            context.send_event(event);
        }
        ProcessStatus::Normal
    }
}
impl<const C: bool, const P: bool> ClapPlugin for Fixture<C, P> {
    const CLAP_ID: &'static str = if C {
        "fixture.combined"
    } else if P {
        "fixture.performance"
    } else {
        "fixture.legacy"
    };
    const CLAP_DESCRIPTION: Option<&'static str> = None;
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::NoteEffect];
    const CLAP_CONFIGURATION: bool = C;
    const CLAP_CONFIGURATION_PARAMS: &'static [&'static str] = &["axis"];
    const CLAP_PERFORMANCE: bool = P;
    const CLAP_PROCESS_TRACE: bool = true;
    fn clap_configuration_install(&mut self, mailbox: Arc<ConfigurationMailbox>) {
        self.mailbox = Some(mailbox);
    }
    fn clap_configuration_apply(
        &mut self,
        _: ConfigurationCommand,
        commit: ConfigurationCommit,
    ) -> Option<ConfigurationSnapshot> {
        let remaining = self.control.apply_limit.load(Ordering::Relaxed);
        if remaining == 0 {
            return None;
        }
        self.control.apply_limit.store(remaining - 1, Ordering::Relaxed);
        self.control.observed.lock().unwrap_or_else(|e| e.into_inner()).applies.push(commit.sample);
        Some(ConfigurationSnapshot::default())
    }
    fn clap_configuration_group_end(&mut self, sample: i64) -> Option<ConfigurationEdit> {
        if !self.learned && self.control.learn_at == Some(sample) {
            self.learned = true;
            let mut edit = ConfigurationEdit::default();
            edit.values[0] = Some(0.25);
            Some(edit)
        } else {
            None
        }
    }
    fn clap_configuration_observe(&mut self, input: OwnedInput) {
        self.control.observed.lock().unwrap_or_else(|e| e.into_inner()).configuration.push(input);
    }
    fn clap_performance_begin(&mut self, callback: perf::Callback, _: &mut perf::Output<'_>) {
        self.callback += 1;
        self.control.observed.lock().unwrap_or_else(|e| e.into_inner()).callbacks.push(callback);
    }
    fn clap_performance_input(&mut self, input: OwnedInput) -> perf::Consumption {
        if self.control.pending.load(Ordering::Acquire) {
            return perf::Consumption::Pending;
        }
        self.control.observed.lock().unwrap_or_else(|e| e.into_inner()).inputs.push(input);
        perf::Consumption::Consumed
    }
    fn clap_performance_process(
        &mut self,
        b: &mut Buffer,
        a: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
        block: perf::Block,
        output: &mut perf::Output<'_>,
    ) -> ProcessStatus {
        self.control.observed.lock().unwrap_or_else(|e| e.into_inner()).blocks.push((
            block.callback.steady_time,
            block.start,
            block.frames,
        ));
        self.process(b, a, context);
        if self.control.misuse {
            context.send_event(NoteEvent::NoteOff {
                timing: 0,
                voice_id: None,
                channel: 0,
                note: 60,
                velocity: 0.0,
            });
        }
        for instruction in &self.control.script {
            if instruction.callback == self.callback && instruction.block == block.start {
                let result = output.stage(instruction.group);
                self.control
                    .observed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .admissions
                    .push(result);
            }
        }
        if self.control.process_error {
            ProcessStatus::Error("fixture")
        } else {
            ProcessStatus::Normal
        }
    }
    fn clap_performance_prepare(&mut self, group: perf::Group) -> bool {
        if matches!(group.event(0), Some(InputValue::Note { kind: CLAP_EVENT_NOTE_ON, .. }))
            && self.control.closed.load(Ordering::Acquire)
        {
            return false;
        }
        assert!(!self.control.busy.swap(true, Ordering::AcqRel));
        true
    }
    fn clap_performance_complete(
        &mut self,
        completion: perf::Completion,
        output: &mut perf::Output<'_>,
    ) {
        self.control
            .observed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .completions
            .push(completion);
        if self.control.auto_emergency
            && completion.accepted == 1
            && completion.group.event_count() == 2
        {
            output
                .stage(single(
                    999,
                    perf::Lane::Emergency,
                    output.cursor(),
                    note(CLAP_EVENT_NOTE_CHOKE),
                ))
                .unwrap();
        }
        self.control.busy.store(false, Ordering::Release);
    }
    fn clap_performance_finalize(
        &mut self,
        _: perf::Callback,
        _: clap_process_status,
        output: &mut perf::Output<'_>,
    ) {
        self.control.observed.lock().unwrap_or_else(|e| e.into_inner()).finals += 1;
        if self.control.final_emergency {
            output
                .stage(single(
                    998,
                    perf::Lane::Emergency,
                    output.cursor(),
                    note(CLAP_EVENT_NOTE_CHOKE),
                ))
                .unwrap();
        }
    }
    fn clap_performance_end(&mut self, _: perf::Callback, summary: perf::Summary) {
        assert!(!self.control.busy.load(Ordering::Acquire));
        self.control.observed.lock().unwrap_or_else(|e| e.into_inner()).summaries.push(summary);
    }
    fn clap_process_trace(&mut self, _: ProcessTrace<'_>) {
        self.control.observed.lock().unwrap_or_else(|e| e.into_inner()).traces += 1;
    }
}
nice_export_clap!(Fixture<true, true>, Fixture<false, true>, Fixture<false, false>);

fn note(kind: u16) -> InputValue {
    InputValue::Note {
        kind,
        note_id: 567,
        port: 0,
        channel: 2,
        key: 61,
        velocity: 0.765432198765,
        flags: CLAP_EVENT_IS_LIVE,
    }
}
fn tuning() -> InputValue {
    InputValue::Expression {
        expression: CLAP_NOTE_EXPRESSION_TUNING,
        note_id: 567,
        port: 0,
        channel: 2,
        key: 61,
        value: 0.123456789123,
        flags: CLAP_EVENT_DONT_RECORD,
    }
}
fn token(n: u64) -> perf::Token {
    perf::Token([n, 0, 0, 0])
}
fn single(n: u64, lane: perf::Lane, time: u32, value: InputValue) -> perf::Group {
    perf::Group::single(token(n), lane, time, value).unwrap()
}
fn pair(n: u64, time: u32) -> perf::Group {
    perf::Group::onset(token(n), time, note(CLAP_EVENT_NOTE_ON), tuning()).unwrap()
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
#[derive(Clone, Copy)]
enum Input {
    Note(clap_event_note),
    Expression(clap_event_note_expression),
    Midi(clap_event_midi),
    Transport(clap_event_transport),
    Param(clap_event_param_value),
    Header(clap_event_header),
}
impl Input {
    fn header(&self) -> &clap_event_header {
        match self {
            Self::Note(e) => &e.header,
            Self::Expression(e) => &e.header,
            Self::Midi(e) => &e.header,
            Self::Transport(e) => &e.header,
            Self::Param(e) => &e.header,
            Self::Header(e) => e,
        }
    }
}
fn on(time: u32) -> Input {
    Input::Note(clap_event_note {
        header: header::<clap_event_note>(CLAP_EVENT_NOTE_ON, time),
        note_id: 567,
        port_index: 0,
        channel: 2,
        key: 61,
        velocity: 0.765432198765,
    })
}
fn transport(time: u32, playing: bool, position: i64) -> Input {
    Input::Transport(clap_event_transport {
        header: header::<clap_event_transport>(CLAP_EVENT_TRANSPORT, time),
        flags: CLAP_TRANSPORT_HAS_SECONDS_TIMELINE
            | if playing { CLAP_TRANSPORT_IS_PLAYING } else { 0 },
        song_pos_seconds: position,
        tempo: 143.123456789,
        tempo_inc: 0.000001,
        ..unsafe { std::mem::zeroed() }
    })
}
unsafe extern "C" fn input_size(list: *const clap_input_events) -> u32 {
    unsafe { &*((*list).ctx.cast::<Vec<Input>>()) }.len() as u32
}
unsafe extern "C" fn input_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    (unsafe { &*((*list).ctx.cast::<Vec<Input>>()) })[index as usize].header()
}
unsafe extern "C" fn restart(host: *const clap_host) {
    unsafe { &*((*host).host_data.cast::<Control>()) }.restarts.fetch_add(1, Ordering::Relaxed);
}
unsafe extern "C" fn latency_changed(host: *const clap_host) {
    unsafe { &*((*host).host_data.cast::<Control>()) }
        .latency_changes
        .fetch_add(1, Ordering::Relaxed);
}
static LATENCY: clap_host_latency = clap_host_latency { changed: Some(latency_changed) };
unsafe extern "C" fn extension(_: *const clap_host, id: *const c_char) -> *const c_void {
    if unsafe { CStr::from_ptr(id) } == CLAP_EXT_LATENCY {
        (&LATENCY as *const clap_host_latency).cast()
    } else {
        ptr::null()
    }
}
unsafe extern "C" fn request(_: *const clap_host) {}
#[derive(Clone, Copy, Debug)]
struct Attempt {
    kind: u16,
    time: u32,
    accepted: bool,
    value: Option<InputValue>,
}
struct Sink {
    control: Arc<Control>,
    script: Vec<bool>,
    attempts: Vec<Attempt>,
}
unsafe extern "C" fn push(
    list: *const clap_output_events,
    event: *const clap_event_header,
) -> bool {
    let sink = unsafe { &mut *((*list).ctx.cast::<Sink>()) };
    let header = unsafe { &*event };
    let accepted = sink.script.get(sink.attempts.len()).copied().unwrap_or(true);
    let value = match header.type_ {
        CLAP_EVENT_NOTE_ON | CLAP_EVENT_NOTE_OFF | CLAP_EVENT_NOTE_CHOKE => {
            let e = unsafe { &*event.cast::<clap_event_note>() };
            Some(InputValue::Note {
                kind: header.type_,
                note_id: e.note_id,
                port: e.port_index,
                channel: e.channel,
                key: e.key,
                velocity: e.velocity,
                flags: header.flags,
            })
        }
        CLAP_EVENT_NOTE_EXPRESSION => {
            let e = unsafe { &*event.cast::<clap_event_note_expression>() };
            Some(InputValue::Expression {
                expression: e.expression_id,
                note_id: e.note_id,
                port: e.port_index,
                channel: e.channel,
                key: e.key,
                value: e.value,
                flags: header.flags,
            })
        }
        _ => None,
    };
    assert!(sink.attempts.len() < sink.attempts.capacity());
    sink.attempts.push(Attempt { kind: header.type_, time: header.time, accepted, value });
    if header.type_ == CLAP_EVENT_NOTE_ON
        && sink.control.fence_on_push.swap(false, Ordering::AcqRel)
    {
        assert!(sink.control.busy.load(Ordering::Acquire));
        sink.control.closed.store(true, Ordering::Release);
    }
    accepted
}
struct Device {
    plugin: *const clap_plugin,
    _host: Box<clap_host>,
    control: Arc<Control>,
    sink: Sink,
}
impl Device {
    fn new(control: Control, id: &CStr) -> Self {
        let control = Arc::new(control);
        *CONSTRUCTION.lock().unwrap_or_else(|e| e.into_inner()) = Some(control.clone());
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: Arc::as_ptr(&control) as *mut c_void,
            name: c"fixture".as_ptr(),
            vendor: c"fixture".as_ptr(),
            url: c"".as_ptr(),
            version: c"1".as_ptr(),
            get_extension: Some(extension),
            request_restart: Some(restart),
            request_process: Some(request),
            request_callback: Some(request),
        });
        let factory = unsafe { (clap_entry.get_factory.unwrap())(CLAP_PLUGIN_FACTORY_ID.as_ptr()) }
            .cast::<clap_plugin_factory>();
        let plugin = unsafe { ((*factory).create_plugin.unwrap())(factory, &*host, id.as_ptr()) };
        CONSTRUCTION.lock().unwrap_or_else(|e| e.into_inner()).take();
        assert!(!plugin.is_null());
        assert!(unsafe { ((*plugin).init.unwrap())(plugin) });
        assert!(unsafe { ((*plugin).activate.unwrap())(plugin, 48000.0, 1, 64) });
        assert!(unsafe { ((*plugin).start_processing.unwrap())(plugin) });
        Self {
            plugin,
            _host: host,
            control: control.clone(),
            sink: Sink { control, script: vec![], attempts: Vec::with_capacity(5000) },
        }
    }
    fn run(
        &mut self,
        start: i64,
        frames: u32,
        input: Vec<Input>,
        output: bool,
    ) -> clap_process_status {
        let list = clap_input_events {
            ctx: (&input as *const Vec<Input>) as *mut c_void,
            size: Some(input_size),
            get: Some(input_get),
        };
        let sink =
            clap_output_events { ctx: (&mut self.sink as *mut Sink).cast(), try_push: Some(push) };
        let mut audio = [[0.0f32; 64]; 6];
        let mut pointers: Vec<_> = audio.iter_mut().map(|c| c.as_mut_ptr()).collect();
        let buffers = [
            clap_audio_buffer {
                data32: pointers.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: 2,
                latency: 0,
                constant_mask: 0,
            },
            clap_audio_buffer {
                data32: unsafe { pointers.as_mut_ptr().add(2) },
                data64: ptr::null_mut(),
                channel_count: 2,
                latency: 0,
                constant_mask: 0,
            },
        ];
        let mut out = clap_audio_buffer {
            data32: unsafe { pointers.as_mut_ptr().add(4) },
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let process = clap_process {
            steady_time: start,
            frames_count: frames,
            transport: ptr::null(),
            audio_inputs: buffers.as_ptr(),
            audio_outputs: &mut out,
            audio_inputs_count: 2,
            audio_outputs_count: 1,
            in_events: &list,
            out_events: if output { &sink } else { ptr::null() },
        };
        unsafe { ((*self.plugin).process.unwrap())(self.plugin, &process) }
    }
    fn param(&self, time: u32) -> Input {
        use clap_sys::ext::params::{CLAP_EXT_PARAMS, clap_param_info, clap_plugin_params};
        let params = unsafe {
            &*(((*self.plugin).get_extension.unwrap())(self.plugin, CLAP_EXT_PARAMS.as_ptr())
                .cast::<clap_plugin_params>())
        };
        let mut info: clap_param_info = unsafe { std::mem::zeroed() };
        assert!(unsafe { (params.get_info.unwrap())(self.plugin, 0, &mut info) });
        Input::Param(clap_event_param_value {
            header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, time),
            param_id: info.id,
            cookie: ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value: 0.5,
        })
    }
    fn mailbox(&self) -> Arc<ConfigurationMailbox> {
        unsafe {
            &*((*self.plugin)
                .plugin_data
                .cast::<nice_plug::wrapper::clap::Wrapper<Fixture<true, true>>>())
        }
        .configuration_handle()
        .unwrap()
    }
}
impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            ((*self.plugin).stop_processing.unwrap())(self.plugin);
            ((*self.plugin).deactivate.unwrap())(self.plugin);
            ((*self.plugin).destroy.unwrap())(self.plugin);
        }
    }
}
fn instructions(groups: impl IntoIterator<Item = perf::Group>) -> Vec<Instruction> {
    groups.into_iter().map(|group| Instruction { callback: 1, block: 0, group }).collect()
}

#[test]
fn retained_input_has_exact_subblocks_transport_and_no_duplicate_consumer() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = Device::new(Control::default(), c"fixture.combined");
    d.control.pending.store(true, Ordering::Release);
    d.run(
        100,
        64,
        vec![
            on(0),
            transport(16, false, 123456),
            on(16),
            transport(32, true, 987654),
            on(32),
            on(63),
        ],
        true,
    );
    assert_eq!(d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).configuration.len(), 6);
    d.control.pending.store(false, Ordering::Release);
    d.run(164, 7, vec![on(0), on(6)], true);
    let o = d.control.observed.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(o.configuration.len(), 8);
    assert_eq!(o.inputs.len(), 8);
    assert_eq!(o.legacy, 0);
    assert_eq!(o.blocks, [(100, 0, 16), (100, 16, 16), (100, 32, 32), (164, 0, 7)]);
    assert_eq!(
        o.inputs.iter().map(|e| e.sample.unwrap()).collect::<Vec<_>>(),
        [100, 116, 116, 132, 132, 163, 164, 170]
    );
    let InputValue::Transport(t) = o.inputs[1].value else { panic!() };
    assert_eq!(t.song_pos_seconds, 123456);
    assert_eq!(t.tempo, 143.123456789);
    assert_eq!(o.inputs[1].enclosing_start, Some(100));
    assert_eq!(o.inputs[1].offset, 16);
}

#[test]
fn raw_signed_addresses_and_f64_are_preserved_without_hub_mailbox() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = Device::new(Control::default(), c"fixture.performance");
    let wrapper = unsafe {
        &*((*d.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<Fixture<false, true>>>())
    };
    assert!(wrapper.configuration_handle().is_none());
    d.run(
        0,
        8,
        vec![
            Input::Note(clap_event_note {
                header: header::<clap_event_note>(CLAP_EVENT_NOTE_OFF, 1),
                note_id: -1,
                port_index: -1,
                channel: -1,
                key: -1,
                velocity: 0.123456789123,
            }),
            Input::Expression(clap_event_note_expression {
                header: header::<clap_event_note_expression>(CLAP_EVENT_NOTE_EXPRESSION, 2),
                expression_id: CLAP_NOTE_EXPRESSION_TUNING,
                note_id: 9001,
                port_index: -1,
                channel: -1,
                key: -1,
                value: 0.123456789123,
            }),
            Input::Midi(clap_event_midi {
                header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 3),
                port_index: 0,
                data: [0xb0, 64, 127],
            }),
        ],
        true,
    );
    let o = d.control.observed.lock().unwrap_or_else(|e| e.into_inner());
    assert!(matches!(
        o.inputs[0].value,
        InputValue::Note {
            port: -1,
            channel: -1,
            key: -1,
            note_id: -1,
            velocity: 0.123456789123,
            ..
        }
    ));
    assert!(matches!(
        o.inputs[1].value,
        InputValue::Expression { note_id: 9001, value: 0.123456789123, .. }
    ));
    assert_eq!(o.legacy, 0);
}

#[test]
fn total_input_pool_and_scan_limits_include_nonperformance_events() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d =
        Device::new(Control { final_emergency: true, ..Default::default() }, c"fixture.combined");
    d.control.pending.store(true, Ordering::Release);
    let mut input = vec![on(0); INPUT_SCAN - 1];
    input[0] = d.param(0);
    input.push(transport(32, false, 100));
    assert_ne!(d.run(0, 64, input, true), CLAP_PROCESS_ERROR);
    assert_eq!(
        d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).configuration.len(),
        INPUT_SCAN
    );
    assert_eq!(d.run(64, 64, vec![on(0)], true), CLAP_PROCESS_ERROR);
    assert_eq!(
        d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).callbacks[1].input_status,
        perf::InputStatus::Full
    );
    assert_eq!(d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).finals, 2);
    assert_eq!(d.sink.attempts.len(), 2);
    let mut d = Device::new(
        Control { final_emergency: true, ..Default::default() },
        c"fixture.performance",
    );
    assert_eq!(d.run(0, 64, vec![on(0); INPUT_SCAN + 1], true), CLAP_PROCESS_ERROR);
    assert!(d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).inputs.is_empty());
    assert_eq!(d.sink.attempts.len(), 1);
}

#[test]
fn enclosing_output_budget_is_shared_across_subblocks_and_emergency() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut script =
        instructions((0..300).map(|n| single(n, perf::Lane::Normal, 0, note(CLAP_EVENT_NOTE_OFF))));
    script.extend((300..513).map(|n| Instruction {
        callback: 1,
        block: 32,
        group: single(n, perf::Lane::Normal, 32, note(CLAP_EVENT_NOTE_OFF)),
    }));
    script.extend((0..129).map(|n| Instruction {
        callback: 1,
        block: 32,
        group: single(1000 + n, perf::Lane::Emergency, 32, note(CLAP_EVENT_NOTE_CHOKE)),
    }));
    let mut d = Device::new(Control { script, ..Default::default() }, c"fixture.performance");
    d.run(0, 64, vec![transport(32, false, 0)], true);
    let o = d.control.observed.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(d.sink.attempts.len(), 640);
    assert_eq!(o.summaries[0].normal_attempts, 512);
    assert_eq!(o.summaries[0].emergency_attempts, 128);
    assert_eq!(o.admissions.iter().filter(|r| **r == Err(perf::StageError::Full)).count(), 2);
}

#[test]
fn onset_pair_reserves_two_credits_when_only_one_is_left() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut script =
        instructions((0..511).map(|n| single(n, perf::Lane::Normal, 0, note(CLAP_EVENT_NOTE_OFF))));
    script.extend(instructions([
        pair(600, 0),
        single(601, perf::Lane::Normal, 0, note(CLAP_EVENT_NOTE_OFF)),
    ]));
    let mut d = Device::new(Control { script, ..Default::default() }, c"fixture.performance");
    d.run(0, 64, vec![], true);
    let o = d.control.observed.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(o.admissions[511], Err(perf::StageError::Full));
    assert_eq!(o.admissions[512], Ok(()));
    assert_eq!(d.sink.attempts.len(), 512);
    assert_eq!(o.completions.len(), 512);
}

#[test]
fn rejected_onset_suppresses_tuning_and_dependent_normal_groups() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = Device::new(
        Control { script: instructions([pair(1, 5), pair(2, 6)]), ..Default::default() },
        c"fixture.performance",
    );
    d.sink.script = vec![false];
    d.run(0, 64, vec![], true);
    let o = d.control.observed.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(d.sink.attempts.len(), 1);
    assert_eq!(
        (o.completions[0].attempted, o.completions[0].accepted, o.completions[0].unattempted),
        (1, 0, 2)
    );
    assert_eq!(o.completions[1].disposition, perf::Disposition::Inhibited);
}

#[test]
fn partial_onset_reports_exact_prefix_and_emergency_at_legal_future_cursor() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = Device::new(
        Control { script: instructions([pair(1, 47)]), auto_emergency: true, ..Default::default() },
        c"fixture.performance",
    );
    d.sink.script = vec![true, false, true];
    d.run(0, 64, vec![], true);
    let o = d.control.observed.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        (o.completions[0].attempted, o.completions[0].accepted, o.completions[0].unattempted),
        (3, 1, 0)
    );
    assert_eq!(
        d.sink.attempts.iter().map(|a| (a.kind, a.time, a.accepted)).collect::<Vec<_>>(),
        [
            (CLAP_EVENT_NOTE_ON, 47, true),
            (CLAP_EVENT_NOTE_EXPRESSION, 47, false),
            (CLAP_EVENT_NOTE_CHOKE, 47, true)
        ]
    );
    assert_eq!(d.sink.attempts[0].value, Some(note(CLAP_EVENT_NOTE_ON)));
    assert_eq!(d.sink.attempts[1].value, Some(tuning()));
}

#[test]
fn fences_before_claim_and_between_host_calls_preserve_permit_truth() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = Device::new(
        Control { script: instructions([pair(1, 0)]), ..Default::default() },
        c"fixture.performance",
    );
    d.control.closed.store(true, Ordering::Release);
    d.run(0, 64, vec![], true);
    assert!(d.sink.attempts.is_empty());
    assert_eq!(
        d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).completions[0].disposition,
        perf::Disposition::Ineligible
    );
    let mut d = Device::new(
        Control { script: instructions([pair(1, 0), pair(2, 1)]), ..Default::default() },
        c"fixture.performance",
    );
    d.control.fence_on_push.store(true, Ordering::Release);
    d.run(0, 64, vec![], true);
    assert_eq!(d.sink.attempts.len(), 2);
    assert_eq!(
        d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).completions[0].accepted,
        3
    );
    assert_eq!(
        d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).completions[1].disposition,
        perf::Disposition::Ineligible
    );
}

#[test]
fn rejected_expression_and_release_remain_exact_retry_values() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let expression = single(1, perf::Lane::Normal, 5, tuning());
    let release = single(2, perf::Lane::Emergency, 5, note(CLAP_EVENT_NOTE_OFF));
    let mut script = instructions([expression, release]);
    script.push(Instruction {
        callback: 2,
        block: 0,
        group: single(3, perf::Lane::Emergency, 0, note(CLAP_EVENT_NOTE_OFF)),
    });
    let mut d = Device::new(Control { script, ..Default::default() }, c"fixture.performance");
    d.sink.script = vec![false, false, true];
    d.run(0, 64, vec![], true);
    d.run(64, 7, vec![], true);
    let o = d.control.observed.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(o.completions[0].accepted, 0);
    assert_eq!(o.completions[0].group, expression);
    assert_eq!(o.completions[1].accepted, 0);
    assert_eq!(o.completions[1].group, release);
    assert_eq!(o.completions[2].accepted, 1);
    assert_eq!(o.completions[2].group.event(0), release.event(0));
}

#[test]
fn all_exits_finalize_missing_output_invalid_input_and_process_error() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for (missing, invalid, error) in
        [(true, false, false), (false, true, false), (false, false, true)]
    {
        let mut d = Device::new(
            Control {
                script: instructions([pair(1, 0)]),
                final_emergency: true,
                process_error: error,
                ..Default::default()
            },
            c"fixture.performance",
        );
        d.run(0, 64, if invalid { vec![on(64)] } else { vec![] }, !missing);
        let o = d.control.observed.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(o.finals, 1);
        assert_eq!(o.summaries.len(), 1);
        assert!(!d.control.busy.load(Ordering::Acquire));
        if missing {
            assert_eq!(o.completions.len(), 2);
            assert!(
                o.completions.iter().all(|c| c.disposition == perf::Disposition::MissingOutput)
            );
        } else {
            assert_eq!(d.sink.attempts.last().unwrap().kind, CLAP_EVENT_NOTE_CHOKE);
        }
    }
}

#[test]
fn notifications_merge_in_time_and_retain_partial_gestures_at_shared_budget() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut script =
        instructions((0..511).map(|n| single(n, perf::Lane::Normal, 0, note(CLAP_EVENT_NOTE_OFF))));
    script.extend((0..128).map(|n| Instruction {
        callback: 1,
        block: 0,
        group: single(n + 1000, perf::Lane::Emergency, 31, note(CLAP_EVENT_NOTE_CHOKE)),
    }));
    let mut d = Device::new(Control { script, ..Default::default() }, c"fixture.combined");
    let mut edit = ConfigurationEdit::default();
    edit.values[0] = Some(0.75);
    d.mailbox().submit(edit).unwrap();
    d.run(0, 64, vec![], true);
    assert_eq!(d.sink.attempts.len(), 640);
    assert_eq!(d.sink.attempts[0].kind, CLAP_EVENT_PARAM_GESTURE_BEGIN);
    d.run(64, 7, vec![], true);
    assert_eq!(d.sink.attempts[640].kind, CLAP_EVENT_PARAM_VALUE);
    assert_eq!(d.sink.attempts[641].kind, CLAP_EVENT_PARAM_GESTURE_END);
}

#[test]
fn optout_legacy_and_activation_latency_remain_unchanged() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = Device::new(Control::default(), c"fixture.legacy");
    d.run(0, 64, vec![on(4)], true);
    assert_eq!(d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).legacy, 1);
    assert_eq!(d.sink.attempts[0].time, 4);
    assert!(d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).callbacks.is_empty());
    let latency = unsafe {
        &*(((*d.plugin).get_extension.unwrap())(d.plugin, CLAP_EXT_LATENCY.as_ptr())
            .cast::<clap_plugin_latency>())
    };
    assert_eq!(unsafe { (latency.get.unwrap())(d.plugin) }, 512);
    assert_eq!(d.control.restarts.load(Ordering::Relaxed), 0);
}

#[test]
fn unsupported_input_and_legacy_send_misuse_are_explicit() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = Device::new(Control { misuse: true, ..Default::default() }, c"fixture.performance");
    d.run(0, 64, vec![], true);
    assert!(
        d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).summaries[0]
            .legacy_send_misuse
    );
    assert!(d.sink.attempts.is_empty());
    d.run(64, 64, vec![Input::Header(header::<clap_event_header>(CLAP_EVENT_MIDI_SYSEX, 0))], true);
    assert_eq!(
        d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).callbacks[1].input_status,
        perf::InputStatus::Unsupported
    );
}

#[test]
fn allocated_boundary_layouts_fit_declared_budgets() {
    assert!(std::mem::size_of::<Option<OwnedInput>>() <= 192);
    assert_eq!(std::mem::align_of::<Option<OwnedInput>>(), 8);
    assert!(std::mem::size_of::<Option<perf::Group>>() <= 256);
    println!(
        "input={} output={} completion={} input_pool={} output_pool={}",
        std::mem::size_of::<Option<OwnedInput>>(),
        std::mem::size_of::<Option<perf::Group>>(),
        std::mem::size_of::<perf::Completion>(),
        INPUT_SCAN * std::mem::size_of::<Option<OwnedInput>>(),
        perf::OUTPUT_CELLS * std::mem::size_of::<Option<perf::Group>>()
    );
}

#[test]
fn future_configuration_notification_does_not_overtake_earlier_performance() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut script = instructions([single(1, perf::Lane::Normal, 0, note(CLAP_EVENT_NOTE_OFF))]);
    script.push(Instruction {
        callback: 1,
        block: 31,
        group: single(2, perf::Lane::Normal, 31, note(CLAP_EVENT_NOTE_OFF)),
    });
    let mut d = Device::new(
        Control { script, learn_at: Some(131), ..Default::default() },
        c"fixture.combined",
    );
    let param = d.param(31);
    d.run(100, 64, vec![on(0), param], true);
    assert_eq!(
        d.sink.attempts.iter().map(|a| (a.kind, a.time)).collect::<Vec<_>>(),
        [
            (CLAP_EVENT_NOTE_OFF, 0),
            (CLAP_EVENT_PARAM_GESTURE_BEGIN, 31),
            (CLAP_EVENT_PARAM_VALUE, 31),
            (CLAP_EVENT_PARAM_GESTURE_END, 31),
            (CLAP_EVENT_NOTE_OFF, 31)
        ]
    );
}

#[test]
fn configuration_backpressure_precedes_performance_ack_without_reobserving_learning() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d =
        Device::new(Control { learn_at: Some(110), ..Default::default() }, c"fixture.combined");
    d.control.apply_limit.store(1, Ordering::Release);
    let param = d.param(10);
    d.run(100, 64, vec![param, on(10)], true);
    assert_eq!(d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).configuration.len(), 2);
    assert!(d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).inputs.is_empty());
    d.control.apply_limit.store(100, Ordering::Release);
    d.run(164, 8, vec![on(0)], true);
    let o = d.control.observed.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(o.configuration.len(), 3);
    assert_eq!(o.inputs.len(), 3);
    assert_eq!(o.applies, [110, 110]);
}

#[test]
fn gesture_closing_debt_survives_rejection_before_new_begin() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = Device::new(Control::default(), c"fixture.combined");
    let mut edit = ConfigurationEdit::default();
    edit.values[0] = Some(0.75);
    d.mailbox().submit(edit).unwrap();
    d.sink.script = vec![true, true, false];
    d.run(0, 64, vec![], true);
    d.mailbox().submit(edit).unwrap();
    d.run(64, 8, vec![], true);
    assert_eq!(
        d.sink.attempts.iter().map(|a| a.kind).collect::<Vec<_>>(),
        [
            CLAP_EVENT_PARAM_GESTURE_BEGIN,
            CLAP_EVENT_PARAM_VALUE,
            CLAP_EVENT_PARAM_GESTURE_END,
            CLAP_EVENT_PARAM_GESTURE_END,
            CLAP_EVENT_PARAM_GESTURE_BEGIN,
            CLAP_EVENT_PARAM_VALUE,
            CLAP_EVENT_PARAM_GESTURE_END
        ]
    );
}

#[test]
fn reused_notification_cells_do_not_overtake_an_open_older_gesture() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let script =
        instructions((0..505).map(|n| single(n, perf::Lane::Normal, 0, note(CLAP_EVENT_NOTE_OFF))));
    let mut d = Device::new(Control { script, ..Default::default() }, c"fixture.combined");
    for value in [0.1, 0.2, 0.3, 0.4] {
        let mut edit = ConfigurationEdit::default();
        edit.values[0] = Some(value);
        d.mailbox().submit(edit).unwrap();
    }
    d.control.apply_limit.store(3, Ordering::Release);
    d.run(0, 64, vec![], true);
    assert_eq!(d.sink.attempts.len(), 512);
    assert_eq!(d.sink.attempts[6].kind, CLAP_EVENT_PARAM_GESTURE_BEGIN);
    d.control.apply_limit.store(10, Ordering::Release);
    d.run(64, 8, vec![], true);
    assert_eq!(d.sink.attempts[512].kind, CLAP_EVENT_PARAM_VALUE);
    assert_eq!(d.sink.attempts[513].kind, CLAP_EVENT_PARAM_GESTURE_END);
    assert_eq!(d.sink.attempts[514].kind, CLAP_EVENT_PARAM_GESTURE_BEGIN);
}
