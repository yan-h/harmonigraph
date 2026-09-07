//! Delays are reached through actual session reservations and exported CLAP
//! callbacks, including the normal prepare/host/complete path.
use super::*;


fn receiver(hub: &Device, slot: usize) -> (Option<u8>, usize, u64, u64) {
    let wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    wrapper.test_inspect_plugin(|plugin| {
        plugin.aggregation.as_ref().unwrap().test_row_receiver(slot, 0)
    })
}


fn midi(channel: u8, status: u8, a: u8, b: u8, time: u32) -> Input {
    Input::Midi(clap_event_midi {
        header: header::<clap_event_midi>(CLAP_EVENT_MIDI, time),
        port_index: 0,
        data: [status | channel, a, b],
    })
}

struct Pressure {
    hub: Device,
    target: Device,
    fillers: Vec<Device>,
    session: std::sync::Arc<protocol::SessionControl>,
    direct_count: i32,
}

impl Pressure {
    fn new(seed: Vec<Input>) -> Self {
        Self::new_with_midi(seed, false)
    }

    fn new_with_midi(seed: Vec<Input>, raw_midi: bool) -> Self {
        let direct_count = 63 - seed.iter().filter(|input| matches!(input, Input::Note(event) if event.header.type_ == CLAP_EVENT_NOTE_ON)).count() as i32;
        let uuid = SavedUuid::default();
        let mut hub = Device::aggregation(false);
        hub.configure(uuid, true);
        hub.activate();
        let session = registry::global().lock().unwrap().test_session(uuid);
        let mut target = Device::aggregation(true);
        target.configure(uuid, false);
        target.activate();
        let fillers: Vec<_> = (0..3)
            .map(|_| {
                let mut source = Device::aggregation(true);
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
        let mut input = seed;
        input.push(if raw_midi { midi(0, 0x90, 60, 64, 4) } else { note(1, 0, 60, 4, true) });
        let sounded = target.run(64, input, None);
        assert!(
            sounded.values.iter().any(|(_, event)| event.attack().is_some()),
            "initial output {:?}; snapshot {:?}",
            sounded.values,
            target.source_snapshot()
        );
        for source in &fillers {
            source.run(
                64,
                (0..64).map(|key| note(key + 1, 0, key as i16, 0, true)).collect(),
                None,
            );
        }
        hub.run(
            64,
            (0..direct_count).map(|key| note(key + 1, 0, key as i16, 0, true)).collect(),
            None,
        );
        assert_eq!(session.credits.load(Ordering::Acquire), 256);
        Self { hub, target, fillers, session, direct_count }
    }

    fn peers(&self, sample: i64, release: bool) {
        for (index, source) in self.fillers.iter().enumerate() {
            source.run(
                sample,
                if release && index == 0 {
                    (0..64).map(|key| note(key + 1, 0, key as i16, 0, false)).collect()
                } else {
                    vec![]
                },
                None,
            );
        }
        self.hub.run(sample, vec![], None);
    }

    fn finish(self, sample: i64) {
        self.target.shared().apply(self.target.shared().value().routing, true).unwrap();
        for source in &self.fillers {
            source.shared().apply(source.shared().value().routing, true).unwrap();
        }
        for block in 0..8 {
            self.target.run(sample + block * 64, vec![], None);
            for source in &self.fillers {
                source.run(sample + block * 64, vec![], None);
            }
            self.hub.run(
                sample + block * 64,
                if block == 0 {
                    (0..self.direct_count)
                        .map(|key| note(key + 1, 0, key as i16, 0, false))
                        .collect()
                } else {
                    vec![]
                },
                None,
            );
        }
        assert_eq!(
            self.session.credits.load(Ordering::Acquire),
            0,
            "target {:?}; peers {:?}",
            self.target.source_snapshot(),
            self.fillers.iter().map(Device::source_snapshot).collect::<Vec<_>>()
        );
    }
}

#[test]
fn channel_gate_precedes_any_control_and_indexed_voices_cross_retained_history() {
    let _scope = crate::test_scope::enter();
    let mut seed = known_seed();
    seed.push(note(11, 1, 67, 3, true));
    let fixture = Pressure::new(seed);
    assert!(fixture.target.run(128, vec![note(2, 0, 64, 4, true)], None).values.is_empty());
    fixture.peers(128, true);
    assert!(fixture.target.run(192, vec![], None).values.is_empty());
    fixture.peers(192, false);
    assert!(fixture.session.credits.load(Ordering::Acquire) < 256);
    assert!(fixture.target.run(256, vec![], None).values.is_empty(), "free credits cannot admit a different translation over the established channel even before any controller");
    fixture.peers(256, false);
    for block in 5..48 {
        let output = fixture.target.run(
            block * 64,
            (0..48).map(|value| midi(0, 0xb0, 7, value, 0)).collect(),
            None,
        );
        assert_eq!(output.values.len(), 48);
        fixture.peers(block * 64, false);
    }
    assert!(
        fixture.target.source_snapshot().pending > 2048,
        "retained younger history exceeds the whole scheduling visit slice"
    );
    let output = fixture.target.run(
        48 * 64,
        vec![expression(1, 0.125, 8), expression(11, 0.25, 12), note(11, 1, 67, 13, false)],
        None,
    );
    assert_eq!(output.values.iter().map(|(time, _)| *time).collect::<Vec<_>>(), [8, 12, 13]);
    assert!(matches!(output.values[0].1, Event::Expression { id: 1, value: 0.125, .. }));
    assert!(matches!(
        output.values[1].1,
        Event::Expression { id: 11, channel: 1, value: 0.25, .. }
    ));
    assert_eq!(fixture.target.source_snapshot().faults, 0);
    fixture.peers(48 * 64, false);
    fixture.finish(49 * 64);
}

#[test]
fn delayed_sostenuto_replays_its_edge_after_the_originally_preceding_onset() {
    let _scope = crate::test_scope::enter();
    let fixture = Pressure::new(known_seed());
    let old = fixture.target.run(
        128,
        vec![note(2, 0, 64, 4, true), midi(0, 0xb0, 66, 127, 20), note(2, 0, 64, 40, false)],
        None,
    );
    assert_eq!(old.values, [(20, Event::Midi { port: 0, data: [0xb0, 66, 127], flags: 0 })]);
    fixture.peers(128, false);
    let old =
        fixture.target.run(192, vec![note(1, 0, 60, 8, false), midi(0, 0xb0, 66, 0, 24)], None);
    assert_eq!(old.values.iter().map(|(time, _)| *time).collect::<Vec<_>>(), [8, 24]);
    fixture.peers(192, true);
    let young = fixture.target.run(256, vec![], None);
    let onset = young.values.iter().position(|(_, event)| event.attack().is_some()).unwrap();
    let down = young
        .values
        .iter()
        .position(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 66, 127], .. }))
        .unwrap();
    assert!(onset < down, "sostenuto must capture B, which preceded its edge at input time");
    assert_eq!((young.values[onset].0, young.values[down].0), (0, 16));
    assert!(young.values.iter().any(|(time, event)| *time == 36 && event.release()));
    fixture.peers(256, false);
    let up = fixture.target.run(320, vec![], None);
    assert_eq!(up.values, [(20, Event::Midi { port: 0, data: [0xb0, 66, 0], flags: 0 })]);
    assert!(!fixture.target.source_snapshot().pedals_held);
    fixture.peers(320, false);
    fixture.finish(384);
}

fn known_seed() -> Vec<Input> {
    vec![
        midi(0, 0xb0, 7, 40, 0),
        midi(0, 0xb0, 64, 0, 1),
        midi(0, 0xb0, 66, 0, 2),
        midi(0, 0xb0, 69, 0, 3),
    ]
}

#[test]
fn delayed_setup_keeps_bank_latches_and_parameter_transactions_in_raw_order() {
    let _scope = crate::test_scope::enter();
    let prelude = [
        (0xb0, 99, 1),
        (0xb0, 98, 2),
        (0xb0, 6, 3),
        (0xb0, 38, 4),
        (0xb0, 96, 0),
        (0xb0, 97, 0),
        (0xb0, 101, 127),
        (0xb0, 100, 127),
        (0xb0, 0, 1),
        (0xb0, 32, 2),
        (0xc0, 3, 0),
        (0xb0, 0, 4),
        (0xb0, 32, 5),
    ];
    let later = [
        (0xb0, 0, 6),
        (0xb0, 32, 7),
        (0xc0, 8, 0),
        (0xb0, 99, 3),
        (0xb0, 98, 4),
        (0xb0, 6, 8),
        (0xb0, 38, 9),
        (0xb0, 96, 0),
        (0xb0, 97, 0),
        (0xb0, 101, 0),
        (0xb0, 100, 0),
        (0xb0, 6, 12),
        (0xb0, 38, 0),
        (0xb0, 96, 0),
        (0xb0, 97, 0),
        (0xb0, 101, 127),
        (0xb0, 100, 127),
    ];
    let mut seed = known_seed();
    seed.extend(prelude.map(|(status, a, b)| midi(0, status, a, b, 3)));
    let fixture = Pressure::new(seed);
    let mut input = vec![note(2, 0, 64, 4, true)];
    input.extend(later.map(|(status, a, b)| midi(0, status, a, b, 20)));
    input.push(note(2, 0, 64, 40, false));
    let old = fixture.target.run(128, input, None);
    let expected = |values: &[(u8, u8, u8)], time| {
        values
            .iter()
            .map(|&(status, a, b)| (time, Event::Midi { port: 0, data: [status, a, b], flags: 0 }))
            .collect::<Vec<_>>()
    };
    assert_eq!(old.values, expected(&later, 20));
    fixture.peers(128, false);
    fixture.target.run(192, vec![note(1, 0, 60, 16, false)], None);
    fixture.peers(192, true);
    let young = fixture.target.run(256, vec![], None);
    let onset = young.values.iter().position(|(_, event)| event.attack().is_some()).unwrap();
    assert_eq!(&young.values[onset - prelude.len()..onset], expected(&prelude, 0));
    assert_eq!(&young.values[onset + 1..onset + 1 + later.len()], expected(&later, 16));
    assert!(young.values.last().is_some_and(|(time, event)| *time == 36 && event.release()));
    fixture.peers(256, false);
    fixture.target.run(320, vec![], None);
    fixture.peers(320, false);
    fixture.target.run(384, vec![], None);
    assert!(
        fixture.target.source_snapshot().pending > 0,
        "raw transaction history remains owned after musical work settles"
    );
    assert_eq!(
        fixture.target.shared().extra_delay.load(Ordering::Relaxed),
        0,
        "acknowledged neutral musical idle clears delay despite retained reconstruction history"
    );
    fixture.peers(384, false);
    fixture.finish(448);
}

#[test]
fn each_setup_claims_the_actual_lease_before_host_acceptance() {
    let _scope = crate::test_scope::enter();
    for before in [true, false] {
        let fixture = Pressure::new(known_seed());
        fixture.target.run(128, vec![note(2, 0, 64, 4, true)], None);
        fixture.peers(128, false);
        fixture.target.run(192, vec![note(1, 0, 60, 8, false)], None);
        fixture.peers(192, true);
        let slot = usize::from(attachment_tests::lease(&fixture.target).unwrap().slot - 1);
        let credits = fixture.session.credits.load(Ordering::Acquire);
        SETUP_CLOSE
            .with(|hook| *hook.borrow_mut() = Some((fixture.session.clone(), slot, before, None)));
        let output = fixture.target.run(256, vec![], None);
        let (_, _, _, observed) = SETUP_CLOSE.with(|hook| hook.borrow_mut().take().unwrap());
        assert_eq!(observed, Some(if before { source::OPEN } else { source::BUSY }));
        assert_eq!(
            output.attempts,
            usize::from(!before),
            "only an already claimed setup may finish"
        );
        assert!(output.values.iter().all(|(_, event)| event.attack().is_none()));
        assert!(
            fixture.session.credits.load(Ordering::Acquire) <= credits,
            "setup never reserves a voice"
        );
        let row = &fixture.session.rows[slot];
        assert_ne!(row.emission_gate.load(Ordering::Acquire), source::BUSY);
        row.withdrawn.store(false, Ordering::Release);
        row.emission_gate.store(source::OPEN, Ordering::Release);
        fixture.peers(256, false);
        let resumed = fixture.target.run(320, vec![], None);
        assert!(resumed.values.iter().any(|(_, event)| event.attack().is_some()));
        fixture.peers(320, false);
        fixture.finish(384);
    }
}

#[test]
fn stop_cancels_an_unattempted_prefix_before_an_already_captured_new_consumer() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::aggregation(false);
    hub.configure(uuid, true);
    hub.activate();
    let mut source = Device::aggregation(true);
    source.configure(uuid, false);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    let mut seed = known_seed();
    seed.extend([midi(0, 0xb0, 88, 0, 3), transport(3, 120.0)]);
    seed.extend((0..64).map(|id| note(id + 1, 0, id as i16, 4, true)));
    let seed = source.run(64, seed, None);
    assert_eq!(seed.values.iter().filter(|(_, event)| event.attack().is_some()).count(), 64);
    hub.run(64, vec![], None);
    source.run(128, vec![], None);
    hub.run(128, vec![], None);
    let mut stop = transport(0, 120.0);
    let Input::Transport(ref mut event) = stop else { unreachable!() };
    event.flags &= !CLAP_TRANSPORT_IS_PLAYING;
    let mut inputs: Vec<_> = (0..512).map(|index| expression(index % 64 + 1, 0.125, 0)).collect();
    inputs.push(midi(0, 0xb0, 88, 55, 1));
    let cut = source.source_snapshot().input_cut;
    let full = source.run(192, inputs, None);
    assert_eq!(
        source.source_snapshot().input_cut,
        cut + 513,
        "all expressions and the final55 were captured"
    );
    assert!(
        full.attempts > 0 && full.attempts < 512,
        "shared visit pressure leaves captured55 unattempted"
    );
    assert!(!full
        .values
        .iter()
        .any(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 88, 55], .. })));
    hub.run(192, vec![], None);
    let mut raw = 256;
    for block in 4..(2 * 512 + 64) {
        let next = source.run(
            block * 64,
            if block == 4 {
                vec![note(1, 0, 0, 0, false), stop, midi(0, 0x90, 60, 64, 4)]
            } else {
                vec![]
            },
            (block == 4).then_some(CLAP_EVENT_NOTE_CHOKE),
        );
        if block == 4 {
            assert!(next.values.iter().any(|(_, event)| matches!(event, Event::Note { kind: CLAP_EVENT_NOTE_OFF, id: 1, .. })), "canceling an old inline expression must preserve its sounded life's essential original Off when emergency Choke is rejected");
        }
        assert!(
            !next.values.iter().any(|(_, event)| event.attack().is_some()),
            "real rejected Choke inhibits new nonessential input until Reset"
        );
        assert!(
            !next
                .values
                .iter()
                .any(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 88, 55], .. })),
            "unattempted canceled55 cannot escape through the captured note"
        );
        hub.run(block * 64, vec![], None);
        raw = (block + 1) * 64;
        if source.source_snapshot().pending == 0 {
            break;
        }
    }
    assert_eq!(source.source_snapshot().faults, source::OUTPUT_FAULT);
    assert_eq!(
        source.source_snapshot().pending,
        0,
        "terminal cancellation must finish independently of the latch: {:?}",
        source.source_snapshot()
    );
    source.shared().apply(source.shared().value().routing, true).unwrap();
    for _ in 0..16 {
        source.run(raw, vec![], None);
        hub.run(raw, vec![], None);
        raw += 64;
    }
    assert_eq!(source.source_snapshot().faults, 0);
    assert!(
        attachment_tests::lease(&source).is_none(),
        "Reset returned the settled lease; a main-thread rematch is still requested"
    );
    source.main();
    hub.main();
    for _ in 0..4 {
        source.run(raw, vec![], None);
        hub.run(raw, vec![], None);
        raw += 64;
    }
    assert!(
        attachment_tests::lease(&source).is_some(),
        "real host main callbacks prepare the fresh lease"
    );
    let new = source.run(raw, vec![midi(0, 0x90, 60, 64, 4), midi(0, 0x80, 60, 0, 12)], None);
    assert_eq!(
        new.values.iter().filter(|(_, event)| event.attack().is_some()).count(),
        1,
        "fresh post-Reset input resumes: {:?}; accepted {:?}",
        source.source_snapshot(),
        new.values
    );
    assert!(!new
        .values
        .iter()
        .any(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 88, 55], .. })));
    hub.run(raw, vec![], None);
    source.run(raw + 64, vec![], None);
    hub.run(raw + 64, vec![], None);
    source.run(raw + 128, vec![], None);
    hub.run(raw + 128, vec![], None);
    assert_eq!(source.source_snapshot().pending, 0);
    assert_eq!(source.source_snapshot().held, 0);
}

#[test]
fn captured_stop_blocks_partial_setup_even_when_stop_work_cannot_advance() {
    let _scope = crate::test_scope::enter();
    let mut seed = known_seed();
    seed.push(midi(0, 0xb0, 0, 1, 3));
    let fixture = Pressure::new(seed);
    for block in 2..10 {
        let output = fixture.target.run(
            block * 64,
            (0..80).map(|value| midi(0, 0xb0, 7, value, 0)).collect(),
            None,
        );
        assert_eq!(output.values.len(), 80);
        fixture.peers(block * 64, false);
    }
    let Input::Transport(playing) = transport(0, 120.0) else { unreachable!() };
    assert!(fixture
        .target
        .run_callback(640, vec![note(2, 0, 64, 4, true)], (None, None), (64, false), Some(playing))
        .values
        .is_empty());
    fixture.peers(640, false);
    fixture.target.run(704, vec![note(1, 0, 60, 16, false)], None);
    fixture.peers(704, true);
    let partial = fixture.target.run(768, vec![], None);
    assert_eq!(
        partial.attempts, 512,
        "retained raw prelude reaches a genuinely partial accepted setup"
    );
    assert!(partial.values.iter().all(|(_, event)| matches!(event, Event::Midi { .. })));
    fixture.peers(768, false);
    let mut stopped = playing;
    stopped.flags &= !CLAP_TRANSPORT_IS_PLAYING;
    let mut input: Vec<_> = (0..64).map(|key| note(key + 20, 1, key as i16, 0, true)).collect();
    input.extend((0..29).map(|value| midi(1, 0xb0, 7, value, 0)));
    // Stop capture costs64 and29 complete64-target groups cost1856 visits;
    // the remaining128 cannot run the Stop's192-visit advance operation.
    let blocked = fixture.target.run_callback(832, input, (None, None), (64, false), Some(stopped));
    assert_eq!(blocked.attempts, 0, "the captured Stop horizon inhibits setup preparation independently of cancellation progress");
    assert!(fixture.target.source_snapshot().references >= 29 * 64);
    fixture.peers(832, false);
    fixture.finish(896);
}

#[test]
fn channel_termination_captures_each_wave_and_sound_off_keeps_physical_offs() {
    let _scope = crate::test_scope::enter();
    for cc in [120, 123] {
        let fixture = Pressure::new(known_seed());
        let old = fixture.target.run(
            128,
            vec![note(2, 0, 64, 4, true), midi(0, 0xb0, cc, 0, 20), note(2, 0, 64, 40, false)],
            None,
        );
        assert_eq!(old.values, [(20, Event::Midi { port: 0, data: [0xb0, cc, 0], flags: 0 })]);
        assert_eq!(fixture.target.source_snapshot().note_off_owed, usize::from(cc == 120));
        fixture.peers(128, false);
        let young = fixture.target.run(192, vec![note(1, 0, 60, 16, false)], None);
        let onset =
            young.values.iter().position(|(_, event)| event.attack().is_some()).unwrap_or_else(
                || panic!("cc{cc}: {:?}, {:?}", young.values, fixture.target.source_snapshot()),
            );
        let start = young.values[onset].0;
        if cc == 120 {
            assert!(
                young.values[..onset].iter().any(|(_, event)| matches!(
                    event,
                    Event::Note { id: 1, kind: CLAP_EVENT_NOTE_OFF, .. }
                )),
                "the owed physical A Off precedes the next wave"
            );
            assert!(young.values.iter().any(|(time, event)| *time == start + 36
                && matches!(event, Event::Note { id: 2, kind: CLAP_EVENT_NOTE_OFF, .. })));
        }
        assert_eq!(young.values.iter().filter(|(_, event)| matches!(event, Event::Midi { data: [0xb0, value, 0], .. } if *value == cc)).count(), 1, "one physical acceptance terminates the younger captured wave");
        assert!(young.values.iter().any(|(time, event)| *time == start + 16
            && matches!(event, Event::Midi { data: [0xb0, value, 0], .. } if *value == cc)));
        assert_eq!(fixture.target.source_snapshot().note_off_owed, 0);
        fixture.peers(192, false);
        fixture.finish(256);
    }
}

#[test]
fn established_controls_survive_withdrawal_before_a_future_initial_snapshot_ack() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::aggregation(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::aggregation(true);
    source.configure_offset(uuid, true, 65536);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    let mut seed = known_seed();
    seed.push(note(73, 0, 60, 4, true));
    let on = source.run(64, seed, None);
    assert_eq!(on.values.len(), 5);
    assert!(matches!(on.values[4], (4, Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 73, .. })));
    let wrapper = unsafe {
        &*((*source.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
    };
    let (lease, adopted, acknowledged, _) =
        wrapper.test_inspect_plugin(|plugin| plugin.source.as_ref().unwrap().test_stream_status());
    assert!(adopted && !acknowledged);
    assert_eq!(source.source_snapshot().baseline_cut, Some(0));
    let row = &session.rows[usize::from(lease.unwrap().slot - 1)];
    assert_eq!(row.emission_gate.load(Ordering::Acquire), source::OPEN);
    let shared = source.shared();
    let setup::Routing::Source(mut routing) = shared.value().routing else { unreachable!() };
    routing.selected = Some(SavedUuid::default());
    shared.apply(setup::Routing::Source(routing), false).unwrap();
    source.main();
    hub.run(64, vec![], None);
    assert!(row.withdrawn.load(Ordering::Acquire));
    assert_eq!(row.emission_gate.load(Ordering::Acquire), source::CLOSED);
    assert_eq!(source.source_snapshot().faults, 0);
    assert_eq!(source.source_snapshot().held, 1);
    assert_eq!(source.source_snapshot().baseline_cut, Some(0));
    let output = source.run(
        128,
        vec![
            expression(73, 0.1234567890123, 4),
            midi(0, 0xb0, 11, 90, 8),
            note(73, 0, 60, 16, false),
        ],
        None,
    );
    assert_eq!(source.source_snapshot().faults, 0);
    assert!(output.values.iter().any(|(time, event)| *time == 4
        && matches!(event, Event::Expression { id: 73, value, .. } if *value == 0.1234567890123)),
        "an established expression must not wait for the future baseline ACK after withdrawal");
    assert!(output.values.iter().any(
        |(time, event)| *time == 8 && matches!(event, Event::Midi { data: [0xb0, 11, 90], .. })
    ));
    assert!(output.values.iter().any(|(time, event)| *time == 16
        && matches!(event, Event::Note { kind: CLAP_EVENT_NOTE_OFF, id: 73, .. })));
    assert_eq!(source.source_snapshot().baseline_cut, Some(0));
    assert_eq!(session.credits.load(Ordering::Acquire), 1);
    drop(hub);
    drop(source);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn a_join_floor_retry_keeps_new_stream_controls_behind_the_fresh_zero_cut_baseline() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut source = Device::aggregation(true);
    source.configure(uuid, false);
    source.activate();
    source.run(0, vec![], None);
    let mut hub = Device::aggregation(false);
    hub.configure(uuid, true);
    hub.activate();
    hub.run(0, vec![], None);
    hub.run(64, vec![], None);
    source.main();
    source.run(64, vec![], None);
    let wrapper = unsafe {
        &*((*source.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
    };
    let (_, adopted, joined, coverage) =
        wrapper.test_inspect_plugin(|plugin| plugin.source.as_ref().unwrap().test_stream_status());
    assert!(adopted && !joined);
    assert_eq!(
        coverage.unwrap().start,
        64,
        "the initial snapshot starts behind Hub's published128 frontier"
    );
    hub.run(128, vec![], None);
    let retry = source.run(128, vec![midi(0, 0xb0, 88, 37, 2), midi(0, 0x90, 60, 64, 4)], None);
    let (_, _, joined, coverage) =
        wrapper.test_inspect_plugin(|plugin| plugin.source.as_ref().unwrap().test_stream_status());
    assert_eq!(coverage.unwrap().start, 128, "the real Hub reply moved the join floor");
    assert!(
        !joined && retry.values.is_empty(),
        "acknowledging an obsolete zero-cut snapshot is not stream admission"
    );
    hub.run(192, vec![], None);
    let output = source.run(192, vec![], None);
    assert!(output
        .values
        .iter()
        .any(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 88, 37], .. })));
    assert!(output.values.iter().any(|(_, event)| event.attack().is_some()));
    hub.run(256, vec![], None);
    let (lease, _, joined, _) =
        wrapper.test_inspect_plugin(|plugin| plugin.source.as_ref().unwrap().test_stream_status());
    assert!(joined);
    // CC88 is an ordinary shared controller now: the receiver mirror holds the
    // value the host actually accepted rather than a prefix cleared by the Note-On.
    assert_eq!(receiver(&hub, usize::from(lease.unwrap().slot - 1)), (Some(37), 1, 2, 2));
    source.run(256, vec![midi(0, 0x80, 60, 0, 4)], None);
    hub.run(320, vec![], None);
    source.run(320, vec![], None);
    hub.run(384, vec![], None);
    source.run(384, vec![], None);
    assert_eq!(source.source_snapshot().held, 0);
}

#[test]
fn rejected_setup_retains_accepted_facts_and_reset_recovers_latest_input_pedal() {
    let _scope = crate::test_scope::enter();
    for raw_prelude in [false, true] {
        let mut seed = known_seed();
        if raw_prelude {
            seed.push(midi(0, 0xb0, 0, 1, 3));
        }
        seed.push(midi(0, 0xb0, 64, 127, 3));
        if raw_prelude {
            seed.push(midi(0, 0xe0, 64, 64, 3));
            seed.push(midi(0, 0xb0, 1, 90, 3));
        }
        let fixture = Pressure::new(seed);
        let mut input = vec![note(2, 0, 64, 4, true), midi(0, 0xb0, 64, 0, 20)];
        if !raw_prelude {
            input.push(midi(0, 0xe0, 64, 64, 24));
        }
        let original = fixture.target.run(128, input, None);
        if !raw_prelude {
            assert_eq!(
                original.values,
                [
                    (20, Event::Midi { port: 0, data: [0xb0, 64, 0], flags: 0 }),
                    (24, Event::Midi { port: 0, data: [0xe0, 64, 64], flags: 0 }),
                ]
            );
        }
        fixture.peers(128, false);
        fixture.target.run(192, vec![note(1, 0, 60, 16, false)], None);
        fixture.peers(192, true);
        let rejected =
            fixture.target.run_select(256, vec![], None, Some(if raw_prelude { 8 } else { 3 }));
        let expected = if raw_prelude {
            vec![
                (0, Event::Midi { port: 0, data: [0xb0, 7, 40], flags: 0 }),
                (0, Event::Midi { port: 0, data: [0xb0, 64, 0], flags: 0 }),
                (0, Event::Midi { port: 0, data: [0xb0, 66, 0], flags: 0 }),
                (0, Event::Midi { port: 0, data: [0xb0, 69, 0], flags: 0 }),
                (0, Event::Midi { port: 0, data: [0xb0, 0, 1], flags: 0 }),
                (0, Event::Midi { port: 0, data: [0xb0, 64, 127], flags: 0 }),
                (0, Event::Midi { port: 0, data: [0xe0, 64, 64], flags: 0 }),
                (0, Event::Midi { port: 0, data: [0xb0, 64, 0], flags: 0 }),
            ]
        } else {
            vec![
                (0, Event::Midi { port: 0, data: [0xb0, 7, 40], flags: 0 }),
                (0, Event::Midi { port: 0, data: [0xb0, 64, 127], flags: 0 }),
                (0, Event::Midi { port: 0, data: [0xb0, 64, 0], flags: 0 }),
            ]
        };
        assert_eq!(rejected.values, expected, "accepted setup prefix and accepted emergency neutralization are actual output; B never sounded");
        assert_eq!(fixture.target.source_snapshot().faults, source::OUTPUT_FAULT);
        assert!(!fixture.target.source_snapshot().pedals_held);
        fixture.peers(256, false);
        let shared = fixture.target.shared();
        shared.apply(shared.value().routing, true).unwrap();
        let reset = shared.value().generation;
        let wrapper = unsafe {
            &*((*fixture.target.plugin)
                .plugin_data
                .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
        };
        let mut fresh_raw = 0;
        for block in 5..32 {
            fixture.target.main();
            fixture.hub.main();
            assert!(fixture.target.run(block * 64, vec![], None).values.is_empty());
            fixture.peers(block * 64, false);
            let admitted = wrapper.test_inspect_plugin(|plugin| {
                let source = plugin.source.as_ref().unwrap();
                let (_, adopted, joined, coverage) = source.test_stream_status();
                adopted && joined && coverage.is_some()
            });
            if shared.applied.load(Ordering::Acquire) == reset
                && fixture.target.source_snapshot().faults == 0
                && admitted
            {
                fresh_raw = (block + 1) * 64;
                break;
            }
        }
        assert_ne!(
            fresh_raw,
            0,
            "{}; {:?}",
            wrapper.test_inspect_plugin(|plugin| plugin
                .source
                .as_ref()
                .unwrap()
                .test_reset_progress()),
            fixture.target.source_snapshot()
        );

        if !raw_prelude {
            assert_eq!(
                (
                    fixture.target.source_snapshot().pending,
                    fixture.target.source_snapshot().references
                ),
                (0, 0),
                "the accepted younger bend has retired into the folded checkpoint"
            );
        }
        let fresh = fixture.target.run(fresh_raw, vec![note(3, 0, 60, 4, true)], None);
        assert!(
            fresh.values.iter().any(|(_, event)| event.attack().is_some()),
            "fresh {:?}; {:?}",
            fresh.values,
            fixture.target.source_snapshot()
        );
        fixture.fillers[0].run(
            fresh_raw,
            (0..64).map(|key| note(key + 1, 0, key as i16, 0, true)).collect(),
            None,
        );
        for source in &fixture.fillers[1..] {
            source.run(fresh_raw, vec![], None);
        }
        fixture.hub.run(fresh_raw, vec![], None);
        assert_eq!(fixture.session.credits.load(Ordering::Acquire), 256);
        assert!(fixture
            .target
            .run(fresh_raw + 64, vec![note(4, 0, 64, 4, true), note(4, 0, 64, 40, false)], None)
            .values
            .is_empty());
        fixture.peers(fresh_raw + 64, false);
        fixture.target.run(fresh_raw + 128, vec![note(3, 0, 60, 16, false)], None);
        fixture.peers(fresh_raw + 128, true);
        let recovered = fixture.target.run(fresh_raw + 192, vec![], None);
        let onset =
            recovered.values.iter().position(|(_, event)| event.attack().is_some()).unwrap();
        assert!(recovered.values[..onset]
            .iter()
            .any(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 64, 0], .. })));
        assert!(
            !recovered
                .values
                .iter()
                .any(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 64, 127], .. })),
            "the acknowledged Reset must not resurrect an overwritten input pedal value"
        );
        assert!(
            recovered.values[..onset]
                .iter()
                .any(|(_, event)| matches!(event, Event::Midi { data: [0xe0, 64, 64], .. })),
            "pedal neutralization must preserve bend8256, not bend64; raw_prelude={raw_prelude}"
        );
        fixture.peers(fresh_raw + 192, false);
        fixture.finish(fresh_raw + 256);
    }
}

#[test]
fn delayed_channel_restores_input_setup_then_replays_gestures_at_original_offsets() {
    let _scope = crate::test_scope::enter();
    let fixture = Pressure::new(known_seed());
    let old = fixture.target.run(
        128,
        vec![
            note(2, 0, 64, 4, true),
            midi(0, 0xb0, 7, 80, 20),
            expression(2, 0.1234567890123456, 28),
            note(2, 0, 64, 40, false),
        ],
        None,
    );
    assert_eq!(
        old.values,
        [(20, Event::Midi { port: 0, data: [0xb0, 7, 80], flags: 0 })],
        "the established voice receives its controller while the younger note has no credit"
    );
    fixture.peers(128, false);
    let released = fixture.target.run(192, vec![note(1, 0, 60, 16, false)], None);
    assert_eq!(released.values.len(), 1);
    assert_eq!(released.values[0].0, 16);
    fixture.peers(192, true);
    let young = fixture.target.run(256, vec![], None);
    let onset = young.values.iter().position(|(_, event)| event.attack().is_some()).unwrap();
    assert_eq!(young.values[onset].0, 0);
    assert!(young.values[..onset]
        .iter()
        .any(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 7, 40], .. })));
    assert!(young.values.iter().any(
        |(time, event)| *time == 16 && matches!(event, Event::Midi { data: [0xb0, 7, 80], .. })
    ));
    assert!(young.values.iter().any(|(time, event)| *time == 24 && matches!(event, Event::Expression { id: 2, value, .. } if *value == 0.1234567890123456)));
    assert!(young.values.iter().any(|(time, event)| *time == 36
        && matches!(event, Event::Note { id: 2, kind: CLAP_EVENT_NOTE_OFF, .. })));
    fixture.peers(256, false);
    fixture.finish(320);
}
