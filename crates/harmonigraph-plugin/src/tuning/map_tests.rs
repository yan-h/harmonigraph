//! Map timing through real CLAP ingress, Hub ordering and Tune output.
use super::*;
use harmonigraph_core::configuration::ConfigEdit;
use harmonigraph_core::lattice_map::{LatticeMap, TuningEngine};
use harmonigraph_core::{LatticePos, Tuning};

fn install(hub: &Device) -> (LatticeMap, LatticeMap) {
    musical_tests::configure(hub, Tuning::just());
    hub.shared().set_retune(true);
    let first = LatticeMap::default();
    let mut second = first;
    second.replace(LatticePos::new(4, 0, 0));
    hub_wrapper(hub).test_inspect_plugin(|plugin| {
        assert_eq!(
            plugin.params.maps.write(|d| d.capture(second, "Pythagorean E".into())),
            Some(1)
        );
    });
    (first, second)
}
fn voice(hub: &Device, source: u8, key: u8) -> harmonigraph_core::canonical::VoiceBaseline {
    inspect_hub(hub, |hub| hub.test_voice(source, 0, key)).unwrap()
}

#[test]
fn lattice_map_automation_precedes_coincident_attacks_in_both_callback_orders() {
    let _scope = crate::test_scope::enter();
    for frames in [32, 128, 512] {
        for hub_first in [false, true] {
            let mut hub = hub().with_adaptive();
            hub.activate_format(48000.0, frames);
            let (first, mut second) = install(&hub);
            // Rotation is zero, so MIDI E still reaches the substituted base E slot.
            second.position = LatticePos::new(48, -4, 4);
            let mut tune = tune();
            tune.activate_format(48000.0, frames);
            // 2x tolerates Hub-first callbacks without changing input timestamps.
            tune.shared().request_delay(2);
            tune.reactivate_format(48000.0, frames);
            hub.run_format(0, vec![parameter("tuning-engine", 2.0, 0)], None, None, frames);
            let start = i64::from(frames);
            let boundary = frames / 2;
            let input = vec![note(1, 0, 64, boundary - 1, true), note(2, 0, 76, boundary, true)];
            let mut automation = vec![
                parameter("map-fifths", 17.0, boundary),
                parameter("map-thirds-extension", 8.0, boundary),
                parameter("lattice-map", 1.0, boundary),
                parameter("map-sevenths", 3.0, boundary),
                parameter("map-fifths-extension", 13.0, boundary),
                parameter("map-thirds", 15.0, boundary),
                parameter("map-sevenths-extension", 10.0, boundary),
            ];
            if hub_first {
                automation.reverse();
            }
            if hub_first {
                hub.run_format(start, automation, None, None, frames);
                tune.run_format(start, input, None, None, frames);
            } else {
                tune.run_format(start, input, None, None, frames);
                hub.run_format(start, automation, None, None, frames);
            }
            hub.run_format(start * 2, vec![], None, None, frames);
            tune.run_format(start * 2, vec![], None, None, frames);
            let output = tune.run_format(start * 3, vec![], None, None, frames);
            assert_eq!(
                voice(&hub, 0, 64).frozen_offset_microcents,
                first.correction(64, Tuning::just())
            );
            assert_eq!(
                voice(&hub, 0, 76).frozen_offset_microcents,
                second.correction(76, Tuning::just())
            );
            assert_eq!(voice(&hub, 0, 76).attack_node, Some(second.node(76)));
            let corrections: Vec<_> = output
                .values
                .iter()
                .filter_map(|(_, event)| match event {
                    Event::Expression { kind: 2, value, .. } => Some(*value),
                    _ => None,
                })
                .collect();
            assert_eq!(corrections.len(), 2);
            assert!(
                (corrections[0] - first.correction(64, Tuning::just()) as f64 / 1e8).abs() < 1e-8
            );
            assert!(
                (corrections[1] - second.correction(76, Tuning::just()) as f64 / 1e8).abs() < 1e-8
            );
            assert_eq!(tune.shared().misses.load(Ordering::Relaxed), 0);
            // Following callbacks seed plain values without translating twice.
            hub.run_format(start * 4, vec![note(3, 0, 88, 0, true)], None, None, frames);
            assert_eq!(voice(&hub, crate::tuning::DIRECT, 88).attack_node, Some(second.node(88)));
            // Updating only one lane retains the other lane's contribution,
            // including when the retained component is negative.
            hub.run_format(
                start * 5,
                vec![parameter("map-thirds", 4.0, 0), note(4, 0, 89, 0, true)],
                None,
                None,
                frames,
            );
            second.position.fives = -15;
            assert_eq!(voice(&hub, crate::tuning::DIRECT, 89).attack_node, Some(second.node(89)));
            hub.run_format(
                start * 6,
                vec![parameter("map-thirds-extension", 11.0, 0), note(5, 0, 90, 0, true)],
                None,
                None,
                frames,
            );
            second.position.fives = 15;
            assert_eq!(voice(&hub, crate::tuning::DIRECT, 90).attack_node, Some(second.node(90)));
            assert_eq!(
                voice(&hub, 0, 64).frozen_offset_microcents,
                first.correction(64, Tuning::just())
            );
        }
    }
}

#[test]
fn lattice_map_held_notes_keep_onset_while_chased_notes_and_off_use_current_state() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub().with_adaptive();
    hub.activate();
    let (first, second) = install(&hub);
    hub.run(0, vec![note(1, 0, 64, 0, true), parameter("tuning-engine", 2.0, 0)], None);
    hub.run(512, vec![parameter("lattice-map", 1.0, 0)], None);
    assert_eq!(
        voice(&hub, crate::tuning::DIRECT, 64).frozen_offset_microcents,
        first.correction(64, Tuning::just())
    );
    // A chased attack needs no remembered original onset.
    hub.run(1024, vec![note(2, 0, 76, 0, true)], None);
    assert_eq!(
        voice(&hub, crate::tuning::DIRECT, 76).frozen_offset_microcents,
        second.correction(76, Tuning::just())
    );
    hub.run(
        1536,
        vec![parameter("tuning-engine", 0.0, 0), note(3, 0, 88, 0, true), expression(3, 0.25, 0)],
        None,
    );
    assert_eq!(voice(&hub, crate::tuning::DIRECT, 88).frozen_offset_microcents, 0);
    assert_eq!(voice(&hub, crate::tuning::DIRECT, 88).player_tuning, 0.25);
    assert_eq!(
        voice(&hub, crate::tuning::DIRECT, 76).frozen_offset_microcents,
        second.correction(76, Tuning::just())
    );
}

/// A host value past the engine's or Follow's top step decodes as that top
/// step, which is what the host's label reads, rather than as Off. Read before
/// the next callback, whose boundary reseeds from the clamped parameters.
#[test]
fn an_engine_or_follow_event_past_the_top_step_selects_the_top_step() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub().with_adaptive();
    hub.activate();
    hub.run(0, vec![parameter("tuning-engine", 5.0, 0), parameter("map-follow", 4.0, 0)], None);
    let playback =
        hub_wrapper(&hub).test_inspect_plugin(|plugin| plugin.params.map_playback.read());
    assert_eq!(playback.engine, TuningEngine::LatticeMap);
    assert_eq!(playback.follow, harmonigraph_core::lattice_map::Follow::ThirdsAndFifths);
}

/// The tuning a note arrives with is the expression on its own sample: one
/// later in the same callback moves it from there, and does not become the
/// pitch the map's correction was decided from.
#[test]
fn lattice_map_corrects_from_the_tuning_on_the_onsets_own_sample() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub().with_adaptive();
    hub.activate();
    let (first, _) = install(&hub);
    hub.run(0, vec![parameter("tuning-engine", 2.0, 0)], None);
    // The tuning the instrument is sent at the onset's sample.
    let sent = |values: &[(u32, Event)]| {
        values.iter().find_map(|(time, event)| match event {
            Event::Expression { kind: 2, value, .. } if *time == 0 => Some(*value),
            _ => None,
        })
    };
    let at_onset = vec![note(1, 0, 64, 0, true), expression(1, 0.1, 0)];
    let alone_sent = sent(&hub.run(512, at_onset, None).values);
    let alone = voice(&hub, crate::tuning::DIRECT, 64);
    hub.run(1024, vec![note(1, 0, 64, 0, false)], None);
    let with_later = vec![note(2, 0, 64, 0, true), expression(2, 0.1, 0), expression(2, 0.3, 30)];
    let later_sent = sent(&hub.run(1536, with_later, None).values);
    let later = inspect_hub(&hub, |hub| hub.test_voice(crate::tuning::DIRECT, 0, 64)).unwrap();
    assert_eq!(later.frozen_offset_microcents, alone.frozen_offset_microcents);
    assert_eq!(later.attack_node, Some(first.node(64)));
    assert!(alone_sent.is_some(), "the onset's own tuning goes on the wire at its sample");
    assert_eq!(later_sent, alone_sent, "and the later one does not replace it there");
}

/// A per-note tuning expression after the onset draws the pitch that goes on
/// the wire: the player's value, the channel's bend and the map's frozen
/// correction together, in the Hub's voice that the display and the take read.
#[test]
fn lattice_map_draws_a_later_tuning_expression_with_its_bend_and_correction() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub().with_adaptive();
    hub.activate();
    let (first, _) = install(&hub);
    hub.run(0, vec![parameter("tuning-engine", 2.0, 0)], None);
    let Input::Note(mut struck) = note(1, 0, 64, 0, true) else { unreachable!() };
    struck.velocity = 0.375;
    hub.run(512, vec![Input::Note(struck)], None);
    let held = voice(&hub, crate::tuning::DIRECT, 64);
    let correction = first.correction(64, Tuning::just());
    assert!(correction != 0, "the map moves E, so the correction is in the pitch");
    assert_eq!(held.frozen_offset_microcents, correction);
    assert_eq!(held.velocity, 0.375, "the attack's velocity is the voice's");
    hub.run(1024, vec![raw_midi([0xE0, 0x00, 0x60], 0)], None);
    let bent = voice(&hub, crate::tuning::DIRECT, 64);
    assert!(bent.pitch_microcents != held.pitch_microcents, "the bend moves the held voice");
    hub.run(1536, vec![expression(1, 0.25, 0)], None);
    let expressed = voice(&hub, crate::tuning::DIRECT, 64);
    assert_eq!(expressed.pitch_microcents, bent.pitch_microcents + 25_000_000);
}

#[test]
fn lattice_map_tunes_a_pre_tuned_keyboard_to_its_rounded_key_rather_than_twice() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub().with_adaptive();
    hub.activate();
    let (first, _) = install(&hub);
    hub.run(0, vec![parameter("tuning-engine", 2.0, 0)], None);
    // A keyboard already sending the map's own 5-limit E, as a meantone or MTS
    // keyboard does. Its own tuning selects the slot; it is not a head start
    // the map adds another 13.7¢ to.
    let e = first.correction(64, Tuning::just());
    let semitones = e as f64 / 100_000_000.0;
    hub.run(512, vec![note(1, 0, 64, 0, true), expression(1, semitones, 0)], None);
    let landed = voice(&hub, crate::tuning::DIRECT, 64);
    assert_eq!(landed.player_tuning, semitones, "the incoming tuning is still the player's");
    assert_eq!(landed.frozen_offset_microcents, 0, "already on the map, nothing left to add");
    assert_eq!(landed.attack_node, Some(first.node(64)));
    assert_eq!(landed.pitch_microcents, 64 * 100_000_000 + e);
    // Sixty cents sharp of E sounds nearer F, so it takes F's assignment and
    // is pulled onto it rather than left between two slots.
    hub.run(1024, vec![note(2, 0, 76, 0, true), expression(2, 0.6, 0)], None);
    let rounded = voice(&hub, crate::tuning::DIRECT, 76);
    assert_eq!(rounded.attack_node, Some(first.node(77)));
    assert_eq!(rounded.pitch_microcents, 77 * 100_000_000 + first.correction(77, Tuning::just()));
    // A channel bend standing at the attack is part of the pitch that arrived,
    // exactly as the adaptive engine reads it.
    hub.run(1536, vec![raw_midi([0xE0, 0x00, 0x60], 0), note(3, 0, 67, 1, true)], None);
    let bent = voice(&hub, crate::tuning::DIRECT, 67);
    assert!(bent.frozen_offset_microcents != first.correction(67, Tuning::just()));
    assert_eq!(bent.pitch_microcents, 68 * 100_000_000 + first.correction(68, Tuning::just()));
}

#[test]
fn lattice_map_shared_axes_and_shape_edits_apply_to_new_attacks_only() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub().with_adaptive();
    hub.activate();
    let (first, second) = install(&hub);
    hub.run(0, vec![parameter("tuning-engine", 2.0, 0)], None);
    hub.run(256, vec![note(1, 0, 67, 9, true)], None);
    let fifth = harmonigraph_core::tuning::microcents(699.0);
    settings(&hub).submit(harmonigraph_core::configuration::ConfigEdit::axis(
        harmonigraph_core::configuration::TuningAxis::Three,
        fifth,
    ));
    hub.run(512, vec![note(2, 0, 79, 10, true)], None);
    assert_eq!(
        voice(&hub, crate::tuning::DIRECT, 67).frozen_offset_microcents,
        first.correction(67, Tuning::just())
    );
    assert!(
        (voice(&hub, crate::tuning::DIRECT, 79).frozen_offset_microcents + 1_000_000).abs() < 100
    );
    hub_wrapper(&hub)
        .test_inspect_plugin(|plugin| assert!(plugin.params.maps.write(|d| d.reshape(0, second))));
    hub.run(
        1024,
        vec![
            parameter("lattice-map", 0.0, 0),
            parameter("map-fifths", 5.0, 0),
            parameter("map-thirds", 12.0, 0),
            parameter("map-sevenths", 7.0, 0),
            note(3, 0, 64, 0, true),
        ],
        None,
    );
    // Keep label rotation zero so the edited shape differs at MIDI E.
    let offset = LatticePos::new(-4, 3, -2);
    let edited = voice(&hub, crate::tuning::DIRECT, 64);
    assert_eq!(edited.attack_node, Some(LatticeMap { position: offset, ..second }.node(64)));
    hub_wrapper(&hub)
        .test_inspect_plugin(|plugin| assert!(plugin.params.maps.write(|d| d.reshape(0, first))));
    hub.run(1536, vec![note(4, 0, 76, 0, true)], None);
    assert_eq!(
        voice(&hub, crate::tuning::DIRECT, 76).attack_node,
        Some(LatticeMap { position: offset, ..first }.node(76))
    );
    assert_eq!(
        voice(&hub, crate::tuning::DIRECT, 64).frozen_offset_microcents,
        edited.frozen_offset_microcents
    );
    // Map, lanes and Follow are automatable stepped state selectors, never
    // additive modulation.
    let params = hub.params();
    let count = unsafe { params.count.unwrap()(hub.plugin) };
    let ids = [
        "lattice-map",
        "map-fifths",
        "map-thirds",
        "map-sevenths",
        "map-fifths-extension",
        "map-thirds-extension",
        "map-sevenths-extension",
        "map-follow",
    ];
    let mut found = 0;
    for index in 0..count {
        let mut info: clap_sys::ext::params::clap_param_info = unsafe { std::mem::zeroed() };
        assert!(unsafe { params.get_info.unwrap()(hub.plugin, index, &mut info) });
        if let Some(id) =
            ids.iter().find(|id| info.id == harmonigraph_clap::wrapper::hash_param_id(id))
        {
            use clap_sys::ext::params::*;
            assert_ne!(info.flags & CLAP_PARAM_IS_STEPPED, 0);
            assert_ne!(info.flags & CLAP_PARAM_IS_AUTOMATABLE, 0);
            assert_eq!(info.flags & CLAP_PARAM_IS_MODULATABLE, 0);
            if !matches!(*id, "lattice-map" | "map-follow") {
                assert_eq!(info.min_value, 0.0);
                assert_eq!(info.max_value, 18.0);
                assert_eq!(info.default_value, 9.0);
            }
            if id.ends_with("-extension") {
                for (index, text) in [(0.0, "-90"), (9.0, "0"), (18.0, "90")] {
                    let mut buffer = [0i8; 32];
                    assert!(unsafe {
                        params.value_to_text.unwrap()(
                            hub.plugin,
                            info.id,
                            index,
                            buffer.as_mut_ptr(),
                            32,
                        )
                    });
                    assert_eq!(
                        unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }.to_str().unwrap(),
                        text
                    );
                    let mut parsed = -1.0;
                    assert!(unsafe {
                        params.text_to_value.unwrap()(
                            hub.plugin,
                            info.id,
                            buffer.as_ptr(),
                            &mut parsed,
                        )
                    });
                    assert_eq!(parsed, index);
                }
            }
            found += 1;
        }
    }
    assert_eq!(found, ids.len());
}

/// A record the Hub holds over is assigned under the configuration in force
/// at its own sample, not the one the next block adopts: an editor edit of the
/// fifth landing between the two leaves the held-over G on the old fifth,
/// while a G struck in the new block takes the new one.
#[test]
fn a_held_over_onset_keeps_its_own_blocks_configuration_across_an_editor_edit() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub().with_adaptive();
    hub.activate_format(44100.0, 512);
    install(&hub);
    let mut source = tune();
    source.activate_format(44100.0, 512);
    hub.run_format(0, vec![parameter("tuning-engine", 1.0, 0)], None, None, 512);
    for raw in [512, 1024] {
        source.run_format(raw, vec![], None, None, 512);
        hub.run_format(raw, vec![], None, None, 512);
    }
    // The Hub runs first in this block, so the source's G is held over.
    hub.run_format(1536, vec![], None, None, 512);
    source.run_format(1536, vec![note(1, 0, 55, 0, true)], None, None, 512);
    let edit = ConfigEdit::axis(harmonigraph_core::configuration::TuningAxis::Three, 699_000_000);
    settings(&hub).submit(edit);
    hub.run_format(2048, vec![note(2, 0, 67, 0, true)], None, None, 512);
    let just = harmonigraph_core::tuning::microcents(harmonigraph_core::tuning::THREE_JUST);
    let held_over = voice(&hub, 0, 55);
    let struck = voice(&hub, crate::tuning::DIRECT, 67);
    assert_eq!(held_over.attack_node, Some(LatticePos::new(1, 0, 0)));
    assert_eq!(struck.attack_node, Some(LatticePos::new(1, 0, 0)));
    assert!((held_over.frozen_offset_microcents - i64::from(just - 700_000_000)).abs() < 100);
    assert!((struck.frozen_offset_microcents + 1_000_000).abs() < 100);
}

/// A record whose sample the history does not reach: here a Tune's, struck on
/// a callback past the end of the Hub's next one, as a Tune whose counter has
/// jumped ahead strikes it. `at` never extrapolates through a callback whose
/// automation it has not seen, so it has nothing to answer with. (A steady
/// counter that goes back no longer reaches this path: the shell runs it on,
/// #1881.)
///
/// Every engine then assigns from the state it holds now, as it would have had
/// the history still reached that sample (#920, #1792), rather than leaving the
/// onset raw.
#[test]
fn a_retained_onset_the_history_does_not_reach_is_assigned_from_the_live_state() {
    for engine in [TuningEngine::Adaptive, TuningEngine::LatticeMap] {
        // The same onset twice: once struck inside the history, which is the
        // control, and once struck past the Hub's next callback.
        let mut landed: [(Option<LatticePos>, i64, bool); 2] = Default::default();
        for (slot, ahead) in [false, true].into_iter().enumerate() {
            let _scope = crate::test_scope::enter();
            let mut hub = hub().with_adaptive();
            hub.activate_format(44100.0, 512);
            install(&hub);
            let mut source = tune();
            source.activate_format(44100.0, 512);
            let selected = if engine == TuningEngine::LatticeMap { 2.0 } else { 1.0 };
            hub.run_format(0, vec![parameter("tuning-engine", selected, 0)], None, None, 512);
            for raw in [512, 1024] {
                source.run_format(raw, vec![], None, None, 512);
                hub.run_format(raw, vec![], None, None, 512);
            }
            // The Hub runs first in this block, so the copy of the note lands
            // in the ring behind the drain that would have taken it.
            hub.run_format(1536, vec![], None, None, 512);
            let struck = if ahead { 4096 } else { 1536 };
            source.run_format(struck, vec![note(1, 0, 50, 0, true)], None, None, 512);
            assert_eq!(inspect_hub(&hub, |hub| hub.test_held(0)), 0, "the copy is still in flight");
            hub.run_format(2048, vec![], None, None, 512);
            let voice = voice(&hub, 0, 50);
            let status = hub.shared().status.load(Ordering::Relaxed);
            landed[slot] =
                (voice.attack_node, voice.frozen_offset_microcents, status & session::POLICY != 0);
        }
        let (node, correction, policy) = landed[0];
        assert!(node.is_some() && correction != 0, "{engine:?} corrects a record it caught up to");
        assert!(!policy, "{engine:?} refuses nothing when the history is still there");
        assert_eq!(landed[1], landed[0], "{engine:?} assigns a record the history lost");
    }
}

#[test]
fn lattice_map_follow_moves_the_map_for_a_chord_and_drifts_with_what_it_heard() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub().with_adaptive();
    hub.activate();
    install(&hub);
    let map = LatticeMap::default();
    let node = |hub: &Device, key| voice(hub, crate::tuning::DIRECT, key).attack_node;
    let at = |offset: LatticePos, key: u8| LatticeMap { position: offset, ..map }.node(key.into());
    let on = |id, key| note(id, 0, key, 0, true);
    let off = |id, key| note(id, 0, key, 0, false);
    hub.run(0, vec![parameter("tuning-engine", 2.0, 0), parameter("map-follow", 2.0, 0)], None);
    // D F A on one sample: the map steps down a fifth, so D is 10/9 and the
    // triad has no wolf.
    hub.run(512, vec![on(1, 62), on(2, 65), on(3, 69)], None);
    let down = LatticePos::new(-1, 0, 0);
    for key in [62, 65, 69] {
        assert_eq!(node(&hub, key), Some(at(down, key)));
    }
    // F and A end on the sample G and B start, and the D held across pulls
    // them a comma down with it rather than sounding a wolf against it.
    hub.run(1024, vec![off(2, 65), off(3, 69), on(4, 67), on(5, 71)], None);
    let drifted = LatticePos::new(-2, 0, 0);
    for key in [67, 71] {
        assert_eq!(node(&hub, key), Some(at(drifted, key)));
    }
    assert_eq!(node(&hub, 62), Some(at(down, 62)), "a held note keeps its onset tuning");
    // Everything released, C E G keeps the G just heard rather than snapping
    // back to the automated place: the map prefers to drift.
    hub.run(1536, vec![off(1, 62), off(4, 67), off(5, 71)], None);
    hub.run(2048, vec![on(6, 60), on(7, 64), on(8, 67)], None);
    let further = LatticePos::new(-3, 0, 0);
    assert_eq!(node(&hub, 67), Some(at(drifted, 67)), "the released G keeps its pitch");
    for key in [60, 64, 67] {
        assert_eq!(node(&hub, key), Some(at(further, key)));
    }
    hub.run(2560, vec![], None);
    hub_wrapper(&hub).test_inspect_plugin(|plugin| {
        let view = crate::lattice_maps::view(&plugin.params);
        assert_eq!(view.followed, further, "the editor draws the map where it sounds");
        assert_eq!(view.playback.map, Some(LatticeMap { position: further, ..map }));
    });
    // Off and on again with nothing played starts over from the automated
    // place, so D F A steps from there rather than from three fifths down.
    hub.run(3072, vec![off(6, 60), off(7, 64), off(8, 67), parameter("map-follow", 0.0, 0)], None);
    hub.run(3584, vec![parameter("map-follow", 2.0, 0)], None);
    hub.run(4096, vec![], None);
    hub_wrapper(&hub).test_inspect_plugin(|plugin| {
        assert_eq!(crate::lattice_maps::view(&plugin.params).followed, LatticePos::ORIGIN);
    });
    hub.run(4608, vec![on(9, 62), on(10, 65), on(11, 69)], None);
    for key in [62, 65, 69] {
        assert_eq!(node(&hub, key), Some(at(down, key)));
    }
    // Turned off, the next attack lands on the automated map, wolf and all.
    hub.run(
        5120,
        vec![off(9, 62), off(10, 65), off(11, 69), parameter("map-follow", 0.0, 0), on(12, 62)],
        None,
    );
    assert_eq!(node(&hub, 62), Some(map.node(62)), "the map no longer follows");
}

#[test]
fn lattice_map_follow_keeps_a_just_released_note_at_its_pitch() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub().with_adaptive();
    hub.activate();
    install(&hub);
    let map = LatticeMap::default();
    let node = |hub: &Device, key| voice(hub, crate::tuning::DIRECT, key).attack_node;
    hub.run(0, vec![parameter("tuning-engine", 2.0, 0), parameter("map-follow", 2.0, 0)], None);
    // Bb F C G, a chord a second, each released before the next. Bb major
    // needs a 10/9 D, so the map steps down a fifth and stays for F and C.
    let second = 48_000;
    let chords: [&[i16]; 4] = [&[58, 62, 65], &[65, 69, 72], &[60, 64, 67], &[67, 71, 74]];
    for (index, chord) in chords.iter().enumerate() {
        // Each chord's notes share the note id of its own index plus one.
        let id = index as i32 + 1;
        let mut input = Vec::new();
        if index > 0 {
            input.extend(chords[index - 1].iter().map(|&key| note(id - 1, 0, key, 0, false)));
        }
        input.extend(chord.iter().map(|&key| note(id, 0, key, 0, true)));
        hub.run(index as i64 * second, input, None);
    }
    // G B D sounds a wolf where the map stands. Stepping on down would tie with
    // stepping back, and a drift-first tie-break would go down; the G of the
    // C major just released is what decides, and it keeps its pitch.
    let back = LatticeMap { position: LatticePos::ORIGIN, ..map };
    for key in [67, 71, 74] {
        assert_eq!(node(&hub, key), Some(back.node(key.into())));
    }
}

#[test]
fn lattice_map_follow_scores_a_retriggered_note_without_its_predecessor() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub().with_adaptive();
    hub.activate();
    install(&hub);
    let map = LatticeMap::default();
    let node = |hub: &Device, key| voice(hub, crate::tuning::DIRECT, key).attack_node;
    hub.run(0, vec![parameter("tuning-engine", 2.0, 0), parameter("map-follow", 2.0, 0)], None);
    // A lone high D moves nothing. Struck again with F and A on one sample
    // it takes over the held D rather than being weighed against it, so the
    // triad mends its wolf as it would from rest. It sorts after F, whose
    // onset decides for the group, so only forgetting it with the whole
    // group keeps the old 9/8 D from pulling the triad onto its own spelling.
    hub.run(512, vec![note(1, 0, 74, 0, true)], None);
    hub.run(
        1024,
        vec![note(2, 0, 65, 0, true), note(3, 0, 69, 0, true), note(4, 0, 74, 0, true)],
        None,
    );
    let down = LatticeMap { position: LatticePos::new(-1, 0, 0), ..map };
    for key in [65, 69, 74] {
        assert_eq!(node(&hub, key), Some(down.node(key.into())));
    }
}

#[test]
fn lattice_map_follow_does_not_strand_a_note_replayed_over_what_is_held() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub().with_adaptive();
    hub.activate();
    install(&hub);
    let node = |hub: &Device, key| voice(hub, crate::tuning::DIRECT, key).attack_node;
    hub.run(0, vec![parameter("tuning-engine", 2.0, 0), parameter("map-follow", 2.0, 0)], None);
    // A chain of fifths from C, all held, then everything but the C let go.
    // A half-life apart, because the older notes must have faded for the
    // newest to pull the map along; struck together they hold it back.
    let half_life = 24_000;
    let chain = [48, 55, 62, 69, 76, 83, 90, 97];
    for (index, &key) in chain.iter().enumerate() {
        let id = index as i32 + 1;
        hub.run(i64::from(id) * half_life, vec![note(id, 0, key, 0, true)], None);
    }
    let g = node(&hub, 55);
    let release = chain.iter().enumerate().skip(1);
    let release = release.map(|(index, &key)| note(index as i32 + 1, 0, key, 0, false)).collect();
    hub.run(9 * half_life, release, None);
    // The map never left the C, so the replayed G is the G it left, a fifth
    // above that C, and not one a long way off at the far side of the map.
    hub.run(10 * half_life, vec![note(9, 0, 55, 0, true)], None);
    assert_eq!(node(&hub, 55), g);
    assert_eq!(g, Some(LatticePos::new(1, 0, 0)));
}

#[test]
fn map_follow_changes_leave_adaptive_context_alone() {
    let _scope = crate::test_scope::enter();
    let mut hub = hub().with_adaptive();
    hub.activate();
    install(&hub);
    // Adaptive, with a note held; Map Follow decides nothing here, so moving
    // it must not restart the context the next note is scored against.
    hub.run(0, vec![note(1, 0, 60, 0, true)], None);
    hub.run(512, vec![parameter("map-follow", 2.0, 0)], None);
    hub.run(1024, vec![note(2, 0, 64, 0, true)], None);
    assert_eq!(inspect_hub(&hub, |hub| hub.test_context()), 2, "the held C is still context");
}

/// A Hub on `engine` (its host step) with `tuning`, Decode fifths on at the
/// decoded fifth `fifth`, and two Tunes, at 48 kHz and 128-frame callbacks:
/// the first with Retune on, so decoded, the second with Retune off.
struct Decoding {
    hub: Device,
    decoded: Device,
    plain: Device,
    raw: i64,
}
impl Decoding {
    fn new(engine: f64, tuning: Tuning, fifth: i32) -> Self {
        Self::with(engine, tuning, fifth, true)
    }
    /// Adaptive's fixed strip: Follow spellings off, so the Position alone
    /// places it.
    fn fixed(tuning: Tuning, fifth: i32) -> Self {
        Self::with(1.0, tuning, fifth, false)
    }
    fn with(engine: f64, tuning: Tuning, fifth: i32, follow_spellings: bool) -> Self {
        let mut hub = hub();
        hub.activate_format(48_000.0, 128);
        musical_tests::configure(&hub, tuning);
        let policy = harmonigraph_core::configuration::PolicyConfig {
            decode_fifths: true,
            decoded_fifth: fifth,
            follow_spellings,
            ..Default::default()
        };
        settings(&hub).submit(ConfigEdit { policy: Some(policy), ..Default::default() });
        let mut decoded = tune();
        decoded.activate_format(48_000.0, 128);
        let mut plain = tune();
        plain.shared().set_retune(false);
        plain.activate_format(48_000.0, 128);
        for tune in [&decoded, &plain] {
            tune.run_format(0, vec![], None, None, 128);
        }
        hub.run_format(0, vec![parameter("tuning-engine", engine, 0)], None, None, 128);
        Self { hub, decoded, plain, raw: 128 }
    }
    /// One callback of each Tune and then the Hub, and the next callback of
    /// each Tune, which emits what this one played, a delay later.
    fn play(&mut self, decoded: Vec<Input>, plain: Vec<Input>) -> [Vec<(u32, Event)>; 2] {
        let raw = self.raw;
        self.decoded.run_format(raw, decoded, None, None, 128);
        self.plain.run_format(raw, plain, None, None, 128);
        self.hub.run_format(raw, vec![], None, None, 128);
        let out = [&self.decoded, &self.plain]
            .map(|tune| tune.run_format(raw + 128, vec![], None, None, 128).values);
        self.hub.run_format(raw + 128, vec![], None, None, 128);
        self.raw += 256;
        out
    }
    fn voice(
        &self,
        decoded: bool,
        channel: u8,
        key: u8,
    ) -> harmonigraph_core::canonical::VoiceBaseline {
        let tune = if decoded { &self.decoded } else { &self.plain };
        let slot = tune.shared().slot.load(Ordering::Acquire) as u8;
        inspect_hub(&self.hub, |hub| hub.test_voice(slot, channel, key)).unwrap()
    }
}

/// The last tuning expression `output` carries for the note `id`, in
/// semitones: none for a note nothing was sent for.
fn tuning_for(output: &[(u32, Event)], id: i32) -> Option<f64> {
    output.iter().rev().find_map(|(_, event)| match event {
        Event::Expression { kind: 2, id: of, value, .. } if *of == id => Some(*value),
        _ => None,
    })
}

/// The microcents a decoded `(channel, key)` lies from 12-TET: its chain of
/// fifths at `fifth`, from the lattice's C at `c_offset`.
fn stage(channel: u8, key: u8, c_offset: i32, fifth: i32) -> i64 {
    let fifths = harmonigraph_core::tuning::fifths_for_midi(channel, key);
    i64::from(c_offset) + i64::from(fifths) * (i64::from(fifth) - 700_000_000)
}

/// Pass through tunes a decoded Tune's chain at the decoded fifth, from the
/// lattice's C, and leaves a Tune with Retune off alone; the player's own
/// per-note tuning still sounds on top.
#[test]
fn pass_through_tunes_a_decoded_tunes_chain_at_the_decoded_fifth() {
    let _scope = crate::test_scope::enter();
    let mut tuning = Tuning::just();
    tuning.c_offset = 10_000_000;
    let fifth = 695_000_000;
    let mut session = Decoding::new(0.0, tuning, fifth);
    // C♯ on the bank above the central one, D♭ on the central one: one key.
    let [decoded, plain] = session.play(
        vec![note(1, 1, 61, 0, true), expression(1, 0.125, 0), note(2, 0, 61, 0, true)],
        vec![note(3, 1, 61, 0, true), note(4, 0, 61, 0, true)],
    );
    for (id, channel) in [(1, 1), (2, 0)] {
        let stage = stage(channel, 61, tuning.c_offset, fifth);
        let voice = session.voice(true, channel, 61);
        assert_eq!(voice.frozen_offset_microcents, stage, "decoded channel {channel}");
        assert_ne!(voice.decision, 0, "the take records a decoded correction");
        let player = if id == 1 { 0.125 } else { 0.0 };
        let sent = tuning_for(&decoded, id).expect("a decoded note is retuned");
        assert!((sent - (player + stage as f64 / 1e8)).abs() < 1e-9, "id {id}: {sent}");
    }
    for (id, channel) in [(3, 1), (4, 0)] {
        let voice = session.voice(false, channel, 61);
        assert_eq!((voice.frozen_offset_microcents, voice.decision), (0, 0));
        assert!(tuning_for(&plain, id).is_none_or(|sent| sent == 0.0), "plain id {id}");
    }
    // The Hub's own input decodes too, once its Retune is on.
    let hub = session.hub.shared();
    hub.set_retune(true);
    session.hub.run_format(session.raw, vec![note(5, 1, 61, 0, true)], None, None, 128);
    let direct = inspect_hub(&session.hub, |hub| hub.test_voice(DIRECT, 1, 61)).unwrap();
    assert_eq!(direct.frozen_offset_microcents, stage(1, 61, tuning.c_offset, fifth));
}

/// Lattice Map reads a decoded Tune's key alone: C♯ and D♭ share key 61's
/// slot, and a spelling whose chain lies most of a semitone from 12-TET still
/// takes its own key's slot rather than its neighbour's.
#[test]
fn lattice_map_tunes_a_decoded_tune_from_its_keys_slot() {
    let _scope = crate::test_scope::enter();
    let mut tuning = Tuning::just();
    tuning.c_offset = 5_000_000;
    let fifth = 696_578_428;
    let mut session = Decoding::new(2.0, tuning, fifth);
    // C on the bank two above the central one is 24 fifths up: at this fifth,
    // nearer B than C.
    assert!(stage(2, 60, tuning.c_offset, fifth) < -50_000_000);
    session.play(
        vec![note(1, 1, 61, 0, true), note(2, 0, 61, 0, true), note(3, 2, 60, 0, true)],
        vec![],
    );
    let map = LatticeMap::default();
    for (channel, key) in [(1, 61), (0, 61), (2, 60)] {
        let voice = session.voice(true, channel, key);
        assert_eq!(voice.attack_node, Some(map.node(key.into())), "channel {channel} key {key}");
        assert_eq!(voice.frozen_offset_microcents, map.correction(key.into(), tuning));
    }
}

/// The pitch `node` sounds at in `tuning`, in the octave of MIDI C `c`.
fn node_pitch(tuning: Tuning, node: LatticePos, c: i64) -> i64 {
    c * 100_000_000
        + i64::from(tuning.c_offset)
        + i64::from(node.threes) * i64::from(tuning.three)
        + i64::from(node.fives) * i64::from(tuning.five)
}

/// Adaptive tunes a decoded Tune's spelling to its one copy in the strip the
/// Position fifths lane places: C♯ and D♭ are 25/24 and 16/15, and a lane one
/// fifth sharp moves A from 5/3 to 27/16, while the A already held keeps its
/// pitch.
#[test]
fn adaptive_tunes_a_decoded_tune_in_the_strip_the_position_fifths_place() {
    let _scope = crate::test_scope::enter();
    let mut tuning = Tuning::just();
    tuning.c_offset = 3_000_000;
    let mut session = Decoding::fixed(tuning, 696_578_428);
    let sharp = LatticePos::new(-1, 2, 0);
    let [out, _] = session.play(vec![note(1, 1, 61, 0, true)], vec![]);
    let voice = session.voice(true, 1, 61);
    assert_eq!(voice.attack_node, Some(sharp));
    let correction = node_pitch(tuning, sharp, 60) - 6_100_000_000;
    assert_eq!(voice.frozen_offset_microcents, correction);
    assert!((tuning_for(&out, 1).unwrap() - correction as f64 / 1e8).abs() < 1e-9);
    session.play(vec![note(2, 0, 61, 0, true)], vec![]);
    let flat = LatticePos::new(-1, -1, 0);
    let voice = session.voice(true, 0, 61);
    assert_eq!(voice.attack_node, Some(flat));
    assert_eq!(voice.frozen_offset_microcents, node_pitch(tuning, flat, 72) - 6_100_000_000);
    let five_three = LatticePos::new(-1, 1, 0);
    session.play(vec![note(3, 0, 69, 0, true)], vec![]);
    let held = session.voice(true, 0, 69);
    assert_eq!(held.attack_node, Some(five_three));
    // A lane's host value is its step plus `OFFSET_LIMIT`: one fifth sharp.
    let raw = session.raw;
    session.hub.run_format(raw, vec![parameter("map-fifths", 10.0, 0)], None, None, 128);
    session.raw += 128;
    session.play(vec![note(4, 0, 81, 0, true)], vec![]);
    let pythagorean = LatticePos::new(3, 0, 0);
    let voice = session.voice(true, 0, 81);
    assert_eq!(voice.attack_node, Some(pythagorean));
    let correction = node_pitch(tuning, pythagorean, 60) - 8_100_000_000;
    assert_eq!(voice.frozen_offset_microcents, correction);
    let after = session.voice(true, 0, 69);
    assert_eq!(
        (after.frozen_offset_microcents, after.pitch_microcents),
        (held.frozen_offset_microcents, held.pitch_microcents),
        "the held A keeps its pitch"
    );
}

/// I–vi–ii–V–I, three rounds, each chord released before the next: every
/// spelling sounds one pitch in every chord, D included, so ii's D–A is the
/// strip's wolf each time and V's D is the same 9/8. Adaptive's search took
/// ii's D as 10/9 and V's as 9/8.
#[test]
fn the_strip_sounds_each_spelling_at_one_pitch_through_a_progression() {
    let _scope = crate::test_scope::enter();
    let mut session = Decoding::fixed(Tuning::just(), 696_578_428);
    let chords: [[i16; 3]; 5] =
        [[60, 64, 67], [57, 60, 64], [62, 65, 69], [55, 59, 62], [60, 64, 67]];
    let mut sounded = std::collections::BTreeMap::<i16, Vec<i64>>::new();
    for (id, chord) in (0..3).flat_map(|_| chords).enumerate() {
        let id = 3 * id as i32;
        let play = |on| (0..3).map(|i| note(id + i, 0, chord[i as usize], 0, on)).collect();
        session.play(play(true), vec![]);
        for key in chord {
            let pitch = session.voice(true, 0, key as u8).pitch_microcents;
            sounded.entry(key).or_default().push(pitch);
        }
        session.play(play(false), vec![]);
    }
    for (key, pitches) in &sounded {
        assert!(pitches.iter().all(|&pitch| pitch == pitches[0]), "key {key}: {pitches:?}");
    }
    let d = sounded[&62][0] - 6_200_000_000;
    assert_eq!(d, i64::from(2 * Tuning::just().three) - 1_200_000_000 - 200_000_000, "9/8");
}

impl Decoding {
    /// Follow spellings on with `edit` on top, as a fresh policy.
    fn policy(&self, edit: impl FnOnce(&mut harmonigraph_core::configuration::PolicyConfig)) {
        let mut policy = harmonigraph_core::configuration::PolicyConfig {
            decode_fifths: true,
            decoded_fifth: 696_578_428,
            ..Default::default()
        };
        edit(&mut policy);
        settings(&self.hub).submit(ConfigEdit { policy: Some(policy), ..Default::default() });
    }
    /// `events` to both Tunes and the Hub, one callback.
    fn everywhere(&mut self, events: impl Fn() -> Vec<Input>) {
        let raw = self.raw;
        for device in [&self.decoded, &self.plain, &self.hub] {
            device.run_format(raw, events(), None, None, 128);
        }
        self.raw += 128;
    }
    /// The node a lone A struck now and let go sounds at.
    fn a(&mut self) -> Option<LatticePos> {
        self.play(vec![note(1, 0, 69, 0, true)], vec![]);
        let node = self.voice(true, 0, 69).attack_node;
        self.play(vec![note(1, 0, 69, 0, false)], vec![]);
        node
    }
}

/// F♯ in two octaves on one sample, on or off.
fn f_sharps(on: bool) -> Vec<Input> {
    [78, 90].map(|key| note(key.into(), 0, key, 0, on)).into()
}

const FIVE_THREE: LatticePos = LatticePos::new(-1, 1, 0);
const PYTHAGOREAN_A: LatticePos = LatticePos::new(3, 0, 0);

/// An A struck after F♯ moves the strip a fifth sharp, so A is 27/16 and
/// pure under the F♯ just played; every reset of the context
/// returns the strip to the Position, so A is 5/3 again: a change of
/// engine, Reset all voices, a transport Stop, a loop or seek, and silence.
/// With Follow spellings off the strip never moves.
#[test]
fn every_reset_returns_the_followed_strip_to_the_position() {
    let _scope = crate::test_scope::enter();
    type Reset = fn(&mut Decoding);
    // Each arm's Silence reset, in ms: off but in the silence arm, so every
    // other arm returns the strip by its own reset.
    let resets: [(&str, u32, Reset); 5] = [
        ("engine", 0, |session| {
            session.everywhere(|| vec![parameter("tuning-engine", 0.0, 0)]);
            session.everywhere(|| vec![parameter("tuning-engine", 1.0, 0)]);
        }),
        ("Reset all voices", 0, |session| {
            crate::tuning::session::session().request_reset();
            session.everywhere(Vec::new);
        }),
        ("Stop", 0, |session| {
            session.everywhere(|| vec![transport(0, 120.0)]);
            session.everywhere(|| vec![musical_tests::at(0.0, false)]);
        }),
        ("loop", 0, |session| {
            for seconds in [5.0, 5.0, 1.0] {
                session.everywhere(|| vec![musical_tests::at(seconds, false)]);
            }
        }),
        ("silence", 10, |session| (0..8).for_each(|_| session.everywhere(Vec::new))),
    ];
    for (name, silence_ms, reset) in resets {
        let mut session = Decoding::new(1.0, Tuning::just(), 696_578_428);
        session.policy(|policy| {
            (policy.reset_stop, policy.reset_loop, policy.silence_ms) = (true, true, silence_ms);
        });
        for on in [true, false] {
            session.play(f_sharps(on), vec![]);
        }
        // Struck at once after the F♯, before the silence arm's 10 ms.
        assert_eq!(session.a(), Some(PYTHAGOREAN_A), "{name}: moved to C G D A");
        reset(&mut session);
        assert_eq!(session.a(), Some(FIVE_THREE), "{name}: back at the Position");
    }
    let mut session = Decoding::new(1.0, Tuning::just(), 696_578_428);
    session.policy(|policy| policy.follow_spellings = false);
    for on in [true, false] {
        session.play(f_sharps(on), vec![]);
    }
    assert_eq!(session.a(), Some(FIVE_THREE), "the strip stays where the Position put it");
}

/// A loop's last release and its first strike on one sample, with Reset on
/// loop on: the reset comes before the strike's group is read, so the A
/// after it moves the strip against those F♯s.
#[test]
fn a_loop_resets_after_its_last_release_and_before_its_first_strike() {
    let _scope = crate::test_scope::enter();
    let mut session = Decoding::new(1.0, Tuning::just(), 696_578_428);
    session.policy(|policy| policy.reset_loop = true);
    session.play(vec![note(1, 0, 60, 0, true)], vec![]);
    for seconds in [5.0, 5.0, 1.0] {
        session.everywhere(|| vec![musical_tests::at(seconds, false)]);
    }
    let mut wrap = f_sharps(true);
    wrap.insert(0, note(1, 0, 60, 0, false));
    session.play(wrap, vec![]);
    session.play(f_sharps(false), vec![]);
    assert_eq!(session.a(), Some(PYTHAGOREAN_A), "the strike moved the strip");
}

/// D struck over a held F major moves the strip flat, so D sounds 10/9, a
/// pure fifth under the held A.
#[test]
fn a_note_over_held_notes_moves_the_strip_off_their_wolf() {
    let _scope = crate::test_scope::enter();
    let mut session = Decoding::new(1.0, Tuning::just(), 696_578_428);
    session.play([65, 69, 72].map(|key| note(key.into(), 0, key, 0, true)).into(), vec![]);
    session.play(vec![note(62, 0, 62, 0, true)], vec![]);
    let [d, a] = [62, 69].map(|key| session.voice(true, 0, key).attack_node.unwrap());
    assert_eq!((d, a - d), (LatticePos::new(-2, 1, 0), LatticePos::new(1, 0, 0)), "D–A pure");
}

/// The Strip block reads the column the next note sounds in, even after the
/// clamp held the strip at a Position the lanes have since left: following
/// slides the strip to the top column from a lane of +8, a note struck at +9
/// leaves it there, and back at +8 with no note struck the editor names the
/// top column, where the next A sounds.
#[test]
fn the_strip_block_names_where_the_next_note_sounds_after_the_clamp() {
    let _scope = crate::test_scope::enter();
    let mut session = Decoding::new(1.0, Tuning::just(), 696_578_428);
    let lane = |session: &mut Decoding, fifths: f64| {
        // A lane's host value is its step plus `OFFSET_LIMIT`.
        session.everywhere(|| vec![parameter("map-fifths", 9.0 + fifths, 0)]);
    };
    let policy = harmonigraph_core::configuration::PolicyConfig {
        decode_fifths: true,
        decoded_fifth: 696_578_428,
        ..Default::default()
    };
    let shown = |session: &Decoding| {
        hub_wrapper(&session.hub).test_inspect_plugin(|plugin| {
            crate::lattice_maps::view(&plugin.params).strip_flat(policy)
        })
    };
    lane(&mut session, 8.0);
    let lane_column = harmonigraph_core::policy::strip::moved_column(8, 0);
    assert_eq!(shown(&session), Some(lane_column), "the lane's own column before any slide");
    for on in [true, false] {
        session.play(f_sharps(on), vec![]);
    }
    let top = harmonigraph_core::policy::strip::moved_column(9, 0);
    let a_at = |flat| harmonigraph_core::policy::strip::node(3, flat);
    assert_eq!(session.a(), Some(a_at(top)), "slid a fifth sharp, to the top column");
    lane(&mut session, 9.0);
    assert_eq!(session.a(), Some(a_at(top)), "still at the top");
    lane(&mut session, 8.0);
    assert_eq!(shown(&session), Some(top), "the Strip block");
    assert_eq!(session.a(), Some(a_at(top)), "where the next A sounds");
}

/// A note from a Tune with Retune off is no spelling: struck on the decoded
/// D's sample, a plain A leaves the strip to the D alone, so the D sounds
/// where it does struck by itself, not moved off the A's wolf.
#[test]
fn a_retune_off_note_on_a_decoded_notes_sample_does_not_move_the_strip() {
    let _scope = crate::test_scope::enter();
    let d = |plain: Vec<Input>| {
        let mut session = Decoding::new(1.0, Tuning::just(), 696_578_428);
        session.play(vec![note(1, 0, 62, 0, true)], plain);
        session.voice(true, 0, 62).attack_node
    };
    let alone = d(vec![]);
    assert_eq!(d(vec![note(2, 0, 69, 0, true)]), alone);
    assert_eq!(alone, Some(LatticePos::new(2, 0, 0)), "the fixture's D is 9/8");
}
