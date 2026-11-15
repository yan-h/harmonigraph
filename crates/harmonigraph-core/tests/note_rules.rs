//! The rules one press follows from voice to roll, checked after every step of
//! random note sequences driven through the public API (#1759).
//!
//! A press is drawn twice: as a voice, on the lattice, the Spiral and the note
//! band, and as a roll note, on the Roll's timeline. Six cases set the two
//! apart, each checked where its operation runs:
//!
//! 1. an off at or before its onset is a voice that pulses and no roll note;
//! 2. a retrigger without an off ends the old note with no fade, and drops it
//!    when the retrigger lands at or before its onset;
//! 3. a gap releases what it lost into a fade, and a baseline listing the note
//!    takes it back, from the fade or from the past;
//! 4. a note hidden at its release is never drawn leaving nor remembered;
//! 5. a held note a baseline omits leaves with no fade and no trail mark;
//! 6. a baseline listing a note that has ended withdraws its fade and opens a
//!    fresh note.
//!
//! And after every step, one note is one record: each note on the roll that is
//! still sounding is a held voice at the same onset and pitch, and each held
//! voice its source shows, unless Clear took it off the roll, is on the roll.
//!
//! Every case is a hit counter, and the run fails when one is reached fewer than
//! [`AT_LEAST`] times over the seeds, so a sequence space too small to reach a
//! rule cannot pass as covering it.

use std::collections::{BTreeMap, BTreeSet};

use harmonigraph_core::canonical::PitchProvenance;
use harmonigraph_core::canonical::{
    CanonicalEvent, GapReason, NoteDelta, PublicationGap, SourceBaseline, VoiceBaseline,
};
use harmonigraph_core::{
    Envelope, Expression, Expressions, NoteEvent, NoteEventKind, NoteTracker, RollNote, SourceId,
    Time, Voice, VoiceKey, VoiceState,
};

const SEEDS: u64 = 96;
const STEPS: usize = 300;
const AT_LEAST: usize = 20;

/// Two published sources, the first also played back by the tape, and the
/// direct input. Every time below is a multiple of a power of two, so a time
/// mapped by an offset and back lands exactly where it started.
const A: SourceId = SourceId(1);
const B: SourceId = SourceId(2);
const DIRECT: SourceId = SourceId::DIRECT;
const OFFSETS: [Time; 2] = [0.0, 0.25];
const NUDGE: Time = 1.0 / 128.0;
const ENV: Envelope = Envelope { attack_time: 0.0625, fade_time: 1.0, shape: 0.0 };

fn tape() -> SourceId {
    A.on_tape()
}

/// A small xorshift generator: the crate takes no dependencies.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    fn pick<T: Copy>(&mut self, from: &[T]) -> T {
        from[self.below(from.len() as u64) as usize]
    }
}

/// A note a published source attacked or a baseline listed, by lifetime.
#[derive(Clone, Copy)]
struct Played {
    key: VoiceKey,
    lifetime: u64,
    /// Its onset before the clock offset, as a baseline states it.
    onset: Time,
    velocity: f32,
    /// Where it last started on the roll.
    start: Time,
}

/// What a step can see, before and after it.
#[derive(Clone)]
struct Snap {
    held: Vec<Voice>,
    released: Vec<Voice>,
    /// The held voices `voices()` draws.
    drawn: Vec<(VoiceKey, Time)>,
    roll: Vec<RollNote>,
    trail: BTreeMap<i32, Time>,
}

type Row = (VoiceKey, Time, Option<Time>, Option<Time>, Option<u64>);

fn as_row(note: &RollNote) -> Row {
    (note.key(), note.start, note.end, note.observed_until, note.lifetime)
}

fn rows(notes: &[RollNote]) -> Vec<Row> {
    let mut rows: Vec<_> = notes.iter().map(as_row).collect();
    rows.sort_by(|a, b| a.partial_cmp(b).unwrap());
    rows
}

fn tail(voices: &[Voice]) -> Vec<(VoiceKey, Time, Time, Option<u64>)> {
    let mut tail: Vec<_> = voices
        .iter()
        .map(|v| match v.state {
            VoiceState::Released { at } => (v.key(), v.on_time, at, v.lifetime),
            VoiceState::Held => panic!("a held voice in the tail"),
        })
        .collect();
    tail.sort_by(|a, b| a.partial_cmp(b).unwrap());
    tail
}

fn identity(voices: &[Voice]) -> Vec<(VoiceKey, Time, Option<u64>)> {
    voices.iter().map(|v| (v.key(), v.on_time, v.lifetime)).collect()
}

fn snap(tracker: &NoteTracker) -> Snap {
    let voices: Vec<Voice> = tracker.voices().collect();
    Snap {
        held: tracker.held_voices().collect(),
        released: voices.iter().filter(|v| v.state != VoiceState::Held).copied().collect(),
        drawn: voices
            .iter()
            .filter(|v| v.state == VoiceState::Held)
            .map(|v| (v.key(), v.on_time))
            .collect(),
        roll: tracker.roll().notes().cloned().collect(),
        trail: tracker
            .history()
            .visits()
            .map(|v| ((v.pitch * 100.0).round() as i32, v.last_off))
            .collect(),
    }
}

impl Snap {
    fn held(&self, key: VoiceKey) -> Option<Voice> {
        self.held.iter().find(|v| v.key() == key).copied()
    }

    fn sounding(&self, key: VoiceKey, start: Time) -> Option<&RollNote> {
        self.roll.iter().find(|n| n.key() == key && n.start == start && n.is_live())
    }
}

struct Run<'a> {
    tracker: NoteTracker,
    rng: Rng,
    clock: Time,
    offset: Time,
    sequence: BTreeMap<SourceId, u64>,
    baselines: BTreeMap<SourceId, u64>,
    lifetimes: u64,
    played: Vec<Played>,
    /// Notes Clear took off the roll while they sounded or faded, by key and
    /// onset.
    cleared: BTreeSet<(VoiceKey, u64)>,
    hits: &'a mut BTreeMap<&'static str, usize>,
}

impl Run<'_> {
    fn hit(&mut self, case: &'static str) {
        *self.hits.entry(case).or_default() += 1;
    }

    fn key(&mut self, source: SourceId) -> VoiceKey {
        VoiceKey { source, channel: 0, note: self.rng.pick(&[60, 64]) }
    }

    fn velocity(&mut self) -> f32 {
        self.rng.pick(&[0.25, 0.5, 1.0])
    }

    fn is_cleared(&self, key: VoiceKey, start: Time) -> bool {
        self.cleared.contains(&(key, start.to_bits()))
    }

    fn step(&mut self) {
        let before = snap(&self.tracker);
        match self.rng.below(20) {
            0..=2 => self.on(&before),
            3..=5 => self.off(&before),
            6..=7 => self.published_on(&before),
            8..=9 => self.published_off(&before),
            10 => self.bend(&before),
            11 => self.gap(&before),
            12 => self.reset(&before),
            13 => self.clear(&before),
            14..=16 => self.advance(&before),
            _ => self.baseline(&before),
        }
        self.one_record();
    }

    /// Each note sounding on the roll is a held voice at the same onset, pitch
    /// and expressions, and each held voice its source shows is on the roll
    /// unless Clear took it off.
    fn one_record(&self) {
        let now = snap(&self.tracker);
        for note in now.roll.iter().filter(|n| n.is_live()) {
            let voice = now.held(note.key()).expect("a sounding roll note is a held voice");
            assert_eq!(voice.on_time, note.start, "one onset for {:?}", note.key());
            assert_eq!(voice.pitch, note.end_pitch(), "one pitch for {:?}", note.key());
            let expressions = note.expressions().last().unwrap().1;
            assert_eq!(voice.expressions, expressions, "one expression for {:?}", note.key());
        }
        for voice in &now.held {
            if !self.tracker.hides(voice.source) && !self.is_cleared(voice.key(), voice.on_time) {
                assert!(
                    now.sounding(voice.key(), voice.on_time).is_some(),
                    "{voice:?} off the roll"
                );
            }
        }
    }

    /// The direct input's or the tape's on.
    fn on(&mut self, before: &Snap) {
        let source = self.rng.pick(&[DIRECT, tape()]);
        let key = self.key(source);
        let velocity = self.velocity();
        self.tracker.handle_event(NoteEvent::on(self.clock, source, 0, key.note, velocity));
        self.after_on(before, key, self.clock);
    }

    fn after_on(&mut self, before: &Snap, key: VoiceKey, at: Time) {
        self.cleared.remove(&(key, at.to_bits()));
        let after = snap(&self.tracker);
        let held = after.held(key).expect("an on holds its key");
        assert_eq!((held.on_time, held.state), (at, VoiceState::Held));
        assert_eq!(tail(&after.released), tail(&before.released), "an on lets nothing go");
        assert_eq!(after.trail, before.trail);
        let mut expected = rows(&before.roll);
        if let Some(prev) = before.held(key) {
            // Rule 2: the old note ends where the new one starts, with no fade.
            self.hit("rule 2: a retrigger ends its note");
            if let Some(index) =
                row_live(before, key).and_then(|live| expected.iter().position(|r| *r == live))
            {
                let mut ended = expected.remove(index);
                if at > prev.on_time {
                    ended.2 = Some(at);
                    expected.push(ended);
                } else {
                    self.hit("rule 2: a retrigger at its onset drops it");
                }
            }
        }
        if !self.tracker.hides(key.source) {
            let lifetime = held.lifetime;
            expected.push((key, at, None, None, lifetime));
        }
        expected.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(rows(&after.roll), expected, "on {key:?} at {at}");
    }

    /// The direct input's or the tape's off, before, at or after the onset.
    fn off(&mut self, before: &Snap) {
        let source = self.rng.pick(&[DIRECT, tape()]);
        let key = self.key(source);
        let onset = before.held(key).map_or(self.clock, |v| v.on_time);
        let at = self.rng.pick(&[onset - NUDGE, onset, self.clock, self.clock]);
        self.tracker.handle_event(NoteEvent::off(at, source, 0, key.note));
        self.after_off(before, key, at);
    }

    fn after_off(&mut self, before: &Snap, key: VoiceKey, at: Time) {
        let after = snap(&self.tracker);
        let Some(prev) = before.held(key) else {
            assert_eq!(rows(&after.roll), rows(&before.roll), "an off with nothing held");
            assert_eq!(identity(&after.held), identity(&before.held));
            return;
        };
        assert!(after.held(key).is_none(), "an off lets its key go");
        let mut tail_expected = tail(&before.released);
        let pulse = at <= prev.on_time;
        if self.tracker.hides(key.source) {
            // Rule 4: not drawn leaving, and so never remembered.
            self.hit("rule 4: hidden at its release");
            if pulse {
                self.hit("rule 4: a hidden pulse is dropped");
            }
        } else {
            if pulse {
                self.hit("rule 1: an off at its onset pulses");
            }
            tail_expected.push((key, prev.on_time, at, prev.lifetime));
            tail_expected.sort_by(|a, b| a.partial_cmp(b).unwrap());
        }
        assert_eq!(tail(&after.released), tail_expected, "off {key:?} at {at}");
        let mut expected = rows(&before.roll);
        if let Some(index) =
            row_live(before, key).and_then(|live| expected.iter().position(|r| *r == live))
        {
            let mut ended = expected.remove(index);
            // Rule 1: a note that never sounded is no roll note.
            if !pulse {
                ended.2 = Some(at);
                expected.push(ended);
            }
        }
        expected.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(rows(&after.roll), expected, "off {key:?} at {at}");
    }

    /// One published delta, mapped onto the clock by the current offset.
    fn publish(&mut self, event: NoteEvent, lifetime: u64, pitch: Option<i64>) {
        let sequence = self.sequence.entry(event.source).or_default();
        *sequence += 1;
        let delta = NoteDelta {
            event: NoteEvent { time: event.time - self.offset, ..event },
            sequence: *sequence,
            lifetime,
            provenance: PitchProvenance::AcceptedOutput,
            pitch_microcents: pitch,
            ..NoteDelta::from(event)
        };
        let accepted =
            self.tracker.handle_canonical_mapped(CanonicalEvent::Note(delta), self.offset);
        assert_eq!(accepted, Ok(true));
    }

    fn published_on(&mut self, before: &Snap) {
        let source = self.rng.pick(&[A, B]);
        let key = self.key(source);
        self.lifetimes += 1;
        let lifetime = self.lifetimes;
        let velocity = self.velocity();
        let pitch = self.rng.chance(50).then(|| i64::from(key.note) * 100_000_000 + 30_000_000);
        let on = NoteEvent::on(self.clock, source, 0, key.note, velocity);
        self.publish(on, lifetime, pitch);
        let onset = self.clock - self.offset;
        let start = self.clock;
        self.played.push(Played { key, lifetime, onset, velocity, start });
        self.after_on(before, key, self.clock);
    }

    /// An off for the held note, before, at or after its onset, or for a note
    /// a gap let go of: a factual off recovered after the gap.
    fn published_off(&mut self, before: &Snap) {
        let source = self.rng.pick(&[A, B]);
        let key = self.key(source);
        let held = before.held(key);
        let stale = self.rng.chance(40);
        let lifetime = match (held, stale) {
            (Some(voice), false) => voice.lifetime.unwrap(),
            _ => {
                let mine: Vec<u64> =
                    self.played.iter().filter(|p| p.key == key).map(|p| p.lifetime).collect();
                if mine.is_empty() {
                    return;
                }
                self.rng.pick(&mine)
            }
        };
        let onset = held.map_or(self.clock, |v| v.on_time);
        let at = self.rng.pick(&[onset - NUDGE, onset, self.clock, self.clock]);
        self.publish(NoteEvent::off(at, source, 0, key.note), lifetime, None);
        if held.is_some_and(|v| v.lifetime == Some(lifetime)) {
            return self.after_off(before, key, at);
        }
        // Not the held note's: only a note a gap closed hears it, and only on
        // the roll. Its fade keeps the time the gap gave it.
        let after = snap(&self.tracker);
        assert_eq!(identity(&after.held), identity(&before.held));
        assert_eq!(
            tail(&after.released),
            tail(&before.released),
            "a recovered off restarts no fade"
        );
        let mut expected = rows(&before.roll);
        let recovered =
            |r: &Row| r.0 == key && r.4 == Some(lifetime) && r.2.is_none() && r.3.is_some();
        if let Some(index) = expected.iter().rposition(recovered) {
            self.hit("rule 3: a factual off recovered after a gap");
            expected[index].2 = Some(at.max(expected[index].1));
            expected.sort_by(|a, b| a.partial_cmp(b).unwrap());
        }
        assert_eq!(rows(&after.roll), expected, "a recovered off for {lifetime}");
    }

    /// A tuning or an expression on a held note.
    fn bend(&mut self, before: &Snap) {
        let Some(voice) = before.held.get(self.rng.below(before.held.len().max(1) as u64) as usize)
        else {
            return;
        };
        let semitones = self.rng.pick(&[-0.5, 0.25, 1.0]);
        let kind = if self.rng.chance(50) {
            NoteEventKind::Tuning { semitones }
        } else {
            NoteEventKind::Expression { expression: Expression::Pressure, value: semitones.abs() }
        };
        let event = NoteEvent {
            time: self.clock,
            source: voice.source,
            channel: voice.channel,
            note: voice.note,
            kind,
        };
        match voice.lifetime {
            Some(lifetime) => self.publish(event, lifetime, None),
            None => self.tracker.handle_event(event),
        }
        let after = snap(&self.tracker);
        assert_eq!(identity(&after.held), identity(&before.held));
        assert_eq!(rows(&after.roll), rows(&before.roll));
    }

    /// A gap over one published source or the whole stream: it releases what
    /// it lost into a fade (rule 3), but not what the tape plays.
    fn gap(&mut self, before: &Snap) {
        let scope = self.rng.pick(&[Some(A), Some(B), None]);
        let at = self.clock;
        let gap = PublicationGap {
            source: scope,
            time: at - self.offset,
            through: at - self.offset,
            first: 1,
            last: 1,
            reason: GapReason::PublicationFull,
        };
        let accepted = self.tracker.handle_canonical_mapped(CanonicalEvent::Gap(gap), self.offset);
        assert_eq!(accepted, Ok(true));
        let lost = |key: VoiceKey| !key.source.is_tape() && scope.is_none_or(|s| key.source == s);
        self.after_release(before, at, lost, "a gap");
    }

    /// A session reset, which leaves the tape's notes to its player, or one
    /// source's reset.
    fn reset(&mut self, before: &Snap) {
        let at = self.clock;
        if self.rng.chance(50) {
            self.tracker.handle_event(NoteEvent::session_reset(at));
            self.after_release(before, at, |key| !key.source.is_tape(), "a session reset");
        } else {
            let source = self.rng.pick(&[DIRECT, tape(), A, B]);
            self.tracker.handle_event(NoteEvent::source_reset(at, source));
            self.after_release(before, at, |key| key.source == source, "a source reset");
        }
    }

    fn after_release(
        &mut self,
        before: &Snap,
        at: Time,
        leaving: impl Fn(VoiceKey) -> bool,
        why: &'static str,
    ) {
        let after = snap(&self.tracker);
        let gap = why == "a gap";
        let mut tail_expected = tail(&before.released);
        let mut expected = rows(&before.roll);
        for voice in &before.held {
            let key = voice.key();
            if !leaving(key) {
                assert!(after.held(key).is_some(), "{why} kept {key:?}");
                if key.source.is_tape() && why == "a session reset" {
                    self.hit("a tape note held through a session reset");
                }
                if key.source.is_tape() && gap {
                    self.hit("a tape note held through a gap");
                }
                continue;
            }
            assert!(after.held(key).is_none(), "{why} let {key:?} go");
            if self.tracker.hides(key.source) {
                self.hit("rule 4: hidden at its release");
            } else {
                if gap {
                    self.hit("rule 3: a gap releases into a fade");
                }
                tail_expected.push((key, voice.on_time, at, voice.lifetime));
            }
            if let Some(index) =
                row_live(before, key).and_then(|live| expected.iter().position(|r| *r == live))
            {
                let closed = at.max(voice.on_time);
                if gap {
                    expected[index].3 = Some(closed);
                } else {
                    expected[index].2 = Some(closed);
                }
            }
        }
        tail_expected.sort_by(|a, b| a.partial_cmp(b).unwrap());
        expected.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(tail(&after.released), tail_expected, "{why} at {at}");
        assert_eq!(rows(&after.roll), expected, "{why} at {at}");
        assert_eq!(after.trail, before.trail);
    }

    fn clear(&mut self, before: &Snap) {
        self.tracker.clear_roll();
        for voice in before.held.iter().chain(&before.released) {
            self.cleared.insert((voice.key(), voice.on_time.to_bits()));
        }
        let after = snap(&self.tracker);
        assert!(after.roll.is_empty(), "Clear empties the roll");
        assert_eq!(identity(&after.held), identity(&before.held), "and leaves the voices");
        assert_eq!(tail(&after.released), tail(&before.released));
    }

    /// Time passes, the offset may move, and a prune ends each fade that is
    /// over in a trail mark, leaving the roll as it was.
    fn advance(&mut self, before: &Snap) {
        self.clock += self.rng.pick(&[0.0, 0.25, 1.5]);
        if self.rng.chance(25) {
            self.offset = self.rng.pick(&OFFSETS);
        }
        self.tracker.prune(self.clock, &ENV);
        let after = snap(&self.tracker);
        let (over, left): (Vec<Voice>, Vec<Voice>) =
            before.released.iter().partition(|v| v.release_level(self.clock, &ENV) <= 0.0);
        assert_eq!(tail(&after.released), tail(&left), "a prune ends exactly the finished fades");
        let mut trail = before.trail.clone();
        for voice in &over {
            self.hit("a finished fade lands in the trail");
            let VoiceState::Released { at } = voice.state else { unreachable!() };
            let last = trail.entry((voice.pitch * 100.0).round() as i32).or_insert(at);
            *last = last.max(at);
        }
        assert_eq!(after.trail, trail, "the trail holds what finished fading");
        assert_eq!(rows(&after.roll), rows(&before.roll), "a prune moves nothing on the roll");
        assert_eq!(identity(&after.held), identity(&before.held));
    }

    /// One published source's complete state: rows drawn from its held,
    /// fading, finished and cleared notes and a note never attacked, each at
    /// its attack velocity, with the source shown or hidden.
    fn baseline(&mut self, before: &Snap) {
        let source = self.rng.pick(&[A, B]);
        let shown = self.rng.chance(80);
        let at = self.clock;
        let time = at - self.offset;
        if self.rng.chance(25) {
            self.lifetimes += 1;
            let key = self.key(source);
            let onset = time - self.rng.pick(&[0.0, 0.25]);
            let velocity = self.velocity();
            let lifetime = self.lifetimes;
            let start = onset + self.offset;
            self.played.push(Played { key, lifetime, onset, velocity, start });
        }
        let mut listed: Vec<Played> = Vec::new();
        for p in self.played.clone().iter().rev().filter(|p| p.key.source == source) {
            if p.onset <= time && self.rng.chance(50) && listed.iter().all(|l| l.key != p.key) {
                listed.push(*p);
            }
        }
        let rows_in: Vec<VoiceBaseline> = listed
            .iter()
            .map(|p| VoiceBaseline {
                note: p.key.note,
                lifetime: p.lifetime,
                actual_onset: p.onset,
                pitch_microcents: i64::from(p.key.note) * 100_000_000
                    + self.rng.pick(&[0, 30_000_000]),
                velocity: p.velocity,
                expressions: Expressions {
                    pressure: self.rng.pick(&[0.0, 0.5]),
                    ..Expressions::NEUTRAL
                },
                provenance: PitchProvenance::AcceptedOutput,
                ..VoiceBaseline::default()
            })
            .collect();
        let id = self.baselines.entry(source).or_default();
        *id += 1;
        let cut = self.sequence.get(&source).copied().unwrap_or(0);
        let frame = SourceBaseline::new(source, *id, time, cut, shown, &rows_in).unwrap();
        let was_shown = !self.tracker.hides(source);
        let accepted =
            self.tracker.handle_canonical_mapped(CanonicalEvent::Baseline(&frame), self.offset);
        assert_eq!(accepted, Ok(true));
        self.after_baseline(before, source, (was_shown, shown), at, &rows_in);
    }

    fn after_baseline(
        &mut self,
        before: &Snap,
        source: SourceId,
        (was_shown, shown): (bool, bool),
        at: Time,
        listed: &[VoiceBaseline],
    ) {
        let after = snap(&self.tracker);
        let mine = |v: &&Voice| v.source == source;
        let theirs = |v: &&Voice| v.source != source;
        let other = |s: &Snap| identity(&s.held.iter().filter(theirs).copied().collect::<Vec<_>>());
        assert_eq!(other(&after), other(before), "a baseline holds only its own source");
        let held: Vec<_> = after.held.iter().filter(mine).map(|v| v.lifetime.unwrap()).collect();
        let mut want: Vec<_> = listed.iter().map(|r| r.lifetime).collect();
        want.sort_unstable();
        let mut got = held.clone();
        got.sort_unstable();
        assert_eq!(got, want, "a baseline holds exactly what it lists");

        // Rule 5: an omitted held note leaves without a fade.
        for voice in before.held.iter().filter(mine) {
            if listed.iter().all(|r| Some(r.lifetime) != voice.lifetime) {
                self.hit("rule 5: an omitted held note leaves without a fade");
                let key = voice.key();
                let open = (key, voice.on_time, None, Some(at.max(voice.on_time)), voice.lifetime);
                if before.sounding(key, voice.on_time).is_some() && shown {
                    assert!(rows(&after.roll).contains(&open), "{key:?} observed until {at}");
                }
            }
        }
        let mut tail_expected = tail(&before.released);
        for row in listed {
            let key = row.key(source);
            let voice = after.held(key).unwrap();
            assert_eq!(voice.pitch, row.pitch());
            assert_eq!(voice.expressions, row.expressions);
            let fresh = row.actual_onset + self.offset;
            let was_held = before.held.iter().find(|v| v.lifetime == Some(row.lifetime));
            let was_released = before.released.iter().find(|v| v.lifetime == Some(row.lifetime));
            let played = self.played.iter().find(|p| p.lifetime == row.lifetime).copied().unwrap();
            // The note's row, the one a gap left open first: a fresh note a
            // baseline opened can share its onset with the one that ended.
            let on_roll = |start: Time| {
                let at = |n: &&RollNote| n.lifetime == Some(row.lifetime) && n.start == start;
                let open = |n: &&RollNote| n.end.is_none() && n.observed_until.is_some();
                let mut notes = before.roll.iter().filter(at);
                notes.clone().find(open).or_else(|| notes.next())
            };
            if let Some(held) = was_held {
                if self.is_cleared(key, held.on_time) {
                    self.hit("revival from held");
                    assert_eq!(voice.on_time, fresh, "a revived note starts afresh");
                } else {
                    assert_eq!(voice.on_time, held.on_time, "a listed note keeps its onset");
                }
            } else if let Some(gone) = was_released {
                let VoiceState::Released { at: off } = gone.state else { unreachable!() };
                tail_expected.retain(|t| *t != (key, gone.on_time, off, gone.lifetime));
                let cleared = self.is_cleared(key, gone.on_time);
                let note = on_roll(gone.on_time);
                if cleared {
                    self.hit("revival from the tail");
                    assert_eq!(voice.on_time, fresh, "a revived note starts afresh");
                } else if !was_shown {
                    // Its row was hidden, so which rule it takes cannot be read.
                } else if note.is_some_and(|n| n.end.is_none()) {
                    self.hit("rule 3: resumed from the tail");
                    assert_eq!(voice.on_time, gone.on_time, "a resumed note keeps its onset");
                } else if off <= gone.on_time && note.is_none() {
                    self.hit("rule 6: a listed pulse is dropped");
                    assert_eq!(voice.on_time, fresh, "a listed pulse opens a fresh note");
                } else {
                    self.hit("rule 6: a listed ended note opens a fresh one");
                    assert_eq!(voice.on_time, fresh, "a listed ended note opens a fresh one");
                    let stays = note.is_none_or(|n| rows(&after.roll).contains(&as_row(n)));
                    assert!(stays || !shown, "the ended note stays");
                }
            } else if self.is_cleared(key, played.start) {
                self.hit("revival from past");
                assert_eq!(voice.on_time, fresh, "a revived note starts afresh");
            } else if !was_shown {
                // As above.
            } else if let Some(note) = on_roll(played.start) {
                if note.end.is_none() && note.observed_until.is_some() {
                    self.hit("rule 3: resumed from past");
                    assert_eq!(voice.on_time, note.start, "a resumed note keeps its onset");
                } else {
                    assert_eq!(voice.on_time, fresh);
                }
            }
            self.cleared.remove(&(key, voice.on_time.to_bits()));
            if let Some(p) = self.played.iter_mut().find(|p| p.lifetime == row.lifetime) {
                p.start = voice.on_time;
            }
        }
        assert_eq!(tail(&after.released), tail_expected, "a baseline withdraws what it lists");
        assert_eq!(after.trail, before.trail);
        if !shown {
            assert!(after.drawn.iter().all(|(k, _)| k.source != source), "hidden voices drawn");
            let hidden = |n: &RollNote| n.source == source || n.source == source.on_tape();
            assert!(!after.roll.iter().any(hidden), "a hidden source's notes on the roll");
            if after.held.iter().any(|v| v.source == source.on_tape()) {
                self.hit("a tape note hidden with its source");
            }
        }
    }
}

/// The roll row a held key is drawn as, if it is on the roll.
fn row_live(before: &Snap, key: VoiceKey) -> Option<Row> {
    before.roll.iter().find(|n| n.key() == key && n.is_live()).map(as_row)
}

#[test]
fn every_press_follows_the_six_rules() {
    let mut hits = BTreeMap::new();
    for seed in 1..=SEEDS {
        let mut run = Run {
            tracker: NoteTracker::new(),
            rng: Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15)),
            clock: 1.0,
            offset: 0.0,
            sequence: BTreeMap::new(),
            baselines: BTreeMap::new(),
            lifetimes: 0,
            played: Vec::new(),
            cleared: BTreeSet::new(),
            hits: &mut hits,
        };
        for _ in 0..STEPS {
            run.step();
        }
    }
    let cases = [
        "rule 1: an off at its onset pulses",
        "rule 2: a retrigger ends its note",
        "rule 2: a retrigger at its onset drops it",
        "rule 3: a gap releases into a fade",
        "rule 3: resumed from the tail",
        "rule 3: resumed from past",
        "rule 3: a factual off recovered after a gap",
        "rule 4: hidden at its release",
        "rule 4: a hidden pulse is dropped",
        "rule 5: an omitted held note leaves without a fade",
        "rule 6: a listed ended note opens a fresh one",
        "rule 6: a listed pulse is dropped",
        "revival from held",
        "revival from the tail",
        "revival from past",
        "a tape note held through a gap",
        "a tape note held through a session reset",
        "a tape note hidden with its source",
        "a finished fade lands in the trail",
    ];
    for case in cases {
        let count = hits.get(case).copied().unwrap_or(0);
        assert!(count >= AT_LEAST, "{case}: reached {count} times\n{hits:#?}");
    }
}
