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

use lattice_core::{NoteEvent, NoteEventKind};
use lattice_take::{NoteKind, Take};
use lattice_ui::params::{ParamBackend, ParamKey};
use lattice_ui::SharedState;

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
}

impl Replay {
    pub fn new(take: Take) -> Replay {
        Replay {
            take,
            params: ReplayParams::default(),
            next_note: 0,
            next_param: 0,
        }
    }

    pub fn take(&self) -> &Take {
        &self.take
    }

    /// Deliver everything that happens at or before `now` and has not
    /// been delivered yet. Call once per frame with a strictly increasing
    /// `now`; seeking backwards is not supported (use [`Replay::new`]).
    pub fn advance_to(&mut self, state: &mut SharedState, now: f64) {
        while let Some(record) = self.take.notes.get(self.next_note) {
            if record.t > now {
                break;
            }
            let kind = match record.kind {
                NoteKind::On { velocity } => NoteEventKind::On { velocity },
                NoteKind::Off => NoteEventKind::Off,
                NoteKind::Tuning { semitones } => NoteEventKind::Tuning { semitones },
                NoteKind::AllOff => NoteEventKind::AllOff,
            };
            state.tracker.handle_event(NoteEvent {
                time: record.t,
                channel: record.channel,
                note: record.note,
                kind,
            });
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

    /// A [`lattice_core::NoteRoll`] holding EVERY note in the take, laid out
    /// from the start — for the whole-song render, where the roll shows the
    /// whole piece at once rather than filling in up to `now`. Pitch comes from
    /// the notes and their bends, exactly as the live tracker builds it.
    pub fn full_roll(&self) -> lattice_core::NoteRoll {
        let mut tracker = lattice_core::NoteTracker::new();
        for record in &self.take.notes {
            let kind = match record.kind {
                NoteKind::On { velocity } => NoteEventKind::On { velocity },
                NoteKind::Off => NoteEventKind::Off,
                NoteKind::Tuning { semitones } => NoteEventKind::Tuning { semitones },
                NoteKind::AllOff => NoteEventKind::AllOff,
            };
            tracker.handle_event(NoteEvent {
                time: record.t,
                channel: record.channel,
                note: record.note,
                kind,
            });
        }
        tracker.roll().clone()
    }

    /// Whether every recorded event has been delivered.
    pub fn is_spent(&self) -> bool {
        self.next_note == self.take.notes.len() && self.next_param == self.take.params.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_take::{Header, NoteRecord, ParamRecord};
    use lattice_render::wgpu::TextureFormat;

    fn take_with(notes: Vec<NoteRecord>, params: Vec<ParamRecord>) -> Take {
        Take { header: Header::default(), notes, params, truncated: false }
    }

    fn on(t: f64, note: u8) -> NoteRecord {
        NoteRecord { t, channel: 0, note, kind: NoteKind::On { velocity: 0.8 } }
    }

    fn off(t: f64, note: u8) -> NoteRecord {
        NoteRecord { t, channel: 0, note, kind: NoteKind::Off }
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
        // Released at 0.6 with a 1 s fade: 0.4 of the fade gone by t=1.0,
        // which the eased envelope reads as 0.6² of the note left. Framed off
        // the release time, not the frame boundary — which is the point here.
        assert!((voice.activation(1.0, 1.0) - 0.36).abs() < 1e-6);
    }

    #[test]
    fn parameter_automation_is_applied_in_time_order() {
        let mut state = SharedState::new(TextureFormat::Rgba8Unorm);
        let id = ParamKey::Fade.id().to_string();
        let mut replay = Replay::new(take_with(
            vec![],
            vec![
                ParamRecord { t: 0.0, id: id.clone(), value: 1.0 },
                ParamRecord { t: 1.0, id: id.clone(), value: 4.0 },
            ],
        ));
        replay.advance_to(&mut state, 0.5);
        assert_eq!(replay.params.get(ParamKey::Fade), 1.0);
        replay.advance_to(&mut state, 1.5);
        assert_eq!(replay.params.get(ParamKey::Fade), 4.0);
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
