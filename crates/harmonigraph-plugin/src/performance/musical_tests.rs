//! Musical integration through exported production callbacks and accepted output.
use super::*;
use harmonigraph_core::{canonical::VoiceBaseline, configuration::ConfigEdit, LatticePos, Tuning};

pub(super) fn configure(hub: &Device, tuning: Tuning) {
    let wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    wrapper
        .configuration_handle()
        .unwrap()
        .submit(crate::configuration::packet(ConfigEdit {
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
        }))
        .unwrap();
}

struct Phrase {
    hub: Device,
    sources: [Device; 3],
    raw: i64,
    maximum: [u128; 4],
}
impl Phrase {
    fn new() -> Self {
        let uuid = SavedUuid::default();
        let calibration =
            Calibration { offset: 0, sample_rate: 44100.0, max_frames: 512, validated: true };
        let mut hub = Device::new(false);
        hub.configure_format(uuid, true, calibration);
        hub.activate_format(44100.0, 512);
        configure(&hub, Tuning::just());
        let sources = std::array::from_fn(|_| {
            let mut source = Device::new(true);
            source.configure_format(uuid, true, calibration);
            source.activate_format(44100.0, 512);
            source
        });
        let mut phrase = Self { hub, sources, raw: 0, maximum: [0; 4] };
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
        phrase.maximum = [0; 4];
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
            self.maximum[index] = self.maximum[index].max(sink.callback_nanos);
            output[index] = sink.values;
            assert_eq!(self.sources[index].source_snapshot().faults, 0);
        }
        let sink = self.hub.run_format(self.raw, vec![], None, None, 512);
        self.maximum[3] = self.maximum[3].max(sink.callback_nanos);
        self.raw += 512;
        output
    }
    fn idle(&mut self) -> [Vec<(u32, Event)>; 3] {
        self.step(std::array::from_fn(|_| vec![]), [0, 1, 2])
    }
    fn voice(&self, source: usize, key: u8, channel: u8) -> VoiceBaseline {
        inspect_source(&self.sources[source], |source| {
            *source
                .state
                .voices()
                .find(|voice| voice.note == key && voice.channel == channel)
                .unwrap()
        })
    }
    fn release_all(&mut self) {
        let input = std::array::from_fn(|index| {
            inspect_source(&self.sources[index], |source| {
                source
                    .state
                    .voices()
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
        for _ in 0..16 {
            self.idle();
        }
        assert!(self
            .sources
            .iter()
            .all(|source| source.source_snapshot().held == 0
                && source.source_snapshot().pending == 0));
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
            assert_eq!(output[i].len(), 2, "{output:?}");
            assert_eq!(output[i][0].0, 0);
            let voice = phrase.voice(i, keys[i] as u8, 0);
            assert_eq!(voice.attack_node, Some(positions[i]));
            assert_eq!(voice.assignment.unwrap().policy, harmonigraph_core::policy::CONFIG);
            let factual =
                inspect_hub(&phrase.hub, |hub| hub.test_voice(i, voice.lifetime).unwrap());
            assert_eq!(factual.pitch_microcents, voice.pitch_microcents);
            assert_eq!(factual.metadata(), voice.metadata());
            pitches[i] = voice.pitch_microcents;
            decisions[i] = voice.decision;
        }
        assert!(decisions[0] < decisions[1] && decisions[1] < decisions[2]);
        assert_eq!(pitches[1] - pitches[0], i64::from(Tuning::just().three - Tuning::just().five));
        assert_eq!(pitches[2] - pitches[0], i64::from(Tuning::just().three));
        phrase.release_all();
    }
}

#[test]
fn production_musical_released_history_is_source_channel_specific_and_off_clears_it() {
    let _scope = crate::test_scope::enter();
    let mut phrase = Phrase::new();
    // C/G establish this Source/channel's E at 5/4, then all three release.
    phrase.step([vec![note(1, 0, 48, 0, true)], vec![note(2, 0, 55, 0, true)], vec![]], [0, 1, 2]);
    phrase.idle();
    phrase.step([vec![], vec![], vec![note(3, 0, 52, 0, true)]], [0, 1, 2]);
    phrase.idle();
    let established = phrase.voice(2, 52, 0);
    assert_eq!(established.attack_node, Some(LatticePos::new(0, 1, 0)));
    phrase.release_all();
    let wrapper = unsafe {
        &*((*phrase.hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    wrapper
        .configuration_handle()
        .unwrap()
        .submit(crate::configuration::packet(ConfigEdit {
            axes: [None, None, None, None, Some(25_000_000)],
            ..ConfigEdit::default()
        }))
        .unwrap();
    phrase.idle();
    phrase.step([vec![note(4, 0, 50, 0, true)], vec![], vec![]], [0, 1, 2]);
    phrase.idle();
    phrase.step([vec![], vec![], vec![note(5, 0, 52, 0, true)]], [0, 1, 2]);
    phrase.idle();
    assert_eq!(
        phrase.voice(2, 52, 0).attack_node,
        Some(LatticePos::new(0, 1, 0)),
        "release retained the exact key history"
    );
    assert_eq!(
        phrase.voice(2, 52, 0).assignment.unwrap().revision,
        established.assignment.unwrap().revision,
        "display tolerance must preserve the musical revision and history"
    );
    phrase.step([vec![], vec![], vec![note(5, 0, 52, 0, false)]], [0, 1, 2]);
    phrase.idle();
    phrase.step([vec![], vec![note(6, 0, 52, 0, true)], vec![]], [0, 1, 2]);
    phrase.idle();
    assert_eq!(
        phrase.voice(1, 52, 0).attack_node,
        Some(LatticePos::new(4, 0, 0)),
        "another Source has no E history"
    );
    phrase.step([vec![], vec![note(6, 0, 52, 0, false)], vec![]], [0, 1, 2]);
    phrase.idle();
    phrase.step([vec![], vec![], vec![note(7, 1, 52, 0, true)]], [0, 1, 2]);
    phrase.idle();
    assert_eq!(
        phrase.voice(2, 52, 1).attack_node,
        Some(LatticePos::new(4, 0, 0)),
        "another channel has no E history"
    );
    phrase.step([vec![], vec![], vec![note(7, 1, 52, 0, false)]], [0, 1, 2]);
    phrase.idle();
    let off = phrase.sources[2].participation(false, 0);
    phrase.step([vec![], vec![], vec![off]], [0, 1, 2]);
    phrase.idle();
    let on = phrase.sources[2].participation(true, 0);
    phrase.step([vec![], vec![], vec![on]], [0, 1, 2]);
    phrase.idle();
    phrase.step([vec![], vec![], vec![note(8, 0, 52, 0, true)]], [0, 1, 2]);
    phrase.idle();
    assert_eq!(
        phrase.voice(2, 52, 0).attack_node,
        Some(LatticePos::new(4, 0, 0)),
        "Off removed the old E preference"
    );
    phrase.release_all();
}

#[test]
fn production_musical_no_candidate_preserves_player_expression() {
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
    assert_eq!(voice.attack_node, None);
    assert_ne!(voice.decision, 0, "NoCandidate is a completed assignment");
    assert_eq!(voice.frozen_offset_microcents, 0);
    assert_eq!(voice.player_tuning, 0.375);
    assert_eq!(voice.pitch_microcents, 6_337_500_000);
    assert!(output[0]
        .iter()
        .any(|(_, event)| matches!(event, Event::Expression { value: 0.375, .. })));
    phrase.release_all();
}

#[test]
fn production_musical_off_preserves_sounding_pitch_but_excludes_future_scoring() {
    let _scope = crate::test_scope::enter();
    let mut phrase = Phrase::new();
    phrase.step([vec![note(1, 0, 50, 0, true)], vec![], vec![]], [0, 1, 2]);
    phrase.idle();
    let held = phrase.voice(0, 50, 0);
    let off = phrase.sources[0].participation(false, 0);
    // Marker and foreign onset share a sample; callback order cannot let the
    // still-sounding D decide this E. Empty context prefers 5/4, not 81/64.
    phrase.step(
        [vec![off, expression(1, 0.25, 0)], vec![note(2, 0, 52, 0, true)], vec![]],
        [1, 2, 0],
    );
    phrase.idle();
    let bent = phrase.voice(0, 50, 0);
    assert_eq!(bent.frozen_offset_microcents, held.frozen_offset_microcents);
    assert_eq!(bent.pitch_microcents, held.pitch_microcents + 25_000_000);
    assert_eq!(phrase.voice(1, 52, 0).attack_node, Some(LatticePos::new(0, 1, 0)));
    assert_eq!(phrase.sources[0].source_snapshot().held, 1);
    phrase.release_all();
}

#[test]
fn production_musical_normal_phrase_keeps_bends_old_configuration_and_take_metadata() {
    use harmonigraph_take::CanonicalRecord;
    let _scope = crate::test_scope::enter();
    let (recorder, mut capture) = harmonigraph_record::testing::channel();
    crate::configuration::inject_recorder(recorder);
    let mut phrase = Phrase::new();
    capture.arm();
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-musical-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("phrase.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    phrase.step(
        std::array::from_fn(|source| {
            (0..5)
                .map(|i| {
                    let id = (source * 5 + i) as i32;
                    note(id, 0, 48 + id as i16, 0, true)
                })
                .collect()
        }),
        [2, 0, 1],
    );
    phrase.idle();
    let original: [Vec<VoiceBaseline>; 3] = std::array::from_fn(|i| {
        inspect_source(&phrase.sources[i], |source| source.state.voices().copied().collect())
    });
    assert!(original.iter().all(|voices| voices.len() == 5));
    phrase.step([vec![expression(0, 7.0, 0)], vec![], vec![]], [0, 1, 2]);
    phrase.idle();
    configure(&phrase.hub, Tuning { three: 690_000_000, ..Tuning::just() });
    phrase.idle();
    let current: [Vec<VoiceBaseline>; 3] = std::array::from_fn(|i| {
        inspect_source(&phrase.sources[i], |source| source.state.voices().copied().collect())
    });
    for source in 0..3 {
        for (old, now) in original[source].iter().zip(&current[source]) {
            assert_eq!(old.frozen_offset_microcents, now.frozen_offset_microcents);
            assert_eq!(old.attack_node, now.attack_node);
            assert_eq!(old.assignment, now.assignment);
            assert_eq!(
                now.pitch_microcents,
                old.pitch_microcents + if old.host_note_id == 0 { 700_000_000 } else { 0 }
            );
        }
    }
    let wrapper = unsafe {
        &*((*phrase.hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    let config = wrapper.test_inspect_plugin(|plugin| {
        plugin.configuration.as_ref().unwrap().timeline.reducer().resolved()
    });
    let context: Vec<_> = current
        .iter()
        .flatten()
        .map(|v| harmonigraph_core::policy::ContextPitch {
            pitch: harmonigraph_core::PitchClass::from_microcents(v.pitch_microcents),
            node: v.attack_node,
        })
        .collect();
    let stale: Vec<_> = current
        .iter()
        .flatten()
        .map(|v| harmonigraph_core::policy::ContextPitch {
            pitch: config.tuning.pitch_class(v.attack_node.unwrap()),
            node: v.attack_node,
        })
        .collect();
    let choose = |key, context: &[_]| {
        harmonigraph_core::policy::assign_new_note(
            config.into(),
            context,
            None,
            harmonigraph_core::policy::OrderedOnset { key },
            &mut Default::default(),
        )
        .unwrap()
    };
    // F# selects (2,1) from the actual phrase, versus (-2,-1) if the old
    // attack nodes were incorrectly interpreted as current pitch authority.
    let key = 66;
    let expected = choose(key, &context);
    let wrong = choose(key, &stale);
    assert_ne!(expected.assignment, wrong.assignment);
    phrase.step([vec![], vec![], vec![note(15, 0, i16::from(key), 0, true)]], [0, 1, 2]);
    phrase.idle();
    let added = phrase.voice(2, key, 0);
    assert_ne!(added.assignment.unwrap().revision, original[0][0].assignment.unwrap().revision);
    let harmonigraph_core::policy::Assignment::Selected { node, .. } = expected.assignment else {
        panic!("selected candidate")
    };
    assert_eq!(added.attack_node, Some(node));
    assert_eq!(
        added.frozen_offset_microcents,
        i64::from(expected.assignment.correction_microcents())
    );
    println!(
        "MUSICAL actual context key{key} expected {:?}, stale {:?}",
        expected.assignment, wrong.assignment
    );
    writer.drain(&mut capture);
    let display = writer.display_events();
    let mut live = harmonigraph_core::NoteTracker::new();
    for record in &display {
        record.apply(&mut live).unwrap();
    }
    assert_eq!(live.held_count(), 16);
    let facts: Vec<_> = phrase
        .sources
        .iter()
        .flat_map(|source| {
            inspect_source(source, |source| {
                let identity = source.offer.as_ref().unwrap().lease.source;
                source.state.voices().map(|v| (identity, *v)).collect::<Vec<_>>()
            })
        })
        .collect();
    for (source, voice) in &facts {
        let shown = live
            .voices()
            .find(|v| v.source == *source && v.note == voice.note && v.channel == voice.channel)
            .unwrap();
        assert_eq!(shown.assignment, voice.metadata());
        assert_eq!(shown.pitch, voice.pitch());
        assert!(display.iter().any(|record| matches!(record,CanonicalRecord::Delta(d) if d.event.source == source.0 && d.lifetime == voice.lifetime && d.assignment.is_some())));
    }
    // The reaching evidence is normal delta metadata, not repaired baselines.
    assert!(display
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Baseline(b) => Some(b),
            _ => None,
        })
        .all(|b| b.voices.is_empty()));
    phrase.release_all();
    writer.drain(&mut capture);
    capture.stop();
    writer.stop();
    phrase.idle();
    writer.drain(&mut capture);
    let take = harmonigraph_take::Take::read(&path).unwrap();
    assert!(take.incomplete.is_none());
    assert!(take.configurations.iter().all(|c| c.policy.version == 1));
    let mut replay = harmonigraph_core::NoteTracker::new();
    for record in &take.events {
        if matches!(record,CanonicalRecord::Delta(d) if matches!(d.event.kind,harmonigraph_take::NoteKind::Off))
        {
            break;
        }
        record.apply(&mut replay).unwrap();
    }
    assert_eq!(replay.held_count(), 16);
    for (source, voice) in facts {
        let replayed = replay
            .voices()
            .find(|v| v.source == source && v.note == voice.note && v.channel == voice.channel)
            .unwrap();
        assert_eq!(replayed.assignment, voice.metadata());
        assert_eq!(replayed.pitch, voice.pitch());
    }
    println!("MUSICAL ordinary15 plus successor callback_max_ns {:?}", phrase.maximum);
    drop(writer);
    drop(phrase);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn production_musical_configuration_and_stop_recovery_clear_history() {
    let _scope = crate::test_scope::enter();
    for stop in [false, true] {
        let mut phrase = Phrase::new();
        phrase.step([vec![], vec![], vec![note(1, 0, 52, 0, true)]], [0, 1, 2]);
        phrase.idle();
        assert_eq!(phrase.voice(2, 52, 0).attack_node, Some(LatticePos::new(0, 1, 0)));
        phrase.release_all();
        phrase.step([vec![note(2, 0, 50, 0, true)], vec![], vec![]], [0, 1, 2]);
        phrase.idle();
        let held = phrase.voice(0, 50, 0);
        if stop {
            // Releasing this held D at Stop changes actual output and reaches
            // recovery. A silent Stop in the same epoch need not clear history.
            phrase.step(std::array::from_fn(|_| vec![transport(0, 120.0)]), [0, 1, 2]);
            phrase.step(
                std::array::from_fn(|_| {
                    let Input::Transport(mut value) = transport(0, 120.0) else { unreachable!() };
                    value.flags &= !CLAP_TRANSPORT_IS_PLAYING;
                    vec![Input::Transport(value)]
                }),
                [0, 1, 2],
            );
            for _ in 0..96 {
                phrase.idle();
            }
            assert!(phrase.sources.iter().all(|source| source.source_snapshot().held == 0));
            phrase.step([vec![note(3, 0, 50, 0, true)], vec![], vec![]], [0, 1, 2]);
            phrase.idle();
        } else {
            let wrapper = unsafe {
                &*((*phrase.hub.plugin)
                    .plugin_data
                    .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
            };
            wrapper
                .configuration_handle()
                .unwrap()
                .submit(crate::configuration::packet(ConfigEdit::axis(3, 960_000_000)))
                .unwrap();
            phrase.idle();
            let unchanged = phrase.voice(0, 50, 0);
            assert_eq!(held.pitch_microcents, unchanged.pitch_microcents);
            assert_eq!(held.metadata(), unchanged.metadata());
        }
        phrase.step([vec![], vec![], vec![note(4, 0, 52, 0, true)]], [0, 1, 2]);
        phrase.idle();
        assert_eq!(
            phrase.voice(2, 52, 0).attack_node,
            Some(LatticePos::new(4, 0, 0)),
            "boundary stop={stop} removes release-surviving E history"
        );
        phrase.release_all();
    }
}
