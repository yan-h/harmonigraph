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

fn known_seed() -> Vec<Input> {
    vec![
        midi(0, 0xb0, 7, 40, 0),
        midi(0, 0xb0, 64, 0, 1),
        midi(0, 0xb0, 66, 0, 2),
        midi(0, 0xb0, 69, 0, 3),
    ]
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
fn withdrawal_terminates_the_forwarded_voice_before_forgetting_its_ownership() {
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
    let (lease, adopted, _, _) =
        wrapper.test_inspect_plugin(|plugin| plugin.source.as_ref().unwrap().test_stream_status());
    assert!(adopted);
    assert_eq!(source.source_snapshot().held, 1);
    let row = &session.rows[usize::from(lease.unwrap().slot - 1)];
    assert_eq!(row.emission_gate.load(Ordering::Acquire), source::OPEN);
    // Re-select the same Hub: the registry withdraws this lease and mints a
    // new incarnation, which is the membership reset boundary.
    let shared = source.shared();
    let setup::Routing::Source(mut routing) = shared.value().routing else { unreachable!() };
    routing.selected = Some(SavedUuid::default());
    shared.apply(setup::Routing::Source(routing), false).unwrap();
    source.main();
    hub.run(64, vec![], None);
    assert!(row.withdrawn.load(Ordering::Acquire));
    assert_eq!(row.emission_gate.load(Ordering::Acquire), source::CLOSED);
    // The Tune owes the receiving instrument a release for the voice it
    // forwarded, and pays it here rather than leaving a note sounding in a
    // session neither side owns any more. The host-side reservation outlives
    // that release: ownership of the note id is only given up at the host's
    // own termination boundary, which is the instance drop below.
    let mut released = 0;
    let mut raw = 128;
    for _ in 0..16 {
        released += source
            .run(raw, vec![], None)
            .values
            .iter()
            .filter(|(_, event)| event.release())
            .count();
        hub.run(raw, vec![], None);
        source.main();
        hub.main();
        raw += 64;
        if released != 0 && source.source_snapshot().emergency == 0 {
            break;
        }
    }
    assert_eq!(released, 1, "exactly one physical Note-Off for the withdrawn voice");
    let settled = source.source_snapshot();
    assert_eq!(
        (settled.emergency, settled.journal, settled.faults),
        (0, 0, 0),
        "the termination is accepted and acknowledged, and an ordinary reset is not a fault"
    );
    drop(hub);
    drop(source);
    assert_eq!(session.credits.load(Ordering::Acquire), 0);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
}

#[test]
fn a_join_floor_retry_keeps_new_stream_controls_behind_the_moved_join_boundary() {
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
        "the join request starts behind Hub's published128 frontier"
    );
    hub.run(128, vec![], None);
    let retry = source.run(128, vec![midi(0, 0xb0, 88, 37, 2), midi(0, 0x90, 60, 64, 4)], None);
    let (_, _, joined, coverage) =
        wrapper.test_inspect_plugin(|plugin| plugin.source.as_ref().unwrap().test_stream_status());
    assert_eq!(coverage.unwrap().start, 128, "the real Hub reply moved the join floor");
    assert!(
        !joined && retry.values.is_empty(),
        "naming a later join floor is not stream admission"
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
