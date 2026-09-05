//! What has already been played: the pitches the
//! [`NoteTracker`](crate::NoteTracker) has finished drawing live.
//!
//! A voice lands here exactly when it stops being a voice — the moment its
//! release fade completes and the tracker drops it (see
//! [`NoteTracker::prune`](crate::NoteTracker::prune)). History and the live
//! voices therefore never describe the same note at the same time, and a
//! trail picks a note up precisely where its fade lets go.
//!
//! Deliberately thin: one entry per distinct pitch, holding only what a
//! mark on the lattice needs. The scene draws from this; nothing here knows
//! that it will be drawn.

use std::collections::BTreeMap;

use crate::notes::{Time, Voice, VoiceState};
use crate::tuning::PitchClass;

/// One pitch the music has been to.
#[derive(Copy, Clone, Debug)]
pub struct Visit {
    /// The sounding pitch in MIDI note units, from the most recent visit
    /// (per-note tuning included, so a bent voice is remembered bent).
    pub pitch: f32,
    pub pitch_class: PitchClass,
    /// When it last stopped sounding, for the optional memory span.
    pub last_off: Time,
}

/// Key a visit is folded under: the sounding pitch rounded to the cent.
/// Coarse enough that repeats of one note always land together, fine
/// enough that two microtonally distinct pitches stay distinct.
fn visit_key(pitch: f32) -> i32 {
    (pitch * 100.0).round() as i32
}

/// Every pitch played so far, as far back as [`MAX_VISITS`](Self::MAX_VISITS).
///
/// Ordered by key — the pitch in cents — for the reason
/// [`NoteTracker`](crate::NoteTracker) is: two remembered pitches can match
/// one node, and where they TIE the trail's `max` keeps the first of them, so
/// a hashed order would settle that differently per process (#135). The tie is
/// the ordinary case rather than a corner: `level` is 1.0 for every visit
/// while the memory span is "never forget", which is the default. Give the
/// span a value and the fresher visit simply wins on level, order or no order.
/// `forget_oldest` is the same argument for a tie in `last_off`, which a
/// chord's notes all share.
#[derive(Default)]
pub struct NoteHistory {
    visits: BTreeMap<i32, Visit>,
}

impl NoteHistory {
    /// Distinct pitches remembered. Past this the least recently played is
    /// forgotten. The bound is on per-frame work — the scene tests every
    /// visit against every visible node — not on memory.
    pub const MAX_VISITS: usize = 384;

    /// Fold a voice the tracker has finished with into history. `now` is
    /// only a fallback for the release time; a voice reaching here is
    /// always `Released`.
    pub fn record(&mut self, voice: &Voice, now: Time) {
        let last_off = match voice.state {
            VoiceState::Released { at } => at,
            VoiceState::Held => now,
        };
        let visit = self.visits.entry(visit_key(voice.pitch)).or_insert(Visit {
            pitch: voice.pitch,
            pitch_class: voice.pitch_class,
            last_off,
        });
        // The freshest visit owns the entry: two playings inside one cent fold
        // together (see `visit_key`), and the mark is drawn at — and colored
        // by — the pitch the last of them sounded at.
        visit.pitch = voice.pitch;
        visit.pitch_class = voice.pitch_class;
        visit.last_off = visit.last_off.max(last_off);

        if self.visits.len() > Self::MAX_VISITS {
            self.forget_oldest();
        }
    }

    /// Drop the least recently played pitch. O(visits), and only ever runs
    /// on the one insert that crosses the cap. A chord's notes share a
    /// `last_off`, so the tie `min_by` breaks by taking the first is a real
    /// case rather than a rounding accident — it resolves to the lowest
    /// pitch of the tied set.
    fn forget_oldest(&mut self) {
        let oldest = self
            .visits
            .iter()
            .min_by(|a, b| a.1.last_off.total_cmp(&b.1.last_off))
            .map(|(key, _)| *key);
        if let Some(key) = oldest {
            self.visits.remove(&key);
        }
    }

    /// Every remembered pitch, lowest first. Not the order they were played
    /// in — the key is the pitch — but a fixed one, which is what the trail
    /// needs of it.
    pub fn visits(&self) -> impl Iterator<Item = &Visit> {
        self.visits.values()
    }

    pub fn is_empty(&self) -> bool {
        self.visits.is_empty()
    }

    /// Forget everything played so far.
    pub fn clear(&mut self) {
        self.visits.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::{Envelope, NoteEvent, NoteEventKind, NoteTracker};

    fn on(time: Time, note: u8) -> NoteEvent {
        NoteEvent {
            source: crate::SourceId::DIRECT,
            time,
            channel: 0,
            note,
            kind: NoteEventKind::On { velocity: 0.8 },
        }
    }

    fn off(time: Time, note: u8) -> NoteEvent {
        NoteEvent {
            source: crate::SourceId::DIRECT,
            time,
            channel: 0,
            note,
            kind: NoteEventKind::Off,
        }
    }

    /// Play `note` and let its fade complete, which is what moves it into
    /// history.
    fn play(tracker: &mut NoteTracker, note: u8, on_time: Time, off_time: Time) {
        tracker.handle_event(on(on_time, note));
        tracker.handle_event(off(off_time, note));
        tracker.prune(off_time + 2.0, &Envelope::default());
    }

    #[test]
    fn a_note_enters_history_when_its_fade_completes_not_before() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(off(1.0, 60));
        // Mid-fade: still a live voice, not yet a memory.
        tracker.prune(1.5, &Envelope::default());
        assert_eq!(tracker.voices().count(), 1);
        assert!(tracker.history().is_empty());
        // The fade completes and the voice becomes history in one step.
        tracker.prune(2.5, &Envelope::default());
        assert_eq!(tracker.voices().count(), 0);
        assert_eq!(tracker.history().visits().count(), 1);
    }

    #[test]
    fn repeats_of_one_pitch_fold_into_a_single_visit() {
        let mut tracker = NoteTracker::new();
        play(&mut tracker, 60, 0.0, 1.0);
        play(&mut tracker, 60, 10.0, 10.5);
        let visits: Vec<&Visit> = tracker.history().visits().collect();
        assert_eq!(visits.len(), 1);
        assert_eq!(visits[0].last_off, 10.5, "the freshest visit wins");
    }

    /// The trail keeps the FIRST of several remembered pitches that match
    /// one node (`max`, and every level is 1.0 while the memory span is
    /// "never forget"), so this order picks the mark's color. It has to be
    /// a property of the music rather than of the map — see the same
    /// argument on `NoteTracker::voices`, and #135.
    ///
    /// Played high to low, so an order inherited from the playing would
    /// come back reversed and only the key order can pass.
    #[test]
    fn visits_come_back_lowest_pitch_first() {
        let mut tracker = NoteTracker::new();
        let played: Vec<u8> = (0..64u8).map(|i| 100 - i).collect();
        for (i, &note) in played.iter().enumerate() {
            play(&mut tracker, note, i as f64 * 4.0, i as f64 * 4.0 + 1.0);
        }
        let pitches: Vec<f32> = tracker.history().visits().map(|v| v.pitch).collect();
        let mut ascending = pitches.clone();
        ascending.sort_by(f32::total_cmp);
        assert_eq!(pitches, ascending, "visits must iterate in pitch order");
    }

    #[test]
    fn distinct_pitches_stay_distinct() {
        let mut tracker = NoteTracker::new();
        play(&mut tracker, 60, 0.0, 1.0);
        play(&mut tracker, 64, 2.0, 3.0);
        play(&mut tracker, 67, 4.0, 5.0);
        assert_eq!(tracker.history().visits().count(), 3);
    }

    #[test]
    fn a_bent_voice_is_remembered_at_its_sounding_pitch() {
        // Per-note tuning is part of the pitch, so the mark lands on the
        // node the note actually sounded at, not the one its key names.
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(NoteEvent {
            source: crate::SourceId::DIRECT,
            time: 0.1,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Tuning { semitones: 2.0 },
        });
        tracker.handle_event(off(1.0, 60));
        tracker.prune(3.0, &Envelope::default());
        let visit = *tracker.history().visits().next().unwrap();
        assert_eq!(visit.pitch, 62.0);
        assert_eq!(visit.pitch_class, PitchClass::from_midi_note(2));
    }

    #[test]
    fn the_oldest_pitch_is_forgotten_once_the_cap_is_reached() {
        let mut tracker = NoteTracker::new();
        // One more distinct pitch than fits, a cent apart so each is its own
        // visit; the first played is the least recently played.
        for i in 0..=NoteHistory::MAX_VISITS {
            let t = i as f64;
            tracker.handle_event(on(t, 60));
            tracker.handle_event(NoteEvent {
                source: crate::SourceId::DIRECT,
                time: t,
                channel: 0,
                note: 60,
                kind: NoteEventKind::Tuning { semitones: i as f32 * 0.01 },
            });
            tracker.handle_event(off(t, 60));
            tracker.prune(t + 2.0, &Envelope::default());
        }
        let history = tracker.history();
        assert_eq!(history.visits().count(), NoteHistory::MAX_VISITS);
        assert!(
            !history.visits().any(|v| v.pitch == 60.0),
            "the first pitch played should be the one dropped"
        );
    }

    #[test]
    fn clear_forgets_everything() {
        let mut tracker = NoteTracker::new();
        play(&mut tracker, 60, 0.0, 1.0);
        assert!(!tracker.history().is_empty());
        tracker.clear_history();
        assert!(tracker.history().is_empty());
    }
}
