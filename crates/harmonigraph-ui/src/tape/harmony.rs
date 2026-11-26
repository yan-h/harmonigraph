//! The tape's harmony, section by section (#1974): where along the song the
//! sounding notes change enough to count as a new harmony, and what sounds in
//! each stretch between. Tape mode draws one small lattice per section.
//!
//! One rule finds the sections. At each attack, what sounded over the Span
//! before it is compared with what sounds over the Span after it, pitch class
//! by pitch class; a section starts at the attacks where that difference peaks
//! above the Detail threshold, no two nearer than a Span. A pitch class counts
//! as present while a note of it sounds and fades over half a Span after its
//! release, which is what holds an arpeggio or a cloud of short notes together
//! as one harmony: each note is still present when the next arrives.
//!
//! Everything here depends on the notes, the tempo, Detail and Span alone, so
//! the plugin's input worker computes it where it rebuilds the tape's roll,
//! off the editor's lock and off the main thread. What depends on the picture
//! (which node a pitch lights, how many thumbnails fit) is the pane's.

use std::collections::{BTreeMap, BTreeSet};

use harmonigraph_core::NoteRoll;

use super::Meter;

/// The presence grid's step is this share of the shortest Span, within
/// [`STEP_FLOOR`]..: fine enough that a window holds eight readings, and
/// bounded below so a half-hour tape at a fast tempo stays a few megabytes.
const STEPS_PER_SPAN: f64 = 8.0;
const STEP_FLOOR: f64 = 0.05;
/// Attacks this close together are one attack: a chord's notes, and the two
/// sources a doubled part arrives on.
const TOGETHER: f64 = 0.08;
/// A note struck this long before a section's start still counts as struck in
/// it: the same slack, so a chord whose first note opened the section is new
/// whole.
const STRUCK_SLACK: f64 = TOGETHER;
/// Below this much presence, summed in quadrature over the pitch classes, a
/// window is silence.
const SILENT: f32 = 0.05;
/// A beat's length with no tempo from the host.
const DEFAULT_BEAT_SECONDS: f64 = 0.5;
/// Pitches within a thousandth of a semitone are one pitch: the same note
/// from two sources. Not two tunings of it, which may light different nodes,
/// or one a node and the other none; the pane tells those apart.
const PITCH_BINS_PER_SEMITONE: f32 = 1000.0;
/// Pitch classes within a fiftieth of a semitone are one class to the rule.
const CLASS_BINS_PER_SEMITONE: f32 = 50.0;

/// One pitch sounding in a section.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    /// MIDI note units, per-note tuning included.
    pub pitch: f32,
    /// Sounding as the section starts, from an attack before it.
    pub held: bool,
    /// Its first attack inside the section, in song seconds.
    pub struck: Option<f64>,
}

/// A stretch of the song heard as one harmony.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// Song seconds of the attack that starts it.
    pub start: f64,
    /// Where the next starts, or where its sound ends if a silence longer than
    /// a Span follows.
    pub end: f64,
    /// How much the harmony changed across its start, 0..=1: what decides
    /// which of two neighbours too narrow to draw apart keeps its start.
    pub strength: f32,
    /// Lowest pitch first.
    pub entries: Vec<Entry>,
}

/// The whole tape's sections, in song order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Harmony {
    pub sections: Vec<Section>,
}

/// The threshold `detail` (0..=1) stands for: how different the harmony must
/// be across an attack to start a section. Halved four times over the bar, so
/// each step along it adds about as many sections as the last.
pub fn threshold(detail: f32) -> f32 {
    0.24 * (-4.0 * detail.clamp(0.0, 1.0)).exp2()
}

/// `span` beats in seconds at song seconds `at`, by the meter anchor that
/// governs it.
fn span_seconds(meters: &[(f64, Meter)], at: f64, span: f32) -> f64 {
    let after = meters.partition_point(|(from, _)| *from <= at);
    let beat = meters
        .get(after.saturating_sub(1))
        .map(|(_, meter)| 60.0 / meter.tempo)
        .filter(|beat| beat.is_finite() && *beat > 0.0)
        .unwrap_or(DEFAULT_BEAT_SECONDS);
    (f64::from(span) * beat).max(STEP_FLOOR)
}

/// One minus the cosine of the angle between two presence vectors; a sound
/// starting from silence, or stopping into it, is a whole change.
fn change(before: &[f32], after: &[f32]) -> f32 {
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    let (b, a) = (norm(before), norm(after));
    match (b < SILENT, a < SILENT) {
        (true, true) => 0.0,
        (true, false) | (false, true) => 1.0,
        (false, false) => {
            let dot: f32 = before.iter().zip(after).map(|(x, y)| x * y).sum();
            (1.0 - dot / (b * a)).clamp(0.0, 1.0)
        }
    }
}

/// Each pitch class's presence along the song, as running sums on a grid, so
/// a window's mean is two reads.
struct Presence {
    step: f64,
    classes: usize,
    /// `(steps + 1) * classes` running sums, and how many steps had any sound.
    sums: Vec<f32>,
    sounding: Vec<u32>,
}

impl Presence {
    fn index(&self, seconds: f64) -> usize {
        ((seconds / self.step).round().max(0.0) as usize).min(self.sounding.len() - 1)
    }

    /// The mean presence of every class over `from..to`, into `out`.
    fn mean(&self, from: f64, to: f64, out: &mut [f32]) {
        let (i, j) = (self.index(from), self.index(to));
        if j <= i {
            out.fill(0.0);
            return;
        }
        let (a, b) = (i * self.classes, j * self.classes);
        for (k, slot) in out.iter_mut().enumerate() {
            *slot = (self.sums[b + k] - self.sums[a + k]) / (j - i) as f32;
        }
    }

    /// Whether nothing sounds anywhere in `from..to`.
    fn silent(&self, from: f64, to: f64) -> bool {
        self.sounding[self.index(from)] == self.sounding[self.index(to)]
    }

    /// Where sound last stops in `from..to`, if it stops more than `gap`
    /// before `to`.
    fn falls_silent(&self, from: f64, to: f64, gap: f64) -> Option<f64> {
        let (i, j) = (self.index(from), self.index(to));
        let mut last = j;
        while last > i && self.sounding[last] == self.sounding[last - 1] {
            last -= 1;
        }
        let stops = last as f64 * self.step;
        (to - stops > gap).then_some(stops.max(from))
    }
}

/// A note as the rule reads it.
#[derive(Clone, Copy)]
struct Note {
    start: f64,
    end: f64,
    pitch: f32,
}

fn class_of(pitch: f32) -> i32 {
    ((pitch * CLASS_BINS_PER_SEMITONE).round() as i32)
        .rem_euclid((12.0 * CLASS_BINS_PER_SEMITONE) as i32)
}

fn presence(notes: &[Note], meters: &[(f64, Meter)], span: f32, end: f64) -> Presence {
    let shortest =
        notes.iter().map(|n| span_seconds(meters, n.start, span)).fold(f64::INFINITY, f64::min);
    let step = (shortest / STEPS_PER_SPAN).max(STEP_FLOOR);
    let steps = (end / step).ceil() as usize + 1;
    let mut index = BTreeMap::new();
    for note in notes {
        let next = index.len();
        index.entry(class_of(note.pitch)).or_insert(next);
    }
    let classes = index.len();
    let mut grid = vec![0.0f32; (steps + 1) * classes];
    for note in notes {
        let class = index[&class_of(note.pitch)];
        // Rounded as [`Presence::index`] rounds the windows read from it, so an
        // attack's first step is in the window after it and never the one
        // before.
        let first = ((note.start / step).round() as usize).min(steps - 1);
        let last = ((note.end / step).round() as usize).max(first + 1).min(steps);
        for t in first..last {
            grid[(t + 1) * classes + class] = 1.0;
        }
    }
    // Row `t + 1` holds step `t`. One pass turns it from "sounds" into
    // presence (fading after a release) and then into running sums.
    let mut sounding = vec![0u32; steps + 1];
    let mut fading = vec![0.0f32; classes];
    for t in 0..steps {
        let keep = (-step / (0.5 * span_seconds(meters, t as f64 * step, span))).exp() as f32;
        let (before, row) = grid.split_at_mut((t + 1) * classes);
        let before = &before[t * classes..];
        let mut any = false;
        for k in 0..classes {
            any |= row[k] > 0.0;
            fading[k] = row[k].max(fading[k] * keep);
            row[k] = before[k] + fading[k];
        }
        sounding[t + 1] = sounding[t] + u32::from(any);
    }
    Presence { step, classes, sums: grid, sounding }
}

/// The sections of `roll`, a whole recording in song seconds, under the
/// tape's `meters`. `detail` is 0..=1 and `span` is in beats.
pub fn analyze(roll: &NoteRoll, meters: &[(f64, Meter)], detail: f32, span: f32) -> Vec<Section> {
    let last = roll.notes().map(|n| n.stop(n.start)).fold(0.0, f64::max);
    let mut notes: Vec<Note> = roll
        .notes()
        .filter(|n| n.start.is_finite() && n.start >= 0.0)
        .map(|n| Note { start: n.start, end: n.stop(last), pitch: n.settled_pitch() })
        .collect();
    notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    let Some(first) = notes.first() else { return Vec::new() };
    let end = notes.iter().map(|n| n.end).fold(0.0, f64::max);
    let presence = presence(&notes, meters, span, end);

    let mut attacks = vec![first.start];
    for note in &notes {
        if note.start - attacks[attacks.len() - 1] > TOGETHER {
            attacks.push(note.start);
        }
    }
    let theta = threshold(detail);
    let (mut before, mut after) = (vec![0.0; presence.classes], vec![0.0; presence.classes]);
    let mut strengths: Vec<f32> = attacks
        .iter()
        .map(|&at| {
            let span = span_seconds(meters, at, span);
            // Nothing sounding over the Span before is a whole change, however
            // much of the last harmony's fade is left.
            if presence.silent(at - span, at) {
                return 1.0;
            }
            presence.mean(at - span, at, &mut before);
            presence.mean(at, at + span, &mut after);
            change(&before, &after)
        })
        .collect();
    strengths[0] = 1.0;

    // The strongest first, each kept unless a stronger one already stands
    // within a Span of it.
    let mut order: Vec<usize> = (0..attacks.len()).filter(|&i| strengths[i] > theta).collect();
    order.sort_by(|&a, &b| strengths[b].total_cmp(&strengths[a]).then(a.cmp(&b)));
    let mut kept = BTreeSet::new();
    for i in order {
        let span = span_seconds(meters, attacks[i], span);
        let clear = |j: Option<&usize>| j.is_none_or(|&j| (attacks[j] - attacks[i]).abs() >= span);
        if clear(kept.range(..i).next_back()) && clear(kept.range(i..).next()) {
            kept.insert(i);
        }
    }

    let starts: Vec<usize> = kept.into_iter().collect();
    let mut sections: Vec<Section> = starts
        .iter()
        .enumerate()
        .map(|(k, &i)| {
            let start = attacks[i];
            let next = starts.get(k + 1).map_or(end, |&j| attacks[j]);
            let gap = span_seconds(meters, start, span);
            let end = presence.falls_silent(start, next, gap).unwrap_or(next);
            Section { start, end, strength: strengths[i], entries: Vec::new() }
        })
        .collect();

    for note in &notes {
        let from = sections.partition_point(|s| s.end <= note.start);
        for section in sections[from..].iter_mut().take_while(|s| s.start < note.end) {
            if note.start < section.end {
                add(&mut section.entries, note, section.start);
            }
        }
    }
    for section in &mut sections {
        section.entries.sort_by(|a, b| a.pitch.total_cmp(&b.pitch));
    }
    sections
}

fn same_pitch(a: f32, b: f32) -> bool {
    (a * PITCH_BINS_PER_SEMITONE).round() == (b * PITCH_BINS_PER_SEMITONE).round()
}

fn add(entries: &mut Vec<Entry>, note: &Note, section_start: f64) {
    let struck = (note.start >= section_start - STRUCK_SLACK).then_some(note.start);
    match entries.iter_mut().find(|e| same_pitch(e.pitch, note.pitch)) {
        Some(entry) => {
            entry.held |= struck.is_none();
            entry.struck = match (entry.struck, struck) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }
        None => entries.push(Entry { pitch: note.pitch, held: struck.is_none(), struck }),
    }
}

/// A thumbnail's worth of song: one section, or several too narrow to draw
/// apart.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub start: f64,
    pub end: f64,
    /// Lowest pitch first.
    pub entries: Vec<Entry>,
}

impl Harmony {
    /// The sections as cells at least `narrowest` song seconds from start to
    /// start: where two starts are nearer than that, the one the harmony
    /// changed less across gives way, and its section joins the one before.
    pub fn cells(&self, narrowest: f64) -> Vec<Cell> {
        let sections = &self.sections;
        let mut order: Vec<usize> = (0..sections.len()).collect();
        order.sort_by(|&a, &b| {
            sections[b].strength.total_cmp(&sections[a].strength).then(a.cmp(&b))
        });
        let mut kept = BTreeSet::new();
        for i in order {
            let clear = |j: Option<&usize>| {
                j.is_none_or(|&j| (sections[j].start - sections[i].start).abs() >= narrowest)
            };
            // The song's first start always stands: every section has a cell.
            if i == 0 || (clear(kept.range(..i).next_back()) && clear(kept.range(i..).next())) {
                kept.insert(i);
            }
        }
        let starts: Vec<usize> = kept.into_iter().collect();
        starts
            .iter()
            .enumerate()
            .map(|(k, &i)| {
                let until = starts.get(k + 1).copied().unwrap_or(sections.len());
                let mut entries: Vec<Entry> = Vec::new();
                for section in &sections[i..until] {
                    for entry in &section.entries {
                        match entries.iter_mut().find(|e| same_pitch(e.pitch, entry.pitch)) {
                            // Seen in an earlier section of this cell: all a
                            // later one can add is the attack that struck it
                            // again.
                            Some(seen) => seen.struck = seen.struck.or(entry.struck),
                            None => entries.push(*entry),
                        }
                    }
                }
                entries.sort_by(|a, b| a.pitch.total_cmp(&b.pitch));
                Cell { start: sections[i].start, end: sections[until - 1].end, entries }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use harmonigraph_core::{SourceId, VoiceKey};

    use super::*;

    /// Detail and Span as a fresh look has them, at the default tempo: a
    /// two-second Span.
    const DETAIL: f32 = 0.5;
    const SPAN: f32 = 4.0;

    /// Four chords of a progression, six seconds each, over a held bass note.
    const CHORDS: [[f32; 3]; 4] =
        [[60.0, 63.86, 67.02], [65.0, 68.86, 72.0], [62.04, 67.02, 70.88], [60.0, 64.0, 67.0]];
    const CHORD_SECONDS: f64 = 6.0;

    struct Song {
        roll: NoteRoll,
        next: u32,
    }

    impl Song {
        fn new() -> Song {
            Song { roll: NoteRoll::uncapped(), next: 0 }
        }

        fn note(&mut self, pitch: f32, start: f64, end: f64) {
            // A key of its own per note, so overlapping notes of one pitch
            // are separate presses.
            let key = VoiceKey {
                source: SourceId(u64::from(self.next / 2048)),
                channel: (self.next / 128 % 16) as u8,
                note: (self.next % 128) as u8,
            };
            self.next += 1;
            self.roll.note_on(key, 0.8, pitch, start);
            self.roll.note_off(key, end);
        }

        fn starts(&self) -> Vec<f64> {
            analyze(&self.roll, &[], DETAIL, SPAN).iter().map(|s| s.start).collect()
        }
    }

    /// The progression with each chord played by `figure(song, pitches, from)`.
    fn progression(figure: impl Fn(&mut Song, &[f32; 3], f64)) -> Song {
        let mut song = Song::new();
        song.note(36.0, 0.0, CHORDS.len() as f64 * CHORD_SECONDS);
        for (i, chord) in CHORDS.iter().enumerate() {
            figure(&mut song, chord, i as f64 * CHORD_SECONDS);
        }
        song
    }

    fn plain() -> Song {
        progression(|song, chord, from| {
            for &pitch in chord {
                song.note(pitch, from, from + CHORD_SECONDS);
            }
        })
    }

    fn near(a: &[f64], b: &[f64]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.5)
    }

    #[test]
    fn each_chord_of_a_progression_is_a_section() {
        assert_eq!(plain().starts(), [0.0, 6.0, 12.0, 18.0]);
    }

    #[test]
    fn an_arpeggio_or_a_cloud_of_a_chord_is_one_section() {
        let expected = plain().starts();
        // One note at a time, fast and then slow enough that a whole cycle
        // takes most of a Span.
        for step in [0.12, 0.45] {
            let arpeggio = progression(|song, chord, from| {
                let mut at = from;
                let mut k = 0;
                while at < from + CHORD_SECONDS - 1e-9 {
                    song.note(chord[k % 3], at, (at + step * 1.2).min(from + CHORD_SECONDS));
                    (at, k) = (at + step, k + 1);
                }
            });
            assert!(near(&arpeggio.starts(), &expected), "{step}: {:?}", arpeggio.starts());
        }
        // Short notes of the chord in any octave, about fourteen a second, on
        // a fixed scramble so the test is the same every run.
        let cloud = progression(|song, chord, from| {
            let mut seed = 0x9E37_79B9u32 ^ from.to_bits() as u32;
            let mut random = move || {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f64 / f64::from(1u32 << 24)
            };
            let mut at = from;
            while at < from + CHORD_SECONDS - 0.3 {
                let pitch = chord[(random() * 3.0) as usize % 3]
                    + 12.0 * (random() * 3.0).floor() as f32
                    - 12.0;
                song.note(pitch, at, at + 0.05 + 0.3 * random());
                at += 0.02 + 0.1 * random();
            }
        });
        assert!(near(&cloud.starts(), &expected), "cloud: {:?}", cloud.starts());
    }

    #[test]
    fn a_held_chord_is_one_section_and_a_silence_ends_it() {
        let mut song = Song::new();
        for pitch in CHORDS[0] {
            song.note(pitch, 0.0, 25.0);
            song.note(pitch, 37.0, 55.0);
        }
        let sections = analyze(&song.roll, &[], DETAIL, SPAN);
        let spans: Vec<(f64, f64)> = sections.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(spans, [(0.0, 25.0), (37.0, 55.0)]);
    }

    #[test]
    fn a_rest_longer_than_a_span_ends_a_section_wherever_its_attacks_fall() {
        // The same chord either side of a three-second rest, the second
        // entry off the presence grid's steps: the fade of the first is still
        // above silence as the second arrives, and the two look alike.
        let mut song = Song::new();
        for pitch in CHORDS[0] {
            song.note(pitch, 0.0, 6.0);
            song.note(pitch, 9.15, 15.0);
        }
        let sections = analyze(&song.roll, &[], DETAIL, SPAN);
        let spans: Vec<(f64, f64)> = sections.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(spans, [(0.0, 6.0), (9.15, 15.0)]);
    }

    #[test]
    fn a_section_says_which_notes_it_struck_and_which_it_was_handed() {
        let sections = analyze(&plain().roll, &[], DETAIL, SPAN);
        let second = &sections[1];
        // The bass was struck with the first chord and still sounds.
        assert_eq!(second.entries[0], Entry { pitch: 36.0, held: true, struck: None });
        assert!(second.entries[1..].iter().all(|e| !e.held && e.struck == Some(6.0)));
        assert_eq!(second.entries.len(), 4);
    }

    #[test]
    fn more_detail_never_gives_fewer_sections_and_a_longer_span_merges_quick_changes() {
        // The progression, then two bars changing chord every second.
        let mut song = plain();
        for i in 0..8 {
            for pitch in CHORDS[i % 4] {
                song.note(pitch + 12.0, 24.0 + i as f64, 25.0 + i as f64);
            }
        }
        let count = |detail, span| analyze(&song.roll, &[], detail, span).len();
        assert!(count(0.0, SPAN) <= count(0.5, SPAN) && count(0.5, SPAN) <= count(1.0, SPAN));
        // One note of five changing is a section only with enough Detail.
        let mut subtle = Song::new();
        for pitch in [60.0, 64.0, 67.0, 71.0] {
            subtle.note(pitch, 0.0, 12.0);
        }
        subtle.note(74.0, 0.0, 6.0);
        subtle.note(77.0, 6.0, 12.0);
        let starts = |detail| analyze(&subtle.roll, &[], detail, SPAN).len();
        assert_eq!((starts(0.0), starts(0.5)), (1, 2));
        // A one-second Span follows every change; an eight-second one cannot.
        assert!(count(DETAIL, 2.0) >= 11, "{}", count(DETAIL, 2.0));
        assert!(count(DETAIL, 16.0) <= 4, "{}", count(DETAIL, 16.0));
    }

    #[test]
    fn span_is_read_in_beats_at_the_tapes_tempo() {
        // At 240 quarters a minute, a four-beat Span is one second: short
        // enough to tell the one-second chords apart, where the default
        // tempo's two seconds is not.
        let meter = Meter {
            beats: 0.0,
            tempo: 240.0,
            bar_start: 0.0,
            bar: 0,
            numerator: 4,
            denominator: 4,
        };
        let mut song = Song::new();
        for i in 0..8 {
            for pitch in CHORDS[i % 4] {
                song.note(pitch, i as f64, 1.0 + i as f64);
            }
        }
        let fast = analyze(&song.roll, &[(0.0, meter)], DETAIL, SPAN).len();
        let slow = analyze(&song.roll, &[], DETAIL, SPAN).len();
        assert_eq!(fast, 8);
        assert!(slow < fast, "{slow}");
    }

    #[test]
    fn cells_too_narrow_to_draw_join_the_one_before_and_keep_every_note() {
        let harmony = Harmony { sections: analyze(&plain().roll, &[], DETAIL, SPAN) };
        let all = harmony.cells(0.0);
        assert_eq!(all.len(), 4);
        // Twelve seconds apart at the nearest: the song's start, and whichever
        // later start changed most, with every section's notes in a cell.
        let wide = harmony.cells(12.0);
        assert_eq!(wide.len(), 2);
        assert_eq!((wide[0].start, wide[1].end), (0.0, 24.0));
        let pitches = |cells: &[Cell]| {
            // By the bins the cells themselves tell pitches
            // apart by.
            let mut all: Vec<i32> = cells
                .iter()
                .flat_map(|c| &c.entries)
                .map(|e| (e.pitch * PITCH_BINS_PER_SEMITONE).round() as i32)
                .collect();
            all.sort_unstable();
            all.dedup();
            all
        };
        assert_eq!(pitches(&wide), pitches(&all));
        // The bass is held into the second cell; a chord struck inside a
        // joined section is struck in the cell.
        assert!(wide[1].entries[0].held);
        assert!(wide[1].entries[1..].iter().all(|e| e.struck.is_some()));

        // A note held into a cell and struck again in a section joined to it
        // is both: there from the start, and struck when it was.
        let held = Entry { pitch: 60.0, held: true, struck: None };
        let again = Entry { pitch: 60.0, held: false, struck: Some(8.0) };
        let section = |start: f64, strength, entry| Section {
            start,
            end: start + 6.0,
            strength,
            entries: vec![entry],
        };
        let joined = Harmony { sections: vec![section(0.0, 1.0, held), section(6.0, 0.5, again)] };
        assert_eq!(joined.cells(12.0)[0].entries, [Entry { struck: Some(8.0), ..held }]);
    }
}
