//! Ordinary production progress, with physical acceptance and observed DIRECT
//! publication asserted independently through the actual exported factory.
use super::*;

fn inspect_hub<R>(device: &Device, f: impl FnOnce(&hub::Hub) -> R) -> R {
    let wrapper = unsafe {
        &*((*device.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    wrapper.test_inspect_plugin(|plugin| f(plugin.aggregation.as_ref().unwrap()))
}

#[test]
fn observation_direct_replays_known_channel_setup_after_bounded_output_delay() {
    let _scope = crate::test_scope::enter();
    // Isolate physical forwarding/prefix replay: this burst includes 1025 events at one sample and
    // exceeds the musical cohort bound and is not a sequencing fixture.
    let mut hub = Device::aggregation(false);
    hub.activate();
    let midi = |data| {
        Input::Midi(clap_event_midi {
            header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
            port_index: 0,
            data,
        })
    };
    let mut seed: Vec<_> = [64, 66, 69].map(|cc| midi([0xb0, cc, 0])).into();
    seed.push(note(1, 0, 60, 1, true));
    assert_eq!(hub.run(0, seed, None).values.len(), 4);
    hub.run(64, vec![], None);
    let mut input = vec![note(1, 0, 60, 0, false)];
    input.extend((0..1024).map(|_| midi([0xf8, 0, 0])));
    input.extend([note(2, 0, 62, 1, true), note(2, 0, 62, 2, false)]);
    let mut output: Vec<_> = hub
        .run(128, input, None)
        .values
        .into_iter()
        .map(|(time, event)| (128 + i64::from(time), event))
        .collect();
    assert!(
        !output.iter().any(|(_, event)| event.attack().is_some()),
        "the real callback grant must delay the second onset"
    );
    for block in 3..67 {
        output.extend(
            hub.run(block * 64, vec![], None)
                .values
                .into_iter()
                .map(|(time, event)| (block * 64 + i64::from(time), event)),
        );
        let state = inspect_hub(&hub, |hub| hub.direct.test_snapshot());
        assert_eq!(state.faults, 0);
        if state.pending == 0 && state.captures == 0 && state.lives == 0 {
            break;
        }
    }
    assert_eq!(output.iter().filter(|(_, event)| event.attack().is_some()).count(), 1);
    assert_eq!(output.iter().filter(|(_, event)| event.release()).count(), 2);
    let onset = output.iter().find(|(_, event)| event.attack().is_some()).unwrap().0;
    let release = output
        .iter()
        .find(|(_, event)| matches!(event, Event::Note { kind: CLAP_EVENT_NOTE_OFF, id: 2, .. }))
        .unwrap()
        .0;
    assert!(onset > 129, "the new gesture is actually delayed");
    assert_eq!(release, onset + 1, "both events retain the same translation");
    assert_eq!(
        output
            .iter()
            .filter(|(_, event)| matches!(event, Event::Midi { data: [0xf8, _, _], .. }))
            .count(),
        1024
    );
    for cc in [64, 66, 69] {
        assert!(output.iter().any(|(_, event)| matches!(event, Event::Midi { data: [0xb0, actual, 0], .. } if *actual == cc)), "actual known setup must be replayed");
    }
    let state = inspect_hub(&hub, |hub| hub.direct.test_snapshot());
    assert_eq!(
        (state.held, state.pending, state.captures, state.lives, state.journal),
        (0, 0, 0, 0, 0)
    );
    assert!(hub.shared().adopted().unwrap().valid);
}

#[test]
fn production_initial_direct_survives_settled_host_reactivation_without_route_calibration() {
    let _scope = crate::test_scope::enter();
    let mut hub = Device::new(false);
    hub.activate();
    for pass in 0..2 {
        let output = hub.run(0, vec![note(27, 0, 60, 3, true), note(27, 0, 60, 19, false)], None);
        assert_eq!(
            output.values,
            [
                (
                    3,
                    Event::Note {
                        kind: CLAP_EVENT_NOTE_ON,
                        id: 27,
                        port: 0,
                        channel: 0,
                        key: 60,
                        velocity: 0.625,
                        flags: 0
                    }
                ),
                (
                    19,
                    Event::Note {
                        kind: CLAP_EVENT_NOTE_OFF,
                        id: 27,
                        port: 0,
                        channel: 0,
                        key: 60,
                        velocity: 0.625,
                        flags: 0
                    }
                ),
            ],
            "same exported instance, lifecycle pass {pass}"
        );
        hub.run(64, vec![], None);
        hub.run(128, vec![], None);
        let snapshot = inspect_hub(&hub, |hub| hub.direct.test_snapshot());
        assert_eq!(
            (snapshot.held, snapshot.pending, snapshot.captures, snapshot.lives),
            (0, 0, 0, 0)
        );
        assert_eq!(snapshot.faults, 0);
        let adopted = hub.shared().adopted().unwrap();
        assert!(adopted.valid);
        if pass == 0 {
            unsafe {
                (*hub.plugin).stop_processing.unwrap()(hub.plugin);
                (*hub.plugin).deactivate.unwrap()(hub.plugin);
            }
            hub.active = false;
            hub.activate();
        }
    }
}

#[test]
fn production_initial_direct_accepts_exact_zero_delay_phrase_without_route_calibration() {
    let _scope = crate::test_scope::enter();
    let (recorder, mut capture) = harmonigraph_record::testing::channel();
    crate::configuration::inject_recorder(recorder);
    let mut hub = Device::new(false);
    hub.activate();
    let value = 0.123456789;
    let accepted = hub.run(
        0,
        vec![note(27, 0, 60, 3, true), expression(27, value, 11), note(27, 0, 60, 19, false)],
        None,
    );
    assert_eq!(accepted.values.len(), 3, "actual CLAP acceptance is independent of observation");
    assert!(matches!(
        accepted.values[0],
        (3, Event::Note { kind: CLAP_EVENT_NOTE_ON, id: 27, .. })
    ));
    assert!(
        matches!(accepted.values[1], (11, Event::Expression { kind: 2, value: actual, .. }) if actual == value)
    );
    assert!(matches!(
        accepted.values[2],
        (19, Event::Note { kind: CLAP_EVENT_NOTE_OFF, id: 27, .. })
    ));
    for raw in [64, 128] {
        assert!(hub.run(raw, vec![], None).values.is_empty());
    }
    let records = capture.drain_canonical();
    assert_eq!(records.len(), 4);
    let harmonigraph_take::CanonicalRecord::Delta(initial) = &records[0] else { unreachable!() };
    // Activation calls Plugin::reset, which publishes this initialization
    // control separately from the three observed phrase events below.
    assert_eq!(initial.event.kind, harmonigraph_take::NoteKind::SourceReset);
    assert_eq!(initial.event.t, 0.0);
    assert_eq!((initial.sequence, initial.lifetime), (0, 0));
    assert_eq!(
        initial.provenance,
        harmonigraph_core::confirmed::PitchProvenance::ObservedDirect.into()
    );
    assert!(initial.timing.is_none());
    let deltas: Vec<_> = records
        .into_iter()
        .filter_map(|record| {
            if let harmonigraph_take::CanonicalRecord::Delta(delta) = record {
                matches!(
                    delta.event.kind,
                    harmonigraph_take::NoteKind::On { .. }
                        | harmonigraph_take::NoteKind::Tuning { .. }
                        | harmonigraph_take::NoteKind::Off
                )
                .then_some(delta)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(deltas.len(), 3);
    assert_eq!(
        deltas.iter().map(|delta| delta.timing.unwrap().sample).collect::<Vec<_>>(),
        [3, 11, 19]
    );
    assert!(deltas.iter().all(|delta| delta.provenance
        == harmonigraph_core::confirmed::PitchProvenance::ObservedDirect.into()));
    let snapshot = inspect_hub(&hub, |hub| hub.direct.test_snapshot());
    assert_eq!((snapshot.sequence, snapshot.acknowledged), (3, 3));
    assert_eq!((snapshot.held, snapshot.pending, snapshot.captures, snapshot.lives), (0, 0, 0, 0));
    assert_eq!(snapshot.faults, 0);
    let adopted = hub.shared().adopted().unwrap();
    assert!(adopted.valid, "the host establishes the default clock automatically");
    let invalid = hub.run(256, vec![note(28, 0, 62, 0, true)], None);
    assert!(!invalid.values.iter().any(|(_, event)| event.attack().is_some()));
    assert_ne!(inspect_hub(&hub, |hub| hub.direct.test_snapshot().faults) & source::CLOCK_FAULT, 0);
    hub.shared().apply(hub.shared().value().routing, true).unwrap();
    let mut resumed = None;
    for block in 5..69 {
        hub.run(block * 64, vec![], None);
        let snapshot = inspect_hub(&hub, |hub| hub.direct.test_snapshot());
        if snapshot.faults == 0 && snapshot.epoch > 1 {
            resumed = Some((block + 1) * 64);
            break;
        }
    }
    let raw = resumed.unwrap_or_else(|| {
        panic!(
            "explicit local Reset must establish fresh callback validity: {:?}",
            inspect_hub(&hub, |hub| hub.direct.test_snapshot())
        )
    });
    assert!(
        inspect_hub(&hub, |hub| hub.test_actual_voice(0, 2)).is_none(),
        "the committed discontinuous Reset clears the old observed DIRECT lifetime"
    );
    assert_eq!(
        hub.run(
            raw,
            vec![note(29, 0, 64, 2, true), expression(29, value, 10), note(29, 0, 64, 18, false)],
            None
        )
        .values
        .len(),
        3
    );
    hub.run(raw + 64, vec![], None);
    hub.run(raw + 128, vec![], None);
    assert_eq!(inspect_hub(&hub, |hub| hub.direct.test_snapshot().pending), 0);
    assert!(hub.shared().adopted().unwrap().valid);
}

#[test]
fn production_calibrated_direct_drains_more_than_one_capture_window_on_empty_callbacks() {
    let _scope = crate::test_scope::enter();
    let mut hub = Device::new(false);
    hub.configure_format(SavedUuid::default(), true, Calibration { offset: 0 });
    hub.activate_format(48000.0, 512);
    let mut input = vec![note(31, 0, 60, 0, true)];
    input.extend((1..512).map(|sample| expression(31, 0.125, sample)));
    let first = hub.run_format(0, input, None, None, 512);
    let prefix = inspect_hub(&hub, |hub| hub.direct.completed_input())
        .expect("a transferred complete prefix must not wait for the whole Original backlog");
    assert_eq!(
        (prefix.0.through, prefix.1),
        (256, 256),
        "exclusive coverage excludes original257 at sample256"
    );
    assert!(
        inspect_hub(&hub, |hub| hub.test_input_sequence_progress().3).unwrap() <= prefix.0.through
    );
    let mut accepted = first.values;
    accepted.extend(
        hub.run_format(
            512,
            (0..512).map(|sample| expression(31, 0.125, sample)).collect(),
            None,
            None,
            512,
        )
        .values,
    );
    let mut input: Vec<_> = (0..511).map(|sample| expression(31, 0.125, sample)).collect();
    input.push(note(31, 0, 60, 511, false));
    accepted.extend(hub.run_format(1024, input, None, None, 512).values);
    let mut callbacks = 0;
    for block in 1..=64 {
        accepted.extend(hub.run_format((block + 2) * 512, vec![], None, None, 512).values);
        callbacks = block;
        let snapshot = inspect_hub(&hub, |hub| hub.direct.test_snapshot());
        assert_eq!(snapshot.faults, 0, "{snapshot:?}");
        if snapshot.pending == 0 && snapshot.captures == 0 && snapshot.lives == 0 {
            break;
        }
    }
    let snapshot = inspect_hub(&hub, |hub| hub.direct.test_snapshot());
    assert_eq!(accepted.len(), 1536, "every original has real accepted output");
    assert_eq!(accepted.iter().filter(|(_, event)| event.attack().is_some()).count(), 1);
    assert_eq!(accepted.iter().filter(|(_, event)| event.release()).count(), 1);
    assert_eq!((snapshot.sequence, snapshot.acknowledged), (1536, 1536));
    assert_eq!(
        (snapshot.pending, snapshot.captures, snapshot.lives, snapshot.held),
        (0, 0, 0, 0),
        "{snapshot:?}"
    );
    assert_eq!(inspect_hub(&hub, |hub| hub.test_capture_phases(0)), (0, 0));
    println!("DIRECT1536 drained in {callbacks} continuous empty callbacks");
}

#[test]
fn factual_index_survives_two_voice_baseline_permutation_and_same_key_reuse() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::aggregation(false);
    hub.configure(uuid, true);
    hub.activate();
    let mut source = Device::aggregation(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    let lease = attachment_tests::lease(&source).unwrap();
    assert_eq!(
        source.run(64, vec![note(11, 0, 60, 0, true), note(22, 1, 64, 1, true)], None).values.len(),
        2
    );
    hub.run(64, vec![], None);
    let before = [1, 2].map(|lifetime| {
        inspect_hub(&hub, |hub| hub.test_actual_voice(lease.slot, lifetime).unwrap().0)
    });
    source.run(128, vec![source.participation(false, 0)], None);
    hub.run(128, vec![], None);
    let after = [1, 2].map(|lifetime| {
        inspect_hub(&hub, |hub| hub.test_actual_voice(lease.slot, lifetime).unwrap().0)
    });
    assert_eq!(
        after,
        [before[1], before[0]],
        "a real complete two-voice baseline swaps both physical slots"
    );
    let next = source.run(
        192,
        vec![
            expression(11, 0.25, 1),
            expression(22, -0.125, 2),
            note(11, 0, 60, 3, false),
            note(33, 0, 60, 4, true),
            expression(33, 0.5, 5),
        ],
        None,
    );
    assert_eq!(next.values.len(), 5);
    hub.run(192, vec![], None);
    assert!(inspect_hub(&hub, |hub| hub.test_actual_voice(lease.slot, 1)).is_none());
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_actual_voice(lease.slot, 2)),
        Some((after[1], 0, -0.125))
    );
    assert_eq!(
        inspect_hub(&hub, |hub| hub.test_actual_voice(lease.slot, 3)),
        Some((after[0], 0, 0.5))
    );
    assert_eq!(
        source
            .run(256, vec![note(22, 1, 64, 0, false), note(33, 0, 60, 1, false)], None)
            .values
            .len(),
        2
    );
    hub.run(256, vec![], None);
    source.run(320, vec![], None);
    hub.run(320, vec![], None);
    assert_eq!(source.source_snapshot().held, 0);
    assert!(inspect_hub(&hub, |hub| hub.test_actual_voice(lease.slot, 2)).is_none());
    assert!(inspect_hub(&hub, |hub| hub.test_actual_voice(lease.slot, 3)).is_none());
}

#[test]
fn production_calibrated_direct_requires_valid_reset_after_clock_failure() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    assert_eq!(
        hub.run(0, vec![note(1, 0, 60, 1, true), note(1, 0, 60, 3, false)], None).values.len(),
        2
    );
    hub.run(64, vec![], None);
    hub.run(128, vec![], None);
    assert!(hub.run(256, vec![note(2, 0, 62, 0, true)], None).values.is_empty());
    let valid = hub.shared().value().routing;
    for block in 5..13 {
        assert!(hub.run(block * 64, vec![note(3, 0, 64, 1, true)], None).values.is_empty());
        assert_ne!(
            inspect_hub(&hub, |hub| hub.direct.test_snapshot().faults) & source::CLOCK_FAULT,
            0
        );
    }
    hub.shared().apply(valid, true).unwrap();
    let mut raw = 13 * 64;
    for _ in 0..32 {
        hub.run(raw, vec![], None);
        raw += 64;
        if inspect_hub(&hub, |hub| hub.direct.test_snapshot().faults) == 0 {
            break;
        }
    }
    assert_eq!(inspect_hub(&hub, |hub| hub.direct.test_snapshot().faults), 0);
    assert!(hub.shared().adopted().unwrap().valid);
    assert_eq!(
        hub.run(raw, vec![note(4, 0, 65, 1, true), note(4, 0, 65, 3, false)], None).values.len(),
        2
    );
    hub.run(raw + 64, vec![], None);
    hub.run(raw + 128, vec![], None);
}

#[test]
fn production_healthy_direct_reanchor_preserves_observed_pitch_until_its_real_input_off() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let (recorder, mut capture) = harmonigraph_record::testing::channel();
    crate::configuration::inject_recorder(recorder);
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    assert_eq!(
        hub.run(0, vec![note(31, 0, 60, 1, true), expression(31, 0.123456789, 2)], None)
            .values
            .len(),
        2
    );
    hub.run(64, vec![], None);
    capture.drain_canonical();
    hub.shared()
        .apply(
            setup::Routing::Hub(HubSetup { uuid, calibration: Calibration { offset: 64 } }),
            false,
        )
        .unwrap();
    let mut raw = 128;
    for _ in 0..64 {
        assert!(
            !hub.run(raw, vec![], None).values.iter().any(|(_, event)| event.attack().is_some()),
            "clock reseed never re-emits a physical On"
        );
        raw += 64;
        if inspect_hub(&hub, |hub| hub.direct.test_snapshot().epoch) == 2 {
            break;
        }
    }
    assert_eq!(inspect_hub(&hub, |hub| hub.direct.test_snapshot().epoch), 2);
    assert_eq!(
        inspect_hub(&hub, |hub| hub.direct.test_snapshot().held),
        0,
        "forwarding's old physical release is settled"
    );
    assert_eq!(
        inspect_hub(&hub, |hub| hub
            .test_actual_voice(0, 1)
            .map(|(_, correction, player)| (correction, player))),
        Some((0, 0.123456789)),
        "the healthy boundary retains observed held input"
    );
    assert!(!capture.drain_canonical().iter().any(|record| matches!(record, harmonigraph_take::CanonicalRecord::Delta(delta) if matches!(delta.event.kind, harmonigraph_take::NoteKind::On { .. }))), "clock reseed never fabricates a canonical On");
    hub.run(raw, vec![expression(31, 0.25, 1)], None);
    raw += 64;
    assert_eq!(
        inspect_hub(&hub, |hub| hub
            .test_actual_voice(0, 1)
            .map(|(_, correction, player)| (correction, player))),
        Some((0, 0.25))
    );
    hub.run(raw, vec![note(31, 0, 60, 1, false)], None);
    hub.run(raw + 64, vec![], None);
    assert!(inspect_hub(&hub, |hub| hub.test_actual_voice(0, 1)).is_none());
    assert_eq!(inspect_hub(&hub, |hub| hub.direct.test_snapshot().faults), 0);
}
