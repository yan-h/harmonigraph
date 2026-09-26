//! Musical integration through the exported production callbacks.
//!
//! The Hub's schedule is the authority for display, take and future context;
//! these fixtures also check the tuning expressions actually emitted to hosts.
use super::*;
use harmonigraph_core::configuration::{ConfigEdit, PolicyConfig};
use harmonigraph_core::{canonical::VoiceBaseline, LatticePos, Tuning};

pub(super) fn configure(hub: &Device, tuning: Tuning) {
    submit(
        hub,
        ConfigEdit {
            axes: [
                Some(tuning.c_offset),
                Some(tuning.three),
                Some(tuning.five),
                Some(tuning.seven),
                None,
            ],
            tempered: [Some(false); 2],
            auto: [Some(false); 2],
            learning: Some(false),
            policy: None,
        },
    );
}

fn configure_policy(hub: &Device, policy: PolicyConfig) {
    submit(hub, ConfigEdit { policy: Some(policy), ..Default::default() });
}

/// A transport carrying the host's seconds timeline at `seconds`.
fn position(seconds: i64) -> Input {
    let Input::Transport(mut value) = transport(0, 120.0) else { unreachable!() };
    value.flags |= CLAP_TRANSPORT_HAS_SECONDS_TIMELINE;
    value.song_pos_seconds = seconds * (1i64 << 31);
    Input::Transport(value)
}

fn submit(hub: &Device, edit: ConfigEdit) {
    hub_wrapper(hub)
        .configuration_handle()
        .unwrap()
        .submit(crate::configuration::packet(edit))
        .unwrap();
}

/// Three Tunes and one Hub, at 44.1 kHz and 512-frame callbacks, so D is 512
/// samples at the default 1x. Every Tune runs before the Hub in a step, which
/// is the order that lets a 1x note make its own deadline.
struct Phrase {
    hub: Device,
    sources: [Device; 3],
    raw: i64,
}

impl Phrase {
    fn new() -> Self {
        let mut hub = Device::new(false);
        hub.activate_format(44100.0, 512);
        configure(&hub, Tuning::just());
        let sources = std::array::from_fn(|_| {
            let mut source = Device::new(true);
            source.activate_format(44100.0, 512);
            source
        });
        let mut phrase = Self { hub, sources, raw: 0 };
        // Neutral CC64/66/69 first, which is harmless rather than needed:
        // nothing reads pedal state before emitting a note — `Tune::pedals`
        // is read only by the cut — so an unseen pedal counts as up and these
        // only pin where the fixture starts.
        phrase.step(
            std::array::from_fn(|_| {
                (0..2)
                    .flat_map(|channel| {
                        [64, 66, 69].map(move |cc| raw_midi([0xb0 | channel, cc, 0], 0))
                    })
                    .collect()
            }),
            [0, 1, 2],
        );
        phrase.idle();
        phrase.idle();
        phrase
    }

    fn step(&mut self, mut input: [Vec<Input>; 3], order: [usize; 3]) -> [Vec<(u32, Event)>; 3] {
        let mut output = std::array::from_fn(|_| Vec::new());
        for index in order {
            let sink = self.sources[index].run_format(
                self.raw,
                std::mem::take(&mut input[index]),
                None,
                None,
                512,
            );
            output[index] = sink.values;
        }
        self.hub.run_format(self.raw, vec![], None, None, 512);
        self.raw += 512;
        output
    }
    fn idle(&mut self) -> [Vec<(u32, Event)>; 3] {
        self.step(std::array::from_fn(|_| vec![]), [0, 1, 2])
    }
    /// The host's seconds timeline, to every device in the graph. A DAW gives
    /// the transport to each plugin it calls, and the Hub is the one that
    /// reads it: a seek is a musical boundary, not a per-track one.
    fn seek(&mut self, seconds: i64) {
        self.transport_all(|| position(seconds));
    }
    /// Play or stop, to every device in the graph. The same reason as `seek`,
    /// and the Hub matters more here than anywhere: it is the one row that
    /// turns a transport Stop into the session's cut, so a fixture that gave
    /// the transport only to the sources would be testing a graph no host
    /// produces.
    fn transport_edge(&mut self, playing: bool) {
        self.transport_all(|| {
            let Input::Transport(mut value) = transport(0, 120.0) else { unreachable!() };
            if !playing {
                value.flags &= !CLAP_TRANSPORT_IS_PLAYING;
            }
            Input::Transport(value)
        });
    }
    fn transport_all(&mut self, event: impl Fn() -> Input) {
        for index in 0..3 {
            self.sources[index].run_format(self.raw, vec![event()], None, None, 512);
        }
        self.hub.run_format(self.raw, vec![event()], None, None, 512);
        self.raw += 512;
    }
    /// What the Hub scheduled for that voice. It is the only authority there
    /// is: display, take and the next decision all read this one table.
    fn voice(&self, source: usize, key: u8, channel: u8) -> VoiceBaseline {
        inspect_hub(&self.hub, |hub| hub.test_voice(source as u8, channel, key))
            .unwrap_or_else(|| panic!("source {source} holds channel {channel} key {key}"))
    }
    fn held(&self, source: usize) -> usize {
        inspect_hub(&self.hub, |hub| hub.test_held(source as u8))
    }
    fn misses(&self) -> u64 {
        self.sources.iter().map(|source| source.shared().misses.load(Ordering::Relaxed)).sum()
    }
    fn release_all(&mut self) {
        let input = std::array::from_fn(|index| {
            inspect_hub(&self.hub, |hub| {
                (0..128)
                    .filter_map(|key| hub.test_voice(index as u8, 0, key))
                    .map(|voice| {
                        note(
                            voice.host_note_id,
                            i16::from(voice.channel),
                            i16::from(voice.note),
                            0,
                            false,
                        )
                    })
                    .collect()
            })
        });
        self.step(input, [0, 1, 2]);
        for _ in 0..4 {
            self.idle();
        }
        assert_eq!((0..3).map(|i| self.held(i)).sum::<usize>(), 0);
    }
}

#[test]
fn production_musical_dfa_permutations_include_each_predecessor() {
    let _scope = crate::test_scope::enter();
    let positions = [LatticePos::new(2, 0, 0), LatticePos::new(3, -1, 0), LatticePos::new(3, 0, 0)];
    let keys = [50, 53, 57];
    for order in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
        let mut phrase = Phrase::new();
        phrase.step(std::array::from_fn(|i| vec![note(i as i32, 0, keys[i], 0, true)]), order);
        let output = phrase.step(std::array::from_fn(|_| vec![]), order.map(|i| 2 - i));
        let mut pitches = [0; 3];
        let mut decisions = [0; 3];
        for i in 0..3 {
            // The note and the tuning expression that states its correction.
            assert_eq!(output[i].len(), 2, "{output:?}");
            assert_eq!(output[i][0].0, 0);
            let voice = phrase.voice(i, keys[i] as u8, 0);
            assert_eq!(voice.attack_node, Some(positions[i]));
            assert_eq!(voice.assignment.unwrap().policy, harmonigraph_core::policy::CONFIG);
            pitches[i] = voice.pitch_microcents;
            decisions[i] = voice.decision;
        }
        assert!(decisions[0] < decisions[1] && decisions[1] < decisions[2]);
        assert_eq!(pitches[1] - pitches[0], i64::from(Tuning::just().three - Tuning::just().five));
        assert_eq!(pitches[2] - pitches[0], i64::from(Tuning::just().three));
        assert_eq!(phrase.misses(), 0, "every 1x note made its own deadline");
        phrase.release_all();
    }
}

#[test]
fn production_musical_ii_v_i_keeps_each_common_tone_through_the_change() {
    let _scope = crate::test_scope::enter();
    use harmonigraph_core::tuning::PitchClass;
    let tuning = Tuning::just();
    let mut phrase = Phrase::new();
    let expect = |phrase: &Phrase, source: usize, key: u8, node: LatticePos| {
        let voice = phrase.voice(source, key, 0);
        assert_eq!(voice.attack_node, Some(node), "source {source} key {key}");
        assert_eq!(PitchClass::from_microcents(voice.pitch_microcents), tuning.pitch_class(node));
        voice
    };
    // ii, one voice per Tune at one sample.
    phrase.step(
        [
            vec![note(10, 0, 50, 0, true)],
            vec![note(11, 0, 53, 0, true)],
            vec![note(12, 0, 57, 0, true)],
        ],
        [0, 1, 2],
    );
    phrase.idle();
    let d = expect(&phrase, 0, 50, LatticePos::new(2, 0, 0));
    expect(&phrase, 1, 53, LatticePos::new(3, -1, 0));
    expect(&phrase, 2, 57, LatticePos::new(3, 0, 0));
    // V. D is the common tone: the two voices it replaces release and their
    // successors start in the same callback, so every release reaches the
    // Hub's context before an onset is scored against what is left. The Tune
    // carrying the LOWER new key runs second, so key order rather than
    // callback order has to decide the assignment chain.
    phrase.step(
        [
            vec![],
            vec![note(11, 0, 53, 0, false), note(21, 0, 59, 0, true)],
            vec![note(12, 0, 57, 0, false), note(22, 0, 55, 0, true)],
        ],
        [0, 1, 2],
    );
    phrase.idle();
    let g = expect(&phrase, 2, 55, LatticePos::new(1, 0, 0));
    let b = expect(&phrase, 1, 59, LatticePos::new(1, 1, 0));
    assert!(g.decision < b.decision, "G is assigned before B, and B sees it");
    assert_eq!(phrase.voice(0, 50, 0), d, "the common tone is neither restarted nor retuned");
    // I. G is now the common tone, and again the lower new key runs later.
    phrase.step(
        [
            vec![note(10, 0, 50, 0, false), note(30, 0, 52, 0, true)],
            vec![note(21, 0, 59, 0, false), note(31, 0, 48, 0, true)],
            vec![],
        ],
        [0, 1, 2],
    );
    phrase.idle();
    let c = expect(&phrase, 1, 48, LatticePos::ORIGIN);
    let e = expect(&phrase, 0, 52, LatticePos::new(0, 1, 0));
    assert!(c.decision < e.decision, "C is assigned before E, and E sees it");
    assert_eq!(phrase.voice(2, 55, 0), g, "the common tone is neither restarted nor retuned");
    phrase.release_all();
}

/// A held note's weight falls with how long before the newest held note it
/// was struck. A just low E held three seconds before a chain of fifths then
/// no longer outvotes the chain, so high E continues it; with every held note
/// weighed alike the low E still decides, as the `held-e` fixture does.
#[test]
fn production_a_held_note_struck_long_before_the_rest_weighs_less() {
    let _scope = crate::test_scope::enter();
    for (half_life_ms, high) in [(0, LatticePos::new(0, 1, 0)), (1000, LatticePos::new(4, 0, 0))] {
        let mut phrase = Phrase::new();
        configure_policy(&phrase.hub, PolicyConfig { half_life_ms, ..Default::default() });
        phrase.idle();
        phrase.step([vec![note(1, 0, 52, 0, true)], vec![], vec![]], [0, 1, 2]);
        // Three seconds of 512-sample callbacks at 44.1 kHz.
        for _ in 0..259 {
            phrase.idle();
        }
        for (id, key) in [(2, 48), (3, 55), (4, 62), (5, 69)] {
            phrase.step([vec![], vec![note(id, 0, key, 0, true)], vec![]], [0, 1, 2]);
        }
        phrase.step([vec![], vec![], vec![note(6, 0, 76, 0, true)]], [0, 1, 2]);
        phrase.idle();
        assert_eq!(phrase.voice(0, 52, 0).attack_node, Some(LatticePos::new(0, 1, 0)));
        assert_eq!(phrase.voice(1, 69, 0).attack_node, Some(LatticePos::new(3, 0, 0)));
        assert_eq!(phrase.voice(2, 76, 0).attack_node, Some(high), "{half_life_ms} ms");
        phrase.release_all();
    }
}

/// Released memory decays on the same clock, from each attack. With nothing
/// held, a just low E struck three seconds before a chain of fifths no longer
/// outvotes the chain once all are released; weighed alike, it does.
#[test]
fn production_a_note_released_long_before_the_rest_weighs_less() {
    let _scope = crate::test_scope::enter();
    for (half_life_ms, high) in [(0, LatticePos::new(0, 1, 0)), (1000, LatticePos::new(4, 0, 0))] {
        let mut phrase = Phrase::new();
        configure_policy(&phrase.hub, PolicyConfig { half_life_ms, ..Default::default() });
        phrase.idle();
        phrase.step([vec![note(1, 0, 52, 0, true)], vec![], vec![]], [0, 1, 2]);
        phrase.step([vec![note(1, 0, 52, 0, false)], vec![], vec![]], [0, 1, 2]);
        for _ in 0..259 {
            phrase.idle();
        }
        for (id, key) in [(2, 48), (3, 55), (4, 62), (5, 69)] {
            phrase.step([vec![], vec![note(id, 0, key, 0, true)], vec![]], [0, 1, 2]);
            phrase.step([vec![], vec![note(id, 0, key, 0, false)], vec![]], [0, 1, 2]);
        }
        phrase.step([vec![], vec![], vec![note(6, 0, 76, 0, true)]], [0, 1, 2]);
        phrase.idle();
        assert_eq!(phrase.voice(2, 76, 0).attack_node, Some(high), "{half_life_ms} ms");
        phrase.release_all();
    }
}

/// A released note never outvotes a held one. C, struck a chord before the F
/// and A still sounding over it, is let go as D arrives; with F–A held it is
/// not in the context, so D is tuned to the sounding F–A, a just fifth under
/// A. When a release restarted the age and released notes shared the context,
/// C came back at the released weight against an F–A eight half-lives old, and
/// D took 9/8 from it: 680 cents under the sounding A.
#[test]
fn production_a_released_note_never_outvotes_a_held_one() {
    let _scope = crate::test_scope::enter();
    let mut phrase = Phrase::new();
    // Eight seconds of 512-sample callbacks at 44.1 kHz.
    let hold = |phrase: &mut Phrase| {
        for _ in 0..689 {
            phrase.idle();
        }
    };
    phrase.step(
        [
            vec![note(1, 0, 48, 0, true)],
            vec![note(2, 0, 52, 0, true)],
            vec![note(3, 0, 55, 0, true)],
        ],
        [0, 1, 2],
    );
    hold(&mut phrase);
    // F major under the held C.
    phrase.step(
        [
            vec![],
            vec![note(2, 0, 52, 0, false), note(4, 0, 53, 0, true)],
            vec![note(3, 0, 55, 0, false), note(5, 0, 57, 0, true)],
        ],
        [0, 1, 2],
    );
    hold(&mut phrase);
    phrase
        .step([vec![note(1, 0, 48, 0, false), note(6, 0, 50, 0, true)], vec![], vec![]], [0, 1, 2]);
    phrase.idle();
    assert_eq!(phrase.voice(1, 53, 0).attack_node, Some(LatticePos::new(-1, 0, 0)));
    assert_eq!(phrase.voice(2, 57, 0).attack_node, Some(LatticePos::new(-1, 1, 0)));
    assert_eq!(phrase.voice(0, 50, 0).attack_node, Some(LatticePos::new(-2, 1, 0)));
    phrase.release_all();
}

/// The frozen correction is composed with the player's own expression rather
/// than replacing it: the emitted tuning value moves with a later per-note
/// bend while the harmonic reference the next decision reads does not.
#[test]
fn production_tune_preserves_player_pitch_and_freezes_only_the_adaptive_correction() {
    let _scope = crate::test_scope::enter();
    let (recorder, mut capture) = harmonigraph_record::testing::channel();
    crate::configuration::inject_recorder(recorder);
    let mut phrase = Phrase::new();
    capture.arm();
    phrase.step(
        [
            vec![note(7, 0, 57, 0, true), expression(7, 0.25, 0), expression(7, 0.5, 0)],
            vec![],
            vec![],
        ],
        [0, 1, 2],
    );
    let output = phrase.idle();
    let onset = phrase.voice(0, 57, 0);
    let correction = onset.frozen_offset_microcents;
    let input_pitch = 5_750_000_000;
    let expected = harmonigraph_core::policy::assign_new_note(
        onset.assignment.unwrap().into(),
        &[],
        &Default::default(),
        harmonigraph_core::policy::OrderedOnset { pitch: input_pitch },
        &mut Default::default(),
    )
    .unwrap()
    .assignment;
    assert_eq!(correction, expected.correction_microcents());
    assert_eq!(onset.attack_node, expected.node());
    assert_eq!(onset.player_tuning, 0.5);
    assert_eq!(onset.onset_pitch_microcents, input_pitch + correction);
    assert_eq!(onset.pitch_microcents, onset.onset_pitch_microcents);
    assert_eq!(phrase.sources[0].shared().last_input.load(Ordering::Relaxed), input_pitch);
    assert_eq!(
        phrase.sources[0].shared().last_output.load(Ordering::Relaxed),
        onset.pitch_microcents
    );
    for records in [capture.display_events(), capture.drain_canonical()] {
        let published = records
            .into_iter()
            .find_map(|record| match record {
                harmonigraph_take::CanonicalRecord::Delta(delta)
                    if matches!(delta.event.kind, harmonigraph_take::NoteKind::On { .. }) =>
                {
                    Some(delta)
                }
                _ => None,
            })
            .expect("both display and take receive the assigned onset");
        assert_eq!(published.pitch_microcents, Some(onset.pitch_microcents));
        assert_eq!(published.assignment, VoiceBaseline::metadata(&onset).map(Into::into));
    }
    assert_ne!(correction, 0, "an adaptive choice was made for this attack");
    let emitted = output[0]
        .iter()
        .find_map(|(_, event)| match event {
            Event::Expression { kind: 2, value, .. } => Some(*value),
            _ => None,
        })
        .expect("the onset states its tuning");
    assert!((emitted - (0.5 + correction as f64 / 100_000_000.0)).abs() < 1e-9);
    // A later expression moves the emitted pitch and not the frozen choice.
    phrase.step([vec![expression(7, 0.75, 0)], vec![], vec![]], [0, 1, 2]);
    let output = phrase.idle();
    let moved = output[0]
        .iter()
        .find_map(|(_, event)| match event {
            Event::Expression { kind: 2, value, .. } => Some(*value),
            _ => None,
        })
        .expect("the later expression is forwarded with the correction added");
    assert!((moved - (0.75 + correction as f64 / 100_000_000.0)).abs() < 1e-9);
    let after = phrase.voice(0, 57, 0);
    assert_eq!(after.frozen_offset_microcents, correction);
    assert_eq!(after.onset_pitch_microcents, onset.onset_pitch_microcents);
    assert_ne!(after.pitch_microcents, onset.pitch_microcents, "what sounds did move");
    phrase.release_all();
}

/// Identical note ids in different rows still belong to independent sources.
#[test]
fn initial_expressions_stay_with_their_source_before_musical_sorting() {
    let _scope = crate::test_scope::enter();
    let mut phrase = Phrase::new();
    let players = [0.125, 0.25, 0.5];
    phrase.step(
        std::array::from_fn(|i| {
            vec![note(7, 0, 57, 0, true), expression(7, -0.25, 0), expression(7, players[i], 0)]
        }),
        [2, 0, 1],
    );
    let output = phrase.idle();
    for i in 0..3 {
        let voice = phrase.voice(i, 57, 0);
        let input = 5_700_000_000 + (players[i] * 100_000_000.0).round() as i64;
        assert_eq!(voice.player_tuning, players[i]);
        assert_eq!(voice.onset_pitch_microcents, input + voice.frozen_offset_microcents);
        assert_eq!(voice.pitch_microcents, voice.onset_pitch_microcents);
        assert!(
            (tuning_of(&output[i]).unwrap()
                - (players[i] + voice.frozen_offset_microcents as f64 / 1e8))
                .abs()
                < 1e-9
        );
    }
    assert_eq!(phrase.misses(), 0);
}

#[test]
fn an_unsnapped_onset_emits_existing_drift_without_claiming_a_node_or_the_reference() {
    let _scope = crate::test_scope::enter();
    let mut phrase = Phrase::new();
    phrase.step([vec![note(1, 0, 60, 0, true)], vec![note(2, 0, 64, 0, true)], vec![]], [0, 1, 2]);
    phrase.idle();
    let correction = phrase.voice(1, 64, 0).frozen_offset_microcents;
    assert_ne!(correction, 0, "the just E establishes drift before the missing candidate");
    configure_policy(&phrase.hub, PolicyConfig { radius: 1, axes: 1, ..Default::default() });
    // Around held C and E, one fifth-step reaches F/G and A/B. None earns
    // enough benefit to retune this C-sharp, including its initial bend.
    phrase.step([vec![], vec![], vec![note(3, 0, 61, 0, true), expression(3, 0.05, 0)]], [0, 1, 2]);
    let output = phrase.idle();
    let voice = phrase.voice(2, 61, 0);
    assert_eq!(voice.attack_node, None);
    assert_eq!(voice.frozen_offset_microcents, correction);
    assert_eq!(voice.onset_pitch_microcents, 6_105_000_000 + correction);
    let emitted = output[2]
        .iter()
        .find_map(|(_, event)| match event {
            Event::Expression { kind: 2, value, .. } => Some(*value),
            _ => None,
        })
        .expect("even an unsnapped onset emits its drift and player expression");
    assert!((emitted - (0.05 + correction as f64 / 100_000_000.0)).abs() < 1e-9);
    assert_eq!(inspect_hub(&phrase.hub, |hub| hub.test_next_context().reference), correction);
    phrase.sources[2].shared().set_retune(false);
    phrase.idle();
    assert_eq!(
        inspect_hub(&phrase.hub, |hub| hub.test_next_context().reference),
        correction,
        "the unsnapped source did not take ownership of the reference"
    );
    phrase.release_all();
}

/// Both reset controls act on released memory only. Held voices keep the
/// choice they were given whatever the transport does. Inspect the Hub's
/// retained musical context to check whether the memory survived.
#[test]
fn production_silence_and_stop_reset_settings_clear_only_the_musical_memory() {
    let _scope = crate::test_scope::enter();
    for reset_stop in [false, true] {
        let mut phrase = Phrase::new();
        configure_policy(&phrase.hub, PolicyConfig { reset_stop, ..Default::default() });
        phrase.idle();
        phrase.step(
            [vec![note(1, 0, 48, 0, true), note(2, 0, 52, 1, true)], vec![], vec![]],
            [0, 1, 2],
        );
        for _ in 0..4 {
            phrase.idle();
        }
        assert_eq!(phrase.held(0), 2, "the fixture reaches the reset with voices held");
        phrase.transport_edge(true);
        phrase.transport_edge(false);
        for _ in 0..8 {
            phrase.idle();
        }
        assert_eq!((0..3).map(|i| phrase.held(i)).sum::<usize>(), 0, "Stop ends every voice");
        let remembered = inspect_hub(&phrase.hub, |hub| hub.test_next_context());
        assert_eq!(remembered.context.is_empty(), reset_stop);
        assert_eq!(remembered.reference == 0, reset_stop);
        if !reset_stop {
            configure_policy(&phrase.hub, PolicyConfig { silence_ms: 500, ..Default::default() });
            for _ in 0..64 {
                phrase.idle();
            }
            let expired = inspect_hub(&phrase.hub, |hub| hub.test_next_context());
            assert!(expired.context.is_empty());
            assert_eq!(expired.reference, 0);
        }
    }
}

/// A reduced policy edit must govern expiry at the next Hub begin even when
/// no source sends a note. Observe after expiry, rather than the old outline
/// mailbox's pre-expiry snapshot, so one callback of stale policy fails.
#[test]
fn silence_policy_edits_apply_before_expiry_without_note_input() {
    let _scope = crate::test_scope::enter();
    for enable in [false, true] {
        let mut phrase = Phrase::new();
        configure_policy(
            &phrase.hub,
            PolicyConfig { silence_ms: if enable { 0 } else { 1000 }, ..Default::default() },
        );
        phrase.step(
            [vec![note(1, 0, 48, 0, true), note(2, 0, 52, 1, true)], vec![], vec![]],
            [0, 1, 2],
        );
        phrase.idle();
        phrase.release_all();
        let before = inspect_hub(&phrase.hub, |hub| hub.test_next_context());
        assert!(!before.context.is_empty(), "the fixture must retain released notes");
        assert_ne!(before.reference, 0, "the fixture must retain actual tuning drift");
        configure_policy(
            &phrase.hub,
            PolicyConfig { silence_ms: if enable { 1000 } else { 0 }, ..Default::default() },
        );
        // The wrapper begins performance before reducing queued editor edits.
        // Reduce this edit while still below either timeout, then cross the
        // timeout on the next begin, when the reducer already holds the edit.
        phrase.hub.run_format(phrase.raw, vec![], None, None, 512);
        let (visible, pending) = hub_wrapper(&phrase.hub).configuration_handle().unwrap().visible();
        assert!(!pending, "the editor edit must already be reduced before the expiry callback");
        assert_eq!(
            crate::configuration::view(visible, false).resolved.policy.silence_ms,
            if enable { 1000 } else { 0 }
        );
        phrase.raw += 2 * 44100;
        // Only the Hub runs: no note, controller or source input can incidentally
        // adopt the edited policy in input_boundary before this observation.
        phrase.hub.run_format(phrase.raw, vec![], None, None, 512);
        let after = inspect_hub(&phrase.hub, |hub| hub.test_next_context());
        if enable {
            assert!(after.context.is_empty(), "the newly enabled timeout expires now");
            assert_eq!(after.reference, 0);
        } else {
            assert_eq!(after.context, before.context, "disabling expiry preserves memory now");
            assert_eq!(after.reference, before.reference);
        }
    }
}

/// Striking the note struck last again, with nothing else struck between, is
/// holding it: the Hub's next context is the same whether A was held or struck
/// four more times over the same stretch.
#[test]
fn production_repeating_the_note_struck_last_weighs_as_holding_it() {
    let _scope = crate::test_scope::enter();
    let context = |repeat: bool| {
        let mut phrase = Phrase::new();
        phrase.step(
            [
                vec![note(1, 0, 48, 0, true), note(2, 0, 52, 0, true), note(3, 0, 55, 0, true)],
                vec![],
                vec![],
            ],
            [0, 1, 2],
        );
        for _ in 0..20 {
            phrase.idle();
        }
        phrase.step([vec![], vec![note(4, 0, 69, 0, true)], vec![]], [0, 1, 2]);
        for _ in 0..4 {
            for _ in 0..10 {
                phrase.idle();
            }
            if repeat {
                phrase.step([vec![], vec![note(4, 0, 69, 0, false)], vec![]], [0, 1, 2]);
                phrase.step([vec![], vec![note(4, 0, 69, 0, true)], vec![]], [0, 1, 2]);
            } else {
                phrase.idle();
                phrase.idle();
            }
        }
        phrase.idle();
        let snapshot = inspect_hub(&phrase.hub, |hub| hub.test_next_context());
        phrase.release_all();
        snapshot.context
    };
    assert_eq!(context(true), context(false));
}

/// The journey the old eight-bit coordinate and 2,147-cent correction limits
/// used to stop. Nothing here is about the transport; it is the one musical
/// case that proves the representation is still wide enough.
#[test]
fn production_moving_major_thirds_cross_old_coordinate_and_correction_limits() {
    let _scope = crate::test_scope::enter();
    let mut phrase = Phrase::new();
    // A chord every ten callbacks, 116 ms. Released memory fades by time
    // alone, so a journey keeps moving only when its chords are at least
    // about a half-life apart; closer, the chords before pull each root back.
    configure_policy(&phrase.hub, PolicyConfig { half_life_ms: 50, ..Default::default() });
    phrase.idle();
    for chord in 0..304 {
        if chord != 0 {
            phrase.release_all();
        }
        let root = 48 + (chord % 3) * 4;
        phrase.step(
            [
                vec![
                    note(1, 0, root, 0, true),
                    note(2, 0, root + 4, 1, true),
                    note(3, 0, root + 7, 2, true),
                ],
                vec![],
                vec![],
            ],
            [0, 1, 2],
        );
        for _ in 0..4 {
            phrase.idle();
        }
        let voice = phrase.voice(0, root as u8, 0);
        assert_eq!(
            voice.attack_node,
            Some(LatticePos::new(0, i32::from(chord), 0)),
            "chord {chord}"
        );
        assert_eq!(voice.onset_pitch_microcents, voice.pitch_microcents);
        if chord == 303 {
            assert!(voice.frozen_offset_microcents < -4_000_000_000);
        }
    }
    phrase.release_all();
}

/// A loop or seek clears released memory once, before the next attack, and
/// only when the control says so. This is the whole of what the transport can
/// do to the policy that a Stop cannot.
#[test]
fn production_a_loop_clears_released_memory_once_before_the_next_attack() {
    let _scope = crate::test_scope::enter();
    for reset_loop in [false, true] {
        let mut phrase = Phrase::new();
        let policy = PolicyConfig { reset_loop, half_life_ms: 50, ..Default::default() };
        configure_policy(&phrase.hub, policy);
        phrase.idle();
        // Three chords of moving thirds, so the reference has travelled and a
        // cleared memory is distinguishable from a kept one. A chord every
        // eight callbacks, 93 ms, so a half-life under that keeps them moving.
        let mut node = None;
        for root in [48, 52, 56] {
            phrase.step(
                [
                    vec![
                        note(1, 0, root, 0, true),
                        note(2, 0, root + 4, 1, true),
                        note(3, 0, root + 7, 2, true),
                    ],
                    vec![],
                    vec![],
                ],
                [0, 1, 2],
            );
            for _ in 0..2 {
                phrase.idle();
            }
            node = phrase.voice(0, root as u8, 0).attack_node;
            phrase.release_all();
        }
        assert_ne!(node, Some(LatticePos::ORIGIN), "the fixture travelled before the loop");
        // The host's seconds timeline jumps backwards while sample time keeps
        // running forward. That discontinuity is the loop.
        phrase.seek(10);
        phrase.seek(1);
        phrase.step(
            [
                vec![note(1, 0, 48, 0, true), note(2, 0, 52, 1, true), note(3, 0, 55, 2, true)],
                vec![],
                vec![],
            ],
            [0, 1, 2],
        );
        for _ in 0..2 {
            phrase.idle();
        }
        let after = phrase.voice(0, 48, 0).attack_node;
        if reset_loop {
            assert_eq!(after, Some(LatticePos::ORIGIN), "the loop cleared the travelled memory");
        } else {
            assert_ne!(after, Some(LatticePos::ORIGIN), "without the control it keeps travelling");
        }
        phrase.release_all();
    }
}

/// The two publication lanes are independent, and a lane that loses a report
/// owes EVERY source a snapshot rather than only the one whose report it lost.
/// The fixture stops draining the take while the display keeps up, so only the
/// take ring overflows.
#[test]
fn a_take_lane_gap_owes_every_source_its_own_snapshot() {
    use harmonigraph_take::CanonicalRecord;
    let _scope = crate::test_scope::enter();
    let (recorder, mut capture) = harmonigraph_record::testing::channel();
    crate::configuration::inject_recorder(recorder);
    let mut phrase = Phrase::new();
    capture.arm();
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-take-gap-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("gap.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    phrase.step(
        [
            vec![note(1, 0, 50, 0, true)],
            vec![note(2, 0, 57, 0, true)],
            vec![note(3, 0, 62, 0, true)],
        ],
        [0, 1, 2],
    );
    for _ in 0..8 {
        phrase.idle();
        writer.drain(&mut capture);
        capture.display_events();
    }
    // The editor keeps up; the writer has stopped. Only the take ring fills,
    // and the report it loses is one source's expression.
    for _ in 0..80 {
        phrase.step(
            [(0..64).map(|t| expression(1, 0.1234567890123, t)).collect(), vec![], vec![]],
            [0, 1, 2],
        );
        capture.display_events();
    }
    for _ in 0..8 {
        phrase.idle();
        writer.drain(&mut capture);
        capture.display_events();
    }
    let take = harmonigraph_take::Take::read(&path).unwrap();
    let gap = take
        .events
        .iter()
        .position(|record| matches!(record, CanonicalRecord::Gap(_)))
        .expect("the fixture must actually overflow the take lane");
    let refreshed: std::collections::BTreeSet<u64> = take.events[gap..]
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Baseline(frame) => frame.baseline().ok(),
            _ => None,
        })
        .map(|frame| frame.source.0)
        .collect();
    for source in 1..=3u64 {
        assert!(
            refreshed.contains(&source),
            "source {source} owes the take a snapshot after the gap: got {refreshed:?}"
        );
    }
    phrase.release_all();
    drop(writer);
    drop(phrase);
    std::fs::remove_dir_all(&directory).unwrap();
}

/// #1129: a take that starts between a note's input and the sample it is
/// scheduled to sound at. The onset is sequenced 400 samples into a callback
/// no take owns, so D = 512 schedules it 400 samples into the first callback
/// the take records — its delta routes to no pass, and only a snapshot of what
/// is already sounding can tell the take it exists.
#[test]
fn a_take_armed_between_an_onset_and_its_sound_opens_with_that_voice() {
    use harmonigraph_take::CanonicalRecord;
    let _scope = crate::test_scope::enter();
    let (recorder, mut capture) = harmonigraph_record::testing::channel();
    crate::configuration::inject_recorder(recorder);
    let mut phrase = Phrase::new();
    phrase.step([vec![note(1, 0, 60, 400, true)], vec![], vec![]], [0, 1, 2]);
    capture.arm();
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-take-onset-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("onset.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    // Expression every callback, as a played note has: each is sequenced in
    // the callback a snapshot may be cut in, and scheduled D after it.
    for step in 0..4 {
        phrase.step([vec![expression(1, 0.01 * f64::from(step), 0)], vec![], vec![]], [0, 1, 2]);
        writer.drain(&mut capture);
    }
    phrase.step([vec![note(1, 0, 60, 100, false)], vec![], vec![]], [0, 1, 2]);
    for _ in 0..4 {
        phrase.idle();
        writer.drain(&mut capture);
    }
    // Read, which is also the check that every snapshot follows the history
    // it cuts: a take that sorts one before it is refused whole.
    let take = harmonigraph_take::Take::read(&path).unwrap();
    let origin = take.configurations.first().expect("the take recorded a block").t;
    let mut tracker = harmonigraph_core::NoteTracker::new();
    for record in &take.events {
        record.apply(&mut tracker).unwrap();
    }
    let drawn: Vec<_> = tracker
        .roll()
        .notes()
        .filter(|n| n.source == harmonigraph_core::SourceId(1) && n.note == 60)
        .collect();
    assert_eq!(drawn.len(), 1, "the take draws the held note: {:?}", take.events);
    assert!(
        (drawn[0].start - (origin + 400.0 / 44100.0)).abs() < 1e-9,
        "at the sample it was scheduled to sound: {} vs origin {origin}",
        drawn[0].start
    );
    assert!(drawn[0].end.is_some(), "and its release, which the take did record");
    assert!(
        !take.events.iter().any(|record| matches!(
            record.note(),
            Some(n) if n.note == 60 && matches!(n.kind, harmonigraph_take::NoteKind::On { .. })
        )),
        "the fixture must put the onset itself in no pass, or a snapshot is not what drew it"
    );
    assert!(take.events.iter().any(|record| matches!(
        record,
        CanonicalRecord::Baseline(frame) if frame.voices.iter().any(|voice| voice.note == 60)
    )));
    drop(writer);
    drop(phrase);
    std::fs::remove_dir_all(&directory).unwrap();
}

/// Custom axis sizes go through the same score, and the player's own attack
/// expression is preserved beside the correction rather than replaced by it.
#[test]
fn production_musical_nonjust_axes_select_locally_and_preserve_attack_expression() {
    let _scope = crate::test_scope::enter();
    let mut phrase = Phrase::new();
    configure(
        &phrase.hub,
        Tuning { three: 720_000_000, five: 360_000_000, seven: 960_000_000, ..Tuning::just() },
    );
    phrase.idle();
    phrase
        .step([vec![note(1, 0, 63, 0, true), expression(1, 0.375, 0)], vec![], vec![]], [0, 1, 2]);
    let output = phrase.idle();
    let voice = phrase.voice(0, 63, 0);
    assert!(voice.attack_node.is_some());
    assert_ne!(voice.decision, 0);
    assert_eq!(voice.player_tuning, 0.375);
    assert_eq!(voice.pitch_microcents, 6_337_500_000 + voice.frozen_offset_microcents);
    assert!(output[0].iter().any(|(_, event)| matches!(event, Event::Expression { .. })));
    phrase.release_all();
}

/// The other half of the lane contract. A lane that refuses a snapshot must
/// not hold back the lane that would take it, and must not spend the identity
/// it refused: a retry under an id the other lane has already seen is dropped
/// as a duplicate, silently. The fixture stops draining the display and keeps
/// draining the take, so the refusal is on the display and the take is whole.
#[test]
fn a_snapshot_one_lane_refused_does_not_freeze_the_other_lanes_identity() {
    use harmonigraph_take::CanonicalRecord;
    let _scope = crate::test_scope::enter();
    let (recorder, mut capture) = harmonigraph_record::testing::channel();
    crate::configuration::inject_recorder(recorder);
    let mut phrase = Phrase::new();
    capture.arm();
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-partial-frame-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("partial.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    // Each Reset is a cut, and a cut owes every row a fresh snapshot on both
    // lanes. Well past SNAPSHOT_SLOTS, so the display is refusing long before
    // the end while the writer keeps up.
    const RESETS: usize = 30;
    for _ in 0..RESETS {
        phrase.hub.shared().request_reset();
        phrase.hub.main();
        for _ in 0..4 {
            phrase.idle();
            writer.drain(&mut capture);
        }
    }
    writer.drain(&mut capture);
    let of_source = |records: &[CanonicalRecord], source: u64| -> Vec<u64> {
        records
            .iter()
            .filter_map(|record| match record {
                CanonicalRecord::Baseline(frame) => frame.baseline().ok(),
                _ => None,
            })
            .filter(|frame| frame.source.0 == source)
            .map(|frame| frame.id)
            .collect()
    };
    let take = harmonigraph_take::Take::read(&path).unwrap();
    let written = of_source(&take.events, 1);
    let displayed = of_source(&capture.display_events(), 1);
    assert!(
        displayed.len() < RESETS,
        "the fixture must actually reach a refusal on the display: {} of {RESETS}",
        displayed.len()
    );
    assert!(
        written.len() >= RESETS,
        "every refresh the row owed reached the take: {} of {RESETS}",
        written.len()
    );
    assert!(
        written.windows(2).all(|pair| pair[1] > pair[0]),
        "each one under a fresh identity, so none is deduplicated away"
    );
    phrase.release_all();
    drop(writer);
    drop(phrase);
    std::fs::remove_dir_all(&directory).unwrap();
}
