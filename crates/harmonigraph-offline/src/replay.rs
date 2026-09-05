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

        while let Some(record) = self.take.notes.get(self.next_note) {
            if record.t > now {
                break;
            }
            state.tracker.handle_event((*record).into());
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
        for record in &self.take.notes {
            tracker.handle_event((*record).into());
        }
        tracker.roll().clone()
    }

    /// Whether every recorded event has been delivered.
    pub fn is_spent(&self) -> bool {
        self.next_note == self.take.notes.len()
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
            notes,
            params,
            configurations: Vec::new(),
            truncated: false,
        }
    }

    fn on(t: f64, note: u8) -> NoteRecord {
        NoteRecord { source: 0, t, channel: 0, note, kind: NoteKind::On { velocity: 0.8 } }
    }

    fn off(t: f64, note: u8) -> NoteRecord {
        NoteRecord { source: 0, t, channel: 0, note, kind: NoteKind::Off }
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
        assert_eq!(take.notes.iter().copied().map(NoteEvent::from).collect::<Vec<_>>(), events);
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
