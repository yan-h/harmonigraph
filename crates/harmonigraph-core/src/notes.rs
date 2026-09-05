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

/// Identity in one canonical display/take stream. Zero is reserved for the
/// hub's direct input. The session owner assigns each tuner lease a fresh
/// nonzero identity; a reusable source slot alone is not an identity.
///
/// This is provenance, not authorization: runtime session/epoch/incarnation
/// validation happens before publication. Those runtime tokens are not saved
/// in takes, and replay must never enroll a recorded source into a session.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(pub u64);

impl SourceId {
    pub const DIRECT: Self = Self(0);
}

/// One held address. Same-source retriggers replace this address; host note
/// IDs remain a shell pass-through concern, not a second held-voice key.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VoiceKey {
    pub source: SourceId,
    pub channel: u8,
    pub note: u8,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum NoteEventKind {
    On {
        velocity: f32,
    },
    Off,
    /// Per-note tuning offset in semitones (CLAP note expression / MPE),
    /// relative to the note's equal-tempered pitch. v1's PolyTuning.
    Tuning {
        semitones: f32,
    },
    /// Release this event's source only. Channel and note are ignored.
    SourceReset,
    /// Release every source. Source, channel and note are ignored.
    SessionReset,
}

/// A note's MIDI channel carries no meaning here. It is kept on [`Voice`] and
/// [`NoteEvent`] because it is part of a note's IDENTITY — the host's key for
/// matching an off to its on, and what lets two lanes hold the same note
/// number at once — and for nothing else. Every channel is tracked, drawn as
/// a filled disc, and colored by pitch height on the gradient, so two notes
/// of one pitch are indistinguishable whichever lanes they arrived on.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct NoteEvent {
    pub time: Time,
    pub source: SourceId,
    pub channel: u8,
    pub note: u8,
    pub kind: NoteEventKind,
}

impl NoteEvent {
    /// A note-on, its arguments in the struct's own field order — so the two
    /// adjacent `u8`s are read in the order they are declared rather than
    /// guessed at.
    ///
    /// Plain `pub` rather than `#[cfg(test)]`, and it is worth saying why,
    /// because the cheap-looking option does not work: `cfg(test)` is set only
    /// while a crate compiles its OWN test harness, so a `#[cfg(test)]` item
    /// here would be invisible to every dependent crate's tests — which is
    /// where all but a handful of the callers are. A cargo feature would
    /// reach them, but only through a dev-dependency edge declared per crate,
    /// and feature unification then leaves it on for the shipped build
    /// anyway; a separate helper crate would trip the dependency guard
    /// `ci.sh` holds over this one. Two dependency-free constructors, always
    /// compiled, is the smallest thing that reaches the callers.
    pub fn on(time: Time, source: SourceId, channel: u8, note: u8, velocity: f32) -> Self {
        NoteEvent { time, source, channel, note, kind: NoteEventKind::On { velocity } }
    }

    /// A note-off, which carries no velocity of its own: a release velocity
    /// reaches nothing here (see [`NoteEventKind::Off`]).
    pub fn off(time: Time, source: SourceId, channel: u8, note: u8) -> Self {
        NoteEvent { time, source, channel, note, kind: NoteEventKind::Off }
    }

    pub fn source_reset(time: Time, source: SourceId) -> Self {
        Self { time, source, channel: 0, note: 0, kind: NoteEventKind::SourceReset }
    }

    pub fn session_reset(time: Time) -> Self {
        Self {
            time,
            source: SourceId::DIRECT,
            channel: 0,
            note: 0,
            kind: NoteEventKind::SessionReset,
        }
    }

    pub fn key(&self) -> VoiceKey {
        VoiceKey { source: self.source, channel: self.channel, note: self.note }
    }
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
/// Two DURATIONS at this level, where the type is a general primitive and the
/// tests below drive the ends independently. The one production caller
/// (`ViewConfig::envelope`) feeds a single number into both, so a note comes
/// up on the same time it goes down on — which costs nothing because the ends
/// are SEQUENCED and not merely multiplied: the departure waits for the
/// arrival to land, so the two never run at once and no length of note dims.
/// [`Voice::release_level`] is where that rule is written.
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
    /// run out; [`power`](Self::power) is where that is done, for both readings
    /// of the curve at once.
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
        1.0 - (1.0 - p).powf(self.power())
    }

    /// The curve's exponent: 1 at the flat end of the shape bar, [`MAX_POWER`]
    /// at the sharp one.
    ///
    /// Its own function because two readings of the curve need it — the
    /// approach above walks it forward, and [`carried`](Self::carried) solves
    /// it backward — and a second copy of the mapping is how the two would come
    /// to draw different curves from one bar.
    ///
    /// A NaN shape straightens rather than clamping: a NaN through `clamp`
    /// stays a NaN, and `min` would answer with the bound and silently pin the
    /// curve at its sharpest.
    fn power(&self) -> f32 {
        let shape = if self.shape.is_finite() { self.shape.clamp(0.0, 1.0) } else { 0.0 };
        // Exponent 1 is the straight line and `powf` returns it exactly, so
        // the flat end of the bar needs no case of its own.
        1.0 + shape * (MAX_POWER - 1.0)
    }

    /// `level` carried `dt` seconds further along this envelope — toward 1
    /// while `arriving`, toward 0 once it is not — on the same curve
    /// [`attack`](Self::attack) and [`release`](Self::release) run.
    ///
    /// A LEVEL in and a level out, where those two take the MOMENT a transition
    /// began, and the difference is what is being faded rather than a
    /// convenience. A note has a note-on to run from and keeps it for the whole
    /// of its life, so the level it has reached is always re-derivable from the
    /// setting on screen — the rule [`Voice::wore_high`] states. The audio
    /// ring's gate has no such moment: it opens when the spectrum crosses a
    /// threshold and can cross back before the fade it started has landed, and
    /// a transition restarted from a stamp would jump the picture to whatever
    /// the new ramp reads at zero. Carrying the level is what makes a reversal
    /// continuous.
    ///
    /// The DURATION and the shape are still read fresh every step, so a drag on
    /// either reaches a transition already under way; only where it has got to
    /// is remembered. Progress is linear in time and the curve is a fixed
    /// bijection over it, so stepping is exact: the same clock walked in one
    /// step or in twenty lands on the same level, which is what keeps a render
    /// at 60 fps and a live pane at 144 drawing one picture.
    ///
    /// The poisoned inputs land where [`approach`](Self::approach) puts them,
    /// and for the same reason: a duration that is not a positive real number
    /// is no ramp to walk, so the transition is a STEP to where it was going,
    /// and a clock that is not a number is a step as well. Both make a ring
    /// appear or vanish at once rather than hanging part way through a fade
    /// that can never finish. A `dt` running BACKWARD is the one case that
    /// holds instead of stepping — a clock that moves back is a picture from
    /// another moment, not a transition — and it is what keeps a pane drawn
    /// twice in one frame from stepping the fade twice.
    pub fn carried(&self, level: f32, dt: Time, arriving: bool) -> f32 {
        let target = if arriving { 1.0 } else { 0.0 };
        let (duration, remaining) =
            if arriving { (self.attack_time, 1.0 - level) } else { (self.fade_time, level) };
        if !dt.is_finite() || !duration.is_finite() || duration <= 0.0 || !remaining.is_finite() {
            return target;
        }
        // A step of nothing, or a clock that has moved BACK. Answered here and
        // not left to the arithmetic, because the arithmetic is not the
        // identity at `dt` = 0: recovering the progress and walking it forward
        // again is a pair of `powf`s, and they land a rounding away. A pane
        // drawn twice in one frame has to come out with the level it already
        // had, exactly, or the picture depends on how many panes are open.
        if dt <= 0.0 {
            return level.clamp(0.0, 1.0);
        }
        let remaining = remaining.clamp(0.0, 1.0);
        if remaining <= 0.0 {
            return target;
        }
        // How far into the transition this level stands: `approach` leaves
        // `(1 - p)^n` of the distance to go, so this is that solved for p.
        let power = self.power();
        let p = 1.0 - remaining.powf(1.0 / power);
        let p = (p + dt as f32 / duration).min(1.0);
        // Arriving at exactly the target is load-bearing rather than tidy: a
        // ring that only approaches nothing is a ring every idle node goes on
        // shipping an instance for. `p` reaches exactly 1 above, and `0^n` is
        // exactly 0.
        let left = (1.0 - p).powf(power);
        if arriving {
            1.0 - left
        } else {
            left
        }
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

    /// What is LEFT of a departure that began at `at`, 1 down to 0 — the
    /// mirror of [`attack`](Self::attack) on the same curve.
    ///
    /// `at` is the moment the departure BEGINS, which for a voice is not the
    /// key-up: it is the later of that and the end of the note's own arrival
    /// ([`Voice::release_level`]). Handing this the key-up direct puts the two
    /// ends back on top of each other, which is the whole of what that rule
    /// buys off.
    pub fn release(&self, now: Time, at: Time) -> f32 {
        1.0 - self.approach(now - at, self.fade_time)
    }
}

/// One sounding (or recently sounding) note.
#[derive(Copy, Clone, Debug)]
pub struct Voice {
    pub source: SourceId,
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
    /// but "who was, on their way out". A melody/bass mark fades with its
    /// note rather than snapping off at the key, and this is what it fades
    /// FROM — the stamp its ease was running against, so the ring leaves at
    /// the level it had reached rather than jumping to full or to nothing.
    ///
    /// The MOMENT and not the level, deliberately. What the ease has reached
    /// depends on the Fade duration and the mark Delay, either of which a drag
    /// (or a host automation lane) can move mid-fade; baking one into the
    /// tracker would leave every already-released ring answering to a setting
    /// that is no longer on screen. The moment is a fact about the music, and
    /// the view is free to re-read it every frame.
    pub wore_high: Option<Time>,
    /// The lowest end's stamp — see [`wore_high`](Self::wore_high).
    pub wore_low: Option<Time>,
}

impl Voice {
    fn new(source: SourceId, channel: u8, note: u8, velocity: f32, on_time: Time) -> Voice {
        let mut voice = Voice {
            source,
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

    pub fn key(&self) -> VoiceKey {
        VoiceKey { source: self.source, channel: self.channel, note: self.note }
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
    /// held note AND for whatever is left of its arrival, then down to 0
    /// across the fade.
    ///
    /// The departure waits for the arrival to LAND, and that sequencing is the
    /// one thing keeping the two ends of a single duration off each other.
    /// Allowed to overlap they MULTIPLY, and a note released while its arrival
    /// is still climbing peaks at whatever the rising ramp and the falling one
    /// cross at — dimmer the faster it was played, in proportion to how little
    /// of the arrival it caught. With one duration driving both ends
    /// (`ViewConfig::envelope`) that would be every note shorter than the
    /// Fade, so a run of staccato notes would read as a dim smear. Sequenced,
    /// a note played at any speed reaches full brightness: what a short key
    /// gives up is TIME at full, which is the thing a short key should cost.
    ///
    /// The floor under a note's life is therefore its arrival PLUS its
    /// departure, and that is the deliberate price — a stab draws the same
    /// gesture a held note does, minus the middle.
    ///
    /// The half of the envelope that answers "is this voice over", which is
    /// not the same question as "is it visible" — an arriving note is barely
    /// visible and is the furthest thing from over. [`prune`](NoteTracker::prune)
    /// wants this one; everything that DRAWS wants
    /// [`activation`](Self::activation).
    pub fn release_level(&self, now: Time, env: &Envelope) -> f32 {
        match self.state {
            VoiceState::Held => 1.0,
            // A poisoned attack time cannot hang a voice here, either way it
            // can fail: `f64::max` answers with its other operand against a
            // NaN, so the wait is dropped and the note leaves on its key,
            // and an infinite one pushes the start past every clock, which
            // `release` reads as already over. Both END the voice, which is
            // the side `prune` can recover from (see `Envelope::approach`).
            VoiceState::Released { at } => {
                env.release(now, at.max(self.on_time + f64::from(env.attack_time)))
            }
        }
    }

    /// Envelope in `[0, 1]` driving the visual intensity of this voice: the
    /// attack it is easing in on times what is left of its release, both on
    /// the one curve the [`Envelope`] carries.
    ///
    /// A product of two ramps that never run at once — the release holds off
    /// until the attack has landed ([`release_level`](Self::release_level)) —
    /// so what it draws is the arrival, then full, then the departure, and a
    /// note is never dimmed for having been short.
    ///
    /// The single source of truth for how lit a voice is, and the chokepoint
    /// every layer of a node multiplies through — the core disc, its glow,
    /// the octave sectors, the gutter it clears, and the piano roll. One
    /// function so a note cannot arrive at one rate and leave at another, nor
    /// have one layer disagree with the next about either.
    ///
    /// The melody/bass marks are the deliberate exception and take
    /// [`release_level`](Self::release_level) instead: a mark runs its own
    /// ease from the moment its note TOOK the end (plus whatever wait
    /// `mark_delay` asks), and multiplying the note's attack in on top would
    /// square the two wherever those moments coincide — the usual case —
    /// leaving a mark visibly slower than the sector it extends.
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
/// that note is pruned a fade after its DEPARTURE begins — which is the later
/// of its key-up and the end of its own arrival, so at most an arrival plus a
/// fade after the key (see [`Voice::release_level`] and
/// [`NoteTracker::prune`]). Read off the released tail instead, the handoff
/// moment vanishes mid-ramp — so any ramp longer than the Fade param loses
/// its own start and lands at full in one frame, which is precisely the pop
/// a slow ease exists to avoid.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct HeldEnd {
    /// The voice holding this end, keyed as the tracker keys it:
    /// `(source, channel, note)`.
    pub key: VoiceKey,
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
    let key = voice.key();
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
/// different pixels (#135). A `BTreeMap` keyed by `(source, channel, note)` makes
/// that choice a property of the music. The map holds a chord, so the
/// ordering costs nothing worth measuring.
#[derive(Default)]
pub struct NoteTracker {
    held: BTreeMap<VoiceKey, Voice>,
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
            NoteEventKind::SessionReset => self.session_notes_off(event.time),
            NoteEventKind::SourceReset => self.source_notes_off(event.source, event.time),
            NoteEventKind::On { velocity } => {
                // A retrigger without an Off silently replaces the held
                // voice (same key); the old voice gets no release fade.
                let voice =
                    Voice::new(event.source, event.channel, event.note, velocity, event.time);
                self.roll.note_on(event.key(), velocity, voice.pitch, event.time);
                self.held.insert(event.key(), voice);
            }
            NoteEventKind::Off => {
                if let Some(mut voice) = self.held.remove(&event.key()) {
                    self.stamp_ends_worn(&mut voice);
                    voice.state = VoiceState::Released { at: event.time };
                    self.released.push(voice);
                    self.roll.note_off(event.key(), event.time);
                }
            }
            NoteEventKind::Tuning { semitones } => {
                if let Some(voice) = self.held.get_mut(&event.key()) {
                    // Octave indicators track the sounding pitch too.
                    voice.set_pitch(f32::from(event.note) + semitones);
                    self.roll.bend(event.key(), event.time, voice.pitch);
                }
            }
        }
        // Every arm reaching here can move the chord's ends: two change which
        // voices are held, and a tuning can bend one past its neighbour. An
        // arm that changes nothing (an off for a key that is not down)
        // restamps to the same answer, `restamp_ends` being a re-read rather
        // than a reset — which is also why the `SessionReset` arm having already
        // restamped inside `session_notes_off` costs nothing. That call is for the
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
        let key = voice.key();
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
    /// on screen. The two part company for the whole of a note's arrival,
    /// which the release waits out ([`Voice::release_level`]): a note switched
    /// off in the same instant it arrived reads an activation of 0 while its
    /// release has not started at all — read that way, a zero-length note
    /// would be dropped before it ever drew.
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
    /// `(source, channel, note)` order, then the released ones in the order they
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

    pub fn session_notes_off(&mut self, now: Time) {
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

    /// A source leaving/resetting cannot release another source's held set.
    /// Keep the same release fade and held-end stamps as a session reset.
    pub fn source_notes_off(&mut self, source: SourceId, now: Time) {
        self.roll.source_off(source, now);
        let high = self.high_end;
        let low = self.low_end;
        let released = &mut self.released;
        self.held.retain(|key, voice| {
            if key.source != source {
                return true;
            }
            let mut voice = *voice;
            let worn = |end: Option<HeldEnd>| {
                end.filter(|e| e.key == *key && e.since >= voice.on_time).map(|e| e.since)
            };
            voice.wore_high = worn(high);
            voice.wore_low = worn(low);
            voice.state = VoiceState::Released { at: now };
            released.push(voice);
            false
        });
        self.restamp_ends(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_key_sources_keep_independent_lifetimes_bends_and_ends() {
        let (a, b) = (SourceId(1), SourceId(2));
        let mut tracker = NoteTracker::new();
        let bend = |time, source, semitones| NoteEvent {
            time,
            source,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Tuning { semitones },
        };
        tracker.handle_event(NoteEvent::on(0.0, a, 0, 60, 0.8));
        tracker.handle_event(NoteEvent::on(0.0, b, 0, 60, 0.6));
        tracker.handle_event(bend(0.01, a, 0.25));
        tracker.handle_event(bend(0.02, b, -0.25));
        assert_eq!(tracker.held_count(), 2);
        let bass = tracker.lowest_held().unwrap();
        assert_eq!(bass.key.source, b);
        assert_eq!(tracker.highest_held().unwrap().key.source, a);

        tracker.handle_event(NoteEvent::off(1.0, a, 0, 60));
        let released = tracker.voices().find(|v| v.source == a).unwrap();
        assert_eq!(released.wore_high, Some(0.01));
        assert_eq!(released.wore_low, None, "B owns the bass stamp despite the same channel/key");
        assert_eq!(tracker.lowest_held(), Some(bass));
        tracker.handle_event(NoteEvent::on(2.0, a, 0, 60, 0.7));
        tracker.handle_event(NoteEvent::on(3.0, a, 0, 60, 0.9));
        assert_eq!(tracker.held_count(), 2, "same-source retrigger still replaces");
        assert_eq!(tracker.voices().find(|v| v.source == b).unwrap().pitch, 59.75);
        tracker.handle_event(NoteEvent::source_reset(4.0, a));
        assert_eq!(tracker.held_count(), 1);
        assert_eq!(tracker.lowest_held(), Some(bass));
        tracker.handle_event(bend(4.5, b, -0.5));
        let roll_b = tracker.roll().notes().find(|n| n.source == b).unwrap();
        assert!(roll_b.is_live());
        assert_eq!((roll_b.start, roll_b.settled_pitch()), (0.0, 59.75));
        assert_eq!(
            roll_b.segments(5.0).collect::<Vec<_>>(),
            vec![
                ((0.0, 60.0), (0.02, 59.75)),
                ((0.02, 59.75), (4.5, 59.5)),
                ((4.5, 59.5), (5.0, 59.5)),
            ]
        );
        let ends_a: Vec<_> =
            tracker.roll().notes().filter(|n| n.source == a).map(|n| (n.start, n.end)).collect();
        assert_eq!(ends_a, vec![(0.0, Some(1.0)), (2.0, Some(3.0)), (3.0, Some(4.0))]);

        tracker.handle_event(NoteEvent::on(5.0, a, 0, 60, 0.8));
        tracker.handle_event(NoteEvent::on(5.0, SourceId::DIRECT, 0, 60, 0.8));
        assert_eq!(tracker.held_count(), 3, "direct input has its own reserved identity");
        tracker.handle_event(NoteEvent::session_reset(6.0));
        assert_eq!(tracker.held_count(), 0);
        assert!(tracker.roll().notes().all(|n| !n.is_live()));
        assert_eq!(tracker.highest_held(), None);
        assert_eq!(tracker.lowest_held(), None);
    }

    fn on(time: Time, note: u8) -> NoteEvent {
        NoteEvent::on(time, crate::SourceId::DIRECT, 0, note, 0.8)
    }

    fn off(time: Time, note: u8) -> NoteEvent {
        NoteEvent::off(time, crate::SourceId::DIRECT, 0, note)
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
                tracker.handle_event(NoteEvent::on(
                    0.0,
                    crate::SourceId::DIRECT,
                    channel,
                    note,
                    0.8,
                ));
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
        tracker.session_notes_off(1.0);
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
        assert_eq!(
            tracker.highest_held(),
            Some(HeldEnd {
                key: VoiceKey { source: SourceId::DIRECT, channel: 0, note: 60 },
                since: 1.0
            })
        );
        assert_eq!(
            tracker.lowest_held(),
            Some(HeldEnd {
                key: VoiceKey { source: SourceId::DIRECT, channel: 0, note: 60 },
                since: 1.0
            })
        );

        // A note inside the chord moves neither end, and must not restamp
        // the ends it did not take.
        tracker.handle_event(on(2.0, 55));
        tracker.handle_event(on(3.0, 57));
        assert_eq!(
            tracker.highest_held(),
            Some(HeldEnd {
                key: VoiceKey { source: SourceId::DIRECT, channel: 0, note: 60 },
                since: 1.0
            })
        );
        assert_eq!(
            tracker.lowest_held(),
            Some(HeldEnd {
                key: VoiceKey { source: SourceId::DIRECT, channel: 0, note: 55 },
                since: 2.0
            })
        );

        // Lifting the top hands the melody DOWN, at the moment of the lift
        // rather than at the note-on of the voice that inherits it — which is
        // older than the chord and would leave nothing to ease.
        tracker.handle_event(off(4.0, 60));
        assert_eq!(
            tracker.highest_held(),
            Some(HeldEnd {
                key: VoiceKey { source: SourceId::DIRECT, channel: 0, note: 57 },
                since: 4.0
            })
        );

        // Pruning the voice that handed it over is not a change of ends. This
        // is the whole reason the stamp is kept here rather than read back off
        // the released tail, which the prune empties.
        tracker.prune(5.0, &Envelope { fade_time: 0.1, ..Envelope::default() });
        assert_eq!(tracker.voices().count(), 2, "the released C4 is gone");
        assert_eq!(
            tracker.highest_held(),
            Some(HeldEnd {
                key: VoiceKey { source: SourceId::DIRECT, channel: 0, note: 57 },
                since: 4.0
            })
        );

        // A retrigger with no off in between replaces the voice on a key it
        // already had, and that is a new note taking the end, not the old one
        // keeping it.
        tracker.handle_event(on(6.0, 57));
        assert_eq!(
            tracker.highest_held(),
            Some(HeldEnd {
                key: VoiceKey { source: SourceId::DIRECT, channel: 0, note: 57 },
                since: 6.0
            })
        );

        // A bend past a neighbour moves the end without any key changing:
        // MPE and per-note tuning are why the ends are compared on pitch.
        tracker.handle_event(NoteEvent {
            source: crate::SourceId::DIRECT,
            time: 7.0,
            channel: 0,
            note: 55,
            kind: NoteEventKind::Tuning { semitones: 6.0 },
        });
        assert_eq!(
            tracker.highest_held(),
            Some(HeldEnd {
                key: VoiceKey { source: SourceId::DIRECT, channel: 0, note: 55 },
                since: 7.0
            })
        );
        assert_eq!(
            tracker.lowest_held(),
            Some(HeldEnd {
                key: VoiceKey { source: SourceId::DIRECT, channel: 0, note: 57 },
                since: 7.0
            })
        );

        // A transport reset takes every held voice, so it takes both ends.
        tracker.session_notes_off(8.0);
        assert_eq!(ends(&tracker), (None, None));
    }

    #[test]
    fn tuning_bends_pitch_class_and_octave() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60)); // C4

        // Bend up a whole tone: D, still octave 4.
        tracker.handle_event(NoteEvent {
            source: crate::SourceId::DIRECT,
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
            source: crate::SourceId::DIRECT,
            time: 0.2,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Tuning { semitones: -1.0 },
        });
        let voice = tracker.voices().next().unwrap();
        assert_eq!(voice.octave, 3);
    }

    /// Every channel is held as a voice. Channel 15 is the one worth naming:
    /// it is v1's reserved channel, and nothing reserves it here — a note on
    /// it sounds on the lattice like a note anywhere else, colored by its own
    /// pitch.
    #[test]
    fn no_channel_is_dropped_on_the_way_in() {
        for channel in 0..16u8 {
            let mut tracker = NoteTracker::new();
            tracker.handle_event(NoteEvent::on(0.0, crate::SourceId::DIRECT, channel, 60, 0.8));
            assert_eq!(tracker.voices().count(), 1, "channel {channel}");
        }
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

    /// What the Fade curve bar buys: 0 is the straight line the lattice fades
    /// on, and turning it up moves the fade EARLIER without moving
    /// either end. Written as an ordering rather than as numbers so it states
    /// the property the bar promises instead of restating the formula.
    #[test]
    fn a_higher_shape_front_loads_the_fade() {
        let level = |shape: f32| {
            Envelope { fade_time: 2.0, shape, ..Envelope::default() }.release(1.0, 0.0)
        };
        assert!((level(0.0) - 0.5).abs() < 1e-6, "0 is the straight line: half gone at half way");
        let mut prev = level(0.0);
        for shape in [0.25, 0.5, 0.75, 1.0] {
            let now = level(shape);
            assert!(now < prev, "shape {shape} leaves less at half way than the shape under it");
            prev = now;
        }
        assert!(prev < 0.1, "and the top of the bar is most of the way gone by half way");
    }

    /// A note released while it is still arriving reaches full anyway, and
    /// then leaves on the whole fade: the departure waits for the arrival to
    /// land ([`Voice::release_level`]). This is what lets ONE duration drive
    /// both ends — multiplied instead, a quarter of an arrival would be a
    /// quarter of the brightness for the rest of the note's life.
    #[test]
    fn a_note_shorter_than_its_arrival_still_reaches_full() {
        // Both ends on one duration, the way `ViewConfig::envelope` builds it.
        let env = Envelope { attack_time: 0.2, fade_time: 0.2, shape: 0.0 };
        let mut voice = Voice::new(SourceId::DIRECT, 0, 60, 1.0, 0.0);
        // A key up a quarter of the way in, which is where a multiplied
        // envelope would peak.
        voice.state = VoiceState::Released { at: 0.05 };

        assert_eq!(voice.activation(0.1, &env), 0.5, "still climbing, key up or not");
        assert_eq!(voice.activation(0.2, &env), 1.0, "and full at the end of its arrival");
        // The fade then runs from THERE rather than from the key, so it is a
        // whole fade and not the tail of one that started early.
        assert_eq!(voice.activation(0.3, &env), 0.5, "half the fade later, half gone");
        assert_eq!(voice.activation(0.4, &env), 0.0, "gone one fade after it landed");

        // The peak over the note's whole life, so nothing above depends on
        // having guessed where it is.
        let peak =
            (0..=100).map(|i| voice.activation(f64::from(i) * 0.01, &env)).fold(0.0f32, f32::max);
        assert_eq!(peak, 1.0, "a note is never dimmed for having been short");
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
    /// straight through its own wait — and it would do it at the low end of
    /// the Fade, a duration of 0 being an ordinary setting rather than a
    /// corner.
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
        let mut voice = Voice::new(SourceId::DIRECT, 0, 60, 1.0, 0.0);
        voice.state = VoiceState::Released { at: 0.0 };
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let fades = Envelope { attack_time: 0.0, fade_time: bad, shape: 0.0 };
            assert_eq!(voice.release_level(1.0, &fades), 0.0, "fade_time {bad} ends the note");

            // The ATTACK time reaches the release as well, being what the
            // departure waits on, so it needs the same answer: a note that
            // ends rather than one held at full for the rest of the session.
            let arrives = Envelope { attack_time: bad, fade_time: 1.0, shape: 0.0 };
            assert_eq!(voice.release_level(2.0, &arrives), 0.0, "attack_time {bad} ends it too");

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

    /// [`Envelope::carried`] walks the same curve [`Envelope::attack`] does,
    /// and lands in the same place: a level carried from 0 for `t` is the
    /// attack at `t`, at any shape.
    ///
    /// The claim that makes the two readings one curve. What it is against is a
    /// second easing written beside the first — the audio ring's gate would
    /// then fade on a shape the Fade curve bar does not draw, and the bar's own
    /// preview would be telling the truth about the notes and not about the
    /// rings.
    #[test]
    fn a_carried_level_walks_the_curve_the_attack_does() {
        for shape in [0.0f32, 0.35, 1.0] {
            let env = Envelope { attack_time: 2.0, fade_time: 2.0, shape };
            for t in [0.25f64, 0.5, 1.0, 1.9] {
                let carried = env.carried(0.0, t, true);
                let attacked = env.attack(t, 0.0);
                assert!(
                    (carried - attacked).abs() < 1e-5,
                    "shape {shape} at {t}s: carried {carried}, attacked {attacked}",
                );
                // ...and the departure is that mirrored, which is what
                // `release` answers off the same approach.
                let left = env.carried(1.0, t, false);
                let released = env.release(t, 0.0);
                assert!(
                    (left - released).abs() < 1e-5,
                    "shape {shape} at {t}s: carried {left} left, released {released}",
                );
            }
        }
    }

    /// Carrying a level in STEPS lands where carrying it in one does, because
    /// progress is linear in time and the curve over it is fixed.
    ///
    /// Not a nicety: the live pane steps this per frame at whatever rate the
    /// host is running and the offline renderer steps it at the export's own,
    /// so a transition that depended on how it was cut up would draw one
    /// picture on screen and another in the mp4.
    #[test]
    fn carrying_a_level_in_steps_lands_where_one_step_does() {
        let env = Envelope { attack_time: 1.0, fade_time: 1.0, shape: 0.6 };
        let once = env.carried(0.0, 0.5, true);
        let mut stepped = 0.0;
        for _ in 0..20 {
            stepped = env.carried(stepped, 0.025, true);
        }
        assert!((once - stepped).abs() < 1e-4, "one step {once}, twenty steps {stepped}");
        // A step of nothing is the picture standing still, which is what keeps
        // a pane drawn twice in one frame from running the fade twice.
        assert_eq!(env.carried(0.4, 0.0, true), 0.4);
        assert_eq!(env.carried(0.4, 0.0, false), 0.4);
        // A clock that moves BACK holds as well: it is a picture from another
        // moment rather than a transition to run backward.
        assert_eq!(env.carried(0.4, -3.0, false), 0.4);
    }

    /// A carried level ARRIVES: past the duration it is exactly 1 or exactly 0,
    /// which is what lets an idle node whose ring has gone stop being drawn at
    /// all rather than carrying a ring nobody can see.
    #[test]
    fn a_carried_level_lands_exactly_on_its_target() {
        let env = Envelope { attack_time: 0.5, fade_time: 0.5, shape: 0.8 };
        assert_eq!(env.carried(0.3, 0.5, false), 0.0);
        assert_eq!(env.carried(0.3, 900.0, false), 0.0);
        assert_eq!(env.carried(0.3, 0.5, true), 1.0);
    }

    /// The guards, which reach this the way they reach the ramp: a duration
    /// that is not a positive real number is no ramp to walk, so the transition
    /// STEPS to where it was going, and so does a clock that is not a number.
    /// Both make a ring appear or vanish at once rather than hanging part way
    /// through a fade that can never finish.
    #[test]
    fn a_non_finite_carry_steps_rather_than_hangs() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
            let env = Envelope { attack_time: bad, fade_time: bad, shape: 0.0 };
            assert_eq!(env.carried(0.5, 0.1, true), 1.0, "attack_time {bad} hung on the way in");
            assert_eq!(env.carried(0.5, 0.1, false), 0.0, "fade_time {bad} hung on the way out");
        }
        let env = Envelope { attack_time: 1.0, fade_time: 1.0, shape: 0.0 };
        assert_eq!(env.carried(0.5, f64::NAN, false), 0.0, "a NaN clock hung the fade");
        assert_eq!(env.carried(f32::NAN, 0.1, false), 0.0, "a NaN level hung the fade");
        // A poisoned SHAPE straightens, the durations being real: the level
        // still moves, and it moves on the line every layer fades on.
        let curve = Envelope { attack_time: 1.0, fade_time: 1.0, shape: f32::NAN };
        assert_eq!(curve.carried(1.0, 0.25, false), 0.75);
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
        tracker.handle_event(NoteEvent::on(0.0, crate::SourceId::DIRECT, 3, 64, 0.5));
        tracker.handle_event(NoteEvent {
            source: crate::SourceId::DIRECT,
            time: 1.0,
            channel: 0,
            note: 0,
            kind: NoteEventKind::SessionReset,
        });
        assert_eq!(tracker.held_count(), 0);
        // Released voices fade out rather than vanish.
        tracker.prune(1.5, &Envelope::default());
        assert_eq!(tracker.voices().count(), 2);
        tracker.prune(2.1, &Envelope::default());
        assert_eq!(tracker.voices().count(), 0);
    }
}
