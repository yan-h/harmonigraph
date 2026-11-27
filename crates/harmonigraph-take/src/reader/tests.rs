use harmonigraph_core::canonical::{SourceBaseline, VoiceBaseline};
use harmonigraph_core::{NoteEvent, SourceId};

use crate::canonical::{BaselineRecord, DeltaRecord, GapRecord};
use crate::*;

const RATE: f64 = 48_000.0;
/// The Hub's delay in these fixtures: one 64-sample block.
const DELAY: i64 = 64;

/// Presentation time of `sample`.
fn presentation(sample: i64) -> f64 {
    10.0 + sample as f64 / RATE
}

/// A run opening at `sample`, at `song` in the take's audio.
fn run(sample: i64, song: f64) -> Record {
    Record::Run(RunRecord { at: presentation(sample), song })
}

fn end(sample: i64) -> Record {
    Record::RunEnd(RunEndRecord { at: presentation(sample) })
}

/// A delta of the DIRECT row played at `input` and sounding `DELAY` later.
/// Each fixture strikes a key once, so its lifetime is the key.
fn delta(sequence: u64, key: u8, on: bool, input: i64) -> Record {
    let time = presentation(input + DELAY);
    let event = if on {
        NoteEvent::on(time, SourceId::DIRECT, 0, key, 0.8)
    } else {
        NoteEvent::off(time, SourceId::DIRECT, 0, key)
    };
    let mut delta = DeltaRecord::from(harmonigraph_core::canonical::NoteDelta::from(event));
    delta.sequence = sequence;
    delta.lifetime = u64::from(key);
    Record::Canonical(CanonicalRecord::Delta(delta))
}

/// A voice played at `input`, as a snapshot restates it.
fn struck(note: u8, input: i64) -> VoiceBaseline {
    VoiceBaseline {
        note,
        velocity: 0.8,
        pitch_microcents: i64::from(note) * 100_000_000,
        lifetime: u64::from(note),
        actual_onset: presentation(input + DELAY),
        ..Default::default()
    }
}

/// A snapshot of the DIRECT row at `sample`, covering output through `cut`.
fn snapshot(cut: u64, voices: &[VoiceBaseline], sample: i64) -> Record {
    let frame =
        SourceBaseline::new(SourceId::DIRECT, presentation(sample), cut, true, voices).unwrap();
    Record::Canonical(CanonicalRecord::Baseline(Box::new(BaselineRecord::from(&frame))))
}

fn gap(first: u64, from: i64, to: i64) -> Record {
    Record::Canonical(CanonicalRecord::Gap(GapRecord {
        t: presentation(from),
        through: presentation(to),
        first,
        last: first,
    }))
}

fn read(records: &[Record]) -> Result<Take, ReadError> {
    let mut text = ron::to_string(&Record::Header(Header::default())).unwrap();
    for record in records {
        text.push('\n');
        text.push_str(&ron::to_string(record).unwrap());
    }
    Take::parse(std::io::Cursor::new(text))
}

/// `(start, end)` of every note the take's events draw for `key`, through the
/// replay path.
fn notes_of(take: &Take, key: u8) -> Vec<(f64, Option<f64>)> {
    let mut tracker = harmonigraph_core::NoteTracker::new();
    for event in &take.events {
        event.apply(&mut tracker).unwrap();
    }
    tracker.roll().notes().filter(|n| n.note == key).map(|n| (n.start, n.end)).collect()
}

/// Song time of a record sounding at `sample` in a run opened at `start` at
/// `song`.
fn song(sample: i64, start: i64, song: f64) -> f64 {
    song + (sample - start) as f64 / RATE
}

/// Each run moves what sounds in it by its one offset, so a resume continues
/// the take's audio. What sounds in the pause, a release played just before it
/// included, is dropped; the resume's own snapshot restates what is held.
#[test]
fn a_take_holds_its_runs_on_its_audio_and_drops_its_pause() {
    let resumed = 2.0 + 128.0 / RATE;
    let take = read(&[
        run(0, 2.0),
        delta(1, 60, true, 10),
        delta(2, 62, true, 40),
        delta(3, 62, false, 100),
        end(128),
        delta(4, 64, true, 160),
        run(256, resumed),
        snapshot(4, &[struck(60, 10), struck(64, 160)], 256),
        delta(5, 60, false, 300),
    ])
    .unwrap();
    assert_eq!(take.audio_start, Some(2.0));
    let t = |sample| presentation(sample) - presentation(0);
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let sixty = notes_of(&take, 60);
    assert!(sixty.len() == 1 && near(sixty[0].0, 2.0 + t(74)), "{sixty:?}");
    assert!(near(sixty[0].1.unwrap(), song(364, 256, resumed)), "{sixty:?}");
    // 62's release sounded after the run ended, so the note runs to the
    // snapshot that no longer lists it.
    let sixty_two = notes_of(&take, 62);
    assert!(sixty_two.len() == 1 && near(sixty_two[0].0, 2.0 + t(104)), "{sixty_two:?}");
    // 64 began in the pause: it starts with the run that restates it.
    let sixty_four = notes_of(&take, 64);
    assert!(sixty_four.len() == 1 && near(sixty_four[0].0, resumed), "{sixty_four:?}");
    assert!(!take.failed && take.gap().is_none());
}

/// The take's end is the end of what it observed. A note still held there is
/// released at it, so a Tail fades the note and counts from the end: its own
/// release sounded after the last block that recorded, as a host's releases do
/// when the transport stops, and was dropped with the stop's snapshot. A take
/// that ends with nothing held gains no record, so its length is still its
/// last event's (#1803).
#[test]
fn the_takes_end_releases_what_is_still_held() {
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let held = read(&[
        run(0, 0.0),
        delta(1, 60, true, 24_000 - DELAY),
        end(96_000),
        delta(2, 60, false, 96_000 - DELAY),
        snapshot(2, &[], 96_000),
    ])
    .unwrap();
    let note = notes_of(&held, 60);
    assert!(note.len() == 1 && near(note[0].0, 0.5), "{note:?}");
    assert!(note[0].1.is_some_and(|end| near(end, 2.0)), "{note:?}");
    assert!(near(held.duration(), 2.0), "{}", held.duration());

    let released = read(&[
        run(0, 0.0),
        delta(1, 60, true, 24_000 - DELAY),
        delta(2, 60, false, 48_000 - DELAY),
        end(96_000),
    ])
    .unwrap();
    assert!(near(released.duration(), 1.0), "{}", released.duration());
}

/// A record played before a pause that sounds after the next run begins lands
/// in that run, before its opening snapshot, and continues the note that
/// snapshot restates rather than splitting or orphaning it.
#[test]
fn a_record_sounding_into_the_next_run_lands_before_its_snapshot() {
    let resumed = 2.0 + 120.0 / RATE;
    let take = read(&[
        run(0, 2.0),
        delta(1, 60, true, 10),
        end(120),
        // Played in the run, sounding after the resume at 160.
        delta(2, 62, true, 110),
        run(160, resumed),
        snapshot(2, &[struck(60, 10), struck(62, 110)], 174),
        delta(3, 60, false, 200),
        delta(4, 62, false, 210),
    ])
    .unwrap();
    for key in [60, 62] {
        let notes = notes_of(&take, key);
        assert!(notes.len() == 1 && notes[0].1.is_some(), "{key}: {notes:?}");
    }
    let start = notes_of(&take, 62)[0].0;
    assert!((start - song(174, 160, resumed)).abs() < 1e-9, "{start}");
}

/// A gap warns if it overlaps a run and is drawn inside it; one wholly in a
/// pause warns nothing. A failure marker always marks the take failed, an
/// older one with its range and reason too, and a gap ahead of it hides
/// neither.
#[test]
fn a_gap_warns_where_it_overlaps_a_run_and_a_failure_always_does() {
    let runs = [run(0, 2.0), end(128), run(256, 2.0 + 128.0 / RATE)];
    let read_with = |extra: Record| {
        let mut records = runs.to_vec();
        records.push(extra);
        read(&records).unwrap()
    };
    let paused = read_with(gap(1, 140, 200));
    assert!(paused.gap().is_none() && !paused.failed);
    let crossing = read_with(gap(1, 100, 300));
    assert!(crossing.gap().is_some() && !crossing.failed);
    let span = crossing.events.iter().find_map(|e| match e {
        CanonicalRecord::Gap(g) => Some((g.t, g.through)),
        _ => None,
    });
    let (t, through) = span.unwrap();
    assert!((t - song(100, 0, 2.0)).abs() < 1e-9 && (through - song(128, 0, 2.0)).abs() < 1e-9);
    let older = "Incomplete((first_publication:0,last_publication:0,reason:ProducerLost))";
    let failed = read_with(ron::from_str(older).unwrap());
    assert!(failed.failed && failed.gap().is_none());
    let mut both = runs.to_vec();
    both.extend([gap(1, 100, 300), Record::Incomplete(IncompleteRecord::default())]);
    let both = read(&both).unwrap();
    assert!(both.failed && both.gap().is_some(), "the gap first hides no failure");
}

/// A voice whose onset the take's lane lost is drawn from the snapshot that
/// restates it, so its restated onset moves by the run it sounded in, as its
/// attack would have, rather than by the snapshot's run.
#[test]
fn a_restated_onset_lost_to_a_gap_keeps_the_run_it_sounded_in() {
    let resumed = 2.0 + 128.0 / RATE;
    let take = read(&[
        run(0, 2.0),
        // 60's onset, sounding at 74, lost from the take's lane.
        gap(1, 74, 74),
        end(128),
        run(256, resumed),
        snapshot(1, &[struck(60, 10)], 256),
        delta(2, 60, false, 300),
    ])
    .unwrap();
    let notes = notes_of(&take, 60);
    assert!(notes.len() == 1 && (notes[0].0 - song(74, 0, 2.0)).abs() < 1e-9, "{notes:?}");
}

/// A take that breaks its source's order is refused, not drawn. No producer
/// writes one since #1885, but a take recorded before it while a Tune's delay
/// was lowered holds a snapshot timed at the old delay after deltas timed at
/// the new, and replaying it would draw the wrong notes with nothing saying so.
#[test]
fn a_take_that_breaks_its_sources_order_is_refused() {
    let read = read(&[
        run(0, 2.0),
        delta(1, 60, true, 10),
        // Timed at the old delay: after the release below, which it does not cover.
        snapshot(1, &[struck(60, 10)], 300),
        delta(2, 60, false, 200),
    ]);
    assert!(matches!(read, Err(ReadError::OutOfOrder(4))), "{read:?}");
}
