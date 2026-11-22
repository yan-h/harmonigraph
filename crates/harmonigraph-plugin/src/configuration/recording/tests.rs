//! The Hub's recording: a run's own unit test, and the take suites, which
//! drive a take through the exported CLAP factory and read what it recorded
//! (moved here from `configuration/tests.rs`, #1868).
use super::*;
use crate::configuration::tests::{hub, note, plain, restored, saved};
use crate::configuration::*;
use crate::test_device::{header, Device, Input};
use clap_sys::events::*;
use clap_sys::process::*;
use harmonigraph_clap::plugin::ParamValue;
use harmonigraph_core::configuration::{ConfigReducer, TuningAxis};
use harmonigraph_core::tuning::microcents;
use harmonigraph_ui::params::ParamKey;
use std::sync::atomic::Ordering;

/// A run is stated on the presentation clock the Hub times its records on,
/// and the configuration in force is written inside it.
#[test]
fn a_run_opens_on_the_presentation_clock_with_its_configuration() {
    let (mut recorder, mut capture) = harmonigraph_record::testing::channel();
    capture.arm();
    let intent = recorder.capture_recording_intent();
    recorder.is_armed_at(intent);
    let origin = recorder.observe_transport(20.0, true, 64.0 / 48_000.0);
    let mut recording =
        Recording { captured_intent: intent, block_start: 1000, ..Default::default() };
    recording.presentation = Presentation { anchor: Some((1000, 5.0)), rate: 48_000.0 };
    recorder.record(recording.presentation.at(recording.block_start), origin);
    recorder.configuration(recording.presentation.at(1000), ConfigReducer::default().resolved());
    recording.registered(1040);
    let entries = capture.drain_entries();
    let Some(harmonigraph_record::Entry::Run { at, song }) = entries.first().copied() else {
        panic!("the first recorded block opens a run");
    };
    assert_eq!((at, song), (5.0, 20.0));
    assert_eq!(entries.len(), 2, "and its configuration");

    capture.stop();
    recording.close_stopped(&mut recorder);
    let entries = capture.drain_entries();
    let Some(harmonigraph_record::Entry::RunEnd { at }) = entries.first().copied() else {
        panic!("the stopped run closes");
    };
    assert_eq!(at, 5.0 + 40.0 / 48_000.0);
}

/// A take recorded with every axis off its default and a link released
/// carries those axes in its configuration records, which replay applies
/// verbatim, and none in its parameter lane.
#[test]
fn a_take_carries_released_axes_in_its_configuration_records() {
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = crate::test_device::recorded(|| hub().with_retune());
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
    device.run(0, vec![], None);
    let linked = settings.view().resolved.tuning;
    assert!(settings.submit(ConfigEdit::temper(harmonigraph_core::Comma::Syntonic, false, linked)));
    device.run(64, vec![], None);
    let expected = settings.view().resolved;
    assert!(!expected.modes.tempered.syntonic);
    let fresh = Tuning::default().axes();
    assert!(expected.tuning.axes().iter().zip(fresh).all(|(axis, fresh)| *axis != fresh));
    capture.arm();
    device.run(128, vec![], None);
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

/// A take improvised in Pass through with Decode fifths on records the
/// lattice drawn in the lattice-board's tuning, and from the very sample
/// Lattice Map is chosen mid-block the saved just axes; a save while the
/// board's tuning is drawn stores the saved fifth.
#[test]
fn a_take_records_the_board_s_tuning_only_while_pass_through_decodes() {
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = crate::test_device::recorded(hub);
    device.activate();
    let settings = device.settings();
    let just = Tuning::just();
    assert!(settings.submit(ConfigEdit {
        axes: [None, Some(just.three), Some(just.five), None, None],
        policy: Some(PolicyConfig { decode_fifths: true, ..PolicyConfig::default() }),
        ..Default::default()
    }));
    device.run(0, vec![], None);
    assert_eq!(saved(&device.save()).axes[1], just.three, "a save stored the drawn fifth");
    capture.arm();
    let lattice_map = Input::Param(clap_event_param_value {
        header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, 10),
        param_id: harmonigraph_clap::wrapper::hash_param_id("tuning-engine"),
        cookie: std::ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value: f64::from(harmonigraph_core::lattice_map::TuningEngine::LatticeMap.index()),
    });
    device.run(64, vec![lattice_map], None);
    let fifths: Vec<_> = capture
        .drain_entries()
        .iter()
        .filter_map(|entry| match entry {
            harmonigraph_record::Entry::Configuration(config) => Some(*config),
            _ => None,
        })
        .collect();
    let board = PolicyConfig::default().decoded_fifth;
    let drawn: Vec<_> = fifths
        .iter()
        .map(|c| c.resolved())
        .map(|c| (c.tuning.three, c.modes.tempered.syntonic))
        .collect();
    assert_eq!(drawn, [(board, true), (just.three, false)]);
    let switched = fifths[1].t - fifths[0].t;
    assert!((switched - 10.0 / 48_000.0).abs() < 1e-9, "switched {switched} s into the block");
}

pub(in crate::configuration) fn transport(seconds: f64, time: u32) -> clap_event_transport {
    let mut transport: clap_event_transport = unsafe { std::mem::zeroed() };
    transport.header = header::<clap_event_transport>(CLAP_EVENT_TRANSPORT, time);
    transport.flags = CLAP_TRANSPORT_HAS_SECONDS_TIMELINE | CLAP_TRANSPORT_IS_PLAYING;
    transport.song_pos_seconds =
        (seconds * clap_sys::fixedpoint::CLAP_SECTIME_FACTOR as f64) as i64;
    transport
}
/// The Stop the editor sends. Each fixture first aims the recorder at a
/// renderer that does not exist, so a finished take launches no render.
pub(in crate::configuration) fn stop(shared: &std::sync::Arc<crate::editor::EditorLock>) {
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
    let (mut device, mut capture) = crate::test_device::recorded(|| hub().with_retune());
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
        let (mut device, mut capture) = crate::test_device::recorded(|| hub().with_retune());
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
    let (mut device, mut capture) = crate::test_device::recorded(|| hub().with_retune());
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
    let (mut device, mut capture) = crate::test_device::recorded(|| hub().with_retune());
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

/// A tuning edit before a host reset is in force after it, and the run the
/// reset opens states it again at its start: each run carries the
/// configuration it sounds in, so the resumed section does not lean on the
/// reader joining it to the run before (#1832 review).
#[test]
fn a_tuning_edit_before_a_host_reset_opens_the_run_after_it() {
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = crate::test_device::recorded(|| hub().with_retune());
    device.activate();
    capture.arm();
    let dir =
        std::env::temp_dir().join(format!("harmonigraph-config-reset-edit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("record.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone());
    let block = 64.0 / 48000.0;
    device.run_transport(0, vec![], false, None, Some(transport(1.0, 0)));
    let edited = microcents(690.0);
    assert!(device.settings().submit(ConfigEdit::axis(TuningAxis::Three, edited)));
    device.run_transport(64, vec![], false, None, Some(transport(1.0 + block, 0)));
    device.run_transport(128, vec![], false, None, Some(transport(1.0 + 2.0 * block, 0)));
    unsafe { ((*device.plugin).reset.unwrap())(device.plugin) };
    let mut raw = 192;
    for i in 3..10 {
        device.run_transport(raw, vec![], false, None, Some(transport(1.0 + i as f64 * block, 0)));
        raw += 64;
    }
    capture.stop();
    for _ in 0..12 {
        let mut parked = transport(1.0 + 10.0 * block, 0);
        parked.flags &= !CLAP_TRANSPORT_IS_PLAYING;
        device.run_transport(raw, vec![], false, None, Some(parked));
        raw += 64;
    }
    writer.stop();
    writer.drain(&mut capture);
    assert!(!writer.failed());
    let take = harmonigraph_take::Take::read(&path).unwrap();
    let configurations: Vec<_> = take.configurations.iter().map(|c| (c.t, c.axes[1])).collect();
    let resumed = 1.0 + 3.0 * block;
    assert!(
        configurations.iter().any(|&(t, axis)| (t - resumed).abs() < 1e-9 && axis == edited),
        "the run after the reset opens with the edit: {configurations:?}"
    );
    assert_eq!(configurations.last().map(|c| c.1), Some(edited), "{configurations:?}");
    drop(writer);
    drop(device);
    std::fs::remove_dir_all(dir).unwrap();
}

/// CLAP lets a host restart its steady counter at a reset. A take recorded
/// across one reads exactly as it does when the counter runs on: its notes and
/// configurations land where they would have, on both sides of the reset
/// (#1881). The note after it is struck in the first block, which a restart
/// left unpinned would time before the release ahead of it: before the fix
/// that take refused to read, and notes only before the reset landed three
/// blocks late.
///
/// 256 blocks before the reset, because by then the plugin's summed block
/// durations have run past the Hub's steady-time mapping by more than the
/// take's song time can hide: the reset's marker for the direct source,
/// stamped from the sum, sorted after the resumed onset and ended it at once,
/// though it sounds for two blocks.
#[test]
fn a_host_reset_that_restarts_the_steady_counter_records_as_one_that_does_not() {
    const BEFORE: i64 = 256;
    let block = 64.0 / 48000.0;
    let record = |restart: bool, after: bool| {
        let _scope = crate::test_scope::enter();
        let (mut device, mut capture) = crate::test_device::recorded(|| hub().with_retune());
        device.activate();
        capture.arm();
        let dir = std::env::temp_dir().join(format!(
            "harmonigraph-config-reset-counter-{restart}-{after}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("record.take");
        let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone());
        let struck = vec![note(10, 60, 0, CLAP_EVENT_NOTE_ON)];
        device.run_transport(0, struck, false, None, Some(transport(1.0, 0)));
        assert!(device.settings().submit(ConfigEdit::axis(TuningAxis::Three, microcents(690.0))));
        let released = vec![note(10, 60, 30, CLAP_EVENT_NOTE_OFF)];
        device.run_transport(64, released, false, None, Some(transport(1.0 + block, 0)));
        for i in 2..BEFORE {
            let position = transport(1.0 + i as f64 * block, 0);
            device.run_transport(i * 64, vec![], false, None, Some(position));
        }
        unsafe { ((*device.plugin).reset.unwrap())(device.plugin) };
        let mut raw = if restart { 0 } else { BEFORE * 64 };
        for i in BEFORE..BEFORE + 7 {
            let events = match i - BEFORE {
                0 if after => vec![note(11, 64, 0, CLAP_EVENT_NOTE_ON)],
                2 if after => vec![note(11, 64, 0, CLAP_EVENT_NOTE_OFF)],
                _ => vec![],
            };
            let position = transport(1.0 + i as f64 * block, 0);
            device.run_transport(raw, events, false, None, Some(position));
            raw += 64;
        }
        capture.stop();
        for _ in 0..12 {
            let mut parked = transport(1.0 + (BEFORE + 7) as f64 * block, 0);
            parked.flags &= !CLAP_TRANSPORT_IS_PLAYING;
            device.run_transport(raw, vec![], false, None, Some(parked));
            raw += 64;
        }
        writer.stop();
        writer.drain(&mut capture);
        assert!(!writer.failed());
        let take = harmonigraph_take::Take::read(&path)
            .unwrap_or_else(|e| panic!("restart {restart}, after {after}: {e:?}"));
        let configurations: Vec<_> = take.configurations.iter().map(|c| (c.t, c.axes[1])).collect();
        let notes = (drawn(&take, 60), drawn(&take, 64));
        drop(writer);
        drop(device);
        std::fs::remove_dir_all(dir).unwrap();
        (notes, configurations)
    };
    for after in [true, false] {
        let ran_on = record(false, after);
        assert_eq!(ran_on.0 .0.len(), 1, "the fixture holds the note before the reset");
        assert_eq!(ran_on.0 .1.len(), usize::from(after), "and the one after it only when asked");
        if let [(start, Some(end))] = ran_on.0 .1[..] {
            let struck = 1.0 + BEFORE as f64 * block;
            assert!(
                (start - struck).abs() < 1e-9 && (end - (struck + 2.0 * block)).abs() < 1e-9,
                "the note after the reset sounds for its two blocks: {start}..{end}"
            );
        }
        assert_eq!(record(true, after), ran_on, "notes after the reset: {after}");
    }
}

/// Armed with the transport stopped, then sent back to the start before
/// anything played — Bitwig's stop-returns-to-start, or a jump to bar 1 for an
/// AtBar take. Under take v7 that move owed a split, and paying it from a pass
/// that never rolled left the writer holding an empty pass open for a close
/// nothing sent: Stop waited on it forever and every later Start was refused.
#[test]
fn a_playhead_moved_back_before_the_take_rolls_lets_stop_finish_one_file() {
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = crate::test_device::recorded(|| hub().with_retune());
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
    let (mut device, mut capture) = crate::test_device::recorded(|| hub().with_retune());
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
            Entry::AudioSamples { samples: n, .. } => samples += n,
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
    let (mut device, mut capture) = crate::test_device::recorded(|| hub().with_retune());
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
        let mut device = hub().with_retune();
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
            shared.take.start(appearance);
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
        assert!(!shared.lock().take.has_ended(), "the restore records nothing and ends nothing");
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
        let mut device = hub().with_retune();
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
            shared.take.start(shared.ui.picture.appearance.serialize());
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
        let mut device = hub().with_retune();
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
            shared.take.start(shared.ui.picture.appearance.serialize());
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
        device.set_active(false);
        if resume == 2 {
            // Stop lands after activate, before start_processing resets routes.
            assert!(unsafe { ((*device.plugin).activate.unwrap())(device.plugin, 48000.0, 1, 64) });
        }
        if !stop_before_deactivate {
            let callbacks = device.host().callbacks.load(Ordering::Relaxed);
            stop(&shared);
            assert!(
                device.host().callbacks.load(Ordering::Relaxed) > callbacks,
                "Stop must request host service even with no further audio"
            );
        }
        if resume == 2 {
            assert!(unsafe { ((*device.plugin).start_processing.unwrap())(device.plugin) });
            device.set_active(true);
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
            shared.lock().take.start(String::new());
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
        // Retune at its shipped default, off: what is claimed is the take path
        // every CLAP session has, not the correction pipeline Retune feeds.
        let mut device = hub();
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
            shared.take.start(appearance);
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
    let (mut device, mut capture) = crate::test_device::recorded(|| hub().with_retune());
    device.activate();
    for i in 0..9 {
        assert!(device
            .settings()
            .submit(ConfigEdit::axis(TuningAxis::Three, microcents(690.0 + i as f32))));
    }
    device.run(0, vec![], None);
    capture.arm();
    device.run(64, vec![], None);
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

/// Lost host input is judged at the callback that loses it: a take armed
/// there fails, however the host goes on, and one armed after it refuses until
/// a host reset clears the loss. A loss while no take is armed fails nothing,
/// rather than refusing every later take until the plugin reloads, or a take
/// whose Stop landed before it (#1832).
#[test]
fn lost_input_fails_the_take_armed_at_the_loss() {
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Case {
        /// The take closed, then the loss; a take armed after it.
        AfterTheTake,
        /// Lost while it records, stopped before the next callback.
        WhileRecording,
        /// Lost while armed and parked, stopped before the next callback.
        WhileParked,
        /// Lost while it records, and the host resets before the next.
        ResetNext,
        /// Stopped, then lost before a callback closed its run.
        StoppedFirst,
    }
    /// One callback at `raw`, the transport playing or parked there.
    fn play(device: &mut Device, raw: i64, playing: bool) {
        let mut at = transport(raw as f64 / 48_000.0, 0);
        if !playing {
            at.flags &= !CLAP_TRANSPORT_IS_PLAYING;
        }
        device.run_transport(raw, vec![], false, None, Some(at));
    }
    /// One callback at `raw` whose input is lost: an event past its end.
    fn lose(device: &mut Device, raw: i64) {
        let lost = vec![note(10, 60, 64, CLAP_EVENT_NOTE_ON)];
        let (status, _) = device.run_transport(raw, lost, false, None, None);
        assert_eq!(status, CLAP_PROCESS_ERROR, "the fixture must lose the callback's input");
    }
    let _scope = crate::test_scope::enter();
    for case in [
        Case::AfterTheTake,
        Case::WhileRecording,
        Case::WhileParked,
        Case::ResetNext,
        Case::StoppedFirst,
    ] {
        let (mut device, capture) = crate::test_device::recorded(|| hub().with_retune());
        device.activate();
        let dir = std::env::temp_dir()
            .join(format!("harmonigraph-config-loss-{}-{case:?}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let writer = harmonigraph_record::testing::FileWriter::new(&capture, dir.join("t.take"));
        capture.arm();
        match case {
            Case::AfterTheTake => {
                play(&mut device, 0, true);
                capture.stop();
                play(&mut device, 64, true);
                lose(&mut device, 128);
                play(&mut device, 192, true);
                assert!(!writer.failed(), "a loss after the take closed fails nothing");
                capture.arm();
                play(&mut device, 256, true);
            }
            Case::WhileRecording | Case::WhileParked => {
                play(&mut device, 0, case == Case::WhileRecording);
                lose(&mut device, 64);
                capture.stop();
                play(&mut device, 128, true);
            }
            Case::ResetNext => {
                play(&mut device, 0, true);
                lose(&mut device, 64);
                unsafe { ((*device.plugin).reset.unwrap())(device.plugin) };
            }
            Case::StoppedFirst => {
                play(&mut device, 0, true);
                capture.stop();
                lose(&mut device, 64);
                play(&mut device, 128, true);
            }
        }
        assert_eq!(writer.failed(), case != Case::StoppedFirst, "{case:?}");
        drop(writer);
        drop(device);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn every_engine_records_a_tuning_edit_at_the_callback_that_applies_it() {
    let _scope = crate::test_scope::enter();
    // Pass through, Adaptive and Lattice Map.
    for engine in 0..3 {
        let (mut device, mut capture) = crate::test_device::recorded(|| hub().with_retune());
        device.activate();
        let mut state = restored(&device, 696.0);
        state
            .params
            .insert("tuning-engine".into(), harmonigraph_clap::plugin::ParamValue::I32(engine));
        device.load(state);
        device.run(0, vec![], None);
        capture.arm();
        device.run(64, vec![], None);
        assert!(device.settings().submit(ConfigEdit::axis(TuningAxis::Three, microcents(690.0))));
        device.run(128, vec![], None);
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
    let mut device = hub().with_retune();
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
        shared.take.start(shared.ui.picture.appearance.serialize());
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

/// What a take came to after the host reactivated the plugin at another rate.
struct AcrossRates {
    failed: bool,
    status: String,
    sealed: bool,
    take: harmonigraph_take::Take,
    /// The WAV header's rate and the frames it holds.
    wav_rate: u32,
    frames: u32,
    /// Song seconds the take recorded.
    recorded: f64,
}

/// A take armed while the engine runs at `armed_at`, recording `before` blocks
/// there, then just over a second more after the host reactivates the plugin
/// at `rate` (a Bitwig export at another rate than the engine's, #1928).
fn take_across_reactivation(armed_at: f64, before: usize, rate: f64) -> AcrossRates {
    let _scope = crate::test_scope::enter();
    let (mut device, mut capture) = crate::test_device::recorded(hub);
    device.activate_format(armed_at, 512);
    device.run(0, vec![], None);
    capture.arm();
    let dir = std::env::temp_dir()
        .join(format!("harmonigraph-rate-change-{}-{before}-{rate}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("record.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone());
    let (mut raw, mut song) = (0, 0.0);
    let mut play = |device: &mut Device, capture: &mut _, writer: &mut _, block_rate: f64| {
        device.run_transport(raw, vec![], false, None, Some(transport(song, 0)));
        harmonigraph_record::testing::FileWriter::drain(writer, capture);
        (raw, song) = (raw + 64, song + 64.0 / block_rate);
    };
    for _ in 0..before {
        play(&mut device, &mut capture, &mut writer, armed_at);
    }
    device.reactivate_format(rate, 512);
    for _ in 0..(rate / 64.0).ceil() as usize {
        play(&mut device, &mut capture, &mut writer, rate);
    }
    capture.stop();
    let mut parked = transport(song, 0);
    parked.flags &= !CLAP_TRANSPORT_IS_PLAYING;
    device.run_transport(raw, vec![], false, None, Some(parked));
    writer.stop();
    writer.drain(&mut capture);
    writer.drain(&mut capture);
    let take = harmonigraph_take::Take::read(&path).unwrap();
    let wav = std::fs::read(path.with_extension("wav")).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let field = |at: usize| u32::from_le_bytes(wav[at..at + 4].try_into().unwrap());
    AcrossRates {
        failed: writer.failed(),
        status: writer.status(),
        sealed: writer.finished.is_some(),
        take,
        wav_rate: field(24),
        frames: field(40) / 8,
        recorded: song,
    }
}

/// A take whose audio all arrives after the host reactivates the plugin at
/// another rate than the one Record was clicked at takes the rate the audio
/// came at: before #1928 its WAV kept the click's 48 kHz, and its soundtrack
/// played 8.8% fast with nothing to say so.
#[test]
fn a_take_whose_audio_follows_a_rate_change_keeps_its_soundtrack_in_time() {
    let across = take_across_reactivation(48_000.0, 0, 44_100.0);
    assert!(!across.failed && across.sealed, "{}", across.status);
    assert_eq!(across.wav_rate, 44_100);
    let soundtrack = f64::from(across.frames) / f64::from(across.wav_rate);
    assert!(
        (soundtrack - across.recorded).abs() < 1e-3,
        "the take recorded {:.4} s of song and its soundtrack lasts {soundtrack:.4} s",
        across.recorded,
    );
}

/// A rate change after the take has recorded audio fails the take, loudly:
/// one WAV cannot state two speeds (#1928). The soundtrack keeps what was
/// recorded before the change, under its own rate.
#[test]
fn a_rate_change_after_a_take_recorded_audio_fails_it() {
    let before = 8;
    let across = take_across_reactivation(44_100.0, before, 48_000.0);
    assert!(across.failed, "the take must fail");
    assert!(
        across.status.contains("sample rate changed from 44100 Hz to 48000 Hz")
            && across.status.contains("no render started"),
        "{}",
        across.status,
    );
    assert!(!across.sealed, "a failed take is never handed to the renderer");
    assert!(across.take.incomplete.is_some(), "the take file is marked incomplete");
    assert_eq!((across.wav_rate, across.frames), (44_100, 64 * before as u32));
}
