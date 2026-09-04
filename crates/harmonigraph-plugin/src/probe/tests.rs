//! Exercises the exported CLAP factory and the actual wrapper, not a substitute scheduler.
//! These are apparatus checks. They do not establish any Bitwig scheduling premise.

use std::ffi::{c_char, c_void, CStr};
use std::mem::size_of;
use std::ptr;

use clap_sys::audio_buffer::clap_audio_buffer;
use clap_sys::events::*;
use clap_sys::ext::latency::{clap_plugin_latency, CLAP_EXT_LATENCY};
use clap_sys::ext::params::{clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS};
use clap_sys::factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID};
use clap_sys::host::clap_host;
use clap_sys::plugin::clap_plugin;
use clap_sys::process::*;
use clap_sys::version::CLAP_VERSION;

use super::trace::Event;

unsafe extern "C" fn extension(_: *const clap_host, _: *const c_char) -> *const c_void {
    ptr::null()
}
unsafe extern "C" fn request(_: *const clap_host) {}

static HOST: clap_host = clap_host {
    clap_version: CLAP_VERSION,
    host_data: ptr::null_mut(),
    name: c"#615 apparatus fixture".as_ptr(),
    vendor: c"test".as_ptr(),
    url: c"https://github.com/yan-h/harmonigraph/issues/615".as_ptr(),
    version: c"1".as_ptr(),
    get_extension: Some(extension),
    request_restart: Some(request),
    request_process: Some(request),
    request_callback: Some(request),
};

#[derive(Clone, Copy)]
enum Input {
    Note(clap_event_note),
    Tuning(clap_event_note_expression),
    Transport(clap_event_transport),
    Param(clap_event_param_value),
}

impl Input {
    fn header(&self) -> &clap_event_header {
        match self {
            Self::Note(e) => &e.header,
            Self::Tuning(e) => &e.header,
            Self::Transport(e) => &e.header,
            Self::Param(e) => &e.header,
        }
    }
}

fn header<T>(kind: u16, time: u32) -> clap_event_header {
    clap_event_header { size: size_of::<T>() as u32, time, space_id: 0, type_: kind, flags: 0 }
}

fn note(kind: u16, time: u32, id: i32, key: i16) -> Input {
    Input::Note(clap_event_note {
        header: header::<clap_event_note>(kind, time),
        note_id: id,
        port_index: 0,
        channel: 0,
        key,
        velocity: 0.7,
    })
}

fn tuning(time: u32, id: i32, value: f64) -> Input {
    Input::Tuning(clap_event_note_expression {
        header: header::<clap_event_note_expression>(CLAP_EVENT_NOTE_EXPRESSION, time),
        expression_id: CLAP_NOTE_EXPRESSION_TUNING,
        note_id: id,
        port_index: 0,
        channel: 0,
        key: 60,
        value,
    })
}

fn transport(time: u32) -> Input {
    // The raw event position is deliberately nonzero; the trace must retain it
    // independently of the framework's derived sub-block transport.
    Input::Transport(clap_event_transport {
        header: header::<clap_event_transport>(CLAP_EVENT_TRANSPORT, time),
        flags: CLAP_TRANSPORT_HAS_SECONDS_TIMELINE
            | CLAP_TRANSPORT_HAS_TEMPO
            | CLAP_TRANSPORT_IS_PLAYING,
        song_pos_beats: 0,
        song_pos_seconds: 123_456,
        tempo: 120.0,
        tempo_inc: 0.0,
        loop_start_beats: 0,
        loop_end_beats: 0,
        loop_start_seconds: 0,
        loop_end_seconds: 0,
        bar_start: 0,
        bar_number: 0,
        tsig_num: 4,
        tsig_denom: 4,
    })
}

unsafe extern "C" fn input_size(list: *const clap_input_events) -> u32 {
    unsafe { (&*((*list).ctx as *const Vec<Input>)).len() as u32 }
}
unsafe extern "C" fn input_get(list: *const clap_input_events, i: u32) -> *const clap_event_header {
    unsafe { (&*((*list).ctx as *const Vec<Input>))[i as usize].header() }
}

struct Sink {
    events: Vec<Event>,
    reject: bool,
}
unsafe extern "C" fn output_push(
    list: *const clap_output_events,
    header: *const clap_event_header,
) -> bool {
    let sink = unsafe { &mut *((*list).ctx as *mut Sink) };
    if sink.reject {
        return false;
    }
    assert!(sink.events.len() < sink.events.capacity());
    sink.events.push(Event::raw(unsafe { &*header }));
    true
}

struct Device {
    plugin: *const clap_plugin,
}

impl Device {
    fn new(id: &CStr) -> Self {
        let factory =
            unsafe { (crate::clap_entry.get_factory.unwrap())(CLAP_PLUGIN_FACTORY_ID.as_ptr()) }
                as *const clap_plugin_factory;
        let plugin = unsafe { ((*factory).create_plugin.unwrap())(factory, &HOST, id.as_ptr()) };
        assert!(!plugin.is_null());
        assert!(unsafe { ((*plugin).init.unwrap())(plugin) });
        Self { plugin }
    }
    fn activate(&self) {
        assert!(unsafe { ((*self.plugin).activate.unwrap())(self.plugin, 48_000.0, 1, 64) });
        assert!(unsafe { ((*self.plugin).start_processing.unwrap())(self.plugin) });
    }
    fn reset(&self) {
        unsafe { ((*self.plugin).reset.unwrap())(self.plugin) };
    }
    fn latency(&self) -> u32 {
        let ext = unsafe {
            ((*self.plugin).get_extension.unwrap())(self.plugin, CLAP_EXT_LATENCY.as_ptr())
        } as *const clap_plugin_latency;
        assert!(!ext.is_null());
        unsafe { ((*ext).get.unwrap())(self.plugin) }
    }
    fn automation(&self, time: u32) -> Input {
        let ext = unsafe {
            ((*self.plugin).get_extension.unwrap())(self.plugin, CLAP_EXT_PARAMS.as_ptr())
        } as *const clap_plugin_params;
        assert!(!ext.is_null());
        let mut info: clap_param_info = unsafe { std::mem::zeroed() };
        assert!(unsafe { ((*ext).get_info.unwrap())(self.plugin, 0, &mut info) });
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
    fn run(&self, steady: i64, frames: u32, events: Vec<Input>, reject: bool) -> (i32, Vec<Event>) {
        let input = clap_input_events {
            ctx: &events as *const _ as *mut c_void,
            size: Some(input_size),
            get: Some(input_get),
        };
        let mut sink = Sink { events: Vec::with_capacity(4096), reject };
        let output = clap_output_events {
            ctx: &mut sink as *mut _ as *mut c_void,
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
            frames_count: frames,
            transport: ptr::null(),
            audio_inputs: &audio,
            audio_outputs: &mut audio,
            audio_inputs_count: 1,
            audio_outputs_count: 1,
            in_events: &input,
            out_events: &output,
        };
        let status = unsafe { ((*self.plugin).process.unwrap())(self.plugin, &process) };
        (status, sink.events)
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

fn accepted_onsets(events: &[Event]) -> Vec<(u32, i32, i16)> {
    events
        .iter()
        .filter(|e| e.kind == CLAP_EVENT_NOTE_ON)
        .map(|e| (e.offset, e.note_id, e.key))
        .collect()
}

#[test]
fn exported_probe_preserves_split_times_and_requires_complete_central_reply() {
    let dir =
        std::env::temp_dir().join(format!("harmonigraph-probe-fixture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("HARMONIGRAPH_PROBE_DIR", &dir);
    let config = super::Config { delay_samples: 64, expected_sources: 3, ..Default::default() };
    std::fs::write(dir.join("config.json"), serde_json::to_vec(&config).unwrap()).unwrap();
    {
        let hub = Device::new(c"com.yan-h.harmonigraph");
        let a = Device::new(c"com.yanhan.harmonigraph-tune-probe");
        let b = Device::new(c"com.yanhan.harmonigraph-tune-probe");
        let silent = Device::new(c"com.yanhan.harmonigraph-tune-probe");
        hub.activate();
        a.activate();
        b.activate();
        silent.activate();
        assert_eq!(a.latency(), 64);
        assert_eq!(hub.latency(), 0);
        let (status, output) = a.run(
            0,
            64,
            vec![
                note(CLAP_EVENT_NOTE_ON, 0, 11, 60),
                tuning(0, 11, 0.25),
                transport(16),
                note(CLAP_EVENT_NOTE_ON, 16, 12, 62),
                a.automation(32),
                note(CLAP_EVENT_NOTE_ON, 63, 13, 64),
            ],
            false,
        );
        assert_ne!(status, CLAP_PROCESS_ERROR);
        assert!(output.is_empty());
        assert!(hub.run(0, 64, vec![transport(16)], false).1.is_empty());
        b.run(0, 64, vec![note(CLAP_EVENT_NOTE_ON, 0, 21, 65)], false);
        hub.run(64, 64, vec![], false);
        // No silent-source progress yet: neither active source can have an answer.
        silent.run(0, 64, vec![], false);
        hub.run(128, 64, vec![], false);
        let (_, out) = a.run(64, 17, vec![], false);
        assert_eq!(accepted_onsets(&out), [(0, 11, 60), (16, 12, 62)]);
        let tuning_values: Vec<_> = out
            .iter()
            .filter(|e| e.kind == CLAP_EVENT_NOTE_EXPRESSION && e.note_id == 11)
            .map(|e| e.value)
            .collect();
        assert_eq!(tuning_values, [0.5, 0.75]);
        let (_, out) = a.run(81, 47, vec![], false);
        assert_eq!(accepted_onsets(&out), [(46, 13, 64)]);
        let (_, out) = b.run(64, 64, vec![], false);
        assert_eq!(accepted_onsets(&out), [(0, 21, 65)]);
        // Later expression adds to the held correction; note-off may omit note id.
        a.run(128, 64, vec![tuning(0, 11, -0.25), note(CLAP_EVENT_NOTE_OFF, 16, -1, 60)], false);
        let (_, out) = a.run(192, 64, vec![], false);
        assert_eq!(out.iter().find(|e| e.kind == CLAP_EVENT_NOTE_EXPRESSION).unwrap().value, 0.25);
        assert!(out
            .iter()
            .any(|e| e.kind == CLAP_EVENT_NOTE_OFF && e.offset == 16 && e.note_id == -1));
        // Reset cancels pending lifetimes; a previously produced reply cannot resurrect one.
        a.reset();
        b.reset();
        silent.reset();
        a.run(256, 64, vec![note(CLAP_EVENT_NOTE_ON, 0, 31, 60)], false);
        b.run(256, 64, vec![], false);
        silent.run(256, 64, vec![], false);
        hub.run(256, 64, vec![], false);
        a.reset();
        assert!(a.run(320, 64, vec![], false).1.is_empty());
    }
    let late_config = super::Config { expected_sources: 1, hold_extra_samples: 96, ..config };
    std::fs::write(dir.join("config.json"), serde_json::to_vec(&late_config).unwrap()).unwrap();
    {
        let hub = Device::new(c"com.yan-h.harmonigraph");
        let source = Device::new(c"com.yanhan.harmonigraph-tune-probe");
        hub.activate();
        source.activate();
        source.run(
            0,
            64,
            vec![
                note(CLAP_EVENT_NOTE_ON, 0, 41, 60),
                tuning(8, 41, 0.25),
                note(CLAP_EVENT_NOTE_OFF, 32, -1, 60),
            ],
            false,
        );
        hub.run(0, 64, vec![], false);
        for now in [64, 128, 192] {
            let (status, output) = source.run(now, 64, vec![], false);
            assert_ne!(status, CLAP_PROCESS_ERROR);
            assert!(output.is_empty());
            hub.run(now, 64, vec![], false);
        }
        let (_, output) = source.run(256, 64, vec![], false);
        assert_eq!(accepted_onsets(&output), [(0, 41, 60)]);
        assert!(output
            .iter()
            .any(|e| e.kind == CLAP_EVENT_NOTE_EXPRESSION && e.offset == 8 && e.value == 0.75));
        assert!(output.iter().any(|e| e.kind == CLAP_EVENT_NOTE_OFF && e.offset == 32));
        assert_eq!(source.run(-1, 64, vec![], false).0, CLAP_PROCESS_ERROR);
    }
    let reject_config = super::Config { expected_sources: 1, hold_extra_samples: 0, ..late_config };
    std::fs::write(dir.join("config.json"), serde_json::to_vec(&reject_config).unwrap()).unwrap();
    {
        let hub = Device::new(c"com.yan-h.harmonigraph");
        let source = Device::new(c"com.yanhan.harmonigraph-tune-probe");
        hub.activate();
        source.activate();
        source.run(0, 64, vec![note(CLAP_EVENT_NOTE_ON, 0, 51, 60)], false);
        hub.run(0, 64, vec![], false);
        assert!(source.run(64, 64, vec![], true).1.is_empty());
        assert_eq!(source.run(128, 64, vec![], false).0, CLAP_PROCESS_ERROR);
    }
    // Complete files are evidence about the fixture, including the exact wrapper split.
    let records: Vec<serde_json::Value> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .flat_map(|e| {
            std::fs::read_to_string(e.path())
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(records
        .iter()
        .any(|r| r["kind"] == "sub_block" && r["enter"] == true && r["clock"]["start"] == 16));
    assert!(records
        .iter()
        .any(|r| r["kind"] == "sub_block" && r["enter"] == true && r["clock"]["start"] == 32));
    assert!(records.iter().any(|r| r["kind"] == "callback_enter"
        && r["latency_queries"].as_u64().unwrap_or(0) > 0
        && r["reported_latency"] == 64));
    assert!(records
        .iter()
        .any(|r| r["kind"] == "fault" && r["reason"] == "obsolete_reply_rejected"));
    assert!(!records.iter().any(|r| r["kind"] == "trace_loss"));
    for reason in ["assignment_deadline_missed", "missing_raw_steady_clock", "host_output_rejected"]
    {
        assert!(
            records.iter().any(|r| r["kind"] == "fault" && r["reason"] == reason),
            "missing {reason}"
        );
    }
    assert!(records.iter().any(|r| r["kind"] == "raw_output" && r["accepted"] == false));
    // Instance 1 is the first hub. Neither of its first two callbacks was
    // allowed to finalize notes before the silent participant's progress.
    let first_hub =
        std::fs::read_to_string(dir.join(format!("trace-{}-1.jsonl", std::process::id()))).unwrap();
    for line in first_hub.lines() {
        let r: serde_json::Value = serde_json::from_str(line).unwrap();
        if r["kind"] == "assignment" {
            assert!(r["clock"]["callback"].as_u64().unwrap() >= 3);
        }
    }
    eprintln!("Apparatus trace: {}", dir.display());
}
