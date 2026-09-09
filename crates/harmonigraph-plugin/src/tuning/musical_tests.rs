//! Musical integration through the exported production callbacks.
//!
//! These are the tests the fault cut did not change: policy v2 is untouched,
//! so what a chord across three tracks comes out as is what it always was.
//! What changed underneath is where the answer is read from — there is one
//! authority now, the Hub's own schedule, rather than a plan and an accepted
//! output that had to agree.
use super::*;
use harmonigraph_core::configuration::{ConfigEdit, PolicyConfig};
use harmonigraph_core::{canonical::VoiceBaseline, LatticePos, Tuning};

fn configure(hub: &Device, tuning: Tuning) {
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
        // Explicit accepted neutral initialization, not a claim about Bitwig's
        // unmeasured initial CC64/66/69 state (#696).
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
        for index in 0..3 {
            self.sources[index].run_format(self.raw, vec![position(seconds)], None, None, 512);
        }
        self.hub.run_format(self.raw, vec![position(seconds)], None, None, 512);
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

/// The frozen correction is composed with the player's own expression rather
/// than replacing it: the emitted tuning value moves with a later per-note
/// bend while the harmonic reference the next decision reads does not.
#[test]
fn production_tune_preserves_player_pitch_and_freezes_only_the_adaptive_correction() {
    let _scope = crate::test_scope::enter();
    let mut phrase = Phrase::new();
    phrase.step([vec![note(7, 0, 57, 0, true), expression(7, 0.25, 0)], vec![], vec![]], [0, 1, 2]);
    let output = phrase.idle();
    let onset = phrase.voice(0, 57, 0);
    let correction = onset.frozen_offset_microcents;
    assert_ne!(correction, 0, "an adaptive choice was made for this attack");
    let emitted = output[0]
        .iter()
        .find_map(|(_, event)| match event {
            Event::Expression { kind: 2, value, .. } => Some(*value),
            _ => None,
        })
        .expect("the onset states its tuning");
    assert!((emitted - (0.25 + correction as f64 / 100_000_000.0)).abs() < 1e-9);
    // A later expression moves the emitted pitch and not the frozen choice.
    phrase.step([vec![expression(7, 0.5, 0)], vec![], vec![]], [0, 1, 2]);
    let output = phrase.idle();
    let moved = output[0]
        .iter()
        .find_map(|(_, event)| match event {
            Event::Expression { kind: 2, value, .. } => Some(*value),
            _ => None,
        })
        .expect("the later expression is forwarded with the correction added");
    assert!((moved - (0.5 + correction as f64 / 100_000_000.0)).abs() < 1e-9);
    let after = phrase.voice(0, 57, 0);
    assert_eq!(after.frozen_offset_microcents, correction);
    assert_eq!(after.onset_pitch_microcents, onset.onset_pitch_microcents);
    assert_ne!(after.pitch_microcents, onset.pitch_microcents, "what sounds did move");
    phrase.release_all();
}

/// Both reset controls act on released memory only. Held voices keep the
/// choice they were given whatever the transport does, and the neighbourhood
/// the Hub publishes is what says whether the memory survived.
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
        phrase.step(std::array::from_fn(|_| vec![transport(0, 120.0)]), [0, 1, 2]);
        phrase.step(
            std::array::from_fn(|_| {
                let Input::Transport(mut value) = transport(0, 120.0) else { unreachable!() };
                value.flags &= !CLAP_TRANSPORT_IS_PLAYING;
                vec![Input::Transport(value)]
            }),
            [0, 1, 2],
        );
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

/// The journey the old eight-bit coordinate and 2,147-cent correction limits
/// used to stop. Nothing here is about the transport; it is the one musical
/// case that proves the representation is still wide enough.
#[test]
fn production_moving_major_thirds_cross_old_coordinate_and_correction_limits() {
    let _scope = crate::test_scope::enter();
    let mut phrase = Phrase::new();
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
        configure_policy(&phrase.hub, PolicyConfig { reset_loop, ..Default::default() });
        phrase.idle();
        // Three chords of moving thirds, so the reference has travelled and a
        // cleared memory is distinguishable from a kept one.
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
