//! What has already been played: the tail of the music that the
//! [`NoteTracker`](crate::NoteTracker) has finished drawing live.
//!
//! A voice enters here exactly when it stops being a voice — the moment
//! its release fade completes and the tracker drops it (see
//! [`NoteTracker::prune`](crate::NoteTracker::prune)). So history and the
//! live voices never describe the same note at the same time, and a trail
//! picks a note up precisely where its fade lets go.
//!
//! Two views of the same stream, because the scene asks two different
//! questions of it:
//! - [`Visit`] — one entry per distinct pitch, aggregated over every time
//!   it sounded. "Where has this piece been, and how much time did it
//!   spend there."
//! - [`Step`] — one entry per onset, in playing order. "In what order did
//!   it go there."

use std::collections::{HashMap, VecDeque};

use crate::notes::{Time, Voice, VoiceState};
use crate::tuning::PitchClass;

/// One pitch the music has visited, folded over every time it sounded.
#[derive(Copy, Clone, Debug)]
pub struct Visit {
    /// The sounding pitch in MIDI note units, from the most recent visit
    /// (per-note tuning included, so a bent voice is remembered bent).
    pub pitch: f32,
    pub pitch_class: PitchClass,
    /// MIDI octave, as [`Voice::octave`].
    pub octave: i8,
    /// The channel of the most recent visit, which is what colors the mark.
    pub channel: u8,
    pub first_on: Time,
    /// When it last stopped sounding — what "recently played" is measured
    /// against.
    pub last_off: Time,
    /// How many times it has been played.
    pub count: u32,
    /// Total seconds spent sounding, summed over every visit.
    pub held: f64,
}

/// One completed note, in the order it was played.
#[derive(Copy, Clone, Debug)]
pub struct Step {
    pub on_time: Time,
    pub off_time: Time,
    pub pitch: f32,
    pub pitch_class: PitchClass,
    pub octave: i8,
    pub channel: u8,
}

/// Key a visit is folded under: the sounding pitch rounded to the cent.
/// Coarse enough that repeats of one note always land together, fine
/// enough that two microtonally distinct pitches stay distinct.
fn visit_key(pitch: f32) -> i32 {
    (pitch * 100.0).round() as i32
}

/// Everything the piece has played, as far back as the caps below reach.
#[derive(Default)]
pub struct NoteHistory {
    visits: HashMap<i32, Visit>,
    /// Oldest first, so the path reads left to right in playing order.
    steps: VecDeque<Step>,
}

impl NoteHistory {
    /// Distinct pitches remembered. Past this the least recently played is
    /// forgotten — the bound is on GPU-free per-frame work (the scene
    /// tests every visit against every visible node), not on memory.
    pub const MAX_VISITS: usize = 384;
    /// Onsets kept for the path. Well past any path length the UI offers,
    /// so the setting alone decides how far back the route runs.
    pub const MAX_STEPS: usize = 1024;

    /// Fold a voice the tracker has finished with into history. `now` is
    /// only a fallback for the release time — a voice reaching here is
    /// always `Released`.
    pub fn record(&mut self, voice: &Voice, now: Time) {
        let off_time = match voice.state {
            VoiceState::Released { at } => at,
            VoiceState::Held => now,
        };
        // Clamped: a release timestamped before its own onset (clock
        // remap across a transport jump) must not credit negative dwell.
        let held = (off_time - voice.on_time).max(0.0);

        self.steps.push_back(Step {
            on_time: voice.on_time,
            off_time,
            pitch: voice.pitch,
            pitch_class: voice.pitch_class,
            octave: voice.octave,
            channel: voice.channel,
        });
        if self.steps.len() > Self::MAX_STEPS {
            self.steps.pop_front();
        }

        let visit = self.visits.entry(visit_key(voice.pitch)).or_insert(Visit {
            pitch: voice.pitch,
            pitch_class: voice.pitch_class,
            octave: voice.octave,
            channel: voice.channel,
            first_on: voice.on_time,
            last_off: off_time,
            count: 0,
            held: 0.0,
        });
        // The freshest visit owns everything a mark is drawn from; only
        // the totals accumulate.
        visit.pitch = voice.pitch;
        visit.pitch_class = voice.pitch_class;
        visit.octave = voice.octave;
        visit.channel = voice.channel;
        visit.last_off = visit.last_off.max(off_time);
        visit.count += 1;
        visit.held += held;

        if self.visits.len() > Self::MAX_VISITS {
            self.forget_oldest();
        }
    }

    /// Drop the least recently played pitch. O(visits), and only ever runs
    /// on the one insert that crosses the cap.
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

    /// Every remembered pitch, in no particular order.
    pub fn visits(&self) -> impl Iterator<Item = &Visit> {
        self.visits.values()
    }

    /// Every remembered onset, oldest first.
    pub fn steps(&self) -> impl DoubleEndedIterator<Item = &Step> {
        self.steps.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.visits.is_empty()
    }

    /// The busiest pitch's dwell weight, for normalizing a heat map. See
    /// [`Visit::heat_weight`].
    pub fn peak_heat(&self) -> f64 {
        self.visits().map(Visit::heat_weight).fold(0.0, f64::max)
    }

    /// Forget everything played so far.
    pub fn clear(&mut self) {
        self.visits.clear();
        self.steps.clear();
    }
}

impl Visit {
    /// The floor a single onset contributes to [`heat_weight`](Self::heat_weight),
    /// in seconds. Without it a staccato passage — every note far shorter
    /// than this — would weigh essentially nothing against one sustained
    /// pedal tone, and the heat map would show only the pedal.
    pub const MIN_DWELL: f64 = 0.05;

    /// How much of the music has happened at this pitch: total sounding
    /// time, with every onset worth at least [`MIN_DWELL`](Self::MIN_DWELL).
    /// Dwell rather than a plain count, because a tonal center is where the
    /// music *stays*, not where it merely touches.
    pub fn heat_weight(&self) -> f64 {
        self.held.max(f64::from(self.count) * Self::MIN_DWELL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::{NoteEvent, NoteEventKind, NoteTracker};

    fn on(time: Time, note: u8) -> NoteEvent {
        NoteEvent { time, channel: 0, note, kind: NoteEventKind::On { velocity: 0.8 } }
    }

    fn off(time: Time, note: u8) -> NoteEvent {
        NoteEvent { time, channel: 0, note, kind: NoteEventKind::Off }
    }

    /// Play `note` from `on_time` to `off_time` and let its fade complete,
    /// which is what moves it into history.
    fn play(tracker: &mut NoteTracker, note: u8, on_time: Time, off_time: Time) {
        tracker.handle_event(on(on_time, note));
        tracker.handle_event(off(off_time, note));
        tracker.prune(off_time + 2.0, 1.0);
    }

    #[test]
    fn a_note_enters_history_when_its_fade_completes_not_before() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(off(1.0, 60));
        // Mid-fade: still a live voice, not yet a memory.
        tracker.prune(1.5, 1.0);
        assert_eq!(tracker.voices().count(), 1);
        assert!(tracker.history().is_empty());
        // The fade completes and the voice becomes history in one step.
        tracker.prune(2.5, 1.0);
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
        assert_eq!(visits[0].count, 2);
        assert_eq!(visits[0].first_on, 0.0);
        assert_eq!(visits[0].last_off, 10.5);
        assert!((visits[0].held - 1.5).abs() < 1e-9);
        // Both onsets are still individually on the path.
        assert_eq!(tracker.history().steps().count(), 2);
    }

    #[test]
    fn distinct_pitches_stay_distinct_and_steps_keep_playing_order() {
        let mut tracker = NoteTracker::new();
        play(&mut tracker, 60, 0.0, 1.0);
        play(&mut tracker, 64, 2.0, 3.0);
        play(&mut tracker, 67, 4.0, 5.0);
        assert_eq!(tracker.history().visits().count(), 3);
        let order: Vec<f64> = tracker.history().steps().map(|s| s.on_time).collect();
        assert_eq!(order, vec![0.0, 2.0, 4.0]);
    }

    #[test]
    fn a_bent_voice_is_remembered_at_its_sounding_pitch() {
        // Per-note tuning is part of the pitch, so the trail lands on the
        // node the note actually sounded at, not the one its key names.
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(NoteEvent {
            time: 0.1,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Tuning { semitones: 2.0 },
        });
        tracker.handle_event(off(1.0, 60));
        tracker.prune(3.0, 1.0);
        let visit = *tracker.history().visits().next().unwrap();
        assert_eq!(visit.pitch, 62.0);
        assert_eq!(visit.pitch_class, PitchClass::from_midi_note(2));
    }

    #[test]
    fn heat_weighs_dwell_but_never_zero_for_a_staccato_note() {
        let mut tracker = NoteTracker::new();
        // A pedal tone against a flurry of instantaneous grace notes.
        play(&mut tracker, 60, 0.0, 4.0);
        for i in 0..3 {
            play(&mut tracker, 67, 10.0 + f64::from(i), 10.0 + f64::from(i));
        }
        let by_note = |pitch: f32| {
            *tracker.history().visits().find(|v| v.pitch == pitch).unwrap()
        };
        assert!((by_note(60.0).heat_weight() - 4.0).abs() < 1e-9);
        // Zero dwell, but three onsets still register.
        assert_eq!(by_note(67.0).held, 0.0);
        assert!((by_note(67.0).heat_weight() - 3.0 * Visit::MIN_DWELL).abs() < 1e-9);
        assert!((tracker.history().peak_heat() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn the_oldest_pitch_is_forgotten_once_the_cap_is_reached() {
        let mut tracker = NoteTracker::new();
        // One more distinct pitch than fits, a cent apart so each is its own
        // visit; the first played is the least recently played.
        for i in 0..=NoteHistory::MAX_VISITS {
            let t = i as f64;
            tracker.handle_event(NoteEvent {
                time: t,
                channel: 0,
                note: 60,
                kind: NoteEventKind::On { velocity: 0.8 },
            });
            tracker.handle_event(NoteEvent {
                time: t,
                channel: 0,
                note: 60,
                kind: NoteEventKind::Tuning { semitones: i as f32 * 0.01 },
            });
            tracker.handle_event(off(t, 60));
            tracker.prune(t + 2.0, 1.0);
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
        assert_eq!(tracker.history().steps().count(), 0);
    }
}
