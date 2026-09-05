//! The exported Harmonigraph CLAP factory, real host state and params vtables,
//! and real structured UI bridge. No editor is opened and no probe is enabled.
use super::*;
use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::events::*;
use clap_sys::ext::params::{clap_host_params, clap_plugin_params, CLAP_EXT_PARAMS};
use clap_sys::ext::state::{clap_host_state, clap_plugin_state, CLAP_EXT_STATE};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::*;
use clap_sys::stream::{clap_istream, clap_ostream};
use clap_sys::version::CLAP_VERSION;
use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Host {
    dirty: AtomicUsize,
}
unsafe extern "C" fn extension(_: *const clap_host, id: *const c_char) -> *const c_void {
    let id = unsafe { CStr::from_ptr(id) };
    if id == CLAP_EXT_PARAMS {
        &PARAMS as *const _ as *const c_void
    } else if id == CLAP_EXT_STATE {
        &STATE as *const _ as *const c_void
    } else {
        ptr::null()
    }
}
unsafe extern "C" fn dirty(host: *const clap_host) {
    unsafe { &*((*host).host_data.cast::<Host>()) }.dirty.fetch_add(1, Ordering::Relaxed);
}
unsafe extern "C" fn request(_: *const clap_host) {}
unsafe extern "C" fn rescan(_: *const clap_host, _: u32) {}
unsafe extern "C" fn clear(_: *const clap_host, _: u32, _: u32) {}
static PARAMS: clap_host_params =
    clap_host_params { rescan: Some(rescan), clear: Some(clear), request_flush: Some(request) };
static STATE: clap_host_state = clap_host_state { mark_dirty: Some(dirty) };

#[derive(Clone, Copy)]
enum Input {
    Param(clap_event_param_value),
    Mod(clap_event_param_mod),
    Note(clap_event_note),
    Tuning(clap_event_note_expression),
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
    values: Vec<(u32, f64)>,
    reject: bool,
}
unsafe extern "C" fn output_push(
    output: *const clap_output_events,
    event: *const clap_event_header,
) -> bool {
    let sink = unsafe { &mut *((*output).ctx.cast::<Sink>()) };
    if sink.reject {
        return false;
    }
    if unsafe { (*event).type_ } == CLAP_EVENT_PARAM_VALUE {
        let event = unsafe { &*event.cast::<clap_event_param_value>() };
        assert!(sink.values.len() < sink.values.capacity());
        sink.values.push((event.param_id, event.value));
    }
    true
}
struct Device {
    plugin: *const clap_plugin,
    _host: Box<clap_host>,
    stats: Box<Host>,
    active: bool,
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
            request_callback: Some(request),
        });
        let factory =
            unsafe { (crate::clap_entry.get_factory.unwrap())(CLAP_PLUGIN_FACTORY_ID.as_ptr()) }
                .cast::<clap_plugin_factory>();
        let plugin = unsafe {
            ((*factory).create_plugin.unwrap())(factory, &*host, c"com.yan-h.harmonigraph".as_ptr())
        };
        assert!(!plugin.is_null());
        assert!(unsafe { ((*plugin).init.unwrap())(plugin) });
        Self { plugin, _host: host, stats, active: false }
    }
    fn wrapper(&self) -> &nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph> {
        unsafe { &*((*self.plugin).plugin_data.cast()) }
    }
    fn mailbox(&self) -> std::sync::Arc<ConfigurationMailbox> {
        self.wrapper().configuration_handle().unwrap()
    }
    fn activate(&mut self) {
        assert!(unsafe { ((*self.plugin).activate.unwrap())(self.plugin, 48000.0, 1, 64) });
        assert!(unsafe { ((*self.plugin).start_processing.unwrap())(self.plugin) });
        self.active = true;
    }
    fn params(&self) -> &clap_plugin_params {
        unsafe {
            &*(((*self.plugin).get_extension.unwrap())(self.plugin, CLAP_EXT_PARAMS.as_ptr())
                .cast())
        }
    }
    fn id(&self, key: ParamKey) -> u32 {
        for index in 0..unsafe { (self.params().count.unwrap())(self.plugin) } {
            let mut info: clap_sys::ext::params::clap_param_info = unsafe { std::mem::zeroed() };
            assert!(unsafe { (self.params().get_info.unwrap())(self.plugin, index, &mut info) });
            if unsafe { CStr::from_ptr(info.name.as_ptr()) }.to_bytes()
                == key.host_name().as_bytes()
            {
                return info.id;
            }
        }
        panic!("missing tuning parameter");
    }
    fn param(&self, key: ParamKey, cents: f32, time: u32) -> Input {
        let values = crate::HarmonigraphParams::default();
        Input::Param(clap_event_param_value {
            header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, time),
            param_id: self.id(key),
            cookie: ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value: f64::from(values.param_for(key).preview_normalized(cents)),
        })
    }
    fn get(&self, key: ParamKey) -> f64 {
        let mut value = -1.0;
        assert!(unsafe {
            (self.params().get_value.unwrap())(self.plugin, self.id(key), &mut value)
        });
        value
    }
    fn load(&self, state: PluginState, gui: bool) {
        if gui {
            self.wrapper().set_state_object_from_gui(state);
            return;
        }
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
        let input = clap_input_events {
            ctx: (&events as *const Vec<Input>).cast_mut().cast(),
            size: Some(input_size),
            get: Some(input_get),
        };
        let mut sink = Sink { values: Vec::with_capacity(128), reject };
        let output = clap_output_events {
            ctx: (&mut sink as *mut Sink).cast(),
            try_push: Some(output_push),
        };
        let mut left = [0.0_f32; 64];
        let mut right = [0.0_f32; 64];
        let mut channels = [left.as_mut_ptr(), right.as_mut_ptr()];
        let mut audio = clap_audio_buffer {
            data32: channels.as_mut_ptr(),
            data64: ptr::null_mut(),
            channel_count: 2,
            latency: 0,
            constant_mask: 0,
        };
        let process = clap_process {
            steady_time: steady,
            frames_count: 64,
            transport: ptr::null(),
            audio_inputs: &audio,
            audio_outputs: &mut audio,
            audio_inputs_count: 1,
            audio_outputs_count: 1,
            in_events: &input,
            out_events: &output,
        };
        let status = unsafe { ((*self.plugin).process.unwrap())(self.plugin, &process) };
        (status, sink)
    }
    fn flush(&self, events: Vec<Input>) {
        let input = clap_input_events {
            ctx: (&events as *const Vec<Input>).cast_mut().cast(),
            size: Some(input_size),
            get: Some(input_get),
        };
        let mut sink = Sink { values: Vec::with_capacity(128), reject: false };
        let output = clap_output_events {
            ctx: (&mut sink as *mut Sink).cast(),
            try_push: Some(output_push),
        };
        unsafe {
            (self.params().flush.unwrap())(self.plugin, &input, &output);
        }
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
fn restored(device: &Device, fifth: f32) -> PluginState {
    let mut state = device.wrapper().get_state_object();
    state.params.insert(ParamKey::Three.id().into(), ParamValue::F32(fifth));
    state.fields.insert(
        MUSICAL_SETTINGS.into(),
        serde_json::to_string(&MusicalSettings {
            meantone: false,
            marvel: false,
            meantone_auto: false,
            marvel_auto: false,
            learning: false,
        })
        .unwrap(),
    );
    state
}
fn plain(state: &PluginState, key: ParamKey) -> f32 {
    match state.params[key.id()] {
        ParamValue::F32(value) => value,
        _ => panic!("float parameter"),
    }
}

#[test]
fn active_restore_without_callbacks_has_coherent_save_readback_and_ordered_adoption() {
    for gui in [false, true] {
        let mut device = Device::new();
        device.activate();
        let mailbox = device.mailbox();
        device.load(restored(&device, 690.0), gui);
        let first = mailbox.visible().0.applied_id;
        let ui = mailbox
            .submit(packet(ConfigEdit::axis(1, harmonigraph_core::tuning::microcents(695.0))))
            .unwrap();
        device.load(restored(&device, 705.0), gui);
        assert!(first < ui && ui < mailbox.visible().0.applied_id);
        assert_eq!(plain(&device.save(), ParamKey::Three), 705.0);
        assert_eq!(plain(&device.wrapper().get_state_object(), ParamKey::Three), 705.0);
        let expected = crate::HarmonigraphParams::default().three.preview_normalized(705.0);
        assert_eq!(device.get(ParamKey::Three), f64::from(expected));
        let sink = device.run(1000, vec![], false);
        assert!(
            sink.values.is_empty(),
            "superseded UI values must not overwrite the newer restore"
        );
        assert!(!mailbox.visible().1);
        assert_eq!(mailbox.visible().0.raw[1], 705.0);
        let after = mailbox
            .submit(packet(ConfigEdit::axis(1, harmonigraph_core::tuning::microcents(700.0))))
            .unwrap();
        device.run(1064, vec![], true);
        assert_eq!(
            mailbox.visible().0.applied_id,
            after,
            "host rejection cannot undo local commit"
        );
        assert_eq!(mailbox.visible().0.raw[1], 700.0);
        unsafe {
            ((*device.plugin).on_main_thread.unwrap())(device.plugin);
        }
        assert!(device.stats.dirty.load(Ordering::Relaxed) > 0);
    }
}

#[test]
fn queued_unlock_and_distinct_ui_ids_survive_same_value_host_automation_and_flush() {
    let mut device = Device::new();
    device.flush(vec![device.param(ParamKey::Three, 690.0, 23)]);
    let mailbox = device.mailbox();
    let before = mailbox.visible().0;
    assert_eq!(before.raw[1], 700.0, "untimed flush has not invented a boundary");
    device.activate();
    device.run(500, vec![], false);
    assert_eq!(mailbox.visible().0.effective_sample, 500);
    let first = mailbox
        .submit(packet(ConfigEdit::unlock(
            harmonigraph_core::Comma::Syntonic,
            harmonigraph_core::tuning::microcents(390.0),
        )))
        .unwrap();
    let second = mailbox
        .submit(packet(ConfigEdit::axis(1, harmonigraph_core::tuning::microcents(700.0))))
        .unwrap();
    assert_ne!(first, second);
    device.run(
        564,
        vec![
            device.param(ParamKey::Five, 390.0, 7),
            device.param(ParamKey::Seven, 975.0, 7),
            device.param(ParamKey::Three, 699.0, 31),
        ],
        false,
    );
    let snapshot = mailbox.visible().0;
    assert_eq!(snapshot.applied_id, second);
    assert_eq!(snapshot.effective_sample, 595);
    assert!(!view(snapshot, false).resolved.modes.tempered.syntonic);
    device.flush(vec![device.param(ParamKey::Three, 697.0, 44)]);
    assert_eq!(mailbox.visible().0.effective_sample, 595);
    device.run(628, vec![], false);
    assert_eq!(mailbox.visible().0.effective_sample, 628);
    assert_eq!(mailbox.visible().0.raw[1], 697.0);
}

#[test]
fn real_same_sample_initial_tuning_is_in_learning_before_any_gui_drain() {
    let mut device = Device::new();
    device.activate();
    let mailbox = device.mailbox();
    mailbox.submit(packet(ConfigEdit { learning: Some(true), ..Default::default() })).unwrap();
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
        if key == 64 {
            events.push(Input::Tuning(clap_event_note_expression {
                header: header::<clap_event_note_expression>(CLAP_EVENT_NOTE_EXPRESSION, 0),
                expression_id: CLAP_NOTE_EXPRESSION_TUNING,
                note_id: 64,
                port_index: 0,
                channel: 0,
                key: 64,
                value: f64::from(harmonigraph_core::tuning::FIVE_JUST - 400.0) / 100.0,
            }));
        }
    }
    device.run(0, events, false);
    let learned = mailbox.visible().0;
    assert!(!view(learned, false).resolved.modes.tempered.syntonic);
    assert!((learned.raw[2] - harmonigraph_core::tuning::FIVE_JUST).abs() < 0.001);
    device.run(64, vec![], false);
    assert_eq!(mailbox.visible().0.revision, learned.revision);
}

#[test]
fn control_budget_retains_original_ui_boundary_and_host_offsets() {
    let mut device = Device::new();
    device.activate();
    let mailbox = device.mailbox();
    let mut last = 0;
    for value in 0..10 {
        last = mailbox.submit(packet(ConfigEdit::axis(0, value * 1_000_000))).unwrap();
    }
    device.run(1000, vec![], false);
    assert!(mailbox.visible().1, "eight insert/apply pairs exhaust the enclosing 16-work budget");
    assert_eq!(mailbox.visible().0.effective_sample, 1000);
    device.run(1064, vec![], false);
    assert_eq!(mailbox.visible().0.applied_id, last);
    assert_eq!(
        mailbox.visible().0.effective_sample,
        1000,
        "continuation must not restamp UI intent"
    );
    assert_eq!(mailbox.visible().0.status, 0);
    let events = (0..12).map(|i| device.param(ParamKey::Three, 690.0 + i as f32, i * 3)).collect();
    device.run(1128, events, false);
    assert_eq!(mailbox.visible().0.effective_sample, 1128 + 21);
    device.run(1192, vec![], false);
    assert_eq!(mailbox.visible().0.effective_sample, 1128 + 33);
    assert_eq!(mailbox.visible().0.raw[1], 701.0);
    assert_eq!(mailbox.visible().0.status, 0, "work exhaustion is not storage exhaustion");
}

#[test]
fn required_marker_capacity_is_independent_of_a_drained_command_queue() {
    let mut device = Device::new();
    device.activate();
    let mailbox = device.mailbox();
    for block in 0..16 {
        let events = (0..8).map(|i| device.param(ParamKey::Three, 690.0 + i as f32, i)).collect();
        device.run(block * 64, events, false);
        assert_eq!(mailbox.visible().0.status, 0);
        assert!(!mailbox.visible().1);
    }
    let retained = mailbox.visible().0;
    device.run(1024, vec![device.param(ParamKey::Three, 710.0, 0)], false);
    assert_eq!(
        mailbox.visible().0.status & 2,
        2,
        "the 129th required marker reaches the actual timeline bound"
    );
    assert_eq!(mailbox.visible().0.raw, retained.raw);
    unsafe {
        ((*device.plugin).reset.unwrap())(device.plugin);
    }
    assert_eq!(
        mailbox.visible().0.revision,
        retained.revision,
        "reset keeps the coherent musical revision"
    );
    device.run(1088, vec![device.param(ParamKey::Three, 710.0, 0)], false);
    assert_eq!(mailbox.visible().0.status, 0);
    assert_eq!(mailbox.visible().0.raw[1], 710.0);
}

#[test]
fn restore_preserves_held_modulation_without_a_new_host_mod_event() {
    let mut device = Device::new();
    device.activate();
    let mailbox = device.mailbox();
    let modulation = Input::Mod(clap_event_param_mod {
        header: header::<clap_event_param_mod>(CLAP_EVENT_PARAM_MOD, 0),
        param_id: device.id(ParamKey::Three),
        cookie: ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        amount: 0.9,
    });
    device.run(0, vec![modulation], false);
    assert_ne!(mailbox.visible().0.raw[1], mailbox.visible().0.unmodulated[1]);
    device.load(restored(&device, 690.0), false);
    let shadow = mailbox.visible().0;
    device.run(64, vec![], false);
    assert_eq!(mailbox.visible().0.raw, shadow.raw);
    assert_eq!(mailbox.visible().0.normalized, shadow.normalized);
    assert_eq!(mailbox.visible().0.modulation[1], 0.9);
    assert_ne!(mailbox.visible().0.raw[1], 690.0);
    assert_eq!(plain(&device.save(), ParamKey::Three), 690.0);
}

#[test]
fn restore_slots_and_command_queue_refuse_without_losing_accepted_state() {
    let mut device = Device::new();
    let mailbox = device.mailbox();
    device.load(restored(&device, 690.0), false);
    device.load(restored(&device, 695.0), false);
    let accepted = mailbox.visible().0;
    assert!(
        !device.try_load(restored(&device, 705.0)),
        "both prepared slots remain owned until adoption"
    );
    assert_eq!(mailbox.visible().0, accepted);
    for _ in 0..126 {
        mailbox.submit(packet(ConfigEdit::default())).unwrap();
    }
    assert_eq!(mailbox.submit(packet(ConfigEdit::default())), Err(SubmitError::Full));
    assert_eq!(plain(&device.save(), ParamKey::Three), 695.0);
    device.activate();
    device.run(0, vec![], false);
    assert!(
        device.try_load(restored(&device, 705.0)),
        "applied restore slots are reusable off audio"
    );
    assert_eq!(plain(&device.save(), ParamKey::Three), 705.0);
}

#[test]
fn one_owned_input_pool_reaches_2048_and_refuses_growth_while_work_is_retained() {
    let mut device = Device::new();
    device.activate();
    let events = (0..2048).map(|_| device.param(ParamKey::Three, 690.0, 0)).collect();
    device.run(0, events, false);
    assert_eq!(device.mailbox().visible().0.status, 0);
    let (status, _) = device.run_status(64, vec![device.param(ParamKey::Three, 695.0, 0)], false);
    assert_eq!(status, CLAP_PROCESS_ERROR);
    assert_eq!(device.mailbox().visible().0.status & 2, 2);
}

#[derive(Default)]
struct Legacy {
    params: std::sync::Arc<crate::HarmonigraphParams>,
}
impl Plugin for Legacy {
    const NAME: &'static str = "Legacy opt-out";
    const VENDOR: &'static str = "test";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = "1";
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[];
    type SysExMessage = ();
    type BackgroundTask = ();
    fn params(&self) -> std::sync::Arc<dyn Params> {
        self.params.clone()
    }
    fn process(
        &mut self,
        _: &mut Buffer,
        _: &mut AuxiliaryBuffers,
        _: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        ProcessStatus::Normal
    }
}
impl ClapPlugin for Legacy {
    const CLAP_ID: &'static str = "test.legacy";
    const CLAP_DESCRIPTION: Option<&'static str> = None;
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[];
}
#[test]
fn opt_out_wrapper_and_non_clap_plugin_construction_have_no_configuration_owner() {
    let device = Device::new();
    let legacy = unsafe { nice_plug::wrapper::clap::Wrapper::<Legacy>::new(&*device._host) };
    assert!(legacy.configuration_handle().is_none());
    let plugin = crate::Harmonigraph::default();
    assert!(
        plugin.configuration.is_none(),
        "VST/standalone initialization has no CLAP-only owner or callback work"
    );
    assert!(plugin.params.configuration.get().is_none());
}
