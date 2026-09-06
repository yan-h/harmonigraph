//! Ownership acceptance through exported production CLAP callbacks. Test controls
//! suspend only the captured-input consumer; actual output/ACK processing runs.
use super::*;
use harmonigraph_core::cohort::{EventPhase, Progress, Role, Selected};

fn with_hub<R>(device: &Device, f: impl FnOnce(&mut hub::Hub) -> R) -> R {
    let wrapper = unsafe {
        &*((*device.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    wrapper.test_with_plugin(|plugin| f(plugin.aggregation.as_mut().unwrap()))
}
fn with_source<R>(device: &Device, f: impl FnOnce(&mut source::Source) -> R) -> R {
    let wrapper = unsafe {
        &*((*device.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
    };
    wrapper.test_with_plugin(|plugin| f(plugin.source.as_mut().unwrap()))
}
fn seed() -> Vec<Input> {
    [64, 66, 69, 88]
        .into_iter()
        .map(|cc| {
            Input::Midi(clap_event_midi {
                header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
                port_index: 0,
                data: [0xb0, cc, 0],
            })
        })
        .collect()
}
fn offered(hub: &Device) -> Option<Selected> {
    with_hub(hub, |hub| match hub.test_capture_result {
        Some(Ok(Progress::Event(selected))) => Some(selected),
        Some(Err(error)) => panic!("capture traversal failed: {error:?}"),
        _ => None,
    })
}

#[test]
fn offered_capture_survives_real_off_ack_and_cell_reuse_in_both_callback_orders() {
    let _scope = crate::test_scope::enter();
    let mut selections = Vec::new();
    for reverse in [false, true] {
        let uuid = SavedUuid::default();
        let mut hub = Device::new(false);
        hub.configure(uuid, true);
        hub.activate();
        let mut a = Device::new(true);
        a.configure(uuid, true);
        a.activate();
        let mut b = Device::new(true);
        b.configure(uuid, true);
        b.activate();
        let session = registry::global().lock().unwrap().test_session(uuid);
        a.run(0, vec![], None);
        b.run(0, vec![], None);
        hub.run(0, vec![], None);
        a.run(64, seed(), None);
        b.run(64, seed(), None);
        hub.run(64, vec![], None);
        let bits = 0x3ff0000000000001;
        let events = |id, key| {
            vec![
                note(id, 0, key, 4, true),
                expression(id, f64::from_bits(bits), 4),
                note(id + 1, 0, key + 4, 4, true),
                expression(id + 1, 0.25, 4),
                expression(-1, 0.5, 4),
            ]
        };
        with_hub(&hub, |hub| hub.test_hold_captures(132));
        if reverse {
            b.run(128, events(201, 62), None);
            a.run(128, events(101, 60), None);
        } else {
            a.run(128, events(101, 60), None);
            b.run(128, events(201, 62), None);
        }
        hub.run(128, vec![], None);
        let mut raw = 192;
        let selected = loop {
            if let Some(selected) = offered(&hub) {
                break selected;
            }
            assert!(raw < 2048, "fixture must reach an actual offered phase");
            a.run(raw, vec![], None);
            b.run(raw, vec![], None);
            hub.run(raw, vec![], None);
            raw += 64;
        };
        assert_eq!(a.source_snapshot().held, 2, "both captured onsets physically accepted");
        assert_eq!(b.source_snapshot().held, 2, "both captured onsets physically accepted");
        assert_eq!((selected.role, selected.phase), (Role::TunerOnset, EventPhase::Original));
        assert_eq!(selected.initial_tuning.unwrap().value_bits, bits);
        assert_eq!(selected.initial_tuning.unwrap().id.source, selected.id.source);
        selections.push(selected);
        let old_on = with_source(&a, |source| source.test_capture(5).unwrap());
        let old_second = with_source(&a, |source| source.test_capture(7).unwrap());
        let old_work = with_source(&a, |source| source.test_capture(9).unwrap());
        assert_eq!(old_work.work_count, 2, "genuine Work/Birth pins, not inline-only targets");
        let before = with_hub(&hub, |hub| hub.test_capture_metadata(1, 9).unwrap());
        assert_eq!(before.3.iter().flatten().count(), 2);
        assert_eq!(before.3[0].unwrap().target.host_note_id, 101);
        assert_eq!(before.3[1].unwrap().target.host_note_id, 102);
        assert_ne!(
            with_hub(&hub, |hub| hub.test_capture_metadata(1, 5).unwrap().1.lease),
            with_hub(&hub, |hub| hub.test_capture_metadata(2, 5).unwrap().1.lease)
        );
        let mut off_cuts = [0; 2];
        for (slot, (device, id, key)) in [(&a, 101, 60), (&b, 201, 62)].into_iter().enumerate() {
            let wire = device.run(
                raw,
                vec![
                    device.participation(false, 0),
                    note(id, 0, key, 2, false),
                    note(id + 1, 0, key + 4, 3, false),
                ],
                None,
            );
            assert_eq!(wire.values.iter().filter(|(_, event)| event.release()).count(), 2);
            off_cuts[slot] = device.source_snapshot().sequence;
        }
        hub.run(raw, vec![], None);
        raw += 64;
        for _ in 0..3 {
            a.run(raw, vec![], None);
            b.run(raw, vec![], None);
            hub.run(raw, vec![], None);
            raw += 64;
        }
        for (slot, device) in [&a, &b].into_iter().enumerate() {
            let snapshot = device.source_snapshot();
            assert!(
                off_cuts[slot] > 0 && snapshot.acknowledged >= off_cuts[slot],
                "exact actual-output ACK covering Off: {snapshot:?}"
            );
            let received = with_hub(&hub, |hub| hub.test_row_retirement(slot));
            assert!(received.0 >= off_cuts[slot] && received.1 >= off_cuts[slot]);
            assert_eq!((snapshot.held, snapshot.journal, snapshot.emergency), (0, 0, 0));
            assert_eq!(snapshot.captures, 5);
        }
        assert_eq!(session.credits.load(Ordering::Acquire), 0);
        let retained = with_source(&a, |source| source.test_capture(5).unwrap());
        assert!(retained.local_done && retained.remote_pending);
        assert_eq!(
            (retained.position, retained.life, retained.life_serial),
            (old_on.position, old_on.life, old_on.life_serial)
        );
        // Actual new input attempts the same host address after old physical Off.
        a.run(raw, vec![note(101, 0, 60, 1, true), note(101, 0, 60, 2, false)], None);
        let replacement = with_source(&a, |source| source.test_capture(12).unwrap());
        assert_ne!(replacement.life, old_on.life);
        b.run(raw, vec![], None);
        hub.run(raw, vec![], None);
        raw += 64;
        assert_eq!(offered(&hub), Some(selected));
        let after = with_hub(&hub, |hub| hub.test_capture_metadata(1, 9).unwrap());
        assert_eq!(before.1, after.1);
        assert_eq!(before.2.targets, after.2.targets);
        for (left, right) in before.3.iter().zip(&after.3) {
            assert_eq!(
                left.map(|link| (link.target, link.next)),
                right.map(|link| (link.target, link.next))
            );
        }
        // Ending a temporary view has left both the offered phase and its pins.
        assert_eq!(
            with_hub(&hub, |hub| hub.advance_captures(0).unwrap()),
            Progress::Event(selected)
        );
        for _ in 0..32 {
            let complete = with_hub(&hub, |hub| {
                if matches!(hub.test_capture_result, Some(Ok(Progress::Complete { .. }))) {
                    return true;
                }
                hub.test_capture_commit =
                    matches!(hub.test_capture_result, Some(Ok(Progress::Event(_))));
                false
            });
            if complete {
                break;
            }
            a.run(raw, vec![], None);
            b.run(raw, vec![], None);
            hub.run(raw, vec![], None);
            raw += 64;
        }
        assert!(with_hub(&hub, |hub| matches!(
            hub.test_capture_result,
            Some(Ok(Progress::Complete { .. }))
        )));
        assert!(with_source(&a, |source| source.test_capture(5).unwrap().remote_pending));
        with_hub(&hub, |hub| hub.release_captures(false));
        hub.run(raw, vec![], None);
        raw += 64;
        assert!(
            with_source(&a, |source| source.test_capture(5).unwrap().remote_pending),
            "reply has not reached Source yet"
        );
        a.run(raw, vec![], None);
        b.run(raw, vec![], None);
        hub.run(raw, vec![], None);
        raw += 64;
        assert!(with_source(&a, |source| source.test_capture(5).is_none()));
        a.run(raw, vec![note(101, 0, 60, 1, true), note(101, 0, 60, 2, false)], None);
        let reused = with_source(&a, |source| source.test_capture(14).unwrap());
        assert_eq!(reused.position, old_work.position, "actual reclaimed Pending cell reuse");
        assert!(
            [old_on.life, old_second.life].contains(&reused.life),
            "actual reclaimed Birth cell reuse"
        );
        assert!(reused.life_serial > old_second.life_serial);
        b.run(raw, vec![], None);
        hub.run(raw, vec![], None);
        raw += 64;
        a.run(raw, vec![], None);
        b.run(raw, vec![], None);
        hub.run(raw, vec![], None);
        drop(a);
        drop(b);
        drop(hub);
        assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
    }
    assert_eq!(selections[0], selections[1]);
}

#[test]
fn full_retirement_reply_window_keeps_exact_owner_until_both_peers_are_destroyed() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    let mut source = Device::new(true);
    source.configure(uuid, true);
    let mut state = source.save();
    state.fields.insert(
        setup::SOURCE_FIELD.into(),
        serde_json::to_string(&SourceSetup {
            selected: Some(uuid),
            calibration: Calibration {
                offset: 65536,
                sample_rate: 48000.0,
                max_frames: 64,
                validated: true,
            },
        })
        .unwrap(),
    );
    assert!(source.load(&state));
    source.activate();
    let weak_arena =
        std::sync::Arc::downgrade(source.shared().source.as_ref().unwrap().arena.get().unwrap());
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    with_hub(&hub, |hub| hub.test_hold_captures(65536 + 64 + 3));
    let wire = source.run(
        64,
        vec![note(73, 0, 60, 3, true), expression(73, 0.125, 3), note(73, 0, 60, 23, false)],
        None,
    );
    assert_eq!(wire.values.len(), 3);
    let off_cut = source.source_snapshot().sequence;
    for block in 1..=1027 {
        hub.run(block * 64, vec![], None);
    }
    assert!(offered(&hub).is_some());
    let key = with_hub(&hub, |hub| hub.test_capture_metadata(1, 1).unwrap().1);
    let slots = with_hub(&hub, |hub| hub.offer.as_ref().unwrap().bank.rows[0].replies.slots());
    assert_eq!(slots, 0, "the actual 1024-cell rtrb retirement reply lane is full");
    let received = with_hub(&hub, |hub| hub.test_row_retirement(0));
    assert_eq!((received.0, received.1, received.2), (0, 0, 0));
    let retained_output = source.source_snapshot();
    assert_eq!(
        (retained_output.sequence, retained_output.journal, retained_output.transfer_cut),
        (off_cut, 3, 0)
    );
    assert_eq!(
        retained_output.baseline_cut,
        Some(0),
        "Source has not resumed to consume its initial baseline ACK"
    );
    assert_eq!(
        session.credits.load(Ordering::Acquire),
        1,
        "Source has not drained the real output ACK"
    );
    with_hub(&hub, |hub| hub.test_capture_commit = true);
    hub.run(1028 * 64, vec![], None);
    // The initial tuning phase also remains an original event in the cohort.
    for block in 1029..1034 {
        if with_hub(&hub, |hub| {
            matches!(hub.test_capture_result, Some(Ok(Progress::Complete { .. })))
        }) {
            break;
        }
        with_hub(&hub, |hub| hub.test_capture_commit = offered_result(hub));
        hub.run(block * 64, vec![], None);
    }
    assert!(with_hub(&hub, |hub| matches!(
        hub.test_capture_result,
        Some(Ok(Progress::Complete { .. }))
    )));
    with_hub(&hub, |hub| hub.release_captures(false));
    hub.run(1034 * 64, vec![], None);
    assert!(
        with_hub(&hub, |hub| hub.test_capture_keys(1).contains(&key)),
        "failed reply publication retains exact retirement authority"
    );
    assert!(with_source(&source, |source| source.test_capture(1).unwrap().remote_pending));
    assert_eq!(with_hub(&hub, |hub| hub.test_capture_phases(1)), (0, 2));
    drop(hub);
    assert!(weak_arena.upgrade().is_some());
    drop(source);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
    assert!(
        weak_arena.upgrade().is_none(),
        "final free after joined owners, without rescue callbacks"
    );
}
fn offered_result(hub: &hub::Hub) -> bool {
    matches!(hub.test_capture_result, Some(Ok(Progress::Event(_))))
}

#[test]
fn reused_ingress_and_birth_reject_old_frozen_binding_and_exact_retirement_replays() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    with_hub(&hub, |hub| hub.test_hold_captures(65));
    assert_eq!(source.run(64, vec![note(71, 0, 60, 1, true)], None).values.len(), 1);
    hub.run(64, vec![], None);
    assert!(offered(&hub).is_some());
    let old_id = with_hub(&hub, |hub| hub.test_frozen_id());
    let old = with_hub(&hub, |hub| hub.test_capture_metadata(1, 1).unwrap());
    let old_source = with_source(&source, |source| source.test_capture(1).unwrap());
    assert_eq!(source.run(128, vec![note(71, 0, 60, 2, false)], None).values.len(), 1);
    hub.run(128, vec![], None);
    source.run(192, vec![], None);
    with_hub(&hub, |hub| hub.test_capture_commit = true);
    hub.run(192, vec![], None);
    assert!(with_hub(&hub, |hub| matches!(
        hub.test_capture_result,
        Some(Ok(Progress::Complete { .. }))
    )));
    with_hub(&hub, |hub| hub.release_captures(false));
    source.run(256, vec![], None);
    hub.run(256, vec![], None);
    // A real next callback consumes retirement, then creates new captures. The
    // two inputs occupy both recently free ingress slots, including old.slot.
    with_hub(&hub, |hub| hub.test_hold_captures(321));
    let next = source.run(320, vec![note(71, 0, 60, 1, true), expression(71, 0.375, 1)], None);
    assert_eq!(next.values.len(), 2);
    let new_source = with_source(&source, |source| source.test_capture(3).unwrap());
    let new_tuning_source = with_source(&source, |source| source.test_capture(4).unwrap());
    assert!([new_source.position, new_tuning_source.position].contains(&old_source.position), "a real new capture occupies the old Pending cell: old={old_source:?} newOn={new_source:?} newTuning={new_tuning_source:?} retainedOld={:?} source={:?} hubkeys={:?}", with_source(&source, |source| source.test_capture(1)), source.source_snapshot(), with_hub(&hub, |hub| hub.test_capture_keys(1)));
    assert_eq!(new_source.life, old_source.life);
    assert!(new_source.life_serial > old_source.life_serial);
    hub.run(320, vec![], None);
    let new_id = with_hub(&hub, |hub| hub.test_frozen_id());
    assert!(new_id.0 > old_id.0);
    let new_on = with_hub(&hub, |hub| hub.test_capture_metadata(1, 3).unwrap());
    let new_tuning = with_hub(&hub, |hub| hub.test_capture_metadata(1, 4).unwrap());
    let reused = [&new_on, &new_tuning]
        .into_iter()
        .find(|entry| entry.0 == old.0)
        .expect("the actual old ingress cell now holds a different token");
    assert_ne!(reused.1.serial, old.1.serial);
    assert!(with_hub(&hub, |hub| hub.test_capture_lookup(1, old_id, old.2.targets.first)).is_none());
    let target = with_hub(&hub, |hub| hub.test_capture_lookup(1, new_id, reused.2.targets.first))
        .unwrap()
        .target;
    assert_eq!((target.host_note_id, target.lifetime), (71, new_source.life_serial));
    assert_eq!(
        new_tuning.2.kind,
        harmonigraph_core::cohort::Kind::Tuning { value_bits: 0.375f64.to_bits() }
    );
    // Duplicate old serial plus wrong lease/incarnation/epoch are delivered in
    // the actual reply ring and rejected by the Source's next callback.
    with_hub(&hub, |hub| {
        hub.test_repeat_capture_retirement(old.1);
        let mut wrong = new_on.1;
        wrong.epoch += 1;
        hub.test_repeat_capture_retirement(wrong);
        let mut wrong = new_on.1;
        wrong.lease.incarnation += 1;
        hub.test_repeat_capture_retirement(wrong);
        let mut wrong = new_on.1;
        wrong.arena ^= 8;
        hub.test_repeat_capture_retirement(wrong);
    });
    let captures = source.source_snapshot().captures;
    assert_eq!(source.run(384, vec![note(71, 0, 60, 2, false)], None).values.len(), 1);
    assert_eq!(
        source.source_snapshot().captures,
        captures + 1,
        "stale retirement did not release either frozen owner"
    );
    assert!(with_source(&source, |source| source.test_capture(3).unwrap().remote_pending));
    hub.run(384, vec![], None);
    source.run(448, vec![], None);
    hub.run(448, vec![], None);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    drop(source);
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn mixed_generation_cancellation_keeps_the_original_complete_captured_targets() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    assert_eq!(source.run(64, vec![note(1, 0, 60, 0, true)], None).values.len(), 1);
    hub.run(64, vec![], None);
    let shared = source.shared();
    let setup::Routing::Source(mut next) = shared.value().routing else { unreachable!() };
    next.selected = Some(SavedUuid::default());
    shared.apply(setup::Routing::Source(next), false).unwrap();
    source.run(128, vec![], None);
    hub.run(128, vec![], None);
    with_hub(&hub, |hub| hub.test_hold_captures(193));
    let attempted = source.run(
        192,
        vec![note(2, 0, 64, 0, true), expression(-1, 0.234567890123, 1)],
        Some(CLAP_EVENT_NOTE_EXPRESSION),
    );
    assert_eq!(attempted.rejected.len(), 1, "actual old-generation expression was rejected");
    // Existing OUTPUT_FAULT containment accepts an emergency old-voice choke
    // and pedal neutralization after the rejected expression; it is not a fake
    // success of the wildcard. The captured old/new target set still survives.
    assert_eq!(attempted.values.iter().filter(|(_, event)| event.release()).count(), 1);
    assert_eq!(
        attempted
            .values
            .iter()
            .filter(|(_, event)| matches!(event, Event::Midi { data: [0xb0, 64 | 66 | 69, 0], .. }))
            .count(),
        3
    );
    assert_eq!(source.source_snapshot().faults, source::OUTPUT_FAULT);
    let captured = with_source(&source, |source| source.test_capture(3).unwrap());
    assert_eq!(captured.work_count, 2);
    hub.run(192, vec![], None);
    let selected = offered(&hub).expect("wildcard's actual offered original phase");
    let before = with_hub(&hub, |hub| hub.test_capture_metadata(1, 3).unwrap());
    assert_eq!(
        before.3.iter().flatten().map(|link| link.target.host_note_id).collect::<Vec<_>>(),
        vec![1, 2]
    );
    shared.apply(shared.value().routing, true).unwrap();
    let reset = source.run(256, vec![], None);
    assert_eq!(
        reset.values.iter().filter(|(_, event)| event.release()).count(),
        0,
        "previous emergency acceptance remains factual"
    );
    assert!(reset.values.iter().all(|(_, event)| event.attack().is_none()));
    let partial = source.source_snapshot();
    assert_eq!((partial.obligations, partial.old_obligations, partial.manifest), (1, 1, 1));
    assert!(with_source(&source, |source| source.test_capture(3).unwrap().remote_pending));
    hub.run(256, vec![], None);
    source.run(320, vec![], None);
    let local = source.source_snapshot();
    assert_eq!(
        (local.obligations, local.old_obligations, local.manifest, local.held, local.journal),
        (0, 0, 0, 0, 0)
    );
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    let retained = with_source(&source, |source| source.test_capture(3).unwrap());
    assert!(retained.local_done && retained.remote_pending);
    assert_eq!((retained.work_head, retained.work_count), (captured.work_head, 2));
    let after = with_hub(&hub, |hub| hub.test_capture_metadata(1, 3).unwrap());
    for (left, right) in before.3.iter().zip(after.3) {
        assert_eq!(
            left.map(|link| (link.target, link.next)),
            right.map(|link| (link.target, link.next))
        );
    }
    assert_eq!(offered(&hub), Some(selected));
    with_hub(&hub, |hub| hub.test_capture_commit = true);
    hub.run(320, vec![], None);
    with_hub(&hub, |hub| hub.release_captures(false));
    source.run(384, vec![], None);
    hub.run(384, vec![], None);
    source.run(448, vec![], None);
    hub.run(448, vec![], None);
    assert!(with_source(&source, |source| source.test_capture(3).is_none()));
    drop(source);
    drop(hub);
    drop(shared);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn direct_input_uses_the_same_persistent_capture_and_retirement_owner() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let session = registry::global().lock().unwrap().test_session(uuid);
    hub.run(0, vec![], None);
    assert_eq!(
        hub.run(64, vec![note(90, 0, 70, 3, true), expression(90, 0.625, 3)], None).values.len(),
        2
    );
    with_hub(&hub, |hub| hub.test_hold_captures(67));
    assert_eq!(hub.run(128, vec![note(90, 0, 70, 2, false)], None).values.len(), 1);
    let selected = offered(&hub).unwrap();
    assert_eq!(selected.role, Role::ObservedDirectOnset);
    assert_eq!(selected.id.source, 0);
    assert_eq!(selected.initial_tuning.unwrap().value_bits, 0.625f64.to_bits());
    let before = with_hub(&hub, |hub| hub.test_capture_metadata(0, 1).unwrap());
    hub.run(192, vec![], None);
    let source = with_hub(&hub, |hub| hub.direct.test_snapshot());
    assert_eq!((source.captures, source.held, source.journal), (2, 0, 0));
    assert!(source.acknowledged >= 3);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert_eq!(offered(&hub), Some(selected));
    assert_eq!(with_hub(&hub, |hub| hub.test_capture_metadata(0, 1).unwrap().1), before.1);
    let mut raw = 256;
    for _ in 0..4 {
        if with_hub(&hub, |hub| {
            matches!(hub.test_capture_result, Some(Ok(Progress::Complete { .. })))
        }) {
            break;
        }
        with_hub(&hub, |hub| hub.test_capture_commit = offered_result(hub));
        hub.run(raw, vec![], None);
        raw += 64;
    }
    with_hub(&hub, |hub| hub.release_captures(false));
    hub.run(raw, vec![], None);
    assert_eq!(with_hub(&hub, |hub| hub.direct.test_snapshot().captures), 0);
    assert!(with_hub(&hub, |hub| hub.direct.test_capture(1).is_none()));
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn continued_arrivals_cannot_starve_released_tune_or_direct_captures() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    let clocks = |count| {
        (0..count)
            .map(|_| {
                Input::Midi(clap_event_midi {
                    header: header::<clap_event_midi>(CLAP_EVENT_MIDI, 0),
                    port_index: 0,
                    data: [0xf8, 0, 0],
                })
            })
            .collect::<Vec<_>>()
    };
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    with_hub(&hub, |hub| hub.test_pause_captures());
    source.run(64, vec![note(1, 0, 60, 1, true)], None);
    hub.run(64, vec![note(2, 0, 70, 1, true)], None);
    with_hub(&hub, |hub| hub.test_hold_captures(65));
    let mut source_input = vec![note(1, 0, 60, 0, false)];
    source_input.extend(clocks(128));
    let mut direct_input = vec![note(2, 0, 70, 0, false)];
    direct_input.extend(clocks(128));
    source.run(128, source_input, None);
    hub.run(128, direct_input, None);
    assert!(offered(&hub).is_some());
    for raw in [192, 256] {
        with_hub(&hub, |hub| hub.test_capture_commit = offered_result(hub));
        assert_eq!(source.run(raw, clocks(64), None).values.len(), 64);
        assert_eq!(hub.run(raw, clocks(64), None).values.len(), 64);
    }
    assert!(with_hub(&hub, |hub| matches!(
        hub.test_capture_result,
        Some(Ok(Progress::Complete { .. }))
    )));
    assert!(with_source(&source, |source| source.test_capture(1).unwrap().remote_pending));
    assert!(with_hub(&hub, |hub| hub.direct.test_capture(1).unwrap().remote_pending));
    with_hub(&hub, |hub| {
        assert!(hub.test_capture_keys(0).len() > 64);
        assert!(hub.test_capture_keys(1).len() > 64);
        hub.release_captures(false);
    });
    let mut retired = false;
    // Keep adding a full parse grant while the cursor has an older backlog.
    // A cursor with no finite sweep boundary chases this tail forever.
    for block in 5..24 {
        assert_eq!(source.run(block * 64, clocks(64), None).values.len(), 64);
        assert_eq!(hub.run(block * 64, clocks(64), None).values.len(), 64);
        retired = with_source(&source, |source| source.test_capture(1).is_none())
            && with_hub(&hub, |hub| hub.direct.test_capture(1).is_none());
        if retired {
            break;
        }
    }
    assert!(retired, "both exact old owners retire despite continuous arrivals");
    drop(source);
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn failed_last_disposition_in_a_slice_cannot_be_overtaken_after_reply_capacity_returns() {
    let _scope = crate::test_scope::enter();
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure(uuid, true);
    hub.activate();
    let mut source = Device::new(true);
    source.configure(uuid, true);
    source.activate();
    source.run(0, vec![], None);
    hub.run(0, vec![], None);
    // Repeated factual coverage positions the first real cancellation at the
    // last visit of a 64-cell slice. It creates no musical input or outcome.
    // The two captured onsets and their coverage supply the other three cells.
    with_source(&source, |source| source.test_pad_disposition_scan(60));
    source.run(
        64,
        vec![note(1, 0, 60, 0, true), note(2, 0, 62, 0, true)],
        Some(CLAP_EVENT_NOTE_ON),
    );
    source.run(128, vec![], None);
    assert_eq!(source.source_snapshot().manifest, 2);
    // Repeat the Hub's already-published output ACK in the real 1024-cell lane.
    with_hub(&hub, |hub| hub.test_fill_reply_with_old_ack());
    hub.run(64, vec![], None);
    assert_eq!(with_hub(&hub, |hub| hub.test_disposition_cursor()), (2, Some(2)));
    for block in 3..=9 {
        assert!(source.run(block * 64, vec![], None).values.is_empty());
        hub.run((block - 1) * 64, vec![], None);
    }
    let settled = source.source_snapshot();
    assert_eq!((settled.manifest, settled.obligations, settled.pending), (0, 0, 0));
    drop(source);
    drop(hub);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}
