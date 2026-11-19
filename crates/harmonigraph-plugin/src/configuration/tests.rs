//! The exported Harmonigraph CLAP factory, real host state and params vtables,
//! and real structured UI bridge. No editor is opened and no probe is enabled.
use super::*;
use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::events::*;
use clap_sys::ext::gui::{clap_host_gui, clap_plugin_gui, CLAP_EXT_GUI};
use clap_sys::ext::params::{clap_host_params, clap_plugin_params, CLAP_EXT_PARAMS};
use clap_sys::ext::state::{clap_host_state, clap_plugin_state, CLAP_EXT_STATE};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::*;
use clap_sys::stream::{clap_istream, clap_ostream};
use clap_sys::version::CLAP_VERSION;
use harmonigraph_clap::plugin::ParamValue;
use harmonigraph_core::configuration::TuningAxis;
use harmonigraph_core::tuning::microcents;
use harmonigraph_core::SourceId;
use harmonigraph_ui::params::ParamKey;
use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

mod host_surface;

#[derive(Default)]
struct Host {
    dirty: AtomicUsize,
    callbacks: AtomicUsize,
    /// `params.rescan` calls, and the flags of the last.
    rescans: AtomicUsize,
    rescan_flags: std::sync::atomic::AtomicU32,
    /// `gui.closed` calls, and whether the last asked for destruction.
    closed: AtomicUsize,
    closed_destroyed: AtomicBool,
    /// The sizes `gui.request_resize` asked for, and whether it grants them.
    resizes: std::sync::Mutex<Vec<(u32, u32)>>,
    grants_resize: AtomicBool,
}
unsafe extern "C" fn extension(_: *const clap_host, id: *const c_char) -> *const c_void {
    let id = unsafe { CStr::from_ptr(id) };
    if id == CLAP_EXT_PARAMS {
        &PARAMS as *const _ as *const c_void
    } else if id == CLAP_EXT_STATE {
        &STATE as *const _ as *const c_void
    } else if id == CLAP_EXT_GUI {
        &GUI as *const _ as *const c_void
    } else {
        ptr::null()
    }
}
unsafe extern "C" fn dirty(host: *const clap_host) {
    unsafe { &*((*host).host_data.cast::<Host>()) }.dirty.fetch_add(1, Ordering::Relaxed);
}
unsafe extern "C" fn request(_: *const clap_host) {}
unsafe extern "C" fn callback(host: *const clap_host) {
    unsafe { &*((*host).host_data.cast::<Host>()) }.callbacks.fetch_add(1, Ordering::Relaxed);
}
unsafe extern "C" fn rescan(host: *const clap_host, flags: u32) {
    let stats = unsafe { &*((*host).host_data.cast::<Host>()) };
    stats.rescans.fetch_add(1, Ordering::Relaxed);
    stats.rescan_flags.store(flags, Ordering::Relaxed);
}
unsafe extern "C" fn clear(_: *const clap_host, _: u32, _: u32) {}
static PARAMS: clap_host_params =
    clap_host_params { rescan: Some(rescan), clear: Some(clear), request_flush: Some(request) };
static STATE: clap_host_state = clap_host_state { mark_dirty: Some(dirty) };
unsafe extern "C" fn hints(_: *const clap_host) {}
unsafe extern "C" fn resize(host: *const clap_host, width: u32, height: u32) -> bool {
    let stats = unsafe { &*((*host).host_data.cast::<Host>()) };
    stats.resizes.lock().unwrap().push((width, height));
    stats.grants_resize.load(Ordering::Relaxed)
}
unsafe extern "C" fn show_hide(_: *const clap_host) -> bool {
    false
}
unsafe extern "C" fn closed(host: *const clap_host, was_destroyed: bool) {
    let stats = unsafe { &*((*host).host_data.cast::<Host>()) };
    stats.closed.fetch_add(1, Ordering::Relaxed);
    stats.closed_destroyed.store(was_destroyed, Ordering::Relaxed);
}
static GUI: clap_host_gui = clap_host_gui {
    resize_hints_changed: Some(hints),
    request_resize: Some(resize),
    request_show: Some(show_hide),
    request_hide: Some(show_hide),
    closed: Some(closed),
};

#[derive(Clone, Copy)]
enum Input {
    Param(clap_event_param_value),
    Note(clap_event_note),
    Tuning(clap_event_note_expression),
    Mod(clap_event_param_mod),
}
impl Input {
    fn header(&self) -> &clap_event_header {
        match self {
            Self::Param(e) => &e.header,
            Self::Mod(e) => &e.header,
            Self::Note(e) => &e.header,
            Self::Tuning(e) => &e.header,
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
unsafe extern "C" fn input_size(input: *const clap_input_events) -> u32 {
    unsafe { &*((*input).ctx.cast::<Vec<Input>>()) }.len() as u32
}
unsafe extern "C" fn input_get(
    input: *const clap_input_events,
    i: u32,
) -> *const clap_event_header {
    (unsafe { &*((*input).ctx.cast::<Vec<Input>>()) })[i as usize].header()
}
#[derive(Default)]
struct Sink {
    reject: bool,
    reject_kind: Option<u16>,
    attempts: Vec<(u16, u32, u32, bool)>,
}
unsafe extern "C" fn output_push(
    output: *const clap_output_events,
    event: *const clap_event_header,
) -> bool {
    let sink = unsafe { &mut *((*output).ctx.cast::<Sink>()) };
    let header = unsafe { &*event };
    let id = match header.type_ {
        CLAP_EVENT_PARAM_VALUE => unsafe { (*event.cast::<clap_event_param_value>()).param_id },
        CLAP_EVENT_PARAM_GESTURE_BEGIN | CLAP_EVENT_PARAM_GESTURE_END => unsafe {
            (*event.cast::<clap_event_param_gesture>()).param_id
        },
        _ => 0,
    };
    let accepted = !sink.reject && sink.reject_kind != Some(header.type_);
    assert!(sink.attempts.len() < sink.attempts.capacity());
    sink.attempts.push((header.type_, header.time, id, accepted));
    accepted
}
struct Device {
    plugin: *const clap_plugin,
    _host: Box<clap_host>,
    stats: Box<Host>,
    active: bool,
    /// The auxiliary input every block carries: silence unless a fixture
    /// needs it told apart from the main input.
    sidechain: std::cell::Cell<[[f32; 64]; 2]>,
}
impl Device {
    fn new() -> Self {
        let mut stats = Box::<Host>::default();
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: (&mut *stats as *mut Host).cast(),
            name: c"Configuration fixture".as_ptr(),
            vendor: c"test".as_ptr(),
            url: c"".as_ptr(),
            version: c"1".as_ptr(),
            get_extension: Some(extension),
            request_restart: Some(request),
            request_process: Some(request),
            request_callback: Some(callback),
        });
        let factory =
            unsafe { (crate::clap_entry.get_factory.unwrap())(CLAP_PLUGIN_FACTORY_ID.as_ptr()) }
                .cast::<clap_plugin_factory>();
        let plugin = unsafe {
            ((*factory).create_plugin.unwrap())(factory, &*host, c"com.yan-h.harmonigraph".as_ptr())
        };
        assert!(!plugin.is_null());
        assert!(unsafe { ((*plugin).init.unwrap())(plugin) });
        let device = Self {
            plugin,
            _host: host,
            stats,
            active: false,
            sidechain: std::cell::Cell::new([[0.0; 64]; 2]),
        };
        // Retune defaults off for the Hub's own input; these fixtures are
        // about the configuration/policy pipeline the correction runs
        // through, so give it the state that pipeline needs.
        device.wrapper().test_inspect_plugin(|plugin| {
            plugin.aggregation.as_ref().unwrap().shared.set_retune(true)
        });
        device
    }
    fn wrapper(&self) -> &harmonigraph_clap::wrapper::clap::Wrapper<crate::Harmonigraph> {
        unsafe { &*((*self.plugin).plugin_data.cast()) }
    }
    /// The host's side of the editor, as the shell gave it to the plugin.
    fn editor_host(&self) -> std::sync::Arc<dyn harmonigraph_clap::prelude::EditorHost> {
        self.wrapper().test_inspect_plugin(|plugin| plugin.editor_host.clone().expect("a host"))
    }
    /// The editor's state lock, which the window's size source logs under.
    fn editor_lock(&self) -> std::sync::Arc<crate::editor::EditorLock> {
        self.wrapper().test_inspect_plugin(|plugin| plugin.editor_shared.clone())
    }
    /// Run the host's main-thread callback, and count the times the project
    /// has been marked modified since the fixture began.
    fn service(&self) -> usize {
        unsafe {
            ((*self.plugin).on_main_thread.unwrap())(self.plugin);
        }
        self.stats.dirty.load(Ordering::Relaxed)
    }
    fn settings(&self) -> std::sync::Arc<Settings> {
        self.wrapper().test_inspect_plugin(|plugin| plugin.params.settings.clone())
    }
    fn activate(&mut self) {
        assert!(unsafe { ((*self.plugin).activate.unwrap())(self.plugin, 48000.0, 1, 64) });
        assert!(unsafe { ((*self.plugin).start_processing.unwrap())(self.plugin) });
        self.active = true;
    }
    /// Called explicitly after a fixture's original assertions. These fixtures
    /// study configuration and recording, so finish their gestures before
    /// destruction: a Hub that still holds a voice marks its take incomplete.
    fn finish_notes(&mut self, mut raw: i64, notes: &[(i32, i16)]) {
        let state = |device: &Self| {
            device.wrapper().test_inspect_plugin(|plugin| {
                let hub = plugin.aggregation.as_ref().unwrap();
                (hub.tune.held(), hub.tune.pending())
            })
        };
        self.run(
            raw,
            notes.iter().map(|&(id, key)| note(id, key, 0, CLAP_EVENT_NOTE_OFF)).collect(),
            false,
        );
        for _ in 0..16 {
            raw += 64;
            let mut stopped = transport(0.0, 0);
            stopped.flags &= !CLAP_TRANSPORT_IS_PLAYING;
            self.run_transport(raw, vec![], false, None, Some(stopped));
            if state(self) == (0, 0) {
                return;
            }
        }
        panic!("the delay line must drain after its releases: {:?}", state(self));
    }
    fn params(&self) -> &clap_plugin_params {
        unsafe {
            &*(((*self.plugin).get_extension.unwrap())(self.plugin, CLAP_EXT_PARAMS.as_ptr())
                .cast())
        }
    }
    fn id(&self, key: ParamKey) -> u32 {
        self.id_named(key.host_name())
    }
    /// The host id of the parameter the host names `name`.
    fn id_named(&self, name: &str) -> u32 {
        for index in 0..unsafe { (self.params().count.unwrap())(self.plugin) } {
            let mut info: clap_sys::ext::params::clap_param_info = unsafe { std::mem::zeroed() };
            assert!(unsafe { (self.params().get_info.unwrap())(self.plugin, index, &mut info) });
            if unsafe { CStr::from_ptr(info.name.as_ptr()) }.to_bytes() == name.as_bytes() {
                return info.id;
            }
        }
        panic!("missing parameter");
    }
    fn param(&self, key: ParamKey, plain: f32, time: u32) -> Input {
        let values = crate::HarmonigraphParams::default();
        Input::Param(clap_event_param_value {
            header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, time),
            param_id: self.id(key),
            cookie: ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value: values.table.spec(crate::Row::camera(key).index()).to_host(plain),
        })
    }
    fn load(&self, state: PluginState) {
        assert!(self.try_load(state));
    }
    fn try_load(&self, state: PluginState) -> bool {
        let json = serde_json::to_vec(&state).unwrap();
        let mut bytes = (json.len() as u64).to_le_bytes().to_vec();
        bytes.extend(json);
        let mut reader = Reader { bytes: &bytes, offset: 0 };
        let stream = clap_istream { ctx: (&mut reader as *mut Reader).cast(), read: Some(read) };
        let extension = unsafe {
            &*(((*self.plugin).get_extension.unwrap())(self.plugin, CLAP_EXT_STATE.as_ptr())
                .cast::<clap_plugin_state>())
        };
        unsafe { (extension.load.unwrap())(self.plugin, &stream) }
    }
    fn save(&self) -> PluginState {
        let mut bytes = Vec::<u8>::new();
        let stream = clap_ostream { ctx: (&mut bytes as *mut Vec<u8>).cast(), write: Some(write) };
        let extension = unsafe {
            &*(((*self.plugin).get_extension.unwrap())(self.plugin, CLAP_EXT_STATE.as_ptr())
                .cast::<clap_plugin_state>())
        };
        assert!(unsafe { (extension.save.unwrap())(self.plugin, &stream) });
        serde_json::from_slice(&bytes[8..]).unwrap()
    }
    fn run(&self, steady: i64, events: Vec<Input>, reject: bool) -> Sink {
        let (status, sink) = self.run_status(steady, events, reject);
        assert_ne!(status, CLAP_PROCESS_ERROR);
        sink
    }
    fn run_status(
        &self,
        steady: i64,
        events: Vec<Input>,
        reject: bool,
    ) -> (clap_process_status, Sink) {
        self.run_transport(steady, events, reject, None, None)
    }
    fn run_transport(
        &self,
        steady: i64,
        events: Vec<Input>,
        reject: bool,
        reject_kind: Option<u16>,
        transport: Option<clap_event_transport>,
    ) -> (clap_process_status, Sink) {
        let input = clap_input_events {
            ctx: (&events as *const Vec<Input>).cast_mut().cast(),
            size: Some(input_size),
            get: Some(input_get),
        };
        let mut sink = Sink { reject, reject_kind, attempts: Vec::with_capacity(1024) };
        let output = clap_output_events {
            ctx: (&mut sink as *mut Sink).cast(),
            try_push: Some(output_push),
        };
        let mut left = [0.0_f32; 64];
        let mut right = [0.0_f32; 64];
        let mut channels = [left.as_mut_ptr(), right.as_mut_ptr()];
        let [mut side_left, mut side_right] = self.sidechain.get();
        let mut side_channels = [side_left.as_mut_ptr(), side_right.as_mut_ptr()];
        let mut audio = clap_audio_buffer {
            data32: channels.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let inputs = [
            audio,
            clap_audio_buffer {
                data32: side_channels.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: 2,
                latency: 0,
                constant_mask: 0,
            },
        ];
        let process = clap_process {
            steady_time: steady,
            frames_count: 64,
            transport: transport.as_ref().map_or(ptr::null(), |t| t),
            audio_inputs: inputs.as_ptr(),
            audio_outputs: &mut audio,
            audio_inputs_count: 2,
            audio_outputs_count: 1,
            in_events: &input,
            out_events: &output,
        };
        let status = unsafe { ((*self.plugin).process.unwrap())(self.plugin, &process) };
        (status, sink)
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
        }
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
unsafe extern "C" fn read(stream: *const clap_istream, out: *mut c_void, size: u64) -> i64 {
    let reader = unsafe { &mut *((*stream).ctx.cast::<Reader>()) };
    let count = (size as usize).min(reader.bytes.len() - reader.offset);
    unsafe {
        ptr::copy_nonoverlapping(reader.bytes[reader.offset..].as_ptr(), out.cast(), count);
    }
    reader.offset += count;
    count as i64
}
unsafe extern "C" fn write(stream: *const clap_ostream, input: *const c_void, size: u64) -> i64 {
    unsafe { &mut *((*stream).ctx.cast::<Vec<u8>>()) }.extend_from_slice(unsafe {
        std::slice::from_raw_parts(input.cast::<u8>(), size as usize)
    });
    size as i64
}
/// A saved state whose musical settings hold `fifth` and no links.
fn restored(device: &Device, fifth: f32) -> PluginState {
    let mut state = device.wrapper().get_state_object();
    let mut settings =
        MusicalSettings { meantone: false, marvel: false, learning: false, ..Default::default() };
    settings.axes[1] = microcents(fifth);
    state.fields.insert(MUSICAL_SETTINGS.into(), serde_json::to_string(&settings).unwrap());
    state
}
/// The musical settings a saved state holds.
/// What the editor shows, with the raw axes a save stores.
struct Shown {
    raw: Tuning,
    view: ConfigurationView,
}
fn shown(settings: &Settings) -> Shown {
    let (raw, view) = settings.current();
    Shown { raw, view }
}
fn saved(state: &PluginState) -> MusicalSettings {
    serde_json::from_str(&state.fields[MUSICAL_SETTINGS]).unwrap()
}
fn plain(state: &PluginState, key: ParamKey) -> f32 {
    match state.params[key.id()] {
        ParamValue::F32(value) => value,
        _ => panic!("float parameter"),
    }
}

#[test]
fn active_restore_without_callbacks_has_coherent_save_readback_and_ordered_adoption() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    let settings = device.settings();
    device.load(restored(&device, 690.0));
    assert!(settings.submit(ConfigEdit::axis(TuningAxis::Three, microcents(695.0))));
    device.load(restored(&device, 705.0));
    let now = shown(&settings);
    assert!(now.view.unadopted.is_some());
    assert_eq!(now.raw.three, microcents(705.0), "the newest restore is what shows");
    assert_eq!(saved(&device.save()).axes[1], microcents(705.0));
    assert_eq!(saved(&device.wrapper().get_state_object()).axes[1], microcents(705.0));
    device.run(1000, vec![], false);
    let adopted = shown(&settings);
    assert!(!adopted.view.unadopted.is_some());
    assert_eq!(adopted.raw.three, microcents(705.0), "the queued UI edit came before it");
    assert!(settings.submit(ConfigEdit::axis(TuningAxis::Three, microcents(700.0))));
    let marked = device.service();
    device.run(1064, vec![], false);
    assert_eq!(shown(&settings).raw.three, microcents(700.0));
    assert!(device.service() > marked, "an applied UI edit marks the project modified");
}

/// The tuning pending label waits on the latest edit queued, not on the
/// configuration audio last applied: with audio stopped, a second edit
/// restarts the wait the first began (#1814 Q2).
#[test]
fn the_tuning_label_waits_on_the_latest_queued_edit() {
    use harmonigraph_ui::{PendingLabel, PENDING_AFTER};
    let settings = Settings::default();
    let start = std::time::Instant::now();
    let at = |ms: u64| start + std::time::Duration::from_millis(ms);
    let mut label = PendingLabel::default();
    assert!(settings.submit(ConfigEdit::axis(TuningAxis::Three, microcents(700.0))));
    assert!(!label.shown(settings.view().unadopted, at(0)));
    let later = PENDING_AFTER.as_millis() as u64 / 2;
    assert!(settings.submit(ConfigEdit::axis(TuningAxis::Three, microcents(701.0))));
    assert!(!label.shown(settings.view().unadopted, at(later)));
    let first_wait = PENDING_AFTER.as_millis() as u64;
    assert!(!label.shown(settings.view().unadopted, at(first_wait)), "the wait restarted");
    assert!(label.shown(settings.view().unadopted, at(later + first_wait)));
}

#[test]
fn interval_links_keep_their_value_on_release_and_recognize_new_entries() {
    let _scope = crate::test_scope::enter();
    for fifth in [696.58, *TuningAxis::Three.range().start(), *TuningAxis::Three.range().end()] {
        let mut device = Device::new();
        device.activate();
        let settings = device.settings();
        assert!(settings.submit(ConfigEdit::axis(TuningAxis::Three, microcents(fifth))));
        device.run(0, vec![], false);
        let linked = shown(&settings).view.resolved;
        assert!(linked.modes.tempered.syntonic && linked.modes.tempered.septimal_kleisma);
        assert!((linked.tuning.five_cents() - (4.0 * fifth - 2400.0)).abs() < 0.001);
        for comma in harmonigraph_core::Comma::ALL.into_iter().rev() {
            assert!(settings.submit(ConfigEdit::temper(comma, false, linked.tuning)));
        }
        assert!(
            shown(&settings).view.unadopted.is_some(),
            "the switches are one pending audio adoption"
        );
        device.run(64, vec![], false);
        let released = shown(&settings);
        assert_eq!(released.view.resolved.modes.tempered, Tempered::default());
        assert_eq!(released.raw.five, linked.tuning.five, "a released link keeps its value");
        assert_eq!(released.raw.seven, linked.tuning.seven);
        assert_eq!(saved(&device.save()).axes[2..4], [released.raw.five, released.raw.seven]);
        assert!(settings.submit(ConfigEdit::axis(TuningAxis::Five, released.raw.five)));
        device.run(128, vec![], false);
        assert!(
            shown(&settings).view.resolved.modes.tempered.syntonic,
            "a typed same-value entry recognizes meantone"
        );
        assert!(settings.submit(ConfigEdit::axis(TuningAxis::Five, 390_000_000)));
        device.run(192, vec![], false);
        let edited = shown(&settings).view.resolved;
        assert!(!edited.modes.tempered.syntonic);
        assert_eq!(edited.tuning.five_cents(), 390.0);
        assert!(edited.modes.tempered.septimal_kleisma);
    }
}

#[test]
fn a_same_sample_initial_tuning_reaches_learning() {
    learn_the_played_third(true, false);
}

/// Retune decides what the Hub corrects, not what Learn hears: a tuner in
/// front of Harmonigraph states the tuning Learn is armed to find.
#[test]
fn learning_reads_the_hub_input_with_retune_off() {
    learn_the_played_third(false, false);
}

#[test]
fn pass_through_learns_axes_even_with_retune_enabled() {
    learn_the_played_third(true, true);
}

fn learn_the_played_third(retune: bool, pass_through: bool) {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.wrapper().test_inspect_plugin(|plugin| {
        plugin.aggregation.as_ref().unwrap().shared.set_retune(retune)
    });
    let mut state = device.wrapper().get_state_object();
    let engine = if pass_through { 0 } else { 1 };
    state.params.insert("tuning-engine".into(), harmonigraph_clap::plugin::ParamValue::I32(engine));
    device.load(state);
    device.activate();
    let settings = device.settings();
    assert!(settings.submit(ConfigEdit { learning: Some(true), ..Default::default() }));
    let mut events = Vec::new();
    for key in [60, 64, 67] {
        events.push(Input::Note(clap_event_note {
            header: header::<clap_event_note>(CLAP_EVENT_NOTE_ON, 0),
            note_id: i32::from(key),
            port_index: 0,
            channel: 0,
            key,
            velocity: 0.8,
        }));
        // A just triad, so the keyboard tuning Learn derives is not 12-TET's,
        // played 10¢ sharp throughout, so the C offset it learns is not 0.
        let bend = 10.0
            + match key {
                64 => harmonigraph_core::tuning::FIVE_JUST - 400.0,
                67 => harmonigraph_core::tuning::THREE_JUST - 700.0,
                _ => 0.0,
            };
        events.push(Input::Tuning(clap_event_note_expression {
            header: header::<clap_event_note_expression>(CLAP_EVENT_NOTE_EXPRESSION, 0),
            expression_id: CLAP_NOTE_EXPRESSION_TUNING,
            note_id: i32::from(key),
            port_index: 0,
            channel: 0,
            key,
            value: f64::from(bend) / 100.0,
        }));
    }
    device.run(0, events, false);
    // The Hub tunes its own notes, so its input reaches learning at the
    // boundary after the one it arrived on rather than the same one.
    let marked = device.service();
    device.run(64, vec![], false);
    let learned = shown(&settings);
    let resolved = learned.view.resolved;
    let keyboard = resolved.policy.keyboard;
    assert_eq!(keyboard, harmonigraph_core::tuning::fifth_generated(keyboard[0]));
    let fifth = microcents(harmonigraph_core::tuning::THREE_JUST);
    assert!((keyboard[0] - fifth).abs() < 1_000);
    // The C offset is learned either way; it is not part of the target.
    let c = learned.raw.c_offset;
    assert!((c - 10_000_000).abs() < 1_000, "C offset {c}");
    // Only active retuning makes the lattice a fixed target.
    if retune && !pass_through {
        assert!(resolved.modes.tempered.syntonic);
        let held = [learned.raw.three, learned.raw.five, learned.raw.seven];
        assert_eq!(held, [700_000_000, 400_000_000, 1_000_000_000]);
    } else {
        assert!(!resolved.modes.tempered.syntonic);
        let third = microcents(harmonigraph_core::tuning::FIVE_JUST);
        assert!((learned.raw.five - third).abs() < 1_000);
    }
    assert!(device.service() > marked, "Learn marks the project");
    assert_eq!(saved(&device.save()).axes, learned.raw.axes(), "a save stores what Learn found");
    device.run(128, vec![], false);
    assert_eq!(shown(&settings).view.resolved.revision, resolved.revision);
    // A cut ends the chord with no delta on any row. Learn must stop hearing
    // it on the next callback, not whenever that source next plays.
    let heard = |device: &Device| {
        device.wrapper().test_inspect_plugin(|plugin| {
            let mut scratch = [harmonigraph_core::tuning::PitchClass::from_microcents(0);
                crate::tuning::HELD_VOICES];
            plugin.aggregation.as_ref().unwrap().held_classes(&mut scratch)
        })
    };
    assert_eq!(heard(&device), 3);
    crate::tuning::session::session().cut();
    device.run(192, vec![], false);
    assert_eq!(heard(&device), 0);
    device.finish_notes(256, &[(60, 60), (64, 64), (67, 67)]);
}

#[test]
fn a_full_command_queue_keeps_accepted_state() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    let settings = device.settings();
    device.load(restored(&device, 690.0));
    device.load(restored(&device, 695.0));
    for _ in 0..126 {
        assert!(settings.submit(ConfigEdit::default()));
    }
    assert!(!settings.submit(ConfigEdit::default()));
    let before = device.save();
    let mut refused = restored(&device, 700.0);
    refused.params.insert(ParamKey::CameraDistance.id().into(), ParamValue::F32(20.0));
    assert!(!device.try_load(refused), "a restore needs a free slot");
    assert_eq!(
        settings.take_notice().as_deref(),
        Some(RESTORE_REFUSED),
        "and says so in the Console"
    );
    let after = device.save();
    assert_eq!(saved(&after).axes[1], microcents(695.0));
    assert_eq!(
        plain(&after, ParamKey::CameraDistance),
        plain(&before, ParamKey::CameraDistance),
        "a refused restore installs none of its state"
    );
    device.activate();
    device.run(0, vec![], false);
    assert!(device.try_load(restored(&device, 705.0)), "an applied restore is repeatable");
    assert_eq!(saved(&device.save()).axes[1], microcents(705.0));
    device.run(64, vec![], false);
    let now = shown(&settings);
    assert!(!now.view.unadopted.is_some());
    assert_eq!(now.raw.three, microcents(705.0));
    assert_eq!(now.view.status, 8, "the deliberately rejected submission remains visible");
}

#[test]
fn destroying_pending_or_faulted_configuration_owners_does_not_panic() {
    let _scope = crate::test_scope::enter();
    for commands in [false, true] {
        let mut device = Device::new();
        if commands {
            let settings = device.settings();
            device.load(restored(&device, 690.0));
            device.load(restored(&device, 695.0));
            for _ in 0..126 {
                assert!(settings.submit(ConfigEdit::default()));
            }
            device.activate();
            device.run(0, vec![], false);
            device.load(restored(&device, 705.0));
            assert!(
                shown(&settings).view.unadopted.is_some(),
                "an accepted restore is still queued"
            );
        } else {
            device.activate();
            device.run(0, vec![device.param(ParamKey::CameraDistance, 6.0, 0)], false);
            // Timed past the block's end: the list is invalid, its input lost.
            let lost = vec![device.param(ParamKey::CameraDistance, 7.0, 64)];
            let (status, _) = device.run_status(64, lost, false);
            assert_eq!(status, CLAP_PROCESS_ERROR);
            assert_eq!(shown(&device.settings()).view.status & 2, 2);
        }
        drop(device);
    }
}

#[test]
fn construction_outside_the_clap_wrapper_has_no_configuration_owner() {
    let plugin = crate::Harmonigraph::default();
    assert!(
        plugin.configuration.is_none(),
        "construction outside the CLAP wrapper has no CLAP-only owner or callback work"
    );
}

// The fixture invokes only CLAP's allowed concurrent main/audio entrypoints;
// activation, destruction, and ordinary processing remain exclusively owned.
unsafe impl Sync for Device {}

/// The restored modes are the saved ones, read back as themselves: two
/// patterns that differ in every bit and neither of which is the default, so a
/// readback, a save or an adoption that dropped, swapped or defaulted one
/// cannot agree with the others by accident.
#[test]
fn a_restore_reads_back_and_saves_its_modes_before_and_after_adoption() {
    let _scope = crate::test_scope::enter();
    for (meantone, marvel, learning) in [(true, false, true), (false, true, false)] {
        let mut device = Device::new();
        device.activate();
        let settings = device.settings();
        let mut state = restored(&device, 700.0);
        let mut restoring = saved(&state);
        (restoring.meantone, restoring.marvel, restoring.learning) = (meantone, marvel, learning);
        state.fields.insert(MUSICAL_SETTINGS.into(), serde_json::to_string(&restoring).unwrap());
        device.load(state);
        let readback = shown(&settings).view;
        assert!(readback.unadopted.is_some());
        let readback = readback.resolved;
        let restored_modes = TuningModes {
            tempered: Tempered { syntonic: meantone, septimal_kleisma: marvel },
            learning,
        };
        assert_eq!(readback.modes, restored_modes);
        assert_eq!(readback.tuning.three, 700_000_000, "the readback's tuning is the restored one");
        let saved_host = saved(&device.save());
        assert_eq!(
            (saved_host.meantone, saved_host.marvel, saved_host.learning),
            (meantone, marvel, learning)
        );
        assert_eq!(saved_host.reducer().resolved().modes, readback.modes);
        assert_eq!(
            saved(&device.wrapper().get_state_object()).reducer().resolved().modes,
            readback.modes
        );
        device.run(0, vec![], false);
        assert_eq!(shown(&settings).view.resolved.modes, readback.modes);
        // And the reducer the audio thread runs Learn and tempering under
        // holds them too, not only the words that report it.
        let adopted = device
            .wrapper()
            .test_inspect_plugin(|plugin| plugin.configuration.as_ref().unwrap().current().modes);
        assert_eq!(adopted, restored_modes);
    }
}

/// A saved field that does not parse costs that field alone, with a Console
/// line: the rest of the project restores, and the instance keeps what it
/// held for the field.
#[test]
fn a_field_that_does_not_parse_is_skipped_with_a_console_line() {
    let _scope = crate::test_scope::enter();
    let device = Device::new();
    let maps = |device: &Device| {
        device.wrapper().test_inspect_plugin(|plugin| {
            serde_json::to_string(&*plugin.params.maps.read()).unwrap()
        })
    };
    let before = maps(&device);
    let mut state = device.wrapper().get_state_object();
    state.fields.insert("lattice-maps".into(), "{not json".into());
    state.fields.insert("editor-state".into(), r#"{"size":[901,702]}"#.into());
    state.params.insert("map-fifths".into(), ParamValue::I32(3));
    device.load(state);
    assert_eq!(maps(&device), before, "the maps the instance held");
    let (size, fifths) = device.wrapper().test_inspect_plugin(|plugin| {
        (plugin.params.editor_state.size(), plugin.params.int(crate::Row::MapFifths))
    });
    assert_eq!((size, fifths), ((901, 702), 3), "the rest of the project");
    let line = device.settings().take_notice().expect("a Console line");
    assert!(line.starts_with("This project's saved lattice-maps could not be read"), "{line}");
}

/// A project saved while the axes were host parameters (#1756) holds them
/// there, beside musical settings without axes. It opens at fresh axes with
/// one Console notice, keeps its links, and saves no tuning parameter back.
#[test]
fn a_project_saved_with_tuning_parameters_opens_at_fresh_axes() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    let settings = device.settings();
    let mut state = device.wrapper().get_state_object();
    device.load(state.clone());
    assert_eq!(settings.take_notice(), None, "a project saved since opens quietly");
    state.params.insert("tuning-three".into(), ParamValue::F32(696.0));
    state.params.insert("tuning-tolerance".into(), ParamValue::F32(3.0));
    state.fields.insert(MUSICAL_SETTINGS.into(), r#"{"meantone":false,"marvel":true}"#.into());
    device.load(state);
    assert_eq!(
        settings.take_notice().as_deref(),
        Some(
            "This project saved parameters this build no longer has, which were not read: \
             tuning-three, tuning-tolerance."
        )
    );
    assert_eq!(settings.take_notice().as_deref(), Some(LEGACY_AXES));
    assert_eq!(settings.take_notice(), None, "the notice is given once");
    device.activate();
    device.run(0, vec![], false);
    let opened = shown(&device.settings());
    assert_eq!(opened.raw.axes(), MusicalSettings::default().axes);
    let modes = opened.view.resolved.modes.tempered;
    assert_eq!((modes.syntonic, modes.septimal_kleisma), (false, true));
    let resaved = device.save();
    assert!(LEGACY_AXIS_IDS.iter().all(|id| !resaved.params.contains_key(*id)));
    assert_eq!(saved(&resaved).axes, MusicalSettings::default().axes);
}

#[test]
fn a_pending_restore_keeps_fault_and_refusal_status_visible() {
    let _scope = crate::test_scope::enter();
    let device = Device::new();
    let settings = device.settings();
    device.load(restored(&device, 690.0));
    let mut applied = settings.published.load();
    applied.status = 2;
    settings.published.publish(applied);
    for _ in 0..127 {
        assert!(settings.submit(ConfigEdit::default()));
    }
    assert!(!settings.submit(ConfigEdit::default()));
    let now = shown(&settings);
    assert_eq!(now.view.status & 10, 10);
    assert_eq!(now.raw.three, microcents(690.0));
}

/// A host that saves right after restoring, before it has run the plugin
/// once, stores the restored record rather than the fresh one.
#[test]
fn a_save_before_any_audio_stores_the_restore() {
    let _scope = crate::test_scope::enter();
    let device = Device::new();
    device.load(restored(&device, 690.0));
    let saved = saved(&device.save());
    assert_eq!(saved.axes[TuningAxis::Three.index()], microcents(690.0));
    assert!(!saved.meantone && !saved.marvel, "the restore's links, not the fresh ones");
}

/// An axis edit changes no host parameter, so the host marks the project
/// modified without rescanning; a map-document edit rescans the parameter
/// text, since the Lattice Map's label names its map.
#[test]
fn an_axis_edit_marks_the_project_without_a_rescan() {
    use clap_sys::ext::params::{CLAP_PARAM_RESCAN_TEXT, CLAP_PARAM_RESCAN_VALUES};
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    let settings = device.settings();
    device.run(0, vec![], false);
    let marked = device.service();
    let rescans = device.stats.rescans.load(Ordering::Relaxed);
    assert!(settings.submit(ConfigEdit::axis(TuningAxis::Three, microcents(696.0))));
    device.run(64, vec![], false);
    assert!(device.service() > marked, "the edit marks the project modified");
    assert_eq!(device.stats.rescans.load(Ordering::Relaxed), rescans, "and rescans nothing");
    settings.maps_changed();
    device.service();
    assert_eq!(device.stats.rescans.load(Ordering::Relaxed), rescans + 1);
    let flags = device.stats.rescan_flags.load(Ordering::Relaxed);
    assert_eq!(flags & CLAP_PARAM_RESCAN_TEXT, CLAP_PARAM_RESCAN_TEXT);
    assert_eq!(flags & CLAP_PARAM_RESCAN_VALUES, 0, "the values did not change");
}

/// Opening a project, and Reset all voices, change nothing the project saves,
/// so neither has the host mark it modified: the musical settings and the
/// instance's own Retune, Show and name alike.
#[test]
fn a_restore_and_a_reset_leave_the_project_unmodified() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    device.run(1000, vec![], false);
    let before = device.service();
    device.load(restored(&device, 700.0));
    device.run(1064, vec![], false);
    assert_eq!(device.service(), before, "a restore is not an edit");
    crate::tuning::instances::edit(0, harmonigraph_ui::params::InstanceEdit::Reset);
    device.run(1128, vec![], false);
    assert_eq!(device.service(), before, "Reset all voices saves nothing");
}

/// An edit to what `ui-state` saves has the host mark the project modified
/// once, as a musical setting's does; restoring it marks nothing (#1813).
/// Ended by a close, which reports a gesture it cut off.
#[test]
fn a_ui_state_edit_marks_the_project_once_and_its_restore_does_not() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    device.run(0, vec![], false);
    let lock = device.wrapper().test_inspect_plugin(|plugin| plugin.editor_shared.clone());
    let before = device.service();
    let mut project = harmonigraph_ui::SharedState::new(crate::editor::ASSUMED_SURFACE_FORMAT);
    let restored = project.picture.appearance.view.max_sevens + 1;
    project.picture.appearance.view.max_sevens = restored;
    let mut state = device.wrapper().get_state_object();
    let blob = harmonigraph_ui::shell::close(&project);
    state.fields.insert("ui-state".into(), serde_json::to_string(&blob).unwrap());
    device.load(state);
    let installed = lock.lock().ui.picture.appearance.view.max_sevens;
    assert_eq!(installed, restored, "the restore installed");
    let _ = lock.end_window_session();
    assert_eq!(device.service(), before, "a restore is not an edit");

    lock.lock().ui.picture.appearance.view.max_sevens = restored + 1;
    let _ = lock.end_window_session();
    assert_eq!(device.service(), before + 1, "the edit marks the project modified");
    let _ = lock.end_window_session();
    assert_eq!(device.service(), before + 1, "once");
}

/// A resize or an Own window switch, what `editor-state` saves, has the host
/// mark the project modified, once per change; a host's resize while the
/// editor opens marks nothing, nor does a restore, whose resize lands at the
/// next frame (#1824). `frame` is what each editor frame does, the window's
/// size source and then the frame's end, and the editor opens docked, where a
/// host's `set_size` is the editor's size.
/// A notice only sets a flag the host services, so the device is serviced
/// after every frame: notices from two frames would otherwise read as one.
#[test]
fn a_window_edit_marks_the_project_and_an_opening_resize_or_a_restore_does_not() {
    let _scope = crate::test_scope::enter();
    let device = Device::new();
    device.stats.grants_resize.store(true, Ordering::Relaxed);
    let gui = unsafe {
        &*(((*device.plugin).get_extension.unwrap())(device.plugin, CLAP_EXT_GUI.as_ptr())
            .cast::<clap_plugin_gui>())
    };
    let (mut api, mut floating) = (ptr::null(), false);
    unsafe {
        assert!((gui.get_preferred_api.unwrap())(device.plugin, &mut api, &mut floating));
        assert!((gui.create.unwrap())(device.plugin, api, false));
    }
    let state = device.wrapper().test_inspect_plugin(|plugin| plugin.params.editor_state.clone());
    let settings = device.settings();
    let (host, lock) = (device.editor_host(), device.editor_lock());
    let opened = std::cell::Cell::new(None);
    let frame = || {
        if let Some(opened) = opened.get() {
            crate::editor::next_size(&state, opened, host.as_ref(), &lock);
        }
        state.report_edit(|| settings.changed());
    };
    let resize = |grow: u32| {
        let (width, height) = state.size();
        let size = (width + grow, height + grow);
        assert!(unsafe { (gui.set_size.unwrap())(device.plugin, size.0, size.1) });
        assert_eq!(state.size(), size, "the host sized the editor");
    };
    let before = device.service();
    state.set_docked(true);
    opened.set(Some(state.open()));
    state.settle();
    resize(40);
    frame();
    assert_eq!(device.service(), before, "a resize while the editor opens");
    resize(30);
    frame();
    assert_eq!(device.service(), before + 1, "a resize");
    frame();
    assert_eq!(device.service(), before + 1, "once");
    state.set_docked(false);
    frame();
    assert_eq!(device.service(), before + 2, "an Own window switch");
    frame();
    assert_eq!(device.service(), before + 2, "once");

    let saved = crate::editor::EguiState::from_size(900, 700);
    saved.set_docked(true);
    let mut project = device.wrapper().get_state_object();
    project.fields.insert("editor-state".into(), serde_json::to_string(&*saved).unwrap());
    device.load(project);
    frame();
    assert_eq!(state.size(), (900, 700), "the restore installed");
    assert_eq!(device.service(), before + 2, "a restore");

    // Closed, then reopened: a frame the own window draws as the view moves
    // into it comes before the opening is settled.
    state.close();
    opened.set(Some(state.open()));
    resize(20);
    frame();
    assert_eq!(device.service(), before + 2, "a resize while the view moves in");
    state.settle();
    resize(20);
    frame();
    assert_eq!(device.service(), before + 2, "a resize while the editor opens again");
}

/// Bitwig's border drag stops at the editor's limits, the answer to
/// `gui.adjust_size`, and a host size outside them is refused, the editor
/// keeping its own (Yan, #1814). Docked and open, where a host size inside
/// them is the editor's.
#[test]
fn a_border_drag_stops_at_the_editors_limits_and_a_size_outside_them_is_refused() {
    let _scope = crate::test_scope::enter();
    let device = Device::new();
    let gui = unsafe {
        &*(((*device.plugin).get_extension.unwrap())(device.plugin, CLAP_EXT_GUI.as_ptr())
            .cast::<clap_plugin_gui>())
    };
    let (mut api, mut floating) = (ptr::null(), false);
    unsafe {
        assert!((gui.get_preferred_api.unwrap())(device.plugin, &mut api, &mut floating));
        assert!((gui.create.unwrap())(device.plugin, api, false));
    }
    let state = device.wrapper().test_inspect_plugin(|plugin| plugin.params.editor_state.clone());
    state.set_docked(true);
    state.set_open(true);
    let adjust = |(mut width, mut height): (u32, u32)| {
        assert!(unsafe { (gui.adjust_size.unwrap())(device.plugin, &mut width, &mut height) });
        (width, height)
    };
    let set_size = |(width, height): (u32, u32)| unsafe {
        (gui.set_size.unwrap())(device.plugin, width, height)
    };
    let limits = crate::editor::LIMITS;
    assert_eq!(adjust((1, 1)), limits.min, "a drag below the minimum");
    assert_eq!(adjust((u32::MAX, u32::MAX)), limits.max, "a drag past the maximum");
    let inside = (limits.min.0 + 1, limits.max.1 - 1);
    assert_eq!(adjust(inside), inside, "a drag inside them");

    let kept = state.size();
    for outside in [(limits.min.0 - 1, limits.min.1), (limits.max.0, limits.max.1 + 1)] {
        assert!(!set_size(outside), "{outside:?} was taken");
        assert_eq!(state.size(), kept, "a refused size replaced the editor's");
    }
    assert!(set_size(limits.min), "the minimum itself");
    assert_eq!(state.size(), limits.min);
}

/// A restore while the editor is open (a preset, Bitwig's undo) resizes its
/// window to the restored size at the next frame, through the host, and
/// marks nothing (Yan, #1814); until then the editor keeps the size its
/// window has. `frame` is the window's size source and the frame's end.
#[test]
fn a_restore_while_the_editor_is_open_resizes_its_window_at_the_next_frame() {
    let _scope = crate::test_scope::enter();
    let device = Device::new();
    device.stats.grants_resize.store(true, Ordering::Relaxed);
    let state = device.wrapper().test_inspect_plugin(|plugin| plugin.params.editor_state.clone());
    let settings = device.settings();
    let (host, lock) = (device.editor_host(), device.editor_lock());
    state.set_docked(true);
    let opened = state.open();
    let frame = || {
        let laid_out = crate::editor::next_size(&state, opened, host.as_ref(), &lock);
        state.report_edit(|| settings.changed());
        laid_out
    };
    frame();
    let kept = state.size();
    let restored = (kept.0 + 120, kept.1 + 80);
    let saved = crate::editor::EguiState::from_size(restored.0, restored.1);
    saved.set_docked(true);
    let mut project = device.wrapper().get_state_object();
    project.fields.insert("editor-state".into(), serde_json::to_string(&*saved).unwrap());
    let marks = device.service();

    device.load(project);
    assert_eq!(state.size(), kept, "resized before the window's next frame");
    assert!(device.stats.resizes.lock().unwrap().is_empty(), "the host asked before the frame");
    assert_eq!(frame(), Some(restored), "the frame was not laid out at the restored size");
    assert_eq!(*device.stats.resizes.lock().unwrap(), [restored], "the host asked");
    assert_eq!(state.size(), restored, "the window took the restored size");
    assert_eq!(device.service(), marks, "a restore marked the project");
}

/// The queue keeps arrival order across kinds: an edit queued before a restore
/// is overwritten by it, and one queued after it lands on the restored record.
#[test]
fn a_restore_queued_behind_an_edit_applies_after_it() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    let settings = device.settings();
    assert!(settings.submit(ConfigEdit::axis(TuningAxis::Three, microcents(695.0))));
    assert!(settings.submit(ConfigEdit::axis(TuningAxis::COffset, microcents(5.0))));
    device.load(restored(&device, 690.0));
    assert!(settings.submit(ConfigEdit::axis(TuningAxis::Seven, microcents(980.0))));
    device.run(0, vec![], false);
    let raw = shown(&settings).raw;
    assert_eq!(raw.three, microcents(690.0), "the restore came after the fifth");
    assert_eq!(raw.c_offset, 0, "and replaced the origin the edit before it set");
    assert_eq!(raw.seven, microcents(980.0), "the edit after it lands on the restore");
}

/// Every word the seqlock carries comes back as itself: distinct values in
/// every one, negatives in the signed ones, and modes that differ bit by bit.
#[test]
fn the_published_record_round_trips_every_word() {
    let record = Record {
        applied: u64::MAX - 7,
        raw: Tuning::from_axes([-1, -200_000_000, 3, 40_000_000, 5]),
        resolved: ResolvedConfig {
            revision: (1 << 40) + 9,
            tuning: Tuning::from_axes([-600_000_000, 11, -12, 13, 14]),
            modes: TuningModes {
                tempered: Tempered { syntonic: false, septimal_kleisma: true },
                learning: true,
            },
            policy: PolicyConfig {
                radius: 2,
                axes: 3,
                pitch_flexibility: 37,
                half_life_ms: 1234,
                register: 850,
                tolerance: 1_000_001,
                silence_ms: 4500,
                reset_stop: true,
                reset_loop: false,
                keyboard: [695_000_007, 380_000_008, 980_000_009],
            },
        },
        status: 16 | 2,
    };
    assert_eq!(Published::new(record).load(), record);
}

/// A take recorded with every axis off its default and a link released
/// carries those axes in its configuration records, which replay applies
/// verbatim, and none in its parameter lane.
#[test]
fn a_take_carries_released_axes_in_its_configuration_records() {
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = recorded_device();
    device.activate();
    let settings = device.settings();
    let edit = ConfigEdit {
        axes: [
            Some(microcents(12.0)),
            Some(microcents(696.0)),
            None,
            Some(microcents(975.0)),
            Some(microcents(3.0)),
        ],
        ..Default::default()
    };
    assert!(settings.submit(edit));
    device.run(0, vec![], false);
    let linked = settings.view().resolved.tuning;
    assert!(settings.submit(ConfigEdit::temper(harmonigraph_core::Comma::Syntonic, false, linked)));
    device.run(64, vec![], false);
    let expected = settings.view().resolved;
    assert!(!expected.modes.tempered.syntonic);
    let fresh = Tuning::default().axes();
    assert!(expected.tuning.axes().iter().zip(fresh).all(|(axis, fresh)| *axis != fresh));
    capture.arm();
    device.run(128, vec![], false);
    let entries = capture.drain_entries();
    let configs: Vec<_> = entries
        .iter()
        .filter_map(|entry| match entry {
            harmonigraph_record::Entry::Configuration(config) => Some(*config),
            _ => None,
        })
        .collect();
    assert_eq!(configs.len(), 1);
    let replayed = configs[0].resolved();
    assert_eq!(replayed.tuning, expected.tuning);
    assert_eq!(replayed.modes, expected.modes);
    let lane = entries.iter().filter(|e| matches!(e, harmonigraph_record::Entry::Param { .. }));
    assert_eq!(lane.count(), ParamKey::ALL.len(), "only the camera, once each");
}

fn note(id: i32, key: i16, time: u32, kind: u16) -> Input {
    Input::Note(clap_event_note {
        header: header::<clap_event_note>(kind, time),
        note_id: id,
        port_index: 0,
        channel: 0,
        key,
        velocity: 0.8,
    })
}
fn id_tuning(id: i32, value: f64) -> Input {
    Input::Tuning(clap_event_note_expression {
        header: header::<clap_event_note_expression>(CLAP_EVENT_NOTE_EXPRESSION, 0),
        expression_id: CLAP_NOTE_EXPRESSION_TUNING,
        note_id: id,
        port_index: -1,
        channel: -1,
        key: -1,
        value,
    })
}
#[test]
fn note_id_only_expression_and_release_match_only_the_addressed_held_note() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    // Retune back at its shipped default: with a source retuning, Learn
    // leaves the lattice axes alone, and the axes are what this reads.
    device.wrapper().test_inspect_plugin(|plugin| {
        plugin.aggregation.as_ref().unwrap().shared.set_retune(false)
    });
    device.activate();
    let settings = device.settings();
    assert!(settings.submit(ConfigEdit { learning: Some(true), ..Default::default() }));
    device.run(
        0,
        vec![note(10, 60, 0, CLAP_EVENT_NOTE_ON), note(20, 67, 0, CLAP_EVENT_NOTE_ON)],
        false,
    );
    // Move C, and let the move reach learning, before asking what learning
    // does with the interval that is left.
    device.run(64, vec![id_tuning(10, 1.0)], false);
    device.run(128, vec![], false);
    assert!(settings.submit(ConfigEdit::axis(TuningAxis::Three, 690_000_000)));
    device.run(192, vec![], false);
    assert_eq!(
        shown(&settings).raw.three,
        690_000_000,
        "only C moved: the remaining interval supplies no fifth evidence"
    );
    let mut release = match note(10, -1, 0, CLAP_EVENT_NOTE_OFF) {
        Input::Note(event) => event,
        _ => unreachable!(),
    };
    release.channel = -1;
    release.port_index = -1;
    device.run(256, vec![Input::Note(release)], false);
    assert!(settings.submit(ConfigEdit::axis(TuningAxis::Five, 390_000_000)));
    device.run(320, vec![id_tuning(20, 5.0), note(30, 64, 0, CLAP_EVENT_NOTE_ON)], false);
    device.run(384, vec![], false);
    assert_eq!(
        shown(&settings).raw.five,
        400_000_000,
        "ID20 survived the ID10 release and now supplies C beside E"
    );
    device.finish_notes(448, &[(20, 67), (30, 64)]);
}

std::thread_local! {
    static RECORDER: std::cell::RefCell<Option<harmonigraph_record::Recorder>> = const { std::cell::RefCell::new(None) };
}
pub(super) fn take_recorder() -> Option<harmonigraph_record::Recorder> {
    RECORDER.with(|r| r.borrow_mut().take())
}
pub(super) fn install_recorder(recorder: harmonigraph_record::Recorder) {
    RECORDER.with(|r| assert!(r.borrow_mut().replace(recorder).is_none()));
}
fn recorded_device() -> (Device, harmonigraph_record::testing::Capture) {
    let (recorder, capture) = harmonigraph_record::testing::channel();
    install_recorder(recorder);
    (Device::new(), capture)
}
fn transport(seconds: f64, time: u32) -> clap_event_transport {
    let mut transport: clap_event_transport = unsafe { std::mem::zeroed() };
    transport.header = header::<clap_event_transport>(CLAP_EVENT_TRANSPORT, time);
    transport.flags = CLAP_TRANSPORT_HAS_SECONDS_TIMELINE | CLAP_TRANSPORT_IS_PLAYING;
    transport.song_pos_seconds =
        (seconds * clap_sys::fixedpoint::CLAP_SECTIME_FACTOR as f64) as i64;
    transport
}
/// The Stop the editor sends. Each fixture first aims the recorder at a
/// renderer that does not exist, so a finished take launches no render.
fn stop(shared: &std::sync::Arc<crate::editor::EditorLock>) {
    let shared = shared.lock();
    shared.take.stop();
}

/// The notes one take file draws for key `key`, replayed as the renderer does.
fn drawn(take: &harmonigraph_take::Take, key: u8) -> Vec<(f64, Option<f64>)> {
    let mut tracker = harmonigraph_core::NoteTracker::new();
    for record in &take.events {
        record.apply(&mut tracker).unwrap();
    }
    tracker.roll().notes().filter(|n| n.note == key).map(|n| (n.start, n.end)).collect()
}

fn records_an_on(take: &harmonigraph_take::Take, key: u8) -> bool {
    take.events.iter().any(|record| {
        matches!(record.note(), Some(n) if n.note == key
            && matches!(n.kind, harmonigraph_take::NoteKind::On { .. }))
    })
}

/// #1129: the transport pauses, a note is struck while nothing records, and
/// playback resumes from where it stopped. The onset sounds in the pause, so
/// only the resume opening with what is sounding reaches it.
#[test]
fn a_take_resumed_from_a_pause_opens_with_the_note_struck_during_it() {
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = recorded_device();
    device.activate();
    capture.arm();
    let dir =
        std::env::temp_dir().join(format!("harmonigraph-config-resume-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("record.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone());
    let block = 64.0 / 48000.0;
    let parked = || {
        let mut parked = transport(block, 0);
        parked.flags &= !CLAP_TRANSPORT_IS_PLAYING;
        parked
    };
    device.run_transport(0, vec![], false, None, Some(transport(0.0, 0)));
    device.run_transport(64, vec![], false, None, Some(transport(block, 0)));
    // Parked where it stopped: no forward motion, so none of these records.
    // The strike waits a block, past the Stop's own session cut.
    device.run_transport(128, vec![], false, None, Some(parked()));
    let struck = vec![note(10, 60, 10, CLAP_EVENT_NOTE_ON)];
    device.run_transport(192, struck, false, None, Some(parked()));
    device.run_transport(256, vec![], false, None, Some(parked()));
    writer.drain(&mut capture);
    device.run_transport(320, vec![], false, None, Some(transport(block, 0)));
    let released = vec![note(10, 60, 0, CLAP_EVENT_NOTE_OFF)];
    device.run_transport(384, released, false, None, Some(transport(2.0 * block, 0)));
    for (raw, blocks) in [(448, 3.0), (512, 4.0)] {
        device.run_transport(raw, vec![], false, None, Some(transport(blocks * block, 0)));
    }
    writer.drain(&mut capture);
    assert!(!writer.failed());
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(!records_an_on(&take, 60), "the fixture must strike where nothing records");
    let notes = drawn(&take, 60);
    assert_eq!(notes.len(), 1, "the take draws the note struck in the pause: {notes:?}");
    assert!(notes[0].1.is_some(), "through the release it recorded");
    drop(writer);
    device.finish_notes(576, &[]);
    drop(device);
    std::fs::remove_dir_all(dir).unwrap();
}

/// A take paused and resumed reads whole, every note drawn and on its
/// soundtrack (#1760; on take v6 each case was refused or lost a note).
/// `behind-retrigger` resumes inside playback's jitter behind the stop, which
/// dropped a note silently (N1), and `snapback` plays its first block where it
/// was parked, then snaps back before any forward motion under a one-file
/// trigger, so its notes follow the audio rather than the host (D4). The cases
/// whose record sounds in the pause need a delay, so they play on a Tune's row
/// (`tuning::tests::a_paused_take_reads_whole_with_a_tunes_records_sounding_in_the_pause`).
#[test]
fn a_paused_take_reads_whole_with_every_note_drawn() {
    let block = 64.0 / 48000.0;
    let on = |id, key, t| note(id, key, t, CLAP_EVENT_NOTE_ON);
    let off = |id, key, t| note(id, key, t, CLAP_EVENT_NOTE_OFF);
    // (raw sample, events, transport position in blocks, playing)
    type Step = (i64, Vec<Input>, f64, bool);
    // Per key: the notes drawn, and whether each drew its release.
    type Drawn = &'static [(u8, usize, bool)];
    let park = |raw: i64, at: f64| -> Step { (raw, vec![], at, false) };
    let play = |raw: i64, events: Vec<Input>, at: f64| -> Step { (raw, events, at, true) };
    let snapback = std::iter::once(park(0, 750.0))
        .chain([play(64, vec![on(10, 60, 10)], 750.0)])
        .chain((0..12).map(|i| {
            let events = match i {
                1 => vec![off(10, 60, 5), on(20, 64, 6)],
                3 => vec![off(20, 64, 7)],
                5 => vec![on(30, 67, 1)],
                8 => vec![off(30, 67, 1)],
                _ => vec![],
            };
            play(128 + 64 * i, events, i as f64)
        }))
        .collect();
    // (name, one-file trigger, steps, notes drawn)
    let cases: [(&str, bool, Vec<Step>, Drawn); 2] = [
        (
            "behind-retrigger",
            false,
            vec![
                play(0, vec![on(10, 60, 10)], 0.0),
                play(64, vec![off(10, 60, 60)], 1.0),
                park(128, 2.0),
                park(192, 2.0),
                play(256, vec![on(11, 60, 0)], 0.0),
                play(320, vec![off(11, 60, 1)], 1.0),
                play(384, vec![on(12, 60, 2)], 2.0),
                play(448, vec![off(12, 60, 2)], 3.0),
                play(512, vec![], 4.0),
            ],
            &[(60, 3, true)],
        ),
        ("snapback", true, snapback, &[(60, 1, true), (64, 1, true), (67, 1, true)]),
    ];
    for (name, one_file, steps, expected) in cases {
        let _scope = crate::test_scope::enter();
        let (mut device, mut capture) = recorded_device();
        device.activate();
        capture.end_at_rewind(one_file);
        capture.arm();
        let dir = std::env::temp_dir()
            .join(format!("harmonigraph-config-paused-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("record.take");
        let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone());
        let mut last = 0;
        for (raw, events, at, playing) in steps {
            let mut position = transport(at * block, 0);
            if !playing {
                position.flags &= !CLAP_TRANSPORT_IS_PLAYING;
            }
            device.run_transport(raw, events, false, None, Some(position));
            last = raw;
        }
        // Parked from here: the last release is sequenced and published, and
        // the run ends.
        device.finish_notes(last + 64, &[]);
        writer.drain(&mut capture);
        assert!(!writer.failed(), "{name}");
        let take = harmonigraph_take::Take::read(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(take.incomplete.is_none(), "{name}: nothing was lost");
        let soundtrack = take.audio_start.unwrap();
        for &(key, count, released) in expected {
            let notes = drawn(&take, key);
            assert_eq!(notes.len(), count, "{name}: key {key}: {notes:?}");
            assert!(
                notes.iter().all(|&(start, end)| end.is_some() == released && start >= soundtrack),
                "{name}: key {key} on the soundtrack from {soundtrack}: {notes:?}"
            );
        }
        drop(writer);
        drop(device);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// A note struck late in a long pause is restated by the resume's opening
/// snapshot, and draws from the resume: its onset, translated by the resume's
/// offset, would otherwise fall before it, across the whole of the run before.
#[test]
fn a_note_struck_in_a_long_pause_draws_from_the_resume() {
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = recorded_device();
    device.activate();
    capture.arm();
    let dir =
        std::env::temp_dir().join(format!("harmonigraph-config-long-pause-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("record.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone());
    let block = 64.0 / 48000.0;
    let mut raw = 0;
    for i in 0..20 {
        device.run_transport(raw, vec![], false, None, Some(transport(i as f64 * block, 0)));
        raw += 64;
    }
    for i in 0..200 {
        let mut parked = transport(20.0 * block, 0);
        parked.flags &= !CLAP_TRANSPORT_IS_PLAYING;
        let struck = if i == 150 { vec![note(10, 60, 10, CLAP_EVENT_NOTE_ON)] } else { vec![] };
        device.run_transport(raw, struck, false, None, Some(parked));
        raw += 64;
        if i % 50 == 0 {
            writer.drain(&mut capture);
        }
    }
    for i in 0..10 {
        let released = if i == 5 { vec![note(10, 60, 0, CLAP_EVENT_NOTE_OFF)] } else { vec![] };
        device.run_transport(
            raw,
            released,
            false,
            None,
            Some(transport((20 + i) as f64 * block, 0)),
        );
        raw += 64;
    }
    device.finish_notes(raw, &[]);
    writer.drain(&mut capture);
    assert!(!writer.failed());
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(!records_an_on(&take, 60), "the fixture must strike where nothing records");
    let resume = take.audio_start.unwrap() + 20.0 * block;
    let notes = drawn(&take, 60);
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].0 >= resume - 1e-9, "drawn from {} before the resume at {resume}", notes[0].0);
    drop(writer);
    drop(device);
    std::fs::remove_dir_all(dir).unwrap();
}

/// A host reset right after a loop wrap: the take keeps what it recorded on
/// either side of the reset whole.
#[test]
fn a_host_reset_right_after_a_loop_wrap_keeps_the_take_whole() {
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = recorded_device();
    device.activate();
    capture.arm();
    let dir = std::env::temp_dir()
        .join(format!("harmonigraph-config-reset-split-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("record.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone());
    let block = 64.0 / 48000.0;
    let struck = vec![note(10, 60, 0, CLAP_EVENT_NOTE_ON)];
    device.run_transport(0, struck, false, None, Some(transport(9.0, 0)));
    let released = vec![note(10, 60, 30, CLAP_EVENT_NOTE_OFF)];
    device.run_transport(64, released, false, None, Some(transport(9.0 + block, 0)));
    device.run_transport(128, vec![], false, None, Some(transport(0.0, 0)));
    unsafe { ((*device.plugin).reset.unwrap())(device.plugin) };
    let mut raw = 192;
    for i in 1..8 {
        let events = match i {
            2 => vec![note(11, 64, 0, CLAP_EVENT_NOTE_ON)],
            4 => vec![note(11, 64, 0, CLAP_EVENT_NOTE_OFF)],
            _ => vec![],
        };
        device.run_transport(raw, events, false, None, Some(transport(i as f64 * block, 0)));
        raw += 64;
    }
    capture.stop();
    for _ in 0..12 {
        let mut parked = transport(8.0 * block, 0);
        parked.flags &= !CLAP_TRANSPORT_IS_PLAYING;
        device.run_transport(raw, vec![], false, None, Some(parked));
        raw += 64;
    }
    writer.stop();
    writer.drain(&mut capture);
    assert!(!writer.failed());
    assert!(writer.finished.is_some());
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(take.incomplete.is_none());
    for key in [60, 64] {
        let notes = drawn(&take, key);
        assert!(notes.len() == 1 && notes[0].1.is_some(), "{key}: {notes:?}");
    }
    drop(writer);
    drop(device);
    std::fs::remove_dir_all(dir).unwrap();
}

/// Armed with the transport stopped, then sent back to the start before
/// anything played — Bitwig's stop-returns-to-start, or a jump to bar 1 for an
/// AtBar take. Under take v7 that move owed a split, and paying it from a pass
/// that never rolled left the writer holding an empty pass open for a close
/// nothing sent: Stop waited on it forever and every later Start was refused.
#[test]
fn a_playhead_moved_back_before_the_take_rolls_lets_stop_finish_one_file() {
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = recorded_device();
    device.activate();
    capture.arm();
    let dir = std::env::temp_dir()
        .join(format!("harmonigraph-config-unrolled-rewind-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("record.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone());
    let parked = |seconds| {
        let mut parked = transport(seconds, 0);
        parked.flags &= !CLAP_TRANSPORT_IS_PLAYING;
        parked
    };
    device.run_transport(0, vec![], false, None, Some(parked(40.0)));
    device.run_transport(64, vec![], false, None, Some(parked(0.0)));
    writer.drain(&mut capture);
    let play = vec![note(10, 60, 0, CLAP_EVENT_NOTE_ON)];
    device.run_transport(128, play, false, None, Some(transport(0.0, 0)));
    writer.drain(&mut capture);
    capture.stop();
    writer.stop();
    device.run_transport(192, vec![], false, None, Some(transport(64.0 / 48000.0, 0)));
    writer.drain(&mut capture);
    assert!(!writer.failed());
    assert!(writer.finished.is_some(), "Stop must not wait on a run that never rolled");
    let take = harmonigraph_take::Take::read(writer.finished.as_ref().unwrap()).unwrap();
    assert!(take.incomplete.is_none());
    drop(writer);
    device.finish_notes(256, &[(10, 60)]);
    drop(device);
    std::fs::remove_dir_all(dir).unwrap();
}

/// #839/#828: no rejected scrub may open a run, consume the unchanged
/// configuration's sole record, or contribute samples to the WAV.
#[test]
fn pre_play_scrubs_emit_no_records_before_the_real_configuration_and_audio_origin() {
    use harmonigraph_record::Entry;
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = recorded_device();
    device.activate();
    capture.arm();
    for (block, seconds) in [10.0, 30.0, 5.0].into_iter().enumerate() {
        let mut stopped = transport(seconds, 0);
        stopped.flags &= !CLAP_TRANSPORT_IS_PLAYING;
        device.run_transport(block as i64 * 64, vec![], false, None, Some(stopped));
        assert!(capture.drain_entries().is_empty(), "a scrub contributes no record");
        assert!(capture.drain_audio().is_empty(), "a scrub contributes no samples");
    }
    for block in 0..3 {
        device.run_transport(
            (block + 3) * 64,
            vec![],
            false,
            None,
            Some(transport(5.0 + block as f64 * 64.0 / 48_000.0, 0)),
        );
    }
    let entries = capture.drain_entries();
    let mut parameters = Vec::new();
    let mut runs = Vec::new();
    let mut configurations = Vec::new();
    let mut samples = 0;
    for entry in entries {
        match entry {
            Entry::Param { t, .. } => parameters.push(t),
            Entry::Run { at, song } => runs.push((song, at)),
            Entry::Configuration(config) => configurations.push(config.t),
            Entry::AudioSamples(n) => samples += n,
            _ => {}
        }
    }
    let [(5.0, opened)] = runs[..] else { panic!("one run, at the accepted origin: {runs:?}") };
    assert_eq!(parameters, [opened; ParamKey::ALL.len()]);
    assert_eq!(configurations, [opened], "one unchanged configuration where the run opened");
    assert_eq!(samples, 3 * 64 * 2);
    assert_eq!(capture.drain_audio().len(), samples);
    capture.stop();
    device.finish_notes(6 * 64, &[]);
}

/// The take records the same buffer the analyzer reads. Main is silent and
/// every sidechain sample distinct, so a take that recorded Main, or the
/// sidechain's channels in the wrong order, cannot pass.
#[test]
fn a_sidechain_selection_reaches_the_armed_takes_audio() {
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = recorded_device();
    let left: [f32; 64] = std::array::from_fn(|i| 1.0 + i as f32);
    let right: [f32; 64] = std::array::from_fn(|i| -1.0 - i as f32);
    device.sidechain.set([left, right]);
    device.wrapper().test_inspect_plugin(|plugin| {
        plugin.params.set(crate::Row::AnalysisInput, 1.0);
    });
    device.activate();
    capture.arm();
    device.run_transport(0, vec![], false, None, Some(transport(5.0, 0)));
    let expected: Vec<f32> = left.iter().zip(&right).flat_map(|(l, r)| [*l, *r]).collect();
    assert_eq!(
        capture.drain_audio(),
        expected,
        "the armed take received Main instead of Sidechain"
    );
    capture.stop();
    device.finish_notes(64, &[]);
}

/// One accepted stopped callback is enough for a take, with or without MIDI:
/// the restore after it records nothing, and the stillness that follows ends
/// it. The real CLAP path must finalize exactly that audio prefix.
#[test]
fn a_short_stopped_export_finishes_with_or_without_midi() {
    let _scope = crate::test_scope::enter();
    for with_note in [false, true] {
        let mut device = Device::new();
        device.activate();
        let dir = std::env::temp_dir()
            .join(format!("harmonigraph-short-export-{}-{with_note}", std::process::id()));
        let shared = device.wrapper().test_inspect_plugin(|plugin| plugin.editor_shared.clone());
        let probe = {
            let mut shared = shared.lock();
            let probe = harmonigraph_record::testing::worker_probe(&shared.take, dir.clone());
            let render = &mut shared.ui.picture.appearance.render;
            render.trigger = harmonigraph_ui::RenderTrigger::OnTransportStop;
            harmonigraph_record::testing::set_renderer_program(
                &shared.take,
                dir.join("absent-renderer"),
            );
            let appearance = shared.ui.picture.appearance.serialize();
            shared.take.start(48_000.0, appearance);
            shared.poll_take_end();
            probe
        };
        let stopped = |seconds| {
            let mut t = transport(seconds, 0);
            t.flags &= !CLAP_TRANSPORT_IS_PLAYING;
            t
        };
        for (block, seconds) in [10.0, 30.0, 5.0, 5.0].into_iter().enumerate() {
            device.run_transport(block as i64 * 64, vec![], false, None, Some(stopped(seconds)));
            shared.lock().poll_through_stop_debounce();
            assert!(shared.lock().take.is_recording(), "parked/scrubbed takes stay armed");
        }
        let events = if with_note { vec![note(10, 60, 0, CLAP_EVENT_NOTE_ON)] } else { vec![] };
        let origin = 5.0 + 64.0 / 48_000.0;
        device.run_transport(256, events, false, None, Some(stopped(origin)));
        assert!(shared.lock().take.has_rolled());
        assert_eq!(shared.lock().take.captured() > 0, with_note);
        device.run_transport(320, vec![], false, None, Some(stopped(5.0)));
        assert!(!shared.lock().take.hit_rewind(), "the restore records nothing and ends nothing");
        shared.lock().poll_through_stop_debounce();
        assert!(!shared.lock().take.is_recording(), "stillness ends even one callback's take");
        device.finish_notes(384, if with_note { &[(10, 60)] } else { &[] });
        drop(shared);
        drop(device);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !probe.finished() && !probe.failed() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(!probe.failed());
        assert!(probe.finished(), "the accepted recording prefix must close");
        let paths: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|p| p.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "take"))
            .collect();
        assert_eq!(paths.len(), 1);
        let take = harmonigraph_take::Take::read(&paths[0]).unwrap();
        let actual_origin = take.audio_start.unwrap();
        assert!((actual_origin - origin).abs() < 1e-8);
        assert_eq!(take.configurations.len(), 1);
        assert!((take.configurations[0].t - actual_origin).abs() < 1e-9);
        assert_eq!(take.params.len(), ParamKey::ALL.len());
        assert!(take.params.iter().all(|p| (p.t - actual_origin).abs() < 1e-9));
        let encoded = std::fs::read_to_string(&paths[0]).unwrap();
        assert_eq!(encoded.matches("Run((").count(), 1, "only one run states the WAV alignment");
        let wav = std::fs::read(paths[0].with_extension("wav")).unwrap();
        assert_eq!(wav.len(), 44 + 64 * 2 * 4, "exactly one callback, no scrub or restore audio");
        // Harmonigraph's own input sounds in the callback it is played in,
        // the one the take recorded, so its onset is the take's one onset.
        let onsets =
            take.notes().filter(|note| matches!(note.kind, harmonigraph_take::NoteKind::On { .. }));
        assert_eq!(onsets.count(), usize::from(with_note), "{:?}", take.events);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// Stop followed immediately by real CLAP destruction has no later process
/// callback to observe disarm. Only its complete, released prefix may finish.
#[test]
fn destruction_closes_a_stopped_take_without_another_process_callback() {
    let _scope = crate::test_scope::enter();
    for (stopped, held) in [(true, false), (false, false), (true, true)] {
        let mut device = Device::new();
        let left: [f32; 64] = std::array::from_fn(|i| (i + 1) as f32 / 128.0);
        let right = left.map(|v| -v);
        device.sidechain.set([left, right]);
        device.wrapper().test_inspect_plugin(|plugin| {
            plugin.params.set(crate::Row::AnalysisInput, 1.0);
        });
        device.activate();
        let dir = std::env::temp_dir()
            .join(format!("harmonigraph-stop-destroy-{}-{stopped}-{held}", std::process::id(),));
        let shared = device.wrapper().test_inspect_plugin(|plugin| plugin.editor_shared.clone());
        let probe = {
            let shared = shared.lock();
            let probe = harmonigraph_record::testing::worker_probe(&shared.take, dir.clone());
            harmonigraph_record::testing::set_renderer_program(
                &shared.take,
                dir.join("absent-renderer"),
            );
            shared.take.start(48_000.0, shared.ui.picture.appearance.serialize());
            probe
        };
        let events = if held { vec![note(10, 60, 0, CLAP_EVENT_NOTE_ON)] } else { vec![] };
        device.run_transport(0, events, false, None, Some(transport(5.0, 0)));
        assert!(shared.lock().take.has_rolled());
        if held {
            device.wrapper().test_inspect_plugin(|plugin| {
                assert!(plugin
                    .aggregation
                    .as_ref()
                    .unwrap()
                    .test_voice(crate::tuning::TUNERS as u8, 0, 60)
                    .is_some());
            });
        }
        if stopped {
            stop(&shared);
        }
        // No finish_notes or other process callback: Drop uses the host's
        // stop_processing/deactivate/destroy vtables and joins the writer.
        drop(shared);
        drop(device);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !probe.finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(probe.finished(), "joined destruction must retire the writer");
        // A held voice is no failure: the take draws it without an end.
        let complete = stopped;
        assert_eq!(probe.failed(), !complete, "stopped={stopped}, held={held}");
        let paths: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|p| p.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "take"))
            .collect();
        assert_eq!(paths.len(), 1);
        let take = harmonigraph_take::Take::read(&paths[0]).unwrap();
        assert_eq!(take.incomplete.is_none(), complete);
        assert_eq!(take.audio_start, Some(5.0));
        assert_eq!(take.configurations.len(), 1);
        let wav = std::fs::read(paths[0].with_extension("wav")).unwrap();
        let expected: Vec<u8> = left
            .iter()
            .zip(&right)
            .flat_map(|(l, r)| [*l, *r])
            .flat_map(f32::to_le_bytes)
            .collect();
        assert_eq!(wav.len(), 44 + expected.len());
        assert_eq!(&wav[44..], expected, "the exact nonempty callback audio survives");
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// Real host lifecycle: Stop after deactivation must close the exact prefix
/// without process or destruction, including when activation beats the wake.
#[test]
fn deactivated_stop_finishes_without_another_process_callback() {
    let _scope = crate::test_scope::enter();
    for (resume, stop_before_deactivate, held, source_state) in [
        (0, false, false, 0),
        (1, false, false, 0),
        (2, false, false, 0),
        (0, true, false, 0),
        (0, false, true, 0),
        (0, false, false, 1),
        (0, false, false, 2),
    ] {
        let mut device = Device::new();
        let left: [f32; 64] = std::array::from_fn(|i| (i + 1) as f32 / 128.0);
        let right = left.map(|v| -v);
        device.sidechain.set([left, right]);
        device.wrapper().test_inspect_plugin(|plugin| {
            plugin.params.set(crate::Row::AnalysisInput, 1.0);
        });
        device.activate();
        let mut source = (source_state != 0).then(|| {
            let mut source = crate::tuning::tune::Tune::new(crate::tuning::setup::Shared::source());
            source.register();
            source.activate(48_000.0, 64, 1);
            source
        });
        let dir = std::env::temp_dir().join(format!(
            "harmonigraph-deactivated-stop-{}-{resume}-{stop_before_deactivate}-{held}-{source_state}",
            std::process::id()
        ));
        let shared = device.wrapper().test_inspect_plugin(|plugin| plugin.editor_shared.clone());
        let probe = {
            let shared = shared.lock();
            let probe = harmonigraph_record::testing::worker_probe(&shared.take, dir.clone());
            harmonigraph_record::testing::set_renderer_program(
                &shared.take,
                dir.join("absent-renderer"),
            );
            shared.take.start(48_000.0, shared.ui.picture.appearance.serialize());
            probe
        };
        let events = if held { vec![note(10, 60, 0, CLAP_EVENT_NOTE_ON)] } else { vec![] };
        device.run_transport(0, events, false, None, Some(transport(5.0, 0)));
        assert!(shared.lock().take.has_rolled());
        if let Some(source) = &mut source {
            source.begin(&harmonigraph_clap::plugin::Boundary {
                steady_time: 0,
                frames: 64,
                transport: None,
                valid: true,
                input_lost: false,
            });
            if source_state == 1 {
                source.end();
            }
            // State 2 pauses the source before its first copy: the ring is
            // empty, but this older callback can still publish into the take.
        }
        if stop_before_deactivate {
            stop(&shared);
            // A main callback while activated must not manufacture closure.
            unsafe { ((*device.plugin).on_main_thread.unwrap())(device.plugin) };
            assert!(shared.lock().take.last_take().is_none());
        }
        unsafe {
            ((*device.plugin).stop_processing.unwrap())(device.plugin);
            ((*device.plugin).deactivate.unwrap())(device.plugin);
        }
        device.active = false;
        if resume == 2 {
            // Stop lands after activate, before start_processing resets routes.
            assert!(unsafe { ((*device.plugin).activate.unwrap())(device.plugin, 48000.0, 1, 64) });
        }
        if !stop_before_deactivate {
            let callbacks = device.stats.callbacks.load(Ordering::Relaxed);
            stop(&shared);
            assert!(
                device.stats.callbacks.load(Ordering::Relaxed) > callbacks,
                "Stop must request host service even with no further audio"
            );
        }
        if resume == 2 {
            assert!(unsafe { ((*device.plugin).start_processing.unwrap())(device.plugin) });
            device.active = true;
        } else if resume == 1 {
            device.activate();
        } else {
            unsafe { ((*device.plugin).on_main_thread.unwrap())(device.plugin) };
        }
        // A held voice, or a source that may still publish, is no failure:
        // nothing is proven, and the take draws what reached it.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let path = loop {
            if let Some(path) = shared.lock().take.last_take() {
                break path;
            }
            assert!(std::time::Instant::now() < deadline,
                "take did not finish while alive: {resume}, {stop_before_deactivate}, {held}, {source_state}");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert!(!probe.failed());
        let take = harmonigraph_take::Take::read(&path).unwrap();
        assert!(take.incomplete.is_none());
        assert_eq!(take.audio_start, Some(5.0));
        assert_eq!(take.configurations.len(), 1);
        let expected: Vec<u8> = left
            .iter()
            .zip(&right)
            .flat_map(|(l, r)| [*l, *r])
            .flat_map(f32::to_le_bytes)
            .collect();
        let wav = std::fs::read(path.with_extension("wav")).unwrap();
        assert_eq!(wav.len(), 44 + expected.len());
        assert_eq!(&wav[44..], expected);
        if let Some(source) = &mut source {
            if source_state == 2 {
                source.input(harmonigraph_clap::events::Input {
                    sample: 10,
                    value: harmonigraph_clap::events::InputValue::Note {
                        kind: CLAP_EVENT_NOTE_ON,
                        note_id: 1,
                        port: 0,
                        channel: 0,
                        key: 60,
                        velocity: 1.0,
                        flags: 0,
                    },
                });
                assert_eq!(source.captured, 1, "the late copy really reaches its ring");
                source.end();
            }
            source.retire();
        }
        {
            // Start is accepted before another process; its fresh epoch must
            // also survive the pending main wake and reactivation.
            shared.lock().take.start(48_000.0, String::new());
            assert!(shared.lock().take.is_recording());
            unsafe { ((*device.plugin).on_main_thread.unwrap())(device.plugin) };
            assert!(shared.lock().take.is_recording());
            if resume == 0 {
                device.activate();
            }
            device.run_transport(64, vec![], false, None, Some(transport(6.0, 0)));
            assert!(shared.lock().take.has_rolled());
            stop(&shared);
            device.run_transport(128, vec![], false, None, Some(transport(7.0, 0)));
        }
        drop(shared);
        drop(device);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !probe.finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(probe.finished());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// A note reaches the take through the Hub's publication, so "has this take
/// captured anything" is answered by what the recorder took in, not by a
/// count kept anywhere else (#818).
///
/// The plugin's own recorder and the editor state wired to it, not an injected
/// one: the claim is that the editor's stop debounce sees a note that
/// only the Hub carried. Nothing rewinds, so the debounce is the only thing
/// here that can end the take.
#[test]
fn a_note_through_the_configuration_owner_lets_a_stopped_transport_end_the_take() {
    let _scope = crate::test_scope::enter();
    for with_notes in [true, false] {
        let mut device = Device::new();
        // Retune back at its shipped default: what is claimed is the take path
        // every CLAP session has, not the correction pipeline `new` enables it for.
        device.wrapper().test_inspect_plugin(|plugin| {
            plugin.aggregation.as_ref().unwrap().shared.set_retune(false)
        });
        device.activate();
        let dir = std::env::temp_dir()
            .join(format!("harmonigraph-config-transport-stop-{}", std::process::id()));
        let shared = device.wrapper().test_inspect_plugin(|plugin| plugin.editor_shared.clone());
        let probe = {
            let mut shared = shared.lock();
            let probe = harmonigraph_record::testing::worker_probe(&shared.take, dir.clone());
            let render = &mut shared.ui.picture.appearance.render;
            render.trigger = harmonigraph_ui::RenderTrigger::OnTransportStop;
            harmonigraph_record::testing::set_renderer_program(
                &shared.take,
                dir.join("absent-renderer"),
            );
            let appearance = shared.ui.picture.appearance.serialize();
            shared.take.start(48_000.0, appearance);
            assert!(shared.take.is_recording(), "armed");
            probe
        };
        // Played: one note on and off, with blocks after each for the Hub's delay
        // line to publish it into the take.
        let mut raw = 0;
        for block in 0..16 {
            let events = match block {
                0 if with_notes => vec![note(10, 60, 0, CLAP_EVENT_NOTE_ON)],
                8 if with_notes => vec![note(10, 60, 0, CLAP_EVENT_NOTE_OFF)],
                _ => vec![],
            };
            device.run_transport(
                raw,
                events,
                false,
                None,
                Some(transport(raw as f64 / 48000.0, 0)),
            );
            raw += 64;
        }
        assert_eq!(shared.lock().take.captured() > 0, with_notes);
        // Then stopped where it was, without the playhead going back.
        let mut parked = transport(raw as f64 / 48000.0, 0);
        parked.flags &= !CLAP_TRANSPORT_IS_PLAYING;
        device.run_transport(raw, vec![], false, None, Some(parked));
        device.run_transport(raw + 64, vec![], false, None, Some(parked));

        shared.lock().poll_through_stop_debounce();
        assert!(
            !shared.lock().take.is_recording(),
            "a played take ends once transport stops, with or without MIDI"
        );
        device.finish_notes(raw + 128, &[]);
        drop(shared);
        drop(device);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !probe.finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn a_configuration_applied_before_arming_records_once_as_the_takes_initial_value() {
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = recorded_device();
    device.activate();
    for i in 0..9 {
        assert!(device
            .settings()
            .submit(ConfigEdit::axis(TuningAxis::Three, microcents(690.0 + i as f32))));
    }
    device.run(0, vec![], false);
    capture.arm();
    device.run(64, vec![], false);
    let records = capture.drain_entries();
    let configs: Vec<_> = records
        .into_iter()
        .filter_map(|entry| match entry {
            harmonigraph_record::Entry::Configuration(config) => Some(config),
            _ => None,
        })
        .collect();
    assert_eq!(configs.len(), 1, "only the take's initial value is recorded");
    assert_eq!(configs[0].axes[1], 698_000_000);
    assert!((configs[0].t - 64.0 / 48000.0).abs() < 1e-9);
}

#[test]
fn canonical_publication_slots_and_loss_are_allocation_free() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_core::canonical::*;
    use harmonigraph_record::publication::*;
    let (mut publisher, mut consumer) = channel();
    let voices: [_; 64] = std::array::from_fn(|note| VoiceBaseline {
        note: note as u8,
        lifetime: note as u64 + 1,
        pitch_microcents: note as i64 * 100_000_000,
        velocity: 0.8,
        ..Default::default()
    });
    let baseline = SourceBaseline::new(SourceId::DIRECT, 1.0, 0, true, &voices).unwrap();
    let start = std::time::Instant::now();
    nice_assert_no_alloc::assert_no_alloc(|| {
        for _ in 0..SNAPSHOT_SLOTS {
            publisher.baseline(&baseline).unwrap();
        }
        assert_eq!(publisher.baseline(&baseline), Err(PublishError::Busy));
        for _ in SNAPSHOT_SLOTS..PUBLICATION_RING - 1 {
            publisher
                .note(harmonigraph_core::NoteEvent::on(1.0, SourceId::DIRECT, 0, 60, 0.8).into())
                .unwrap();
        }
        assert_eq!(
            publisher.note(harmonigraph_core::NoteEvent::off(2.0, SourceId::DIRECT, 0, 60).into()),
            Err(PublishError::Lost)
        );
    });
    let duration = start.elapsed();
    // Consume outside the audio guard, then exercise payload reuse under it.
    consumer.drain(|_| true);
    nice_assert_no_alloc::assert_no_alloc(|| publisher.baseline(&baseline).unwrap());
    consumer.drain(|_| true);
    eprintln!("canonical guarded fill: {SNAPSHOT_SLOTS} complete 64-voice payloads + {} notes + Busy/Lost = {duration:?}; no allocation/deallocation", PUBLICATION_RING - 1 - SNAPSHOT_SLOTS);
}

#[test]
fn adaptive_settings_restore_readback_save_and_audio_adoption_agree() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    let policy = PolicyConfig {
        pitch_flexibility: 37,
        axes: 3,
        silence_ms: 4500,
        reset_loop: true,
        reset_stop: true,
        ..Default::default()
    };
    let mut state = restored(&device, 700.0);
    state.fields.insert(
        MUSICAL_SETTINGS.into(),
        serde_json::to_string(&MusicalSettings { adaptive: policy.into(), ..Default::default() })
            .unwrap(),
    );
    device.load(state);
    let settings = device.settings();
    assert_eq!(shown(&settings).view.resolved.policy, policy);
    let saved: MusicalSettings =
        serde_json::from_str(&device.save().fields[MUSICAL_SETTINGS]).unwrap();
    assert_eq!(PolicyConfig::from(saved.adaptive), policy);
    device.run(0, vec![], false);
    assert_eq!(shown(&settings).view.resolved.policy, policy);
    let defaults: MusicalSettings = serde_json::from_str("{\"adaptive\":{}}").unwrap();
    assert_eq!(PolicyConfig::from(defaults.adaptive), PolicyConfig::default());
    let old: MusicalSettings =
        serde_json::from_str(r#"{"adaptive":{"version":2,"harmonic":20000,"pitch_scale":20}}"#)
            .unwrap();
    assert_eq!(PolicyConfig::from(old.adaptive), PolicyConfig::default());
    let clamped: MusicalSettings =
        serde_json::from_str(r#"{"adaptive":{"pitch_flexibility":65535}}"#).unwrap();
    assert_eq!(PolicyConfig::from(clamped.adaptive).pitch_flexibility, 100);
}

/// The presentation clock, which dates the live audio ring and the take's
/// published clock, runs one block's length per process call at the host's
/// rate.
#[test]
fn the_presentation_clock_advances_by_each_blocks_length() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    for block in 0..3 {
        device.run(block * 64, vec![], false);
    }
    let seconds = device.wrapper().test_inspect_plugin(|plugin| plugin.presentation_seconds);
    assert!((seconds - 3.0 * 64.0 / 48_000.0).abs() < 1e-12, "{seconds} s after three blocks");
}

/// A fresh Hub leaves player pitch alone until someone picks a retuning
/// engine: the saved param and the engine the audio thread adopted both say
/// so.
#[test]
fn a_fresh_hub_passes_note_pitch_through() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    device.run(0, vec![], false);
    let (param, engine) = device.wrapper().test_inspect_plugin(|plugin| {
        (plugin.params.int(crate::Row::TuningEngine), plugin.params.map_playback.lock().engine)
    });
    assert_eq!(param, 0);
    assert_eq!(engine, harmonigraph_core::lattice_map::TuningEngine::Off);
}

#[test]
fn lattice_maps_restore_without_editor_preserves_geometry_and_shared_tuning() {
    use harmonigraph_core::lattice_map::{LatticeMap, TuningEngine};
    use harmonigraph_ui::lattice_maps::{MapDocument, MapEdit, MapOffsetLane};
    use std::sync::Arc;
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    let original_names = device
        .wrapper()
        .test_inspect_plugin(|plugin| crate::lattice_maps::view(&plugin.params).names);
    let mut map =
        LatticeMap { position: harmonigraph_core::LatticePos::new(50, 0, 0), ..Default::default() };
    map.replace(harmonigraph_core::LatticePos::new(54, 0, 0));
    let mut document = MapDocument::default();
    document.capture(map, "Distant passage".into());
    map.position = harmonigraph_core::LatticePos::new(50, -2, 1);
    let mut state = restored(&device, 696.5);
    state.fields.insert("lattice-maps".into(), serde_json::to_string(&document).unwrap());
    state.params.insert("lattice-map".into(), harmonigraph_clap::plugin::ParamValue::I32(1));
    for (id, value) in [
        ("map-fifths", 0),
        ("map-thirds", 8),
        ("map-sevenths", -9),
        ("map-fifths-extension", 5),
        ("map-thirds-extension", -1),
        ("map-sevenths-extension", 1),
    ] {
        state.params.insert(id.into(), harmonigraph_clap::plugin::ParamValue::I32(value));
    }
    state.params.insert("tuning-engine".into(), harmonigraph_clap::plugin::ParamValue::I32(2));
    device.load(state);
    device.wrapper().test_inspect_plugin(|plugin| {
        let preview = crate::lattice_maps::view(&plugin.params);
        assert_eq!(preview.playback.engine, TuningEngine::LatticeMap);
        assert_eq!(preview.playback.map, Some(map));
        assert_eq!(preview.playback.offset, preview.offsets.total());
        assert!(!Arc::ptr_eq(&original_names, &preview.names));
        assert_eq!(&preview.names[1], &(1, "Distant passage".into()));
        assert!(
            preview.unadopted.is_some(),
            "restored intent must be visible before audio adoption"
        );
    });
    let saved = device.save();
    let recalled: MapDocument = serde_json::from_str(&saved.fields["lattice-maps"]).unwrap();
    assert_eq!(
        recalled.map(1),
        Some(LatticeMap { position: harmonigraph_core::LatticePos::ORIGIN, ..map })
    );
    for (id, value) in [
        ("map-fifths", 0),
        ("map-thirds", 8),
        ("map-sevenths", -9),
        ("map-fifths-extension", 5),
        ("map-thirds-extension", -1),
        ("map-sevenths-extension", 1),
    ] {
        assert!(
            matches!(saved.params[id], harmonigraph_clap::plugin::ParamValue::I32(actual) if actual == value)
        );
    }
    device.run(0, vec![], false);
    device.wrapper().test_inspect_plugin(|plugin| {
        let playback = *plugin.params.map_playback.lock();
        assert_eq!(playback.engine, TuningEngine::LatticeMap);
        assert_eq!(playback.selected, 1);
        assert_eq!(playback.map, Some(map));
        assert_eq!(plugin.configuration.as_ref().unwrap().current().tuning.three, 696_500_000);
    });
    let context = device.editor_host();
    let destination = map.position + harmonigraph_core::LatticePos::new(-3, 0, 0);
    let mut changed = map;
    assert!(changed.replace(destination), "the edit must change the captured geometry");
    device.wrapper().test_inspect_plugin(|plugin| {
        assert!(!crate::lattice_maps::view(&plugin.params).unadopted.is_some());
        let edit = |action| crate::lattice_maps::edit(&plugin.params, context.as_ref(), action);
        let dirty = &plugin.params.settings.modified;
        dirty.store(false, std::sync::atomic::Ordering::Release);
        edit(MapEdit::EditShape(true));
        edit(MapEdit::Replace(map.node(64)));
        edit(MapEdit::Undo);
        assert!(
            !dirty.load(std::sync::atomic::Ordering::Acquire),
            "an edit that changes nothing must not mark the project modified"
        );
        edit(MapEdit::Replace(destination));
        assert!(dirty.load(std::sync::atomic::Ordering::Acquire));
        let preview = crate::lattice_maps::view(&plugin.params);
        assert!(preview.editing() && preview.can_undo);
        assert_eq!(preview.playback.map, Some(changed));
        assert_eq!(
            plugin.params.maps.read().map(1),
            Some(LatticeMap { position: harmonigraph_core::LatticePos::ORIGIN, ..changed }),
            "an edit is saved to the selected map itself"
        );
        assert!(
            preview.unadopted.is_some(),
            "editing follows intent before audio adopts the shape"
        );
        edit(MapEdit::Undo);
        let undone = crate::lattice_maps::view(&plugin.params);
        assert_eq!(undone.playback.map, Some(map));
        assert!(!undone.can_undo);
        edit(MapEdit::Replace(destination));
        edit(MapEdit::Duplicate);
        let copied = crate::lattice_maps::view(&plugin.params);
        assert_eq!(&copied.names[2], &(2, "Distant passage copy".into()));
        // Thirds stand at fine 8, coarse −1: two arrow presses carry into
        // Coarse rather than stopping at the end of Fine, and the second,
        // pressed before the host has applied the first, still counts — with
        // a frame drawn between them, as the editor draws one.
        let right = harmonigraph_core::LatticePos::new(0, 1, 0);
        edit(MapEdit::Translate(right, MapOffsetLane::Fine));
        crate::lattice_maps::view(&plugin.params);
        edit(MapEdit::Translate(right, MapOffsetLane::Fine));
    });
    device.run(64, vec![], false);
    device.wrapper().test_inspect_plugin(|plugin| {
        let playback = *plugin.params.map_playback.lock();
        let moved = crate::lattice_maps::offsets(&plugin.params);
        assert_eq!((moved.fine.fives, moved.extension.fives), (0, 0), "−2 + 2 is exactly zero");
        assert_eq!(playback.selected, 2, "a duplicate is selected for further edits");
        let translated = LatticeMap { position: moved.total(), ..changed };
        assert_eq!(playback.map, Some(translated), "audio adopts the translated shape");
        assert!(
            !crate::lattice_maps::view(&plugin.params).can_undo,
            "undo never reaches past the selected map to slot 1's edits"
        );
        // Each row of the editor's list deletes its own map, so a delete can
        // name a map that is not selected.
        crate::lattice_maps::edit(&plugin.params, context.as_ref(), MapEdit::Delete(0));
    });
    device.run(80, vec![], false);
    device.wrapper().test_inspect_plugin(|plugin| {
        let playback = *plugin.params.map_playback.lock();
        assert_eq!(playback.selected, 2, "deleting another map leaves the selection alone");
        let moved = crate::lattice_maps::offsets(&plugin.params).total();
        let kept = LatticeMap { position: moved, ..changed };
        assert_eq!(playback.map, Some(kept), "new attacks keep the selected map");
        let names = crate::lattice_maps::view(&plugin.params).names;
        assert!(names.iter().all(|(id, _)| *id != 0), "the deleted map leaves the list");
        crate::lattice_maps::edit(&plugin.params, context.as_ref(), MapEdit::Delete(2));
    });
    device.run(96, vec![], false);
    device.wrapper().test_inspect_plugin(|plugin| {
        let playback = *plugin.params.map_playback.lock();
        assert_eq!(playback.selected, 1, "deleting the last map in order selects the previous");
        let moved = crate::lattice_maps::offsets(&plugin.params).total();
        let neighbour = LatticeMap { position: moved, ..changed };
        assert_eq!(playback.map, Some(neighbour), "new attacks are tuned by the neighbour");
    });
    device.load(saved);
    device.wrapper().test_inspect_plugin(|plugin| {
        let preview = crate::lattice_maps::view(&plugin.params);
        assert!(!preview.editing() && !preview.can_undo, "undo never reaches a restored project");
        assert_eq!(preview.playback.map, Some(map));
        assert!(
            preview.unadopted.is_some(),
            "audio still holds the edited shape until its next callback"
        );
    });
    device.run(128, vec![], false);
    device.wrapper().test_inspect_plugin(|plugin| {
        assert_eq!(plugin.params.map_playback.lock().map, Some(map));
    });
    // Old or partial presets must not retain lanes from the current preset,
    // and repaired plain values must agree with host and editor readback.
    let mut partial = device.save();
    partial.params.insert("map-fifths".into(), harmonigraph_clap::plugin::ParamValue::I32(50));
    partial.params.insert("map-thirds".into(), harmonigraph_clap::plugin::ParamValue::I32(-50));
    partial.params.insert("map-follow".into(), harmonigraph_clap::plugin::ParamValue::I32(1));
    for id in [
        "map-fifths-extension",
        "map-thirds-extension",
        "map-sevenths-extension",
        "tuning-engine",
        "lattice-map",
    ] {
        partial.params.remove(id);
    }
    device.load(partial);
    device.run(192, vec![], false);
    device.wrapper().test_inspect_plugin(|plugin| {
        let preview = crate::lattice_maps::view(&plugin.params);
        // The instance was in Lattice Map on slot 1: neither leaks in.
        assert_eq!(preview.playback.engine, TuningEngine::Off);
        assert_eq!(preview.playback.selected, 0);
        assert_eq!(preview.offsets.extension, harmonigraph_core::LatticePos::ORIGIN);
        assert_eq!(preview.playback.offset, harmonigraph_core::LatticePos::new(9, -9, -9));
        assert_eq!(plugin.params.map_playback.lock().offset, preview.playback.offset);
        assert_eq!(preview.playback.follow, harmonigraph_core::lattice_map::Follow::Thirds);
    });
    let repaired = device.save();
    assert!(matches!(repaired.params["map-follow"], harmonigraph_clap::plugin::ParamValue::I32(1)));
    assert!(matches!(repaired.params["map-fifths"], harmonigraph_clap::plugin::ParamValue::I32(9)));
    assert!(matches!(
        repaired.params["map-thirds"],
        harmonigraph_clap::plugin::ParamValue::I32(-9)
    ));
}

/// A restore once stored an integer parameter's value unclamped, while the
/// host's label clamps it. An engine or map index out of range at either end —
/// hand-edited or foreign state, since the plugin never saves one — must load
/// as the value the host shows, in the editor and in the audio alike, and save
/// back as that value. On `main` before the repair, the high side's engines,
/// map label and saved values failed and the low side's saved values failed;
/// the selected slots and the low side's engines already agreed, and stay
/// here as the agreement this pins.
#[test]
fn an_out_of_range_engine_or_map_restores_as_the_value_the_host_shows() {
    use harmonigraph_core::lattice_map::TuningEngine;
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    for (round, (engine, map, shown, decoded, slot)) in [
        (3, 200, "Lattice Map", TuningEngine::LatticeMap, 127),
        (-1, -5, "Pass through", TuningEngine::Off, 0),
    ]
    .into_iter()
    .enumerate()
    {
        let mut state = device.save();
        state
            .params
            .insert("tuning-engine".into(), harmonigraph_clap::plugin::ParamValue::I32(engine));
        state.params.insert("lattice-map".into(), harmonigraph_clap::plugin::ParamValue::I32(map));
        device.load(state);
        device.run(64 * round as i64, vec![], false);
        device.wrapper().test_inspect_plugin(|plugin| {
            let preview = crate::lattice_maps::view(&plugin.params);
            let audio = *plugin.params.map_playback.lock();
            assert_eq!(plugin.params.label(crate::Row::TuningEngine), shown);
            assert_eq!(preview.playback.engine, decoded, "the editor disagrees");
            assert_eq!(audio.engine, decoded, "the audio disagrees");
            let label = plugin.params.label(crate::Row::LatticeMap);
            assert!(label.starts_with(&format!("{} · ", slot + 1)), "the host shows {label}");
            assert_eq!((preview.playback.selected, audio.selected), (slot, slot));
        });
        let repaired = device.save();
        assert!(matches!(
            (&repaired.params["tuning-engine"], &repaired.params["lattice-map"]),
            (harmonigraph_clap::plugin::ParamValue::I32(e), harmonigraph_clap::plugin::ParamValue::I32(m))
                if (*e, *m as usize) == (decoded.index(), slot)
        ));
    }
}

#[test]
fn every_engine_records_a_tuning_edit_at_the_callback_that_applies_it() {
    let _scope = crate::test_scope::enter();
    // Pass through, Adaptive and Lattice Map.
    for engine in 0..3 {
        let (mut device, mut capture) = recorded_device();
        device.activate();
        let mut state = restored(&device, 696.0);
        state
            .params
            .insert("tuning-engine".into(), harmonigraph_clap::plugin::ParamValue::I32(engine));
        device.load(state);
        device.run(0, vec![], false);
        capture.arm();
        device.run(64, vec![], false);
        assert!(device.settings().submit(ConfigEdit::axis(TuningAxis::Three, microcents(690.0))));
        device.run(128, vec![], false);
        let configs: Vec<_> = capture
            .drain_entries()
            .into_iter()
            .filter_map(|entry| match entry {
                harmonigraph_record::Entry::Configuration(config) => Some(config),
                _ => None,
            })
            .collect();
        assert_eq!(configs.len(), 2, "engine {engine}");
        assert_eq!(configs[0].axes[1], 696_000_000);
        assert_eq!(configs[1].axes[1], 690_000_000);
        assert!((configs[0].t - 64.0 / 48000.0).abs() < 1e-9);
        assert!((configs[1].t - 128.0 / 48000.0).abs() < 1e-9);
    }
}

/// Real CLAP automation and project restoration with no editor constructed.
/// This qualifies block-rate capture, not Bitwig or sample-offset automation.
#[test]
fn camera_automation_is_captured_mid_song() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    let initial = [0.7, -0.2, 9.0, 12.25, -7.375];
    let mut saved = device.save();
    for (key, value) in ParamKey::CAMERA.into_iter().zip(initial) {
        saved.params.insert(key.id().into(), ParamValue::F32(value));
    }
    device.load(saved);
    device.activate();
    let restored = device.save();
    for (key, value) in ParamKey::CAMERA.into_iter().zip(initial) {
        assert!((plain(&restored, key) - value).abs() < 1e-5);
    }
    let shared = device.wrapper().test_inspect_plugin(|plugin| plugin.editor_shared.clone());
    let dir =
        std::env::temp_dir().join(format!("harmonigraph-camera-capture-{}", std::process::id()));
    let probe = {
        let mut shared = shared.lock();
        let probe = harmonigraph_record::testing::worker_probe(&shared.take, dir.clone());
        harmonigraph_record::testing::set_renderer_program(
            &shared.take,
            dir.join("absent-renderer"),
        );
        // A stale appearance snapshot must never own the recorded movement.
        shared.ui.picture.appearance.camera.distance = 20.0;
        // A trigger that records on through the rewind below; the fresh
        // Transport stop ends at it instead.
        shared.ui.picture.appearance.render.trigger = harmonigraph_ui::RenderTrigger::OnDisarm;
        shared.take.start(48_000.0, shared.ui.picture.appearance.serialize());
        shared.poll_take_end();
        probe
    };
    let dt = 64.0 / 48_000.0;
    device.run_transport(0, vec![], false, None, Some(transport(12.0, 0)));
    device.run_transport(
        64,
        vec![device.param(ParamKey::CameraDistance, 6.0, 32)],
        false,
        None,
        Some(transport(12.0 + dt, 0)),
    );
    device.run_transport(128, vec![], false, None, Some(transport(10.0, 0)));
    stop(&shared);
    device.finish_notes(192, &[]);
    drop(shared);
    drop(device);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !probe.finished() && !probe.failed() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!probe.failed());
    assert!(probe.finished());
    let path = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|e| e == "take"))
        .unwrap();
    let take = harmonigraph_take::Take::read(&path).unwrap();
    let origin = take.audio_start.unwrap();
    assert_eq!(origin, 12.0, "the take starts where the transport was");
    for (key, value) in ParamKey::CAMERA.into_iter().zip(initial) {
        let record = take.params.iter().find(|p| p.id == key.id()).unwrap();
        assert!((record.value - value).abs() < 1e-5, "{key:?}");
        assert!((record.t - origin).abs() < 1e-8, "camera baseline at the take's start");
    }
    let changed = take
        .params
        .iter()
        .find(|p| p.id == ParamKey::CameraDistance.id() && (p.value - 6.0).abs() < 1e-5)
        .unwrap();
    assert!(
        (changed.t - (12.0 + dt)).abs() < 1e-8,
        "nonzero event offset is sampled at block origin"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn camera_orbit_gestures_reach_the_host_once_per_axis() {
    use harmonigraph_ui::params::ParamBackend;
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    let context = device.editor_host();
    let params = device.wrapper().test_inspect_plugin(|plugin| plugin.params.clone());
    let gestures = std::cell::Cell::default();
    let backend = crate::PluginParamBackend {
        params: &params,
        host: context.as_ref(),
        configuration: params.settings.view(),
        gestures: &gestures,
    };
    let mut events = Vec::new();
    for frame in 0..3 {
        for key in [ParamKey::CameraYaw, ParamKey::CameraPitch] {
            backend.begin_set(key);
            backend.set(key, 0.1 * (frame + 1) as f32);
        }
        events.extend(device.run(frame * 64, vec![], false).attempts);
    }
    for _ in 0..2 {
        for key in ParamKey::CAMERA {
            backend.end_set(key);
        }
    }
    events.extend(device.run(192, vec![], false).attempts);
    for key in [ParamKey::CameraYaw, ParamKey::CameraPitch] {
        let id = device.id(key);
        for kind in [CLAP_EVENT_PARAM_GESTURE_BEGIN, CLAP_EVENT_PARAM_GESTURE_END] {
            assert_eq!(events.iter().filter(|e| e.0 == kind && e.2 == id && e.3).count(), 1);
        }
        assert_eq!(
            events.iter().filter(|e| e.0 == CLAP_EVENT_PARAM_VALUE && e.2 == id && e.3).count(),
            3
        );
    }
    assert_eq!(gestures.get().open().count(), 0);
}

/// Every host gesture the editor holds open, a map-offset lane's as well as a
/// camera axis's, is ended once when the editor stops being seen, and the
/// close after that ends nothing more; a lane's gesture alone is ended by the
/// close. Through the plug-in's CLAP events, so what is counted is what the
/// host receives.
#[test]
fn the_editors_gestures_end_once_when_it_stops_being_seen_or_closes() {
    use harmonigraph_ui::lattice_maps::{MapAxis, MapEdit, MapOffsetLane};
    use harmonigraph_ui::params::ParamBackend;
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    let context = device.editor_host();
    let (params, lock) = device
        .wrapper()
        .test_inspect_plugin(|plugin| (plugin.params.clone(), plugin.editor_shared.clone()));
    let (axis, lane) = (MapAxis::Thirds, MapOffsetLane::Extension);
    let with_backend = |act: &dyn Fn(&crate::PluginParamBackend)| {
        let shared = lock.lock();
        act(&crate::PluginParamBackend {
            params: &params,
            host: context.as_ref(),
            configuration: params.settings.view(),
            gestures: &shared.gestures,
        });
    };
    let lane_id =
        device.id_named(params.table.spec(crate::lattice_maps::lane_row(axis, lane).index()).name);
    let yaw_id = device.id(ParamKey::CameraYaw);
    let count = |events: &[_], kind, id| {
        events.iter().filter(|e: &&(u16, _, u32, bool)| e.0 == kind && e.2 == id && e.3).count()
    };
    let (begins, ends) = (CLAP_EVENT_PARAM_GESTURE_BEGIN, CLAP_EVENT_PARAM_GESTURE_END);

    // A camera drag and a lane drag, then the editor unseen, then closed.
    with_backend(&|backend| {
        backend.begin_set(ParamKey::CameraYaw);
        backend.set(ParamKey::CameraYaw, 0.2);
        backend.edit_lattice_map(MapEdit::BeginOffset(axis, lane));
        backend.edit_lattice_map(MapEdit::Offset(axis, lane, 1));
    });
    let mut events = device.run(0, vec![], false).attempts;
    let open = lock.end_unseen().expect("the editor lock is free");
    crate::editor::gestures::end(open, context.as_ref());
    events.extend(device.run(64, vec![], false).attempts);
    for id in [yaw_id, lane_id] {
        assert_eq!(count(&events, begins, id), 1, "param {id}");
        assert_eq!(count(&events, ends, id), 1, "unseen left {id} open");
    }
    let open = lock.end_window_session();
    crate::editor::gestures::end(open, context.as_ref());
    events.extend(device.run(128, vec![], false).attempts);
    for id in [yaw_id, lane_id] {
        assert_eq!(count(&events, ends, id), 1, "the close ended {id} again");
    }

    // The lane's drag still held at the reveal: its next value begins a new
    // gesture, which the drag's end ends.
    with_backend(&|backend| backend.edit_lattice_map(MapEdit::Offset(axis, lane, 2)));
    with_backend(&|backend| backend.edit_lattice_map(MapEdit::EndOffset(axis, lane)));
    events.extend(device.run(192, vec![], false).attempts);
    assert_eq!(count(&events, begins, lane_id), 2, "a value set with no gesture open");
    assert_eq!(count(&events, ends, lane_id), 2);

    // A lane's gesture alone, begun and never moved, ended by the close.
    with_backend(&|backend| backend.edit_lattice_map(MapEdit::BeginOffset(axis, lane)));
    let open = lock.end_window_session();
    crate::editor::gestures::end(open, context.as_ref());
    events.extend(device.run(256, vec![], false).attempts);
    assert_eq!(count(&events, begins, lane_id), 3, "a lane's begin bypassed the registry");
    assert_eq!(count(&events, ends, lane_id), 3, "the close left a lane open");
}

/// The own window's close button tells the host through `EditorHost::closed`,
/// which must ask for the editor's destruction: Bitwig's show/hide control
/// shows the editor closed after `closed(true)`, and still open after a hide
/// or `closed(false)` (#1609).
#[test]
fn the_close_button_tells_the_host_to_destroy_the_editor() {
    let _scope = crate::test_scope::enter();
    let device = Device::new();
    let host = device.editor_host();
    assert!(host.closed(), "a host with a gui extension is told");
    assert_eq!(device.stats.closed.load(Ordering::Relaxed), 1);
    assert!(device.stats.closed_destroyed.load(Ordering::Relaxed));
}

/// A size the host gives its plugin window before it attaches the editor
/// (CLAP allows `set_size` before `set_parent`) becomes the editor's only
/// where the editor will open in the host's window. Opening in its own, it
/// opens at the size its project saved, which a save keeps
/// (yan-h/egui-plugin-window#12); a platform that offers no own window takes
/// the host's size either way.
#[test]
fn a_host_size_before_attach_is_the_editors_only_where_it_opens_docked() {
    let _scope = crate::test_scope::enter();
    let device = Device::new();
    let gui = unsafe {
        &*(((*device.plugin).get_extension.unwrap())(device.plugin, CLAP_EXT_GUI.as_ptr())
            .cast::<clap_plugin_gui>())
    };
    let state = device.wrapper().test_inspect_plugin(|plugin| plugin.params.editor_state.clone());
    let (mut api, mut floating) = (ptr::null(), false);
    unsafe {
        assert!((gui.get_preferred_api.unwrap())(device.plugin, &mut api, &mut floating));
        assert!((gui.create.unwrap())(device.plugin, api, false));
    }
    let set_size = |(width, height): (u32, u32)| {
        assert!(unsafe { (gui.set_size.unwrap())(device.plugin, width, height) });
    };
    let size = || {
        let (mut width, mut height) = (0, 0);
        assert!(unsafe { (gui.get_size.unwrap())(device.plugin, &mut width, &mut height) });
        (width, height)
    };
    let saved = size();
    let host = (saved.0 + 160, saved.1 + 90);

    state.set_docked(false);
    set_size(host);
    let own = cfg!(target_os = "macos");
    assert_eq!(size(), if own { saved } else { host }, "the project's saved size stays");

    state.set_docked(true);
    let host = (host.0 + 40, host.1 + 30);
    set_size(host);
    assert_eq!(size(), host, "docked, the host's size is the editor's");
}
