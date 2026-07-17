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
    /// Envelope in `[0, 1]` driving the visual intensity of this voice:
    /// 1 while held, then a linear decay over `highlight_time` seconds.
    /// Fancier envelope shapes belong in the scene layer once we experiment;
    /// this is the single source of truth for "is this voice still visible".
    pub fn activation(&self, now: Time, highlight_time: f32) -> f32 {
        match self.state {
            VoiceState::Held => 1.0,
            VoiceState::Released { at } => {
                if highlight_time <= 0.0 {
                    return 0.0;
                }
                let elapsed = (now - at).max(0.0) as f32;
                (1.0 - elapsed / highlight_time).max(0.0)
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
        }
    }

    /// Drop released voices whose highlight has fully decayed. Call once per
    /// frame before iterating.
    pub fn prune(&mut self, now: Time, highlight_time: f32) {
        self.released
            .retain(|v| v.activation(now, highlight_time) > 0.0);
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
        // Still visible mid-decay...
        tracker.prune(1.5, 1.0);
        assert_eq!(tracker.voices().count(), 1);
        // ...gone after the highlight time has fully elapsed.
        tracker.prune(2.1, 1.0);
        assert_eq!(tracker.voices().count(), 0);
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
