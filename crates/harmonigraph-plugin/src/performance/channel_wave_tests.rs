//! Delays are reached through actual session reservations and exported CLAP
//! callbacks, including the normal prepare/host/complete path.
use super::*;

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
        let mut hub = Device::new(false);
        hub.configure(uuid, true);
        hub.activate();
        let session = registry::global().lock().unwrap().test_session(uuid);
        let mut target = Device::new(true);
        target.configure(uuid, false);
        target.activate();
        let fillers: Vec<_> = (0..3)
            .map(|_| {
                let mut source = Device::new(true);
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
        assert_eq!(self.session.credits.load(Ordering::Acquire), 0);
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
fn rejected_prefix_repair_is_retried_before_an_owed_midi_off() {
    let _scope = crate::test_scope::enter();
    let fixture = Pressure::new_with_midi(known_seed(), true);
    let old =
        fixture.target.run(128, vec![midi(0, 0xb0, 120, 0, 1), midi(0, 0xb0, 88, 55, 2)], None);
    assert_eq!(old.values.len(), 2);
    fixture.peers(128, false);
    let malformed = Input::Midi(clap_event_midi {
        header: clap_event_header {
            size: std::mem::size_of::<clap_event_header>() as u32,
            ..header::<clap_event_midi>(CLAP_EVENT_MIDI, 0)
        },
        port_index: 0,
        data: [0xf8, 0, 0],
    });
    let rejected = fixture.target.run_status(192, vec![malformed], None, Some(1), 64, true);
    assert_eq!(
        rejected.attempts, 3,
        "repair rejection, accepted retry, then owed Off consume three actual attempts"
    );
    assert_eq!(
        rejected.values,
        [
            (0, Event::Midi { port: 0, data: [0xb0, 88, 0], flags: 0 }),
            (0, Event::Midi { port: 0, data: [0x80, 60, 0], flags: 0 }),
        ]
    );
    assert_eq!(fixture.target.source_snapshot().note_off_owed, 0);
    assert_eq!(fixture.target.source_snapshot().velocity_prefix[0], Some(0));
    fixture.peers(192, false);
    assert!(fixture.target.run(256, vec![], None).values.is_empty());
    fixture.peers(256, false);
    fixture.finish(320);
}

#[test]
fn midi_velocity_prefix_follows_its_original_consumer_and_partial_acceptance_is_factual() {
    let _scope = crate::test_scope::enter();
    for reject in [None, Some(5), Some(6)] {
        let mut seed = known_seed();
        seed.push(midi(0, 0xb0, 88, 37, 3));
        let fixture = Pressure::new_with_midi(seed, true);
        assert_eq!(
            fixture.target.source_snapshot().velocity_prefix[0],
            Some(0),
            "accepted raw A consumed37 without a fake wire reset record"
        );
        let prefix = fixture.target.run(128, vec![midi(0, 0xb0, 88, 55, 8)], None);
        assert_eq!(
            prefix.values,
            [(8, Event::Midi { port: 0, data: [0xb0, 88, 55], flags: 0 })],
            "healthy prefix timing is preserved before its future consumer is even captured"
        );
        fixture.peers(128, false);
        assert!(fixture
            .target
            .run(192, vec![midi(0, 0x90, 64, 88, 4), midi(0, 0x80, 64, 3, 40)], None)
            .values
            .is_empty());
        fixture.peers(192, false);
        let older = fixture.target.run(256, vec![midi(0, 0x80, 60, 2, 16)], None);
        assert_eq!(
            older.values,
            [
                (16, Event::Midi { port: 0, data: [0xb0, 88, 0], flags: 0 }),
                (16, Event::Midi { port: 0, data: [0x80, 60, 2], flags: 0 }),
            ],
            "responsive A Off uses its original zero prefix, not delayed B's55"
        );
        fixture.peers(256, true);
        let younger = fixture.target.run_select(320, vec![], None, reject);
        assert!(
            !younger
                .values
                .iter()
                .any(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 88, 37], .. })),
            "setup never replays A's consumed prefix"
        );
        if reject.is_none() {
            let onset =
                younger.values.iter().position(|(_, event)| event.attack().is_some()).unwrap();
            assert_eq!(
                younger.values[onset - 1],
                (0, Event::Midi { port: 0, data: [0xb0, 88, 55], flags: 0 })
            );
            assert_eq!(
                younger.values[onset],
                (0, Event::Midi { port: 0, data: [0x90, 64, 88], flags: 0 })
            );
            assert!(younger.values.iter().any(|(time, event)| *time == 36
                && matches!(event, Event::Midi { data: [0x80, 64, 3], .. })));
        } else {
            assert!(!younger.values.iter().any(|(_, event)| event.attack().is_some()));
            assert_eq!(
                fixture.target.source_snapshot().held,
                0,
                "control-only acceptance cannot confirm a held voice"
            );
            assert_eq!(fixture.session.credits.load(Ordering::Acquire), 255, "the unaccepted On returns only its preflight credit; foreign releases remain charged until their owners receive acknowledgement");
            assert_eq!(fixture.target.source_snapshot().faults, source::OUTPUT_FAULT);
            if reject == Some(6) {
                assert_eq!(&younger.values[4..], [
                    (0, Event::Midi { port: 0, data: [0xb0, 88, 55], flags: 0 }),
                    (0, Event::Midi { port: 0, data: [0xb0, 88, 0], flags: 0 }),
                ], "accepted prefix and emergency repair are both real records; rejected B is absent");
            }
        }
        assert_eq!(fixture.target.source_snapshot().velocity_prefix[0], Some(0));
        fixture.peers(320, false);
        fixture.finish(384);
    }
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
fn rejected_setup_retains_accepted_facts_and_reset_recovers_latest_input_pedal() {
    let _scope = crate::test_scope::enter();
    let mut seed = known_seed();
    seed.push(midi(0, 0xb0, 64, 127, 3));
    let fixture = Pressure::new(seed);
    fixture.target.run(128, vec![note(2, 0, 64, 4, true), midi(0, 0xb0, 64, 0, 20)], None);
    fixture.peers(128, false);
    fixture.target.run(192, vec![note(1, 0, 60, 16, false)], None);
    fixture.peers(192, true);
    let rejected = fixture.target.run_select(256, vec![], None, Some(3));
    assert_eq!(rejected.values, [
        (0, Event::Midi { port: 0, data: [0xb0, 7, 40], flags: 0 }),
        (0, Event::Midi { port: 0, data: [0xb0, 64, 127], flags: 0 }),
        (0, Event::Midi { port: 0, data: [0xb0, 64, 0], flags: 0 }),
    ], "accepted setup prefix and accepted emergency neutralization are actual output; B never sounded");
    assert_eq!(fixture.target.source_snapshot().faults, source::OUTPUT_FAULT);
    assert!(!fixture.target.source_snapshot().pedals_held);
    fixture.peers(256, false);
    fixture.target.shared().apply(fixture.target.shared().value().routing, true).unwrap();
    for block in 5..=10 {
        fixture.target.main();
        fixture.hub.main();
        assert!(fixture.target.run(block * 64, vec![], None).values.is_empty());
        fixture.peers(block * 64, false);
    }
    assert_eq!(fixture.target.source_snapshot().faults, 0);
    let fresh = fixture.target.run(704, vec![note(3, 0, 60, 4, true)], None);
    assert!(
        fresh.values.iter().any(|(_, event)| event.attack().is_some()),
        "fresh {:?}; {:?}",
        fresh.values,
        fixture.target.source_snapshot()
    );
    fixture.fillers[0].run(
        704,
        (0..64).map(|key| note(key + 1, 0, key as i16, 0, true)).collect(),
        None,
    );
    for source in &fixture.fillers[1..] {
        source.run(704, vec![], None);
    }
    fixture.hub.run(704, vec![], None);
    assert_eq!(fixture.session.credits.load(Ordering::Acquire), 256);
    assert!(fixture
        .target
        .run(768, vec![note(4, 0, 64, 4, true), note(4, 0, 64, 40, false)], None)
        .values
        .is_empty());
    fixture.peers(768, false);
    fixture.target.run(832, vec![note(3, 0, 60, 16, false)], None);
    fixture.peers(832, true);
    let recovered = fixture.target.run(896, vec![], None);
    let onset = recovered.values.iter().position(|(_, event)| event.attack().is_some()).unwrap();
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
    fixture.peers(896, false);
    fixture.finish(960);
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
