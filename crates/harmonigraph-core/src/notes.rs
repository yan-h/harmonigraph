//! Tracking of active and recently-released MIDI voices.
//!
//! The plugin's audio thread converts host MIDI into [`NoteEvent`]s and
//! ships them to the GUI over a lock-free ring buffer; the standalone dev
//! harness generates them from a mock source. Either way, the GUI thread
//! owns a [`NoteTracker`] and feeds every event into it.

use std::collections::BTreeMap;

use crate::history::NoteHistory;
use crate::roll::NoteRoll;
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
    /// Release every held voice at once (transport reset: per-note offs
    /// may never arrive). `channel` and `note` are meaningless here.
    AllOff,
}

/// What a MIDI channel means for tracking and rendering, inherited verbatim
/// from midi_lattice v1. Channels are zero-indexed here (v1's docs speak in
/// 1-indexed MIDI convention). This is the single source of truth for the
/// channel policy; the tracker and the scene's coloring both match on it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ChannelRole {
    /// Fixed per-channel color (channels 0-8).
    FixedColor,
    /// Colored by pitch height on a gradient (channels 9-13).
    PitchGradient,
    /// Rendered as an outline ring instead of a filled disc (channel 14).
    Outline,
    /// Never tracked or displayed (channel 15).
    Ignored,
}

impl ChannelRole {
    pub fn of(channel: u8) -> ChannelRole {
        match channel {
            0..=8 => ChannelRole::FixedColor,
            9..=13 => ChannelRole::PitchGradient,
            14 => ChannelRole::Outline,
            _ => ChannelRole::Ignored,
        }
    }
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
    fn new(channel: u8, note: u8, velocity: f32, on_time: Time) -> Voice {
        let mut voice = Voice {
            channel,
            note,
            velocity,
            pitch: 0.0,
            pitch_class: PitchClass::from_cents(0.0),
            octave: 0,
            on_time,
            state: VoiceState::Held,
        };
        voice.set_pitch(f32::from(note));
        voice
    }

    /// `pitch_class` and `octave` are pure functions of `pitch`; every
    /// write goes through here so the three fields can never disagree.
    fn set_pitch(&mut self, pitch: f32) {
        self.pitch = pitch;
        self.pitch_class = PitchClass::from_cents(pitch * 100.0);
        self.octave = (pitch / 12.0).floor() as i8 - 1;
    }

    /// The octave number for display, in Bitwig's convention where middle
    /// C (MIDI 60) is C3. (The internal `octave` field uses the C4 = middle
    /// C convention inherited from note/12 arithmetic.)
    pub fn display_octave(&self) -> i8 {
        self.octave - 1
    }

    /// Envelope in `[0, 1]` driving the visual intensity of this voice:
    /// 1 while held, then a linear decay over `fade_time` seconds.
    /// The scene layer shapes this further (attack ramps, per-octave
    /// envelopes); this stays the single source of truth for "is this voice
    /// still visible".
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

/// The display octave containing MIDI note `midi`, in Bitwig's convention
/// where middle C (MIDI 60) is C3. The inverse of [`octave_start_midi`].
/// Matches [`Voice::display_octave`]; use these rather than rewriting the
/// `/ 12 - 2` by hand, so the convention lives in one place.
pub fn display_octave_of(midi: i32) -> i32 {
    midi.div_euclid(12) - 2
}

/// The lowest MIDI note of display octave `octave` (Bitwig's convention).
/// The inverse of [`display_octave_of`].
pub fn octave_start_midi(octave: i32) -> i32 {
    (octave + 2) * 12
}

/// Tracks held voices plus a tail of recently released ones (so releases can
/// fade out instead of vanishing), and behind those a [`NoteHistory`] of
/// every pitch that has finished fading and a [`NoteRoll`] of when each
/// note sounded.
///
/// Ordered, not hashed, and that is load-bearing rather than a taste in
/// containers: `voices()` decides which of two voices lighting ONE node
/// wins its color, and a `HashMap`'s iteration order is seeded per map, so
/// the same take rendered twice picked different winners and produced
/// different pixels (#135). A `BTreeMap` keyed by `(channel, note)` makes
/// that choice a property of the music. The map holds a chord, so the
/// ordering costs nothing worth measuring.
#[derive(Default)]
pub struct NoteTracker {
    held: BTreeMap<(u8, u8), Voice>,
    released: Vec<Voice>,
    history: NoteHistory,
    roll: NoteRoll,
}

impl NoteTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle_event(&mut self, event: NoteEvent) {
        match event.kind {
            // Control event: applies regardless of the event's channel.
            NoteEventKind::AllOff => self.all_notes_off(event.time),
            _ if ChannelRole::of(event.channel) == ChannelRole::Ignored => {}
            NoteEventKind::On { velocity } => {
                // A retrigger without an Off silently replaces the held
                // voice (same key); the old voice gets no release fade.
                let voice = Voice::new(event.channel, event.note, velocity, event.time);
                self.roll.note_on(
                    event.channel,
                    event.note,
                    velocity,
                    voice.pitch,
                    event.time,
                );
                self.held.insert((event.channel, event.note), voice);
            }
            NoteEventKind::Off => {
                if let Some(mut voice) = self.held.remove(&(event.channel, event.note)) {
                    voice.state = VoiceState::Released { at: event.time };
                    self.released.push(voice);
                    self.roll.note_off(event.channel, event.note, event.time);
                }
            }
            NoteEventKind::Tuning { semitones } => {
                if let Some(voice) = self.held.get_mut(&(event.channel, event.note)) {
                    // Octave indicators track the sounding pitch too.
                    voice.set_pitch(f32::from(event.note) + semitones);
                    self.roll.bend(event.channel, event.note, event.time, voice.pitch);
                }
            }
        }
    }

    /// Drop released voices whose fade has fully completed, folding each
    /// into the history as it goes. Call once per frame before iterating.
    ///
    /// A voice becomes a memory in the same step it stops being drawn, so
    /// the two never describe one note at once and a trail picks the note
    /// up exactly where its fade lets go. (A retrigger without an off
    /// replaces its voice outright — see `handle_event` — so that voice is
    /// never recorded; the retrigger's own release covers the pitch.)
    pub fn prune(&mut self, now: Time, fade_time: f32) {
        self.roll.trim(now);
        let history = &mut self.history;
        self.released.retain(|voice| {
            if voice.activation(now, fade_time) > 0.0 {
                return true;
            }
            history.record(voice, now);
            false
        });
    }

    /// Every pitch played so far, for the trail (see [`NoteHistory`]).
    pub fn history(&self) -> &NoteHistory {
        &self.history
    }

    /// Forget everything played so far, leaving the live voices alone.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// When each note sounded, for the piano roll (see [`NoteRoll`]).
    pub fn roll(&self) -> &NoteRoll {
        &self.roll
    }

    /// Forget the played-note timeline. Independent of
    /// [`clear_history`](Self::clear_history): the two answer different
    /// questions and are cleared from different places in the UI.
    pub fn clear_roll(&mut self) {
        self.roll.clear();
    }

    /// All voices that should currently be visualized: held first, in
    /// `(channel, note)` order, then the released ones in the order they
    /// were let go.
    ///
    /// The order is part of the contract. Consumers accumulate over this —
    /// the lattice's node color goes to the first voice at the winning
    /// envelope, and every held voice shares one — so an unspecified order
    /// is an unspecified picture.
    pub fn voices(&self) -> impl Iterator<Item = &Voice> {
        self.held.values().chain(self.released.iter())
    }

    pub fn held_count(&self) -> usize {
        self.held.len()
    }

    pub fn all_notes_off(&mut self, now: Time) {
        self.roll.all_off(now);
        // Key order into `released`, which keeps its own order stable too —
        // a Vec built by draining a map inherits whatever order the map
        // iterated in, and then holds it for the whole fade.
        for mut voice in std::mem::take(&mut self.held).into_values() {
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

    /// Press one note on every tracked channel at each of several pitches,
    /// arriving in nothing like key order. Enough keys that a map iterating
    /// in some order of its own could not come back sorted by accident,
    /// which is what makes the assertion below a test rather than a coin
    /// toss.
    fn scrambled_chord(tracker: &mut NoteTracker) -> Vec<(u8, u8)> {
        let mut keys = Vec::new();
        // Pitch outside, channel inside: the presses walk across the
        // channels at one pitch before moving on, so insertion order and
        // key order share nothing but their first element.
        for step in 0..11u8 {
            for channel in 0..15u8 {
                let note = 21 + step * 7;
                tracker.handle_event(NoteEvent {
                    time: 0.0,
                    channel,
                    note,
                    kind: NoteEventKind::On { velocity: 0.8 },
                });
                keys.push((channel, note));
            }
        }
        keys.sort_unstable();
        keys
    }

    /// The order `voices()` hands the held voices back in is part of the
    /// picture rather than an implementation detail. Every held voice sits
    /// at activation 1.0, and the lattice gives a node's color and seed to
    /// the FIRST voice at the winning envelope — so which of two voices
    /// lighting one node (an octave doubling, say) wins is settled by this
    /// order alone. Off a hashed map it was settled per process instead,
    /// and one take rendered twice came out with different pixels in it
    /// (#135).
    #[test]
    fn held_voices_come_back_in_channel_note_order() {
        let mut tracker = NoteTracker::new();
        let expected = scrambled_chord(&mut tracker);
        let order: Vec<(u8, u8)> = tracker.voices().map(|v| (v.channel, v.note)).collect();
        assert_eq!(order, expected, "held voices must iterate in key order");
    }

    /// The same order has to survive the transport reset that turns every
    /// held voice into a releasing one: `released` is a Vec, so whatever
    /// order it is filled in is the order it keeps for the whole fade.
    #[test]
    fn all_notes_off_releases_the_voices_in_that_same_order() {
        let mut tracker = NoteTracker::new();
        let expected = scrambled_chord(&mut tracker);
        tracker.all_notes_off(1.0);
        let order: Vec<(u8, u8)> = tracker.voices().map(|v| (v.channel, v.note)).collect();
        assert_eq!(order, expected, "the released tail must inherit key order");
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

    #[test]
    fn display_octave_uses_bitwig_c3_convention() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60)); // middle C
        let voice = tracker.voices().next().unwrap();
        assert_eq!(voice.octave, 4); // internal: C4 = middle C
        assert_eq!(voice.display_octave(), 3); // shown one lower, as C3
    }

    #[test]
    fn activation_is_full_while_held_then_decays_linearly() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        let held = *tracker.voices().next().unwrap();
        // Held voices read full intensity regardless of the fade time.
        assert_eq!(held.activation(100.0, 2.0), 1.0);
        assert_eq!(held.activation(100.0, 0.0), 1.0);

        tracker.handle_event(off(10.0, 60));
        let released = *tracker.voices().next().unwrap();
        assert_eq!(released.activation(10.0, 2.0), 1.0); // full at release
        assert!((released.activation(11.0, 2.0) - 0.5).abs() < 1e-6); // half-way
        assert_eq!(released.activation(12.0, 2.0), 0.0); // fully faded
        assert_eq!(released.activation(20.0, 2.0), 0.0); // clamps, not negative
        assert_eq!(released.activation(9.0, 2.0), 1.0); // `now` before release
        // A non-positive fade time releases instantly (guards div-by-zero).
        assert_eq!(released.activation(10.0, 0.0), 0.0);
        assert_eq!(released.activation(10.0, -1.0), 0.0);
    }

    #[test]
    fn retrigger_without_off_replaces_the_held_voice() {
        // Pins existing behavior: no release fade for the first voice.
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(on(1.0, 60));
        assert_eq!(tracker.voices().count(), 1);
        assert_eq!(tracker.voices().next().unwrap().on_time, 1.0);
    }

    #[test]
    fn all_off_releases_every_channel() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 3,
            note: 64,
            kind: NoteEventKind::On { velocity: 0.5 },
        });
        tracker.handle_event(NoteEvent {
            time: 1.0,
            channel: 0,
            note: 0,
            kind: NoteEventKind::AllOff,
        });
        assert_eq!(tracker.held_count(), 0);
        // Released voices fade out rather than vanish.
        tracker.prune(1.5, 1.0);
        assert_eq!(tracker.voices().count(), 2);
        tracker.prune(2.1, 1.0);
        assert_eq!(tracker.voices().count(), 0);
    }

    #[test]
    fn channel_role_boundaries_match_v1() {
        assert_eq!(ChannelRole::of(0), ChannelRole::FixedColor);
        assert_eq!(ChannelRole::of(8), ChannelRole::FixedColor);
        assert_eq!(ChannelRole::of(9), ChannelRole::PitchGradient);
        assert_eq!(ChannelRole::of(13), ChannelRole::PitchGradient);
        assert_eq!(ChannelRole::of(14), ChannelRole::Outline);
        assert_eq!(ChannelRole::of(15), ChannelRole::Ignored);
    }
}
