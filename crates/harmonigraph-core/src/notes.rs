//! Tracking of active and recently-released MIDI voices.
//!
//! The plugin's audio thread converts host MIDI into [`NoteEvent`]s and
//! ships them to the GUI over a lock-free ring buffer; the standalone dev
//! harness generates them from a mock source. Either way, the GUI thread
//! owns a [`NoteTracker`] and feeds every event into it.

#[cfg(test)]
use crate::canonical::VoiceBaseline;
use crate::canonical::{CanonicalEvent, CanonicalOrder, InvalidCanonical, SourceBaseline};
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
    /// The tape's namespace (#1719): a recorded note played back from the
    /// tape keeps the source that played it, with this bit set, so a key held
    /// live and the same key sounding on the tape are two voices. No live
    /// source reaches it: the Hub numbers seventeen.
    const TAPE: u64 = 1 << 40;

    /// The source a note recorded from `self` sounds under on the tape.
    pub fn on_tape(self) -> SourceId {
        SourceId(self.0 | Self::TAPE)
    }

    /// The note is the tape's, not one played live.
    pub fn is_tape(self) -> bool {
        self.0 & Self::TAPE != 0
    }

    /// The source that played a note the tape plays: what its Show switch
    /// is read from.
    pub fn off_tape(self) -> SourceId {
        SourceId(self.0 & !Self::TAPE)
    }
}

/// One held address. Same-source retriggers replace this address; host note
/// IDs remain a shell pass-through concern, not a second held-voice key.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VoiceKey {
    pub source: SourceId,
    pub channel: u8,
    pub note: u8,
}

/// One press: a serial its [`NoteRoll`] assigns as the press
/// opens, at a note-on or a baseline's fresh entry. What a press is known by
/// across frames, as a [`Voice`] and as a [`RollNote`](crate::RollNote) alike.
///
/// Two presses of one roll never share one, whatever their keys and onsets: a
/// fade a baseline withdrew and the fresh press it opened at the same onset are
/// two (#1883). Only within a roll, and only for its life: a roll rebuilt from
/// the same notes numbers them again, and two rolls number theirs alike.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PressId(pub(crate) u64);

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

    /// Each expression `t` of the way from `self` to `to`.
    pub fn lerp(self, to: Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Self {
            pressure: mix(self.pressure, to.pressure),
            gain: mix(self.gain, to.gain),
            timbre: mix(self.timbre, to.timbre),
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

    /// Whether the payload is one replay can carry: a finite time, a MIDI
    /// channel and note, a velocity in 0..=1, a finite tuning, an expression
    /// inside its [`range`](Expression::range).
    ///
    /// The one statement of it, shared by [`NoteDelta::validate`] and a take's
    /// plain note record, so the two cannot drift (#1128 added the expression
    /// range to each by hand).
    ///
    /// [`NoteDelta::validate`]: crate::canonical::NoteDelta::validate
    pub fn has_valid_payload(&self) -> bool {
        self.time.is_finite()
            && self.channel < 16
            && self.note < 128
            && match self.kind {
                NoteEventKind::On { velocity } => {
                    velocity.is_finite() && (0.0..=1.0).contains(&velocity)
                }
                NoteEventKind::Tuning { semitones } => semitones.is_finite(),
                NoteEventKind::Expression { expression, value } => {
                    expression.range().contains(&value)
                }
                NoteEventKind::Off | NoteEventKind::SourceReset | NoteEventKind::SessionReset => {
                    true
                }
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

    /// Where the release of a press that sounded from `on` and let go at
    /// `off` starts: at its off, or once its attack has landed if that is
    /// later. The one statement that a release waits out the attack, which
    /// every voice's does ([`Voice::release_level`]) and every lattice note's.
    pub fn release_start(&self, on: Time, off: Time) -> Time {
        off.max(on + f64::from(self.attack_time))
    }
}

/// One sounding (or recently sounding) note: a held or fading press of the
/// [`NoteRoll`], read as what the lattice, the Spiral and the note band draw.
/// A copy made on each read, so it is never out of step with the press.
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
    /// The correction its source's tuner froze at the attack, in semitones,
    /// so `pitch - correction` is the pitch the player sent; zero for a note
    /// no tuner corrected.
    pub correction: f32,
    /// Where the note's expressions stand now; [`Expressions::NEUTRAL`] until
    /// the host sends one.
    pub expressions: Expressions,
    pub on_time: Time,
    /// Canonical accepted lifetime, absent for ordinary direct observations.
    pub lifetime: Option<u64>,
    pub state: VoiceState,
    /// The press this voice is ([`PressId`]).
    pub(crate) id: PressId,
}

impl Voice {
    pub fn key(&self) -> VoiceKey {
        VoiceKey { source: self.source, channel: self.channel, note: self.note }
    }

    /// The press this voice is ([`PressId`]).
    pub fn id(&self) -> PressId {
        self.id
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
            VoiceState::Released { at } => env.release(now, env.release_start(self.on_time, at)),
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
    /// How lit a voice is for the panes that read voices directly: the
    /// Spectral pane's note band and the Spiral's sounding pitches. One
    /// function so a note cannot arrive at one rate and leave at another, nor
    /// have one pane disagree with the other about either.
    ///
    /// The lattice does not read it: its nodes carry their own levels in the
    /// scene layer's motion state.
    pub fn activation(&self, now: Time, env: &Envelope) -> f32 {
        env.attack(now, self.on_time) * self.release_level(now, env)
    }

    /// Whether [`activation`](Self::activation) still changes after `now`
    /// with no edit reaching the voice: its arrival has not landed, which
    /// includes an onset still ahead, or its key is up and its fade runs
    /// until [`prune`](NoteTracker::prune) ends it. A held voice whose arrival
    /// has landed draws the same until an edit reaches it.
    pub fn envelope_moving(&self, now: Time, env: &Envelope) -> bool {
        matches!(self.state, VoiceState::Released { .. }) || env.attack(now, self.on_time) < 1.0
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

/// Takes every note event into one [`NoteRoll`], which holds each press once
/// (#1759) and is read here as held voices plus a tail of recently released
/// ones (so releases can fade out instead of vanishing), and behind those a
/// [`NoteHistory`] of every pitch that has finished fading.
///
/// Held voices iterate in `(source, channel, note)` order. Batch releases
/// preserve that order in the released tail and history as well.
#[derive(Default)]
pub struct NoteTracker {
    roll: NoteRoll,
    history: NoteHistory,
    canonical: CanonicalOrder,
    edits: Edits,
}

/// What a [`NoteTracker`] has taken in that changes what it draws: how many
/// events and baselines of sources it shows, or that change a source's Show
/// switch, and the latest time any of them was dated. A shell that draws only when asked
/// reads it to ask: an edit changes what the voices draw, and one dated
/// ahead of the shell's clock changes it again when the clock gets there,
/// as a Hub's notes are, a Tuning delay ahead of the block that publishes
/// them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edits {
    pub count: u64,
    pub latest: Time,
}

impl Default for Edits {
    fn default() -> Self {
        Edits { count: 0, latest: Time::NEG_INFINITY }
    }
}

impl Edits {
    fn take(&mut self, time: Time) {
        self.count += 1;
        self.latest = self.latest.max(time);
    }
}

impl NoteTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume one canonical item. This non-RT consumer cannot acknowledge
    /// source journals.
    pub fn handle_canonical(&mut self, event: CanonicalEvent<'_>) -> Result<(), InvalidCanonical> {
        self.handle_canonical_mapped(event, 0.0)
    }

    /// Map drawing times by `offset`; a voice keeps the lifetime it was struck
    /// with.
    pub fn handle_canonical_mapped(
        &mut self,
        event: CanonicalEvent<'_>,
        offset: Time,
    ) -> Result<(), InvalidCanonical> {
        if !offset.is_finite() {
            return Err(InvalidCanonical);
        }
        match event {
            CanonicalEvent::Note(mut delta) => {
                delta.validate()?;
                delta.event.time += offset;
                delta.validate()?;
                let ordered = self.canonical.note(delta.event.source, delta.sequence);
                debug_assert!(ordered, "a delta out of its source's order: {delta:?}");
                let key = delta.event.key();
                if !matches!(
                    delta.event.kind,
                    NoteEventKind::On { .. }
                        | NoteEventKind::SourceReset
                        | NoteEventKind::SessionReset
                ) && delta.lifetime != 0
                    && self
                        .roll
                        .held_on(key)
                        .is_some_and(|note| note.lifetime != Some(delta.lifetime))
                {
                    return Ok(());
                }
                self.handle_event(delta.display_event());
                if matches!(delta.event.kind, NoteEventKind::On { .. }) {
                    if let Some(pitch) = delta.pitch_microcents {
                        let pitch = (pitch as f64 / 100_000_000.0) as f32;
                        self.roll.bend(key, delta.event.time, pitch);
                    }
                    self.roll.set_lifetime(key, (delta.lifetime != 0).then_some(delta.lifetime));
                    let correction = delta.assignment.map_or(0, |a| a.correction_microcents);
                    self.roll.set_correction(key, semitones(correction));
                } else if let Some(assignment) = delta.assignment {
                    // A later delta states the voice's correction as it
                    // stands: a withdrawn one (#1799) is zero from here.
                    self.roll.set_correction(key, semitones(assignment.correction_microcents));
                }
            }
            CanonicalEvent::Baseline(frame) => return self.replace_source_mapped(frame, offset),
            // A gap changes nothing drawn: the next baseline alone corrects
            // the held set, and a take reports the gap in its export warning
            // (#1798 F1).
            CanonicalEvent::Gap(gap) => gap.validate()?,
        }
        Ok(())
    }

    /// Validate the entire held set before mutation. Matching lifetimes keep
    /// their onset and bend history. Missing observations
    /// are closed as history gaps, never fictional downstream releases.
    pub fn replace_source(&mut self, frame: &SourceBaseline) -> Result<(), InvalidCanonical> {
        self.replace_source_mapped(frame, 0.0)
    }

    fn replace_source_mapped(
        &mut self,
        frame: &SourceBaseline,
        offset: Time,
    ) -> Result<(), InvalidCanonical> {
        frame.validate()?;
        let mut mapped = *frame;
        mapped.translate(offset);
        mapped.validate()?;
        let ordered = self.canonical.baseline(frame.source, frame.output_cut);
        debug_assert!(ordered, "a snapshot cut below a delta it should cover: {frame:?}");
        // Drawn either way only when it shows or its Show switch changes.
        if frame.shown || !self.hides(frame.source) {
            self.edits.take(mapped.time);
        }
        self.roll.set_shown(frame.source, frame.shown);
        self.roll.replace_source(frame.source, frame.voices(), mapped.time, offset);
        Ok(())
    }

    pub fn handle_event(&mut self, event: NoteEvent) {
        // A hidden source's playing is drawn nowhere. A session reset ends
        // every source's notes, shown ones included.
        if matches!(event.kind, NoteEventKind::SessionReset) || !self.hides(event.source) {
            self.edits.take(event.time);
        }
        let key = event.key();
        match event.kind {
            // Control event: applies regardless of the event's channel.
            NoteEventKind::SessionReset => self.session_notes_off(event.time),
            NoteEventKind::SourceReset => self.source_notes_off(event.source, event.time),
            NoteEventKind::On { velocity } => {
                self.roll.note_on(key, velocity, f32::from(event.note), event.time);
            }
            NoteEventKind::Off => self.roll.note_off(key, event.time),
            // Octave indicators track the sounding pitch too.
            NoteEventKind::Tuning { semitones } => {
                self.roll.bend(key, event.time, f32::from(event.note) + semitones);
            }
            NoteEventKind::Expression { expression, value } => {
                self.roll.set_expression(key, event.time, expression, value);
            }
        }
    }

    /// End each fade that is over, folding the voice into the history as it
    /// goes. Call once per frame before iterating.
    ///
    /// A voice becomes a memory in the step its Note fade ends, so the tracker
    /// never holds one note as a voice and a memory at once, and a trail
    /// picks the note up where that fade lets go. The lattice's own release
    /// can outlast it, with the Fade-out delay and the last slice's delay
    /// ahead of its Note fade, and the scene's departing-name reserve covers
    /// the part before the trail lands. Drawn and remembered are one
    /// question, asked once at the release ([`NoteRoll`]'s rule 4): a voice
    /// whose source was hidden when it let go is never drawn AND never
    /// recorded, and one visible then finishes its fade on screen and lands in
    /// the history even if its source is hidden mid-fade. A retrigger without
    /// an off, a held note a baseline omits, and a fade a baseline withdraws
    /// never reach here: none of them is a departure the trail should mark.
    ///
    /// Asks the RELEASE rather than the full activation, because the question
    /// here is whether the fade is over and not whether anything is currently
    /// on screen. The two part company for the whole of a note's arrival,
    /// which the release waits out ([`Voice::release_level`]): a note switched
    /// off in the same instant it arrived reads an activation of 0 while its
    /// release has not started at all — read that way, a zero-length note
    /// would be dropped before it ever drew.
    pub fn prune(&mut self, now: Time, env: &Envelope) {
        let history = &mut self.history;
        self.roll.prune(now, env, |voice| history.record(voice, now));
    }

    /// What it has taken in so far ([`Edits`]).
    pub fn edits(&self) -> Edits {
        self.edits
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

    /// Forget the played-note timeline, leaving the voices alone. Independent
    /// of [`clear_history`](Self::clear_history): the two answer different
    /// questions and are cleared from different places in the UI.
    pub fn clear_roll(&mut self) {
        self.roll.clear();
    }

    /// All voices that should currently be visualized: held first, in
    /// `(source, channel, note)` order, then the released ones in the order
    /// their releases arrived.
    ///
    /// The two halves ask visibility of different state, and that is the
    /// whole rule. A HELD voice is filtered on the CURRENT Show switch,
    /// so hiding a source takes everything it is holding off the screen in
    /// that instant. A RELEASED one was decided once when it left the held set
    /// ([`NoteRoll`]'s rule 4), the same answer [`prune`](Self::prune) records
    /// on, which is what makes "drawn" and "remembered" one answer instead of
    /// two that can disagree (#905). The visible cost is that hiding a source
    /// does not cut its already-fading notes: they finish the fade they were
    /// drawing.
    pub fn voices(&self) -> impl Iterator<Item = Voice> + '_ {
        self.roll.voices()
    }

    /// The source's notes are hidden: its Show switch is off, or for a note
    /// the tape plays, the switch of the source that played it.
    pub fn hides(&self, source: SourceId) -> bool {
        self.roll.hides(source)
    }

    /// Every held voice, hidden sources' included: what is down now, for a
    /// recorder that must not depend on what is shown.
    pub fn held_voices(&self) -> impl Iterator<Item = Voice> + '_ {
        self.roll.held()
    }

    /// Every held voice the picture draws: [`held_voices`](Self::held_voices)
    /// less the hidden sources'.
    pub fn held_shown(&self) -> impl Iterator<Item = Voice> + '_ {
        self.roll.held().filter(|v| !self.hides(v.source))
    }

    pub fn held_count(&self) -> usize {
        self.held_shown().count()
    }

    /// Every published source's notes off. The tape's stay: its player
    /// alone ends them, and ends them itself when the plugin resets.
    pub fn session_notes_off(&mut self, now: Time) {
        self.roll.published_off(now);
    }

    /// A source leaving/resetting cannot release another source's held set.
    /// Keep the same release fade as a session reset.
    pub fn source_notes_off(&mut self, source: SourceId, now: Time) {
        self.roll.source_off(source, now);
    }
}

/// Microcents as semitones, the unit the roll keeps pitch in.
pub(crate) fn semitones(microcents: i64) -> f32 {
    (microcents as f64 / 100_000_000.0) as f32
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
        let mut ends_a: Vec<_> =
            tracker.roll().notes().filter(|n| n.source == a).map(|n| (n.start, n.end)).collect();
        ends_a.sort_by(|x, y| x.partial_cmp(y).unwrap());
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

    /// Middle C held from 0 at full velocity, untuned.
    fn held_voice() -> Voice {
        Voice {
            source: SourceId::DIRECT,
            channel: 0,
            note: 60,
            velocity: 1.0,
            pitch: 60.0,
            pitch_class: PitchClass::from_midi_note(0),
            correction: 0.0,
            expressions: Expressions::NEUTRAL,
            on_time: 0.0,
            lifetime: None,
            state: VoiceState::Held,
            id: PressId(0),
        }
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
    /// held voice into a releasing one: the fading presses are a Vec, so
    /// whatever order it is filled in is the order it keeps for the whole fade.
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
        // A press the baseline names by its lifetime.
        let row = VoiceBaseline { actual_onset: 0.0, ..published_voice(60, 61) };

        // Hidden at the release. The source's Show goes off while the
        // note is still down — its baseline still lists the note, so the
        // voice stays HELD and merely drops off the screen.
        let mut tracker = NoteTracker::new();
        let struck = SourceBaseline::new(SourceId::DIRECT, 0.0, 0, true, &[row]).unwrap();
        tracker.replace_source(&struck).unwrap();
        let hide = SourceBaseline::new(SourceId::DIRECT, 1.0, 0, false, &[row]).unwrap();
        tracker.replace_source(&hide).unwrap();
        assert_eq!(tracker.voices().count(), 0, "a held voice cuts the instant its source hides");
        tracker.handle_event(off(2.0, 60));
        assert_eq!(tracker.held_voices().count(), 0, "the off released the hidden voice");
        // The source rejoins with two thirds of the fade still to run. The
        // fixture has to actually reach a release, or every assertion below
        // passes on a voice that was dropped at the hide.
        let rejoin = SourceBaseline::new(SourceId::DIRECT, 2.3, 0, true, &[]).unwrap();
        tracker.replace_source(&rejoin).unwrap();
        assert_eq!(tracker.roll().notes().count(), 1, "released onto the timeline, not dropped");
        assert_eq!(tracker.voices().count(), 0, "a fade begun hidden is not handed back");
        tracker.prune(3.1, &env);
        assert!(tracker.history().is_empty(), "and what was never drawn is never remembered");

        // Visible at the release, hidden mid-fade: the opposite direction.
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(off(2.0, 60));
        let hide = SourceBaseline::new(SourceId::DIRECT, 2.3, 0, false, &[]).unwrap();
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
        let held = tracker.voices().next().unwrap();
        // Held voices read full intensity regardless of the fade time — with
        // no attack, which is what `fade` builds; the attack has its own
        // tests below.
        assert_eq!(held.activation(100.0, &fade(2.0)), 1.0);
        assert_eq!(held.activation(100.0, &fade(0.0)), 1.0);

        tracker.handle_event(off(10.0, 60));
        let released = tracker.voices().next().unwrap();
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
        let mut voice = held_voice();
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
        let voice = tracker.voices().next().unwrap();
        assert_eq!(voice.activation(0.0, &env), 0.0, "nothing on screen at the instant itself");
        assert_eq!(voice.release_level(0.0, &env), 1.0, "but the release has not started");

        tracker.prune(0.0, &env);
        assert_eq!(tracker.voices().count(), 1, "so the note is still there to draw");
        let voice = tracker.voices().next().unwrap();
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
        let mut voice = held_voice();
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

    /// One voice as a canonical source PUBLISHES it: accepted output, with the
    /// lifetime a baseline resumes it by.
    ///
    /// `AcceptedOutput` is the load-bearing field and the reason this helper
    /// exists: `VoiceBaseline::default()` is `ObservedDirect`, which
    /// `SourceBaseline::validate` rejects on every source but
    /// [`SourceId::DIRECT`] — so the obvious fixture is refused, and refused
    /// for a reason that has nothing to do with the note. Every restated voice
    /// also needs a nonzero lifetime.
    fn published_voice(note: u8, lifetime: u64) -> VoiceBaseline {
        VoiceBaseline {
            note,
            lifetime,
            actual_onset: 1.0,
            pitch_microcents: i64::from(note) * 100_000_000,
            velocity: 0.8,
            ..Default::default()
        }
    }

    /// Publication lost over one source, or over the whole canonical stream.
    fn gap(time: Time, source: Option<SourceId>) -> CanonicalEvent<'static> {
        CanonicalEvent::Gap(crate::canonical::PublicationGap {
            source,
            time,
            through: time,
            first: 1,
            last: 1,
            reason: crate::canonical::GapReason::PublicationFull,
        })
    }

    /// A publication gap changes nothing drawn: what was held stays held, and
    /// the source's next baseline alone corrects the held set, ending a note it
    /// omits and keeping the one it lists (#1798 F1).
    #[test]
    fn a_publication_gap_leaves_the_held_set_to_the_next_baseline() {
        let source = SourceId(1);
        let (kept, lost) = (published_voice(60, 61), published_voice(64, 62));
        let baseline = |time, rows: &[VoiceBaseline]| {
            SourceBaseline::new(source, time, 0, true, rows).unwrap()
        };
        let mut tracker = NoteTracker::new();
        tracker.replace_source(&baseline(1.0, &[kept, lost])).unwrap();
        for gap in [gap(2.0, Some(source)), gap(2.5, None)] {
            assert_eq!(tracker.handle_canonical(gap), Ok(()));
        }
        assert_eq!(tracker.held_count(), 2, "a gap releases nothing");
        tracker.replace_source(&baseline(3.0, &[kept])).unwrap();
        let held: Vec<_> = tracker.held_voices().map(|v| (v.note, v.on_time)).collect();
        assert_eq!(held, [(60, kept.actual_onset)], "the baseline ends the note it omits");
        assert_eq!(tracker.roll().notes().count(), 2, "and keeps the one it lists as one note");
    }

    /// A baseline states each voice's frozen correction, for a note it opens
    /// and for one it restates: Learn takes it back off (#1938).
    #[test]
    fn a_baseline_states_each_voices_correction() {
        let source = SourceId(1);
        let mut row = published_voice(60, 61);
        row.frozen_offset_microcents = 10_000_000;
        let mut tracker = NoteTracker::new();
        let baseline = |time, row| SourceBaseline::new(source, time, 0, true, &[row]);
        tracker.replace_source(&baseline(2.0, row).unwrap()).unwrap();
        let correction = |tracker: &NoteTracker| tracker.held_voices().next().unwrap().correction;
        assert_eq!(correction(&tracker), 0.1, "a note the baseline opens");
        row.frozen_offset_microcents = 0;
        tracker.replace_source(&baseline(3.0, row).unwrap()).unwrap();
        assert_eq!(correction(&tracker), 0.0, "a note it restates");
    }

    /// A delta that states a voice's assignment states its correction as it
    /// stands: the onset's, and the zero a withdrawn correction leaves (#1799).
    #[test]
    fn a_tuning_delta_restates_a_voices_correction() {
        use crate::canonical::{AssignmentMetadata, NoteDelta};
        let source = SourceId(1);
        let assignment = |correction_microcents| {
            Some(AssignmentMetadata {
                decision: 1,
                node: None,
                correction_microcents,
                player_tuning: 0.0,
            })
        };
        let mut tracker = NoteTracker::new();
        let on = NoteDelta {
            event: NoteEvent::on(1.0, source, 0, 60, 0.8),
            sequence: 1,
            lifetime: 7,
            pitch_microcents: Some(6_010_000_000),
            assignment: assignment(10_000_000),
        };
        tracker.handle_canonical(CanonicalEvent::Note(on)).unwrap();
        let correction = |tracker: &NoteTracker| tracker.held_voices().next().unwrap().correction;
        assert_eq!(correction(&tracker), 0.1);
        let withdrawn = NoteDelta {
            event: NoteEvent {
                source,
                time: 1.1,
                channel: 0,
                note: 60,
                kind: NoteEventKind::Tuning { semitones: 0.0 },
            },
            sequence: 2,
            pitch_microcents: Some(6_000_000_000),
            assignment: assignment(0),
            ..on
        };
        tracker.handle_canonical(CanonicalEvent::Note(withdrawn)).unwrap();
        assert_eq!(correction(&tracker), 0.0, "the correction was withdrawn");
    }

    /// A baseline states a voice's expressions along with its pitch: a note it
    /// opens starts at them, and a note it restates takes the value it states
    /// as a step, rather than snapping to neutral.
    #[test]
    fn a_baseline_carries_its_voices_expressions() {
        let source = SourceId(1);
        let mut row = published_voice(60, 61);
        row.expressions.pressure = 0.5;
        let mut tracker = NoteTracker::new();
        let baseline = |time, row| SourceBaseline::new(source, time, 0, true, &[row]);
        assert_eq!(tracker.replace_source(&baseline(2.0, row).unwrap()), Ok(()));
        row.expressions.pressure = 0.8;
        assert_eq!(tracker.replace_source(&baseline(3.2, row).unwrap()), Ok(()));

        assert_eq!(tracker.held_voices().next().unwrap().expressions.pressure, 0.8);
        let note = tracker.roll().notes().next().unwrap();
        let points: Vec<_> = note.expressions().iter().map(|(t, e)| (*t, e.pressure)).collect();
        assert_eq!(points, vec![(2.0, 0.5), (3.2, 0.5), (3.2, 0.8)]);
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

    /// A note the tape plays is shown or hidden with the source that played
    /// it, on the lattice and on the roll.
    #[test]
    fn the_tape_plays_a_source_under_its_own_show_switch() {
        let mut tracker = NoteTracker::new();
        let source = SourceId(3);
        tracker.handle_event(NoteEvent::on(1.0, source.on_tape(), 0, 60, 1.0));
        assert_eq!((tracker.voices().count(), tracker.roll().notes().count()), (1, 1));
        tracker.roll.set_shown(source, false);
        assert_eq!((tracker.voices().count(), tracker.roll().notes().count()), (0, 0));
        assert!(!tracker.hides(SourceId(4).on_tape()), "another source's tape notes still show");
    }

    /// What a hidden source plays is drawn nowhere, so it is no edit: a bend
    /// on a hidden track over a still picture wakes nothing. A baseline that
    /// changes the Show switch is one, once, either way.
    #[test]
    fn a_hidden_sources_events_are_no_edit_and_a_show_change_is_one() {
        use crate::canonical::SourceBaseline;
        let source = SourceId(2);
        let mut tracker = NoteTracker::new();
        let show = |tracker: &mut NoteTracker, at: Time, cut: u64, shown: bool| {
            let baseline = SourceBaseline::new(source, at, cut, shown, &[]).unwrap();
            tracker.replace_source(&baseline).unwrap();
        };
        show(&mut tracker, 0.0, 0, false);
        let hidden = tracker.edits().count;
        assert_eq!(hidden, 1, "hiding a shown source is an edit");
        tracker.handle_event(NoteEvent::on(1.0, source, 0, 60, 1.0));
        let bend = NoteEventKind::Tuning { semitones: 0.3 };
        tracker.handle_event(NoteEvent { kind: bend, ..NoteEvent::off(1.1, source, 0, 60) });
        show(&mut tracker, 1.2, 0, false);
        assert_eq!(tracker.edits().count, hidden, "a hidden source's playing was an edit");
        show(&mut tracker, 1.3, 0, true);
        assert_eq!(tracker.edits().count, hidden + 1, "showing it again was not one edit");
        tracker.handle_event(NoteEvent { kind: bend, ..NoteEvent::off(1.4, source, 0, 60) });
        assert_eq!(tracker.edits().count, hidden + 2, "a shown source's bend was no edit");
    }

    /// A session reset ends what was published, not what the tape plays: its
    /// player still counts those voices as sounding.
    #[test]
    fn a_session_reset_leaves_the_tapes_voices_to_its_player() {
        let live = SourceId(1);
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent::on(0.0, live, 0, 60, 0.8));
        tracker.handle_event(NoteEvent::on(0.0, live.on_tape(), 0, 64, 0.8));
        let held =
            |tracker: &NoteTracker| tracker.held_voices().map(|v| v.note).collect::<Vec<_>>();
        let sounding = |tracker: &NoteTracker| {
            let live = |n: &&crate::RollNote| n.end.is_none() && n.observed_until.is_none();
            tracker.roll().notes().filter(live).map(|n| n.note).collect::<Vec<_>>()
        };

        tracker.session_notes_off(3.0);
        assert_eq!((held(&tracker), sounding(&tracker)), (vec![64], vec![64]));
    }
}
