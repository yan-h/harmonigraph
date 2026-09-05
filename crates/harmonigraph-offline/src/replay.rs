//! Turning a recorded take back into the state a frame is drawn from.
//!
//! The contract that makes this work is that the display is a pure
//! function of `(events so far, parameter values, view/camera, now)`.
//! So a replay is just: advance `now` in exact steps, feed everything
//! that happened before it, and draw. No wall clock is consulted, which
//! is precisely why the output is constant-frame-rate and reproducible.
//!
//! Note events keep **their own** timestamps rather than being snapped to
//! the frame they are delivered on — the tracker derives envelopes from
//! those, so a note that started a third of the way through a frame still
//! fades from exactly there. The replay is sub-frame accurate even though
//! the frames are not.

use std::cell::Cell;

use harmonigraph_take::Take;
use harmonigraph_ui::params::{ParamBackend, ParamKey};
use harmonigraph_ui::SharedState;

/// The parameter values at one instant of the replay.
///
/// A [`ParamBackend`] whose values are set by the replay rather than by
/// the UI. Writes are accepted and kept (nothing offline should be
/// writing, but learn mode's write path exists and must not panic) —
/// they are simply overwritten by the next automation record.
pub struct ReplayParams {
    values: [Cell<f32>; ParamKey::ALL.len()],
}

impl Default for ReplayParams {
    fn default() -> Self {
        ReplayParams {
            values: std::array::from_fn(|i| Cell::new(ParamKey::ALL[i].default_value())),
        }
    }
}

impl ReplayParams {
    fn index(key: ParamKey) -> usize {
        ParamKey::ALL.iter().position(|k| *k == key).expect("ParamKey::ALL is exhaustive")
    }
}

impl ParamBackend for ReplayParams {
    fn get(&self, key: ParamKey) -> f32 {
        self.values[ReplayParams::index(key)].get()
    }

    fn set(&self, key: ParamKey, value: f32) {
        self.values[ReplayParams::index(key)].set(value);
    }

    fn begin_set(&self, _key: ParamKey) {}

    fn end_set(&self, _key: ParamKey) {}
}

/// Walks a take forward in time, feeding a [`SharedState`].
pub struct Replay {
    take: Take,
    pub params: ReplayParams,
    /// Index of the first note not yet delivered.
    next_note: usize,
    /// Index of the first parameter change not yet applied.
    next_param: usize,
    next_configuration: usize,
}

impl Replay {
    pub fn new(take: Take) -> Replay {
        Replay {
            take,
            params: ReplayParams::default(),
            next_note: 0,
            next_param: 0,
            next_configuration: 0,
        }
    }

    pub fn take(&self) -> &Take {
        &self.take
    }

    /// Deliver everything that happens at or before `now` and has not
    /// been delivered yet. Call once per frame with a strictly increasing
    /// `now`; seeking backwards is not supported (use [`Replay::new`]).
    pub fn advance_to(&mut self, state: &mut SharedState, now: f64) {
        while let Some(record) = self.take.configurations.get(self.next_configuration) {
            if record.t > now {
                break;
            }
            state.replayed_configuration = Some(record.resolved());
            self.next_configuration += 1;
        }

        while let Some(record) = self.take.events.get(self.next_note) {
            if record.time() > now {
                break;
            }
            record.apply(&mut state.tracker).expect("validated canonical take");
            self.next_note += 1;
        }

        while let Some(record) = self.take.params.get(self.next_param) {
            if record.t > now {
                break;
            }
            // A take written by a newer build can name a parameter this
            // one has never heard of; skipping it beats refusing to
            // render the whole piece.
            if let Some(key) = ParamKey::from_id(&record.id) {
                self.params.set(key, record.value);
            }
            self.next_param += 1;
        }
    }

    /// A [`harmonigraph_core::NoteRoll`] holding EVERY note in the take, laid out
    /// from the start — for the whole-song render, where the roll shows the
    /// whole piece at once rather than filling in up to `now`. Pitch comes from
    /// the notes and their bends, exactly as the live tracker builds it.
    pub fn full_roll(&self) -> harmonigraph_core::NoteRoll {
        let mut tracker = harmonigraph_core::NoteTracker::new();
        for record in &self.take.events {
            record.apply(&mut tracker).expect("validated canonical take");
        }
        tracker.roll().clone()
    }

    /// Whether every recorded event has been delivered.
    pub fn is_spent(&self) -> bool {
        self.next_note == self.take.events.len()
            && self.next_param == self.take.params.len()
            && self.next_configuration == self.take.configurations.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_render::wgpu::TextureFormat;
    use harmonigraph_take::{Header, NoteKind, NoteRecord, ParamRecord};

    fn take_with(notes: Vec<NoteRecord>, params: Vec<ParamRecord>) -> Take {
        Take {
            header: Header::default(),
            events: notes.into_iter().map(harmonigraph_take::CanonicalRecord::Note).collect(),
            params,
            configurations: Vec::new(),
            truncated: false,
            incomplete: None,
        }
    }

    fn on(t: f64, note: u8) -> NoteRecord {
        NoteRecord { source: 0, t, channel: 0, note, kind: NoteKind::On { velocity: 0.8 } }
    }

    fn off(t: f64, note: u8) -> NoteRecord {
        NoteRecord { source: 0, t, channel: 0, note, kind: NoteKind::Off }
    }

    #[test]
    fn accepted_off_after_gap_is_retained_without_a_held_baseline() {
        use harmonigraph_core::canonical::*;
        use harmonigraph_core::confirmed::PitchProvenance;
        use harmonigraph_core::{NoteEvent, SourceId};
        use harmonigraph_take::{CanonicalRecord, Record};
        let source = SourceId(1);
        let delta = |event: NoteEvent, sequence| {
            CanonicalRecord::from_event(CanonicalEvent::Note(NoteDelta {
                event,
                sequence,
                lifetime: 61,
                provenance: PitchProvenance::AcceptedOutput,
                timing: Some(EventTiming {
                    clock: ClockId::default(),
                    input: 0,
                    planned: None,
                    sample: (event.time * 48000.0) as i64,
                    sample_rate: 48000.0,
                }),
                pitch_microcents: None,
            }))
        };
        let empty = SourceBaseline::new(
            source,
            1,
            5.0,
            5.0,
            4,
            true,
            &[],
            [ChannelBaseline::default(); 16],
        )
        .unwrap();
        let mut text = ron::to_string(&Record::Header(Header::default())).unwrap();
        for record in [
            delta(NoteEvent::on(1.0, source, 0, 60, 0.8), 1),
            CanonicalRecord::from_event(CanonicalEvent::Gap(PublicationGap {
                source: Some(source),
                time: 2.0,
                through: 3.0,
                first: 2,
                last: 3,
                reason: GapReason::PublicationFull,
            })),
            delta(NoteEvent::off(4.0, source, 0, 60), 4),
            CanonicalRecord::from_event(CanonicalEvent::Baseline(&empty)),
        ] {
            text.push('\n');
            text.push_str(&ron::to_string(&Record::Canonical(record)).unwrap());
        }
        let take = Take::parse(std::io::Cursor::new(text)).unwrap();
        for cadence in [0.001, 1.0 / 24.0, 1.0 / 60.0] {
            let mut replay = Replay::new(take.clone());
            let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
            let mut now = 0.0;
            while now < 4.5 {
                replay.advance_to(&mut state, now);
                now += cadence;
            }
            assert!(
                !state.tracker.source_current_certain(source),
                "actual Off does not repair source completeness"
            );
            replay.advance_to(&mut state, 5.0);
            for roll in [state.tracker.roll(), &replay.full_roll()] {
                let note = roll.notes().next().unwrap();
                assert_eq!(
                    (note.start, note.end, note.observed_until),
                    (1.0, Some(4.0), Some(2.0))
                );
                assert!(note.segments(5.0).all(|segment| segment.1 .0 <= 2.0));
                assert_eq!(roll.notes().count(), 1);
            }
        }
    }

    #[test]
    fn visibility_boundaries_keep_pitch_history_independent_of_prune_cadence() {
        use harmonigraph_core::canonical::*;
        use harmonigraph_core::{NoteEvent, SourceId};
        use harmonigraph_take::{CanonicalRecord, Record};
        let source = SourceId::DIRECT;
        let off = SourceBaseline::new(
            source,
            1,
            0.03,
            0.0,
            0,
            false,
            &[],
            [ChannelBaseline::default(); 16],
        )
        .unwrap();
        let rejoin = SourceBaseline::new(
            source,
            2,
            0.08,
            0.0,
            0,
            true,
            &[],
            [ChannelBaseline::default(); 16],
        )
        .unwrap();
        let mut text = ron::to_string(&Record::Header(Header::default())).unwrap();
        for record in [
            CanonicalRecord::from_event(CanonicalEvent::Note(
                NoteEvent::on(0.01, source, 0, 60, 0.8).into(),
            )),
            CanonicalRecord::from_event(CanonicalEvent::Note(
                NoteEvent::off(0.02, source, 0, 60).into(),
            )),
            CanonicalRecord::from_event(CanonicalEvent::Baseline(&off)),
            CanonicalRecord::from_event(CanonicalEvent::Baseline(&rejoin)),
        ] {
            text.push('\n');
            text.push_str(&ron::to_string(&Record::Canonical(record)).unwrap());
        }
        let take = Take::parse(std::io::Cursor::new(text)).unwrap();
        for cadence in [0.001, 1.0 / 24.0, 1.0 / 60.0] {
            let mut replay = Replay::new(take.clone());
            let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
            let envelope = harmonigraph_core::Envelope {
                attack_time: 0.0,
                fade_time: 0.0,
                ..Default::default()
            };
            let mut now = 0.0;
            while now < 0.1 {
                replay.advance_to(&mut state, now);
                state.tracker.prune(now, &envelope);
                now += cadence;
            }
            replay.advance_to(&mut state, 0.1);
            state.tracker.prune(0.1, &envelope);
            let history: Vec<_> =
                state.tracker.history().visits().map(|v| (v.pitch, v.last_off)).collect();
            assert_eq!(history, [(60.0, 0.02)], "cadence {cadence}");
        }
    }

    #[test]
    fn canonical_serialization_has_one_history_at_every_replay_cadence() {
        use harmonigraph_core::canonical::*;
        use harmonigraph_core::confirmed::PitchProvenance;
        use harmonigraph_core::{NoteEvent, NoteEventKind, SourceId};
        use harmonigraph_take::{CanonicalRecord, Record};
        let source = SourceId(1);
        let delta = |event: NoteEvent, sequence| {
            CanonicalRecord::from_event(CanonicalEvent::Note(NoteDelta {
                event,
                sequence,
                lifetime: 61,
                provenance: PitchProvenance::AcceptedOutput,
                timing: Some(EventTiming {
                    clock: ClockId { runtime_session: 7, epoch: 3 },
                    input: (event.time * 48000.0) as i64 - 32,
                    planned: None,
                    sample: (event.time * 48000.0) as i64,
                    sample_rate: 48000.0,
                }),
                pitch_microcents: None,
            }))
        };
        let first = delta(NoteEvent::on(0.01, source, 0, 60, 0.8), 1);
        let empty = SourceBaseline::new(
            source,
            1,
            0.08,
            0.0,
            3,
            true,
            &[],
            [ChannelBaseline::default(); 16],
        )
        .unwrap();
        let row = VoiceBaseline {
            note: 60,
            lifetime: 61,
            input_onset: 0.09,
            actual_onset: 0.1,
            onset: Some(EventTiming {
                clock: ClockId { runtime_session: 7, epoch: 3 },
                input: 4320,
                planned: None,
                sample: 4800,
                sample_rate: 48000.0,
            }),
            pitch_microcents: 6_025_000_000,
            velocity: 0.8,
            provenance: PitchProvenance::AcceptedOutput,
            ..Default::default()
        };
        let held = SourceBaseline::new(
            source,
            2,
            0.15,
            0.08,
            5,
            true,
            &[row],
            [ChannelBaseline::default(); 16],
        )
        .unwrap();
        let recovered = SourceBaseline::new(
            source,
            3,
            0.3,
            0.3,
            8,
            true,
            &[row],
            [ChannelBaseline::default(); 16],
        )
        .unwrap();
        let events = [
            first.clone(),
            first,
            delta(
                NoteEvent {
                    time: 0.02,
                    source,
                    channel: 0,
                    note: 60,
                    kind: NoteEventKind::Tuning { semitones: 0.25 },
                },
                2,
            ),
            delta(NoteEvent::off(0.07, source, 0, 60), 3),
            CanonicalRecord::from_event(CanonicalEvent::Baseline(&empty)),
            delta(NoteEvent::on(0.1, source, 0, 60, 0.8), 4),
            delta(
                NoteEvent {
                    time: 0.11,
                    source,
                    channel: 0,
                    note: 60,
                    kind: NoteEventKind::Tuning { semitones: 0.25 },
                },
                5,
            ),
            CanonicalRecord::from_event(CanonicalEvent::Baseline(&held)),
            CanonicalRecord::from_event(CanonicalEvent::Gap(PublicationGap {
                source: Some(source),
                time: 0.2,
                through: 0.29,
                first: 6,
                last: 8,
                reason: GapReason::PublicationFull,
            })),
            CanonicalRecord::from_event(CanonicalEvent::Baseline(&recovered)),
            delta(NoteEvent::off(0.4, source, 0, 60), 9),
        ];
        let mut encoded = ron::to_string(&Record::Header(Header::default())).unwrap();
        for event in events {
            encoded.push('\n');
            encoded.push_str(&ron::to_string(&Record::Canonical(event)).unwrap());
        }
        let take = Take::parse(std::io::Cursor::new(encoded)).unwrap();
        assert!(take.incomplete.is_some());
        let roll_snapshot = |roll: &harmonigraph_core::NoteRoll| {
            roll.notes()
                .map(|n| {
                    (
                        n.source,
                        n.lifetime,
                        n.start,
                        n.end,
                        n.observed_until,
                        n.history_complete,
                        n.settled_pitch(),
                        n.segments(0.5).collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut expected = None;
        for cadence in [0.001, 1.0 / 24.0, 1.0 / 60.0] {
            let mut replay = Replay::new(take.clone());
            let mut state = SharedState::new(TextureFormat::Bgra8Unorm);
            let mut now = 0.0;
            while now < 0.5 {
                replay.advance_to(&mut state, now);
                now += cadence;
            }
            replay.advance_to(&mut state, 0.5);
            assert!(replay.is_spent());
            let snapshot = roll_snapshot(state.tracker.roll());
            assert_eq!(snapshot, roll_snapshot(&replay.full_roll()));
            assert_eq!(snapshot.len(), 2);
            assert_eq!((snapshot[0].2, snapshot[0].3, snapshot[0].6), (0.01, Some(0.07), 60.25));
            assert_eq!(
                (snapshot[1].2, snapshot[1].3, snapshot[1].4, snapshot[1].5),
                (0.1, Some(0.4), None, false)
            );
            assert!(snapshot[1].7.iter().any(|segment| segment.0 .0 == 0.3));
            assert!(
                !snapshot[1].7.iter().any(|segment| segment.0 .0 < 0.3 && segment.1 .0 > 0.2),
                "one recovered lifetime preserves the gap without a duplicate cache identity"
            );
            assert_eq!(state.tracker.publication_gaps().len(), 1);
            assert_eq!(state.tracker.source_baseline(source).unwrap(), &recovered);
            if let Some(ref expected) = expected {
                assert_eq!(&snapshot, expected);
            } else {
                expected = Some(snapshot);
            }
        }
    }

    #[test]
    fn resolved_configuration_round_trip_is_independent_of_replay_cadence() {
        use harmonigraph_core::configuration::{ConfigEdit, ConfigMutation, ConfigReducer};
        use harmonigraph_take::{ConfigurationRecord, Record};
        let mut reducer = ConfigReducer::default();
        let mut encoded = ron::to_string(&Record::Header(Header::default())).unwrap();
        let mut expected = Vec::new();
        for (t, edit) in [
            (0.0, ConfigEdit { learning: Some(true), ..Default::default() }),
            (0.013, ConfigEdit::unlock(harmonigraph_core::Comma::Syntonic, 390_000_000)),
            (0.031, ConfigEdit::axis(1, 696_000_000)),
        ] {
            reducer.apply(ConfigMutation::Edit(edit));
            let config = reducer.resolved();
            expected.push((t, config));
            encoded.push('\n');
            encoded.push_str(
                &ron::to_string(&Record::Configuration(ConfigurationRecord::new(t, config)))
                    .unwrap(),
            );
        }
        // A held chord and contradictory raw lane would trigger GUI learning or
        // detection if replay accidentally treated these exact records as hints.
        for record in [
            Record::Note(on(0.0, 60)),
            Record::Note(on(0.0, 64)),
            Record::Param(ParamRecord { t: 0.0, id: ParamKey::Three.id().into(), value: 710.0 }),
        ] {
            encoded.push('\n');
            encoded.push_str(&ron::to_string(&record).unwrap());
        }
        for cadence in [0.001, 1.0 / 24.0, 1.0 / 60.0] {
            let take = Take::parse(std::io::Cursor::new(&encoded)).unwrap();
            assert!(!take.truncated);
            let mut replay = Replay::new(take);
            let mut state = SharedState::new(TextureFormat::Rgba8Unorm);
            for frame in 0..=100 {
                let now = f64::from(frame) * cadence;
                replay.advance_to(&mut state, now);
                harmonigraph_ui::begin_frame(&mut state, &replay.params, now);
                let config = expected.iter().rev().find(|(t, _)| *t <= now).unwrap().1;
                assert_eq!(state.tuning, config.tuning);
                assert_eq!(state.view.meantone, config.modes.tempered.syntonic);
                assert_eq!(state.learn_active, config.modes.learning);
                assert_eq!(state.replayed_configuration, Some(config));
            }
        }
    }

    #[test]
    fn events_arrive_on_the_frame_that_passes_them_and_not_before() {
        let mut state = SharedState::new(TextureFormat::Rgba8Unorm);
        let mut replay = Replay::new(take_with(vec![on(0.5, 60), on(1.5, 64)], vec![]));

        replay.advance_to(&mut state, 0.25);
        assert_eq!(state.tracker.held_count(), 0, "not yet");
        replay.advance_to(&mut state, 0.75);
        assert_eq!(state.tracker.held_count(), 1);
        replay.advance_to(&mut state, 2.0);
        assert_eq!(state.tracker.held_count(), 2);
        assert!(replay.is_spent());
    }

    /// The reason a 60 fps replay isn't quantized to 60 fps: a voice
    /// keeps the timestamp it was recorded with, so its envelope starts
    /// where it actually started.
    #[test]
    fn a_note_keeps_its_own_timestamp_not_the_frames() {
        let mut state = SharedState::new(TextureFormat::Rgba8Unorm);
        let mut replay = Replay::new(take_with(vec![on(0.5, 60), off(0.6, 60)], vec![]));
        replay.advance_to(&mut state, 1.0);
        let voice = *state.tracker.voices().next().unwrap();
        assert_eq!(voice.on_time, 0.5);
        // Released at 0.6 with a 1 s straight-line fade: 40% gone by t=1.0.
        // The envelope is spelled out rather than taken from the state's view
        // because what is under test is the TIMESTAMP — reading it through
        // whatever curve the default view happens to carry would have this
        // fail the day that default is retuned, for no reason it names.
        let env = harmonigraph_core::Envelope { fade_time: 1.0, ..Default::default() };
        assert!((voice.activation(1.0, &env) - 0.6).abs() < 1e-6);
    }

    #[test]
    fn source_scopes_and_bends_round_trip_into_incremental_and_full_replay() {
        use harmonigraph_core::{NoteEvent, NoteEventKind, SourceId, VoiceState};
        let (a, b) = (SourceId(1), SourceId(2));
        let tuning = |time, source, semitones| NoteEvent {
            time,
            source,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Tuning { semitones },
        };
        let events = [
            NoteEvent::on(0.125, a, 0, 60, 0.8),
            tuning(0.125, a, 0.25),
            NoteEvent::on(0.25, b, 0, 60, 0.6),
            tuning(0.25, b, -0.25),
            NoteEvent::off(0.75, a, 0, 60),
            NoteEvent::on(1.0, a, 0, 60, 0.8),
            NoteEvent::on(1.25, a, 0, 60, 0.7),
            NoteEvent::source_reset(1.5, a),
            tuning(1.75, b, -0.5),
            NoteEvent::on(2.0, a, 0, 60, 0.9),
            NoteEvent::session_reset(2.25),
        ];
        let mut encoded =
            ron::to_string(&harmonigraph_take::Record::Header(Header::default())).unwrap();
        for event in events {
            encoded.push('\n');
            encoded
                .push_str(&ron::to_string(&harmonigraph_take::Record::Note(event.into())).unwrap());
        }
        let take = Take::parse(std::io::Cursor::new(encoded)).unwrap();
        assert!(!take.truncated);
        assert_eq!(take.notes().map(NoteEvent::from).collect::<Vec<_>>(), events);
        let mut replay = Replay::new(take);
        let mut state = SharedState::new(TextureFormat::Rgba8Unorm);
        replay.advance_to(&mut state, 0.5);
        assert_eq!(state.tracker.held_count(), 2);
        let pitches: Vec<_> = state.tracker.voices().map(|v| (v.source, v.pitch)).collect();
        assert_eq!(pitches, vec![(a, 60.25), (b, 59.75)]);
        replay.advance_to(&mut state, 1.9);
        let held: Vec<_> = state.tracker.voices().filter(|v| v.state == VoiceState::Held).collect();
        assert_eq!(held.len(), 1);
        assert_eq!((held[0].source, held[0].pitch, held[0].on_time), (b, 59.5, 0.25));
        replay.advance_to(&mut state, 3.0);
        assert_eq!(state.tracker.held_count(), 0);
        assert!(replay.is_spent());

        for roll in [state.tracker.roll(), &replay.full_roll()] {
            let notes: Vec<_> =
                roll.notes().map(|n| (n.source, n.start, n.end, n.end_pitch())).collect();
            assert_eq!(
                notes,
                vec![
                    (a, 0.125, Some(0.75), 60.25),
                    (a, 1.0, Some(1.25), 60.0),
                    (a, 1.25, Some(1.5), 60.0),
                    (a, 2.0, Some(2.25), 60.0),
                    (b, 0.25, Some(2.25), 59.5),
                ]
            );
            let note_b = roll.notes().find(|n| n.source == b).unwrap();
            assert_eq!(
                note_b.segments(3.0).collect::<Vec<_>>(),
                vec![((0.25, 59.75), (1.75, 59.5)), ((1.75, 59.5), (2.25, 59.5)),]
            );
        }
    }

    #[test]
    fn parameter_automation_is_applied_in_time_order() {
        let mut state = SharedState::new(TextureFormat::Rgba8Unorm);
        let id = ParamKey::Fade.id().to_string();
        // Two values a bar can actually reach, so what the test replays is a
        // lane that could have been recorded (`ParamKey::Fade.range()`).
        let mut replay = Replay::new(take_with(
            vec![],
            vec![
                ParamRecord { t: 0.0, id: id.clone(), value: 0.25 },
                ParamRecord { t: 1.0, id: id.clone(), value: 0.75 },
            ],
        ));
        replay.advance_to(&mut state, 0.5);
        assert_eq!(replay.params.get(ParamKey::Fade), 0.25);
        replay.advance_to(&mut state, 1.5);
        assert_eq!(replay.params.get(ParamKey::Fade), 0.75);
    }

    #[test]
    fn an_unknown_parameter_id_is_skipped_rather_than_fatal() {
        let mut state = SharedState::new(TextureFormat::Rgba8Unorm);
        let mut replay = Replay::new(take_with(
            vec![],
            vec![ParamRecord { t: 0.0, id: "invented-by-a-later-build".into(), value: 9.0 }],
        ));
        replay.advance_to(&mut state, 1.0);
        assert!(replay.is_spent());
        assert_eq!(replay.params.get(ParamKey::Fade), ParamKey::Fade.default_value());
    }

    #[test]
    fn params_start_at_their_defaults() {
        let params = ReplayParams::default();
        for key in ParamKey::ALL {
            assert_eq!(params.get(key), key.default_value());
        }
    }
}
