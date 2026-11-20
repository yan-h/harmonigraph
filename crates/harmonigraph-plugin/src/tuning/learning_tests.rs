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
        plugin.aggregation.as_ref().unwrap().held_classes(&mut scratch)
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

/// With Decode fifths on, Learn is suspended as under Lattice Map: the
/// lattice is drawn in the lattice-board's tuning, and what is left to hear is
/// the 12-TET tracks beside it. A chord held on a Tune with Retune off, which
/// the rows still hold for Learn, leaves the just fifth standing from the very
/// callback that turns decoding on, and is learned once decoding is off.
#[test]
fn learn_is_suspended_while_decoding() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub();
    hub.activate();
    musical_tests::configure(&hub, harmonigraph_core::Tuning::just());
    let settings = settings(&hub);
    let fifth = || settings.view().resolved.tuning.three;
    assert!(settings.submit(ConfigEdit { learning: Some(true), ..Default::default() }));
    let mut plain = tune();
    plain.shared().set_retune(false);
    plain.activate();
    let chord =
        [60, 64, 67].into_iter().zip(1..).map(|(key, id)| note(id, 0, key, 0, true)).collect();
    plain.run(0, chord, None);
    hub.run(0, vec![], None);
    // Decoding turns on after the chord reached the rows, and Learn's next
    // reading, on the callback that adopts the edit, is already suspended.
    let decoding = |on| harmonigraph_core::configuration::PolicyConfig {
        decode_fifths: on,
        ..Default::default()
    };
    assert!(settings.submit(ConfigEdit { policy: Some(decoding(true)), ..Default::default() }));
    hub.run(64, vec![], None);
    let just = harmonigraph_core::tuning::microcents(harmonigraph_core::tuning::THREE_JUST);
    assert_eq!((heard(&hub), fifth()), (3, just), "Learn learned while decoding");
    assert!(settings.submit(ConfigEdit { policy: Some(decoding(false)), ..Default::default() }));
    hub.run(128, vec![], None);
    assert_eq!(fifth(), harmonigraph_core::tuning::microcents(700.0), "Learn did not resume");
}
