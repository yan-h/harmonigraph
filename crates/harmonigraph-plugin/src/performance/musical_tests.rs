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
            ..Default::default()
        }))
        .unwrap();
}

#[test]
fn production_musical_setup_is_automatic_when_three_sources_precede_hub_audio() {
    let _scope = crate::test_scope::enter();
    for (restored, rate, frames) in [(false, 44100.0, 512u32), (true, 48000.0, 256)] {
        let restore = |device: &Device| {
            if !restored {
                return;
            }
            // Real state-load path: obsolete manual validation and host format
            // fields neither disable playback nor survive the next save.
            let mut state = device.save();
            let field = if device.tuner { setup::SOURCE_FIELD } else { setup::HUB_FIELD };
            let mut routing: serde_json::Value =
                serde_json::from_str(&state.fields[field]).unwrap();
            routing["calibration"]["sample_rate"] = serde_json::json!(0.0);
            routing["calibration"]["max_frames"] = serde_json::json!(17);
            routing["calibration"]["validated"] = serde_json::json!(false);
            state.fields.insert(field.into(), routing.to_string());
            assert!(device.load(&state));
            let saved: serde_json::Value =
                serde_json::from_str(&device.save().fields[field]).unwrap();
            assert_eq!(saved["calibration"], serde_json::json!({"offset": 0}));
        };
        let mut sources: [Device; 3] = std::array::from_fn(|_| {
            let mut source = Device::new(true);
            restore(&source);
            source.activate_format(rate, frames);
            source
        });
        let mut output: [Vec<(u32, Event)>; 3] = std::array::from_fn(|_| Vec::new());
        let mut startup: [Vec<(i64, Event)>; 3] = std::array::from_fn(|_| Vec::new());
        let mut raw = 0;
        for _ in 0..3 {
            for source in &mut sources {
                source.run_format(raw, vec![], None, None, frames);
            }
            raw += i64::from(frames);
        }
        let mut hub = Device::new(false);
        restore(&hub);
        configure(&hub, Tuning::just());
        hub.activate_format(rate, frames);
        // Bitwig can reset a restored Hub before its first audio callback.
        // Registry offers still contain the provisional epoch at this point.
        unsafe { (*hub.plugin).reset.unwrap()(hub.plugin) };
        // Initial enrollment can miss D512; allow the existing sliced lateness
        // recovery to settle before starting the four later gestures.
        for step in 0..96 {
            for (index, source) in sources.iter_mut().enumerate() {
                source.main();
                // Real input after Hub registration/reset but before its first
                // audio callback must survive the initial enrollment wait.
                let key = [60, 64, 67][index];
                let input = match step {
                    0 => {
                        let mut input: Vec<_> =
                            [64, 66, 69].map(|cc| raw_midi([0xb0, cc, 0], 0)).into();
                        input.push(note(0, 0, key, 0, true));
                        input
                    }
                    1 => vec![note(0, 0, key, 0, false)],
                    _ => vec![],
                };
                let sink = source.run_format(raw, input, None, None, frames);
                startup[index].extend(
                    sink.values.iter().map(|(time, event)| (raw + i64::from(*time), *event)),
                );
                output[index].extend(sink.values);
                assert_eq!(source.source_snapshot().faults, 0);
            }
            hub.main();
            hub.run_format(raw, vec![], None, None, frames);
            raw += i64::from(frames);
        }
        for device in [&hub, &sources[0], &sources[1], &sources[2]] {
            let shared = device.shared();
            let adopted = shared.adopted().unwrap();
            assert_eq!((adopted.sample_rate, adopted.max_frames), (rate, frames));
            assert!(adopted.valid, "restore={restored}: {adopted:?}");
            assert_eq!(adopted.generation, shared.value().generation);
        }
        for events in &startup {
            let onset = events.iter().find(|(_, event)| event.attack().is_some()).unwrap().0;
            let release = events.iter().find(|(_, event)| event.release()).unwrap().0;
            assert_eq!(release - onset, i64::from(frames), "the original startup gesture survives");
            assert_eq!(
                events
                    .iter()
                    .filter(|(_, event)| matches!(
                        event,
                        Event::Midi { data: [0xb0, 64 | 66 | 69, 0], .. }
                    ))
                    .count(),
                3
            );
        }
        for source in &sources {
            let state = source.source_snapshot();
            assert_eq!((state.held, state.pending, state.faults), (0, 0, 0), "{state:?}");
        }
        // Five ordinary C/E/G cohorts exercise fifteen real accepted notes,
        // including nonzero Just corrections and their matching releases.
        for id in 1..5 {
            for step in 0..16 {
                for (index, source) in sources.iter_mut().enumerate() {
                    let key = [60, 64, 67][index];
                    let input = match step {
                        0 => {
                            let mut input: Vec<_> =
                                [64, 66, 69].map(|cc| raw_midi([0xb0, cc, 0], 0)).into();
                            input.push(note(id, 0, key, 0, true));
                            input
                        }
                        8 => vec![note(id, 0, key, 0, false)],
                        _ => vec![],
                    };
                    output[index].extend(source.run_format(raw, input, None, None, frames).values);
                }
                hub.run_format(raw, vec![], None, None, frames);
                raw += i64::from(frames);
            }
        }
        for (index, source) in sources.iter().enumerate() {
            assert_eq!(
                output[index].iter().filter(|(_, e)| e.attack().is_some()).count(),
                5,
                "source={index} restore={restored}: {:?}",
                source.source_snapshot()
            );
            assert_eq!(output[index].iter().filter(|(_, e)| e.release()).count(), 5);
            if index != 0 {
                assert!(output[index].iter().any(|(_, event)| matches!(event,
                    Event::Expression { kind: CLAP_NOTE_EXPRESSION_TUNING, value, .. } if *value != 0.0)));
            }
            let snapshot = source.source_snapshot();
            assert_eq!((snapshot.held, snapshot.pending, snapshot.faults), (0, 0, 0));
        }
    }
}

#[test]
fn production_tune_preserves_player_pitch_and_freezes_only_adaptive_correction() {
    let _scope = crate::test_scope::enter();
    for incoming in [0.0, -0.12446594] {
        let mut phrase = Phrase::new();
        for (source, key) in [60, 64, 67].into_iter().enumerate() {
            let mut input: [Vec<Input>; 3] = std::array::from_fn(|_| vec![]);
            input[source] = vec![note(1, 0, key, 0, true), expression(1, incoming, 0)];
            phrase.step(input, [0, 1, 2]);
            for _ in 0..8 {
                phrase.idle();
            }
        }
        let pitches = std::array::from_fn::<_, 3, _>(|index| {
            let voice = phrase.voice(index, [60, 64, 67][index], 0);
            assert_eq!(voice.player_tuning, incoming);
            (
                voice.pitch_microcents,
                voice.attack_node,
                voice.frozen_offset_microcents,
                voice.onset_pitch_microcents,
            )
        });
        let Input::Expression(mut pressure) = expression(1, 0.7, 2) else { unreachable!() };
        pressure.expression_id = 6;
        let mut accepted = phrase.step(
            [
                vec![],
                vec![
                    expression(1, 0.5, 0),
                    raw_midi([0xe0, 127, 127], 1),
                    Input::Expression(pressure),
                ],
                vec![],
            ],
            [0, 1, 2],
        )[1]
        .clone();
        for _ in 0..8 {
            accepted.extend(phrase.idle()[1].iter().copied());
        }
        let after = phrase.voice(1, 64, 0);
        assert_eq!(after.frozen_offset_microcents, pitches[1].2);
        assert_eq!(after.onset_pitch_microcents, pitches[1].3);
        assert_eq!(after.attack_node, pitches[1].1);
        assert_eq!(after.player_tuning, 0.5);
        assert!(
            (after.pitch_microcents - (6_400_000_000 + pitches[1].2 + 50_000_000 + 199_975_585))
                .abs()
                <= 2
        );
        assert!(accepted
            .iter()
            .any(|(_, event)| matches!(event, Event::Expression { kind: 6, value: 0.7, .. })));
        assert!(accepted
            .iter()
            .any(|(_, event)| matches!(event, Event::Midi { data: [0xe0, 127, 127], .. })));
        phrase.release_all();
    }
}

#[test]
fn production_hub_reinitialize_settles_while_new_notes_arrive() {
    let _scope = crate::test_scope::enter();
    let mut phrase = Phrase::new();
    phrase.step([vec![], vec![note(1, 0, 60, 0, true)], vec![]], [0, 1, 2]);
    for _ in 0..8 {
        phrase.idle();
    }
    assert_eq!(phrase.sources[1].source_snapshot().held, 1);
    let shared = phrase.hub.shared();
    shared.apply(shared.value().routing, true).unwrap();
    let mut output: [Vec<(u32, Event)>; 3] = std::array::from_fn(|_| Vec::new());
    for step in 0..160 {
        for source in &phrase.sources {
            source.main();
        }
        phrase.hub.main();
        let input = match step {
            2 => vec![note(2, 0, 64, 0, true)],
            3 => vec![expression(2, 0.125, 0)],
            4 => vec![note(1, 0, 60, 0, false), note(2, 0, 64, 1, false)],
            _ => vec![],
        };
        for (all, next) in output.iter_mut().zip(phrase.step([vec![], input, vec![]], [0, 1, 2])) {
            all.extend(next);
        }
    }
    assert_eq!(
        shared.adopted().unwrap().generation,
        shared.value().generation,
        "{}; {}",
        inspect_hub(&phrase.hub, |hub| hub.direct.test_reset_progress()),
        inspect_source(&phrase.sources[1], |source| source.test_reset_progress())
    );
    for source in &phrase.sources {
        let snapshot = source.source_snapshot();
        assert_eq!(
            (snapshot.held, snapshot.pending, snapshot.captures, snapshot.faults),
            (0, 0, 0, 0)
        );
    }
    assert!(output[1]
        .iter()
        .any(|(_, event)| event.release() && matches!(event, Event::Note { id: 1, .. })));
    assert_eq!(
        output[1]
            .iter()
            .filter(
                |(_, event)| event.attack().is_some() && matches!(event, Event::Note { id: 2, .. })
            )
            .count(),
        1
    );
    assert_eq!(
        output[1]
            .iter()
            .filter(|(_, event)| event.release() && matches!(event, Event::Note { id: 2, .. }))
            .count(),
        1
    );
    // A fresh phrase must sound after the reset, without another setup click.
    phrase.step([vec![], vec![note(3, 0, 67, 0, true)], vec![]], [0, 1, 2]);
    for _ in 0..8 {
        phrase.idle();
    }
    assert_eq!(phrase.voice(1, 67, 0).note, 67);
    phrase.release_all();
}

struct Phrase {
    uuid: SavedUuid,
    hub: Device,
    sources: [Device; 3],
    raw: i64,
    maximum: [u128; 4],
}
impl Phrase {
    fn new() -> Self {
        Self::with_calibration([0; 3])
    }
    fn with_calibration(offsets: [i64; 3]) -> Self {
        let uuid = SavedUuid::default();
        let calibration = Calibration { offset: 0 };
        let mut hub = Device::new(false);
        hub.configure_format(uuid, true, calibration);
        hub.activate_format(44100.0, 512);
        configure(&hub, Tuning::just());
        let sources = std::array::from_fn(|i| {
            let mut source = Device::new(true);
            source.configure_format(uuid, true, Calibration { offset: offsets[i] });
            source.activate_format(44100.0, 512);
            source
        });
        let mut phrase = Self { uuid, hub, sources, raw: 0, maximum: [0; 4] };
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
fn production_diagnostics_identify_three_sources_and_apply_history_wait() {
    let _scope = crate::test_scope::enter();
    use super::super::diagnostics::{HUB_FIELDS, ROW_FIELDS, SOURCE_FIELDS};
    fn field(labels: &[&str], values: &[i64], name: &str) -> i64 {
        values[labels.iter().position(|label| *label == name).unwrap()]
    }
    let mut phrase = Phrase::new();
    for source in &phrase.sources {
        source.main();
    }
    phrase.hub.main();
    let wakes_before = phrase.sources[1]._stats.callbacks.load(Ordering::Acquire);

    phrase.step(
        [
            vec![note(0, 0, 60, 0, true)],
            vec![note(1, 0, 64, 0, true)],
            vec![note(2, 0, 67, 0, true)],
        ],
        [0, 1, 2],
    );
    // Cross the real one-second audio sampling interval, with no editors.
    for _ in 0..96 {
        phrase.idle();
    }
    assert!(
        phrase.sources[1]._stats.callbacks.load(Ordering::Acquire) > wakes_before,
        "periodic diagnostics request the real host callback after initial setup was drained"
    );
    let mut identities = Vec::new();
    for (index, source) in phrase.sources.iter().enumerate() {
        let values = source.shared().diagnostics.source.read().unwrap();
        identities.push(field(SOURCE_FIELDS, &values, "source"));
        assert_eq!(field(SOURCE_FIELDS, &values, "input_on"), 1);
        assert_eq!(field(SOURCE_FIELDS, &values, "output_on"), 1);
        assert!(field(SOURCE_FIELDS, &values, "assignments") > 0);
        assert_eq!(field(SOURCE_FIELDS, &values, "last_input_key"), [60, 64, 67][index]);
        assert_eq!(
            field(SOURCE_FIELDS, &values, "last_output_pitch_mc"),
            phrase.voice(index, [60, 64, 67][index], 0).pitch_microcents
        );
        source.main(); // The existing callback drains the real stderr logger.
    }
    identities.sort_unstable();
    identities.dedup();
    assert_eq!(identities.len(), 3);
    let shared = phrase.hub.shared();
    let values = shared.diagnostics.hub.as_ref().unwrap().read().unwrap();
    assert_eq!(field(HUB_FIELDS, &values, "published_on"), 3);
    assert_eq!(field(HUB_FIELDS, &values, "third_mc"), i64::from(Tuning::just().five));
    for row in shared.diagnostics.rows.as_ref().unwrap().iter().take(3) {
        assert_eq!(field(ROW_FIELDS, &row.read().unwrap(), "member"), 1);
    }
    phrase.hub.main();
    // A pitch update from the first Tune must not retain the last onset's
    // identity from the third Tune while reporting the first Tune's pitch.
    phrase.step([vec![expression(0, 0.5, 0)], vec![], vec![]], [0, 1, 2]);
    for _ in 0..96 {
        phrase.idle();
    }
    let source_values = phrase.sources[0].shared().diagnostics.source.read().unwrap();
    assert_eq!(field(SOURCE_FIELDS, &source_values, "last_output_player_mc"), 50_000_000);
    assert_eq!(
        field(SOURCE_FIELDS, &source_values, "last_output_correction_mc"),
        phrase.voice(0, 60, 0).frozen_offset_microcents
    );
    let hub_values = shared.diagnostics.hub.as_ref().unwrap().read().unwrap();
    assert_eq!(field(HUB_FIELDS, &hub_values, "last_published_source"), identities[0]);
    assert_eq!(field(HUB_FIELDS, &hub_values, "last_published_key"), 60);
    assert_eq!(
        field(HUB_FIELDS, &hub_values, "last_published_pitch_mc"),
        phrase.voice(0, 60, 0).pitch_microcents
    );
    let before = phrase.sources[1].shared().diagnostics.report(&phrase.sources[1].shared());
    for _ in 0..96 {
        phrase.idle();
    }
    let after = phrase.sources[1].shared().diagnostics.report(&phrase.sources[1].shared());
    assert_ne!(before.0, after.0, "actual sample frontiers advanced");
    assert_eq!(before.1, after.1, "healthy idle clock advance must not log again");

    // An actual Apply while the Hub stops consuming leaves accepted history
    // owned by this Source. Observe the real gate, without changing recovery.
    let source = &phrase.sources[1];
    let shared = source.shared();
    let previous = shared.adopted().unwrap().generation;
    shared.apply(shared.value().routing, true).unwrap();
    let paused_at = phrase.raw;
    for _ in 0..96 {
        source.run_format(phrase.raw, vec![], None, None, 512);
        phrase.raw += 512;
    }
    let values = shared.diagnostics.source.read().unwrap();
    assert!(shared.value().generation > previous);
    assert_eq!(shared.adopted().unwrap().generation, previous);
    assert_eq!(field(SOURCE_FIELDS, &values, "setup_wait"), 3);
    assert_ne!(field(SOURCE_FIELDS, &values, "output_settlement_wait") & ((1 << 0) | (1 << 3)), 0);
    assert_eq!(field(SOURCE_FIELDS, &values, "output_on"), 1);
    assert_eq!(field(SOURCE_FIELDS, &values, "source"), identities[1]);
    let report = shared.diagnostics.report(&shared);
    assert_ne!(report.1, after.1, "Apply and stalled ownership must trigger a changed log");
    assert!(report.0.contains("setup_wait=3(accepted_history)"));
    // Restore the actual missing callback intervals before destroying this
    // fixture; abandoned held streams would pollute the process-wide registry.
    for raw in (paused_at..phrase.raw).step_by(512) {
        for index in [0, 2] {
            phrase.sources[index].run_format(raw, vec![], None, None, 512);
        }
        phrase.hub.run_format(raw, vec![], None, None, 512);
    }
    for _ in 0..96 {
        for source in &phrase.sources {
            source.main();
        }
        phrase.hub.main();
        phrase.idle();
    }
    phrase.release_all();
    drop(phrase);
    assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
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
    // successors start in the same callback, so every terminal reaches the
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

#[test]
fn production_musical_new_harmony_replaces_released_memory_across_sources_and_channels() {
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
        Some(LatticePos::new(4, 0, 0)),
        "the new held D can outweigh released just-E memory"
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
        "another Source uses the same shared harmonic context"
    );
    phrase.step([vec![], vec![note(6, 0, 52, 0, false)], vec![]], [0, 1, 2]);
    phrase.idle();
    phrase.step([vec![], vec![], vec![note(7, 1, 52, 0, true)]], [0, 1, 2]);
    phrase.idle();
    assert_eq!(
        phrase.voice(2, 52, 1).attack_node,
        Some(LatticePos::new(4, 0, 0)),
        "another channel uses the same shared harmonic context"
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

#[test]
fn production_musical_off_ends_its_own_phrase_and_excludes_future_scoring() {
    let _scope = crate::test_scope::enter();
    let mut phrase = Phrase::new();
    phrase.step([vec![note(1, 0, 50, 0, true)], vec![], vec![]], [0, 1, 2]);
    phrase.idle();
    assert_eq!(phrase.sources[0].source_snapshot().held, 1);
    let off = phrase.sources[0].participation(false, 0);
    // Marker and foreign onset share a sample; callback order cannot let the
    // ended D decide this E. Empty context prefers 5/4, not 81/64.
    let output = phrase.step(
        [vec![off, expression(1, 0.25, 0)], vec![note(2, 0, 52, 0, true)], vec![]],
        [1, 2, 0],
    );
    phrase.idle();
    assert_eq!(phrase.voice(1, 52, 0).attack_node, Some(LatticePos::new(0, 1, 0)));
    // Off is a reset in either direction: the toggle terminates the D this
    // Tune had already forwarded rather than carrying it into the new mode,
    // and it does so on the wire before the ownership is given up.
    assert!(output[0]
        .iter()
        .any(|(_, event)| matches!(event, Event::Note { kind: 1 | 2, key: 50, .. })));
    for _ in 0..8 {
        phrase.idle();
    }
    assert_eq!(phrase.sources[0].source_snapshot().held, 0);
    assert_eq!(inspect_source(&phrase.sources[0], |source| source.state.count()), 0);
    phrase.release_all();
}

/// The Hub sequences later input against the membership the toggle
/// established, so nothing the toggle ended is still scoring. Two ways that
/// state goes obsolete: an Off voice still sounding when Participating is
/// selected -- it would suddenly become context for everyone -- and an onset
/// cancelled by the marker standing at its own sample.
#[test]
fn production_musical_a_toggle_drops_the_context_of_what_it_ended() {
    let _scope = crate::test_scope::enter();
    let mut phrase = Phrase::new();
    let off = phrase.sources[0].participation(false, 0);
    phrase.step([vec![off], vec![], vec![]], [0, 1, 2]);
    phrase.idle();
    phrase.step([vec![note(1, 0, 50, 0, true)], vec![], vec![]], [0, 1, 2]);
    phrase.idle();
    assert_eq!(phrase.sources[0].source_snapshot().held, 1, "an Off D is on the wire");
    // Returning to Participating ends that D. It must not spend the callbacks
    // its release takes as tuning context for a track that is participating.
    let on = phrase.sources[0].participation(true, 0);
    phrase.step([vec![on], vec![note(2, 0, 52, 0, true)], vec![]], [1, 2, 0]);
    phrase.idle();
    assert_eq!(
        phrase.voice(1, 52, 0).attack_node,
        Some(LatticePos::new(0, 1, 0)),
        "the ended Off D is not context: empty context prefers 5/4, not 81/64"
    );
    phrase.step([vec![], vec![note(2, 0, 52, 0, false)], vec![]], [0, 1, 2]);
    for _ in 0..8 {
        phrase.idle();
    }
    // The other way: an onset the marker cancels at its own sample. It never
    // sounds, so it is never anyone's context either.
    let off = phrase.sources[0].participation(false, 0);
    phrase.step(
        [vec![note(3, 0, 50, 0, true), off], vec![], vec![note(4, 0, 52, 0, true)]],
        [2, 1, 0],
    );
    phrase.idle();
    assert_eq!(
        phrase.voice(2, 52, 0).attack_node,
        Some(LatticePos::new(0, 1, 0)),
        "the cancelled D is not context either"
    );
    assert_eq!(phrase.sources[0].source_snapshot().held, 0, "and it never sounded");
    phrase.release_all();
}

#[test]
fn production_musical_normal_phrase_preserves_bends_onset_context_and_take_metadata() {
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
            assert_eq!(now.onset_pitch_microcents, old.onset_pitch_microcents);
            let bend = if source == 0 && old.note == 48 { 700_000_000 } else { 0 };
            assert_eq!(now.pitch_microcents, old.pitch_microcents + bend);
        }
    }
    let wrapper = unsafe {
        &*((*phrase.hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    let config = wrapper
        .test_inspect_plugin(|plugin| plugin.configuration.as_ref().unwrap().reducer.resolved());
    let snapshot = inspect_hub(&phrase.hub, |hub| hub.test_next_context());
    assert_eq!(snapshot.config, config.into());
    assert!(snapshot.context.iter().all(|context| current
        .iter()
        .flatten()
        .any(|v| v.onset_pitch_microcents == context.pitch)));
    let key = 66;
    let expected = harmonigraph_core::policy::assign_new_note(
        config.into(),
        &snapshot.context,
        snapshot.reference,
        harmonigraph_core::policy::OrderedOnset { pitch: i64::from(key) * 100_000_000 },
        &mut Default::default(),
    )
    .unwrap();
    phrase.step([vec![], vec![], vec![note(15, 0, i16::from(key), 0, true)]], [0, 1, 2]);
    phrase.idle();
    let added = phrase.voice(2, key, 0);
    assert_ne!(added.assignment.unwrap().revision, original[0][0].assignment.unwrap().revision);
    let harmonigraph_core::policy::Assignment::Selected { node, .. } = expected.assignment else {
        panic!("selected candidate")
    };
    assert_eq!(added.attack_node, Some(node));
    assert_eq!(added.frozen_offset_microcents, expected.assignment.correction_microcents());
    writer.drain(&mut capture);
    let display = capture.display_events();
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
    assert!(take.configurations.iter().all(|c| c.policy.version == 2));
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
fn production_musical_configuration_edits_and_default_stop_preserve_context() {
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
            // Stop terminates this held D downstream. It is not one of the
            // Hub's history boundaries -- those are a configuration revision,
            // a participation toggle, a re-pairing and a clock reset -- so the
            // prospective spelling of a key survives it.
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
        let node = LatticePos::new(0, 1, 0);
        assert_eq!(
            phrase.voice(2, 52, 0).attack_node,
            Some(node),
            "stop={stop}: neither event resets the moving musical context by default"
        );
        phrase.release_all();
    }
}

/// #712: Off "excludes the track from adaptive context and visualization". The
/// context half is 4C's refusal in the Hub's `apply`; this is the other half.
/// Publication runs off the accepted-output lane, which is the same lane in
/// both modes, so nothing but an explicit gate keeps an Off track out of the
/// display ring and the take.
#[test]
fn production_musical_off_is_excluded_from_display_and_the_take() {
    use harmonigraph_take::CanonicalRecord;
    let _scope = crate::test_scope::enter();
    let (recorder, mut capture) = harmonigraph_record::testing::channel();
    crate::configuration::inject_recorder(recorder);
    let mut phrase = Phrase::new();
    capture.arm();
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-off-display-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("off.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    let identity = |index: usize| {
        inspect_source(&phrase.sources[index], |source| source.offer.as_ref().unwrap().lease.source)
    };
    let (quiet, shown) = (identity(0).0, identity(1).0);
    let off = phrase.sources[0].participation(false, 0);
    phrase.step([vec![off], vec![], vec![]], [0, 1, 2]);
    for _ in 0..4 {
        phrase.idle();
    }
    phrase.step([vec![note(1, 0, 50, 0, true)], vec![note(2, 0, 57, 0, true)], vec![]], [0, 1, 2]);
    for _ in 0..8 {
        phrase.idle();
    }
    writer.drain(&mut capture);
    let display = capture.display_events();
    let deltas = |source: u64| {
        display
            .iter()
            .filter(
                |record| matches!(record, CanonicalRecord::Delta(d) if d.event.source == source),
            )
            .count()
    };
    println!("PROBE off deltas quiet={} shown={}", deltas(quiet), deltas(shown));
    assert_ne!(deltas(shown), 0, "the Participating track is published, or nothing is measured");
    assert_eq!(deltas(quiet), 0, "an Off track is not published to the display or the take");
    let mut live = harmonigraph_core::NoteTracker::new();
    for record in &display {
        record.apply(&mut live).unwrap();
    }
    assert_eq!(live.held_count(), 1, "and only the Participating note is shown");
    // The gate reads the row's mode, so the note this track has ALREADY shown
    // is the case that could strand it: the reset that ends the phrase emits
    // that note's release, and a release published under the new mode would be
    // dropped. Measured rather than assumed -- probes on both sites put the
    // marker and the release it arms at one sample (8704 here), and the Hub
    // merges accepted output before it sequences copied input, so the release
    // is published while the row is still Participating and the mode moves
    // after it. The delta assertion below is what holds that order; if it ever
    // inverts, the display would depend instead on the non-participating
    // baseline the toggle's `repair` owes.
    let off = phrase.sources[1].participation(false, 0);
    phrase.step([vec![], vec![off], vec![]], [0, 1, 2]);
    for _ in 0..8 {
        phrase.idle();
    }
    writer.drain(&mut capture);
    let after = capture.display_events();
    for record in &after {
        record.apply(&mut live).unwrap();
    }
    println!("PROBE off toggled held={} records={}", live.held_count(), after.len());
    assert!(
        after.iter().any(|record| matches!(record, CanonicalRecord::Delta(d)
            if d.event.source == shown
                && matches!(d.event.kind, harmonigraph_take::NoteKind::Off))),
        "the toggle's own release is published before the mode moves"
    );
    assert_eq!(live.held_count(), 0, "so the toggle strands nothing the track had shown");
    phrase.release_all();
    drop(writer);
    drop(phrase);
    std::fs::remove_dir_all(&directory).unwrap();
}

/// The third exclusion, and the one with nothing behind it until now: 4D
/// measured that `Hub::confirm`'s `row.participating` clause forced true
/// survives the whole suite, so an Off row contributing confirmed pitches to
/// learning was invisible. Learning is armed here and the Off track is bent
/// well off twelve-tone, so its contribution would be audible in the inferred
/// tuning rather than only in a count.
#[test]
fn production_musical_off_pitches_do_not_reach_learning() {
    let _scope = crate::test_scope::enter();
    let mut phrase = Phrase::new();
    let hub = unsafe {
        &*((*phrase.hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    hub.configuration_handle()
        .unwrap()
        .submit(crate::configuration::packet(ConfigEdit {
            learning: Some(true),
            ..Default::default()
        }))
        .unwrap();
    phrase.idle();
    let learning = || {
        hub.test_inspect_plugin(|plugin| {
            let owner = plugin.configuration.as_ref().unwrap();
            let rows: Vec<_> =
                owner.confirmed.rows().map(|row| (row.key.source, row.pitch_microcents)).collect();
            (rows, owner.reducer.resolved().tuning)
        })
    };
    let identity = |index: usize| {
        inspect_source(&phrase.sources[index], |source| source.offer.as_ref().unwrap().lease.source)
    };
    let (quiet, shown) = (identity(0), identity(1));
    let (_, original) = learning();
    let off = phrase.sources[0].participation(false, 0);
    phrase.step([vec![off], vec![], vec![]], [0, 1, 2]);
    for _ in 0..4 {
        phrase.idle();
    }
    // The only Participating note is a C, which is one class and therefore no
    // interval at all: the learner reads no fifth from it and leaves the axis
    // alone. Off forwards the player's own pitch, so the G beside it is heard
    // twenty cents flat -- the ONLY fifth in the session, and the wrong one.
    // If that class counts, `three` moves from just to 680 cents.
    phrase.step(
        [
            vec![note(1, 0, 67, 0, true), expression(1, -0.2, 0)],
            vec![note(2, 0, 60, 0, true)],
            vec![],
        ],
        [0, 1, 2],
    );
    for _ in 0..8 {
        phrase.idle();
    }
    let (rows, learned) = learning();
    println!("PROBE learning rows={rows:?} tuning={learned:?}");
    assert_eq!(rows.len(), 1, "only the Participating track supplies a confirmed pitch");
    assert_eq!(rows[0], (shown, 6_000_000_000), "and it is the C, heard at C");
    assert_ne!(quiet, shown, "the two tracks are distinguishable, or the row above proves nothing");
    assert_eq!(learned, original, "so the flat Off G is no fifth: the learned axis holds");
    phrase.release_all();
}

/// #712, finding 2: `publication_free()` was the MINIMUM across both lanes, so
/// a display ring the editor had stopped draining refused a snapshot the TAKE
/// was waiting for. The harm is silent and lands in a recording whose own lane
/// is healthy: the file carries no gap and no warning, and simply keeps the
/// source's stale `participating: false`. Replaying it hides every note that
/// source played after the toggle -- a wrong video, drawn from a take that says
/// nothing is wrong.
///
/// Off first, so the toggle below has something to change; the display lane is
/// filled while the writer keeps up, so exactly one of the two lanes is short.
#[test]
fn a_full_display_lane_does_not_hold_back_the_takes_participation_snapshot() {
    use harmonigraph_take::CanonicalRecord;
    let _scope = crate::test_scope::enter();
    let (recorder, mut capture) = harmonigraph_record::testing::channel();
    crate::configuration::inject_recorder(recorder);
    let mut phrase = Phrase::new();
    capture.arm();
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-display-block-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("block.take");
    let mut writer = harmonigraph_record::testing::FileWriter::new(&capture, path.clone(), None);
    let toggled =
        inspect_source(&phrase.sources[0], |source| source.offer.as_ref().unwrap().lease.source).0;
    let off = phrase.sources[0].participation(false, 0);
    phrase.step([vec![off], vec![], vec![]], [0, 1, 2]);
    for _ in 0..4 {
        phrase.idle();
        writer.drain(&mut capture);
    }
    phrase.step([vec![note(1, 0, 50, 0, true)], vec![note(2, 0, 57, 0, true)], vec![]], [0, 1, 2]);
    for _ in 0..8 {
        phrase.idle();
        writer.drain(&mut capture);
    }
    // The writer keeps up; the editor has stopped. Only the display ring fills.
    for _ in 0..80 {
        phrase.step(
            [vec![], (0..64).map(|t| expression(2, 0.1234567890123, t)).collect(), vec![]],
            [0, 1, 2],
        );
        writer.drain(&mut capture);
    }
    assert!(!writer.failed(), "the take lane kept up, so the take is healthy");
    let participating = |take: &harmonigraph_take::Take, source: u64| {
        take.events
            .iter()
            .filter_map(|record| match record {
                CanonicalRecord::Baseline(frame) => frame.baseline().ok(),
                _ => None,
            })
            .rfind(|frame| frame.source.0 == source)
            .map(|frame| frame.participating)
    };
    writer.drain(&mut capture);
    let before = harmonigraph_take::Take::read(&path).unwrap();
    assert_eq!(
        participating(&before, toggled),
        Some(false),
        "the fixture must reach the toggle with the take holding the Off state"
    );
    // Off -> Participating, with the display lane still full. Its notes reach
    // the take either way; what the take needs is the baseline that says the
    // source is no longer hidden.
    let on = phrase.sources[0].participation(true, 0);
    phrase.step([vec![on], vec![], vec![]], [0, 1, 2]);
    for _ in 0..12 {
        phrase.idle();
        writer.drain(&mut capture);
    }
    let after = harmonigraph_take::Take::read(&path).unwrap();
    assert!(after.incomplete.is_none(), "the take's own lane lost nothing");
    assert_eq!(
        participating(&after, toggled),
        Some(true),
        "the Participating snapshot reached the take through a full display lane"
    );
    phrase.release_all();
    drop(writer);
    drop(phrase);
    std::fs::remove_dir_all(&directory).unwrap();
}

/// #712, finding 3: a snapshot one lane accepted and the other refused as
/// `Busy` must not come back under an identity the accepting lane already has.
///
/// 5A judged this benign because "both consumers dedup on id" -- the dedup is
/// exactly what makes it harmful. The old code advanced the row's `baseline_id`
/// only when BOTH lanes took the frame, so once the display's 20-slot frame
/// FIFO filled, every later refresh was republished under the id the TAKE had
/// already taken, and the take's fanout dropped each one as a duplicate. The
/// file's last word about that source then stays stale for good, however many
/// refreshes the row goes on owing.
///
/// The fixture fills the display's frame FIFO with participation toggles --
/// each one arms a repair without touching the item ring, so the display can be
/// short of FRAMES while it still has room for items, which is the only way
/// `Busy` (rather than "skipped, no cells") is reachable at all.
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
    let toggled =
        inspect_source(&phrase.sources[0], |source| source.offer.as_ref().unwrap().lease.source).0;
    // Well past SNAPSHOT_SLOTS, so the display is refusing long before the end.
    const TOGGLES: usize = 30;
    for index in 0..TOGGLES {
        let value = index % 2 == 1;
        let event = phrase.sources[0].participation(value, 0);
        phrase.step([vec![event], vec![], vec![]], [0, 1, 2]);
        for _ in 0..6 {
            phrase.idle();
            writer.drain(&mut capture);
        }
    }
    writer.drain(&mut capture);
    let take = harmonigraph_take::Take::read(&path).unwrap();
    let of_source = |records: &[CanonicalRecord], source: u64| -> Vec<(u64, bool)> {
        records
            .iter()
            .filter_map(|record| match record {
                CanonicalRecord::Baseline(frame) => frame.baseline().ok(),
                _ => None,
            })
            .filter(|frame| frame.source.0 == source)
            .map(|frame| (frame.id, frame.participating))
            .collect()
    };
    let written = of_source(&take.events, toggled);
    let displayed = of_source(&capture.display_events(), toggled);
    println!("PROBE take={} display={}", written.len(), displayed.len());
    assert!(
        displayed.len() < TOGGLES,
        "the fixture must actually reach a refusal on the display: {} of {TOGGLES}",
        displayed.len()
    );
    assert!(
        written.len() >= TOGGLES,
        "every refresh the row owed reached the take: {} of {TOGGLES}",
        written.len()
    );
    assert!(
        written.windows(2).all(|pair| pair[1].0 > pair[0].0),
        "each one under a fresh identity, so none is deduplicated away"
    );
    assert_eq!(
        written.last().map(|(_, participating)| *participating),
        Some(TOGGLES.is_multiple_of(2)),
        "so the file's last word about the source is the current one"
    );
    phrase.release_all();
    drop(writer);
    drop(phrase);
    std::fs::remove_dir_all(&directory).unwrap();
}

/// The take's twin of what 5A gave the display: a gap clears EVERY source in
/// `NoteTracker`, so one lost report owes every source a fresh snapshot on the
/// lane that lost it -- not only the row whose report did not fit. The display
/// got that from `take_display_outage()`; the take lane had nothing, and a
/// replay of such a take would draw the other sources from state the gap had
/// already thrown away.
///
/// Here only the writer stops draining, so the take lane is the one that
/// overflows and the editor's copy stays whole.
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
    let sources: Vec<u64> = (0..3)
        .map(|index| {
            inspect_source(&phrase.sources[index], |source| {
                source.offer.as_ref().unwrap().lease.source
            })
            .0
        })
        .collect();
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
    for source in &sources {
        assert!(
            refreshed.contains(source),
            "source {source} owes the take a snapshot after the gap: got {refreshed:?}"
        );
    }
    phrase.release_all();
    drop(writer);
    drop(phrase);
    std::fs::remove_dir_all(&directory).unwrap();
}

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
        for _ in 0..8 {
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

pub(super) fn configure_policy(
    hub: &Device,
    policy: harmonigraph_core::configuration::PolicyConfig,
) {
    let wrapper = unsafe {
        &*((*hub.plugin)
            .plugin_data
            .cast::<nice_plug::wrapper::clap::Wrapper<crate::Harmonigraph>>())
    };
    wrapper
        .configuration_handle()
        .unwrap()
        .submit(crate::configuration::packet(ConfigEdit {
            policy: Some(policy),
            ..Default::default()
        }))
        .unwrap();
}
#[test]
fn production_silence_and_stop_reset_settings_clear_only_the_musical_memory() {
    let _scope = crate::test_scope::enter();
    for reset_stop in [false, true] {
        let mut phrase = Phrase::new();
        configure_policy(
            &phrase.hub,
            harmonigraph_core::configuration::PolicyConfig { reset_stop, ..Default::default() },
        );
        phrase.idle();
        phrase.step(
            [vec![note(1, 0, 48, 0, true), note(2, 0, 52, 1, true)], vec![], vec![]],
            [0, 1, 2],
        );
        for _ in 0..4 {
            phrase.idle();
        }
        phrase.step(std::array::from_fn(|_| vec![transport(0, 120.0)]), [0, 1, 2]);
        phrase.step(
            std::array::from_fn(|_| {
                let Input::Transport(mut value) = transport(0, 120.0) else { unreachable!() };
                value.flags &= !CLAP_TRANSPORT_IS_PLAYING;
                vec![Input::Transport(value)]
            }),
            [0, 1, 2],
        );
        for _ in 0..24 {
            phrase.idle();
        }
        assert!(phrase.sources.iter().all(|s| s.source_snapshot().held == 0));
        let remembered = inspect_hub(&phrase.hub, |h| h.test_next_context());
        assert_eq!(remembered.context.is_empty(), reset_stop);
        assert_eq!(remembered.reference == 0, reset_stop);
        if !reset_stop {
            configure_policy(
                &phrase.hub,
                harmonigraph_core::configuration::PolicyConfig {
                    silence_ms: 500,
                    ..Default::default()
                },
            );
            for _ in 0..64 {
                phrase.idle();
            }
            let expired = inspect_hub(&phrase.hub, |h| h.test_next_context());
            assert!(expired.context.is_empty());
            assert_eq!(expired.reference, 0);
        }
    }
}

#[test]
fn production_loop_reset_survives_a_committed_reset_and_lower_host_clock() {
    let _scope = crate::test_scope::enter();
    let position = |seconds: i64| {
        let Input::Transport(mut value) = transport(0, 120.0) else { unreachable!() };
        value.flags |= CLAP_TRANSPORT_HAS_SECONDS_TIMELINE;
        value.song_pos_seconds = seconds * (1i64 << 31);
        Input::Transport(value)
    };
    let chord = |phrase: &mut Phrase, key: i16| {
        phrase.step(
            [
                vec![
                    note(1, 0, key, 0, true),
                    note(2, 0, key + 4, 1, true),
                    note(3, 0, key + 7, 2, true),
                ],
                vec![],
                vec![],
            ],
            [0, 1, 2],
        );
        for _ in 0..4 {
            phrase.idle();
        }
        let node = phrase.voice(0, key as u8, 0).attack_node;
        phrase.release_all();
        node
    };
    let uuid = SavedUuid::default();
    let mut hub = Device::new(false);
    hub.configure_format(uuid, true, Calibration { offset: 0 });
    hub.activate_format(44100.0, 512);
    configure(&hub, Tuning::just());
    configure_policy(
        &hub,
        harmonigraph_core::configuration::PolicyConfig { reset_loop: true, ..Default::default() },
    );
    // DIRECT shares the musical reset frontier. Seed its high-raw loop before
    // any Tunes are paired, so an idle CLAP Reset can commit synchronously.
    hub.run_format(1_000_000, vec![position(10)], None, None, 512);
    hub.run_format(1_000_512, vec![position(1), note(1, 0, 48, 1, true)], None, None, 512);
    hub.run_format(1_001_024, vec![note(1, 0, 48, 0, false)], None, None, 512);
    for step in 3..16 {
        hub.run_format(1_000_000 + step * 512, vec![], None, None, 512);
    }
    let before = inspect_hub(&hub, |h| h.direct.test_snapshot().epoch);
    assert!(inspect_hub(&hub, |h| h.direct.settled()));
    unsafe { (*hub.plugin).reset.unwrap()(hub.plugin) };
    assert!(inspect_hub(&hub, |h| h.direct.test_snapshot().epoch) > before);
    let sources = std::array::from_fn(|_| {
        let mut source = Device::new(true);
        source.configure_format(uuid, true, Calibration { offset: 0 });
        source.activate_format(44100.0, 512);
        source
    });
    let mut phrase = Phrase { uuid, hub, sources, raw: 0, maximum: [0; 4] };
    phrase.step(
        std::array::from_fn(|_| [64, 66, 69].map(|cc| raw_midi([0xb0, cc, 0], 0)).into()),
        [0, 1, 2],
    );
    for _ in 0..96 {
        for source in &phrase.sources {
            source.main();
        }
        phrase.hub.main();
        phrase.idle();
    }
    assert!(phrase.sources.iter().all(|source| source.source_snapshot().epoch > before));
    for root in [48, 52, 56] {
        chord(&mut phrase, root);
    }
    assert!(phrase.raw < 1_000_000, "the second loop must have a lower raw identity");
    phrase.step(std::array::from_fn(|_| vec![position(10)]), [0, 1, 2]);
    phrase.step(std::array::from_fn(|_| vec![position(1)]), [2, 1, 0]);
    assert_eq!(
        chord(&mut phrase, 48),
        Some(LatticePos::new(0, 0, 0)),
        "the lower-raw loop clears new memory"
    );
    for root in [52, 56] {
        chord(&mut phrase, root);
    }
    phrase.step([vec![], vec![note(5, 0, 48, 0, true)], vec![]], [1, 0, 2]);
    for _ in 0..4 {
        phrase.idle();
    }
    assert_eq!(
        phrase.voice(1, 48, 0).attack_node,
        Some(LatticePos::new(0, 3, 0)),
        "another source must not repeat that reset"
    );
    phrase.release_all();
}

#[test]
fn production_loop_reset_is_applied_once_across_sources_before_the_next_attack() {
    let _scope = crate::test_scope::enter();
    let position = |seconds: i64| {
        let Input::Transport(mut value) = transport(0, 120.0) else { unreachable!() };
        value.flags |= CLAP_TRANSPORT_HAS_SECONDS_TIMELINE;
        value.song_pos_seconds = seconds * (1i64 << 31);
        Input::Transport(value)
    };
    for (reset_loop, offsets, epoch_change) in [
        (false, [0; 3], false),
        (true, [0; 3], false),
        (true, [0, 64, -32], false),
        (true, [0, 64, -32], true),
    ] {
        let mut phrase = Phrase::with_calibration(offsets);
        configure_policy(
            &phrase.hub,
            harmonigraph_core::configuration::PolicyConfig { reset_loop, ..Default::default() },
        );
        phrase.idle();
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
            for _ in 0..4 {
                phrase.idle();
            }
            phrase.release_all();
        }
        phrase.step(std::array::from_fn(|_| vec![position(10)]), [0, 1, 2]);
        phrase.step(
            [
                vec![
                    position(1),
                    note(4, 0, 48, 1, true),
                    note(9, 0, 52, 2, true),
                    note(10, 0, 55, 3, true),
                ],
                vec![position(1)],
                vec![position(1)],
            ],
            [2, 1, 0],
        );
        for _ in 0..4 {
            phrase.idle();
        }
        let root = if reset_loop { 0 } else { 3 };
        assert_eq!(phrase.voice(0, 48, 0).attack_node, Some(LatticePos::new(0, root, 0)));
        phrase.release_all();
        if epoch_change {
            let before = phrase.sources[1].source_snapshot().epoch;
            phrase.sources[1]
                .shared()
                .apply(
                    setup::Routing::Source(SourceSetup {
                        selected: Some(phrase.uuid),
                        calibration: Calibration { offset: 128 },
                    }),
                    false,
                )
                .unwrap();
            phrase
                .hub
                .shared()
                .apply(
                    setup::Routing::Hub(HubSetup {
                        uuid: phrase.uuid,
                        calibration: Calibration { offset: 64 },
                    }),
                    false,
                )
                .unwrap();
            for _ in 0..96 {
                phrase.idle();
                for source in &phrase.sources {
                    source.main();
                }
                phrase.hub.main();
            }
            assert!(phrase.sources[1].source_snapshot().epoch > before);
            assert_eq!(inspect_source(&phrase.sources[1], |s| s.clock.calibration.offset), 128);
        }
        // Move again before this source's first post-loop attack: an incorrect
        // second reset would otherwise be hidden by the still-sounding C.
        for key in [52, 56] {
            phrase.step(
                [
                    vec![
                        note(6, 0, key, 0, true),
                        note(7, 0, key + 4, 1, true),
                        note(8, 0, key + 7, 2, true),
                    ],
                    vec![],
                    vec![],
                ],
                [0, 1, 2],
            );
            for _ in 0..4 {
                phrase.idle();
            }
            phrase.release_all();
        }
        phrase.step([vec![], vec![note(5, 0, 48, 0, true)], vec![]], [1, 0, 2]);
        for _ in 0..4 {
            phrase.idle();
        }
        assert_eq!(
            phrase.voice(1, 48, 0).attack_node,
            Some(LatticePos::new(0, root + 3, 0)),
            "offsets={offsets:?}, epoch_change={epoch_change}"
        );
        phrase.release_all();
    }
}
