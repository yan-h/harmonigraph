//! Learn's reading of the Hub's rows through the exported CLAP factory,
//! including real source admission and sequencing.
use super::*;
use harmonigraph_core::configuration::ConfigEdit;
use harmonigraph_core::tuning::PitchClass;

/// How many held pitches Learn would read from the Hub's rows now, repeats
/// included: one per voice every row holds.
fn heard(device: &Device) -> usize {
    hub_wrapper(device).test_inspect_plugin(|plugin| {
        let mut scratch = [PitchClass::from_microcents(0); HELD_VOICES];
        let decoded = plugin.configuration.as_ref().unwrap().current().policy.decode_fifths;
        plugin.aggregation.as_ref().unwrap().held_classes(decoded, &mut scratch)
    })
}

/// Five sources of 64 held notes are 320 voices, past the 256 the session's
/// adaptive context holds: Learn hears every one and learns from them, moving
/// a just fifth to the 12-TET one the keys were played in.
#[test]
fn learn_hears_more_held_voices_than_the_adaptive_context_holds() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub().with_adaptive();
    hub.activate();
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let settings = settings(&hub);
    let fifth = || settings.view().resolved.tuning.three;
    let learn = ConfigEdit { learning: Some(true), ..Default::default() };
    assert!(settings.submit(learn));
    // Every Tune pairs before any plays: a pairing is a session cut.
    let tunes: Vec<_> = (0..5)
        .map(|_| {
            let mut tune = tune();
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
    assert_eq!(heard(&hub), 320, "Learn reads every voice of every row");
    assert_eq!(fifth(), harmonigraph_core::tuning::microcents(700.0), "Learn learned");
    drop(tunes);
}

/// With Decode fifths on, a Tune with Retune on declares its tuning, so Learn
/// does not hear its raw 12-TET keys: a chord held there leaves the just fifth
/// standing, from the very callback that turns decoding on, and the same chord
/// on a Tune with Retune off is learned.
#[test]
fn learn_does_not_hear_a_decoded_tune() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub();
    hub.activate();
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let settings = settings(&hub);
    let fifth = || settings.view().resolved.tuning.three;
    assert!(settings.submit(ConfigEdit { learning: Some(true), ..Default::default() }));
    let mut decoded = tune();
    decoded.activate();
    let mut plain = tune();
    plain.shared().set_retune(false);
    plain.activate();
    let chord = |first: i32| {
        [60, 64, 67].into_iter().zip(first..).map(|(key, id)| note(id, 0, key, 0, true)).collect()
    };
    decoded.run(0, chord(1), None);
    plain.run(0, vec![], None);
    hub.run(0, vec![], None);
    // Decoding turns on after the chord reached the rows, and Learn's next
    // reading, on the callback that adopts the edit, already skips them.
    let policy = harmonigraph_core::configuration::PolicyConfig {
        decode_fifths: true,
        ..Default::default()
    };
    assert!(settings.submit(ConfigEdit { policy: Some(policy), ..Default::default() }));
    hub.run(64, vec![], None);
    let just = harmonigraph_core::tuning::microcents(harmonigraph_core::tuning::THREE_JUST);
    let slot = decoded.shared().slot.load(Ordering::Acquire) as u8;
    assert_eq!(inspect_hub(&hub, |hub| hub.test_held(slot)), 3, "the decoded Tune holds the chord");
    assert_eq!((heard(&hub), fifth()), (0, just), "Learn heard the decoded Tune");
    plain.run(64, chord(4), None);
    hub.run(128, vec![], None);
    hub.run(192, vec![], None);
    assert_eq!(heard(&hub), 3);
    assert_eq!(fifth(), harmonigraph_core::tuning::microcents(700.0), "Learn learned");
    // With Decode fifths off, Learn hears the Tune with Retune on again, the
    // chord it struck decoded included.
    assert!(settings.submit(ConfigEdit { policy: Some(Default::default()), ..Default::default() }));
    hub.run(256, vec![], None);
    hub.run(320, vec![], None);
    assert_eq!(heard(&hub), 6, "Learn did not hear the Tune once decoding was off");
}
