//! The shell's process callback, flush and editor writes, through the real
//! exported factory with a fixture plugin that records what it is handed.
use clap_sys::factory::plugin_factory::{CLAP_PLUGIN_FACTORY_ID, clap_plugin_factory};
use clap_sys::{
    audio_buffer::clap_audio_buffer, events::*, ext::gui::*, ext::params::*, host::clap_host,
    plugin::clap_plugin, process::*, version::CLAP_VERSION,
};
use harmonigraph_clap::prelude::*;
use harmonigraph_clap::wrapper::clap::setup::{Prepared, Serviced, Setup};
use std::{
    ffi::{CStr, c_char, c_void},
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicPtr, AtomicUsize, Ordering},
    },
};

static SERIAL: Mutex<()> = Mutex::new(());
static OBSERVED: Mutex<Observed> = Mutex::new(Observed::new());
/// `Setup::notice` calls.
static NOTICES: AtomicUsize = AtomicUsize::new(0);

/// What the fixture saw of its callbacks, in order.
struct Observed {
    /// Each `begin`'s boundary flags and the table's value there.
    begins: Vec<(bool, bool, f32)>,
    /// Each `process`'s events and the table's value there.
    events: Vec<Input>,
    processed: Vec<f32>,
    /// The host's answer to each note the fixture pushed.
    pushes: Vec<bool>,
    /// Whether each `process` had audio.
    audio: Vec<bool>,
}
impl Observed {
    const fn new() -> Self {
        Self {
            begins: Vec::new(),
            events: Vec::new(),
            processed: Vec::new(),
            pushes: Vec::new(),
            audio: Vec::new(),
        }
    }
    /// Room for every test's callbacks, so the guarded callback never
    /// allocates.
    fn reset(&mut self) {
        *self = Self {
            begins: Vec::with_capacity(64),
            events: Vec::with_capacity(64),
            processed: Vec::with_capacity(64),
            pushes: Vec::with_capacity(64),
            audio: Vec::with_capacity(64),
        };
    }
}

fn observed() -> std::sync::MutexGuard<'static, Observed> {
    OBSERVED.lock().unwrap_or_else(|e| e.into_inner())
}

struct Fixture {
    params: Arc<Table>,
    /// The editor's side of the host, as the shell gives it.
    host: Option<Arc<dyn EditorHost>>,
}
impl Default for Fixture {
    fn default() -> Self {
        Self {
            params: Arc::new(Table::new([Spec::float("axis", "Axis", 0.0, 0.0, 1.0)])),
            host: None,
        }
    }
}

/// The setup's one saved field: opaque to the shell, which only frames it.
static FIELD: Mutex<String> = Mutex::new(String::new());

struct FixtureSetup;
struct Field(Option<String>);
impl Prepared for Field {
    fn commit(self: Box<Self>) {
        if let Some(field) = self.0 {
            *FIELD.lock().unwrap_or_else(|e| e.into_inner()) = field;
        }
    }
}
impl Setup for FixtureSetup {
    fn prepare(&self, state: &PluginState) -> Result<Box<dyn Prepared>, &'static str> {
        Ok(Box::new(Field(state.fields.get("field").cloned())))
    }
    fn save(&self, state: &mut PluginState) {
        let field = FIELD.lock().unwrap_or_else(|e| e.into_inner()).clone();
        state.fields.insert("field".into(), field);
    }
    fn install_wakeup(&self, _: Box<dyn Fn() + Send + Sync>) {}
    fn service(&self) -> Serviced {
        Serviced::default()
    }
    fn notice(&self, _: &str) {
        NOTICES.fetch_add(1, Ordering::Relaxed);
    }
}

impl Plugin for Fixture {
    const NAME: &'static str = "Boundary fixture";
    const VENDOR: &'static str = "fixture";
    const URL: &'static str = "";
    const VERSION: &'static str = "1";
    const CLAP_ID: &'static str = "fixture.boundary";
    const CLAP_DESCRIPTION: Option<&'static str> = None;
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::NoteEffect];
    const PORTS: Ports = Ports { main_input: 2, main_output: 2, aux_inputs: &[] };
    const NOTE_PORTS: bool = true;

    fn params(&self) -> Arc<Table> {
        self.params.clone()
    }
    fn setup(&self) -> Option<Arc<dyn Setup>> {
        Some(Arc::new(FixtureSetup))
    }
    fn editor_host(&mut self, host: Arc<dyn EditorHost>) {
        self.host = Some(host);
    }
    fn editor(&mut self) -> Option<Box<dyn Editor>> {
        let host = self.host.clone()?;
        Some(Box::new(FixtureEditor { host, size: Mutex::new(OPENED) }))
    }
    fn begin(&mut self, boundary: &Boundary) {
        observed().begins.push((boundary.valid, boundary.input_lost, self.params.float(0)));
    }
    fn process(&mut self, block: &mut Block<'_>) {
        let mut observed = observed();
        observed.events.extend(block.events());
        observed.processed.push(self.params.float(0));
        observed.audio.push(block.audio.is_some());
        let note = InputValue::Note {
            kind: CLAP_EVENT_NOTE_ON,
            note_id: 1,
            port: 0,
            channel: 0,
            key: 60,
            velocity: 1.0,
            flags: 0,
        };
        let pushed = block.output.push(note, 0);
        observed.pushes.push(pushed);
    }
}
export_clap!(Fixture);

/// The fixture editor's size before it opens, and the size it asks the host
/// for as it opens.
const OPENED: (u32, u32) = (800, 600);
const RESIZED: (u32, u32) = (900, 650);

/// An editor that, as it opens, asks the host for [`RESIZED`], as a fold in
/// Harmonigraph's asks from its first frame, and takes the size when the
/// host agrees.
struct FixtureEditor {
    host: Arc<dyn EditorHost>,
    size: Mutex<(u32, u32)>,
}
impl FixtureEditor {
    fn size_lock(&self) -> std::sync::MutexGuard<'_, (u32, u32)> {
        self.size.lock().unwrap_or_else(|e| e.into_inner())
    }
}
impl Editor for FixtureEditor {
    /// # Safety
    ///
    /// None needed: the fixture never touches `parent`.
    unsafe fn spawn(&self, _parent: *mut c_void) -> Box<dyn std::any::Any + Send> {
        // No lock held across the host's answer, which may re-enter.
        if self.host.request_resize(RESIZED) {
            *self.size_lock() = RESIZED;
        }
        Box::new(())
    }
    fn size(&self) -> (u32, u32) {
        *self.size_lock()
    }
    fn resizable(&self) -> bool {
        true
    }
    fn set_size(&self, width: u32, height: u32) -> bool {
        *self.size_lock() = (width, height);
        true
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
#[derive(Clone, Copy)]
enum Event {
    Note(clap_event_note),
    Param(clap_event_param_value),
    Transport(clap_event_transport),
    SysEx(clap_event_midi_sysex),
    Header(clap_event_header),
}
impl Event {
    /// A pointer to the whole event, as a host hands one over: a pointer
    /// to only its header field could not read the rest (Miri).
    fn header(&self) -> *const clap_event_header {
        match self {
            Self::Note(e) => (e as *const clap_event_note).cast(),
            Self::Param(e) => (e as *const clap_event_param_value).cast(),
            Self::Transport(e) => (e as *const clap_event_transport).cast(),
            Self::SysEx(e) => (e as *const clap_event_midi_sysex).cast(),
            Self::Header(e) => e,
        }
    }
}
fn note(kind: u16, time: u32) -> Event {
    Event::Note(clap_event_note {
        header: header::<clap_event_note>(kind, time),
        note_id: 7,
        port_index: 0,
        channel: 0,
        key: 64,
        velocity: 0.5,
    })
}
fn param(value: f64, time: u32) -> Event {
    Event::Param(clap_event_param_value {
        header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, time),
        param_id: harmonigraph_clap::wrapper::hash_param_id("axis"),
        cookie: ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value,
    })
}
fn transport(time: u32) -> clap_event_transport {
    clap_event_transport {
        header: header::<clap_event_transport>(CLAP_EVENT_TRANSPORT, time),
        flags: CLAP_TRANSPORT_IS_PLAYING,
        ..unsafe { std::mem::zeroed() }
    }
}
unsafe extern "C" fn input_size(list: *const clap_input_events) -> u32 {
    unsafe { &*((*list).ctx.cast::<Vec<Event>>()) }.len() as u32
}
unsafe extern "C" fn input_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    (unsafe { &*((*list).ctx.cast::<Vec<Event>>()) })[index as usize].header()
}
fn list(events: &Vec<Event>) -> clap_input_events {
    clap_input_events {
        ctx: (events as *const Vec<Event>).cast_mut().cast(),
        size: Some(input_size),
        get: Some(input_get),
    }
}

#[derive(Default)]
struct Host {
    callbacks: AtomicUsize,
    /// The plugin, for a host callback that calls back into it.
    plugin: AtomicPtr<clap_plugin>,
    /// The last `request_resize`.
    resize: Mutex<Option<Resize>>,
}
unsafe extern "C" fn extension(_: *const clap_host, id: *const c_char) -> *const c_void {
    if unsafe { CStr::from_ptr(id) } == CLAP_EXT_PARAMS {
        (&PARAMS as *const clap_host_params).cast()
    } else if unsafe { CStr::from_ptr(id) } == CLAP_EXT_GUI {
        (&GUI as *const clap_host_gui).cast()
    } else {
        ptr::null()
    }
}
unsafe extern "C" fn host_callback(host: *const clap_host) {
    unsafe { &*((*host).host_data.cast::<Host>()) }.callbacks.fetch_add(1, Ordering::Relaxed);
}
unsafe extern "C" fn rescan(_: *const clap_host, _: u32) {}
unsafe extern "C" fn clear(_: *const clap_host, _: u32, _: u32) {}
unsafe extern "C" fn request(_: *const clap_host) {}
static PARAMS: clap_host_params =
    clap_host_params { rescan: Some(rescan), clear: Some(clear), request_flush: Some(request) };

/// A `request_resize` as the host heard it: the size asked for, and the
/// plugin's answers to `gui.get_size` and `gui.set_size(asked)` made from
/// inside it, as a host that resizes at once does.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Resize {
    asked: (u32, u32),
    reported: (u32, u32),
    set: bool,
}
unsafe extern "C" fn request_resize(host: *const clap_host, width: u32, height: u32) -> bool {
    let stats = unsafe { &*((*host).host_data.cast::<Host>()) };
    let plugin = stats.plugin.load(Ordering::Acquire).cast_const();
    let gui = unsafe { plugin_gui(plugin) };
    let mut reported = (0, 0);
    assert!(unsafe { (gui.get_size.unwrap())(plugin, &mut reported.0, &mut reported.1) });
    let set = unsafe { (gui.set_size.unwrap())(plugin, width, height) };
    let resize = Resize { asked: (width, height), reported, set };
    *stats.resize.lock().unwrap_or_else(|e| e.into_inner()) = Some(resize);
    true
}
unsafe extern "C" fn gui_noop(_: *const clap_host) {}
unsafe extern "C" fn gui_refuse(_: *const clap_host) -> bool {
    false
}
unsafe extern "C" fn gui_closed(_: *const clap_host, _: bool) {}
static GUI: clap_host_gui = clap_host_gui {
    resize_hints_changed: Some(gui_noop),
    request_resize: Some(request_resize),
    request_show: Some(gui_refuse),
    request_hide: Some(gui_refuse),
    closed: Some(gui_closed),
};
/// The plugin's gui extension.
///
/// # Safety
///
/// `plugin` is a live plugin with an editor.
unsafe fn plugin_gui(plugin: *const clap_plugin) -> &'static clap_plugin_gui {
    unsafe {
        &*((*plugin).get_extension.unwrap())(plugin, CLAP_EXT_GUI.as_ptr())
            .cast::<clap_plugin_gui>()
    }
}

/// What the host's output list was offered, as (type, time, value), and
/// whether it took each.
struct Sink {
    /// The answer to each offer in turn; past its end, yes.
    script: Vec<bool>,
    offers: Vec<(u16, u32, f64, bool)>,
}
unsafe extern "C" fn push(
    list: *const clap_output_events,
    event: *const clap_event_header,
) -> bool {
    let sink = unsafe { &mut *((*list).ctx.cast::<Sink>()) };
    let header = unsafe { &*event };
    let accepted = sink.script.get(sink.offers.len()).copied().unwrap_or(true);
    let value = match header.type_ {
        CLAP_EVENT_PARAM_VALUE => unsafe { (*event.cast::<clap_event_param_value>()).value },
        _ => f64::NAN,
    };
    assert!(sink.offers.len() < sink.offers.capacity());
    sink.offers.push((header.type_, header.time, value, accepted));
    accepted
}

/// The host's structs are raw pointers the plugin holds on to, freed at the
/// end: a `Box` moved after the plugin took its pointer would end that
/// pointer's reach (Miri).
struct Device {
    plugin: *const clap_plugin,
    host: *mut Host,
    clap_host: *mut clap_host,
    sink: Sink,
}
impl Device {
    fn new() -> Self {
        observed().reset();
        let host = Box::into_raw(Box::<Host>::default());
        let clap_host = Box::into_raw(Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: host.cast(),
            name: c"fixture".as_ptr(),
            vendor: c"fixture".as_ptr(),
            url: c"".as_ptr(),
            version: c"1".as_ptr(),
            get_extension: Some(extension),
            request_restart: Some(request),
            request_process: Some(request),
            request_callback: Some(host_callback),
        }));
        let factory = unsafe { (clap_entry.get_factory.unwrap())(CLAP_PLUGIN_FACTORY_ID.as_ptr()) }
            .cast::<clap_plugin_factory>();
        let plugin = unsafe {
            ((*factory).create_plugin.unwrap())(factory, clap_host, c"fixture.boundary".as_ptr())
        };
        assert!(!plugin.is_null());
        unsafe { &*host }.plugin.store(plugin.cast_mut(), Ordering::Release);
        assert!(unsafe { ((*plugin).init.unwrap())(plugin) });
        assert!(unsafe { ((*plugin).activate.unwrap())(plugin, 48000.0, 1, 64) });
        assert!(unsafe { ((*plugin).start_processing.unwrap())(plugin) });
        Self {
            plugin,
            host,
            clap_host,
            sink: Sink { script: vec![], offers: Vec::with_capacity(64) },
        }
    }
    fn run(
        &mut self,
        steady: i64,
        events: Vec<Event>,
        transport: Option<clap_event_transport>,
    ) -> clap_process_status {
        let input = list(&events);
        let output =
            clap_output_events { ctx: (&mut self.sink as *mut Sink).cast(), try_push: Some(push) };
        let mut audio = [[0.0f32; 64]; 4];
        let mut pointers: Vec<_> = audio.iter_mut().map(|c| c.as_mut_ptr()).collect();
        let inputs = clap_audio_buffer {
            data32: pointers.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let mut outputs =
            clap_audio_buffer { data32: unsafe { pointers.as_mut_ptr().add(2) }, ..inputs };
        let process = clap_process {
            steady_time: steady,
            frames_count: 64,
            transport: transport.as_ref().map_or(ptr::null(), |t| t),
            audio_inputs: &inputs,
            audio_outputs: &mut outputs,
            audio_inputs_count: 1,
            audio_outputs_count: 1,
            in_events: &input,
            out_events: &output,
        };
        unsafe { ((*self.plugin).process.unwrap())(self.plugin, &process) }
    }
    fn params(&self) -> &clap_plugin_params {
        unsafe {
            &*(((*self.plugin).get_extension.unwrap())(self.plugin, CLAP_EXT_PARAMS.as_ptr())
                .cast())
        }
    }
    fn value(&self) -> f64 {
        let mut value = f64::NAN;
        let id = harmonigraph_clap::wrapper::hash_param_id("axis");
        assert!(unsafe { (self.params().get_value.unwrap())(self.plugin, id, &mut value) });
        value
    }
    fn flush(&self, events: Vec<Event>) {
        let input = list(&events);
        unsafe { (self.params().flush.unwrap())(self.plugin, &input, ptr::null()) };
    }
    /// A flush whose output is the device's sink.
    fn flush_to_sink(&mut self, events: Vec<Event>) {
        let input = list(&events);
        let output =
            clap_output_events { ctx: (&mut self.sink as *mut Sink).cast(), try_push: Some(push) };
        unsafe { (self.params().flush.unwrap())(self.plugin, &input, &output) };
    }
    /// Load `json` through the CLAP state extension, framed as a save is.
    fn load(&self, json: &[u8]) {
        use clap_sys::ext::state::{CLAP_EXT_STATE, clap_plugin_state};
        use clap_sys::stream::clap_istream;
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
        let framed = [&(json.len() as u64).to_le_bytes()[..], json].concat();
        let mut unread = &framed[..];
        let stream = clap_istream { ctx: (&mut unread as *mut &[u8]).cast(), read: Some(read) };
        let state = unsafe {
            &*(((*self.plugin).get_extension.unwrap())(self.plugin, CLAP_EXT_STATE.as_ptr())
                .cast::<clap_plugin_state>())
        };
        assert!(unsafe { (state.load.unwrap())(self.plugin, &stream) });
    }
    /// What the host's list was offered since the last call, in order, each
    /// gesture or note by name and each value by its number.
    fn told(&mut self) -> Vec<String> {
        // Cleared rather than taken: the sink keeps its room for the next
        // guarded callback.
        let told = self
            .sink
            .offers
            .iter()
            .map(|&(kind, _, value, _)| match kind {
                CLAP_EVENT_PARAM_GESTURE_BEGIN => "begin".to_owned(),
                CLAP_EVENT_PARAM_GESTURE_END => "end".to_owned(),
                CLAP_EVENT_PARAM_VALUE => format!("{value}"),
                _ => "note".to_owned(),
            })
            .collect();
        self.sink.offers.clear();
        told
    }
    /// The editor's side of the host, and the fixture's one parameter's row.
    fn editor(&self) -> (Arc<dyn EditorHost>, usize) {
        let wrapper = unsafe {
            &*((*self.plugin)
                .plugin_data
                .cast::<harmonigraph_clap::wrapper::clap::Wrapper<Fixture>>())
        };
        let (host, table) =
            wrapper.test_inspect_plugin(|fixture| (fixture.host.clone(), fixture.params.clone()));
        let row = (0..table.len()).find(|&row| table.spec(row).id == "axis").expect("a row");
        (host.expect("a host"), row)
    }
    fn main(&self) {
        unsafe { ((*self.plugin).on_main_thread.unwrap())(self.plugin) };
    }
    fn tail(&self) -> u32 {
        use clap_sys::ext::tail::{CLAP_EXT_TAIL, clap_plugin_tail};
        unsafe {
            let tail = ((*self.plugin).get_extension.unwrap())(self.plugin, CLAP_EXT_TAIL.as_ptr())
                .cast::<clap_plugin_tail>();
            ((*tail).get.unwrap())(self.plugin)
        }
    }
}
impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            ((*self.plugin).stop_processing.unwrap())(self.plugin);
            ((*self.plugin).deactivate.unwrap())(self.plugin);
            ((*self.plugin).destroy.unwrap())(self.plugin);
            drop(Box::from_raw(self.clap_host));
            drop(Box::from_raw(self.host));
        }
    }
}

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// No tail before a processed callback; an infinite one after, since the
/// plugin keeps the host running; a lost callback changes neither, and a new
/// processing run starts with none.
#[test]
fn the_tail_is_infinite_once_a_callback_is_processed() {
    let _serial = serial();
    let mut d = Device::new();
    assert_eq!(d.tail(), 0);
    assert_eq!(d.run(0, vec![note(CLAP_EVENT_NOTE_ON, 64)], None), CLAP_PROCESS_ERROR);
    assert_eq!(d.tail(), 0, "a lost callback is not processed");
    assert_eq!(d.run(64, vec![], None), CLAP_PROCESS_CONTINUE);
    assert_eq!(d.tail(), u32::MAX);
    assert_eq!(d.run(128, vec![note(CLAP_EVENT_NOTE_ON, 64)], None), CLAP_PROCESS_ERROR);
    assert_eq!(d.tail(), u32::MAX);
    unsafe {
        ((*d.plugin).reset.unwrap())(d.plugin);
        assert_eq!(d.tail(), u32::MAX);
        ((*d.plugin).stop_processing.unwrap())(d.plugin);
        assert_eq!(d.tail(), u32::MAX);
        assert!(((*d.plugin).start_processing.unwrap())(d.plugin));
    }
    assert_eq!(d.tail(), 0);
}

/// `begin` reads the table as the callback found it, and `process` reads it
/// with the callback's automation applied (#1814's ordering rule).
#[test]
fn begin_reads_the_table_before_the_callbacks_automation() {
    let _serial = serial();
    let mut d = Device::new();
    assert_eq!(d.run(0, vec![param(0.75, 10)], None), CLAP_PROCESS_CONTINUE);
    let observed = observed();
    assert_eq!(observed.begins, [(true, false, 0.0)]);
    assert_eq!(observed.processed, [0.75]);
    assert_eq!(observed.audio, [true]);
    assert_eq!(observed.events.len(), 1, "the automation reaches the plugin as an event too");
    assert_eq!(observed.events[0].sample, 10);
}

/// SysEx, an event outside the core space and a per-note parameter value are
/// skipped; the callback and the Note-Off after them are not (#1814 Q3).
#[test]
fn an_event_the_plugin_does_not_read_is_skipped() {
    let _serial = serial();
    let mut d = Device::new();
    let sysex = Event::SysEx(clap_event_midi_sysex {
        header: header::<clap_event_midi_sysex>(CLAP_EVENT_MIDI_SYSEX, 1),
        port_index: 0,
        buffer: ptr::null(),
        size: 0,
    });
    let foreign =
        Event::Header(clap_event_header { space_id: 9, ..header::<clap_event_header>(0, 2) });
    let Event::Param(mut per_note) = param(0.75, 3) else { unreachable!() };
    per_note.note_id = 7;
    let events = vec![
        note(CLAP_EVENT_NOTE_ON, 0),
        sysex,
        foreign,
        Event::Param(per_note),
        note(CLAP_EVENT_NOTE_OFF, 4),
    ];
    assert_eq!(d.run(0, events, None), CLAP_PROCESS_CONTINUE);
    let observed = observed();
    let kinds: Vec<_> = observed
        .events
        .iter()
        .map(|input| match input.value {
            InputValue::Note { kind, .. } => (kind, input.sample),
            other => panic!("{other:?} should have been skipped"),
        })
        .collect();
    assert_eq!(kinds, [(CLAP_EVENT_NOTE_ON, 0), (CLAP_EVENT_NOTE_OFF, 4)]);
    assert_eq!(observed.processed, [0.0], "the per-note value reached no row");
}

/// A list out of time order loses the callback's input whole: nothing reaches
/// the table or the plugin, which is told, and every push it makes fails.
#[test]
fn an_invalid_list_loses_the_callbacks_input() {
    let _serial = serial();
    let mut d = Device::new();
    let events = vec![note(CLAP_EVENT_NOTE_ON, 5), param(0.5, 3)];
    assert_eq!(d.run(0, events, Some(transport(0))), CLAP_PROCESS_ERROR);
    assert_eq!(d.value(), 0.0);
    let observed = observed();
    assert_eq!(observed.begins, [(true, true, 0.0)]);
    assert!(observed.events.is_empty(), "not even the callback's transport");
    assert_eq!(observed.pushes, [false]);
    assert_eq!(observed.audio, [false], "no audio to record or analyze");
    assert!(d.sink.offers.is_empty());
}

/// A flush's value is the table's before the flush returns (#1814 Q1); a flush
/// whose list fails validation applies nothing and loses the next callback's
/// input, and only that one.
#[test]
fn a_flush_applies_at_once_and_a_failed_one_loses_the_next_input() {
    let _serial = serial();
    let mut d = Device::new();
    d.flush(vec![param(0.5, 0)]);
    assert_eq!(d.value(), 0.5, "read straight after the flush");
    let Event::Param(mut short) = param(0.25, 0) else { unreachable!() };
    short.header.size = 8;
    d.flush(vec![Event::Param(short)]);
    assert_eq!(d.value(), 0.5);
    assert_eq!(d.run(0, vec![note(CLAP_EVENT_NOTE_ON, 0)], None), CLAP_PROCESS_ERROR);
    assert_eq!(d.run(64, vec![note(CLAP_EVENT_NOTE_ON, 0)], None), CLAP_PROCESS_CONTINUE);
    let observed = observed();
    assert_eq!(observed.begins, [(true, true, 0.5), (true, false, 0.5)]);
    assert_eq!(observed.events.len(), 1, "the second callback's note only");
}

/// An editor edit is the table's at once (#1814 Q1), and the host hears it at
/// the next callback at offset 0, gesture and value, ahead of the plugin's
/// own output. A refusal stops the report, which the next callback repeats.
#[test]
fn an_editor_edit_applies_at_once_and_reaches_the_host_at_offset_zero() {
    let _serial = serial();
    let mut d = Device::new();
    let (context, row) = d.editor();
    let edit = |value| {
        context.begin_set_parameter(row);
        context.set_parameter(row, value);
        context.end_set_parameter(row);
    };
    // (type, time, whether the host took it), and the values offered.
    let offers = |sink: &mut Sink| -> (Vec<(u16, u32, bool)>, Vec<f64>) {
        // Cleared rather than taken: the sink keeps its room for the next
        // guarded callback.
        let offers = sink.offers.clone();
        sink.offers.clear();
        let values = offers.iter().map(|o| o.2).filter(|v| !v.is_nan()).collect();
        (offers.into_iter().map(|(kind, time, _, taken)| (kind, time, taken)).collect(), values)
    };
    let reported = [
        (CLAP_EVENT_PARAM_GESTURE_BEGIN, 0, true),
        (CLAP_EVENT_PARAM_VALUE, 0, true),
        (CLAP_EVENT_PARAM_GESTURE_END, 0, true),
        (CLAP_EVENT_NOTE_ON, 0, true),
    ];
    edit(0.25);
    assert_eq!(d.value(), 0.25, "read straight after the edit");
    d.run(0, vec![], None);
    assert_eq!(offers(&mut d.sink), (reported.to_vec(), vec![0.25]), "the plugin's output after");

    edit(0.5);
    d.sink.script = vec![false];
    d.run(64, vec![], None);
    let refused = vec![(CLAP_EVENT_PARAM_GESTURE_BEGIN, 0, false), (CLAP_EVENT_NOTE_ON, 0, true)];
    assert_eq!(offers(&mut d.sink), (refused, vec![]));
    d.sink.script = vec![];
    d.run(128, vec![], None);
    assert_eq!(offers(&mut d.sink), (reported.to_vec(), vec![0.5]), "told again");
    drop(context);
}

/// A transport event in the host's list reaches the plugin at its sample,
/// after the callback's own transport, and asks the main thread to say so
/// once per instance.
#[test]
fn a_transport_event_in_the_list_reaches_the_plugin_and_is_noticed_once() {
    let _serial = serial();
    NOTICES.store(0, Ordering::Relaxed);
    let mut d = Device::new();
    let events = vec![Event::Transport(transport(5))];
    assert_eq!(d.run(64, events.clone(), Some(transport(0))), CLAP_PROCESS_CONTINUE);
    let samples: Vec<_> = observed()
        .events
        .iter()
        .map(|input| {
            assert!(matches!(input.value, InputValue::Transport(_)));
            input.sample
        })
        .collect();
    assert_eq!(samples, [64, 69]);
    let callbacks = unsafe { &(*d.host).callbacks };
    assert_eq!(callbacks.load(Ordering::Relaxed), 1, "the main thread is asked");
    d.main();
    assert_eq!(NOTICES.load(Ordering::Relaxed), 1);
    d.run(128, events, Some(transport(0)));
    d.main();
    assert_eq!(NOTICES.load(Ordering::Relaxed), 1, "once");
}

/// The bytes a save writes and a load reads, those the shell owns: an 8-byte
/// little-endian length, then the version, each parameter's unmodulated value
/// under its id, and each field the setup writes as a JSON string the shell
/// does not read. Saved projects hold exactly these, so a change is a change
/// to every project.
#[test]
fn a_save_is_framed_json_of_the_version_the_parameters_and_the_fields() {
    use clap_sys::ext::state::{CLAP_EXT_STATE, clap_plugin_state};
    use clap_sys::stream::{clap_istream, clap_ostream};
    unsafe extern "C" fn write(stream: *const clap_ostream, data: *const c_void, size: u64) -> i64 {
        let bytes = unsafe { &mut *((*stream).ctx.cast::<Vec<u8>>()) };
        bytes.extend_from_slice(unsafe { std::slice::from_raw_parts(data.cast(), size as usize) });
        size as i64
    }
    unsafe extern "C" fn read(stream: *const clap_istream, data: *mut c_void, size: u64) -> i64 {
        let bytes = unsafe { &mut *((*stream).ctx.cast::<&[u8]>()) };
        let count = bytes.len().min(size as usize);
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast(), count) };
        *bytes = &bytes[count..];
        count as i64
    }
    let _serial = serial();
    let d = Device::new();
    let state = unsafe {
        &*(((*d.plugin).get_extension.unwrap())(d.plugin, CLAP_EXT_STATE.as_ptr())
            .cast::<clap_plugin_state>())
    };
    *FIELD.lock().unwrap() = r#"{"opaque":[1,2]}"#.into();
    d.flush(vec![param(0.5, 0)]);
    let json =
        br#"{"version":"1","params":{"axis":{"f32":0.5}},"fields":{"field":"{\"opaque\":[1,2]}"}}"#;
    let mut saved = Vec::new();
    let stream = clap_ostream { ctx: (&mut saved as *mut Vec<u8>).cast(), write: Some(write) };
    assert!(unsafe { (state.save.unwrap())(d.plugin, &stream) });
    assert_eq!(saved[..8], (json.len() as u64).to_le_bytes());
    assert_eq!(std::str::from_utf8(&saved[8..]).unwrap(), std::str::from_utf8(json).unwrap());

    let json = br#"{"version":"0","params":{"axis":{"f32":0.25}},"fields":{"field":"x"}}"#;
    let framed = [&(json.len() as u64).to_le_bytes()[..], json].concat();
    let mut unread = &framed[..];
    let stream = clap_istream { ctx: (&mut unread as *mut &[u8]).cast(), read: Some(read) };
    assert!(unsafe { (state.load.unwrap())(d.plugin, &stream) });
    assert_eq!(d.value(), 0.25);
    assert_eq!(*FIELD.lock().unwrap(), "x", "the field reached the setup as it was saved");
}

/// A gesture the editor holds open across callbacks is told once, and ends
/// when the editor ends it.
#[test]
fn a_gesture_held_open_across_callbacks_ends_when_the_editor_ends_it() {
    let _serial = serial();
    let mut d = Device::new();
    let (context, row) = d.editor();
    context.begin_set_parameter(row);
    context.set_parameter(row, 0.25);
    d.run(0, vec![], None);
    assert_eq!(d.told(), ["begin", "0.25", "note"]);
    d.run(64, vec![], None);
    assert_eq!(d.told(), ["note"], "still open");
    context.end_set_parameter(row);
    d.run(128, vec![], None);
    assert_eq!(d.told(), ["end", "note"]);
    drop(context);
}

/// Host automation of the row an editor edit is pending on takes effect in
/// callback order, and the host is told the value the editor set, not its
/// own (Yan, 2026-11-18): here the first callback's report is refused, so
/// the automation writes the row before the report the host takes.
#[test]
fn the_editor_value_is_told_though_automation_wrote_the_row() {
    let _serial = serial();
    let mut d = Device::new();
    let (context, row) = d.editor();
    context.begin_set_parameter(row);
    context.set_parameter(row, 0.25);
    context.end_set_parameter(row);
    d.sink.script = vec![false];
    d.run(0, vec![param(0.75, 3)], None);
    assert_eq!(d.told(), ["begin", "note"], "refused");
    assert_eq!(d.value(), 0.75);
    assert_eq!(observed().processed, [0.75]);
    d.sink.script = vec![];
    d.run(64, vec![], None);
    assert_eq!(d.told(), ["begin", "0.25", "end", "note"]);
    drop(context);
}

/// A flush tells the host what the editor did, the editor's value though the
/// flush's own value wrote the row, and the next callback does not tell it
/// again.
#[test]
fn a_flush_tells_the_editor_value_though_its_own_wrote_the_row() {
    let _serial = serial();
    let mut d = Device::new();
    let (context, row) = d.editor();
    context.begin_set_parameter(row);
    context.set_parameter(row, 0.25);
    context.end_set_parameter(row);
    d.flush_to_sink(vec![param(0.75, 0)]);
    assert_eq!(d.told(), ["begin", "0.25", "end"]);
    assert_eq!(d.value(), 0.75);
    d.run(0, vec![], None);
    assert_eq!(d.told(), ["note"]);
    drop(context);
}

/// A restore drops an edit the host was not told: the host reads the
/// restored state by rescanning (Yan, 2026-11-18).
#[test]
fn a_restore_drops_an_edit_the_host_was_not_told() {
    let _serial = serial();
    let mut d = Device::new();
    let (context, row) = d.editor();
    context.begin_set_parameter(row);
    context.set_parameter(row, 0.25);
    context.end_set_parameter(row);
    d.load(br#"{"version":"1","params":{"axis":{"f32":0.5}},"fields":{}}"#);
    d.run(0, vec![], None);
    assert_eq!(d.told(), ["note"]);
    assert_eq!(d.value(), 0.5);
    drop(context);
}

/// The editor's open path, through `gui.set_parent`: a resize the editor
/// asks for as it opens reaches the host at the size asked, and the host may
/// call the plugin's `gui.get_size` and `gui.set_size` from inside that
/// request, as Bitwig echoes a resize, without the shell holding a lock they
/// wait on. The editor then has the size.
#[test]
fn a_resize_asked_as_the_editor_opens_reaches_the_host_which_may_reenter() {
    let _serial = serial();
    let d = Device::new();
    let gui = unsafe { plugin_gui(d.plugin) };
    let (mut api, mut floating) = (ptr::null(), false);
    unsafe {
        assert!((gui.get_preferred_api.unwrap())(d.plugin, &mut api, &mut floating));
        assert!((gui.create.unwrap())(d.plugin, api, false));
        // The fixture's editor never touches the view.
        let window = clap_window { api, specific: clap_window_handle { cocoa: ptr::null_mut() } };
        assert!((gui.set_parent.unwrap())(d.plugin, &window));
    }
    let resize = *unsafe { &*d.host }.resize.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(resize, Some(Resize { asked: RESIZED, reported: OPENED, set: true }));
    let mut size = (0, 0);
    assert!(unsafe { (gui.get_size.unwrap())(d.plugin, &mut size.0, &mut size.1) });
    assert_eq!(size, RESIZED, "the editor's size after the round trip");
    unsafe { (gui.destroy.unwrap())(d.plugin) };
}
