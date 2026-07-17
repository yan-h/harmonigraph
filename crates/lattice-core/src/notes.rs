//! Tracking of active and recently-released MIDI voices.
//!
//! The plugin's audio thread converts host MIDI into [`NoteEvent`]s and
//! ships them to the GUI over a lock-free ring buffer; the standalone dev
//! harness generates them from a mock source. Either way, the GUI thread
//! owns a [`NoteTracker`] and feeds every event into it.

use std::collections::HashMap;

use crate::tuning::PitchClass;

/// Timestamps are seconds on a monotonic clock chosen by the shell (sample
/// clock in the plugin, wall clock in the standalone harness). Only
/// differences are ever used.
pub type Time = f64;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum NoteEventKind {
    On { velocity: f32 },
    Off,
    /// Per-note tuning offset in semitones (CLAP note expression / MPE),
    /// relative to the note's equal-tempered pitch. v1's PolyTuning.
    Tuning { semitones: f32 },
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct NoteEvent {
    pub time: Time,
    pub channel: u8,
    pub note: u8,
    pub kind: NoteEventKind,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum VoiceState {
    Held,
    Released { at: Time },
}

/// One sounding (or recently sounding) note.
#[derive(Copy, Clone, Debug)]
pub struct Voice {
    pub channel: u8,
    pub note: u8,
    pub velocity: f32,
    /// The sounding pitch in MIDI note units, including any per-note
    /// tuning (PolyTuning/MPE). Equal to `note` until a tuning arrives.
    pub pitch: f32,
    pub pitch_class: PitchClass,
    /// MIDI octave (C4 = middle C = note 60 → octave 4).
    pub octave: i8,
    pub on_time: Time,
    pub state: VoiceState,
}

impl Voice {
    /// The octave number for display, in Bitwig's convention where middle
    /// C (MIDI 60) is C3. (The internal `octave` field uses the C4 = middle
    /// C convention inherited from note/12 arithmetic.)
    pub fn display_octave(&self) -> i8 {
        self.octave - 1
    }

    /// Envelope in `[0, 1]` driving the visual intensity of this voice:
    /// 1 while held, then a linear decay over `fade_time` seconds.
    /// Fancier envelope shapes belong in the scene layer once we experiment;
    /// this is the single source of truth for "is this voice still visible".
    pub fn activation(&self, now: Time, fade_time: f32) -> f32 {
        match self.state {
            VoiceState::Held => 1.0,
            VoiceState::Released { at } => {
                if fade_time <= 0.0 {
                    return 0.0;
                }
                let elapsed = (now - at).max(0.0) as f32;
                (1.0 - elapsed / fade_time).max(0.0)
            }
        }
    }
}

/// Tracks held voices plus a tail of recently released ones (so releases can
/// fade out instead of vanishing).
#[derive(Default)]
pub struct NoteTracker {
    held: HashMap<(u8, u8), Voice>,
    released: Vec<Voice>,
}

impl NoteTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle_event(&mut self, event: NoteEvent) {
        // v1 semantics: the last channel (15 zero-indexed / 16 in MIDI
        // convention) is ignored entirely.
        if event.channel == 15 {
            return;
        }
        match event.kind {
            NoteEventKind::On { velocity } => {
                let voice = Voice {
                    channel: event.channel,
                    note: event.note,
                    velocity,
                    pitch: f32::from(event.note),
                    pitch_class: PitchClass::from_midi_note(event.note),
                    octave: (event.note / 12) as i8 - 1,
                    on_time: event.time,
                    state: VoiceState::Held,
                };
                self.held.insert((event.channel, event.note), voice);
            }
            NoteEventKind::Off => {
                if let Some(mut voice) = self.held.remove(&(event.channel, event.note)) {
                    voice.state = VoiceState::Released { at: event.time };
                    self.released.push(voice);
                }
            }
            NoteEventKind::Tuning { semitones } => {
                if let Some(voice) = self.held.get_mut(&(event.channel, event.note)) {
                    voice.pitch = f32::from(event.note) + semitones;
                    voice.pitch_class = PitchClass::from_cents(voice.pitch * 100.0);
                    // Octave indicators should track the sounding pitch too.
                    voice.octave = (voice.pitch / 12.0).floor() as i8 - 1;
                }
            }
        }
    }

    /// Drop released voices whose fade has fully completed. Call once per
    /// frame before iterating.
    pub fn prune(&mut self, now: Time, fade_time: f32) {
        self.released
            .retain(|v| v.activation(now, fade_time) > 0.0);
    }

    /// All voices that should currently be visualized (held first).
    pub fn voices(&self) -> impl Iterator<Item = &Voice> {
        self.held.values().chain(self.released.iter())
    }

    pub fn held_count(&self) -> usize {
        self.held.len()
    }

    pub fn all_notes_off(&mut self, now: Time) {
        for (_, mut voice) in self.held.drain() {
            voice.state = VoiceState::Released { at: now };
            self.released.push(voice);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(time: Time, note: u8) -> NoteEvent {
        NoteEvent { time, channel: 0, note, kind: NoteEventKind::On { velocity: 0.8 } }
    }

    fn off(time: Time, note: u8) -> NoteEvent {
        NoteEvent { time, channel: 0, note, kind: NoteEventKind::Off }
    }

    #[test]
    fn held_then_released_then_pruned() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        assert_eq!(tracker.voices().count(), 1);
        assert_eq!(tracker.held_count(), 1);

        tracker.handle_event(off(1.0, 60));
        assert_eq!(tracker.held_count(), 0);
        // Still visible mid-fade...
        tracker.prune(1.5, 1.0);
        assert_eq!(tracker.voices().count(), 1);
        // ...gone after the fade time has fully elapsed.
        tracker.prune(2.1, 1.0);
        assert_eq!(tracker.voices().count(), 0);
    }

    #[test]
    fn tuning_bends_pitch_class_and_octave() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60)); // C4
        // Bend up a whole tone: D, still octave 4.
        tracker.handle_event(NoteEvent {
            time: 0.1,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Tuning { semitones: 2.0 },
        });
        let voice = tracker.voices().next().unwrap();
        assert_eq!(voice.pitch, 62.0);
        assert_eq!(voice.pitch_class, PitchClass::from_midi_note(2));
        assert_eq!(voice.octave, 4);

        // Bend down past the octave boundary: B3.
        tracker.handle_event(NoteEvent {
            time: 0.2,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Tuning { semitones: -1.0 },
        });
        let voice = tracker.voices().next().unwrap();
        assert_eq!(voice.octave, 3);
    }

    #[test]
    fn channel_15_is_ignored() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 15,
            note: 60,
            kind: NoteEventKind::On { velocity: 0.8 },
        });
        assert_eq!(tracker.voices().count(), 0);
    }

    #[test]
    fn octave_is_derived_from_note_number() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60)); // middle C
        let voice = tracker.voices().next().unwrap();
        assert_eq!(voice.octave, 4);
        assert_eq!(voice.pitch_class, PitchClass::from_midi_note(0));
    }
}
