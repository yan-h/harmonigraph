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
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

#[derive(Default)]
struct Host {
    reject_one_end: AtomicBool,
    dirty: AtomicUsize,
    callbacks: AtomicUsize,
    plugin: AtomicUsize,
    cache: AtomicU64,
    pause_value: AtomicBool,
    value_entered: AtomicBool,
    resume_value: AtomicBool,
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
unsafe extern "C" fn callback(host: *const clap_host) {
    unsafe { &*((*host).host_data.cast::<Host>()) }.callbacks.fetch_add(1, Ordering::Relaxed);
}
unsafe extern "C" fn rescan(host: *const clap_host, _: u32) {
    let stats = unsafe { &*((*host).host_data.cast::<Host>()) };
    let plugin = stats.plugin.load(Ordering::Relaxed) as *const clap_plugin;
    if !plugin.is_null() {
        let wrapper = unsafe {
            &*((*plugin)
                .plugin_data
                .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
        };
        stats.cache.store(
            f64::from(wrapper.configuration_handle().unwrap().visible().0.normalized[1]).to_bits(),
            Ordering::Relaxed,
        );
    }
}
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
    Transport(clap_event_transport),
}
impl Input {
    fn header(&self) -> &clap_event_header {
        match self {
            Self::Param(e) => &e.header,
            Self::Mod(e) => &e.header,
            Self::Note(e) => &e.header,
            Self::Tuning(e) => &e.header,
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
    reject_kind: Option<u16>,
    attempts: Vec<(u16, u32, u32, bool)>,
    host: usize,
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
    let reject_end = header.type_ == CLAP_EVENT_PARAM_GESTURE_END
        && sink.host != 0
        && unsafe { &*(sink.host as *const Host) }.reject_one_end.swap(false, Ordering::AcqRel);
    let accepted = !sink.reject && sink.reject_kind != Some(header.type_) && !reject_end;
    assert!(sink.attempts.len() < sink.attempts.capacity());
    sink.attempts.push((header.type_, header.time, id, accepted));
    if !accepted {
        return false;
    }
    if header.type_ == CLAP_EVENT_PARAM_VALUE {
        let event = unsafe { &*event.cast::<clap_event_param_value>() };
        if sink.host != 0 {
            let host = unsafe { &*(sink.host as *const Host) };
            if host.pause_value.swap(false, Ordering::AcqRel) {
                host.value_entered.store(true, Ordering::Release);
                while !host.resume_value.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
            }
            host.cache.store(event.value.to_bits(), Ordering::Relaxed);
        }
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
            request_callback: Some(callback),
        });
        let factory =
            unsafe { (crate::clap_entry.get_factory.unwrap())(CLAP_PLUGIN_FACTORY_ID.as_ptr()) }
                .cast::<clap_plugin_factory>();
        let plugin = unsafe {
            ((*factory).create_plugin.unwrap())(factory, &*host, c"com.yan-h.harmonigraph".as_ptr())
        };
        stats.plugin.store(plugin as usize, Ordering::Relaxed);
        assert!(!plugin.is_null());
        assert!(unsafe { ((*plugin).init.unwrap())(plugin) });
        let device = Self { plugin, _host: host, stats, active: false };
        // These fixtures isolate configuration/recording consumers, including
        // intentionally retained timeline markers. Musical sequencing has its
        // own factory-default acceptance fixtures under performance/tests.
        device.wrapper().test_with_plugin(|plugin| {
            plugin.aggregation.as_mut().unwrap().test_aggregation = true;
        });
        device
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
    // Called explicitly after a fixture's original assertions. These fixtures
    // study configuration/recording, so finish their accepted physical gestures
    // before destruction; joined unknown-wire debt must otherwise remain owned.
    fn finish_notes(&mut self, mut raw: i64, notes: &[(i32, i16)]) {
        let snapshot = |device: &Self| {
            device.wrapper().test_inspect_plugin(|plugin| {
                plugin.aggregation.as_ref().unwrap().direct.test_snapshot()
            })
        };
        let held = snapshot(self).held;
        let output = self.run(
            raw,
            notes.iter().map(|&(id, key)| note(id, key, 0, CLAP_EVENT_NOTE_OFF)).collect(),
            false,
        );
        assert!(
            output
                .attempts
                .iter()
                .filter(|event| event.0 == CLAP_EVENT_NOTE_OFF && event.3)
                .count()
                >= held
        );
        for _ in 0..16 {
            raw += 64;
            // Recording fixtures can retain an unsounded On/Off pair behind
            // unknown pedal state. A real falling Stop edge cancels that pair
            // after the physical Offs above; an Off alone is not cancellation.
            let mut stopped = transport(0.0, 0);
            stopped.flags &= !CLAP_TRANSPORT_IS_PLAYING;
            self.run_transport(raw, vec![], false, None, Some(stopped));
            let state = snapshot(self);
            if (state.held, state.pending, state.captures, state.lives, state.journal)
                == (0, 0, 0, 0, 0)
            {
                return;
            }
        }
        panic!("accepted gesture release must finish bounded ownership: {:?}", snapshot(self));
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
        self.run_reject_kind(steady, events, reject, None)
    }
    fn run_reject_kind(
        &self,
        steady: i64,
        events: Vec<Input>,
        reject: bool,
        reject_kind: Option<u16>,
    ) -> (clap_process_status, Sink) {
        self.run_transport(steady, events, reject, reject_kind, None)
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
        let mut sink = Sink {
            values: Vec::with_capacity(128),
            reject,
            reject_kind,
            attempts: Vec::with_capacity(1024),
            host: (&*self.stats as *const Host) as usize,
        };
        let output = clap_output_events {
            ctx: (&mut sink as *mut Sink).cast(),
            try_push: Some(output_push),
        };
        let mut left = [0.0_f32; 64];
        let mut right = [0.0_f32; 64];
        let mut channels = [left.as_mut_ptr(), right.as_mut_ptr()];
        let mut side_left = [0.0_f32; 64];
        let mut side_right = [0.0_f32; 64];
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
    fn flush(&self, events: Vec<Input>) {
        let input = clap_input_events {
            ctx: (&events as *const Vec<Input>).cast_mut().cast(),
            size: Some(input_size),
            get: Some(input_get),
        };
        let mut sink = Sink {
            values: Vec::with_capacity(128),
            reject: false,
            attempts: Vec::with_capacity(1024),
            ..Default::default()
        };
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
            ..Default::default()
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
    let _scope = crate::test_scope::enter();
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
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.flush(vec![device.param(ParamKey::Three, 690.0, 23)]);
    let mailbox = device.mailbox();
    let before = mailbox.visible().0;
    assert_eq!(before.raw[1], 700.0, "untimed flush has not invented a boundary");
    device.activate();
    device.run(500, vec![], false);
    assert_eq!(
        mailbox.visible().0.effective_sample,
        564,
        "an edit adopted at the boundary of [500,564) is effective at the next one"
    );
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
    assert_eq!(snapshot.effective_sample, 628, "at7 and at31 share this block's boundary");
    assert!(!view(snapshot, false).resolved.modes.tempered.syntonic);
    device.flush(vec![device.param(ParamKey::Three, 697.0, 44)]);
    assert_eq!(mailbox.visible().0.effective_sample, 628, "an untimed flush applies nothing yet");
    device.run(628, vec![], false);
    assert_eq!(mailbox.visible().0.effective_sample, 692);
    assert_eq!(mailbox.visible().0.raw[1], 697.0);
}

#[test]
fn real_same_sample_initial_tuning_is_in_learning_before_any_gui_drain() {
    let _scope = crate::test_scope::enter();
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
    device.finish_notes(128, &[(60, 60), (64, 64), (67, 67)]);
}

#[test]
fn restore_preserves_held_modulation_without_a_new_host_mod_event() {
    let _scope = crate::test_scope::enter();
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
fn a_refused_restore_and_a_full_command_queue_keep_accepted_state() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    let mailbox = device.mailbox();
    device.load(restored(&device, 690.0), false);
    device.load(restored(&device, 695.0), false);
    let accepted = mailbox.visible().0;
    assert!(
        !device.try_load(restored(&device, 705.0)),
        "the prepared restore is owned until adoption"
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
        "an applied restore is repeatable off audio"
    );
    assert_eq!(plain(&device.save(), ParamKey::Three), 705.0);
    // The assertions deliberately leave accepted commands unfinished. Reset
    // clears them before the remaining ones drain through real callbacks.
    let accepted_command = mailbox.accepted_command.load(Ordering::Acquire);
    unsafe {
        ((*device.plugin).reset.unwrap())(device.plugin);
    }
    for block in 1..=32 {
        device.run(block * 64, vec![], false);
        if !mailbox.visible().1 {
            break;
        }
    }
    assert!(!mailbox.visible().1);
    assert_eq!(mailbox.visible().0.applied_id, accepted_command);
    assert_eq!(mailbox.visible().0.raw[1], 705.0);
    assert_eq!(
        mailbox.visible().0.status,
        8,
        "the deliberately rejected submission remains visible"
    );
}

#[test]
fn one_owned_input_pool_reaches_2048_in_a_callback_and_refuses_growth_past_it() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    let events = (0..2048).map(|_| device.param(ParamKey::Three, 690.0, 0)).collect();
    device.run(0, events, false);
    assert_eq!(device.mailbox().visible().0.status, 0);
    let (status, _) = device.run_status(
        64,
        (0..2049).map(|_| device.param(ParamKey::Three, 695.0, 0)).collect(),
        false,
    );
    assert_eq!(status, CLAP_PROCESS_ERROR);
    assert_eq!(device.mailbox().visible().0.status & 2, 2);
    // Exercise the host's recovery boundary after proving capacity
    // exhaustion, so this fixture does not leave an unresolved global owner.
    unsafe {
        ((*device.plugin).reset.unwrap())(device.plugin);
    }
    device.run(128, vec![], false);
    assert_eq!(device.mailbox().visible().0.status, 0);
}

#[test]
fn destroyed_configuration_owners_settle_without_reset_or_another_callback() {
    let _scope = crate::test_scope::enter();
    let mut retained = Vec::new();
    for commands in [false, true] {
        let mut device = Device::new();
        if commands {
            let mailbox = device.mailbox();
            device.load(restored(&device, 690.0), false);
            device.load(restored(&device, 695.0), false);
            for _ in 0..126 {
                mailbox.submit(packet(ConfigEdit::default())).unwrap();
            }
            device.activate();
            device.run(0, vec![], false);
            device.load(restored(&device, 705.0), false);
            assert!(mailbox.visible().1, "accepted configuration is still owned by the wrapper");
        } else {
            device.activate();
            device.run(
                0,
                (0..2048).map(|_| device.param(ParamKey::Three, 690.0, 0)).collect(),
                false,
            );
            let (status, _) = device.run_status(
                64,
                (0..2049).map(|_| device.param(ParamKey::Three, 695.0, 0)).collect(),
                false,
            );
            assert_eq!(status, CLAP_PROCESS_ERROR);
            assert_eq!(device.mailbox().visible().0.status & 2, 2);
        }
        drop(device);
        let counts = (0usize, 0usize, 0usize);
        retained.push(counts);
    }
    assert_eq!(retained, [(0,0,0), (0,0,0)], "actual destruction settles both states without Reset, a rescue callback, or another instance's main-thread service");
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
    let _scope = crate::test_scope::enter();
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

// The fixture invokes only CLAP's allowed concurrent main/audio entrypoints;
// activation, destruction, and ordinary processing remain exclusively owned.
unsafe impl Sync for Device {}

#[test]
fn accepted_auto_restore_has_one_preview_and_save_before_and_after_adoption() {
    let _scope = crate::test_scope::enter();
    for gui in [false, true] {
        let mut device = Device::new();
        device.activate();
        let mailbox = device.mailbox();
        let mut state = restored(&device, 700.0);
        state.fields.insert(
            MUSICAL_SETTINGS.into(),
            serde_json::to_string(&MusicalSettings {
                meantone: false,
                marvel: false,
                meantone_auto: true,
                marvel_auto: true,
                learning: false,
                ..Default::default()
            })
            .unwrap(),
        );
        device.load(state, gui);
        let preview = view(mailbox.visible().0, true).resolved;
        let saved: MusicalSettings =
            serde_json::from_str(&device.save().fields[MUSICAL_SETTINGS]).unwrap();
        assert_eq!(saved.modes(), preview.modes);
        let saved_gui: MusicalSettings =
            serde_json::from_str(&device.wrapper().get_state_object().fields[MUSICAL_SETTINGS])
                .unwrap();
        assert_eq!(saved_gui.modes(), preview.modes);
        device.run(0, vec![], false);
        assert_eq!(view(mailbox.visible().0, false).resolved.modes, preview.modes);
    }
}

#[test]
fn a_pending_restore_keeps_fault_and_refusal_status_visible() {
    let _scope = crate::test_scope::enter();
    let device = Device::new();
    let mailbox = device.mailbox();
    device.load(restored(&device, 690.0), false);
    let mut applied = mailbox.published.load();
    applied.status = 2;
    mailbox.published.publish(applied);
    for _ in 0..127 {
        mailbox.submit(packet(ConfigEdit::default())).unwrap();
    }
    assert_eq!(mailbox.submit(packet(ConfigEdit::default())), Err(SubmitError::Full));
    assert_eq!(mailbox.visible().0.status & 10, 10);
    assert_eq!(mailbox.visible().0.unmodulated[1], 690.0);
}

#[test]
fn suspended_gui_restore_requests_main_thread_without_processing() {
    let _scope = crate::test_scope::enter();
    let device = Device::new();
    let before = device.stats.callbacks.load(Ordering::Relaxed);
    device.load(restored(&device, 690.0), true);
    assert!(device.stats.callbacks.load(Ordering::Relaxed) > before);
    unsafe {
        ((*device.plugin).on_main_thread.unwrap())(device.plugin);
    }
    assert!(device.stats.dirty.load(Ordering::Relaxed) > 0);
}

#[test]
fn successful_old_notification_after_restore_rescan_retains_another_rescan() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    let mailbox = device.mailbox();
    mailbox.submit(packet(ConfigEdit::axis(1, 690_000_000))).unwrap();
    let restore = restored(&device, 705.0);
    device.stats.pause_value.store(true, Ordering::Release);
    std::thread::scope(|scope| {
        let audio = scope.spawn(|| device.run(0, vec![], false).values);
        while !device.stats.value_entered.load(Ordering::Acquire) {
            std::thread::yield_now();
        }
        device.load(restore, true);
        unsafe {
            ((*device.plugin).on_main_thread.unwrap())(device.plugin);
        }
        assert!(!mailbox.dirty.load(Ordering::Acquire));
        device.stats.resume_value.store(true, Ordering::Release);
        audio.join().unwrap();
    });
    assert!(
        mailbox.dirty.load(Ordering::Acquire),
        "successful stale output must leave a new rescan obligation"
    );
    unsafe {
        ((*device.plugin).on_main_thread.unwrap())(device.plugin);
    }
    assert_eq!(
        f64::from_bits(device.stats.cache.load(Ordering::Relaxed)),
        device.get(ParamKey::Three)
    );
}

#[test]
fn rejected_gesture_end_is_retried_before_another_begin() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    let mailbox = device.mailbox();
    mailbox.submit(packet(ConfigEdit::axis(1, 690_000_000))).unwrap();
    let (_, rejected) =
        device.run_reject_kind(0, vec![], false, Some(CLAP_EVENT_PARAM_GESTURE_END));
    assert!(rejected.attempts.contains(&(
        CLAP_EVENT_PARAM_GESTURE_END,
        0,
        device.id(ParamKey::Three),
        false
    )));
    mailbox.submit(packet(ConfigEdit::axis(1, 695_000_000))).unwrap();
    let accepted = device.run(64, vec![], false);
    let kinds: Vec<_> = accepted
        .attempts
        .iter()
        .filter(|e| e.2 == device.id(ParamKey::Three))
        .map(|e| e.0)
        .collect();
    assert_eq!(
        kinds,
        [
            CLAP_EVENT_PARAM_GESTURE_END,
            CLAP_EVENT_PARAM_GESTURE_BEGIN,
            CLAP_EVENT_PARAM_VALUE,
            CLAP_EVENT_PARAM_GESTURE_END
        ]
    );
    mailbox.submit(packet(ConfigEdit::axis(1, 697_000_000))).unwrap();
    let (_, rejected_begin) =
        device.run_reject_kind(128, vec![], false, Some(CLAP_EVENT_PARAM_GESTURE_BEGIN));
    assert_eq!(
        rejected_begin.attempts.iter().filter(|e| e.2 == device.id(ParamKey::Three)).count(),
        1,
        "an unaccepted begin creates no gesture or value/end output"
    );
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
fn learning_notifications_land_at_the_boundary_and_merge_with_performance() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    let mailbox = device.mailbox();
    mailbox.submit(packet(ConfigEdit { learning: Some(true), ..Default::default() })).unwrap();
    let sink = device.run(
        1000,
        vec![note(10, 60, 0, CLAP_EVENT_NOTE_ON), note(20, 67, 31, CLAP_EVENT_NOTE_ON)],
        false,
    );
    let fifth: Vec<_> = sink
        .attempts
        .iter()
        .filter(|e| e.0 == CLAP_EVENT_PARAM_VALUE && e.2 == device.id(ParamKey::Three))
        .collect();
    assert_eq!(fifth.len(), 1);
    assert_eq!(fifth[0].1, 0, "learning reads the whole block and lands at its boundary");
    assert!(
        sink.attempts.windows(2).all(|events| events[0].1 <= events[1].1),
        "configuration and performance outputs must share chronological order"
    );
    device.finish_notes(1064, &[(10, 60), (20, 67)]);
}
#[test]
fn note_id_only_expression_and_release_match_only_the_addressed_held_note() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    let mailbox = device.mailbox();
    mailbox.submit(packet(ConfigEdit { learning: Some(true), ..Default::default() })).unwrap();
    device.run(
        0,
        vec![note(10, 60, 0, CLAP_EVENT_NOTE_ON), note(20, 67, 0, CLAP_EVENT_NOTE_ON)],
        false,
    );
    mailbox.submit(packet(ConfigEdit::axis(1, 690_000_000))).unwrap();
    device.run(64, vec![id_tuning(10, 1.0)], false);
    assert_eq!(
        mailbox.visible().0.raw[1],
        690.0,
        "only C moved: the remaining interval supplies no fifth evidence"
    );
    let mut release = match note(10, -1, 0, CLAP_EVENT_NOTE_OFF) {
        Input::Note(event) => event,
        _ => unreachable!(),
    };
    release.channel = -1;
    release.port_index = -1;
    device.run(128, vec![Input::Note(release)], false);
    mailbox.submit(packet(ConfigEdit::axis(2, 390_000_000))).unwrap();
    device.run(192, vec![id_tuning(20, 5.0), note(30, 64, 0, CLAP_EVENT_NOTE_ON)], false);
    assert_eq!(
        mailbox.visible().0.raw[2],
        400.0,
        "ID20 survived the ID10 release and now supplies C beside E"
    );
    device.finish_notes(256, &[(20, 67), (30, 64)]);
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

#[test]
fn a_rewind_splits_the_take_and_an_edit_lands_in_the_pass_that_adopts_it() {
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = recorded_device();
    device.activate();
    capture.arm_audio();
    let dir =
        std::env::temp_dir().join(format!("harmonigraph-config-rewind-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("record.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(
        &capture,
        path.clone(),
        Some(harmonigraph_record::AudioSpec { sample_rate: 48000.0, channels: 2 }),
    );
    // Warm up the transport so a subsequent rewind is a real pass split.
    device.run_transport(
        0,
        vec![note(10, 60, 0, CLAP_EVENT_NOTE_ON)],
        false,
        None,
        Some(transport(9.0, 0)),
    );
    writer.drain(&mut capture);
    let mut events: Vec<_> =
        (0..9).map(|i| device.param(ParamKey::Three, 690.0 + i as f32, 8)).collect();
    events.push(Input::Transport(transport(0.0, 32)));
    events.push(note(20, 64, 40, CLAP_EVENT_NOTE_ON));
    device.run_transport(64, events, false, None, Some(transport(10.0, 0)));
    writer.drain(&mut capture);
    // The boundary that adopts the nine at8 edits, in the rewound pass.
    device.run_transport(128, vec![], false, None, Some(transport(32.0 / 48000.0, 0)));
    writer.drain(&mut capture);
    capture.stop();
    writer.stop();
    writer.drain(&mut capture);
    assert!(
        writer.finished.is_none(),
        "Stop intent cannot finalize pending configuration or producer work"
    );
    device.run_transport(192, vec![], false, None, Some(transport(96.0 / 48000.0, 0)));
    writer.drain(&mut capture);
    assert!(!writer.failed());
    assert!(writer.finished.is_some());
    let first = harmonigraph_take::Take::read(&path).unwrap();
    let second = harmonigraph_take::Take::read(dir.join("record-2.take")).unwrap();
    assert!(
        !first.configurations.iter().any(|c| c.axes[1] == 698_000_000),
        "nine at8 edits are adopted at the next boundary, never backdated into the old pass"
    );
    assert_eq!(second.configurations.first().unwrap().t, 0.0);
    assert_eq!(
        second.configurations.first().unwrap().axes[1],
        700_000_000,
        "the rewound pass opens on the configuration this block actually used"
    );
    let adopted = second.configurations.iter().find(|c| c.axes[1] == 698_000_000).unwrap();
    assert!(
        (adopted.t - 32.0 / 48000.0).abs() < 1e-9,
        "the ninth edit reaches the pass current at the boundary that adopts it"
    );
    // Exactly warmup64 + oldsegment32 versus newsegment32 + one whole block.
    // Closure may not let the final old audio prefix leak into the new WAV or
    // lose its tail.
    for (name, frames) in [("record.wav", 96_u32), ("record-2.wav", 96)] {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        assert_eq!(bytes.len(), 44 + frames as usize * 2 * 4);
    }
    drop(writer);
    device.finish_notes(256, &[(10, 60), (20, 64)]);
    drop(device);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn configuration_observed_disarmed_does_not_become_later_armed_automation() {
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = recorded_device();
    device.activate();
    let events = (0..9).map(|i| device.param(ParamKey::Three, 690.0 + i as f32, 8)).collect();
    device.run(0, events, false);
    capture.arm();
    device.run(64, vec![], false);
    let records = capture.drain_entries();
    let configs: Vec<_> = records
        .into_iter()
        .filter_map(|entry| match entry {
            harmonigraph_record::Entry::ConfigurationAt { config, .. } => Some(config),
            _ => None,
        })
        .collect();
    assert_eq!(configs.len(), 1, "only the new pass's initial value is recorded");
    assert_eq!(configs[0].axes[1], 698_000_000);
    assert!((configs[0].t - 64.0 / 48000.0).abs() < 1e-9);
}

#[test]
fn stop_during_parked_callback_cannot_close_its_later_playing_segment() {
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = recorded_device();
    device.activate();
    capture.arm_audio();
    let dir = std::env::temp_dir()
        .join(format!("harmonigraph-config-stop-segment-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("record.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(
        &capture,
        path.clone(),
        Some(harmonigraph_record::AudioSpec { sample_rate: 48000.0, channels: 2 }),
    );
    let mut parked = transport(0.0, 0);
    parked.flags &= !CLAP_TRANSPORT_IS_PLAYING;
    let mut events = vec![note(9, 48, 0, CLAP_EVENT_NOTE_ON), Input::Transport(transport(0.0, 32))];
    events.extend((0..9).map(|i| device.param(ParamKey::Three, 690.0 + i as f32, 40)));
    events.push(note(10, 60, 40, CLAP_EVENT_NOTE_ON));
    capture.pause_boundary(true);
    std::thread::scope(|scope| {
        let callback = scope.spawn(|| device.run_transport(0, events, false, None, Some(parked)).0);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !capture.boundary_entered()
            && !callback.is_finished()
            && std::time::Instant::now() < until
        {
            std::thread::yield_now();
        }
        let entered = capture.boundary_entered();
        capture.stop();
        writer.stop();
        capture.pause_boundary(false);
        assert_ne!(callback.join().unwrap(), CLAP_PROCESS_ERROR);
        assert!(entered, "fixture must stop after the actual callback captures armed intent");
    });
    writer.drain(&mut capture);
    assert!(writer.finished.is_none());
    capture.pause_producer_close(true);
    let premature = std::thread::scope(|scope| {
        let callback = scope.spawn(|| {
            device.run_transport(64, vec![], false, None, Some(transport(32.0 / 48000.0, 0))).0
        });
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !capture.producer_close_entered()
            && !callback.is_finished()
            && std::time::Instant::now() < until
        {
            std::thread::yield_now();
        }
        let entered = capture.producer_close_entered();
        // Consume the ACTUAL producer closure before owner.record can route the
        // ninth, earlier configuration change. Stop must still remain pending.
        writer.drain(&mut capture);
        let premature = writer.finished.is_some();
        capture.pause_producer_close(false);
        assert_ne!(callback.join().unwrap(), CLAP_PROCESS_ERROR);
        assert!(entered, "fixture must stop after the actual ProducerClosed ring push");
        premature
    });
    assert!(
        !premature,
        "a live disarm cannot close configuration for a callback that captured armed intent"
    );
    writer.drain(&mut capture);
    assert!(!writer.failed());
    assert!(writer.finished.is_some());
    assert_eq!(std::fs::read(path.with_extension("wav")).unwrap().len(), 44 + 32 * 2 * 4);
    drop(writer);
    device.finish_notes(128, &[(9, 48), (10, 60)]);
    drop(device);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn canonical_publication_slots_and_loss_are_allocation_free() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_core::canonical::*;
    use harmonigraph_record::publication::*;
    let (mut publisher, mut consumer) = channel();
    let voices: [_; 64] = std::array::from_fn(|note| VoiceBaseline {
        note: note as u8,
        pitch_microcents: note as i64 * 100_000_000,
        velocity: 0.8,
        ..Default::default()
    });
    let baseline = SourceBaseline::new(SourceId::DIRECT, 1, 1.0, 0, true, &voices).unwrap();
    let mut confirmed = ConfirmedPitches::default();
    for row in &voices {
        confirmed.on(row.confirmed(SourceId::DIRECT)).unwrap();
    }
    let before: Vec<_> = confirmed.rows().copied().collect();
    let start = std::time::Instant::now();
    nice_assert_no_alloc::assert_no_alloc(|| {
        for _ in 0..SNAPSHOT_SLOTS {
            publisher.baseline(&baseline, Route::default()).unwrap();
        }
        assert_eq!(publisher.baseline(&baseline, Route::default()), Err(PublishError::Busy));
        for _ in SNAPSHOT_SLOTS..PUBLICATION_RING - 1 {
            publisher
                .note(
                    harmonigraph_core::NoteEvent::on(1.0, SourceId::DIRECT, 0, 60, 0.8).into(),
                    Route::default(),
                )
                .unwrap();
        }
        assert_eq!(
            publisher.note(
                harmonigraph_core::NoteEvent::off(2.0, SourceId::DIRECT, 0, 60).into(),
                Route::default()
            ),
            Err(PublishError::Lost)
        );
        // Reporting ownership and failure have no access to musical state.
        assert!(confirmed.is_complete());
        assert!(confirmed.rows().copied().eq(before.iter().copied()));
    });
    let duration = start.elapsed();
    // Consume outside the audio guard, then exercise payload reuse under it.
    consumer.drain(|_, _| true);
    nice_assert_no_alloc::assert_no_alloc(|| {
        publisher.baseline(&baseline, Route::default()).unwrap()
    });
    consumer.drain(|_, _| true);
    eprintln!("canonical guarded fill: {SNAPSHOT_SLOTS} complete 64-voice payloads + {} notes + Busy/Lost = {duration:?}; no allocation/deallocation", PUBLICATION_RING - 1 - SNAPSHOT_SLOTS);
}

#[test]
fn direct_publication_loss_recovers_64_exact_lifetimes_without_new_attacks() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_core::confirmed::PitchProvenance;
    use harmonigraph_take::CanonicalRecord;
    let (mut device, mut capture) = recorded_device();
    device.activate();
    capture.arm();
    let dir =
        std::env::temp_dir().join(format!("harmonigraph-direct-repair-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("record.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    device.run(
        0,
        (0..64).map(|key| note(1000 + key, key as i16, 0, CLAP_EVENT_NOTE_ON)).collect(),
        false,
    );
    // The REAL 4096-cell primary ring, while every one of the 64 rich voices
    // remains held. Distinct f64 expression values must survive the legacy f32
    // forwarding adapter, and an ID-only wildcard must resolve on exact ingress.
    for block in 1..=65 {
        device.run(
            block * 64,
            (0..64)
                .map(|key| id_tuning(1000 + key, f64::from(key) / 1000.0 + 0.000000123))
                .collect(),
            false,
        );
    }
    writer.drain(&mut capture);
    // Changed with #712's export decision: a real lost publication used to fail
    // this take durably. It now marks it and keeps recording — the file below
    // still carries `incomplete`, which is what the export warns from.
    assert!(!writer.failed(), "a hole in the note history is not a recording failure");
    let mut tracker = harmonigraph_core::NoteTracker::default();
    for record in capture.display_events() {
        record.apply(&mut tracker).unwrap();
    }
    writer.drain(&mut capture);
    for record in capture.display_events() {
        record.apply(&mut tracker).unwrap();
    }
    assert!(!tracker.publication_gaps().is_empty());
    device.run(66 * 64, vec![], false);
    writer.drain(&mut capture);
    let recovered = capture.display_events();
    let frame = recovered
        .iter()
        .find_map(|record| match record {
            CanonicalRecord::Baseline(frame) => Some(frame.baseline().unwrap()),
            _ => None,
        })
        .expect("silent production callback repairs current state");
    assert_eq!(frame.voices().len(), 64);
    for voice in frame.voices() {
        assert_eq!(voice.host_note_id, 1000 + i32::from(voice.note));
        assert_ne!(voice.lifetime, 0);
        assert_eq!(voice.player_tuning, f64::from(voice.note) / 1000.0 + 0.000000123);
        assert_eq!(
            voice.pitch_microcents,
            ((f64::from(voice.note) + voice.player_tuning) * 100_000_000.0).round() as i64
        );
        assert_eq!(voice.onset.unwrap().input, 0);
        assert_eq!(voice.actual_onset, 0.0);
        assert_eq!(voice.provenance, PitchProvenance::ObservedDirect);
    }
    assert!(!recovered.iter().any(|record| matches!(record,
        CanonicalRecord::Delta(d) if matches!(d.event.kind, harmonigraph_take::NoteKind::On { .. }))));
    for record in recovered {
        record.apply(&mut tracker).unwrap();
    }
    assert_eq!(tracker.held_count(), 64);
    assert!(!tracker.publication_gaps().is_empty(), "repair never erases missing history");
    assert!(harmonigraph_take::Take::read(path).unwrap().incomplete.is_some());
    drop(writer);
    device.finish_notes(67 * 64, &(0..64).map(|key| (1000 + key, key as i16)).collect::<Vec<_>>());
    drop(device);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn display_only_loss_requests_one_factual_direct_repair_after_capacity_returns() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::CanonicalRecord;
    let (mut device, mut capture) = recorded_device();
    device.activate();
    capture.arm();
    let dir = std::env::temp_dir()
        .join(format!("harmonigraph-direct-display-repair-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("record.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    device.run(0, vec![note(10, 60, 0, CLAP_EVENT_NOTE_ON)], false);
    writer.drain(&mut capture);
    for block in 1..=65 {
        device.run(block * 64, (0..64).map(|_| id_tuning(10, 0.123456789)).collect(), false);
        writer.drain(&mut capture);
    }
    // Replacement after display loss belongs to a new original lifetime, even
    // though its key/channel are identical. The disk fanout has kept up.
    device.run(
        66 * 64,
        vec![note(10, 60, 0, CLAP_EVENT_NOTE_OFF), note(20, 60, 1, CLAP_EVENT_NOTE_ON)],
        false,
    );
    writer.drain(&mut capture);
    let mut tracker = harmonigraph_core::NoteTracker::default();
    for record in capture.display_events() {
        record.apply(&mut tracker).unwrap();
    }
    assert!(!tracker.publication_gaps().is_empty());
    assert!(!writer.failed(), "display loss cannot turn retained disk history into loss");
    writer.drain(&mut capture); // capacity has returned; emit one reporting hint
    device.run(67 * 64, vec![], false);
    writer.drain(&mut capture);
    let recovered = capture.display_events();
    let frames: Vec<_> = recovered
        .iter()
        .filter_map(|r| match r {
            CanonicalRecord::Baseline(b) => Some(b.baseline().unwrap()),
            _ => None,
        })
        .collect();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].voices().len(), 1);
    assert_eq!(frames[0].voices()[0].host_note_id, 20);
    assert_eq!(frames[0].voices()[0].onset.unwrap().input, 66 * 64 + 1);
    for record in recovered {
        record.apply(&mut tracker).unwrap();
    }
    assert_eq!(tracker.held_count(), 1);
    device.run(68 * 64, vec![id_tuning(20, 0.987654321)], false);
    writer.drain(&mut capture);
    let later = capture.display_events();
    assert!(
        !later.iter().any(|r| matches!(r, CanonicalRecord::Baseline(_))),
        "no repeated repair after a successful copy"
    );
    for record in later {
        record.apply(&mut tracker).unwrap();
    }
    capture.stop();
    writer.stop();
    device.run(69 * 64, vec![], false);
    writer.drain(&mut capture);
    assert!(writer.finished.is_some());
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(take.incomplete.is_none());
    assert_eq!(take.events.iter().filter(|r| matches!(r,
        CanonicalRecord::Delta(d) if matches!(d.event.kind, harmonigraph_take::NoteKind::On { .. }))).count(), 2);
    drop(writer);
    device.finish_notes(70 * 64, &[(20, 60)]);
    drop(device);
    std::fs::remove_dir_all(dir).unwrap();
}

/// #712, findings 2 and 3 on the DIRECT observer: a lane is gated on its own
/// free cells and paid with its own identity.
///
/// Both lanes overflow together, then only the writer drains. The take's ring
/// is healthy again while the editor's is still full, and DIRECT owes both a
/// snapshot. `publication_free()` used to be the MINIMUM of the two, so the
/// healthy take got nothing: its file kept the notes and lost the frame that
/// says what is still sounding. The identity half is the second assertion --
/// the take is paid with ITS next id, not with a number the display's refusal
/// moved.
#[test]
fn a_full_display_lane_does_not_hold_back_directs_take_snapshot() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_take::CanonicalRecord;
    let (mut device, mut capture) = recorded_device();
    device.activate();
    capture.arm();
    let dir = std::env::temp_dir().join(format!("harmonigraph-direct-lane-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("record.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    device.run(0, vec![note(10, 60, 0, CLAP_EVENT_NOTE_ON)], false);
    // Neither consumer runs, so both rings fill and both lose the same report.
    for block in 1..=66 {
        device.run(block * 64, (0..64).map(|_| id_tuning(10, 0.123456789)).collect(), false);
    }
    // Now the writer catches up and the editor does not.
    writer.drain(&mut capture);
    device.run(67 * 64, vec![], false);
    writer.drain(&mut capture);
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(take.incomplete.is_some(), "the fixture must actually lose a report");
    let frames: Vec<_> = take
        .events
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Baseline(frame) => Some(frame.baseline().unwrap()),
            _ => None,
        })
        .collect();
    let restored = frames.last().expect("the take is owed a snapshot and its own lane has room");
    assert_eq!(restored.voices().len(), 1, "and it says what is still sounding");
    assert_eq!(restored.voices()[0].host_note_id, 10);
    assert_eq!(
        restored.id, 1,
        "paid with the take lane's own next identity, which no refusal on the display moved"
    );
    // The editor comes back. Its lane starts from ITS cursor, so the frame it
    // gets is not one its tracker has already seen and deduplicated away.
    let mut tracker = harmonigraph_core::NoteTracker::default();
    for record in capture.display_events() {
        record.apply(&mut tracker).unwrap();
    }
    assert_eq!(tracker.held_count(), 0, "the gap cleared the display's held set");
    device.run(68 * 64, vec![], false);
    writer.drain(&mut capture);
    let recovered = capture.display_events();
    let display_frame = recovered
        .iter()
        .find_map(|record| match record {
            CanonicalRecord::Baseline(frame) => Some(frame.baseline().unwrap()),
            _ => None,
        })
        .expect("the display lane is owed its own snapshot once it has room again");
    assert_eq!(display_frame.id, 1, "and its own next identity, independent of the take's");
    for record in recovered {
        record.apply(&mut tracker).unwrap();
    }
    assert_eq!(tracker.held_count(), 1, "so the sounding note comes back on the display too");
    drop(writer);
    device.finish_notes(69 * 64, &[(10, 60)]);
    drop(device);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn adaptive_settings_restore_preview_save_and_audio_adoption_agree() {
    let _scope = crate::test_scope::enter();
    let mut device = Device::new();
    device.activate();
    let policy = PolicyConfig {
        harmonic: 2300,
        axes: 3,
        memory: 11,
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
    device.load(state, true);
    let mailbox = device.mailbox();
    assert_eq!(view(mailbox.visible().0, true).resolved.policy, policy);
    let saved: MusicalSettings =
        serde_json::from_str(&device.save().fields[MUSICAL_SETTINGS]).unwrap();
    assert_eq!(PolicyConfig::from(saved.adaptive), policy);
    device.run(0, vec![], false);
    assert_eq!(view(mailbox.visible().0, false).resolved.policy, policy);
    let defaults: MusicalSettings = serde_json::from_str("{\"adaptive\":{}}").unwrap();
    assert_eq!(PolicyConfig::from(defaults.adaptive), PolicyConfig::default());
}
