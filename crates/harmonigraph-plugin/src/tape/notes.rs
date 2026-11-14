//! The tape's notes (#1719 slice 3): what the display showed while the song
//! played, recorded by song position beside the audio, and played back into
//! the display during an audition.
//!
//! - **What is recorded** is the display's own reading of each note, after the
//!   Hub's tuning and any pedal: the live drain compares the tracker's held
//!   voices before and after every event it applies ([`Recorder`]), so a mark
//!   is what the picture did, not what the wire said.
//! - **Each pass keeps its own timeline** ([`PassNotes`]), never cut into the
//!   store's chunks. What sounds at a song frame is the pass whose audio won
//!   there, so the notes at a frame are that pass's, replayed from its start
//!   ([`Replay`]): a chunk trimmed by a newer pass changes which timeline is
//!   read there and nothing in any timeline. That is what makes the chase at a
//!   cue exact, where per-chunk marks lost the notes held across a cut.
//! - **Playback** ([`Player`]) follows the analyzer ring: each block says
//!   which stretch of song the tape added to it, and the notes in that stretch
//!   reach the tracker at the display time of the frames they sound in, under
//!   tape-owned sources ([`harmonigraph_core::SourceId::on_tape`]).

use std::collections::BTreeMap;

use harmonigraph_core::notes::{Expression, NoteEvent, NoteEventKind, NoteTracker};
use harmonigraph_core::{Expressions, NoteRoll, Tuning, VoiceKey};
use harmonigraph_ui::tape::Audition;

use super::store::Store;

/// What one held voice sounds like.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sounding {
    pub velocity: f32,
    /// MIDI note units, per-note tuning included.
    pub pitch: f32,
    pub expressions: Expressions,
}

/// What the display did to one voice.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Mark {
    On {
        key: VoiceKey,
        sounding: Sounding,
    },
    /// A held voice moved: a bend, a tuning or an expression.
    Change {
        key: VoiceKey,
        sounding: Sounding,
    },
    Off {
        key: VoiceKey,
    },
}

impl Mark {
    pub(crate) fn apply(&self, held: &mut BTreeMap<VoiceKey, Sounding>) {
        match *self {
            Mark::On { key, sounding } | Mark::Change { key, sounding } => {
                held.insert(key, sounding);
            }
            Mark::Off { key } => {
                held.remove(&key);
            }
        }
    }
}

/// One held voice as the recorder compares it: identity, onset, and sound.
type Voice = (VoiceKey, f64, Sounding);

/// The live drain's half: the marks one tracker event made, read as the
/// change in the held voices since the last event. The baseline is what the
/// marks so far say is held, so a change made to the tracker outside the drain
/// is recorded at the next event rather than lost, and so is a change the
/// [`NOTES`] bound held back.
#[derive(Default)]
pub(crate) struct Recorder {
    before: Vec<Voice>,
    after: Vec<Voice>,
    next: Vec<Voice>,
}

/// Marks the live drain may hold for one step; past it, a step's marks wait
/// for the next event with room, so they are recorded late rather than lost.
/// Swapped out every step.
pub(crate) const NOTES: usize = 4096;

impl Recorder {
    fn held(tracker: &NoteTracker, into: &mut Vec<Voice>) {
        into.clear();
        into.extend(tracker.held_voices().filter(|v| !v.source.is_tape()).map(|v| {
            let sounding =
                Sounding { velocity: v.velocity, pitch: v.pitch, expressions: v.expressions };
            (v.key(), v.on_time, sounding)
        }));
    }

    /// After the tracker applies an event: what changed, at presentation
    /// time `time`, into `out`.
    ///
    /// A voice whose marks do not fit keeps its old reading in the baseline,
    /// so the next event's comparison makes them again. Moving the baseline
    /// to what was seen instead lost a dropped Off for good, and the tape
    /// held that note to the end of its pass on every playback.
    pub(crate) fn record(&mut self, tracker: &NoteTracker, time: f64, out: &mut Vec<(f64, Mark)>) {
        Self::held(tracker, &mut self.after);
        self.next.clear();
        let next = &mut self.next;
        // Every mark one voice needs, or none of them.
        let mut push = |marks: &[Mark]| {
            let fits = out.len() + marks.len() <= NOTES;
            if fits {
                out.extend(marks.iter().map(|&mark| (time, mark)));
            }
            fits
        };
        // Both are in key order: the tracker's held map is a BTreeMap.
        let (mut a, mut b) = (self.before.iter().peekable(), self.after.iter().peekable());
        loop {
            match (a.peek(), b.peek()) {
                (None, None) => break,
                (Some(&&old @ (key, _, _)), None) => {
                    if !push(&[Mark::Off { key }]) {
                        next.push(old);
                    }
                    a.next();
                }
                (None, Some(&&new @ (key, _, sounding))) => {
                    if push(&[Mark::On { key, sounding }]) {
                        next.push(new);
                    }
                    b.next();
                }
                (Some(&&old), Some(&&new)) => {
                    let ((was_key, onset, was), (key, again, sounding)) = (old, new);
                    if was_key < key {
                        if !push(&[Mark::Off { key: was_key }]) {
                            next.push(old);
                        }
                        a.next();
                        continue;
                    }
                    if key < was_key {
                        if push(&[Mark::On { key, sounding }]) {
                            next.push(new);
                        }
                        b.next();
                        continue;
                    }
                    let recorded = if onset != again {
                        push(&[Mark::Off { key }, Mark::On { key, sounding }])
                    } else if was != sounding {
                        push(&[Mark::Change { key, sounding }])
                    } else {
                        true
                    };
                    next.push(if recorded { new } else { old });
                    a.next();
                    b.next();
                }
            }
        }
        std::mem::swap(&mut self.before, &mut self.next);
    }
}

/// Marks between two of a pass's checkpoints: what bounds a chase. A dense
/// pass (MPE pressure on four notes for half an hour, 675k marks) replays
/// from its start in 4.6 ms, under the editor lock, at every cue and seam.
const CHECKPOINT: usize = 1024;

/// One pass's notes and tuning, by song frame.
#[derive(Default)]
pub(crate) struct PassNotes {
    /// Sorted by frame; marks at one frame in the order they happened. Opens
    /// with the voices already held when the pass started.
    pub events: Vec<(i64, Mark)>,
    /// The tuning axes from each frame on, sorted: what the display drew
    /// with, sampled once a step. Only the axes, the one thing an audition
    /// draws with.
    pub tunings: Vec<(i64, Tuning)>,
    /// The voices held after the first `n` events, every [`CHECKPOINT`]
    /// events, so a chase replays from the nearest one rather than the
    /// pass's start. A mark placed among them drops those after it, and
    /// they are taken again as the timeline grows.
    checkpoints: Vec<(usize, BTreeMap<VoiceKey, Sounding>)>,
}

impl PassNotes {
    pub(crate) fn place(&mut self, frame: i64, mark: Mark) {
        let at = self.events.partition_point(|(f, _)| *f <= frame);
        self.events.insert(at, (frame, mark));
        self.checkpoints.retain(|(n, _)| *n <= at);
        loop {
            let last = self.checkpoints.last().map_or(0, |(n, _)| *n);
            let to = last + CHECKPOINT;
            if to > self.events.len() {
                break;
            }
            let mut replay = self.replay_from(last);
            while replay.next < to {
                self.events[replay.next].1.apply(&mut replay.held);
                replay.next += 1;
            }
            self.checkpoints.push((to, replay.held));
        }
    }

    /// A replay standing after the checkpoint at `n` events, or the start.
    fn replay_from(&self, n: usize) -> Replay {
        self.checkpoints
            .iter()
            .rev()
            .find(|(at, _)| *at == n)
            .map_or_else(Replay::default, |(at, held)| Replay { next: *at, held: held.clone() })
    }

    /// The tuning at `frame`: the last change at or before it, else the
    /// first, which the pass's opening moments were drawn with too.
    pub(crate) fn tuning_at(&self, frame: i64) -> Option<Tuning> {
        let after = self.tunings.partition_point(|(f, _)| *f <= frame);
        self.tunings.get(after.saturating_sub(1)).map(|(_, tuning)| *tuning)
    }

    pub(crate) fn bytes(&self) -> usize {
        self.events.len() * std::mem::size_of::<(i64, Mark)>()
            + self.tunings.len() * std::mem::size_of::<(i64, Tuning)>()
    }
}

/// A pass's timeline read forward: the voices held after every mark before
/// [`advance`](Self::advance)'s frame.
#[derive(Clone, Default)]
pub(crate) struct Replay {
    next: usize,
    pub held: BTreeMap<VoiceKey, Sounding>,
}

impl Replay {
    /// Apply every mark before frame `to`, handing each to `emit` first.
    pub(crate) fn advance(
        &mut self,
        notes: &PassNotes,
        to: i64,
        mut emit: impl FnMut(i64, &Mark, &BTreeMap<VoiceKey, Sounding>),
    ) {
        while let Some((frame, mark)) = notes.events.get(self.next).filter(|(f, _)| *f < to) {
            emit(*frame, mark, &self.held);
            mark.apply(&mut self.held);
            self.next += 1;
        }
    }

    /// The voices `notes` holds at `frame`, read on from the last
    /// checkpoint before it.
    pub(crate) fn at(notes: &PassNotes, frame: i64) -> Replay {
        let before = notes.events.partition_point(|(f, _)| *f < frame);
        let start =
            notes.checkpoints.iter().rev().find(|(n, _)| *n <= before).map_or(0, |(n, _)| *n);
        let mut replay = notes.replay_from(start);
        replay.advance(notes, frame, |_, _, _| {});
        replay
    }
}

/// The whole recording's notes along the song, in song seconds, for Tape
/// mode's roll: each unbroken run of one pass's chunks draws that pass's
/// notes over the run, opening with what it held there and closing at its
/// end.
pub(crate) fn song_roll(store: &Store, passes: &BTreeMap<u64, PassNotes>) -> NoteRoll {
    let rate = f64::from(store.rate);
    let seconds = |frame: i64| frame as f64 / rate;
    let mut roll = NoteRoll::uncapped();
    let mut replays: BTreeMap<u64, Replay> = BTreeMap::new();
    let mut run: Option<(u64, i64)> = None;
    let on = |roll: &mut NoteRoll, key: VoiceKey, s: &Sounding, at: f64| {
        roll.note_on(key, s.velocity, s.pitch, at);
        roll.express(key, at, s.expressions);
    };
    for chunk in store.chunks() {
        let pass = chunk.stamp.pass;
        let Some(notes) = passes.get(&pass) else {
            if let Some((_, end)) = run.take() {
                roll.all_off(seconds(end));
            }
            continue;
        };
        let replay = replays.entry(pass).or_default();
        if run != Some((pass, chunk.start)) {
            if let Some((_, end)) = run {
                roll.all_off(seconds(end));
            }
            replay.advance(notes, chunk.start, |_, _, _| {});
            for (key, sounding) in &replay.held {
                on(&mut roll, *key, sounding, seconds(chunk.start));
            }
        }
        replay.advance(notes, chunk.end, |frame, mark, _| {
            let at = seconds(frame);
            match *mark {
                Mark::On { key, sounding } => on(&mut roll, key, &sounding, at),
                Mark::Change { key, sounding } => {
                    roll.bend(key, at, sounding.pitch);
                    roll.express(key, at, sounding.expressions);
                }
                Mark::Off { key } => roll.note_off(key, at),
            }
        });
        run = Some((pass, chunk.end));
    }
    if let Some((_, end)) = run {
        roll.all_off(seconds(end));
    }
    roll
}

/// Playback's half: what the tape has put in the tracker, and where in the
/// song it has got to.
#[derive(Default)]
pub(crate) struct Player {
    /// The song frame the next block of tape should continue from.
    cursor: Option<i64>,
    /// The pass whose notes are sounding, read up to `cursor`.
    owner: Option<(u64, Replay)>,
    /// The voices the tape holds in the tracker, by their recorded keys.
    sounding: BTreeMap<VoiceKey, Sounding>,
    /// The audition the notes belong to, for Tape mode.
    pub audition: Option<Audition>,
    /// The tuning axes at the cursor, while an audition is cued or sounds.
    pub tuning: Option<Tuning>,
}

/// One block's events into the tracker, on the display clock.
struct Out<'a> {
    tracker: &'a mut NoteTracker,
}

impl Out<'_> {
    fn event(&mut self, key: VoiceKey, time: f64, kind: NoteEventKind) {
        self.tracker.handle_event(NoteEvent {
            source: key.source.on_tape(),
            time,
            channel: key.channel,
            note: key.note,
            kind,
        });
    }

    /// Move a voice the tracker holds sounding as `was` to sound as `s`:
    /// only what changed, since an expression arrives every block.
    fn sound(&mut self, key: VoiceKey, time: f64, was: &Sounding, s: &Sounding) {
        if s.pitch != was.pitch {
            let semitones = s.pitch - f32::from(key.note);
            self.event(key, time, NoteEventKind::Tuning { semitones });
        }
        let (old, new) = (was.expressions, s.expressions);
        for (expression, was, value) in [
            (Expression::Pressure, old.pressure, new.pressure),
            (Expression::Gain, old.gain, new.gain),
            (Expression::Timbre, old.timbre, new.timbre),
        ] {
            if value != was {
                self.event(key, time, NoteEventKind::Expression { expression, value });
            }
        }
    }

    fn on(&mut self, key: VoiceKey, time: f64, s: &Sounding) {
        self.event(key, time, NoteEventKind::On { velocity: s.velocity });
        // A struck voice sounds at its key, with no expression yet.
        let struck =
            Sounding { pitch: f32::from(key.note), expressions: Expressions::NEUTRAL, ..*s };
        self.sound(key, time, &struck, s);
    }

    fn off(&mut self, key: VoiceKey, time: f64) {
        self.event(key, time, NoteEventKind::Off);
    }
}

/// Two song positions this close are one audition continuing: a block's
/// rounding, never a new cue.
const CONTINUES_FRAMES: i64 = 2;

impl Player {
    /// Release everything the tape holds, at display time `time`.
    fn release(&mut self, out: &mut Out, time: f64) {
        for key in std::mem::take(&mut self.sounding).into_keys() {
            out.off(key, time);
        }
        self.owner = None;
    }

    /// Make the tracker hold `target` from display time `time`: a voice that
    /// sounds the same on both sides carries on, anything else is let go or
    /// struck.
    fn become_(&mut self, out: &mut Out, target: &BTreeMap<VoiceKey, Sounding>, time: f64) {
        let gone: Vec<VoiceKey> = self
            .sounding
            .iter()
            .filter(|(key, s)| target.get(key) != Some(s))
            .map(|(key, _)| *key)
            .collect();
        for key in gone {
            self.sounding.remove(&key);
            out.off(key, time);
        }
        for (key, s) in target {
            if !self.sounding.contains_key(key) {
                self.sounding.insert(*key, *s);
                out.on(*key, time, s);
            }
        }
    }

    /// A block in which no audition was cued or sounding, at display time
    /// `time`: the audition is over.
    pub(crate) fn silent(&mut self, tracker: &mut NoteTracker, time: f64, rolling: bool) {
        let mut out = Out { tracker };
        self.release(&mut out, time);
        if self.cursor.take().is_some() {
            if let Some(audition) = &mut self.audition {
                audition.until = Some(time);
            }
        }
        self.tuning = None;
        if rolling {
            self.audition = None;
        }
    }

    /// A block whose first `frames` frames carried tape from song seconds
    /// `seconds`, its first frame at display time `clock`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn heard(
        &mut self,
        tracker: &mut NoteTracker,
        store: &Store,
        passes: &BTreeMap<u64, PassNotes>,
        seconds: f64,
        frames: usize,
        clock: f64,
    ) {
        let rate = f64::from(store.rate);
        let first = (seconds * rate).round() as i64;
        let end = first + frames as i64;
        let time = |frame: i64| clock + (frame - first) as f64 / rate;
        let mut out = Out { tracker };
        let continues = self.cursor.is_some_and(|c| (c - first).abs() <= CONTINUES_FRAMES);
        if !continues {
            // A new cue: what the last one held stops, and the spot's own
            // notes are struck, so a chord held across the cue is struck again
            // as it would be by playing from there.
            self.release(&mut out, clock);
            self.audition = Some(Audition { clock, song: seconds, until: None });
        }
        let mut frame = first;
        while frame < end {
            let Some(chunk) = store.holding(frame) else {
                self.release(&mut out, time(frame));
                frame = store.next_start(frame).map_or(end, |next| next.min(end));
                continue;
            };
            let pass = chunk.stamp.pass;
            let until = chunk.end.min(end);
            let Some(notes) = passes.get(&pass) else {
                self.release(&mut out, time(frame));
                frame = until;
                continue;
            };
            if self.owner.as_ref().is_none_or(|(p, _)| *p != pass) {
                let replay = Replay::at(notes, frame);
                self.become_(&mut out, &replay.held, time(frame));
                self.owner = Some((pass, replay));
            }
            let (_, replay) = self.owner.as_mut().expect("an owner");
            let sounding = &mut self.sounding;
            replay.advance(notes, until, |at, mark, _| {
                let t = time(at);
                match *mark {
                    Mark::On { key, sounding: s } => {
                        if sounding.insert(key, s).is_some() {
                            out.off(key, t);
                        }
                        out.on(key, t, &s);
                    }
                    Mark::Change { key, sounding: s } => {
                        if let Some(held) = sounding.get_mut(&key) {
                            out.sound(key, t, held, &s);
                            *held = s;
                        }
                    }
                    Mark::Off { key } => {
                        if sounding.remove(&key).is_some() {
                            out.off(key, t);
                        }
                    }
                }
            });
            self.tuning = notes.tuning_at(until - 1);
            frame = until;
        }
        self.cursor = Some(end);
    }
}

/// Every pass's notes, and the live voices a pass that starts now opens with.
#[derive(Default)]
pub(crate) struct Book {
    pub passes: BTreeMap<u64, PassNotes>,
    /// The live voices after every mark settled so far, rolling or not.
    pub held: BTreeMap<VoiceKey, Sounding>,
}

impl Book {
    /// Open pass `pass` at song frame `frame` with the voices held now.
    pub(crate) fn open(&mut self, pass: u64, frame: i64, tuning: Option<Tuning>) {
        let notes = self.passes.entry(pass).or_default();
        for (key, sounding) in &self.held {
            notes.place(frame, Mark::On { key: *key, sounding: *sounding });
        }
        if let Some(tuning) = tuning {
            notes.tunings.push((frame, tuning));
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.passes.values().map(PassNotes::bytes).sum()
    }
}

#[cfg(test)]
pub(crate) fn key(source: u64, note: u8) -> VoiceKey {
    VoiceKey { source: harmonigraph_core::SourceId(source), channel: 0, note }
}

#[cfg(test)]
mod tests {
    use super::super::store::{Chunk, Stamp};
    use super::*;

    const RATE: f32 = 1000.0;

    fn sounding(pitch: f32) -> Sounding {
        Sounding { velocity: 1.0, pitch, expressions: Expressions::NEUTRAL }
    }

    fn on(note: u8) -> Mark {
        Mark::On { key: key(1, note), sounding: sounding(f32::from(note)) }
    }

    fn off(note: u8) -> Mark {
        Mark::Off { key: key(1, note) }
    }

    /// Tuning axes told apart by their origin.
    fn tuning(origin: i32) -> Tuning {
        Tuning { c_offset: origin, ..Tuning::default() }
    }

    fn chunk(start: i64, end: i64, pass: u64) -> Chunk {
        let stamp = Stamp { pass, when: std::time::Instant::now(), pass_start: 0 };
        Chunk { start, end, stamp, data_start: start, samples: vec![0.0; (end - start) as usize] }
    }

    /// Pass 1 played 0..3 s: a held 60 across the whole of it and a short
    /// 64. Pass 2, from the song's start too and so the winner, replaced
    /// 1..1.5 s with a 72 of its own. Pass 1's chunk is cut in two, and its
    /// 60 is held across the cut.
    fn two_passes() -> (Store, BTreeMap<u64, PassNotes>) {
        let mut store = Store::new(RATE, 1);
        store.insert(chunk(0, 3000, 1));
        store.insert(chunk(1000, 1500, 2));
        let mut first = PassNotes::default();
        for (frame, mark) in [(500, on(60)), (1200, on(64)), (1400, off(64)), (2500, off(60))] {
            first.place(frame, mark);
        }
        first.tunings.push((0, tuning(1)));
        let mut second = PassNotes::default();
        second.place(1100, on(72));
        second.place(1300, off(72));
        second.tunings.push((1000, tuning(2)));
        (store, BTreeMap::from([(1, first), (2, second)]))
    }

    /// What the tape holds in the tracker: note and onset, by note.
    fn tape_voices(tracker: &NoteTracker) -> Vec<(u8, f64)> {
        let mut voices: Vec<(u8, f64)> = tracker
            .held_voices()
            .filter(|v| v.source.is_tape())
            .map(|v| (v.note, (v.on_time * 1e6).round() / 1e6))
            .collect();
        voices.sort_by_key(|a| a.0);
        voices
    }

    #[test]
    fn the_recorder_marks_what_the_tracker_did_to_each_voice() {
        let mut tracker = NoteTracker::new();
        let mut recorder = Recorder::default();
        let mut marks = Vec::new();
        let source = harmonigraph_core::SourceId(1);
        let events = [
            NoteEvent::on(1.0, source, 0, 60, 1.0),
            NoteEvent {
                kind: NoteEventKind::Tuning { semitones: 0.25 },
                ..NoteEvent::off(1.01, source, 0, 60)
            },
            // Struck again with no release between: two presses.
            NoteEvent::on(1.5, source, 0, 60, 1.0),
            NoteEvent::off(2.0, source, 0, 60),
            // The tape's own playback is never recorded.
            NoteEvent::on(2.5, source.on_tape(), 0, 62, 1.0),
        ];
        for event in events {
            tracker.handle_event(event);
            recorder.record(&tracker, event.time, &mut marks);
        }
        let k = key(1, 60);
        assert_eq!(
            marks,
            [
                (1.0, Mark::On { key: k, sounding: sounding(60.0) }),
                (1.01, Mark::Change { key: k, sounding: sounding(60.25) }),
                (1.5, Mark::Off { key: k }),
                (1.5, Mark::On { key: k, sounding: sounding(60.0) }),
                (2.0, Mark::Off { key: k }),
            ]
        );
    }

    /// A step already holding [`NOTES`] marks has no room for a release. It
    /// was lost for good, and the tape held the note to the end of its pass;
    /// the next event with room records it, late.
    #[test]
    fn a_mark_the_bound_held_back_is_recorded_at_the_next_event() {
        let mut tracker = NoteTracker::new();
        let mut recorder = Recorder::default();
        let mut marks = Vec::new();
        let source = harmonigraph_core::SourceId(1);
        for (time, note) in [(1.0, 60), (1.1, 62)] {
            tracker.handle_event(NoteEvent::on(time, source, 0, note, 1.0));
            recorder.record(&tracker, time, &mut marks);
        }
        let full = (0.0, Mark::Off { key: key(1, 0) });
        marks = vec![full; NOTES];
        tracker.handle_event(NoteEvent::off(2.0, source, 0, 60));
        recorder.record(&tracker, 2.0, &mut marks);
        assert_eq!(marks.len(), NOTES, "the full step took the release");

        // The next step's drain starts empty.
        marks.clear();
        tracker.handle_event(NoteEvent::off(3.0, source, 0, 62));
        recorder.record(&tracker, 3.0, &mut marks);
        assert_eq!(
            marks,
            [(3.0, Mark::Off { key: key(1, 60) }), (3.0, Mark::Off { key: key(1, 62) })]
        );
    }

    /// The defect that kept notes out of #1720: a chunk cut by a newer pass
    /// lost the notes held across the cut. Cued after it, the 60 struck
    /// before the cut sounds; cued before the newer pass's stretch, its 72
    /// sounds and ends, and the 60 is struck again where the older pass's
    /// audio takes over, which is what an audition plays there.
    #[test]
    fn a_cue_chases_the_notes_of_the_pass_whose_audio_won_there() {
        let (store, passes) = two_passes();
        let mut tracker = NoteTracker::new();
        let mut player = Player::default();

        player.heard(&mut tracker, &store, &passes, 2.0, 100, 50.0);
        assert_eq!(tape_voices(&tracker), [(60, 50.0)]);
        assert_eq!(player.tuning, Some(tuning(1)));

        // A new cue at 1.2 s: the old voices stop, pass 2's 72 is held there
        // and ends at 1.3 s, and from 1.5 s pass 1's 60 sounds.
        player.heard(&mut tracker, &store, &passes, 1.2, 200, 60.0);
        assert_eq!(tape_voices(&tracker), [], "the 72 ended and the seam is not reached");
        assert_eq!(player.tuning, Some(tuning(2)));
        player.heard(&mut tracker, &store, &passes, 1.4, 200, 60.2);
        assert_eq!(tape_voices(&tracker), [(60, 60.3)]);
        assert_eq!(player.audition, Some(Audition { clock: 60.0, song: 1.2, until: None }));

        // A block with no tape ends it.
        player.silent(&mut tracker, 61.0, false);
        assert_eq!(tape_voices(&tracker), []);
        assert_eq!(player.tuning, None);
        assert_eq!(player.audition.and_then(|a| a.until), Some(61.0));
    }

    /// A chase read from a checkpoint is the chase read from the start, and
    /// still is after a mark lands among the checkpoints.
    #[test]
    fn a_checkpointed_chase_matches_one_replayed_from_the_start() {
        let mut notes = PassNotes::default();
        for i in 0..3 * CHECKPOINT as i64 {
            let note = 40 + (i % 32) as u8;
            notes.place(i * 10, if i % 64 < 32 { on(note) } else { off(note) });
        }
        // Late, and early in the pass: a release before most checkpoints.
        notes.place(15, off(40));
        let whole = |frame: i64| {
            let mut held = BTreeMap::new();
            for (_, mark) in notes.events.iter().filter(|(f, _)| *f < frame) {
                mark.apply(&mut held);
            }
            held
        };
        // Every checkpoint after the late mark was taken again, so a chase
        // into the middle of the pass reads from one.
        assert_eq!(notes.checkpoints.len(), notes.events.len() / CHECKPOINT);
        for frame in [0, 16, 10_235, 10_250, 15_000, 20_480, 25_000, 30_000, i64::MAX] {
            assert_eq!(Replay::at(&notes, frame).held, whole(frame), "frame {frame}");
        }
    }

    /// Tape mode's roll is what an audition would play: each stretch's notes
    /// from the pass that won it, opening with what that pass held there.
    #[test]
    fn the_song_roll_draws_each_stretch_from_the_pass_that_won_it() {
        let (store, passes) = two_passes();
        let roll = song_roll(&store, &passes);
        let mut notes: Vec<(u8, f64, f64)> =
            roll.notes().map(|n| (n.note, n.start, n.end.expect("ended"))).collect();
        notes.sort_by(|a, b| a.1.total_cmp(&b.1));
        assert_eq!(notes, [(60, 0.5, 1.0), (72, 1.1, 1.3), (60, 1.5, 2.5)]);
    }
}
