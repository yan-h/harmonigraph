use super::*;

#[test]
fn factual_lookup_preserves_sparse_lifetimes_and_charges_identity_misses_once() {
    let mut hub = Hub::new();
    let sequencer = &mut hub.sequencer;
    let lease =
        Lease { session: 7, source: harmonigraph_core::SourceId(12), incarnation: 9, slot: 1 };
    let a = ActualKey { lease, epoch: 4, lifetime: 1 };
    let b = ActualKey { lifetime: 65, ..a };
    let addresses = [actual_address(lease, 0, 60), actual_address(lease, 0, 64)];
    for (key, address, note) in [(a, addresses[0], 60), (b, addresses[1], 64)] {
        let mut work = 0;
        let lookup = sequencer.lookup_actual(key, Some(address), true, &mut work).unwrap();
        sequencer
            .store_actual(
                lookup,
                Some(Voice {
                    source: 1,
                    lifetime: key.lifetime,
                    correction: 0,
                    player: 0.0,
                    key: note,
                }),
            )
            .unwrap();
        assert_eq!(work, 1);
    }
    // Both legitimate survivors collide in a lifetime%64 directory. Changing
    // accepted expression/revision must not invalidate either stable address.
    let mut work = 0;
    for event in 0..200 {
        let i = event % 2;
        let key = [a, b][i];
        let lookup = sequencer.lookup_actual(key, Some(addresses[i]), false, &mut work).unwrap();
        let mut voice = sequencer.actual[usize::from(lookup.index)].unwrap();
        voice.player = event as f64 / 1000.0;
        sequencer.store_actual(lookup, Some(voice)).unwrap();
    }
    assert_eq!(work, 200, "sparse lifetimes and changing values still have bounded direct lookup");
    let mut work = 3840;
    assert!(sequencer.lookup_actual(a, None, false, &mut work).is_none());
    assert_eq!(work, 3841, "only the checked hint fits; no fallback may run before debit");
    let mut work = 3839;
    let lookup = sequencer.lookup_actual(a, None, false, &mut work).unwrap();
    assert_ne!(lookup.index, NO_VOICE);
    assert_eq!(work, 4096);
    let voice = sequencer.actual[usize::from(lookup.index)];
    sequencer.store_actual(lookup, voice).unwrap();
    assert_eq!(work, 4096, "application reuses the paid resolution even with no work left");
    for stale in [
        ActualKey { epoch: a.epoch + 1, ..a },
        ActualKey { lease: Lease { incarnation: lease.incarnation + 1, ..lease }, ..a },
        ActualKey { lease: Lease { session: lease.session + 1, ..lease }, ..a },
        ActualKey { lease: Lease { source: harmonigraph_core::SourceId(13), ..lease }, ..a },
        ActualKey { lifetime: a.lifetime + 1, ..a },
    ] {
        let mut work = 0;
        let absent = sequencer.lookup_actual(stale, Some(addresses[0]), false, &mut work).unwrap();
        assert_eq!(absent.index, NO_VOICE, "every authority component must match the reused hint");
        assert_eq!(work, 257);
        sequencer.store_actual(absent, None).unwrap();
        assert_eq!(work, 257, "proven absence never triggers a second scan");
        assert_eq!(sequencer.actual.iter().flatten().count(), 2);
    }
    let source = hub.direct.test_snapshot();
    hub.reset_idle_clock(ClockId { runtime_session: 0, epoch: 5 });
    assert!(hub.sequencer.actual.iter().all(Option::is_none));
    assert!(hub.sequencer.actual_keys.iter().all(Option::is_none));
    assert!(hub.sequencer.context.iter().all(Option::is_none));
    assert_eq!(hub.sequencer.actual_free.len(), HELD_SESSION);
    assert_eq!(
        hub.direct.test_snapshot(),
        source,
        "no-offer observation reset leaves forwarding ownership untouched"
    );
}
