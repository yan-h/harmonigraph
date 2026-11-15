//! Learn's reading of the Hub's rows through the exported CLAP factory,
//! including real source admission, sequencing, departures and resets.
use super::*;
use harmonigraph_core::configuration::ConfigEdit;
use harmonigraph_core::tuning::{Incomplete, PitchClass};

/// How many distinct pitch classes Learn would read from the Hub's rows now.
fn heard(device: &Device) -> Result<usize, Incomplete> {
    hub_wrapper(device).test_inspect_plugin(|plugin| {
        let mut scratch = [PitchClass::from_microcents(0); LEARN_CLASSES];
        plugin.aggregation.as_ref().unwrap().learn_classes(&mut scratch)
    })
}

/// Five sources of 64 held notes are 320 voices, past the 256 the session's
/// adaptive context holds: Learn hears every one and learns from them, moving
/// a just fifth to the 12-TET one the keys were played in.
#[test]
fn learn_hears_more_held_voices_than_the_adaptive_context_holds() {
    let _scope = crate::test_scope::enter();
    let mut hub = Device::new(false);
    hub.activate();
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let mailbox = hub_wrapper(&hub).configuration_handle().unwrap();
    let fifth = || crate::configuration::view(mailbox.visible().0, false).resolved.tuning.three;
    let learn = ConfigEdit { learning: Some(true), ..Default::default() };
    mailbox.submit(crate::configuration::packet(learn)).unwrap();
    // Every Tune pairs before any plays: a pairing is a session cut.
    let tunes: Vec<_> = (0..5)
        .map(|_| {
            let mut tune = Device::new(true);
            tune.shared().set_retune(false);
            tune.activate();
            tune
        })
        .collect();
    for tune in &tunes {
        tune.run(0, (0..64).map(|key| note(key, 0, key as i16, 0, true)).collect(), None);
    }
    hub.run(0, vec![], None);
    let just = harmonigraph_core::tuning::microcents(harmonigraph_core::tuning::THREE_JUST);
    assert_eq!(fifth(), just, "nothing is learned before the rows hold the chord");
    hub.run(64, vec![], None);
    let held: usize = (0..=DIRECT).map(|row| inspect_hub(&hub, |hub| hub.test_held(row))).sum();
    assert_eq!(held, 5 * HELD_PER_SOURCE, "the fixture never held 320 voices");
    assert_eq!(heard(&hub), Ok(12), "64 keys fold into twelve classes");
    assert_eq!(fifth(), harmonigraph_core::tuning::microcents(700.0), "Learn learned");
    drop(tunes);
}

#[test]
fn incomplete_source_blocks_learning_until_departure_or_reset() {
    for overflow in [false, true] {
        let _scope = crate::test_scope::enter();
        let mut hub = Device::new(false);
        hub.activate();
        // Armed through the owner, so its own gate is what refuses and recovers.
        let mailbox = hub_wrapper(&hub).configuration_handle().unwrap();
        let learn = ConfigEdit { learning: Some(true), ..Default::default() };
        mailbox.submit(crate::configuration::packet(learn)).unwrap();
        let refused = || mailbox.visible().0.status & 1 != 0;
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
        assert_eq!(heard(&hub), Ok(12));
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
        assert_eq!(heard(&hub), Err(Incomplete), "a row that lost an event refuses");
        assert!(refused());
        bad.run(320, vec![note(1, 0, 1, 0, false)], None);
        hub.run(320, vec![], None);
        hub.run(384, vec![], None);
        assert!(refused(), "release cannot repair missing state");
        if overflow {
            session::session().reset();
        } else {
            drop(bad);
        }
        hub.run(448, vec![], None);
        assert_eq!(heard(&hub), Ok(0), "reset and departure both authoritatively cut state");
        assert!(!refused(), "the owner recovers with the state, not at a host reset");
        let revision = mailbox.visible().0.revision;
        // A just third, so what Learn derives differs from the 12-TET it heard.
        let bend = f64::from(harmonigraph_core::tuning::FIVE_JUST - 400.0) / 100.0;
        let mut triad: Vec<_> = [60, 64, 67].map(|key| note(key, 0, key as i16, 0, true)).to_vec();
        triad.push(expression(64, bend, 0));
        good.run(448, triad, None);
        hub.run(512, vec![], None);
        hub.run(576, vec![], None);
        assert_eq!(heard(&hub), Ok(3));
        assert_ne!(
            mailbox.visible().0.revision,
            revision,
            "a complete new chord is learned after recovery"
        );
    }
}
