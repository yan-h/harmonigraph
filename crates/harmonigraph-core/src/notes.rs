//! Tracking of active and recently-released MIDI voices.
//!
//! The plugin's audio thread converts host MIDI into [`NoteEvent`]s and
//! ships them to the GUI over a lock-free ring buffer; the standalone dev
//! harness generates them from a mock source. Either way, the GUI thread
//! owns a [`NoteTracker`] and feeds every event into it.

use std::collections::{BTreeMap, BTreeSet};

use crate::canonical::{
    CanonicalEvent, InvalidCanonical, PublicationGap, SourceBaseline, VoiceBaseline,
};
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
    /// Any other per-note expression the picture can read, in the units it
    /// arrived in (see [`Expression`]).
    Expression {
        expression: Expression,
        value: f32,
    },
    /// Release this event's source only. Channel and note are ignored.
    SourceReset,
    /// Release every source. Source, channel and note are ignored.
    SessionReset,
}

/// A per-note expression besides tuning: the three Bitwig writes as a note's
/// Pressure, Gain and Timbre, which reach a CLAP plugin as the pressure,
/// volume and brightness note expressions.
///
/// Recorded as the host states them, so a take keeps what was played and a
/// reader decides what it means. Pan, vibrato and CLAP's generic `expression`
/// are dropped at the shell: nothing reads them.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Expression {
    /// 0 to 1, released to pressed hard.
    Pressure,
    /// Linear amplitude where 1 is unity (CLAP's note volume). CLAP promises
    /// at most 4 (+12 dB); nothing caps it here, so a take keeps what a host
    /// sent. Bitwig's Gain lane reaches +18 dB but arrives capped at exactly 4
    /// (measured 2026-09-25, uncapped build), so +12 dB and up read as one.
    Gain,
    /// 0 to 1 (CLAP's brightness; MPE's CC 74).
    Timbre,
}

impl Expression {
    /// The range the host promises, which a delta outside is refused for.
    pub fn range(self) -> std::ops::RangeInclusive<f32> {
        match self {
            Expression::Pressure | Expression::Timbre => 0.0..=1.0,
            Expression::Gain => 0.0..=f32::MAX,
        }
    }

    /// A host's value brought into [`range`](Self::range), or `None` when it
    /// is not a number. Clamped rather than refused at the shell, because a
    /// canonical delta out of range is a publication gap and a host rounding a
    /// hair past its own limit should not cost one.
    pub fn accept(self, value: f32) -> Option<f32> {
        let range = self.range();
        value.is_finite().then(|| value.clamp(*range.start(), *range.end()))
    }
}

/// Where each [`Expression`] stands on one note.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Expressions {
    pub pressure: f32,
    pub gain: f32,
    pub timbre: f32,
}

impl Expressions {
    /// A note nothing has touched: no pressure, unity gain, and timbre at the
    /// centre MPE resets CC 74 to. A host that never sends an expression
    /// leaves every note here.
    pub const NEUTRAL: Self = Self { pressure: 0.0, gain: 1.0, timbre: 0.5 };

    pub fn set(&mut self, expression: Expression, value: f32) {
        match expression {
            Expression::Pressure => self.pressure = value,
            Expression::Gain => self.gain = value,
            Expression::Timbre => self.timbre = value,
        }
    }

    pub fn valid(self) -> bool {
        [
            (Expression::Pressure, self.pressure),
            (Expression::Gain, self.gain),
            (Expression::Timbre, self.timbre),
        ]
        .into_iter()
        .all(|(expression, value)| expression.range().contains(&value))
    }
}

impl Default for Expressions {
    fn default() -> Self {
        Self::NEUTRAL
    }
}

/// A note's MIDI channel carries no meaning here. It is kept on [`Voice`] and
/// [`NoteEvent`] because it is part of a note's IDENTITY — the host's key for
/// matching an off to its on, and what lets two lanes hold the same note
/// number at once — and for nothing else. Every channel is tracked and
/// colored by pitch height on the gradient, so two notes
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
    /// setting on screen. The audio
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
    /// Where the note's expressions stand now; [`Expressions::NEUTRAL`] until
    /// the host sends one.
    pub expressions: Expressions,
    pub on_time: Time,
    /// Presentation timestamp before a shell's moving clock offset.
    original_onset: Time,
    /// Whether this voice's source was visible at its FACTUAL release —
    /// the single question a released voice is asked, and the answer to both
    /// halves of it. [`voices`](NoteTracker::voices) draws the released tail
    /// on this and [`prune`](NoteTracker::prune) records on it, so a voice
    /// that was on screen when it let go finishes its fade AND lands in the
    /// history, and one that was not does neither.
    ///
    /// Stamped once, at the release, so neither answer depends on the frame
    /// or prune cadence that happens to follow it.
    ///
    /// Meaningless while a voice is held, and never read there: the held half
    /// of `voices()` filters on the CURRENT `hidden_sources` instead, so
    /// hiding a source still clears everything it has DOWN from the screen at
    /// once. This decides only what happens to a voice on its way out.
    visible_at_release: bool,
    /// Canonical accepted lifetime, absent for ordinary direct observations.
    pub lifetime: Option<u64>,
    pub assignment: Option<crate::canonical::AssignmentMetadata>,
    pub state: VoiceState,
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
            expressions: Expressions::NEUTRAL,
            on_time,
            original_onset: on_time,
            visible_at_release: true,
            lifetime: None,
            assignment: None,
            state: VoiceState::Held,
        };
        voice.set_pitch(f32::from(note));
        voice
    }

    pub fn key(&self) -> VoiceKey {
        VoiceKey { source: self.source, channel: self.channel, note: self.note }
    }

    /// Keep the sounding pitch and its pitch class in agreement on every bend.
    fn set_pitch(&mut self, pitch: f32) {
        self.pitch = pitch;
        self.pitch_class = PitchClass::from_cents(pitch * 100.0);
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
    /// Lattice animation carries its own levels in the scene layer; this
    /// envelope remains the voice-based reading used by the other consumers.
    pub fn activation(&self, now: Time, env: &Envelope) -> f32 {
        env.attack(now, self.on_time) * self.release_level(now, env)
    }
}

/// The display octave containing MIDI note `midi`, in Bitwig's convention
/// where middle C (MIDI 60) is C3. The inverse of [`octave_start_midi`].
/// Use this rather than rewriting `/ 12 - 2` by hand, so the convention
/// lives in one place.
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
/// Held voices iterate in `(source, channel, note)` order. Batch releases
/// preserve that order in the released tail and history as well.
#[derive(Default)]
pub struct NoteTracker {
    held: BTreeMap<VoiceKey, Voice>,
    released: Vec<Voice>,
    history: NoteHistory,
    roll: NoteRoll,
    canonical: BTreeMap<SourceId, CanonicalCursor>,
    hidden_sources: BTreeSet<SourceId>,
    baselines: BTreeMap<SourceId, SourceBaseline>,
    gaps: Vec<PublicationGap>,
    certainty: Certainty,
}

/// Whose published state is currently in doubt.
///
/// One field rather than a flag beside two sets, which is what it was: a
/// `bool` for "everything", a set of sources a gap had closed, and a set of
/// sources that had republished since. The three were one predicate — the two
/// sets were kept disjoint by hand at five sites, and which of them the
/// predicate READ depended on the flag, so the other was dead weight that
/// still had to be written in step.
///
/// Said as one type it is a default and its exceptions, and a stream-wide gap
/// is simply which side of that default the exceptions sit on.
enum Certainty {
    /// Every source is certain but these, each closed by its own gap.
    AllBut(BTreeSet<SourceId>),
    /// No source is certain but these, each republished since a stream-wide
    /// gap. Sources this tracker has never heard from are in doubt too, which
    /// is why this cannot be a set of the uncertain ones.
    NoneBut(BTreeSet<SourceId>),
}

impl Default for Certainty {
    fn default() -> Self {
        Self::AllBut(BTreeSet::new())
    }
}

impl Certainty {
    fn certain(&self, source: SourceId) -> bool {
        match self {
            Self::AllBut(doubted) => !doubted.contains(&source),
            Self::NoneBut(restored) => restored.contains(&source),
        }
    }

    /// A gap. `None` scopes it to the whole canonical stream, which puts every
    /// source in doubt — including the ones no gap has named.
    fn doubt(&mut self, source: Option<SourceId>) {
        match (source, &mut *self) {
            (Some(source), Self::AllBut(doubted)) => {
                doubted.insert(source);
            }
            (Some(source), Self::NoneBut(restored)) => {
                restored.remove(&source);
            }
            (None, _) => *self = Self::NoneBut(BTreeSet::new()),
        }
    }

    /// One source's complete baseline or reset, which is the only thing that
    /// answers for its completeness (see
    /// [`NoteTracker::source_current_certain`]).
    fn restore(&mut self, source: SourceId) {
        match self {
            Self::AllBut(doubted) => {
                doubted.remove(&source);
            }
            Self::NoneBut(restored) => {
                restored.insert(source);
            }
        }
    }
}

fn baseline_matches(row: &VoiceBaseline, voice: &Voice) -> bool {
    row.channel == voice.channel
        && row.note == voice.note
        && if row.lifetime == 0 {
            voice.lifetime.is_none() && row.actual_onset == voice.original_onset
        } else {
            voice.lifetime == Some(row.lifetime)
        }
}

#[derive(Default)]
struct CanonicalCursor {
    output: u64,
    baseline: u64,
    state_cut: u64,
}

impl NoteTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume one canonical item. False identifies a duplicate, not a new
    /// musical event. This non-RT consumer cannot acknowledge source journals.
    pub fn handle_canonical(
        &mut self,
        event: CanonicalEvent<'_>,
    ) -> Result<bool, InvalidCanonical> {
        self.handle_canonical_mapped(event, 0.0)
    }

    /// Map drawing times while retaining the source's immutable onset identity.
    pub fn handle_canonical_mapped(
        &mut self,
        event: CanonicalEvent<'_>,
        offset: Time,
    ) -> Result<bool, InvalidCanonical> {
        if !offset.is_finite() {
            return Err(InvalidCanonical);
        }
        match event {
            CanonicalEvent::Note(mut delta) => {
                delta.validate()?;
                let original_onset = delta.event.time;
                delta.event.time += offset;
                delta.validate()?;
                if delta.sequence != 0 {
                    let cursor = self.canonical.entry(delta.event.source).or_default();
                    if delta.sequence <= cursor.output {
                        return Ok(false);
                    }
                    // Available history must precede its baseline. A baseline
                    // is not permission to silently discard late history.
                    if delta.sequence <= cursor.state_cut {
                        return Err(InvalidCanonical);
                    }
                    cursor.output = delta.sequence;
                }
                let key = delta.event.key();
                if matches!(delta.event.kind, NoteEventKind::Off) && delta.lifetime != 0 {
                    self.roll.observed_release(key, delta.lifetime, delta.event.time);
                }
                if !matches!(
                    delta.event.kind,
                    NoteEventKind::On { .. }
                        | NoteEventKind::SourceReset
                        | NoteEventKind::SessionReset
                ) && delta.lifetime != 0
                    && self
                        .held
                        .get(&key)
                        .is_some_and(|voice| voice.lifetime != Some(delta.lifetime))
                {
                    return Ok(true);
                }
                self.handle_event(delta.display_event());
                if let Some(voice) = self.held.get_mut(&key) {
                    if delta.assignment.is_some() {
                        voice.assignment = delta.assignment;
                    }
                }
                if matches!(delta.event.kind, NoteEventKind::On { .. }) {
                    if let Some(voice) = self.held.get_mut(&key) {
                        voice.lifetime = (delta.lifetime != 0).then_some(delta.lifetime);
                        voice.original_onset = original_onset;
                        if let Some(pitch) = delta.pitch_microcents {
                            voice.set_pitch((pitch as f64 / 100_000_000.0) as f32);
                            self.roll.bend(key, delta.event.time, voice.pitch);
                        }
                    }
                    self.roll.set_identity(
                        key,
                        (delta.lifetime != 0).then_some(delta.lifetime),
                        original_onset,
                    );
                }
            }
            CanonicalEvent::Baseline(frame) => return self.replace_source_mapped(frame, offset),
            CanonicalEvent::Gap(mut gap) => {
                gap.validate()?;
                gap.time += offset;
                gap.through += offset;
                gap.validate()?;
                self.certainty.doubt(gap.source);
                self.roll.gap(gap.source, gap.time);
                // The same reach the roll's own gap has, and the same exit every
                // other departure takes: what publication lost is no longer known
                // to be held, so those voices leave with a release fade rather than being
                // dropped between two frames. A stream-wide gap is the one
                // production emits, and it takes every source with it.
                self.release_held(gap.time, |key, _| {
                    gap.source.is_none_or(|source| key.source == source)
                });
                self.gaps.push(gap);
                if self.gaps.len() > NoteRoll::MAX_NOTES {
                    self.gaps.remove(0);
                }
            }
        }
        Ok(true)
    }

    /// Validate the entire held set before mutation. Matching lifetimes keep
    /// their onset and bend history. Missing observations
    /// are closed as history gaps, never fictional downstream releases.
    pub fn replace_source(&mut self, frame: &SourceBaseline) -> Result<bool, InvalidCanonical> {
        self.replace_source_mapped(frame, 0.0)
    }

    fn replace_source_mapped(
        &mut self,
        frame: &SourceBaseline,
        offset: Time,
    ) -> Result<bool, InvalidCanonical> {
        frame.validate()?;
        let mut mapped = *frame;
        mapped.translate(offset);
        mapped.validate()?;
        if self.canonical.get(&frame.source).is_some_and(|cursor| frame.id <= cursor.baseline) {
            return Ok(false);
        }
        if self.canonical.get(&frame.source).is_some_and(|cursor| cursor.output > frame.output_cut)
        {
            return Err(InvalidCanonical);
        }
        let voices = frame.voices();
        if frame.participating {
            self.hidden_sources.remove(&frame.source);
        } else {
            self.hidden_sources.insert(frame.source);
        }
        self.roll.set_participating(frame.source, frame.participating);
        self.roll.replace_source(frame.source, voices, mapped.time, offset);
        self.held.retain(|key, voice| {
            key.source != frame.source || voices.iter().any(|row| baseline_matches(row, voice))
        });
        // The same identity question asked of the released tail, and the reason
        // it has to be asked there too is that a departure can be WITHDRAWN. A
        // publication gap releases what it could not see, because what it
        // cannot see may have ended and a fade is the guess the picture
        // recovers from; this frame is the source saying the note never
        // stopped. Leaving the copy in place would draw the note twice and
        // then, at the end of a fade it is not having, hand a still-sounding
        // pitch to [`NoteHistory`] — a trail mark on a node the music has not
        // left (#936). A row this baseline does NOT list is a genuine
        // departure and keeps its fade.
        //
        // KNOWN GAP, accepted deliberately: this only reaches a copy that is
        // still IN the tail. A `prune` landing between the outage and this
        // frame has already recorded the mark and dropped the voice, and there
        // is nothing left to withdraw — at a `Fade` near 0 that is every time,
        // since the release is over on the very next frame. What that costs is
        // a trail mark arriving early on a node whose note is still sounding;
        // `NoteHistory::record` is idempotent per cent-key and only the trail
        // reads it, so the note's real release overwrites it. Closing it needs
        // a third note-lifecycle state — released, and released-but-unconfirmed
        // — which is not worth carrying for a source that has lost contact with
        // the thing telling it what is playing (Yan's call, 2026-09-19).
        self.released.retain(|voice| {
            voice.source != frame.source || !voices.iter().any(|row| baseline_matches(row, voice))
        });
        for row in voices {
            let onset = self.roll.live_onset(row.key(frame.source)).unwrap();
            let voice = self.held.entry(row.key(frame.source)).or_insert_with(|| {
                let mut voice =
                    Voice::new(frame.source, row.channel, row.note, row.velocity, onset);
                voice.lifetime = (row.lifetime != 0).then_some(row.lifetime);
                voice.original_onset = row.actual_onset;
                voice
            });
            voice.set_pitch(row.pitch());
            voice.expressions = row.expressions;
            voice.assignment = row.metadata();
        }
        let cursor = self.canonical.entry(frame.source).or_default();
        cursor.baseline = frame.id;
        cursor.state_cut = frame.output_cut;
        self.baselines.insert(frame.source, mapped);
        self.certainty.restore(frame.source);

        Ok(true)
    }

    pub fn source_baseline(&self, source: SourceId) -> Option<&SourceBaseline> {
        self.baselines.get(&source)
    }

    /// New note deltas establish individual lifetimes, never completeness of
    /// a source after reporting loss. Only its complete baseline/reset does.
    pub fn source_current_certain(&self, source: SourceId) -> bool {
        self.certainty.certain(source)
    }

    pub fn publication_gaps(&self) -> &[PublicationGap] {
        &self.gaps
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
                if let Some(voice) = self.held.remove(&event.key()) {
                    self.release_voice(voice, event.time);
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
            NoteEventKind::Expression { expression, value } => {
                if let Some(voice) = self.held.get_mut(&event.key()) {
                    voice.expressions.set(expression, value);
                    self.roll.express(event.key(), event.time, voice.expressions);
                }
            }
        }
    }

    /// Preserve the release time and the source visibility at that moment.
    fn release_voice(&mut self, mut voice: Voice, at: Time) {
        voice.state = VoiceState::Released { at };
        voice.visible_at_release = !self.hidden_sources.contains(&voice.source);
        self.released.push(voice);
    }

    /// Release matching voices in key order, preserving the released tail order.
    fn release_held(&mut self, at: Time, leaving: impl Fn(&VoiceKey, &Voice) -> bool) {
        for (key, voice) in std::mem::take(&mut self.held) {
            if leaving(&key, &voice) {
                self.release_voice(voice, at);
            } else {
                self.held.insert(key, voice);
            }
        }
    }

    /// Drop released voices whose fade has fully completed, folding each
    /// into the history as it goes. Call once per frame before iterating.
    ///
    /// A voice becomes a memory in the same step it stops being drawn, so
    /// the two never describe one note at once and a trail picks the note
    /// up exactly where its fade lets go. Drawn and remembered are one
    /// question here, asked once: [`Voice::visible_at_release`] decides both,
    /// so a voice whose source was hidden when it let go is never drawn AND
    /// never recorded, and one visible then finishes its fade on screen and
    /// lands in the history even if its source is hidden mid-fade. (A
    /// retrigger without an off replaces its voice outright — see
    /// `handle_event` — so that voice is never recorded; the retrigger's own
    /// release covers the pitch. A resuming baseline withdraws a release the
    /// same way, see [`replace_source`](Self::replace_source): a note a
    /// publication gap let go of and the source then says is still sounding
    /// never reaches here at all, so the trail cannot mark a node the music
    /// has not left — as far as this gets, which is only while the voice is
    /// still in the tail for the baseline to find. `replace_source` carries
    /// what that leaves open.)
    ///
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
            if voice.visible_at_release {
                history.record(voice, now);
            }
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
    /// The order is part of the contract for callers choosing between voices.
    ///
    /// The two halves ask visibility of different state, and that is the
    /// whole rule. A HELD voice is filtered on the CURRENT `hidden_sources`,
    /// so hiding a source takes everything it is holding off the screen in
    /// that instant. A RELEASED one is filtered on
    /// [`Voice::visible_at_release`], decided once when it left the held set
    /// — the same flag [`prune`](Self::prune) records on, which is what makes
    /// "drawn" and "remembered" one answer instead of two that can disagree
    /// (#905). The visible cost is that hiding a source does not cut its
    /// already-fading notes: they finish the fade they were drawing.
    ///
    /// One case looks like an exception to that and is really the two halves
    /// changing hands. A baseline that arrives NOT participating and still
    /// lists a note a publication gap had released withdraws that release
    /// ([`replace_source`](Self::replace_source)) — so the voice is no longer
    /// a fading one whose fade is protected, it is a HELD one, and the held
    /// rule cuts it in that instant along with everything else the source is
    /// holding. It leaves no trail mark either, which is the same answer read
    /// through [`prune`](Self::prune): the source says the note never stopped,
    /// and a note that never stopped is neither drawn as departing nor
    /// remembered as departed.
    pub fn voices(&self) -> impl Iterator<Item = &Voice> {
        self.held
            .values()
            .filter(|v| !self.hidden_sources.contains(&v.source))
            .chain(self.released.iter().filter(|v| v.visible_at_release))
    }

    pub fn held_count(&self) -> usize {
        self.held.values().filter(|v| !self.hidden_sources.contains(&v.source)).count()
    }

    pub fn session_notes_off(&mut self, now: Time) {
        self.certainty = Certainty::default();
        self.roll.all_off(now);
        self.release_held(now, |_, _| true);
    }

    /// A source leaving/resetting cannot release another source's held set.
    /// Keep the same release fade as a session reset.
    pub fn source_notes_off(&mut self, source: SourceId, now: Time) {
        self.certainty.restore(source);
        self.roll.source_off(source, now);
        self.release_held(now, |key, _| key.source == source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_key_sources_keep_independent_lifetimes_and_bends() {
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

        tracker.handle_event(NoteEvent::off(1.0, a, 0, 60));
        let released = tracker.voices().find(|v| v.source == a).unwrap();
        assert_eq!(released.state, VoiceState::Released { at: 1.0 });
        assert_eq!(released.pitch, 60.25);
        tracker.handle_event(NoteEvent::on(2.0, a, 0, 60, 0.7));
        tracker.handle_event(NoteEvent::on(3.0, a, 0, 60, 0.9));
        assert_eq!(tracker.held_count(), 2, "same-source retrigger still replaces");
        assert_eq!(tracker.voices().find(|v| v.source == b).unwrap().pitch, 59.75);
        tracker.handle_event(NoteEvent::source_reset(4.0, a));
        assert_eq!(tracker.held_count(), 1);
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

    /// Held voices keep their documented key order regardless of arrival order.
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

    /// A released voice is decided ONCE, at its release (#905). The flag
    /// stamped there says both whether the rest of its fade is drawn and
    /// whether the trail picks the pitch up, so the two can never disagree —
    /// a voice hidden when it let go is neither drawn nor remembered, and one
    /// visible then is both, whatever its source does mid-fade. A HELD voice
    /// is the other question and still answers to the current state: hiding a
    /// source takes everything it is holding off the screen at once.
    #[test]
    fn a_released_voice_is_drawn_and_remembered_on_its_visibility_at_the_release() {
        let env = Envelope::default();
        let row = VoiceBaseline {
            note: 60,
            velocity: 0.8,
            pitch_microcents: 6_000_000_000,
            ..Default::default()
        };

        // Hidden at the release. The source stops participating while the
        // note is still down — its baseline still lists the note, so the
        // voice stays HELD and merely drops off the screen.
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        let hide = SourceBaseline::new(SourceId::DIRECT, 1, 1.0, 0, false, &[row]).unwrap();
        tracker.replace_source(&hide).unwrap();
        assert_eq!(tracker.voices().count(), 0, "a held voice cuts the instant its source hides");
        tracker.handle_event(off(2.0, 60));
        // The fixture has to actually reach the released tail, or every
        // assertion below passes on a voice that was dropped at the hide.
        assert_eq!(tracker.released.len(), 1, "the off released the hidden voice, not dropped it");
        assert!(!tracker.released[0].visible_at_release, "and stamped it invisible");
        // The source rejoins with two thirds of the fade still to run.
        let rejoin = SourceBaseline::new(SourceId::DIRECT, 2, 2.3, 0, true, &[]).unwrap();
        tracker.replace_source(&rejoin).unwrap();
        assert!(tracker.released[0].release_level(2.3, &env) > 0.0, "mid-fade at the rejoin");
        assert_eq!(tracker.voices().count(), 0, "a fade begun hidden is not handed back");
        tracker.prune(3.1, &env);
        assert!(tracker.history().is_empty(), "and what was never drawn is never remembered");

        // Visible at the release, hidden mid-fade: the opposite direction.
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(off(2.0, 60));
        let hide = SourceBaseline::new(SourceId::DIRECT, 1, 2.3, 0, false, &[]).unwrap();
        tracker.replace_source(&hide).unwrap();
        assert_eq!(tracker.voices().count(), 1, "a fade begun visible finishes on screen");
        assert!(tracker.voices().next().unwrap().activation(2.3, &env) > 0.0, "and is still lit");
        tracker.prune(2.9, &env);
        assert_eq!(tracker.voices().count(), 1, "still fading, still drawn");
        tracker.prune(3.1, &env);
        assert_eq!(tracker.voices().count(), 0, "gone when the fade is over");
        let visits: Vec<_> = tracker.history().visits().map(|v| (v.pitch, v.last_off)).collect();
        assert_eq!(visits, [(60.0, 2.0)], "and the trail picks it up where the fade let go");
    }

    #[test]
    fn tuning_bends_pitch_and_pitch_class() {
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

        // Bend down past the octave boundary: B3.
        tracker.handle_event(NoteEvent {
            source: crate::SourceId::DIRECT,
            time: 0.2,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Tuning { semitones: -1.0 },
        });
        let voice = tracker.voices().next().unwrap();
        assert_eq!(voice.pitch, 59.0);
        assert_eq!(voice.pitch_class, PitchClass::from_midi_note(11));
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
    fn display_octave_uses_bitwig_c3_convention() {
        assert_eq!(display_octave_of(60), 3);
        assert_eq!(display_octave_of(59), 2);
        assert_eq!(octave_start_midi(3), 60);
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

    /// One voice as a canonical source PUBLISHES it, which is the only shape
    /// that carries a lifetime and so the only one a baseline can resume.
    ///
    /// `AcceptedOutput` is the load-bearing field and the reason this helper
    /// exists: `VoiceBaseline::default()` is `ObservedDirect`, which
    /// `SourceBaseline::validate` rejects on every source but
    /// [`SourceId::DIRECT`] — so the obvious fixture is refused, and refused
    /// for a reason that has nothing to do with the note. Saying
    /// `AcceptedOutput` then requires a nonzero lifetime and a real `onset`.
    fn published_voice(note: u8, lifetime: u64) -> VoiceBaseline {
        VoiceBaseline {
            note,
            lifetime,
            actual_onset: 1.0,
            onset: Some(crate::canonical::EventTiming {
                clock: crate::canonical::ClockId::default(),
                input: 0,
                planned: None,
                sample: 48_000,
                sample_rate: 48_000.0,
            }),
            pitch_microcents: i64::from(note) * 100_000_000,
            velocity: 0.8,
            provenance: crate::confirmed::PitchProvenance::AcceptedOutput,
            ..Default::default()
        }
    }

    /// Publication lost over one source, or over the whole canonical stream.
    fn gap(time: Time, source: Option<SourceId>) -> CanonicalEvent<'static> {
        CanonicalEvent::Gap(PublicationGap {
            source,
            time,
            through: time,
            first: 1,
            last: 1,
            reason: crate::canonical::GapReason::PublicationFull,
        })
    }

    /// A publication outage removes uncertain held voices through the usual
    /// release path, preserving their fade and eventual trail entry.
    #[test]
    fn a_publication_gap_releases_its_voices_rather_than_dropping_them() {
        let (a, b) = (SourceId(1), SourceId(2));
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent::on(0.0, a, 0, 60, 0.8));
        tracker.handle_event(NoteEvent::on(0.0, b, 0, 55, 0.8));

        assert_eq!(tracker.handle_canonical(gap(1.0, Some(a))), Ok(true));
        assert_eq!(tracker.held_count(), 1, "a gap still clears the lost source's held state");
        let lost = *tracker.voices().find(|v| v.source == a).unwrap();
        assert_eq!(lost.state, VoiceState::Released { at: 1.0 }, "it fades rather than popping");

        // The whole-stream gap takes every source at once, through the same
        // sequence — it is the arm a full publication lane actually takes.
        assert_eq!(tracker.handle_canonical(gap(2.0, None)), Ok(true));
        assert_eq!(tracker.held_count(), 0);
        assert_eq!(
            tracker.voices().find(|v| v.source == b).unwrap().state,
            VoiceState::Released { at: 2.0 }
        );

        // And the fade ends in the trail, where a pitch that was drawn right up
        // to the gap belongs.
        tracker.prune(4.0, &Envelope::default());
        assert_eq!(tracker.voices().count(), 0);
        let visited: Vec<_> = tracker.history().visits().map(|visit| visit.pitch).collect();
        assert_eq!(visited, vec![55.0, 60.0]);
    }

    /// A baseline states a voice's expressions along with its pitch: a note it
    /// opens starts at them, and a note it resumes after an outage takes the
    /// value it came back with as a step, rather than snapping to neutral.
    #[test]
    fn a_baseline_carries_its_voices_expressions() {
        let source = SourceId(1);
        let mut row = published_voice(60, 61);
        row.expressions.pressure = 0.5;
        let mut tracker = NoteTracker::new();
        let baseline = |id, time, row| SourceBaseline::new(source, id, time, 0, true, &[row]);
        assert_eq!(tracker.replace_source(&baseline(1, 2.0, row).unwrap()), Ok(true));
        assert_eq!(tracker.handle_canonical(gap(3.0, None)), Ok(true));
        row.expressions.pressure = 0.8;
        assert_eq!(tracker.replace_source(&baseline(2, 3.2, row).unwrap()), Ok(true));

        assert_eq!(tracker.held.values().next().unwrap().expressions.pressure, 0.8);
        let note = tracker.roll().notes().next().unwrap();
        let points: Vec<_> = note.expressions().iter().map(|(t, e)| (*t, e.pressure)).collect();
        assert_eq!(points, vec![(2.0, 0.5), (3.2, 0.5), (3.2, 0.8)]);
    }

    /// The other half of the outage above: publication comes BACK, and the
    /// repair baseline says the note never stopped (#936).
    ///
    /// The gap has to guess — what it cannot see may have ended — and it
    /// guesses the way the picture can recover from, releasing the voice into
    /// a fade. The baseline that follows is the source stating what it is
    /// sounding NOW, same lifetime, same onset, and that overrules the guess:
    /// the note is held again, resumed out of the roll's past rather than
    /// re-attacked. What must not survive that is the released COPY of it,
    /// which would otherwise draw the note a second time and then, at the end
    /// of a fade it is not having, hand a still-sounding pitch to the trail —
    /// [`NoteHistory`] marking a node the music has not left yet.
    ///
    /// The lifetime is what makes this fixture reach the resume at all: a
    /// voice from a bare note-on carries none, so a baseline would attach a
    /// fresh voice beside it instead of resuming this one, and every
    /// assertion below would pass on the wrong shape.
    ///
    /// The repair arrives with the copy still FADING, which is the case
    /// production actually takes — `tuning::Hub::flush` republishes within a
    /// callback or two of the outage. A prune landing in between instead is
    /// the known gap `replace_source` documents, deliberately not covered.
    #[test]
    fn a_resuming_baseline_takes_back_the_release_the_gap_handed_out() {
        let env = Envelope::default();
        let source = SourceId(1);
        let row = published_voice(60, 61);
        let mut tracker = NoteTracker::new();
        assert_eq!(
            tracker.replace_source(&SourceBaseline::new(source, 1, 2.0, 0, true, &[row]).unwrap()),
            Ok(true)
        );
        assert_eq!(
            tracker.held.values().next().map(|voice| voice.lifetime),
            Some(Some(61)),
            "a published voice carries the lifetime the resume matches on"
        );

        assert_eq!(tracker.handle_canonical(gap(3.0, None)), Ok(true));
        assert_eq!(tracker.released.len(), 1, "the outage released it rather than dropping it");
        assert!(tracker.released[0].visible_at_release, "on screen when it went, so recordable");

        // The repair: `tuning::Hub` republishes every live row's baseline once
        // an outage sets its `repair` flag, and those rows keep the lifetime
        // and onset they attacked with.
        assert_eq!(
            tracker.replace_source(&SourceBaseline::new(source, 2, 3.2, 0, true, &[row]).unwrap()),
            Ok(true)
        );
        assert_eq!(tracker.held_count(), 1, "the note is sounding again");
        assert_eq!(tracker.voices().count(), 1, "one note, drawn once");
        assert_eq!(tracker.roll().notes().count(), 1, "and one note in the roll to match");

        tracker.prune(4.5, &env);
        assert_eq!(tracker.held_count(), 1, "still down a fade-length later");
        assert!(tracker.history().is_empty(), "so the trail has nowhere to mark yet");

        // And it still arrives in the trail on the release it really has.
        tracker.source_notes_off(source, 5.0);
        tracker.prune(6.5, &env);
        let visits: Vec<_> = tracker.history().visits().map(|v| (v.pitch, v.last_off)).collect();
        assert_eq!(visits, [(60.0, 5.0)], "picked up where its own fade let go");
    }

    /// The one case where withdrawing a release also takes a note off the
    /// screen, written down because it reads like a contradiction of the
    /// `voices` contract and is not one.
    ///
    /// A repair baseline can arrive NOT participating — the source recovered
    /// and is hidden. It still lists the note, so the release is withdrawn and
    /// the voice goes back to being HELD, and the held half of `voices` cuts a
    /// hidden source's voices in the instant they hide. So a fade that was on
    /// screen stops, which `voices` otherwise promises not to do. The promise
    /// is about a note that is DEPARTING, and this one turned out not to be.
    #[test]
    fn a_hiding_repair_baseline_cuts_the_fade_it_withdraws() {
        let instant = Envelope { attack_time: 0.0, fade_time: 0.0, shape: 0.0 };
        let source = SourceId(1);
        let row = published_voice(60, 61);
        let mut tracker = NoteTracker::new();
        tracker
            .replace_source(&SourceBaseline::new(source, 1, 2.0, 0, true, &[row]).unwrap())
            .unwrap();
        tracker.handle_canonical(gap(3.0, None)).unwrap();
        assert_eq!(tracker.voices().count(), 1, "fading on screen, and visible at its release");

        tracker
            .replace_source(&SourceBaseline::new(source, 2, 3.1, 0, false, &[row]).unwrap())
            .unwrap();
        assert_eq!(tracker.voices().count(), 0, "cut, because it is held again and hidden");
        assert_eq!(tracker.held.len(), 1, "held is where it went — not dropped");
        assert_eq!(tracker.held_count(), 0, "just not counted while its source is hidden");
        tracker.prune(9.0, &instant);
        assert!(tracker.history().is_empty(), "a note that never stopped leaves no trail mark");
    }

    /// Certainty is a DEFAULT and its exceptions, and the stream-wide gap is
    /// what makes that shape necessary rather than tidy: it puts sources this
    /// tracker has never heard from in doubt, so no set of the uncertain ones
    /// could carry it. Nothing but a source's own complete baseline or reset
    /// answers for it afterwards.
    #[test]
    fn a_stream_wide_gap_doubts_even_the_sources_it_never_named() {
        let (a, b) = (SourceId(1), SourceId(2));
        let mut tracker = NoteTracker::new();
        assert!(tracker.source_current_certain(a), "nothing lost, nothing in doubt");

        tracker.handle_canonical(gap(1.0, Some(a))).unwrap();
        assert!(!tracker.source_current_certain(a));
        assert!(tracker.source_current_certain(b), "one source's gap is not another's");
        tracker.source_notes_off(a, 2.0);
        assert!(tracker.source_current_certain(a), "its own reset answers for it");

        tracker.handle_canonical(gap(3.0, None)).unwrap();
        for source in [a, b, SourceId(9)] {
            assert!(!tracker.source_current_certain(source), "{source:?} outlived a stream gap");
        }
        tracker.source_notes_off(b, 4.0);
        assert!(tracker.source_current_certain(b));
        assert!(!tracker.source_current_certain(a), "b speaking for itself is not a speaking");
        // A gap inside that doubt still lands on the source it names, and on
        // no other.
        tracker.handle_canonical(gap(5.0, Some(b))).unwrap();
        assert!(!tracker.source_current_certain(b));

        // A transport reset is the whole session speaking for itself.
        tracker.session_notes_off(6.0);
        assert!(tracker.source_current_certain(a) && tracker.source_current_certain(b));
    }

    /// The gap list is bounded, and what it drops is the OLDEST — the same way
    /// the roll bounds its own past (`NoteRoll::MAX_NOTES`, which this shares).
    /// Worth pinning because nothing else can see it: the cap is reached one
    /// publication outage at a time, and the export warning and the pane badge
    /// that read `publication_gaps` both take the list as given.
    #[test]
    fn the_gap_list_keeps_the_newest_and_forgets_past_its_cap() {
        let mut tracker = NoteTracker::new();
        // Past the cap rather than up to it, so the eviction runs more than the
        // once that an off-by-one would also satisfy.
        let pushed = NoteRoll::MAX_NOTES + 8;
        for i in 0..pushed {
            tracker.handle_canonical(gap(i as Time, Some(SourceId(1)))).unwrap();
        }
        let times: Vec<Time> = tracker.publication_gaps().iter().map(|gap| gap.time).collect();
        assert_eq!(times.len(), NoteRoll::MAX_NOTES);
        assert_eq!(times.first(), Some(&8.0), "the first eight outages are the ones forgotten");
        assert_eq!(times.last(), Some(&((pushed - 1) as Time)), "and the newest is still there");
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
