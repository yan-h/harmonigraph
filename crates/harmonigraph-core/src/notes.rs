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

/// The sharpest curve `shape` 1 asks for, as the exponent in
/// [`Envelope::approach`]. `shape` walks the exponent from 1 (a straight
/// line) to this.
///
/// 4 is where the curve stops changing character rather than where it stops
/// being drawable: a quartic is 94% gone by half way, and what is left after
/// that is already too faint to read as motion. Higher exponents move the
/// knee closer to the start without changing what the viewer sees, so the top
/// of the bar would be a stretch of settings that all look alike — the bar's
/// own resolution spent on nothing.
const MAX_POWER: f32 = 4.0;

/// The shape and the two durations every fading layer of a node runs on: how
/// long a note takes to arrive, how long it takes to leave, and the one curve
/// both of those follow.
///
/// ONE curve for the two directions, so "snappy" cannot come to mean two
/// different things at once. It is applied as an APPROACH to whichever level
/// is being approached — full on the way in, nothing on the way out — which
/// is what makes the pair symmetric without either end having to be written
/// backwards (see [`Envelope::approach`]).
///
/// Two DURATIONS, though, and that asymmetry is load-bearing rather than an
/// omission — the two want the same RANGE, which is what the bars give them,
/// and not the same value. The ends MULTIPLY: a note released mid-attack
/// peaks at whatever its attack had reached by then, so an attack as long as
/// the fade means anything played faster than the fade never reaches full
/// brightness. One shared number would tie a long, deliberate release to a
/// long, deliberate arrival, and playing into it would go DIM — the two
/// settings are wanted at opposite ends far more often than together, a
/// slow departure under prompt arrivals being the ordinary ask. Held apart,
/// the shape stays global and the two times stay honest.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Envelope {
    /// Seconds a note takes to reach full brightness from its note-on.
    pub attack_time: f32,
    /// Seconds a released note keeps fading before it is gone.
    pub fade_time: f32,
    /// How curved both ends are, 0..=1: 0 a straight line, 1 the sharpest
    /// curve on offer ([`MAX_POWER`]).
    pub shape: f32,
}

impl Default for Envelope {
    /// A straight line, which is what every layer of the lattice faded on
    /// before the shape was a setting: a blob or a test that says nothing
    /// about the curve gets the one it always drew.
    fn default() -> Envelope {
        Envelope { attack_time: 0.0, fade_time: 1.0, shape: 0.0 }
    }
}

impl Envelope {
    /// How far a transition of `duration` seconds that began `elapsed`
    /// seconds ago has travelled, 0..=1. The one shape both directions run
    /// on, and the only place the curve is written.
    ///
    /// An APPROACH: it leaves where it started fast and settles into where it
    /// is going, `1 - (1-p)^n`. Both ends want that same fast-then-slow
    /// character, which is why one function serves them: reversing it for the
    /// release instead would give an attack that dawdles and then snaps to
    /// full, and a trigger that arrives late reads as a dropped frame rather
    /// than as a soft one.
    ///
    /// A POWER rather than the exponential a note's own sound decays on, and
    /// the reason is the parameter rather than the picture. An exponential is
    /// naturally described by a RATE — a time constant it never quite reaches
    /// the end of — and the setting above it here is a DURATION, the moment
    /// the note is gone. Fitting one to the other means normalizing
    /// `(1 - e^-kp)` by its own value at `p = 1`, and that normalization is
    /// not free: it needs a divide, a special case at `k = 0` where the ratio
    /// is 0/0, and a constant whose units are a rate on a bar that reads as a
    /// shape. A power arrives at exactly 1 on its own, at every exponent, with
    /// none of that — and the two draw the same picture over the range a bar
    /// can reach, a cubic sitting about where a rate constant of 4 does.
    ///
    /// What the power gives up is the true asymptotic tail, and it gives up
    /// nothing, because the normalization was cutting that tail off anyway.
    ///
    /// Arriving at exactly 1 is load-bearing and not tidiness: a release that
    /// only approaches 0 is a voice [`NoteTracker::prune`] can never drop, and
    /// the released tail would accumulate for the whole session against an
    /// O(nodes × voices) loop.
    ///
    /// A DURATION that is not a positive real number falls to "already over",
    /// and that is the safe direction rather than the tidy one: an attack
    /// lands at full and a release at nothing, so a poisoned number makes a
    /// node appear or vanish instantly instead of hanging a voice at full
    /// brightness that `prune` will never drop. The infinity is the case that
    /// matters and the one a `<= 0.0` test misses — it divides to a progress
    /// of 0, which is a release that never moves, and every note ever played
    /// then stays in the released tail for the rest of the session.
    ///
    /// A poisoned CLOCK falls the same way, and needs its own arm to do it:
    /// a real duration is the path a NaN `now` takes, and `f64::max` answers
    /// with the other operand against a NaN, so left to the divide it would
    /// come out a progress of 0 — the same never-moving release, reached
    /// through the branch that looks safe.
    ///
    /// A poisoned SHAPE straightens instead, there being a real duration to
    /// run out: a NaN through `clamp` stays a NaN, and `min` would answer with
    /// the bound and silently pin the curve at its sharpest.
    fn approach(&self, elapsed: Time, duration: f32) -> f32 {
        if !elapsed.is_finite() {
            // Over, whatever the duration says, so the two poisoned inputs
            // land in the same place instead of one of them being caught by
            // the ramp below (see the doc above).
            return 1.0;
        }
        if !duration.is_finite() || duration <= 0.0 {
            // No ramp to walk, so a STEP — but a step at the moment the
            // transition starts, not one that has always been over. The
            // difference is invisible on a note's own attack, where the clock
            // starts at the note-on and `elapsed` is never negative, and it is
            // the whole of the mark Delay, which works by handing this a start
            // moment in the future ([`ViewConfig::mark_delay`]). Answering 1
            // regardless would leave every ring drawn through its own wait,
            // and the Delay bar inert at exactly the attack of 0 that every
            // saved project loads on.
            return if elapsed >= 0.0 { 1.0 } else { 0.0 };
        }
        // Finite by the guard above, so this floor is only the transition
        // that has not started yet — a ring inside its mark Delay.
        let p = (elapsed.max(0.0) as f32 / duration).min(1.0);
        let shape = if self.shape.is_finite() { self.shape.clamp(0.0, 1.0) } else { 0.0 };
        // Exponent 1 is the straight line and `powf` returns it exactly, so
        // the flat end of the bar needs no case of its own.
        let power = 1.0 + shape * (MAX_POWER - 1.0);
        1.0 - (1.0 - p).powf(power)
    }

    /// How far a note that arrived at `since` has eased in, 0..=1.
    ///
    /// Keeps climbing after the key comes up, which is what lets a staccato
    /// note read at all: the release fades this ramp while it is still rising
    /// and the product peaks shortly after the note-off, rather than the note
    /// being cut off at whatever it had reached (see [`Voice::activation`]).
    pub fn attack(&self, now: Time, since: Time) -> f32 {
        self.approach(now - since, self.attack_time)
    }

    /// What is LEFT of a note released at `at`, 1 down to 0 — the mirror of
    /// [`attack`](Self::attack) on the same curve.
    pub fn release(&self, now: Time, at: Time) -> f32 {
        1.0 - self.approach(now - at, self.fade_time)
    }
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
    /// The moment this voice took the highest end, stamped as it LEFT the
    /// held set, and `None` if it was not wearing that end then (or is still
    /// held). Same for [`wore_low`](Self::wore_low) and the lowest.
    ///
    /// Held apart from [`HeldEnd`], which stays strictly about what is down
    /// now, because this is the opposite question: not "who is the melody"
    /// but "who was, on their way out". A melody/bass ring fades with its
    /// note rather than snapping off at the key, and this is what it fades
    /// FROM — the stamp its ease was running against, so the ring leaves at
    /// the level it had reached rather than jumping to full or to nothing.
    ///
    /// The MOMENT and not the level, deliberately. What the ease has reached
    /// depends on the Attack and the mark Delay, which are view settings a
    /// drag can change mid-fade; baking one into the tracker would leave
    /// every already-released ring answering to a setting that is no longer
    /// on screen. The moment is a fact about the music, and the view is free
    /// to re-read it every frame.
    pub wore_high: Option<Time>,
    /// The lowest end's stamp — see [`wore_high`](Self::wore_high).
    pub wore_low: Option<Time>,
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
            wore_high: None,
            wore_low: None,
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

    /// What is left of this voice's RELEASE, `[0, 1]`: 1 for the whole of a
    /// held note, then down to 0 across the fade.
    ///
    /// The half of the envelope that answers "is this voice over", which is
    /// not the same question as "is it visible" — an arriving note is barely
    /// visible and is the furthest thing from over. [`prune`](NoteTracker::prune)
    /// wants this one; everything that DRAWS wants
    /// [`activation`](Self::activation).
    pub fn release_level(&self, now: Time, env: &Envelope) -> f32 {
        match self.state {
            VoiceState::Held => 1.0,
            VoiceState::Released { at } => env.release(now, at),
        }
    }

    /// Envelope in `[0, 1]` driving the visual intensity of this voice: the
    /// attack it is easing in on times what is left of its release, both on
    /// the one curve the [`Envelope`] carries.
    ///
    /// The single source of truth for how lit a voice is, and the chokepoint
    /// every layer of a node multiplies through — the core disc, its glow,
    /// the octave sectors, the gutter it clears, and the piano roll. One
    /// function so a note cannot arrive at one rate and leave at another, nor
    /// have one layer disagree with the next about either.
    ///
    /// The melody/bass rings are the deliberate exception and take
    /// [`release_level`](Self::release_level) instead: a ring runs its own
    /// ease from the moment its note TOOK the end (plus whatever wait
    /// `mark_delay` asks), and multiplying the note's attack in on top would
    /// square the two wherever those moments coincide — the usual case —
    /// leaving a ring visibly slower than the sector it brackets.
    pub fn activation(&self, now: Time, env: &Envelope) -> f32 {
        env.attack(now, self.on_time) * self.release_level(now, env)
    }
}

/// One end of the chord that is DOWN — the highest or lowest held voice —
/// and the moment that voice took it.
///
/// HELD is the load-bearing word, and it is this type's guarantee rather than
/// its caller's: the scene rings these two ends LIVE, and a released voice
/// rings from the stamp it left with instead ([`Voice::wore_high`], read in
/// `derive::marks`). The two answer different questions — who is the melody,
/// and who was on their way out — so a released voice allowed back in here
/// would hold the end against the note that replaced it, and the incoming
/// ring would have nothing to ease from. It must never appear here, which
/// rests on the ends being read off `held` alone and restamped from every
/// mutation of it.
///
/// The "when" is state rather than a per-frame derivation because the answer
/// is not in the current voices: a voice that takes an end by INHERITING it,
/// when the note outside it comes up, took it at that note's release, and
/// that note is pruned a fade after its key does (see
/// [`NoteTracker::prune`]). Read off the released tail instead, the handoff
/// moment vanishes mid-ramp — so any ramp longer than the Fade param loses
/// its own start and lands at full in one frame, which is precisely the pop
/// a slow ease exists to avoid.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct HeldEnd {
    /// The voice holding this end, keyed as the tracker keys it:
    /// `(channel, note)`.
    pub key: (u8, u8),
    /// When this voice took the end — its own note-on when it arrived as
    /// the outer note of the chord, or the moment the voice outside it was
    /// released, whichever made it the end.
    pub since: Time,
}

/// The end `voice` now holds, carrying `prev`'s stamp forward when it is the
/// same voice still holding it and stamping `now` when it is not.
///
/// "The same voice" is the key AND the note-on behind it: a retrigger with no
/// off in between replaces the voice on a key it already had (see
/// [`NoteTracker::handle_event`]), and that is a new note taking the end, not
/// the old one keeping it.
fn took(prev: Option<HeldEnd>, voice: Option<&Voice>, now: Time) -> Option<HeldEnd> {
    let voice = voice?;
    let key = (voice.channel, voice.note);
    match prev {
        Some(end) if end.key == key && end.since >= voice.on_time => Some(end),
        _ => Some(HeldEnd { key, since: now }),
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
/// wins its color, and a `HashMap`'s iteration order is seeded per map — so
/// off one, the same take rendered twice picks different winners and produces
/// different pixels (#135). A `BTreeMap` keyed by `(channel, note)` makes
/// that choice a property of the music. The map holds a chord, so the
/// ordering costs nothing worth measuring.
#[derive(Default)]
pub struct NoteTracker {
    held: BTreeMap<(u8, u8), Voice>,
    released: Vec<Voice>,
    history: NoteHistory,
    roll: NoteRoll,
    /// The two ends of the held chord, restamped as the held set changes
    /// (see [`HeldEnd`] for why they are remembered rather than derived).
    high_end: Option<HeldEnd>,
    low_end: Option<HeldEnd>,
}

impl NoteTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle_event(&mut self, event: NoteEvent) {
        match event.kind {
            // Control event: applies regardless of the event's channel.
            NoteEventKind::AllOff => self.all_notes_off(event.time),
            // Returns rather than falling through to the restamp below: this
            // channel is not tracked at all, so nothing it carries can have
            // moved an end, and a per-note expression stream on it would
            // otherwise pay two scans of the held chord to conclude that.
            _ if ChannelRole::of(event.channel) == ChannelRole::Ignored => return,
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
                    self.stamp_ends_worn(&mut voice);
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
        // Every arm reaching here can move the chord's ends: two change which
        // voices are held, and a tuning can bend one past its neighbour. An
        // arm that changes nothing (an off for a key that is not down)
        // restamps to the same answer, `restamp_ends` being a re-read rather
        // than a reset — which is also why the `AllOff` arm having already
        // restamped inside `all_notes_off` costs nothing. That call is for the
        // shells that reach the transport reset directly, not through here.
        self.restamp_ends(event.time);
    }

    /// Copy onto `voice`, as it leaves the held set, the stamp of each end it
    /// was wearing — see [`Voice::wore_high`]. Call BEFORE
    /// [`restamp_ends`](Self::restamp_ends), which is what hands the ends to
    /// whoever is left.
    ///
    /// The `since >= on_time` guard is the same identity test [`took`] makes,
    /// and it matters for the same reason: a retrigger replaces the voice on a
    /// key the tracker already had, so an end still keyed to that key may have
    /// been stamped for the voice BEFORE this one. Matching on the key alone
    /// would hand this voice a ring its predecessor earned.
    fn stamp_ends_worn(&self, voice: &mut Voice) {
        let key = (voice.channel, voice.note);
        let worn = |end: Option<HeldEnd>| {
            end.filter(|e| e.key == key && e.since >= voice.on_time).map(|e| e.since)
        };
        voice.wore_high = worn(self.high_end);
        voice.wore_low = worn(self.low_end);
    }

    /// Re-read the two ends, keeping a `since` for as long as the SAME voice
    /// holds the end it stamped.
    ///
    /// Called from every mutation of the held set, and deliberately NOT from
    /// [`prune`](Self::prune): a released voice finishing its fade is not a
    /// change of ends, and it must not disturb a ramp that is still climbing.
    fn restamp_ends(&mut self, now: Time) {
        // Compared on `pitch` rather than the raw key, because MPE and
        // per-note tuning can bend a voice past its neighbour — the same
        // reason the notes pane sorts on pitch.
        let by_pitch = |a: &&Voice, b: &&Voice| a.pitch.total_cmp(&b.pitch);
        self.high_end = took(self.high_end, self.held.values().max_by(by_pitch), now);
        self.low_end = took(self.low_end, self.held.values().min_by(by_pitch), now);
    }

    /// Drop released voices whose fade has fully completed, folding each
    /// into the history as it goes. Call once per frame before iterating.
    ///
    /// A voice becomes a memory in the same step it stops being drawn, so
    /// the two never describe one note at once and a trail picks the note
    /// up exactly where its fade lets go. (A retrigger without an off
    /// replaces its voice outright — see `handle_event` — so that voice is
    /// never recorded; the retrigger's own release covers the pitch.)
    /// Asks the RELEASE rather than the full activation, because the question
    /// here is whether the fade is over and not whether anything is currently
    /// on screen. The two part company at exactly one moment and it is a real
    /// one: a note switched off in the same instant it arrived has an attack
    /// of 0, so its activation is 0 while its release has not started — read
    /// that way, a zero-length note would be dropped before it ever drew.
    pub fn prune(&mut self, now: Time, env: &Envelope) {
        self.roll.trim(now);
        let history = &mut self.history;
        self.released.retain(|voice| {
            if voice.release_level(now, env) > 0.0 {
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

    /// The highest held voice and when it took that end — the chord's top
    /// line, which the scene marks as the melody. `None` while nothing is
    /// down. See [`HeldEnd`].
    pub fn highest_held(&self) -> Option<HeldEnd> {
        self.high_end
    }

    /// The lowest held voice, the same way: the bass end.
    pub fn lowest_held(&self) -> Option<HeldEnd> {
        self.low_end
    }

    pub fn all_notes_off(&mut self, now: Time) {
        self.roll.all_off(now);
        // Key order into `released`, which keeps its own order stable too —
        // a Vec built by draining a map inherits whatever order the map
        // iterated in, and then holds it for the whole fade.
        for mut voice in std::mem::take(&mut self.held).into_values() {
            self.stamp_ends_worn(&mut voice);
            voice.state = VoiceState::Released { at: now };
            self.released.push(voice);
        }
        self.restamp_ends(now);
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
    /// at activation 1.0, and the lattice gives a node's color and outline to
    /// the FIRST voice at the winning envelope — so which of two voices
    /// lighting one node (an octave doubling, say) wins is settled by this
    /// order alone. Off a hashed map it is settled per process instead, and
    /// one take rendered twice comes out with different pixels in it (#135).
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
        tracker.prune(1.5, &Envelope::default());
        assert_eq!(tracker.voices().count(), 1);
        // ...gone after the fade time has fully elapsed.
        tracker.prune(2.1, &Envelope::default());
        assert_eq!(tracker.voices().count(), 0);
    }

    /// The chord's two ends, and the moment each one changed hands. The
    /// scene rings them and eases each ring in from the stamp, so a stamp
    /// that moves when nothing changed hands is a ring that restarts under a
    /// note nobody touched.
    #[test]
    fn the_ends_are_stamped_when_they_change_hands_and_not_otherwise() {
        let mut tracker = NoteTracker::new();
        let ends = |t: &NoteTracker| (t.highest_held(), t.lowest_held());
        assert_eq!(ends(&tracker), (None, None), "nothing down, no ends");

        // A lone note is both ends, taken at its own note-on.
        tracker.handle_event(on(1.0, 60));
        assert_eq!(tracker.highest_held(), Some(HeldEnd { key: (0, 60), since: 1.0 }));
        assert_eq!(tracker.lowest_held(), Some(HeldEnd { key: (0, 60), since: 1.0 }));

        // A note inside the chord moves neither end, and must not restamp
        // the ends it did not take.
        tracker.handle_event(on(2.0, 55));
        tracker.handle_event(on(3.0, 57));
        assert_eq!(tracker.highest_held(), Some(HeldEnd { key: (0, 60), since: 1.0 }));
        assert_eq!(tracker.lowest_held(), Some(HeldEnd { key: (0, 55), since: 2.0 }));

        // Lifting the top hands the melody DOWN, at the moment of the lift
        // rather than at the note-on of the voice that inherits it — which is
        // older than the chord and would leave nothing to ease.
        tracker.handle_event(off(4.0, 60));
        assert_eq!(tracker.highest_held(), Some(HeldEnd { key: (0, 57), since: 4.0 }));

        // Pruning the voice that handed it over is not a change of ends. This
        // is the whole reason the stamp is kept here rather than read back off
        // the released tail, which the prune empties.
        tracker.prune(5.0, &Envelope { fade_time: 0.1, ..Envelope::default() });
        assert_eq!(tracker.voices().count(), 2, "the released C4 is gone");
        assert_eq!(tracker.highest_held(), Some(HeldEnd { key: (0, 57), since: 4.0 }));

        // A retrigger with no off in between replaces the voice on a key it
        // already had, and that is a new note taking the end, not the old one
        // keeping it.
        tracker.handle_event(on(6.0, 57));
        assert_eq!(tracker.highest_held(), Some(HeldEnd { key: (0, 57), since: 6.0 }));

        // A bend past a neighbour moves the end without any key changing:
        // MPE and per-note tuning are why the ends are compared on pitch.
        tracker.handle_event(NoteEvent {
            time: 7.0,
            channel: 0,
            note: 55,
            kind: NoteEventKind::Tuning { semitones: 6.0 },
        });
        assert_eq!(tracker.highest_held(), Some(HeldEnd { key: (0, 55), since: 7.0 }));
        assert_eq!(tracker.lowest_held(), Some(HeldEnd { key: (0, 57), since: 7.0 }));

        // A transport reset takes every held voice, so it takes both ends.
        tracker.all_notes_off(8.0);
        assert_eq!(ends(&tracker), (None, None));
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

    /// A fade of `secs` with no attack, on a straight line.
    fn fade(secs: f32) -> Envelope {
        Envelope { fade_time: secs, ..Envelope::default() }
    }

    #[test]
    fn activation_is_full_while_held_then_decays_over_the_fade() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        let held = *tracker.voices().next().unwrap();
        // Held voices read full intensity regardless of the fade time — with
        // no attack, which is what `fade` builds; the attack has its own
        // tests below.
        assert_eq!(held.activation(100.0, &fade(2.0)), 1.0);
        assert_eq!(held.activation(100.0, &fade(0.0)), 1.0);

        tracker.handle_event(off(10.0, 60));
        let released = *tracker.voices().next().unwrap();
        assert_eq!(released.activation(10.0, &fade(2.0)), 1.0); // full at release
        assert!((released.activation(11.0, &fade(2.0)) - 0.5).abs() < 1e-6); // half-way
        assert_eq!(released.activation(12.0, &fade(2.0)), 0.0); // fully faded
        assert_eq!(released.activation(20.0, &fade(2.0)), 0.0); // clamps, not negative
        assert_eq!(released.activation(9.0, &fade(2.0)), 1.0); // `now` before release
        // A non-positive fade time releases instantly (guards div-by-zero).
        assert_eq!(released.activation(10.0, &fade(0.0)), 0.0);
        assert_eq!(released.activation(10.0, &fade(-1.0)), 0.0);
    }

    /// The two ends are one curve read in two directions, which is the whole
    /// claim of the [`Envelope`] type: whatever the shape, what an attack has
    /// GAINED at a given fraction of its time is exactly what a release of
    /// the same shape has LOST at the same fraction of its own.
    #[test]
    fn the_attack_and_the_release_are_the_same_curve() {
        for shape in [0.0, 0.35, 1.0] {
            let env = Envelope { attack_time: 4.0, fade_time: 4.0, shape };
            for step in 0..=8 {
                let t = f64::from(step) * 0.5;
                let gained = env.attack(t, 0.0);
                let left = env.release(t, 0.0);
                assert!(
                    (gained + left - 1.0).abs() < 1e-6,
                    "shape {shape} at {t}s: gained {gained}, left {left}"
                );
            }
        }
    }

    /// Both ends hit their endpoints exactly, at every shape. The release end
    /// is the load-bearing one: `prune` drops a voice when its release
    /// reaches 0, so a curve that only approached 0 would keep every note
    /// ever played in the released tail (see [`Envelope::approach`]).
    #[test]
    fn every_shape_starts_and_finishes_exactly() {
        for shape in [0.0, 0.1, 0.35, 0.9, 1.0] {
            let env = Envelope { attack_time: 0.5, fade_time: 2.0, shape };
            assert_eq!(env.attack(0.0, 0.0), 0.0, "attack starts at nothing, shape {shape}");
            assert_eq!(env.attack(0.5, 0.0), 1.0, "and is full at its time, shape {shape}");
            assert_eq!(env.release(0.0, 0.0), 1.0, "release starts at full, shape {shape}");
            assert_eq!(env.release(2.0, 0.0), 0.0, "and is gone at the fade, shape {shape}");
            assert_eq!(env.release(1e6, 0.0), 0.0, "and stays gone, shape {shape}");
        }
    }

    /// What the Shape bar buys: 0 is the straight line the lattice has always
    /// faded on, and turning it up moves the fade EARLIER without moving
    /// either end. Written as an ordering rather than as numbers so it states
    /// the property the bar promises instead of restating the formula.
    #[test]
    fn a_higher_shape_front_loads_the_fade() {
        let level = |shape: f32| Envelope { fade_time: 2.0, shape, ..Envelope::default() }
            .release(1.0, 0.0);
        assert!((level(0.0) - 0.5).abs() < 1e-6, "0 is the straight line: half gone at half way");
        let mut prev = level(0.0);
        for shape in [0.25, 0.5, 0.75, 1.0] {
            let now = level(shape);
            assert!(now < prev, "shape {shape} leaves less at half way than the shape under it");
            prev = now;
        }
        assert!(prev < 0.1, "and the top of the bar is most of the way gone by half way");
    }

    /// A note switched off before its attack finishes never reaches full —
    /// the two ends multiply — and this is the reason the attack has its own
    /// duration rather than sharing the Fade (see [`Envelope`]).
    #[test]
    fn a_note_shorter_than_its_attack_peaks_below_full() {
        let env = Envelope { attack_time: 0.2, fade_time: 1.0, shape: 0.0 };
        let mut voice = Voice::new(0, 60, 1.0, 0.0);
        voice.state = VoiceState::Released { at: 0.05 };
        // Sampled across the whole of the note's life, peak included: the
        // attack keeps climbing after the key is up, so the brightest moment
        // is not the note-off.
        let peak = (0..=100)
            .map(|i| voice.activation(f64::from(i) * 0.01, &env))
            .fold(0.0f32, f32::max);
        assert!(peak > 0.0, "a short note still draws");
        assert!(peak < 1.0, "but a quarter of an attack cannot reach full: {peak}");
    }

    /// A zero-length note — on and off at one instant — still fades in rather
    /// than never drawing at all. This is why `prune` asks the RELEASE and not
    /// the activation: at the note-on instant the attack is 0, so a prune
    /// reading the activation would drop the voice in the same frame it
    /// arrived.
    #[test]
    fn a_zero_length_note_survives_its_first_prune() {
        let env = Envelope { attack_time: 0.1, fade_time: 1.0, shape: 0.0 };
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(off(0.0, 60));
        let voice = *tracker.voices().next().unwrap();
        assert_eq!(voice.activation(0.0, &env), 0.0, "nothing on screen at the instant itself");
        assert_eq!(voice.release_level(0.0, &env), 1.0, "but the release has not started");

        tracker.prune(0.0, &env);
        assert_eq!(tracker.voices().count(), 1, "so the note is still there to draw");
        let voice = *tracker.voices().next().unwrap();
        assert!(voice.activation(0.05, &env) > 0.0, "and it does draw, a frame later");
    }

    /// A zero-length ramp is a STEP AT ITS START, not a level that has always
    /// been reached. The mark Delay is the whole reason the difference is
    /// visible: it works by handing [`Envelope::attack`] a start moment in the
    /// future, so a curve answering 1 regardless would draw every ring
    /// straight through its own wait — and it would do it at an Attack of 0,
    /// which is what every project saved before the bar existed loads on.
    #[test]
    fn a_zero_length_ramp_steps_at_its_start() {
        let env = Envelope { attack_time: 0.0, fade_time: 1.0, shape: 0.0 };
        assert_eq!(env.attack(0.5, 1.0), 0.0, "the start is still ahead");
        assert_eq!(env.attack(1.0, 1.0), 1.0, "full the moment it arrives");
        assert_eq!(env.attack(1.5, 1.0), 1.0, "and stays there");
    }

    /// The guards in [`Envelope::approach`], which a shell can reach without
    /// passing `ViewConfig::sanitize`. Every one falls to "already over",
    /// because the alternative for the release end is a voice stuck at full
    /// brightness that `prune` will never drop.
    #[test]
    fn a_non_finite_envelope_ends_rather_than_hangs() {
        let mut voice = Voice::new(0, 60, 1.0, 0.0);
        voice.state = VoiceState::Released { at: 0.0 };
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let fades = Envelope { attack_time: 0.0, fade_time: bad, shape: 0.0 };
            assert_eq!(voice.release_level(1.0, &fades), 0.0, "fade_time {bad} ends the note");

            // A poisoned SHAPE straightens rather than ending, the duration
            // still being a real number: there is nothing to hang on.
            let curve = Envelope { attack_time: 0.0, fade_time: 2.0, shape: bad };
            let level = curve.release(1.0, 0.0);
            assert!(level.is_finite(), "shape {bad} keeps a real level, got {level}");
        }
        // A poisoned CLOCK ends too, and the release is the half that has to:
        // a real duration takes the ramp's own path, where `f64::max` turns a
        // NaN into a progress of 0 — a release stuck at 1 forever, which is
        // the hang this whole test is named for, reached with every duration
        // finite.
        let env = Envelope { attack_time: 1.0, fade_time: 1.0, shape: 0.5 };
        voice.state = VoiceState::Released { at: 0.0 };
        assert_eq!(voice.release_level(f64::NAN, &env), 0.0, "a NaN clock ends the note");
        assert_eq!(env.attack(f64::NAN, 0.0), 1.0, "and lands the arrival rather than stalling it");
        // The ramp still has a "not started" end, and it is reached by a real
        // clock ahead of the transition — the mark Delay, which is the whole
        // reason `approach` distinguishes the two at all.
        assert_eq!(env.attack(0.5, 1.0), 0.0, "a start still ahead has not started");
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
        tracker.prune(1.5, &Envelope::default());
        assert_eq!(tracker.voices().count(), 2);
        tracker.prune(2.1, &Envelope::default());
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
