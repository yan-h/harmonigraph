//! Production exported-factory fixtures. The real CLAP entry point, real
//! endpoint capacities and real recorder fanout, without opening an editor.
use super::*;
use crate::test_device::{header, Device, Host, Input};
use clap_sys::{
    events::*,
    ext::track_info::{clap_plugin_track_info, CLAP_EXT_TRACK_INFO, CLAP_EXT_TRACK_INFO_COMPAT},
    process::*,
};
use event::Event;

#[path = "control_tests.rs"]
mod control_tests;
#[path = "map_tests.rs"]
mod map_tests;
#[path = "musical_tests.rs"]
mod musical_tests;
use std::ptr;
use std::sync::atomic::Ordering;

/// The tuning suites' host, which answers latency alone; a fixture that
/// reads the track name adds track info.
fn host() -> Host {
    Host::new().latency()
}
/// A Hub on [`host`], initialized and not yet active.
fn hub() -> Device {
    Device::hub(host())
}
/// A Tune on [`host`], initialized and not yet active.
fn tune() -> Device {
    Device::tune(host())
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
fn hub_wrapper(device: &Device) -> &harmonigraph_clap::wrapper::clap::Wrapper<crate::Harmonigraph> {
    device.wrapper()
}
/// The Hub's tuning configuration, as the editor submits to and reads it.
fn settings(device: &Device) -> std::sync::Arc<crate::configuration::Settings> {
    hub_wrapper(device).test_inspect_plugin(|plugin| plugin.params.settings.clone())
}
fn inspect_hub<R>(device: &Device, f: impl FnOnce(&hub::Hub) -> R) -> R {
    hub_wrapper(device).test_inspect_plugin(|plugin| f(plugin.aggregation.as_ref().unwrap()))
}
fn inspect_tune<R>(device: &Device, f: impl FnOnce(&tune::Tune) -> R) -> R {
    device.tune_wrapper().test_inspect_plugin(|plugin| f(plugin.tune.as_ref().unwrap()))
}
fn transport(time: u32, tempo: f64) -> Input {
    let mut event: clap_event_transport = unsafe { std::mem::zeroed() };
    event.header = header::<clap_event_transport>(CLAP_EVENT_TRANSPORT, time);
    event.flags = CLAP_TRANSPORT_HAS_TEMPO | CLAP_TRANSPORT_IS_PLAYING;
    event.tempo = tempo;
    Input::Transport(event)
}
/// A callback's own transport, at `seconds` on the timeline, which is what the
/// take's lifecycle reads to decide whether a block records.
fn timeline(seconds: f64, playing: bool) -> clap_event_transport {
    let mut event: clap_event_transport = unsafe { std::mem::zeroed() };
    event.header = header::<clap_event_transport>(CLAP_EVENT_TRANSPORT, 0);
    event.flags = CLAP_TRANSPORT_HAS_TEMPO
        | CLAP_TRANSPORT_HAS_SECONDS_TIMELINE
        | if playing { CLAP_TRANSPORT_IS_PLAYING } else { 0 };
    event.tempo = 120.0;
    event.song_pos_seconds = (seconds * 2_147_483_648.0) as i64;
    event
}
/// A host parameter event by string id, at its CLAP value: a stepped
/// parameter's step index.
fn parameter(id: &str, value: f64, time: u32) -> Input {
    Input::Param(clap_event_param_value {
        header: header::<clap_event_param_value>(CLAP_EVENT_PARAM_VALUE, time),
        param_id: harmonigraph_clap::wrapper::hash_param_id(id),
        cookie: ptr::null_mut(),
        note_id: -1,
        port_index: -1,
        channel: -1,
        key: -1,
        value,
    })
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
    /// The Hub runs the Adaptive engine.
    fn new() -> Self {
        let mut hub = hub().with_adaptive();
        hub.activate();
        let mut tune = tune();
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

#[test]
fn tune_and_hub_diagnostics_publish_once_per_audio_interval() {
    let _scope = crate::test_scope::enter();
    for tuner in [true, false] {
        let mut device = if tuner { tune() } else { hub().with_adaptive() };
        device.activate_format(48_000.0, 64);
        let shared = device.shared();
        assert_eq!(shared.diagnostics.test_publications(), (0, (!tuner).then_some(0)));
        for callback in 0..=1500 {
            device.run(i64::from(callback) * 64, vec![], None);
            let sequence = 2 * (1 + callback as u64 / 750);
            assert_eq!(
                shared.diagnostics.test_publications(),
                (sequence, (!tuner).then_some(sequence)),
                "both snapshots share one countdown, callback {callback}, tuner {tuner}",
            );
        }
    }
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

/// A note the host refuses is kept, not lost: it goes out at the start of the
/// next callback, with its tuning.
#[test]
fn a_refused_note_goes_out_at_the_start_of_the_next_callback() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    assert!(pair.step(vec![note(1, 0, 60, 17, true)]).is_empty(), "nothing emits before D");
    let refused = pair.tune.run(pair.raw, vec![], Some(CLAP_EVENT_NOTE_ON));
    pair.hub.run(pair.raw, vec![], None);
    pair.raw += 512;
    let onset =
        |event: &Event| matches!(event, Event::Note { kind: CLAP_EVENT_NOTE_ON, key: 60, .. });
    assert!(refused.rejected.iter().any(|(_, event)| onset(event)), "the host refused it");
    assert!(!refused.values.iter().any(|(_, event)| onset(event)));
    let output = pair.idle();
    assert!(
        output.iter().any(|(time, event)| *time == 0 && onset(event)),
        "the refused note goes out at offset 0 of the next callback: {output:?}"
    );
    assert!(tuning_of(&output).is_some(), "with its tuning");
}

/// An onset whose correction has not arrived by its own deadline emits at raw
/// pitch and counts the miss. Nothing waits, and the next note is unaffected.
#[test]
fn an_onset_without_its_correction_emits_uncorrected_and_counts_the_miss() {
    let _scope = crate::test_scope::enter();
    // No Hub in the process at all: the request has nowhere to go, which is
    // the cheapest way to reach the uncorrected path from outside.
    let mut tune = tune();
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
    let mut tune = tune();
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

/// A delayed answer cannot change what already emitted. With no Hub or an
/// unprocessed Hub, the player's duplicate initial expressions remain raw.
#[test]
fn initial_expressions_survive_unpaired_and_missed_reply_fallback() {
    let _scope = crate::test_scope::enter();
    for paired in [false, true] {
        let mut hub = paired.then(|| hub().with_adaptive());
        if let Some(hub) = &mut hub {
            hub.activate();
        }
        let mut tune = tune();
        tune.activate();
        let input = vec![note(7, 0, 57, 0, true), expression(7, 0.25, 0), expression(7, 0.5, 0)];
        assert!(tune.run(0, input, None).values.is_empty());
        let output = tune.run(512, vec![], None).values;
        assert_eq!(output.len(), 3, "raw note and both original expressions");
        assert_eq!(tuning_of(&output[1..]), Some(0.25));
        assert_eq!(tuning_of(&output[2..]), Some(0.5));
        assert_eq!(tune.shared().misses.load(Ordering::Relaxed), u64::from(paired));
        assert_eq!(tune.shared().last_input.load(Ordering::Relaxed), 5_750_000_000);
        if let Some(hub) = &mut hub {
            hub.run(512, vec![], None);
            assert!(
                tune.run(1024, vec![], None).values.is_empty(),
                "late reply does not replay the note"
            );
        }
    }
}

/// #1127. A snapshot never covers history still waiting to publish. The Hub's
/// first callback owes every row a snapshot, and here it also collects a late
/// onset, which `publish` puts on both lanes before it builds the snapshots
/// that cover it. Both lanes are
/// read on this thread by consumers that refuse history at or below a cut they
/// have adopted: the display's tracker, and a tracker fed the take lane. That
/// second one holds the take to the same rule the writer does (it fences on
/// `sequence <= cut`), though it is not the writer's code.
#[test]
fn a_late_reply_publishes_its_history_before_the_snapshot_that_covers_it() {
    use harmonigraph_core::{canonical::CanonicalEvent, NoteTracker};
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
    hub.activate();
    let mut tune = tune();
    tune.activate();
    assert!(tune.run(0, vec![note(7, 0, 57, 0, true)], None).values.is_empty());
    assert_eq!(tune.run(512, vec![], None).values.len(), 1, "emitted uncorrected, on time");
    let mut display = NoteTracker::new();
    let mut drain = |capture: &mut harmonigraph_record::testing::Capture| {
        let mut snapshots = Vec::new();
        capture.display_into(&mut display, |event, _| {
            if let CanonicalEvent::Baseline(frame) = event {
                snapshots.push(frame.source.0);
            }
        });
        snapshots
    };
    hub.run(512, vec![], None);
    let first = drain(&mut capture);
    // A second callback, so a snapshot paid twice or only a callback late fails.
    hub.run(1024, vec![], None);
    let second = drain(&mut capture);
    let note = display.roll().notes().find(|n| n.note == 57).expect("the late onset is drawn");
    assert!(note.history_complete, "drawn from its own attack, not only from a snapshot");
    // The row owed a snapshot with its history pending, which is what says the
    // fixture reached the held-back path; `end` pays it in that same callback.
    let owed = |seen: &[u64]| seen.iter().filter(|&&s| s == note.source.0).count();
    assert_eq!((owed(&first), owed(&second)), (1, 0), "{first:?} then {second:?}");
    let mut take = NoteTracker::new();
    for record in capture.drain_canonical() {
        record.apply(&mut take).expect("the take lane keeps history before its snapshot");
    }
}

/// Each initial value follows original input order, even though the Hub
/// sorts controllers before attacks. Publication keeps every retrigger's
/// assigned pitch, including the lifetime replaced within the same sample.
#[test]
fn initial_expression_order_and_identity_match_in_both_production_routes() {
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    let _scope = crate::test_scope::enter();
    let addressed = |id, port, channel, key, value| {
        let Input::Expression(mut e) = expression(id, value, 0) else { unreachable!() };
        e.port_index = port;
        e.channel = channel;
        e.key = key;
        Input::Expression(e)
    };
    for direct in [false, true] {
        let cases = [
            (vec![expression(7, 0.75, 0), note(7, 0, 57, 0, true)], vec![0.0]),
            (
                vec![note(7, 0, 57, 0, true), expression(7, 0.25, 0), expression(7, 0.5, 0)],
                vec![0.5],
            ),
            (
                vec![
                    note(7, 0, 57, 0, true),
                    expression(7, 0.25, 0),
                    note(7, 0, 57, 0, true),
                    expression(7, 0.5, 0),
                ],
                vec![0.25, 0.5],
            ),
            (
                vec![
                    note(7, 0, 57, 0, true),
                    expression(7, 0.25, 0),
                    note(8, 0, 57, 0, true),
                    expression(8, 0.5, 0),
                    expression(7, 0.75, 0),
                ],
                vec![0.25, 0.5],
            ),
            (
                vec![
                    note(7, 0, 57, 0, true),
                    addressed(-1, 0, 0, 57, 0.25),
                    addressed(7, 1, -1, -1, 0.75),
                    addressed(7, -1, 1, -1, 0.75),
                    addressed(7, -1, -1, 58, 0.75),
                ],
                vec![0.25],
            ),
            (vec![note(-1, 0, 57, 0, true), expression(7, 0.5, 0)], vec![0.0]),
            (vec![note(-1, 0, 57, 0, true), addressed(-1, -1, 0, 57, 0.5)], vec![0.5]),
        ];
        for (input, players) in cases {
            let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
            hub.activate();
            musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
            hub.shared().set_retune(true);
            let mut tune = (!direct).then(tune);
            let output = if let Some(tune) = &mut tune {
                tune.activate();
                tune.run(0, input, None);
                hub.run(0, vec![], None);
                tune.run(512, vec![], None).values
            } else {
                hub.run(0, input, None).values
            };
            let published: Vec<_> = capture
                .display_events()
                .into_iter()
                .filter_map(|record| match record {
                    CanonicalRecord::Delta(delta)
                        if matches!(delta.event.kind, NoteKind::On { .. }) =>
                    {
                        Some(delta)
                    }
                    _ => None,
                })
                .collect();
            let emitted: Vec<_> = output
                .windows(2)
                .filter_map(|events| events[0].1.attack().and_then(|_| tuning_of(&events[1..])))
                .collect();
            assert_eq!(published.len(), players.len());
            assert_eq!(emitted.len(), players.len(), "{output:?}");
            for ((delta, emitted), player) in published.iter().zip(emitted).zip(&players) {
                let metadata = delta.assignment.as_ref().unwrap();
                let correction = metadata.correction_microcents;
                assert_eq!(
                    delta.pitch_microcents,
                    Some(5_700_000_000 + (player * 1e8_f64).round() as i64 + correction)
                );
                assert_eq!(metadata.player_tuning, *player);
                assert!((emitted - (player + correction as f64 / 1e8)).abs() < 1e-9);
            }
            let voice =
                inspect_hub(&hub, |hub| hub.test_voice(if direct { DIRECT } else { 0 }, 0, 57))
                    .unwrap();
            assert_eq!(voice.player_tuning, *players.last().unwrap());
            assert_eq!(voice.pitch_microcents, voice.onset_pitch_microcents);
        }
    }
}

/// One broadcast expression initializes every addressed key, including when
/// Retune is disabled and the host receives the original wildcard unchanged.
#[test]
fn a_same_sample_wildcard_initializes_each_addressed_onset() {
    let _scope = crate::test_scope::enter();
    for (id, channel, key, players) in [
        (-1, 0, -1, [0.5, 0.5]),
        (-1, -1, 60, [0.5, 0.0]),
        (8, -1, -1, [0.0, 0.5]),
        (-1, 1, -1, [0.0, 0.0]),
    ] {
        let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
        hub.activate();
        let mut tune = tune();
        tune.activate();
        tune.shared().set_retune(false);
        let Input::Expression(mut broadcast) = expression(id, 0.5, 0) else { unreachable!() };
        broadcast.port_index = 0;
        broadcast.channel = channel;
        broadcast.key = key;
        let mut pair = Pair { hub, tune, raw: 0 };
        pair.step(vec![
            note(7, 0, 60, 0, true),
            note(8, 0, 64, 0, true),
            Input::Expression(broadcast),
        ]);
        let output = pair.idle();
        assert_eq!(output.len(), 3, "two notes and the uncorrected broadcast");
        assert!(matches!(
            output[2].1,
            Event::Expression { id: out_id, port: 0, channel: out_channel, key: out_key, value: 0.5, .. }
                if (out_id, out_channel, out_key) == (id, channel, key)
        ));
        let published: Vec<_> = capture
            .display_events()
            .into_iter()
            .filter_map(|record| match record {
                harmonigraph_take::CanonicalRecord::Delta(delta)
                    if matches!(delta.event.kind, harmonigraph_take::NoteKind::On { .. }) =>
                {
                    Some(delta)
                }
                _ => None,
            })
            .collect();
        assert_eq!(published.len(), 2);
        for (key, player) in [60, 64].into_iter().zip(players) {
            let voice = inspect_hub(&pair.hub, |hub| hub.test_voice(0, 0, key)).unwrap();
            assert_eq!(voice.player_tuning, player, "addressed key {key}");
            let expected = i64::from(key) * 100_000_000 + (player * 100_000_000.0).round() as i64;
            assert_eq!(voice.onset_pitch_microcents, expected);
            assert_eq!(voice.pitch_microcents, expected);
            assert_eq!(
                published.iter().find(|delta| delta.event.note == key).unwrap().pitch_microcents,
                Some(expected)
            );
        }
    }
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

/// A transport Stop as Bitwig sends one, the callback's own transport going
/// not-playing with nothing in the list, is a cut: the Tune's held note gets
/// its Note-Off in that callback.
#[test]
fn a_stop_in_the_callbacks_own_transport_cuts_held_notes() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    let block = |pair: &mut Pair, input: Vec<Input>, playing: bool| {
        let at = Some(timeline(pair.raw as f64 / 48_000.0, playing));
        let sink = pair.tune.run_callback(pair.raw, input, (None, None), (512, false), at);
        pair.hub.run_callback(pair.raw, vec![], (None, None), (512, false), at);
        pair.raw += 512;
        sink.values
    };
    block(&mut pair, vec![note(1, 0, 60, 0, true)], true);
    let sounded = block(&mut pair, vec![], true);
    assert!(sounded.iter().any(|(_, event)| event.attack().is_some()), "{sounded:?}");
    assert!(!sounded.iter().any(|(_, event)| event.release()), "still held");
    let stopped = block(&mut pair, vec![], false);
    assert!(stopped.iter().any(|(_, event)| event.release()), "the Stop cut: {stopped:?}");
}

/// The missed-correction count is advice about the delay, read when playing
/// stops, so a transport Stop keeps it while its cut clears the fault bits.
/// Reset all voices and a reactivation clear it, a Reset landing right behind
/// the Stop included.
#[test]
fn missed_corrections_survive_a_stop_and_clear_at_reset_or_reactivation() {
    let _scope = crate::test_scope::enter();
    let edge = |pair: &mut Pair, playing: bool| {
        let Input::Transport(mut value) = transport(0, 120.0) else { unreachable!() };
        if !playing {
            value.flags &= !CLAP_TRANSPORT_IS_PLAYING;
        }
        pair.tune.run(pair.raw, vec![Input::Transport(value)], None);
        pair.hub.run(pair.raw, vec![Input::Transport(value)], None);
        pair.raw += 512;
    };
    for clear in ["Reset", "reactivation", "Reset behind the Stop"] {
        let mut pair = Pair::new();
        edge(&mut pair, true);
        // A full copy ring: the note sounds raw, a miss, beside RING_FULL.
        let mut input: Vec<_> = (0..CAPTURE_RING).map(|_| raw_midi([0xb0, 20, 0], 0)).collect();
        input.push(note(1, 0, 60, 0, true));
        pair.step(input);
        for _ in 0..3 {
            pair.idle();
        }
        assert_eq!((pair.misses(), pair.status() & session::RING_FULL), (1, session::RING_FULL));
        edge(&mut pair, false);
        if clear == "Reset behind the Stop" {
            // The Hub takes the Reset before the Tune adopts the Stop.
            session::session().request_reset();
            pair.hub.run(pair.raw, vec![], None);
            pair.raw += 512;
        } else {
            for _ in 0..2 {
                pair.idle();
            }
            assert_eq!((pair.misses(), pair.status()), (1, 0), "Stop keeps the count alone");
            // Counted misses with no status bit still draw the Tune's name in
            // the fault colour (`docs/intent/tuning.md`, #1798 H3).
            let tune = instances::snapshots().into_iter().find(|row| !row.is_hub).unwrap();
            assert!(tune.faulted, "{clear}: a counted miss alone drew no fault");
            match clear {
                "Reset" => session::session().request_reset(),
                _ => pair.tune.reactivate_format(48000.0, 512),
            }
        }
        for _ in 0..2 {
            pair.idle();
        }
        assert_eq!(pair.misses(), 0, "{clear}");
    }
}

/// #1799. A note whose correction reaches its Tune too late sounds raw, and
/// is drawn raw from the Tune's miss on, in the display and in the take. The
/// usual miss: the Tune's whole callback runs before the Hub's, so the Hub
/// collects the onset and its miss in one pass and draws the note raw from its
/// start, though the onset delta itself is published corrected. A just E, so
/// the correction is a real one.
#[test]
fn a_late_reply_draws_the_note_raw_from_the_miss_on() {
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
    hub.activate();
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let mut tune = tune();
    tune.activate();
    let dir = std::env::temp_dir().join(format!("harmonigraph-late-miss-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.take");
    capture.arm();
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone());
    let at = |raw: i64| Some(timeline(1.0 + raw as f64 / 48_000.0, true));
    let hub_block = |raw: i64| hub.run_callback(raw, vec![], (None, None), (512, false), at(raw));
    let tune_block =
        |raw: i64, played| tune.run_callback(raw, played, (None, None), (512, false), at(raw));
    // The Hub runs before the Tune in the block the E is played in, and the
    // Tune before the Hub in the block it sounds in, so no reply is back.
    hub_block(0);
    tune_block(0, vec![note(1, 0, 64, 0, true)]);
    let sounded = tune_block(512, vec![]);
    assert!(tuning_of(&sounded.values).is_none(), "the fixture must reach a late reply");
    hub_block(512);
    let mut display = harmonigraph_core::NoteTracker::new();
    capture.display_into(&mut display, |_, _| {});
    let drawn = display.held_voices().find(|v| v.note == 64).unwrap();
    assert_eq!(drawn.pitch, 64.0, "the display draws the pitch the note sounds at");
    for raw in [1024, 1536] {
        hub_block(raw);
        tune_block(raw, vec![]);
    }
    tune_block(2048, vec![note(1, 0, 64, 0, false)]);
    hub_block(2048);
    for raw in [2560, 3072] {
        tune_block(raw, vec![]);
        hub_block(raw);
    }
    capture.stop();
    writer.stop();
    hub_block(3584);
    writer.drain(&mut capture);
    let take = harmonigraph_take::Take::read(&path).unwrap();
    let mut tracker = harmonigraph_core::NoteTracker::new();
    for record in &take.events {
        record.apply(&mut tracker).unwrap();
    }
    let onset = take.events.iter().find_map(|record| match record {
        harmonigraph_take::CanonicalRecord::Delta(delta)
            if matches!(delta.event.kind, harmonigraph_take::NoteKind::On { .. }) =>
        {
            delta.pitch_microcents
        }
        _ => None,
    });
    assert!(
        matches!(onset, Some(pitch) if pitch != 6_400_000_000),
        "the fixture's onset must be published corrected: {onset:?}"
    );
    let note = tracker.roll().notes().find(|n| n.note == 64).unwrap();
    assert_eq!(note.end_pitch(), 64.0, "the take draws the pitch the note sounded at");
    drop(writer);
    std::fs::remove_dir_all(dir).unwrap();
}

/// #1799. A miss the Hub reads a callback after its onset was scheduled is
/// withdrawn at the start of the callback that reads it, never at the onset's
/// earlier time: a snapshot an earlier callback published covers the voice
/// with its correction, and a withdrawal timed before it would sort ahead of
/// it on read, and the take would refuse to export.
#[test]
fn a_miss_read_a_callback_late_is_withdrawn_at_that_callbacks_start() {
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
    hub.activate();
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let mut tune = tune();
    tune.activate();
    let at = |raw: i64| Some(timeline(1.0 + raw as f64 / 48_000.0, true));
    let hub_block = |raw: i64| hub.run_callback(raw, vec![], (None, None), (512, false), at(raw));
    let tune_block =
        |raw: i64, played| tune.run_callback(raw, played, (None, None), (512, false), at(raw));
    hub_block(0);
    // What the Hub's first callback publishes is timed at its first sample.
    let zero = capture.display_events()[0].time();
    // The Hub's callback at 512 runs before the Tune's first, which it never
    // sees: the one at 1024 reads the onset and its miss. The Hub processes
    // every block, so its clock reaches the onset's scheduled time (#1885).
    hub_block(512);
    tune_block(0, vec![note(1, 0, 64, 0, true)]);
    let sounded = tune_block(512, vec![]);
    assert!(tuning_of(&sounded.values).is_none(), "the fixture must reach a late reply");
    hub_block(1024);
    // The onset, then the delta that withdraws its correction.
    let events = capture.display_events();
    let at_kind = |on: bool| {
        events.iter().find_map(|record| match record {
            harmonigraph_take::CanonicalRecord::Delta(d)
                if matches!(d.event.kind, harmonigraph_take::NoteKind::On { .. }) == on =>
            {
                Some(d.event.t - zero)
            }
            _ => None,
        })
    };
    let (onset, withdrawn) = (at_kind(true).unwrap(), at_kind(false).unwrap());
    let start = 1024.0 / 48_000.0;
    assert!(onset < start, "the fixture's onset must be scheduled before {start}: {onset}");
    assert!((withdrawn - start).abs() < 1e-9, "withdrawn at {withdrawn}, not at {start}");
}

/// An invalid first callback, whose steady time may be CLAP's -1 for
/// "unknown", pins nothing: the presentation clock is pinned at the first
/// valid one, so a note struck there is timed as it is without the invalid
/// callback before it.
#[test]
fn an_invalid_first_callback_does_not_pin_the_presentation_clock() {
    let onset = |invalid_first: bool| {
        let _scope = crate::test_scope::enter();
        let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
        hub.activate();
        if invalid_first {
            hub.run_status(-1, vec![], None, None, 64, true);
        }
        hub.run(0, vec![note(1, 0, 60, 0, true)], None);
        capture.display_events().iter().find_map(|record| match record {
            harmonigraph_take::CanonicalRecord::Delta(d)
                if matches!(d.event.kind, harmonigraph_take::NoteKind::On { .. }) =>
            {
                Some(d.event.t)
            }
            _ => None,
        })
    };
    let pinned = onset(false);
    assert!(pinned.is_some(), "the fixture publishes its onset");
    assert_eq!(onset(true), pinned);
}

/// #1799. A miss names its onset by serial, so one for a voice already
/// released changes nothing, even with a new press held on the same key.
#[test]
fn a_miss_for_a_released_voice_changes_nothing() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub().with_adaptive();
    hub.activate();
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let mut tune = tune();
    tune.activate();
    hub.run(0, vec![], None);
    tune.run(0, vec![note(1, 0, 64, 0, true)], None);
    // The first E sounds raw here, a miss, as the second takes its key.
    let sounded = tune.run(512, vec![note(1, 0, 64, 0, false), note(2, 0, 64, 1, true)], None);
    assert!(tuning_of(&sounded.values).is_none(), "the fixture must reach a late reply");
    hub.run(512, vec![], None);
    let output = tune.run(1024, vec![], None);
    assert!(tuning_of(&output.values).is_some(), "the second E is corrected");
    let held = inspect_hub(&hub, |hub| hub.test_voice(0, 0, 64)).unwrap();
    assert_eq!(held.host_note_id, 2);
    assert_ne!(held.frozen_offset_microcents, 0, "the miss left the second E corrected");
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
    pair.tune.host_reset();
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
/// reasons is now one epoch bump: it cuts what is sounding, forgets the
/// history, and the very next phrase is corrected again.
#[test]
fn reset_cuts_and_audio_resumes_within_two_callbacks() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    pair.step(vec![note(2, 0, 67, 0, true)]);
    pair.step(vec![note(2, 0, 67, 0, false)]);
    pair.step(vec![note(1, 0, 60, 0, true)]);
    let sounding = pair.idle();
    assert!(tuning_of(&sounding).is_some(), "the fixture reaches the Reset with a note held");
    assert!(
        inspect_hub(&pair.hub, |hub| hub.test_memory()) > 0,
        "and with a released one remembered"
    );
    session::session().request_reset();
    // The Hub takes the cut in its next callback; the Tune adopts it in the
    // one after, and owes a Note-Off for the voice it ended.
    let mut cut = pair.idle();
    cut.extend(pair.idle());
    assert!(
        cut.iter().any(|(_, event)| event.release()),
        "the cut releases what it forgot it was holding: {cut:?}"
    );
    assert_eq!(
        inspect_hub(&pair.hub, |hub| hub.test_memory()),
        0,
        "and the history is forgotten, the voice the cut released included"
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
    pair.tune.host_reset();
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
    let mut hub = hub().with_adaptive();
    hub.activate();
    let mut early = tune();
    early.activate();
    let mut late = tune();
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

/// The same lag against the take: a Tune the host runs after the Hub is a
/// callback behind, so the Hub sequences its note a callback after it was
/// played, and a take stopped then still has it: the Hub publishes what it
/// collected before a later callback ends the run. The writer's own count
/// order is not exercised here; this harness drains everything at once.
#[test]
fn a_tune_run_after_the_hub_has_its_last_notes_in_a_stopped_take() {
    let _scope = crate::test_scope::enter();
    let (hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
    let (mut hub, mut tune) = (hub, tune());
    hub.activate();
    tune.activate();
    let dir = std::env::temp_dir().join(format!("harmonigraph-tune-after-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    capture.arm();
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, dir.join("t.take"));
    let mut position = 1.0;
    let mut cycle = |raw: i64, played: Vec<Input>| {
        hub.run_callback(raw, vec![], (None, None), (512, false), Some(timeline(position, true)));
        tune.run_format(raw, played, None, None, 512);
        position += 512.0 / 48_000.0;
    };
    cycle(0, vec![]);
    // Played in the Tune after the Hub's callback: sequenced at 1024, where it
    // sounds, inside the run.
    cycle(512, vec![note(1, 0, 60, 0, true)]);
    cycle(1024, vec![]);
    capture.stop();
    writer.stop();
    // This callback observes the disarm and ends the run.
    cycle(1536, vec![]);
    writer.drain(&mut capture);
    assert!(!writer.failed(), "a routine lag must not fail the recorder");
    let sealed = writer.finished.clone().expect("the stopped take seals");
    let take = harmonigraph_take::Take::read(&sealed).unwrap();
    let struck = take
        .notes()
        .any(|n| n.note == 60 && matches!(n.kind, harmonigraph_take::NoteKind::On { .. }));
    assert!(struck, "the Tune's last note before the run's end is in the take");
    std::fs::remove_dir_all(dir).unwrap();
}

/// Lowering a Tune's delay while a take records leaves the snapshot after its
/// cut timed at the old delay, and a note at the new one would sound before
/// it. A row's times never go back, so the note is recorded at the row's
/// latest time, up to the old delay late, and the take exports (#1885).
#[test]
fn lowering_a_tunes_delay_while_recording_records_the_next_notes_late() {
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
    let mut tune = tune();
    tune.shared().request_delay(4);
    hub.activate();
    tune.activate();
    let dir = std::env::temp_dir().join(format!("harmonigraph-delay-down-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    capture.arm();
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, dir.join("t.take"));
    let mut position = 1.0;
    let mut cycle = |hub: &Device, tune: &Device, raw: i64, played: Vec<Input>| {
        tune.run_format(raw, played, None, None, 512);
        hub.run_callback(raw, vec![], (None, None), (512, false), Some(timeline(position, true)));
        position += 512.0 / 48_000.0;
    };
    // Sounds at 512 + 4 x 512, where the snapshot after the cut is timed.
    cycle(&hub, &tune, 512, vec![note(1, 0, 60, 0, true)]);
    tune.shared().request_delay(1);
    tune.reactivate_format(48000.0, 512);
    // Sounds at 1024 + 512, before that snapshot.
    cycle(&hub, &tune, 1024, vec![note(2, 0, 64, 0, true)]);
    for raw in [1536, 2048, 2560, 3072] {
        cycle(&hub, &tune, raw, vec![]);
    }
    capture.stop();
    writer.stop();
    cycle(&hub, &tune, 3584, vec![]);
    writer.drain(&mut capture);
    let sealed = writer.finished.clone().expect("the stopped take seals");
    let take = harmonigraph_take::Take::read(&sealed).expect("the take reads in order");
    let mut tracker = harmonigraph_core::NoteTracker::new();
    for record in &take.events {
        record.apply(&mut tracker).unwrap();
    }
    let start = |key: u8| tracker.roll().notes().find(|n| n.note == key).map(|n| n.start);
    let (cut, late) = (start(60).expect("the first note"), start(64).expect("the second"));
    assert!(late >= cut, "the second note is not before the first: {cut} {late}");
    assert!(late - cut <= 4.0 * 512.0 / 48_000.0, "it lands at most the old delay late");
    std::fs::remove_dir_all(dir).unwrap();
}

/// Armed with the transport stopped, a note played, and disarmed: nothing
/// recorded, so the take draws nothing, though the note was played armed.
#[test]
fn an_armed_take_whose_transport_never_rolled_draws_no_note() {
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
    hub.activate();
    let dir =
        std::env::temp_dir().join(format!("harmonigraph-never-rolled-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    capture.arm();
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, dir.join("t.take"));
    let mut raw = 0;
    let mut step = |hub: &Device, events: Vec<Input>| {
        hub.run_callback(raw, events, (None, None), (512, false), Some(timeline(5.0, false)));
        raw += 512;
    };
    for _ in 0..3 {
        step(&hub, vec![]);
    }
    step(&hub, vec![note(1, 0, 60, 0, true), note(1, 0, 60, 100, false)]);
    for _ in 0..3 {
        step(&hub, vec![]);
    }
    capture.stop();
    for _ in 0..4 {
        step(&hub, vec![]);
    }
    writer.stop();
    writer.drain(&mut capture);
    assert!(!writer.failed());
    let sealed = writer.finished.clone().expect("the take seals");
    let take = harmonigraph_take::Take::read(&sealed).unwrap();
    let notes: Vec<_> = take.notes().map(|n| (n.note, n.t)).collect();
    assert!(notes.is_empty(), "an unrecorded note reaches the take: {notes:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

/// Membership is the epoch. A Tune attaching cuts every paired track, which
/// is the price of never having a hot-plug reconciliation to get wrong.
#[test]
fn a_tune_attaching_cuts_every_track_already_playing() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    pair.step(vec![note(1, 0, 60, 0, true)]);
    assert!(tuning_of(&pair.idle()).is_some());
    let mut joining = tune();
    joining.activate();
    let cut = pair.idle();
    assert!(
        cut.iter().any(|(_, event)| event.release()),
        "the attach cut the voice the first Tune was holding: {cut:?}"
    );
    assert!(inspect_hub(&pair.hub, |hub| hub.test_memory()) > 0, "attach retains released voices");
    drop(joining);
}

/// #757 and #672, retargeted. Destruction is one main-thread release: the row
/// goes back, and the next Tune takes it. There is nothing to settle, so
/// there is nothing that can fail to.
#[test]
fn destroy_leaves_no_registry_entry() {
    let _scope = crate::test_scope::enter();
    let session = session::session();
    let mut hub = hub().with_adaptive();
    hub.activate();
    assert_ne!(session.hub(), 0);
    for _ in 0..3 {
        let mut tune = tune();
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
    let mut first = hub().with_adaptive();
    first.activate();
    let mut second = hub().with_adaptive();
    second.activate();
    let mut tune = tune();
    tune.activate();
    tune.run(0, vec![note(1, 0, 60, 0, true)], None);
    first.run(0, vec![], None);
    second.run(0, vec![], None);
    let output = tune.run(512, vec![], None).values;
    assert_eq!(output.len(), 2, "the note is still corrected by the Hub that owns the session");
    assert_ne!(tune.shared().status() & session::SECOND_HUB, 0);
    assert_ne!(first.shared().status() & session::SECOND_HUB, 0);
}

/// A Tune's notes wait on the Hub's callbacks for their corrections, and a
/// Tune may be added while Harmonigraph's track is quiet, so the Hub asks the
/// host to keep calling it with no Tune paired and nothing sounding.
#[test]
fn the_hub_asks_to_be_kept_running_with_nothing_paired_or_sounding() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub().with_adaptive();
    hub.activate();
    assert_eq!(hub.run(0, vec![], None).status, CLAP_PROCESS_CONTINUE);
}

/// The delay is `multiplier x advertised maximum frames`, resolved at
/// activation and reported to the host as latency before any Hub exists. The
/// multiplier is a saved setting, so a project reopens at the one it saved.
#[test]
fn the_delay_is_saved_and_reported_as_latency_before_any_hub_exists() {
    let _scope = crate::test_scope::enter();
    let mut tune = tune();
    assert_eq!(tune.latency(), 0, "no activation has advertised a format yet");
    tune.activate_format(48000.0, 256);
    assert_eq!(tune.latency(), 256);
    tune.shared().request_delay(4);
    tune.reactivate_format(48000.0, 256);
    assert_eq!(tune.latency(), 4 * 256, "the new multiplier is adopted at the reactivation");
    let saved = tune.tune_wrapper().get_state_object();
    let mut reopened = Device::tune(host());
    assert!(reopened.tune_wrapper().test_restore(&saved));
    reopened.activate_format(48000.0, 256);
    assert_eq!(reopened.latency(), 4 * 256, "a reopened project reports the saved delay");
}

/// The #615 latency protocol, as the host sees it: one notification per
/// activation that moves the latency, sent during that activation, and no
/// restart until the plugin asks for a different latency, then one. A restart
/// requested at activation stalls Bitwig's next offline bounce (#615).
#[test]
fn latency_is_announced_once_during_activation_and_a_change_asks_one_restart() {
    let _scope = crate::test_scope::enter();
    let mut tune = tune();
    // (notifications, notifications outside an activation, restarts, the
    // latency the host read when notified)
    let seen = |device: &Device| {
        let stats = device.host();
        (
            stats.latency_changes.load(Ordering::Relaxed),
            stats.out_of_phase.load(Ordering::Relaxed),
            stats.restarts.load(Ordering::Relaxed),
            stats.observed_latency.load(Ordering::Relaxed),
        )
    };
    let mut raw = 0;
    let mut play = |device: &Device| {
        for _ in 0..3 {
            device.run_format(raw, vec![], None, None, 256);
            raw += 256;
            device.main();
        }
    };
    tune.activate_format(48000.0, 256);
    assert_eq!(seen(&tune), (1, 0, 0, 256), "announced during the activation, no restart");
    play(&tune);
    assert_eq!(seen(&tune), (1, 0, 0, 256), "the same latency asks for nothing");
    tune.shared().request_delay(4);
    play(&tune);
    assert_eq!(seen(&tune), (1, 0, 1, 256), "a new latency asks one restart and waits for it");
    assert_eq!(tune.latency(), 256, "the host reads the running activation's latency");
    tune.reactivate_format(48000.0, 256);
    assert_eq!(seen(&tune), (2, 0, 1, 4 * 256), "the reactivation announces the new one");
    play(&tune);
    assert_eq!(seen(&tune), (2, 0, 1, 4 * 256));
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

/// The two publication lanes stay independent, and the display shows what the
/// Hub scheduled: input plus that source's D, not the sample it arrived at.
#[test]
fn the_display_shows_the_schedule_rather_than_the_input() {
    let _scope = crate::test_scope::enter();
    let (mut hub, capture) = crate::test_device::recorded(|| hub().with_adaptive());
    hub.activate();
    let mut tune = tune();
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
                Some(delta.event.t)
            }
            _ => None,
        })
        .expect("the onset reached the display lane");
    let sample = onset * 48_000.0;
    assert!((sample - f64::from(64 + 512)).abs() < 1e-6, "drawn where it sounds: {sample}");
}

/// A paired Tune's pressure reaches the display as an expression on the note
/// it addresses, in the host's own units — including a starting value sent on
/// the onset's own sample, which the Hub's same-sample merge must not apply
/// ahead of the onset it belongs to.
#[test]
fn a_tunes_pressure_reaches_the_display_as_an_expression() {
    use harmonigraph_take::{CanonicalRecord, ExpressionKind, NoteKind};
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
    hub.activate();
    let mut tune = tune();
    tune.activate();
    let pressure = Input::Expression(clap_event_note_expression {
        header: header::<clap_event_note_expression>(CLAP_EVENT_NOTE_EXPRESSION, 64),
        expression_id: CLAP_NOTE_EXPRESSION_PRESSURE,
        note_id: 1,
        port_index: -1,
        channel: -1,
        key: -1,
        value: 0.375,
    });
    tune.run_format(0, vec![note(1, 0, 60, 64, true), pressure], None, None, 512);
    hub.run_format(0, vec![], None, None, 512);
    let kinds: Vec<_> = capture
        .display_events()
        .into_iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta) => Some((delta.event.note, delta.event.kind)),
            _ => None,
        })
        .collect();
    let expected = NoteKind::Expression { expression: ExpressionKind::Pressure, value: 0.375 };
    assert!(kinds.contains(&(60, expected)), "{kinds:?}");
}

/// Both a cohort still wholly pending and a sounding set reach the actual
/// 64-cell ceiling. Refused attacks reach neither the wire nor publication;
/// every admitted voice keeps its frozen expression and cut ownership.
#[test]
fn an_onset_past_the_held_ceiling_is_refused_before_copy_and_output() {
    use harmonigraph_take::{CanonicalRecord, NoteKind};
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
    hub.activate();
    // A Just lattice, so the frozen correction below is a real one: on the
    // default 12-TET lattice the 12-TET keyboard keeps every key at its pitch.
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let mut tune = tune();
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
    pair.tune.host_reset();
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

/// A same-key onset replaces the voice State holds for that identity, so the
/// lifetime it replaced will never be addressed by a release. Policy context
/// has to let go of it at the replacement, or the ghost both biases every
/// later assignment and occupies one of the 256 session slots for good.
#[test]
fn a_same_key_retrigger_forgets_the_lifetime_it_replaced_from_policy_context() {
    let _scope = crate::test_scope::enter();
    let mut pair = Pair::new();
    pair.step(vec![note(1, 0, 60, 0, true)]);
    assert!(tuning_of(&pair.idle()).is_some());
    assert_eq!(inspect_hub(&pair.hub, |hub| (hub.test_held(0), hub.test_context())), (1, 1));
    pair.step(vec![note(2, 0, 60, 0, true)]);
    assert!(tuning_of(&pair.idle()).is_some(), "the replacement is assigned in its own right");
    assert_eq!(
        inspect_hub(&pair.hub, |hub| hub.test_voice(0, 0, 60).unwrap().host_note_id),
        2,
        "the fixture reaches the replacement rather than a second cell"
    );
    assert_eq!(
        inspect_hub(&pair.hub, |hub| (hub.test_held(0), hub.test_context())),
        (1, 1),
        "one sounding note is one context voice, not the replaced one as well"
    );
    pair.step(vec![note(2, 0, 60, 0, false)]);
    pair.idle();
    assert_eq!(inspect_tune(&pair.tune, |tune| tune.held()), 0);
    assert_eq!(
        inspect_hub(&pair.hub, |hub| (hub.test_held(0), hub.test_context())),
        (0, 0),
        "and the release of the surviving lifetime empties context"
    );
}

/// Losing a release copy can leave the Hub at capacity after the Tune has
/// room. The Hub must refuse before sending a correction or changing context;
/// the locally admitted note still sounds raw and retains cut ownership.
#[test]
fn a_full_hub_refuses_assignment_before_replying_to_a_locally_admitted_note() {
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
    hub.activate();
    let mut tune = tune();
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
    let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
    hub.activate();
    let mut tune = tune();
    tune.activate();
    tune.shared().request_delay(16);
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

/// #1195. A source that published and then left still answers for itself
/// after a stream-wide display gap, with its row's empty snapshot, so a
/// display that missed its last release stops drawing that note. The Tune
/// plays a note; the Hub's own note floods the display lane past its ring
/// while nothing drains it; the Tune releases its note into that full lane
/// and is removed.
#[test]
fn a_departed_source_answers_after_a_display_gap() {
    use harmonigraph_core::{canonical::CanonicalEvent, NoteTracker, SourceId};
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
    hub.activate();
    let mut tune = Some(tune());
    tune.as_mut().unwrap().activate();
    let mut raw = 0;
    let step = |hub: &mut Device, tune: &Option<Device>, raw: &mut i64, input: Vec<Input>| {
        if let Some(tune) = tune {
            tune.run(*raw, input, None);
        }
        hub.run(*raw, vec![], None);
        *raw += 512;
    };
    let mut display = NoteTracker::new();
    step(&mut hub, &tune, &mut raw, vec![note(7, 0, 57, 0, true)]);
    step(&mut hub, &tune, &mut raw, vec![]);
    let mut heard = None;
    capture.display_into(&mut display, |event, _| {
        if let CanonicalEvent::Note(delta) = event {
            heard = Some(delta.event.source);
        }
    });
    let departed = heard.expect("the Tune's note reached the display");
    assert_ne!(departed, SourceId::DIRECT);
    let held =
        |display: &NoteTracker| display.held_voices().filter(|v| v.source == departed).count();
    assert_eq!(held(&display), 1, "the display holds the Tune's note");

    // The Hub's own note, and more distinct tunings of it than the display
    // ring holds, with nothing draining it.
    step(&mut hub, &tune, &mut raw, vec![]);
    hub.run(raw, vec![note(1, 0, 60, 0, true)], None);
    raw += 512;
    for block in 0..8 {
        let bends = (0..512).map(|i| expression(1, (block * 512 + i) as f64 / 8192.0, 0));
        hub.run(raw, bends.collect(), None);
        raw += 512;
    }
    // The release goes into the full lane, and the Tune leaves.
    step(&mut hub, &tune, &mut raw, vec![note(7, 0, 57, 0, false)]);
    step(&mut hub, &tune, &mut raw, vec![]);
    tune = None;
    let mut gaps = 0;
    let mut answered = false;
    for _ in 0..4 {
        hub.run(raw, vec![], None);
        raw += 512;
        capture.display_into(&mut display, |event, _| match event {
            CanonicalEvent::Gap(_) => {
                gaps += 1;
                answered = false;
            }
            CanonicalEvent::Baseline(frame) if frame.source == departed => answered = true,
            _ => {}
        });
    }
    assert!(gaps > 0, "the fixture must overflow the display lane");
    assert!(answered, "the departed source must answer for itself after the last gap");
    assert_eq!(
        held(&display),
        0,
        "and the display no longer draws the note it lost the release of"
    );
    drop(tune);
}

/// #1151. A MIDI channel bend moves every voice held on its channel and has
/// no delta of its own, so the Hub states each moved voice's new pitch as a
/// bend. Both lanes must draw it as one: a connected ribbon from the onset
/// through every step of the sweep. A snapshot would carry the pitch too, but
/// replaces the row the way a recovery does, breaking the roll before it.
#[test]
fn a_channel_bend_on_a_held_note_reaches_the_display_and_the_take() {
    use harmonigraph_core::NoteTracker;
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
    hub.activate();
    let mut raw = 0;
    let mut step = |input: Vec<Input>| {
        hub.run_format(raw, input, None, None, 512);
        raw += 512;
    };
    step(vec![note(1, 0, 60, 0, true)]);
    step(vec![]);
    // A two-step sweep up the default two-semitone range: 0x50 << 7 is a
    // quarter of the way up, 0x60 << 7 half of it.
    step(vec![raw_midi([0xE0, 0, 0x50], 0)]);
    step(vec![]);
    step(vec![raw_midi([0xE0, 0, 0x60], 0)]);
    step(vec![]);
    let mut display = NoteTracker::new();
    capture.display_into(&mut display, |_, _| {});
    let mut take = NoteTracker::new();
    for record in capture.drain_canonical() {
        record.apply(&mut take).expect("the take lane stays consistent");
    }
    for (lane, tracker) in [("display", &display), ("take", &take)] {
        let voice = tracker.voices().find(|v| v.note == 60);
        let voice = voice.unwrap_or_else(|| panic!("the {lane} lane holds the note"));
        assert_eq!(voice.pitch, 61.0, "the {lane} lane draws the whole sweep");
        let row = tracker.roll().notes().find(|n| n.note == 60).unwrap();
        assert!(row.history_complete, "the {lane} lane saw a bend, not a recovery");
        let segments: Vec<_> = row.segments(100.0).collect();
        let pitches: Vec<_> = segments.iter().map(|((_, p), _)| *p).collect();
        assert_eq!(pitches, [60.0, 60.5, 61.0], "{lane}: {segments:?}");
        assert_eq!(segments[0].0 .0, row.start, "{lane}: the ribbon starts at the onset");
        for pair in segments.windows(2) {
            assert_eq!(pair[0].1 .0, pair[1].0 .0, "{lane}: the ribbon breaks: {segments:?}");
        }
    }
}

#[test]
fn channel_bend_fanout_overflow_is_not_silent() {
    use harmonigraph_take::CanonicalRecord;
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
    hub.activate();
    capture.arm();
    hub.run(0, (0..64).map(|key| note(key, 0, key as i16, 0, true)).collect(), None);
    hub.run(512, vec![], None);
    let _ = capture.drain_canonical();
    let _ = capture.display_events();
    hub.run(1024, (0..33).map(|i| raw_midi([0xE0, i as u8 + 1, 0x40], i)).collect(), None);
    // Where the last bend sounds on the presentation clock, which leaves out
    // the samples between these 64-frame callbacks (#1885).
    let bent = hub.wrapper().test_inspect_plugin(|plugin| plugin.clock.mapping().at(1024 + 32));
    let records = capture.drain_canonical();
    let notes = records.iter().filter(|r| matches!(r, CanonicalRecord::Delta(_))).count();
    let gaps = records.iter().filter(|r| matches!(r, CanonicalRecord::Gap(_))).count();
    let frame = records
        .iter()
        .find_map(|r| match r {
            CanonicalRecord::Baseline(b) => Some(b),
            _ => None,
        })
        .expect("overflow must repair the current held state");
    assert_eq!(frame.voices.len(), 64, "fixture must fill one source's held set");
    let repaired_at = frame.t;
    hub.run(1536, (0..64).map(|key| note(key, 0, key as i16, 0, false)).collect(), None);
    hub.run(2048, vec![], None);
    assert_eq!(notes, 2048, "fixture must exhaust the pending queue");
    assert!(gaps > 0, "lost bend history must be explicit");
    // The last bend sounds at its own sample: Harmonigraph's own input has no
    // delay, and the repair's block starts 32 samples before it.
    assert!(
        repaired_at >= bent,
        "repair must not place the last bend at an earlier retained delta's time"
    );
}

/// One block on a Tune and the Hub, both told the transport at `at` blocks,
/// playing or parked. A Tune at 1x sounds what it is given one block later.
fn take_block(hub: &Device, tune: &Device, raw: i64, played: Vec<Input>, at: f64, playing: bool) {
    let position = timeline(at * 512.0 / 48_000.0, playing);
    tune.run_callback(raw, played, (None, None), (512, false), Some(position));
    hub.run_callback(raw, vec![], (None, None), (512, false), Some(position));
}

/// The notes a take draws for `key`, replayed as the renderer does.
fn drawn(take: &harmonigraph_take::Take, key: u8) -> Vec<(f64, Option<f64>)> {
    let mut tracker = harmonigraph_core::NoteTracker::new();
    for record in &take.events {
        record.apply(&mut tracker).unwrap();
    }
    tracker.roll().notes().filter(|n| n.note == key).map(|n| (n.start, n.end)).collect()
}

/// #1760 on a Tune's row, where a record sounds a block after it is played: a
/// paused take reads whole, every note drawn and on its soundtrack. The first
/// two bend in the last block a take records, so the bend sounds in the pause
/// and is dropped: `parked-at-start` parks at the last played block's own
/// position, and `input-in-parked-block` plays into the first parked block,
/// which still records. `release-in-parked-block` releases there, sounding in
/// the pause: the release is dropped with it, and the resume's snapshot, which
/// no longer lists the note, ends it. A note whose release was not recorded
/// draws without an end.
#[test]
fn a_paused_take_reads_whole_with_a_tunes_records_sounding_in_the_pause() {
    let on = |id, key, t| note(id, 0, key, t, true);
    let off = |id, key, t| note(id, 0, key, t, false);
    // (raw sample, events, transport position in blocks, playing)
    type Step = (i64, Vec<Input>, f64, bool);
    let park = |raw: i64, at: f64| -> Step { (raw, vec![], at, false) };
    let play = |raw: i64, events: Vec<Input>, at: f64| -> Step { (raw, events, at, true) };
    // Per key: the notes drawn, and whether each drew its release.
    type Drawn = &'static [(u8, usize, bool)];
    let cases: [(&str, Vec<Step>, Drawn); 3] = [
        (
            "parked-at-start",
            vec![
                play(0, vec![on(10, 60, 80)], 0.0),
                play(512, vec![expression(10, 0.5, 480)], 1.0),
                park(1024, 1.0),
                park(1536, 1.0),
                play(2048, vec![], 2.0),
                play(2560, vec![], 3.0),
                play(3072, vec![off(10, 60, 0)], 4.0),
            ],
            &[(60, 1, false)],
        ),
        (
            "input-in-parked-block",
            vec![
                play(0, vec![on(10, 60, 80)], 0.0),
                play(512, vec![], 1.0),
                (1024, vec![expression(10, 0.5, 480)], 2.0, false),
                park(1536, 2.0),
                park(2048, 2.0),
                play(2560, vec![], 2.0),
                play(3072, vec![], 3.0),
                play(3584, vec![off(10, 60, 0)], 4.0),
            ],
            &[(60, 1, false)],
        ),
        (
            "release-in-parked-block",
            vec![
                play(0, vec![on(10, 60, 80)], 0.0),
                play(512, vec![], 1.0),
                (1024, vec![off(10, 60, 480)], 2.0, false),
                park(1536, 2.0),
                park(2048, 2.0),
                play(2560, vec![on(12, 64, 16)], 2.0),
                play(3072, vec![off(12, 64, 16)], 3.0),
                play(3584, vec![], 4.0),
            ],
            &[(60, 1, false), (64, 1, true)],
        ),
    ];
    for (name, steps, expected) in cases {
        let _scope = crate::test_scope::enter();
        let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
        let mut tune = tune();
        hub.activate();
        tune.activate();
        capture.arm();
        let dir = std::env::temp_dir()
            .join(format!("harmonigraph-tune-paused-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.take");
        let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone());
        let (mut raw, mut at) = (0, 0.0);
        for (block, events, position, playing) in steps {
            take_block(&hub, &tune, block, events, position, playing);
            (raw, at) = (block, position);
        }
        // Parked from here: the last record sounds and is published, and the
        // run ends.
        for _ in 0..4 {
            raw += 512;
            take_block(&hub, &tune, raw, vec![], at, false);
        }
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
        std::fs::remove_dir_all(dir).unwrap();
    }
}

/// #1771 on a Tune's row: a note struck just before the take is armed sounds a
/// block later, inside the take's first run, whose opening snapshot restates
/// it. The Hub gave it a lifetime when it was struck, so the snapshot continues
/// the note the take recorded rather than opening a second one beside it.
#[test]
fn a_tune_note_restated_by_a_snapshot_stays_one_note_with_its_history() {
    use harmonigraph_take::CanonicalRecord;
    let _scope = crate::test_scope::enter();
    let (mut hub, mut capture) = crate::test_device::recorded(|| hub().with_adaptive());
    let mut tune = tune();
    hub.activate();
    tune.activate();
    let dir =
        std::env::temp_dir().join(format!("harmonigraph-tune-restated-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.take");
    // Struck in a block no take owns; it sounds a block later, once one does.
    take_block(&hub, &tune, 0, vec![note(10, 0, 60, 80, true)], 0.0, true);
    capture.arm();
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone());
    take_block(&hub, &tune, 512, vec![], 1.0, true);
    take_block(&hub, &tune, 1024, vec![], 2.0, true);
    take_block(&hub, &tune, 1536, vec![note(10, 0, 60, 0, false)], 3.0, true);
    for (raw, at) in [(2048, 4.0), (2560, 5.0)] {
        take_block(&hub, &tune, raw, vec![], at, true);
    }
    for raw in [3072, 3584] {
        take_block(&hub, &tune, raw, vec![], 5.0, false);
    }
    writer.drain(&mut capture);
    assert!(!writer.failed());
    let take = harmonigraph_take::Take::read(&path).unwrap();
    let onset = take.events.iter().any(|record| {
        matches!(record.note(), Some(n) if n.note == 60
            && matches!(n.kind, harmonigraph_take::NoteKind::On { .. }))
    });
    assert!(onset, "the onset sounds where the take records");
    let restated = take.events.iter().any(|record| {
        matches!(record, CanonicalRecord::Baseline(frame)
            if frame.voices.iter().any(|voice| voice.note == 60))
    });
    assert!(restated, "and the run's opening snapshot restates it");
    let mut tracker = harmonigraph_core::NoteTracker::new();
    for record in &take.events {
        record.apply(&mut tracker).unwrap();
    }
    let notes: Vec<_> = tracker.roll().notes().filter(|n| n.note == 60).collect();
    assert_eq!(notes.len(), 1, "one press, one note: {:?}", take.events);
    assert!(notes[0].history_complete, "continued with its history");
    assert!(notes[0].end.is_some(), "through the release it recorded");
    drop(writer);
    std::fs::remove_dir_all(dir).unwrap();
}

/// A Tune whose only trouble is missed corrections says so alone, with no
/// "No faults" line above it, and the Sources table and the Tune's own editor
/// say it in the same words (#1841).
#[test]
fn a_tune_with_only_missed_corrections_says_so_without_no_faults() {
    let missed = setup::detail_text(0, 1);
    assert_eq!(missed, "1 note sounded uncorrected - raise the delay and play again");
    assert_eq!(setup::detail_text(0, 0), "No faults");
    let both = setup::detail_text(session::RING_FULL, 2);
    assert_eq!(both.lines().count(), 2, "a fault, then the misses: {both}");
    assert!(both.starts_with("Tuning status: the copy queue is full"), "{both}");
    assert!(!both.contains("raise the delay"), "no remedy for a full queue: {both}");
}
