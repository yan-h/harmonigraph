//! Callback snapshots through the exported CLAP factory, including real source
//! admission, sequencing, departures and the allocation-guarded audio callback.
use super::*;
use harmonigraph_core::confirmed::{ConfirmedError, LearningState};

fn snapshot(device: &Device) -> (bool, Vec<(u64, u8)>) {
    hub_wrapper(device).test_inspect_plugin(|plugin| {
        let confirmed = &plugin.configuration.as_ref().unwrap().confirmed;
        let mut keys: Vec<_> =
            confirmed.rows().map(|row| (row.key.source.0, row.key.note)).collect();
        keys.sort_unstable();
        (confirmed.is_complete(), keys)
    })
}

#[test]
fn confirmed_snapshot_turnover_at_capacity_and_departure() {
    let _scope = crate::test_scope::enter();
    let mut hub = Device::new(false);
    hub.activate();
    let mut tunes: Vec<_> = (0..5)
        .map(|_| {
            let mut tune = Device::new(true);
            tune.shared().set_retune(false);
            tune.activate();
            tune
        })
        .collect();
    for tune in &tunes[1..] {
        tune.run(0, (0..64).map(|key| note(key, 0, key as i16, 0, true)).collect(), None);
    }
    hub.run(0, vec![], None);
    assert!(snapshot(&hub).1.is_empty(), "input does not republish during flush");
    hub.run(64, vec![], None);
    let (complete, before) = snapshot(&hub);
    assert!(complete);
    assert_eq!(before.len(), HELD_SESSION, "fixture fills all 256 confirmed slots");
    let departed = before.last().unwrap().0;
    tunes[4].run(128, (0..64).map(|key| note(key, 0, key as i16, 0, false)).collect(), None);
    tunes[0].run(128, (0..64).map(|key| note(key, 0, key as i16, 1, true)).collect(), None);
    hub.run(128, vec![], None);
    assert_eq!(snapshot(&hub), (true, before.clone()), "snapshot lasts for the callback");
    hub.run(192, vec![], None);
    let (complete, after) = snapshot(&hub);
    assert!(complete, "obsolete later-source occupancy must not refuse the earlier source");
    assert_eq!(after.len(), HELD_SESSION);
    assert!(after.iter().all(|&(source, _)| source != departed));
    assert!(after[0].0 < before[0].0, "turnover really moves to an earlier row");
    assert_eq!(&after[64..], &before[..192]);
    drop(tunes.remove(0));
    hub.run(256, vec![], None);
    // Membership changes cut the whole session, not just the departed row.
    assert_eq!(snapshot(&hub), (true, vec![]));
    hub.run(320, vec![note(90, 0, 60, 0, true)], None);
    hub.run(384, vec![], None);
    assert_eq!(snapshot(&hub).1.len(), 1);
    session::session().reset();
    hub.run(448, vec![], None);
    assert_eq!(snapshot(&hub), (true, vec![]));
}

#[test]
fn incomplete_source_blocks_learning_until_departure_or_reset() {
    for overflow in [false, true] {
        let _scope = crate::test_scope::enter();
        let mut hub = Device::new(false);
        hub.activate();
        let mut bad = Device::new(true);
        bad.shared().set_retune(false);
        bad.activate();
        let mut good = Device::new(true);
        good.shared().set_retune(false);
        good.activate();
        bad.run(0, (0..64).map(|key| note(key, 0, key as i16, 0, true)).collect(), None);
        good.run(0, [60, 64, 67].map(|key| note(key, 0, key as i16, 0, true)).to_vec(), None);
        hub.run(0, vec![], None);
        hub.run(64, vec![], None);
        assert_eq!(snapshot(&hub).1.len(), 67);
        if overflow {
            // A plain 65th attack is refused by Tune admission. Lose a release
            // copy instead: Tune reserves its freed cell while Hub stays full.
            let mut events: Vec<_> =
                (0..CAPTURE_RING).map(|_| raw_midi([0xb0, 20, 0], 0)).collect();
            events.push(note(0, 0, 0, 1, false));
            bad.run(128, events, None);
            assert_ne!(bad.shared().status() & session::RING_FULL, 0);
            hub.run(128, vec![], None);
            assert_eq!(inspect_hub(&hub, |hub| hub.test_held(0)), HELD_PER_SOURCE);
            bad.run(192, vec![note(64, 0, 64, 0, true)], None);
        } else {
            bad.run(128, vec![expression(0, f64::MAX, 0)], None);
            hub.run(128, vec![], None);
        }
        hub.run(192, vec![], None);
        hub.run(256, vec![], None);
        let infer = || {
            hub_wrapper(&hub).test_inspect_plugin(|plugin| {
                LearningState::default()
                    .infer(&plugin.configuration.as_ref().unwrap().confirmed, true)
            })
        };
        assert_eq!(snapshot(&hub).1.len(), 3, "only the independent learnable triad remains");
        assert_eq!(infer(), Err(ConfirmedError::Incomplete));
        bad.run(320, vec![note(1, 0, 1, 0, false)], None);
        hub.run(320, vec![], None);
        hub.run(384, vec![], None);
        assert_eq!(infer(), Err(ConfirmedError::Incomplete), "release cannot repair missing state");
        if overflow {
            session::session().reset();
        } else {
            drop(bad);
        }
        hub.run(448, vec![], None);
        assert_eq!(
            snapshot(&hub),
            (true, vec![]),
            "reset and departure both authoritatively cut state"
        );
        good.run(448, [60, 64, 67].map(|key| note(key, 0, key as i16, 0, true)).to_vec(), None);
        hub.run(512, vec![], None);
        hub.run(576, vec![], None);
        assert_eq!(snapshot(&hub).1.len(), 3);
        assert!(infer().unwrap().is_some(), "a complete new chord can be learned after recovery");
    }
}
