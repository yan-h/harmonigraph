//! Real exported factory, declared stereo main and auxiliary input, exact raw
//! hooks and scripted host acceptance. Runs independently of the tuning probe.
use clap_sys::ext::latency::{CLAP_EXT_LATENCY, clap_host_latency};
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
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

type GuiEdit = Box<dyn Fn(&[f32])>;

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
    gui: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    tasks_run: AtomicUsize,
    host_callbacks: AtomicUsize,
    plugin: AtomicUsize,
    host_cache: AtomicU64,
    pause_audio: AtomicBool,
    pause_begin: AtomicBool,
    audio_entered: AtomicBool,
    audio_resume: AtomicBool,
    pause_value: AtomicBool,
    value_entered: AtomicBool,
    value_resume: AtomicBool,
    observed: Mutex<Observed>,
    closed: AtomicBool,
    busy: AtomicBool,
    fence_on_push: AtomicBool,
    auto_emergency: bool,
    final_emergency: bool,
    process_error: bool,
    misuse: bool,
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
    faults: usize,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            script: vec![],
            gui: Mutex::new(None),
            tasks_run: AtomicUsize::new(0),
            host_callbacks: AtomicUsize::new(0),
            plugin: AtomicUsize::new(0),
            host_cache: AtomicU64::new(0),
            pause_audio: AtomicBool::new(false),
            pause_begin: AtomicBool::new(false),
            audio_entered: AtomicBool::new(false),
            audio_resume: AtomicBool::new(false),
            pause_value: AtomicBool::new(false),
            value_entered: AtomicBool::new(false),
            value_resume: AtomicBool::new(false),
            observed: Mutex::new(Observed {
                inputs: Vec::with_capacity(8000),
                configuration: Vec::with_capacity(5000),
                blocks: Vec::with_capacity(5000),
                completions: Vec::with_capacity(3000),
                summaries: Vec::with_capacity(100),
                callbacks: Vec::with_capacity(100),
                admissions: Vec::with_capacity(3000),
                applies: Vec::with_capacity(5000),
                ..Default::default()
            }),
            closed: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            fence_on_push: AtomicBool::new(false),
            auto_emergency: false,
            final_emergency: false,
            process_error: false,
            misuse: false,
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
    fn editor(&mut self, executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        *self.control.gui.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(Box::new(move || executor.execute_gui(())));
        None
    }
    fn task_executor(&mut self) -> nice_plug::plugin::TaskExecutor<Self> {
        let control = self.control.clone();
        Box::new(move |()| {
            control.tasks_run.fetch_add(1, Ordering::Relaxed);
        })
    }
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
    const CLAP_ID: &'static str = if C && P {
        "fixture.combined"
    } else if C {
        "fixture.configuration"
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
    fn clap_configuration_prepare(
        state: &nice_plug::plugin::PluginState,
    ) -> Result<ConfigurationEdit, SubmitError> {
        let Some(nice_plug::plugin::ParamValue::F32(value)) = state.params.get("axis") else {
            return Err(SubmitError::Invalid);
        };
        let mut edit = ConfigurationEdit::default();
        edit.values[0] = Some(*value);
        Ok(edit)
    }
    fn clap_configuration_fault(&mut self) {
        self.control.observed.lock().unwrap_or_else(|e| e.into_inner()).faults += 1;
    }
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
    fn clap_configuration_observe(&mut self, input: OwnedInput) {
        self.control.observed.lock().unwrap_or_else(|e| e.into_inner()).configuration.push(input);
    }
    fn clap_performance_begin(&mut self, callback: perf::Callback, _: &mut perf::Output<'_>) {
        self.callback += 1;
        self.control.observed.lock().unwrap_or_else(|e| e.into_inner()).callbacks.push(callback);
        if self.control.pause_begin.swap(false, Ordering::AcqRel) {
            self.control.audio_entered.store(true, Ordering::Release);
            while !self.control.audio_resume.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
        }
    }
    fn clap_performance_input(&mut self, input: OwnedInput) {
        self.control.observed.lock().unwrap_or_else(|e| e.into_inner()).inputs.push(input);
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
        if self.control.pause_audio.swap(false, Ordering::AcqRel) {
            self.control.audio_entered.store(true, Ordering::Release);
            while !self.control.audio_resume.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
        }
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
        if (0..group.event_count()).any(|index| match group.event(index) {
            Some(InputValue::Note { kind: CLAP_EVENT_NOTE_ON, .. }) => true,
            Some(InputValue::Midi { data: [status, _, velocity], .. }) => status & 0xf0 == 0x90 && velocity != 0,
            _ => false,
        })
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
nice_export_clap!(Fixture<true, true>, Fixture<false, true>, Fixture<false, false>, Fixture<true, false>);

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
    } else if unsafe { CStr::from_ptr(id) } == clap_sys::ext::params::CLAP_EXT_PARAMS {
        (&PARAMS as *const clap_sys::ext::params::clap_host_params).cast()
    } else {
        ptr::null()
    }
}
unsafe extern "C" fn host_callback(host: *const clap_host) {
    unsafe { &*((*host).host_data.cast::<Control>()) }
        .host_callbacks
        .fetch_add(1, Ordering::Relaxed);
}
unsafe extern "C" fn rescan(host: *const clap_host, _: u32) {
    use clap_sys::ext::params::*;
    let control = unsafe { &*((*host).host_data.cast::<Control>()) };
    let plugin = control.plugin.load(Ordering::Relaxed) as *const clap_plugin;
    let params = unsafe {
        &*(((*plugin).get_extension.unwrap())(plugin, CLAP_EXT_PARAMS.as_ptr())
            .cast::<clap_plugin_params>())
    };
    let mut info: clap_param_info = unsafe { std::mem::zeroed() };
    assert!(unsafe { (params.get_info.unwrap())(plugin, 0, &mut info) });
    let mut value = 0.0;
    assert!(unsafe { (params.get_value.unwrap())(plugin, info.id, &mut value) });
    control.host_cache.store(value.to_bits(), Ordering::Relaxed);
}
unsafe extern "C" fn clear(_: *const clap_host, _: u32, _: u32) {}
static PARAMS: clap_sys::ext::params::clap_host_params = clap_sys::ext::params::clap_host_params {
    rescan: Some(rescan),
    clear: Some(clear),
    request_flush: Some(request),
};
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
    if header.type_ == CLAP_EVENT_PARAM_VALUE {
        if sink.control.pause_value.swap(false, Ordering::AcqRel) {
            sink.control.value_entered.store(true, Ordering::Release);
            while !sink.control.value_resume.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
        }
        if accepted {
            sink.control.host_cache.store(
                unsafe { (*event.cast::<clap_event_param_value>()).value }.to_bits(),
                Ordering::Relaxed,
            );
        }
    }
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
        CLAP_EVENT_MIDI => {
            let e = unsafe { &*event.cast::<clap_event_midi>() };
            Some(InputValue::Midi { port: e.port_index, data: e.data, flags: header.flags })
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
    transport: Option<clap_event_transport>,
}
// Fixtures move only the serialized process call to a worker and return the
// device before main-thread lifecycle/destruction. The host/owned sink stay pinned.
unsafe impl Send for Device {}
impl Device {
    fn gui<const C: bool, const P: bool>(&self) -> impl Fn(&[f32]) + use<C, P> {
        let wrapper = unsafe {
            &*((*self.plugin)
                .plugin_data
                .cast::<nice_plug::wrapper::clap::Wrapper<Fixture<C, P>>>())
        };
        let (context, param) = wrapper.test_gui_context("axis");
        move |values| unsafe {
            context.raw_begin_set_parameter(param);
            for &value in values {
                context.raw_set_parameter_normalized(param, value);
            }
            context.raw_end_set_parameter(param);
        }
    }
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
            request_callback: Some(host_callback),
        });
        let factory = unsafe { (clap_entry.get_factory.unwrap())(CLAP_PLUGIN_FACTORY_ID.as_ptr()) }
            .cast::<clap_plugin_factory>();
        let plugin = unsafe { ((*factory).create_plugin.unwrap())(factory, &*host, id.as_ptr()) };
        CONSTRUCTION.lock().unwrap_or_else(|e| e.into_inner()).take();
        assert!(!plugin.is_null());
        control.plugin.store(plugin as usize, Ordering::Relaxed);
        assert!(unsafe { ((*plugin).init.unwrap())(plugin) });
        assert!(unsafe { ((*plugin).activate.unwrap())(plugin, 48000.0, 1, 64) });
        assert!(unsafe { ((*plugin).start_processing.unwrap())(plugin) });
        Self {
            plugin,
            _host: host,
            control: control.clone(),
            sink: Sink { control, script: vec![], attempts: Vec::with_capacity(5000) },
            transport: None,
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
            transport: self.transport.as_ref().map_or(ptr::null(), |t| t),
            audio_inputs: buffers.as_ptr(),
            audio_outputs: &mut out,
            audio_inputs_count: 2,
            audio_outputs_count: 1,
            in_events: &list,
            out_events: if output { &sink } else { ptr::null() },
        };
        unsafe { ((*self.plugin).process.unwrap())(self.plugin, &process) }
    }
    fn flush(&self, input: Vec<Input>) {
        use clap_sys::ext::params::*;
        let params = unsafe {
            &*(((*self.plugin).get_extension.unwrap())(self.plugin, CLAP_EXT_PARAMS.as_ptr())
                .cast::<clap_plugin_params>())
        };
        let list = clap_input_events {
            ctx: (&input as *const Vec<Input>) as *mut c_void,
            size: Some(input_size),
            get: Some(input_get),
        };
        unsafe {
            (params.flush.unwrap())(self.plugin, &list, ptr::null());
        }
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
    fn parameter_value(&self, id: u32) -> f64 {
        use clap_sys::ext::params::{CLAP_EXT_PARAMS, clap_plugin_params};
        let params = unsafe {
            &*(((*self.plugin).get_extension.unwrap())(self.plugin, CLAP_EXT_PARAMS.as_ptr())
                .cast::<clap_plugin_params>())
        };
        let mut value = 0.0;
        assert!(unsafe { (params.get_value.unwrap())(self.plugin, id, &mut value) });
        value
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
fn native_gui_admission_survives_missing_output_and_rejected_notification_retries() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for combined in [false, true] {
        let mut d = Device::new(
            Control::default(),
            if combined { c"fixture.combined" } else { c"fixture.performance" },
        );
        let gui: GuiEdit = if combined {
            Box::new(d.gui::<true, true>())
        } else {
            Box::new(d.gui::<false, true>())
        };
        gui(&[1.0]);
        d.run(0, 8, vec![], true);
        d.sink.attempts.clear();
        gui(&[0.0]);
        let Input::Param(mut host_on) = d.param(1) else { unreachable!() };
        host_on.value = 1.0;
        d.run(8, 8, vec![Input::Param(host_on), on(2)], false);
        assert!(d.sink.attempts.is_empty());
        assert_eq!(d.parameter_value(host_on.param_id), 1.0);
        // Repeated Begin refusal cannot hide the Off from local input or pin
        // the following host On/note. Then reject the old Set twice as well.
        d.sink.script = vec![false, false, true, false, false, true, true];
        for start in [16, 24, 32, 40, 48, 56] {
            d.run(start, 8, vec![], true);
            assert_eq!(d.parameter_value(host_on.param_id), 1.0);
        }
        assert_eq!(
            d.sink.attempts.iter().map(|a| (a.kind, a.accepted)).collect::<Vec<_>>(),
            vec![
                (CLAP_EVENT_PARAM_GESTURE_BEGIN, false),
                (CLAP_EVENT_PARAM_GESTURE_BEGIN, false),
                (CLAP_EVENT_PARAM_GESTURE_BEGIN, true),
                (CLAP_EVENT_PARAM_VALUE, false),
                (CLAP_EVENT_PARAM_VALUE, false),
                (CLAP_EVENT_PARAM_VALUE, true),
                (CLAP_EVENT_PARAM_GESTURE_END, true),
            ]
        );
        let observed = d.control.observed.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(observed.inputs.len(), 4);
        assert_eq!(
            observed.inputs[1].value,
            InputValue::Parameter { id: host_on.param_id, value: 0.0, modulation: false }
        );
        assert_eq!(observed.inputs[1].sample, Some(8));
        assert_eq!(
            observed.inputs[2].value,
            InputValue::Parameter { id: host_on.param_id, value: 1.0, modulation: false }
        );
        assert!(matches!(observed.inputs[3].value, InputValue::Note { .. }));
        if combined {
            assert_eq!(observed.configuration, observed.inputs);
        }
        drop(observed);
        drop(gui);
    }
}

#[test]
fn native_gui_waits_behind_full_input_and_gestures_spend_capture_budget() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = Device::new(Control::default(), c"fixture.performance");
    let gui = d.gui::<false, true>();
    d.run(0, 8, vec![on(0); INPUT_SCAN], false);
    gui(&[0.25]);
    // A host batch that fills the scan leaves no cell for a GUI entry, so the
    // change waits for a callback with room rather than displacing input.
    d.run(8, 8, vec![on(0); INPUT_SCAN], true);
    assert!(d.sink.attempts.is_empty());
    assert_eq!(
        d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).inputs.len(),
        2 * INPUT_SCAN
    );
    // Only one work/cell remains after reserving this host batch: Begin uses
    // that credit, so Set must wait even though Begin needs no payload cell.
    d.run(16, 8, vec![on(0); INPUT_SCAN - 1], true);
    assert_eq!(d.sink.attempts.len(), 1);
    assert_eq!(d.sink.attempts[0].kind, CLAP_EVENT_PARAM_GESTURE_BEGIN);
    assert_eq!(
        d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).inputs.len(),
        3 * INPUT_SCAN - 1
    );
    let Input::Transport(t) = transport(0, true, 42) else { unreachable!() };
    d.transport = Some(t);
    d.run(24, 8, vec![on(1)], true);
    let o = d.control.observed.lock().unwrap_or_else(|e| e.into_inner());
    let tail = &o.inputs[3 * INPUT_SCAN - 1..];
    assert_eq!(tail.len(), 3);
    assert!(matches!(tail[0].value, InputValue::Transport(_)));
    assert!(matches!(tail[1].value, InputValue::Parameter { value: 0.25, .. }));
    assert!(matches!(tail[2].value, InputValue::Note { .. }));
    assert_eq!(tail[1].sample, Some(24));
    drop(o);
    drop(gui);
}

#[test]
fn native_gui_flush_is_untimed_and_invalid_host_capture_does_not_commit_admission() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = Device::new(Control::default(), c"fixture.performance");
    let gui = d.gui::<false, true>();
    gui(&[0.25]);
    assert_eq!(
        d.run(
            0,
            8,
            vec![Input::Header(header::<clap_event_header>(CLAP_EVENT_MIDI_SYSEX, 0))],
            true
        ),
        CLAP_PROCESS_ERROR
    );
    assert!(d.sink.attempts.is_empty());
    d.flush(vec![d.param(57)]);
    assert!(d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).inputs.is_empty());
    let Input::Param(host) = d.param(1) else { unreachable!() };
    assert_eq!(d.parameter_value(host.param_id), 0.0);
    d.run(100, 8, vec![on(2)], true);
    let o = d.control.observed.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(o.inputs.len(), 3);
    assert!(matches!(o.inputs[0].value, InputValue::Parameter { value: 0.25, .. }));
    assert!(o.inputs[0].flush);
    assert_eq!(o.inputs[0].sample, Some(100));
    assert_eq!(o.inputs[0].enclosing_start, None);
    assert_eq!(o.inputs[0].enclosing_frames, 0);
    assert_eq!(o.inputs[0].offset, 0);
    assert_eq!(o.inputs[1].offset, 57);
    assert_eq!(d.parameter_value(host.param_id), 0.5);
    drop(o);
    d.run(108, 8, vec![], true);
    assert_eq!(d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).inputs.len(), 3);
    drop(gui);
}

#[test]
fn native_gui_arrival_after_snapshot_waits_through_configuration_subblocks_and_error_exit() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for mode in 0..3 {
        let early_error = mode == 2;
        let mut d = Device::new(
            Control { process_error: mode != 0, ..Default::default() },
            c"fixture.combined",
        );
        let gui = d.gui::<true, true>();
        let control = d.control.clone();
        if early_error {
            control.pause_begin.store(true, Ordering::Release);
        } else {
            control.pause_audio.store(true, Ordering::Release);
        }
        let input = if early_error {
            vec![Input::Header(header::<clap_event_header>(CLAP_EVENT_MIDI_SYSEX, 0))]
        } else {
            vec![d.param(2), transport(4, true, 99), on(6)]
        };
        let worker = std::thread::spawn(move || {
            assert_eq!(
                d.run(0, 8, input, true),
                if mode == 0 { CLAP_PROCESS_CONTINUE_IF_NOT_QUIET } else { CLAP_PROCESS_ERROR }
            );
            d
        });
        while !control.audio_entered.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        gui(&[0.25]);
        control.audio_resume.store(true, Ordering::Release);
        let mut d = worker.join().unwrap();
        let previous = {
            let o = control.observed.lock().unwrap_or_else(|e| e.into_inner());
            assert!(
                o.inputs
                    .iter()
                    .all(|i| !matches!(i.value, InputValue::Parameter { value: 0.25, .. }))
            );
            assert_eq!(o.configuration, o.inputs);
            if mode == 0 {
                assert_eq!(o.blocks.iter().map(|b| b.1).collect::<Vec<_>>(), vec![0, 2, 4]);
            }
            if mode == 1 {
                assert_eq!(o.blocks.iter().map(|b| b.1).collect::<Vec<_>>(), vec![0]);
            }
            o.inputs.len()
        };
        assert!(d.sink.attempts.is_empty());
        let Input::Transport(t) = transport(0, true, 100) else { unreachable!() };
        d.transport = Some(t);
        d.run(8, 8, vec![d.param(1), on(2)], true);
        let o = control.observed.lock().unwrap_or_else(|e| e.into_inner());
        let next = &o.inputs[previous..];
        assert_eq!(next.len(), 4);
        assert!(matches!(next[0].value, InputValue::Transport(_)));
        assert!(matches!(next[1].value, InputValue::Parameter { value: 0.25, .. }));
        assert_eq!(next[1].sample, Some(8));
        assert!(matches!(next[2].value, InputValue::Parameter { value: 0.5, .. }));
        assert!(matches!(next[3].value, InputValue::Note { .. }));
        assert_eq!(o.configuration, o.inputs);
        drop(o);
        d.run(16, 8, vec![], true);
        assert_eq!(
            control.observed.lock().unwrap_or_else(|e| e.into_inner()).inputs.len(),
            previous + 5
        );
        drop(gui);
    }
}

#[test]
fn native_gui_nonperformance_paths_keep_direct_parameter_delivery() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for configuration in [false, true] {
        let mut d = Device::new(
            Control::default(),
            if configuration { c"fixture.configuration" } else { c"fixture.legacy" },
        );
        let gui: GuiEdit = if configuration {
            Box::new(d.gui::<true, false>())
        } else {
            Box::new(d.gui::<false, false>())
        };
        gui(&[0.25]);
        d.run(0, 8, vec![], true);
        assert_eq!(
            d.sink.attempts.iter().map(|a| a.kind).collect::<Vec<_>>(),
            vec![
                CLAP_EVENT_PARAM_GESTURE_BEGIN,
                CLAP_EVENT_PARAM_VALUE,
                CLAP_EVENT_PARAM_GESTURE_END
            ]
        );
        assert_eq!(d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).legacy, 0);
        // Configuration-only keeps its existing configuration snapshot readback;
        // inspect the generic Param directly to prove the legacy output applies.
        if configuration {
            let w = unsafe {
                &*((*d.plugin)
                    .plugin_data
                    .cast::<nice_plug::wrapper::clap::Wrapper<Fixture<true, false>>>())
            };
            assert_eq!(w.test_inspect_plugin(|p| p.params.axis.value()), 0.25);
        } else {
            let Input::Param(p) = d.param(0) else { unreachable!() };
            assert_eq!(d.parameter_value(p.param_id), 0.25);
        }
        drop(gui);
    }
}

#[test]
fn native_gui_full_notification_bank_wraps_without_reapplying_its_admitted_prefix() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = Device::new(Control::default(), c"fixture.performance");
    let gui = d.gui::<false, true>();
    // Exactly 2,048 notification entries: Begin + 2,046 Sets + End.
    gui(&vec![0.25; INPUT_SCAN - 2]);
    d.run(0, 8, vec![], true);
    assert_eq!(d.sink.attempts.len(), 512);
    assert_eq!(
        d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).inputs.len(),
        INPUT_SCAN - 2
    );
    // Refill all 512 released cells across the physical wrap. The original
    // 1,536 notifications still await the host but have already entered input.
    gui(&vec![0.75; 510]);
    for start in [8, 16, 24, 32, 40] {
        d.run(start, 8, vec![], true);
    }
    let o = d.control.observed.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(o.inputs.len(), INPUT_SCAN - 2 + 510);
    assert!(
        o.inputs[..INPUT_SCAN - 2]
            .iter()
            .all(|i| matches!(i.value, InputValue::Parameter { value: 0.25, .. })
                && i.sample == Some(0))
    );
    assert!(
        o.inputs[INPUT_SCAN - 2..]
            .iter()
            .all(|i| matches!(i.value, InputValue::Parameter { value: 0.75, .. })
                && i.sample == Some(8))
    );
    assert_eq!(d.sink.attempts.len(), INPUT_SCAN + 512);
    drop(o);
    drop(gui);
}

#[test]
fn owned_input_has_exact_subblocks_transport_and_no_duplicate_consumer() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = Device::new(Control::default(), c"fixture.combined");
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
fn one_batch_scan_limit_includes_nonperformance_events() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d =
        Device::new(Control { final_emergency: true, ..Default::default() }, c"fixture.combined");
    let mut input = vec![on(0); INPUT_SCAN - 1];
    input[0] = d.param(0);
    input.push(transport(32, false, 100));
    assert_ne!(d.run(0, 64, input, true), CLAP_PROCESS_ERROR);
    // A parameter and a transport occupy the same scan as note input.
    assert_eq!(
        d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).configuration.len(),
        INPUT_SCAN
    );
    assert_eq!(d.run(64, 64, vec![on(0); INPUT_SCAN + 1], true), CLAP_PROCESS_ERROR);
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

#[test]
fn flush_loss_reaches_owner_and_next_process_boundary() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for id in [c"fixture.combined", c"fixture.configuration"] {
        let mut d = Device::new(Control::default(), id);
        d.flush(vec![d.param(57); INPUT_SCAN]);
        d.flush(vec![d.param(58)]);
        assert!(d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).faults > 0);
        assert_eq!(d.run(100, 64, vec![], true), CLAP_PROCESS_ERROR);
        if id == c"fixture.combined" {
            assert_eq!(
                d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).callbacks[0]
                    .input_status,
                perf::InputStatus::Full
            );
        }
    }
    for (input, expected) in [
        (vec![on(0); INPUT_SCAN + 1], perf::InputStatus::Full),
        (
            vec![Input::Header(header::<clap_event_header>(CLAP_EVENT_MIDI_SYSEX, 0))],
            perf::InputStatus::Unsupported,
        ),
    ] {
        let mut d = Device::new(Control::default(), c"fixture.performance");
        d.flush(input);
        assert_eq!(d.run(0, 64, vec![], true), CLAP_PROCESS_ERROR);
        assert_eq!(
            d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).callbacks[0].input_status,
            expected
        );
    }
}

#[test]
fn retained_flush_parameter_is_applied_before_error_recovery_acknowledges_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = Device::new(Control::default(), c"fixture.performance");
    let Input::Param(parameter) = d.param(57) else { unreachable!() };
    d.flush(vec![Input::Param(parameter)]);
    d.flush(vec![on(0); INPUT_SCAN + 1]);
    assert_eq!(d.run(100, 64, vec![], true), CLAP_PROCESS_ERROR);
    {
        let o = d.control.observed.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(o.inputs.len(), 1);
        let input = o.inputs[0];
        assert!(input.flush);
        assert_eq!(input.offset, 57);
        assert_eq!(input.sample, Some(100));
        assert_eq!(input.enclosing_start, None);
        assert_eq!(input.enclosing_frames, 0);
        assert_eq!(input.event_index, 0);
        assert_eq!(
            input.value,
            InputValue::Parameter { id: parameter.param_id, value: 0.5, modulation: false }
        );
    }
    assert_eq!(d.parameter_value(parameter.param_id), 0.5);
    assert_eq!(d.run(164, 64, vec![], true), CLAP_PROCESS_CONTINUE_IF_NOT_QUIET);
    assert_eq!(d.parameter_value(parameter.param_id), 0.5);
    assert_eq!(d.control.observed.lock().unwrap_or_else(|e| e.into_inner()).inputs.len(), 1);
}

#[test]
fn deferred_gui_producer_finishing_after_audio_still_wakes_host() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut d = Device::new(Control::default(), c"fixture.performance");
    let entered = Arc::new(AtomicBool::new(false));
    let resume = Arc::new(AtomicBool::new(false));
    let hook_entered = entered.clone();
    let hook_resume = resume.clone();
    let wrapper = unsafe {
        &*((*d.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<Fixture<false, true>>>())
    };
    wrapper.test_on_deferred_gui_observation(move || {
        hook_entered.store(true, Ordering::Release);
        while !hook_resume.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
    });
    d.control.pause_audio.store(true, Ordering::Release);
    d.control.host_callbacks.store(0, Ordering::Release);
    let control = d.control.clone();
    std::thread::scope(|scope| {
        let producer = scope.spawn(|| {
            while !control.audio_entered.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            (control.gui.lock().unwrap_or_else(|e| e.into_inner()).as_ref().unwrap())();
        });
        let release_audio = scope.spawn(|| {
            while !entered.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            control.audio_resume.store(true, Ordering::Release);
        });
        d.run(0, 64, vec![], true);
        resume.store(true, Ordering::Release);
        producer.join().unwrap();
        release_audio.join().unwrap();
    });
    assert!(control.host_callbacks.load(Ordering::Acquire) > 0);
    unsafe {
        ((*d.plugin).on_main_thread.unwrap())(d.plugin);
    }
    assert_eq!(control.tasks_run.load(Ordering::Relaxed), 1);
}

#[test]
fn final_error_drain_requests_rescan_after_racing_restore() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for invalid in [false, true] {
        let script = if invalid {
            instructions(
                (0..512).map(|n| single(n, perf::Lane::Normal, 0, note(CLAP_EVENT_NOTE_OFF))),
            )
        } else {
            vec![]
        };
        let mut d = Device::new(
            Control { process_error: !invalid, script, ..Default::default() },
            c"fixture.combined",
        );
        let mut edit = ConfigurationEdit::default();
        edit.values[0] = Some(0.1);
        d.mailbox().submit(edit).unwrap();
        if invalid {
            d.run(0, 64, vec![], true);
        }
        d.control.pause_value.store(true, Ordering::Release);
        let control = d.control.clone();
        let plugin_address = d.plugin as usize;
        d = std::thread::scope(|scope| {
            let audio = scope.spawn(move || {
                d.run(64, 64, if invalid { vec![on(64)] } else { vec![] }, true);
                d
            });
            while !control.value_entered.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            let plugin = plugin_address as *const clap_plugin;
            let wrapper = unsafe {
                &*((*plugin)
                    .plugin_data
                    .cast::<nice_plug::wrapper::clap::Wrapper<Fixture<true, true>>>())
            };
            let mut state = wrapper.get_state_object();
            state.params.insert("axis".to_owned(), nice_plug::plugin::ParamValue::F32(0.9));
            wrapper.set_state_object_from_gui(state);
            unsafe {
                ((*plugin).on_main_thread.unwrap())(plugin);
            }
            assert!(
                (f64::from_bits(control.host_cache.load(Ordering::Relaxed)) - 0.9).abs() < 1e-6
            );
            control.host_callbacks.store(0, Ordering::Release);
            control.value_resume.store(true, Ordering::Release);
            audio.join().unwrap()
        });
        assert!((f64::from_bits(control.host_cache.load(Ordering::Relaxed)) - 0.1).abs() < 1e-6);
        assert!(control.host_callbacks.load(Ordering::Acquire) > 0);
        unsafe {
            ((*d.plugin).on_main_thread.unwrap())(d.plugin);
        }
        assert!((f64::from_bits(control.host_cache.load(Ordering::Relaxed)) - 0.9).abs() < 1e-6);
    }
}
