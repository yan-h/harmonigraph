//! Every note press, sounding, fading or done: the one record the tracker draws
//! as voices and the Roll draws as a timeline (#1759).
//!
//! The same MIDI stream has two shapes downstream, and only one of them is
//! kept here:
//!
//! - [`NoteRoll`] (here) holds *each press* — one entry per press, with the
//!   pitch it traced and the expressions it held — in one of three buckets by
//!   where it is in its life. [`NoteTracker`](crate::NoteTracker) reads its
//!   held and fading presses as [`Voice`]s.
//! - [`NoteHistory`](crate::NoteHistory) holds *which pitches* have been
//!   visited — no time, one entry per pitch — and takes a press when its fade
//!   ends.
//!
//! A press is written the moment it starts, not when it ends — a piano roll
//! has to draw the note that is sounding right now — and moves between the
//! buckets as it is released or finished.
//!
//! The voice and the timeline still answer differently for five cases, each
//! written once here, where its bucket move is (the third, a publication gap,
//! retired with #1798 F1: it changes nothing here, and the next baseline
//! corrects the held set by rules 5 and 6):
//!
//! 1. an off at or before its onset pulses as a voice and is no timeline note
//!    ([`note_off`](NoteRoll::note_off));
//! 2. a retrigger without an off ends the old press with no fade, and drops it
//!    when the retrigger is at or before its onset
//!    ([`note_on`](NoteRoll::note_on));
//! 4. a note hidden at its release is never drawn leaving nor remembered, while
//!    the timeline follows the current Show switch;
//! 5. a held note a baseline omits leaves with no fade and no trail mark;
//! 6. a baseline listing a note that has ended withdraws its fade without a
//!    trail mark and opens a fresh note.
//!
//! `tests/note_rules.rs` drives random sequences through the tracker and checks
//! each rule after every step.
//!
//! Bounded on both ends (a count of finished notes and an age) once the
//! tracker prunes it, so an idle plugin left open for a day does not grow
//! without limit.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::canonical::VoiceBaseline;
use crate::notes::{
    Envelope, Expression, Expressions, SourceId, Time, Voice, VoiceKey, VoiceState,
};
use crate::tuning::PitchClass;

/// One press of one key: when it started, when it stopped, and the pitch
/// it traced in between.
#[derive(Clone, Debug)]
pub struct RollNote {
    pub source: SourceId,
    pub channel: u8,
    pub note: u8,
    pub velocity: f32,
    pub start: Time,
    /// When it was released, or `None` while it is still sounding.
    pub end: Option<Time>,
    /// Observation stopped without proof of termination. This is not a release.
    pub observed_until: Option<Time>,
    pub lifetime: Option<u64>,
    /// False when a baseline supplied current state without the earlier bends.
    pub history_complete: bool,
    /// Sounding pitch (MIDI note units, per-note tuning included) over
    /// time, oldest first. Never empty: note-on seeds it with the key's
    /// own pitch. Between breakpoints the pitch is read as a straight
    /// line — MPE bends arrive densely enough that interpolating looks
    /// like the glide it is, where holding each value would draw stairs.
    bends: Vec<(Time, f32)>,
    breaks: Vec<Time>,
    /// The pitch reached by the end of the onset — see
    /// [`settled_pitch`](Self::settled_pitch).
    ///
    /// Kept as a field rather than read back out of `bends`, so that it is
    /// genuinely immutable once the note is [`SETTLE`](Self::SETTLE) old. Read
    /// back, it is not: the [`MAX_BENDS`](Self::MAX_BENDS) fold overwrites the
    /// LAST breakpoint in place, so a note bent hard enough to fill the vector
    /// inside the onset window would have its final in-window breakpoint
    /// replaced by an out-of-window one, and the answer would move — long
    /// after anything derived from it had been told it could not.
    settled: f32,
    /// Every expression the note has held, oldest first, read as a straight
    /// line between breakpoints like `bends`. Never empty: the onset seeds it.
    ///
    /// Thinned by TIME rather than folded by count. Pressure arrives every
    /// block, so the fold `bends` uses would spend its whole budget in the
    /// first moments of a note and draw the rest as one straight line.
    expressions: Vec<(Time, Expressions)>,
    /// The closest two breakpoints in `expressions` may sit, which doubles
    /// every time the note fills its budget.
    expression_spacing: Time,
    /// The pitch class of the last breakpoint of `bends`, kept beside it
    /// because a voice is read per node and per frame.
    pitch_class: PitchClass,
    /// A voice and no timeline note: released at or before its onset (rule
    /// 1), or held or fading when Clear roll took the timeline. Leaving the
    /// voices drops it rather than filing it in `past`.
    off_timeline: bool,
}

impl RollNote {
    /// A press starting at `start`, its pitch and expressions first stated at
    /// `at`.
    fn new(
        key: VoiceKey,
        velocity: f32,
        start: Time,
        at: Time,
        pitch: f32,
        expressions: Expressions,
    ) -> Self {
        RollNote {
            source: key.source,
            channel: key.channel,
            note: key.note,
            velocity,
            start,
            end: None,
            observed_until: None,
            lifetime: None,
            history_complete: true,
            bends: vec![(at, pitch)],
            breaks: Vec::new(),
            settled: pitch,
            expressions: vec![(at, expressions)],
            expression_spacing: Self::EXPRESSION_SPACING,
            pitch_class: PitchClass::from_cents(pitch * 100.0),
            off_timeline: false,
        }
    }

    pub fn key(&self) -> VoiceKey {
        VoiceKey { source: self.source, channel: self.channel, note: self.note }
    }

    /// When it left the held set — its release, a reset, or a baseline that
    /// omitted it — which is where its fade starts, or `None` while it is
    /// held. A note has an end or an observation's end, never both, and
    /// `past` is ordered by this.
    fn departed(&self) -> Option<Time> {
        self.observed_until.or(self.end)
    }

    /// The press as the voice the tracker draws: its current pitch and
    /// expressions, its onset, and held until it departs.
    fn voice(&self) -> Voice {
        Voice {
            source: self.source,
            channel: self.channel,
            note: self.note,
            velocity: self.velocity,
            pitch: self.end_pitch(),
            pitch_class: self.pitch_class,
            expressions: self.expressions[self.expressions.len() - 1].1,
            on_time: self.start,
            lifetime: self.lifetime,
            state: self.departed().map_or(VoiceState::Held, |at| VoiceState::Released { at }),
        }
    }

    /// Its fade is still running: what [`NoteRoll::prune`] asks of a voice.
    fn fading_at(&self, now: Time, env: &Envelope) -> bool {
        self.voice().release_level(now, env) > 0.0
    }

    /// Breakpoints kept per note. Past this the newest overwrites the
    /// previous one, so a long continuous bend keeps an accurate *current*
    /// pitch and loses only middle detail. Generous: a note has to be bent
    /// for a while to reach it.
    pub const MAX_BENDS: usize = 64;

    /// How long after the onset a pitch change still counts as part of the
    /// onset rather than as a bend — see [`settled_pitch`](Self::settled_pitch).
    ///
    /// Long enough to cover a per-note tuning that arrives a block or two
    /// behind its note-on (a 512-frame block at 48 kHz is 11 ms), far short of
    /// any glide played on purpose.
    pub const SETTLE: Time = 0.05;

    /// The pitch this note started on.
    pub fn start_pitch(&self) -> f32 {
        self.bends[0].1
    }

    /// The pitch this note SETTLED on: the last breakpoint within
    /// [`SETTLE`](Self::SETTLE) of the onset.
    ///
    /// The pitch that identifies a note, as against
    /// [`start_pitch`](Self::start_pitch), which is where the KEY sits. Under
    /// per-note tuning the two are different notes: a retuned note arrives as
    /// a note-on at its equal-tempered pitch followed by a tuning expression,
    /// so its first breakpoint is 12-EDO however the tuning is set, and every
    /// note in a just-intoned piece would answer `start_pitch` on the same
    /// twelve pitches. This is the plugin's whole subject matter, so a caller
    /// asking "which pitch is this" wants this one.
    ///
    /// Immutable once the note is [`SETTLE`](Self::SETTLE) old, which is what
    /// makes it safe to derive something from and keep — unlike
    /// [`end_pitch`](Self::end_pitch), which moves under a held note for as
    /// long as it is being bent.
    ///
    /// Like [`NoteHistory`](crate::NoteHistory), it prefers a note's SOUNDING
    /// pitch to its key; unlike history, which records where a note ended, it
    /// records where the note began once its tuning had landed. For a glided
    /// note the two name different pitches, and each is right about a
    /// different moment.
    pub fn settled_pitch(&self) -> f32 {
        self.settled
    }

    /// Expression breakpoints kept per note before it is thinned to half.
    pub const MAX_EXPRESSIONS: usize = 128;

    /// The closest two expression breakpoints sit on a fresh note: finer than
    /// the roll draws at any zoom it offers, coarse enough that a note has to
    /// be pressed for over a second before it thins.
    pub const EXPRESSION_SPACING: Time = 0.01;

    /// Every expression the note has held, oldest first, as breakpoints to be
    /// read as straight lines. Two sharing a time are a step. Never empty.
    pub fn expressions(&self) -> &[(Time, Expressions)] {
        &self.expressions
    }

    /// Where the note's expressions stood at `at`, read off
    /// [`expressions`](Self::expressions) as straight lines. A step is taken
    /// at its own time, so `at` on a step reads the value after it; before the
    /// first breakpoint and after the last, the nearest one holds.
    pub fn expressions_at(&self, at: Time) -> Expressions {
        let after = self.expressions.partition_point(|(time, _)| *time <= at);
        let Some(&(t0, from)) = after.checked_sub(1).map(|i| &self.expressions[i]) else {
            return self.expressions[0].1;
        };
        match self.expressions.get(after) {
            Some(&(t1, to)) => from.lerp(to, ((at - t0) / (t1 - t0)) as f32),
            None => from,
        }
    }

    /// Record the note's expressions changing to `values` at `at`.
    ///
    /// A value arriving within one spacing of the breakpoint before last moves
    /// the last breakpoint rather than adding one, so the history keeps about
    /// one point per spacing and its end is always the current value. A value
    /// arriving after a quiet stretch re-asserts the old value at the same
    /// instant first: without it the line from the last change would draw a
    /// slow ramp across the whole of a held level.
    fn express(&mut self, at: Time, values: Expressions) {
        let last = self.expressions.len() - 1;
        let (last_at, last_values) = self.expressions[last];
        if last_values == values {
            return;
        }
        if at <= last_at
            || (last > 0 && at - self.expressions[last - 1].0 < self.expression_spacing)
        {
            self.expressions[last] = (at.max(last_at), values);
            return;
        }
        if self.expressions.len() + 2 > Self::MAX_EXPRESSIONS {
            self.thin_expressions();
        }
        if at - last_at >= self.expression_spacing {
            self.expressions.push((at, last_values));
        }
        self.expressions.push((at, values));
    }

    /// Halve the history: keep every other breakpoint, the first and the
    /// last, and double the spacing so it fills at the same rate again.
    ///
    /// Blind to steps: halving can keep one point of a same-time pair and drop
    /// its partner, which turns that step into a ramp one spacing long: 20 ms
    /// after the first halving, doubling with each one after.
    fn thin_expressions(&mut self) {
        let last = self.expressions.len() - 1;
        let mut index = 0;
        self.expressions.retain(|_| {
            let keep = index % 2 == 0 || index == last;
            index += 1;
            keep
        });
        self.expression_spacing *= 2.0;
    }

    /// The pitch it is sounding (or last sounded) at.
    pub fn end_pitch(&self) -> f32 {
        self.bends[self.bends.len() - 1].1
    }

    /// True while the note is still held.
    pub fn is_live(&self) -> bool {
        self.end.is_none() && self.observed_until.is_none()
    }

    /// When this note stops: its release, or `now` while it is still
    /// sounding (a held note's segment reaches the present moment).
    pub fn stop(&self, now: Time) -> Time {
        self.departed().unwrap_or(now).max(self.start)
    }

    /// The note as straight-line segments `((t0, pitch0), (t1, pitch1))`,
    /// in time order, ending at [`stop`](Self::stop). A note that was
    /// never bent yields exactly one flat segment.
    pub fn segments(&self, now: Time) -> impl Iterator<Item = ((Time, f32), (Time, f32))> + '_ {
        let stop = self.stop(now);
        let last = self.bends.len() - 1;
        (0..=last).filter_map(move |i| {
            let (t0, p0) = self.bends[i];
            let (t1, p1) = if i == last { (stop, p0) } else { self.bends[i + 1] };
            if i != last && self.breaks.contains(&t1) {
                return None;
            }
            // A breakpoint recorded after `stop` (only reachable if the
            // shell hands out non-monotonic times) collapses to nothing.
            let t1 = t1.clamp(t0, stop.max(t0));
            (t1 > t0 || i == last).then_some(((t0, p0), (t1, p1)))
        })
    }

    /// Append a bend, folding it into the last breakpoint once
    /// [`MAX_BENDS`](Self::MAX_BENDS) is reached.
    fn bend(&mut self, at: Time, pitch: f32) {
        let last = self.bends.len() - 1;
        if self.bends[last].1 == pitch {
            return;
        }
        if self.bends.len() >= Self::MAX_BENDS && !self.breaks.contains(&self.bends[last].0) {
            self.bends[last] = (at, pitch);
        } else {
            self.push_point(at, pitch, false);
        }
        // Still part of the onset: this is how a retuned note reaches the
        // pitch it is actually playing. Recorded as it happens, so the fold
        // above can never take it back.
        if at <= self.start + Self::SETTLE {
            self.settled = pitch;
        }
        self.pitch_class = PitchClass::from_cents(pitch * 100.0);
    }

    /// Preserve both ends of every missing interval when the buffer folds.
    /// If every point is a boundary, forget the oldest prefix conservatively.
    fn push_point(&mut self, at: Time, pitch: f32, break_before: bool) {
        if self.bends.len() >= Self::MAX_BENDS {
            let removable = (1..self.bends.len() - 1).rev().find(|&i| {
                !self.breaks.contains(&self.bends[i].0)
                    && !self.breaks.contains(&self.bends[i + 1].0)
            });
            self.bends.remove(removable.unwrap_or(0));
            self.breaks.retain(|at| self.bends.iter().any(|point| point.0 == *at));
        }
        self.bends.push((at, pitch));
        if break_before && !self.breaks.contains(&at) {
            self.breaks.push(at);
        }
        self.pitch_class = PitchClass::from_cents(pitch * 100.0);
    }
}

/// Every note press still sounding, fading or remembered, each in exactly one
/// bucket.
#[derive(Clone, Debug, Default)]
pub struct NoteRoll {
    /// Held presses, keyed and ORDERED the way voices are drawn: by
    /// `(source, channel, note)` (#135). Not for the Roll's sake — its readers
    /// sort for themselves — but because a reset walks this map into `fading`,
    /// whose order `voices()` keeps, and `past` is what `enforce_cap` pops from
    /// the front of: a hashed order here would decide which press is forgotten.
    /// Bounded by the number of keys, so never trimmed.
    live: BTreeMap<VoiceKey, RollNote>,
    /// Released presses still fading, in the order their releases arrived.
    /// Voices, and timeline notes unless marked off it (rule 1, Clear);
    /// [`prune`](Self::prune) records each in the trail as its fade ends and
    /// moves it to `past`. Only notes visible at their release are here (rule
    /// 4). A roll nothing prunes, such as the tape's, keeps what it releases
    /// here for good.
    fading: Vec<RollNote>,
    /// Finished notes, oldest release first ([`RollNote::departed`]), held so by
    /// [`retire`](Self::retire) whatever order the releases arrive in — sources
    /// are mapped onto the clock by their own offsets, so one source's release
    /// can arrive after a later one's. Timeline notes only: `retire` drops a
    /// note marked off the timeline.
    past: VecDeque<RollNote>,
    hidden_sources: BTreeSet<SourceId>,
    /// Keeps every note, past [`MAX_NOTES`](Self::MAX_NOTES): a roll of a
    /// whole recording, built once and drawn a window at a time, where the
    /// cap would forget the song's start.
    uncapped: bool,
}

impl NoteRoll {
    /// Finished notes kept in `past`. The bound is on per-frame drawing work,
    /// not memory: the pane tests every note against the visible window. It
    /// holds for a roll the tracker prunes, which files a note there as its
    /// fade ends; the timeline also holds what is sounding and still fading,
    /// so it can briefly hold more.
    pub const MAX_NOTES: usize = 4096;
    /// Seconds of history kept. Comfortably past the longest window the
    /// display offers, so the display bound is the one that shows.
    pub const MAX_AGE: Time = 900.0;

    /// Open an entry for a new press.
    ///
    /// Rule 2: a retrigger with no intervening note-off ends the previous
    /// press at the same instant, straight into `past` with no fade and so no
    /// trail mark — the retrigger's own release covers the pitch. If that
    /// instant is at or before the previous press's own onset, it is dropped
    /// instead, the note having never sounded.
    pub fn note_on(&mut self, key: VoiceKey, velocity: f32, pitch: f32, at: Time) {
        if let Some(mut previous) = self.live.remove(&key) {
            if at > previous.start {
                previous.end = Some(at);
                self.retire(previous);
            }
        }
        self.live.insert(key, RollNote::new(key, velocity, at, at, pitch, Expressions::NEUTRAL));
    }

    /// Release one key.
    ///
    /// Rule 1: a note whose off lands at or before its own on never sounded,
    /// and the timeline drops it rather than remembering a note of no
    /// duration. Hosts do emit these: one press can arrive as an on, an off
    /// and a second on all stamped at the same sample, and the pair in the
    /// middle leaves an entry that begins and ends together. As a voice it
    /// still PULSES — a full arrival and departure on the lattice, the Spiral
    /// and the note band, since the release waits out the attack
    /// ([`Voice::release_level`]) — and lands in the trail: it fades marked
    /// off the timeline.
    ///
    /// It has to leave the timeline rather than be drawn small, because what
    /// shows there is not the ribbon. A ribbon of no length is floored to a
    /// couple of pixels and reads as grain, while the NAME the roll writes on
    /// it is full size and anchored at the moment it ended — so a single press
    /// leaves a letter scrolling away from the letter held at the now-line, as
    /// though the key had been played twice.
    ///
    /// The test is `at <= start`, which is a fact about the note rather than a
    /// span short enough to look wrong: a key cannot be released before it was
    /// pressed, and the shortest note anyone can play still ends after it
    /// began. A staccato note is untouched however brief it is.
    ///
    /// The resets deliberately do NOT go through this. Their timestamp is a
    /// transport event's, not a statement about any one note, so a held note
    /// one closes did sound — only its length is unknown, and clamping that to
    /// the onset must not erase it.
    pub fn note_off(&mut self, key: VoiceKey, at: Time) {
        let Some(mut note) = self.live.remove(&key) else {
            return;
        };
        note.end = Some(at);
        note.off_timeline |= at <= note.start;
        self.release(note);
    }

    /// A press leaves the held set with a fade, its `end` or `observed_until`
    /// already set to where the fade starts.
    ///
    /// Rule 4: whether its source shows is asked once, here, and decides both
    /// whether the rest of the fade is drawn and whether the trail picks the
    /// pitch up — a press hidden when it let go is neither, and one visible
    /// then is both, whatever its source does mid-fade (#905). So a hidden one
    /// is retired at once: filed in `past`, where the timeline still follows
    /// the CURRENT Show switch, or dropped if it is marked off the timeline.
    fn release(&mut self, note: RollNote) {
        if self.hides(note.source) {
            self.retire(note);
        } else {
            self.fading.push(note);
        }
    }

    pub(crate) fn set_lifetime(&mut self, key: VoiceKey, lifetime: Option<u64>) {
        if let Some(note) = self.live.get_mut(&key) {
            note.lifetime = lifetime;
        }
    }

    /// The press held on `key`, on the timeline or off it.
    pub(crate) fn held_on(&self, key: VoiceKey) -> Option<&RollNote> {
        self.live.get(&key)
    }

    /// File a finished note into `past` at its place in release order: almost
    /// always the end, but kept sorted on every insert rather than assumed,
    /// because trim, activity and the note cap all read the order. Equal
    /// releases keep their arrival order.
    ///
    /// A note marked off the timeline is dropped instead: it was a voice and
    /// nothing else. The note cap applies as each note is filed.
    fn retire(&mut self, note: RollNote) {
        if note.off_timeline {
            return;
        }
        let at = note.departed();
        let index = self.past.partition_point(|other| other.departed() <= at);
        self.past.insert(index, note);
        self.enforce_cap();
    }

    pub(crate) fn set_shown(&mut self, source: SourceId, shown: bool) {
        if shown {
            self.hidden_sources.remove(&source);
        } else {
            self.hidden_sources.insert(source);
        }
    }

    /// One source's complete held set, `voices` mapped onto the clock by
    /// `offset`.
    ///
    /// Rule 5: a held note it omits leaves observed until now, straight into
    /// `past` with no fade and no trail mark.
    ///
    /// Rule 6: a fading note it lists that has an end — a factual off, a
    /// reset — is retired without a trail mark, since the source says the
    /// note is sounding: filed in `past`, or dropped if it is marked off the
    /// timeline, a rule-1 pulse or a note Clear took. A fresh entry starts at
    /// the row's onset.
    ///
    /// A note Clear took off the timeline is REVIVED by a row that lists it:
    /// replaced by a fresh entry from the row, so the
    /// Roll draws a flat line from the baseline as it does for any note it has
    /// no history of, and the voice's onset follows it.
    pub(crate) fn replace_source(
        &mut self,
        source: SourceId,
        voices: &[VoiceBaseline],
        at: Time,
        offset: Time,
    ) {
        let matches = |row: &VoiceBaseline, note: &RollNote| {
            row.matches_voice(source, note.key(), note.lifetime)
        };
        let omitted: Vec<_> = self
            .live
            .iter()
            .filter(|(key, note)| {
                key.source == source && !voices.iter().any(|row| matches(row, note))
            })
            .map(|(key, _)| *key)
            .collect();
        for key in omitted {
            let mut note = self.live.remove(&key).unwrap();
            note.observed_until = Some(at.max(note.start));
            self.retire(note);
        }
        for row in voices {
            let key = row.key(source);
            let ended: Vec<_> = self.fading.extract_if(.., |note| matches(row, note)).collect();
            for note in ended {
                self.retire(note);
            }
            if let Some(note) = self.live.get_mut(&key).filter(|note| !note.off_timeline) {
                if note.end_pitch() != row.pitch() {
                    // A baseline is a state observation, not an expression
                    // path connecting the previous sample to the recovered one.
                    note.history_complete = false;
                    note.push_point(at, row.pitch(), true);
                }
                note.express(at, row.expressions);
                continue;
            }
            let start = row.actual_onset + offset;
            let mut note =
                RollNote::new(key, row.velocity, start, at, row.pitch(), row.expressions);
            note.lifetime = Some(row.lifetime);
            note.history_complete = false;
            self.live.insert(key, note);
        }
    }

    /// Record a per-note tuning change on a sounding note. Unknown keys
    /// are ignored (the bend arrived after the note-off).
    pub fn bend(&mut self, key: VoiceKey, at: Time, pitch: f32) {
        if let Some(note) = self.live.get_mut(&key) {
            note.bend(at, pitch);
        }
    }

    /// Record a sounding note's expressions changing. Unknown keys are ignored,
    /// as for [`bend`](Self::bend).
    pub fn express(&mut self, key: VoiceKey, at: Time, values: Expressions) {
        if let Some(note) = self.live.get_mut(&key) {
            note.express(at, values);
        }
    }

    /// One of a sounding note's expressions changing, the others held.
    pub(crate) fn set_expression(
        &mut self,
        key: VoiceKey,
        at: Time,
        expression: Expression,
        value: f32,
    ) {
        if let Some(note) = self.live.get_mut(&key) {
            let mut values = note.expressions[note.expressions.len() - 1].1;
            values.set(expression, value);
            note.express(at, values);
        }
    }

    /// Every sounding note but the tape's: what a session reset or a
    /// stream-wide gap reaches, which the tape is not part of.
    fn published(&self) -> Vec<VoiceKey> {
        self.live.keys().filter(|key| !key.source.is_tape()).copied().collect()
    }

    /// End the held notes `keys` names with a fade, at an instant that is a
    /// transport event's rather than any one note's: a note it closes at or
    /// before its onset still sounded, its length unknown.
    fn end_all(&mut self, keys: Vec<VoiceKey>, at: Time) {
        for key in keys {
            let mut note = self.live.remove(&key).expect("a live key");
            note.end = Some(at.max(note.start));
            self.release(note);
        }
    }

    pub(crate) fn published_off(&mut self, at: Time) {
        self.end_all(self.published(), at);
    }

    /// Close every sounding note (transport reset).
    pub fn all_off(&mut self, at: Time) {
        self.end_all(self.live.keys().copied().collect(), at);
    }

    /// Close this source's sounding notes, with the same unknown-duration
    /// clamp as a session reset. Other sources' bend histories remain live.
    pub fn source_off(&mut self, source: SourceId, at: Time) {
        self.end_all(self.live.keys().filter(|k| k.source == source).copied().collect(), at);
    }

    /// A roll that keeps every note it is given. Nothing trims it either:
    /// [`trim`](Self::trim) is the caller's, and nothing prunes it, so what
    /// it releases stays in `fading`, on the timeline; only a retrigger files
    /// a note in `past`.
    pub fn uncapped() -> Self {
        Self { uncapped: true, ..Self::default() }
    }

    fn enforce_cap(&mut self) {
        while !self.uncapped && self.past.len() > Self::MAX_NOTES {
            self.past.pop_front();
        }
    }

    /// Forget notes that finished more than [`MAX_AGE`](Self::MAX_AGE) ago.
    /// Cheap: `past` is release-ordered, so this stops at the first keeper.
    /// Reaches what is in `past`, so for a roll the tracker prunes it is every
    /// finished note; what is still fading is not finished.
    pub fn trim(&mut self, now: Time) {
        let cutoff = now - Self::MAX_AGE;
        while self.past.front().is_some_and(|n| n.departed().is_some_and(|e| e < cutoff)) {
            self.past.pop_front();
        }
    }

    /// End each fade that is over: the note records its pitch and moves to
    /// `past`, or is dropped if it is off the timeline. Then the two bounds.
    /// Called once per frame through
    /// [`NoteTracker::prune`](crate::NoteTracker::prune).
    pub(crate) fn prune(&mut self, now: Time, env: &Envelope, mut record: impl FnMut(&Voice)) {
        let over: Vec<_> = self.fading.extract_if(.., |note| !note.fading_at(now, env)).collect();
        for note in over {
            record(&note.voice());
            self.retire(note);
        }
        self.trim(now);
    }

    /// The source's notes are hidden: its Show switch is off, or for a note
    /// the tape plays, the switch of the source that played it.
    pub fn hides(&self, source: SourceId) -> bool {
        self.hidden_sources.contains(&source) || self.hidden_sources.contains(&source.off_tape())
    }

    /// Held presses as voices, hidden sources' included.
    pub(crate) fn held(&self) -> impl Iterator<Item = Voice> + '_ {
        self.live.values().map(RollNote::voice)
    }

    /// Every voice to draw: held presses whose source shows NOW, in key order,
    /// then the fading ones in the order their releases arrived. Rule 4 has
    /// already left out a release that was hidden.
    pub(crate) fn voices(&self) -> impl Iterator<Item = Voice> + '_ {
        self.live
            .values()
            .filter(|note| !self.hides(note.source))
            .chain(&self.fading)
            .map(RollNote::voice)
    }

    /// Every timeline note, hidden sources' included: finished, then fading,
    /// then sounding. Leaves out what is marked off it; `past` holds none.
    fn timeline(&self) -> impl Iterator<Item = &RollNote> {
        let on = |n: &&RollNote| !n.off_timeline;
        self.past.iter().chain(self.fading.iter().chain(self.live.values()).filter(on))
    }

    /// Every remembered note, finished ones first, then the fading ones and
    /// then the sounding ones in `(source, channel, note)` order. Not sorted
    /// by start time: overlapping notes draw as overlapping ribbons either
    /// way, and the pane sorts for itself when the paint order matters.
    pub fn notes(&self) -> impl Iterator<Item = &RollNote> {
        self.timeline().filter(|n| !self.hides(n.source))
    }

    /// The most recent moment any remembered note was sounding — `now`
    /// while anything is held — or `None` if the roll is empty.
    ///
    /// This is how a display decides whether it still has to animate: a
    /// roll scrolls for as long as its window reaches back to something,
    /// which outlasts the last voice's release fade. `past` is
    /// release-ordered, so its latest release is the one on the end; `fading`
    /// is scanned, which for a roll the tracker prunes is a fade's worth of
    /// notes.
    pub fn latest_activity(&self, now: Time) -> Option<Time> {
        let shown = |n: &&RollNote| !n.off_timeline && !self.hides(n.source);
        if self.live.values().any(|n| shown(&n)) {
            return Some(now);
        }
        let past = self.past.iter().rev().find(|n| !self.hides(n.source));
        let past = past.and_then(RollNote::departed);
        self.fading
            .iter()
            .filter(shown)
            .filter_map(RollNote::departed)
            .chain(past)
            .reduce(Time::max)
    }

    pub fn len(&self) -> usize {
        self.timeline().count()
    }

    pub fn is_empty(&self) -> bool {
        self.timeline().next().is_none()
    }

    /// Forget the timeline, sounding notes included. A note held or fading
    /// across a clear stays a voice but leaves the timeline, and its bends
    /// land nowhere the Roll draws, until a baseline revives it or it is played
    /// again. That is what "clear the roll" is asked for; the alternative (keep
    /// the held ones) leaves a stub whose start time is already off-screen.
    pub fn clear(&mut self) {
        self.past.clear();
        for note in self.live.values_mut().chain(&mut self.fading) {
            note.off_timeline = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::{Envelope, NoteEvent, NoteEventKind, NoteTracker};

    fn on(time: Time, note: u8) -> NoteEvent {
        NoteEvent::on(time, crate::SourceId::DIRECT, 0, note, 0.8)
    }

    fn off(time: Time, note: u8) -> NoteEvent {
        NoteEvent::off(time, crate::SourceId::DIRECT, 0, note)
    }

    fn bend(time: Time, note: u8, semitones: f32) -> NoteEvent {
        NoteEvent {
            source: crate::SourceId::DIRECT,
            time,
            channel: 0,
            note,
            kind: NoteEventKind::Tuning { semitones },
        }
    }

    #[test]
    fn a_note_is_in_the_roll_the_moment_it_sounds() {
        // Unlike history, which only takes a note once its fade completes:
        // a piano roll has to draw the note being held right now.
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        let roll = tracker.roll();
        assert_eq!(roll.len(), 1);
        let note = roll.notes().next().unwrap();
        assert_eq!(note.start, 0.0);
        assert!(note.is_live());
        assert_eq!(note.end, None);
    }

    #[test]
    fn a_held_note_reaches_the_present_moment() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(1.0, 60));
        let note = tracker.roll().notes().next().unwrap();
        assert_eq!(note.stop(9.0), 9.0);
        tracker.handle_event(off(4.0, 60));
        let note = tracker.roll().notes().next().unwrap();
        assert_eq!(note.stop(9.0), 4.0, "a released note stops at its release");
    }

    /// Sounding notes reach the pane through `notes()`, and the transport
    /// reset walks them into `fading` and, once the prune ends their fades,
    /// into `past` — which then holds that order for as long as it remembers
    /// them. All three have to be the music's order, not a map's; see
    /// `NoteTracker::voices` and #135.
    ///
    /// Pressed high to low so only the key order can pass.
    #[test]
    fn sounding_notes_come_back_in_channel_note_order() {
        let mut tracker = NoteTracker::new();
        let expected: Vec<u8> = (40..104u8).collect();
        for &note in expected.iter().rev() {
            tracker.handle_event(on(0.0, note));
        }
        let live: Vec<u8> = tracker.roll().notes().map(|n| n.note).collect();
        assert_eq!(live, expected, "sounding notes must iterate in key order");

        tracker.session_notes_off(1.0);
        let fading: Vec<u8> = tracker.roll().notes().map(|n| n.note).collect();
        assert_eq!(fading, expected, "the fading tail must inherit key order");
        tracker.prune(3.0, &Envelope::default());
        assert_eq!(tracker.voices().count(), 0, "every fade has ended, so all are in the past");
        let past: Vec<u8> = tracker.roll().notes().map(|n| n.note).collect();
        assert_eq!(past, expected, "the finished notes keep it");
    }

    #[test]
    fn repeats_of_one_pitch_stay_separate_entries() {
        // The opposite of NoteHistory, which folds them into one visit.
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(off(1.0, 60));
        tracker.handle_event(on(2.0, 60));
        tracker.handle_event(off(3.0, 60));
        assert_eq!(tracker.roll().len(), 2);
    }

    /// A note switched off the instant it started never sounded, and the roll
    /// does not remember it.
    #[test]
    fn a_note_switched_off_the_instant_it_started_never_sounded() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(1.0, 60));
        tracker.handle_event(off(1.0, 60));
        assert!(tracker.roll().is_empty());
    }

    /// The shortest note anyone can play still ends after it began, and is
    /// kept — the rule above is a fact about the note, not a span short enough
    /// to look wrong.
    #[test]
    fn a_staccato_note_is_kept_however_brief() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(1.0, 60));
        tracker.handle_event(off(1.000_001, 60));
        assert_eq!(tracker.roll().len(), 1);
    }

    /// One press, as a host really delivers it: an on, an off and a second on
    /// all stamped at the same sample, then the release nearly two seconds
    /// later. That is ONE note on the roll.
    ///
    /// The pair in the middle otherwise leaves an entry that begins and ends
    /// together — and what shows is not its ribbon, which is floored to a
    /// couple of pixels, but the NAME written on it at full size and anchored
    /// where it ended. So a single press put a letter on the pane that
    /// scrolled away from the letter held at the now-line, as though the key
    /// had been played twice.
    #[test]
    fn one_press_delivered_as_on_off_on_is_one_note() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(9.9314, 60));
        tracker.handle_event(off(9.9314, 60));
        tracker.handle_event(on(9.9314, 60));
        assert_eq!(tracker.roll().len(), 1, "the off/on pair is not a note of its own");
        assert!(tracker.roll().notes().next().unwrap().is_live(), "the press is still sounding");

        tracker.handle_event(off(11.6881, 60));
        let roll = tracker.roll();
        assert_eq!(roll.len(), 1);
        let note = roll.notes().next().unwrap();
        assert_eq!((note.start, note.end), (9.9314, Some(11.6881)));
    }

    #[test]
    fn a_retrigger_without_an_off_closes_the_previous_entry() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(on(2.0, 60));
        let roll = tracker.roll();
        assert_eq!(roll.len(), 2);
        let closed = roll.notes().find(|n| !n.is_live()).unwrap();
        assert_eq!((closed.start, closed.end), (0.0, Some(2.0)));
    }

    #[test]
    fn bends_become_breakpoints_and_interpolate() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(bend(1.0, 60, 2.0));
        tracker.handle_event(off(2.0, 60));
        let roll = tracker.roll();
        let note = roll.notes().next().unwrap();
        assert_eq!(note.start_pitch(), 60.0);
        assert_eq!(note.end_pitch(), 62.0);
        let segments: Vec<_> = note.segments(5.0).collect();
        assert_eq!(segments.len(), 2, "{segments:?}");
        // The glide, then the flat tail at the bent pitch.
        assert_eq!(segments[0], ((0.0, 60.0), (1.0, 62.0)));
        assert_eq!(segments[1], ((1.0, 62.0), (2.0, 62.0)));
    }

    #[test]
    fn an_unbent_note_is_one_flat_segment() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 67));
        tracker.handle_event(off(1.5, 67));
        let roll = tracker.roll();
        let segments: Vec<_> = roll.notes().next().unwrap().segments(9.0).collect();
        assert_eq!(segments, vec![((0.0, 67.0), (1.5, 67.0))]);
    }

    #[test]
    fn breakpoints_are_capped_and_the_newest_wins() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        for i in 1..(RollNote::MAX_BENDS as i32 * 2) {
            tracker.handle_event(bend(i as Time, 60, i as f32 * 0.01));
        }
        let roll = tracker.roll();
        let note = roll.notes().next().unwrap();
        assert!(note.segments(999.0).count() <= RollNote::MAX_BENDS);
        // The current pitch survives the fold, which is the point of it.
        let last = (RollNote::MAX_BENDS as i32 * 2 - 1) as f32 * 0.01;
        assert!((note.end_pitch() - (60.0 + last)).abs() < 1e-4);
    }

    #[test]
    fn all_off_closes_every_sounding_note() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(on(0.0, 64));
        tracker.handle_event(NoteEvent {
            source: crate::SourceId::DIRECT,
            time: 3.0,
            channel: 0,
            note: 0,
            kind: NoteEventKind::SessionReset,
        });
        let roll = tracker.roll();
        assert_eq!(roll.len(), 2);
        assert!(roll.notes().all(|n| n.end == Some(3.0)));
    }

    /// No channel is filtered out on its way in. The top one is the case
    /// worth naming: it is v1's reserved channel, and the reservation is
    /// gone — a note there is a note like any other, tracked, rolled and
    /// colored by its pitch.
    #[test]
    fn every_channel_reaches_the_roll() {
        for channel in 0..16u8 {
            let mut tracker = NoteTracker::new();
            tracker.handle_event(NoteEvent::on(0.0, crate::SourceId::DIRECT, channel, 60, 0.8));
            assert_eq!(tracker.roll().len(), 1, "channel {channel}");
        }
    }

    #[test]
    fn old_notes_age_out_but_sounding_ones_never_do() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(off(1.0, 60));
        tracker.handle_event(on(1.0, 72)); // still held at the far future
        tracker.prune(NoteRoll::MAX_AGE + 100.0, &Envelope::default());
        let roll = tracker.roll();
        assert_eq!(roll.len(), 1);
        assert!(roll.notes().next().unwrap().is_live());
    }

    /// The cap is on finished notes, which a note becomes once its fade ends:
    /// the prune is what files them, so the fixture runs one.
    #[test]
    fn the_note_cap_drops_the_oldest_releases_first() {
        let mut tracker = NoteTracker::new();
        let presses = NoteRoll::MAX_NOTES + 8;
        for i in 0..presses {
            let t = i as Time * 0.1;
            tracker.handle_event(on(t, 60));
            tracker.handle_event(off(t + 0.05, 60));
        }
        tracker.prune(presses as Time * 0.1 + 2.0, &Envelope::default());
        let roll = tracker.roll();
        assert_eq!(roll.len(), NoteRoll::MAX_NOTES);
        assert!(
            roll.notes().all(|n| n.start >= 0.75),
            "the first eight presses should have been dropped"
        );
    }

    /// A retuned note is a note-on at its equal-tempered pitch followed by a
    /// tuning expression, so the pitch it was PRESSED at says nothing about
    /// which note it is — in a just-intoned piece every note would answer on
    /// the same twelve. What it settles on within `SETTLE` is the answer; a
    /// bend after that is a glide, and leaves the note's identity alone.
    #[test]
    fn a_note_settles_on_the_pitch_its_tuning_lands_it_at() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 64));
        assert_eq!(tracker.roll().notes().next().unwrap().settled_pitch(), 64.0);

        // The tuning, arriving a block behind its note-on.
        tracker.handle_event(bend(0.01, 64, -0.137));
        let note = tracker.roll().notes().next().unwrap();
        assert!((note.settled_pitch() - 63.863).abs() < 1e-4, "{}", note.settled_pitch());
        assert_eq!(note.start_pitch(), 64.0, "the KEY is still where it always was");

        // A glide, well past the window: it moves where the note sounds and
        // not which note it is.
        tracker.handle_event(bend(1.0, 64, 2.0));
        let note = tracker.roll().notes().next().unwrap();
        assert!((note.settled_pitch() - 63.863).abs() < 1e-4);
        assert_eq!(note.end_pitch(), 66.0);
    }

    /// A bend LANDING on the window's edge is still part of the onset, and one
    /// past it is not — the constant is a real boundary, not a rough size.
    #[test]
    fn the_settle_window_has_edges() {
        let settled = |at: Time| -> f32 {
            let mut tracker = NoteTracker::new();
            tracker.handle_event(on(0.0, 60));
            tracker.handle_event(bend(at, 60, 1.0));
            let pitch = tracker.roll().notes().next().unwrap().settled_pitch();
            pitch
        };
        assert_eq!(settled(RollNote::SETTLE), 61.0, "on the edge counts");
        assert_eq!(settled(RollNote::SETTLE * 2.0), 60.0, "past it does not");
    }

    /// The settled pitch does not move once the onset is over — including
    /// under an expression ramp fast enough to fill the breakpoint vector
    /// inside the onset itself.
    ///
    /// Read back out of `bends` it DOES move: the MAX_BENDS fold overwrites
    /// the last breakpoint in place, so the final in-window one is eventually
    /// replaced by an out-of-window one and `take_while` drops back a step.
    /// Small — one sub-50 ms increment — and enough to re-place a name that
    /// had been told the pitch was fixed.
    #[test]
    fn a_ramp_that_fills_the_breakpoints_cannot_move_the_settled_pitch() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        // Fill the vector inside the settle window...
        for i in 1..RollNote::MAX_BENDS {
            let at = RollNote::SETTLE * (i as Time / RollNote::MAX_BENDS as Time);
            tracker.handle_event(bend(at, 60, i as f32 * 0.001));
        }
        let settled = tracker.roll().notes().next().unwrap().settled_pitch();
        assert!(settled > 60.0, "the ramp did land inside the window: {settled}");

        // ...then keep bending, long after. The fold now rewrites the last
        // breakpoint, which was the one inside the window.
        for i in 1..10 {
            tracker.handle_event(bend(1.0 + i as Time, 60, 1.0 + i as f32));
        }
        let note = tracker.roll().notes().next().unwrap();
        assert_eq!(note.settled_pitch(), settled, "the onset is over; the answer is fixed");
    }

    fn pressure(time: Time, note: u8, value: f32) -> NoteEvent {
        NoteEvent {
            source: crate::SourceId::DIRECT,
            time,
            channel: 0,
            note,
            kind: NoteEventKind::Expression { expression: crate::Expression::Pressure, value },
        }
    }

    /// A level held through a quiet stretch stays level until the moment it
    /// changes: the change is a step at its own time, not a ramp from the last
    /// one. What arrives after the release reaches neither the voice nor the
    /// roll.
    #[test]
    fn an_expression_after_a_held_level_is_a_step() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        tracker.handle_event(pressure(0.0, 60, 0.25));
        tracker.handle_event(pressure(2.0, 60, 0.75));
        assert_eq!(tracker.voices().next().unwrap().expressions.pressure, 0.75);
        tracker.handle_event(off(3.0, 60));
        tracker.handle_event(pressure(3.5, 60, 1.0));
        let note = tracker.roll().notes().next().unwrap();
        let points: Vec<_> = note.expressions().iter().map(|(t, e)| (*t, e.pressure)).collect();
        assert_eq!(points, vec![(0.0, 0.25), (2.0, 0.25), (2.0, 0.75)]);
    }

    /// Read at a time, the history is straight lines that hold at both ends,
    /// and a step reads its new value from its own instant on.
    #[test]
    fn expressions_at_reads_lines_and_steps() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(1.0, 60));
        let mut note = tracker.roll().notes().next().unwrap().clone();
        let point = |t, pressure| (t, crate::Expressions { pressure, ..Default::default() });
        // Two seconds between the first points, so a line's fraction of the
        // way is not the seconds it has run.
        note.expressions = vec![point(1.0, 0.2), point(3.0, 0.6), point(4.0, 0.6), point(4.0, 1.0)];
        let at = |t| note.expressions_at(t).pressure;
        assert_eq!(at(0.0), 0.2, "before the first point, it holds");
        assert!((at(2.0) - 0.4).abs() < 1e-6, "a line between two points: {}", at(2.0));
        assert_eq!(at(3.9), 0.6, "a held level stays level up to its step");
        assert_eq!(at(4.0), 1.0, "and takes the new value at the step itself");
        assert_eq!(at(9.0), 1.0, "after the last, the last holds");
    }

    /// Pressure every millisecond for ten seconds: the history stays inside
    /// its budget, still covers the whole note rather than its first moments,
    /// and ends on the current value.
    #[test]
    fn a_dense_expression_thins_across_the_whole_note() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        let value = |i: u32| (i % 1000) as f32 / 1000.0;
        for i in 1..=10_000 {
            tracker.handle_event(pressure(Time::from(i) / 1000.0, 60, value(i)));
        }
        let note = tracker.roll().notes().next().unwrap();
        let points = note.expressions();
        assert!(points.len() <= RollNote::MAX_EXPRESSIONS, "{} points", points.len());
        assert!(points.len() > RollNote::MAX_EXPRESSIONS / 2 - 2, "{} points", points.len());
        for second in 0..10 {
            let within = |t: Time| t >= Time::from(second) && t < Time::from(second + 1);
            assert!(points.iter().any(|(t, _)| within(*t)), "nothing in second {second}");
        }
        assert_eq!(
            *points.last().unwrap(),
            (10.0, crate::Expressions { pressure: value(10_000), ..Default::default() })
        );
    }

    #[test]
    fn clear_forgets_everything() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(on(0.0, 60));
        assert!(!tracker.roll().is_empty(), "a sounding note alone is something");
        tracker.handle_event(off(1.0, 60));
        tracker.handle_event(on(1.0, 64));
        assert!(!tracker.roll().is_empty());
        tracker.clear_roll();
        assert!(tracker.roll().is_empty());
    }

    /// Two sources mapped onto the clock by their own offsets: one's release
    /// can ARRIVE after a later release of the other's. The roll still reads
    /// as release order — the latest release is the latest activity, and the
    /// older release ages out on time rather than waiting behind the newer.
    /// Both fades have ended by the first prune, so the order read is the
    /// finished notes'.
    #[test]
    fn releases_arriving_out_of_clock_order_keep_release_order() {
        let mut tracker = NoteTracker::new();
        let (a, b) = (SourceId(1), SourceId(2));
        tracker.handle_event(NoteEvent::on(0.0, a, 0, 60, 0.8));
        tracker.handle_event(NoteEvent::on(0.0, b, 0, 60, 0.8));
        tracker.handle_event(NoteEvent::off(10.0, a, 0, 60));
        tracker.handle_event(NoteEvent::off(2.0, b, 0, 60)); // arrives last, released first
        tracker.prune(20.0, &Envelope::default());
        assert_eq!(tracker.roll().latest_activity(20.0), Some(10.0));
        tracker.prune(2.0 + NoteRoll::MAX_AGE + 1.0, &Envelope::default());
        let left: Vec<_> = tracker.roll().notes().map(|n| n.end).collect();
        assert_eq!(left, [Some(10.0)], "the older release is forgotten on time");
    }

    /// A whole recording's roll keeps the song's start however many notes
    /// follow it. Retriggers, because a retrigger is what files a note in
    /// `past` where the cap reads, in a roll nothing prunes.
    #[test]
    fn an_uncapped_roll_keeps_every_note() {
        let mut roll = NoteRoll::uncapped();
        let key = VoiceKey { source: SourceId::DIRECT, channel: 0, note: 60 };
        for i in 0..NoteRoll::MAX_NOTES + 10 {
            roll.note_on(key, 1.0, 60.0, i as f64);
        }
        roll.all_off(NoteRoll::MAX_NOTES as f64 + 10.0);
        assert_eq!(roll.len(), NoteRoll::MAX_NOTES + 10);
        assert_eq!(roll.notes().map(|n| n.start).reduce(f64::min), Some(0.0), "the song's start");
    }
}
