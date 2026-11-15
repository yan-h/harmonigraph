use harmonigraph_core::canonical::{SourceBaseline, VoiceBaseline};
use harmonigraph_core::{NoteEvent, SourceId};

use crate::canonical::{BaselineRecord, DeltaRecord, GapRecord, ProvenanceRecord, TimingRecord};
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
    delta.provenance = ProvenanceRecord::AcceptedOutput;
    delta.timing = Some(TimingRecord { sample: input + DELAY, sample_rate: RATE });
    Record::Canonical(CanonicalRecord::Delta(delta))
}

/// A voice played at `input`, as a snapshot restates it.
fn struck(note: u8, input: i64) -> VoiceBaseline {
    VoiceBaseline {
        note,
        velocity: 0.8,
        pitch_microcents: i64::from(note) * 100_000_000,
        lifetime: u64::from(note),
        provenance: harmonigraph_core::confirmed::PitchProvenance::AcceptedOutput,
        input_onset: presentation(input),
        actual_onset: presentation(input + DELAY),
        onset: Some(harmonigraph_core::canonical::EventTiming {
            sample: input + DELAY,
            sample_rate: RATE,
        }),
        ..Default::default()
    }
}

/// A snapshot of the DIRECT row at `sample`, covering output through `cut`.
fn snapshot(id: u64, cut: u64, voices: &[VoiceBaseline], sample: i64) -> Record {
    let frame =
        SourceBaseline::new(SourceId::DIRECT, id, presentation(sample), cut, true, voices).unwrap();
    let mut record = BaselineRecord::from(&frame);
    for voice in &mut record.voices {
        voice.assignment =
            Some(ConfigurationRecord { t: voice.actual_onset, ..ConfigurationRecord::default() });
    }
    Record::Canonical(CanonicalRecord::Baseline(Box::new(record)))
}

fn gap(first: u64, from: i64, to: i64) -> Record {
    Record::Canonical(CanonicalRecord::Gap(GapRecord {
        t: presentation(from),
        through: presentation(to),
        first,
        last: first,
        ..Default::default()
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
/// replay path, after checking each restated voice's assignment is at its
/// onset.
fn notes_of(take: &Take, key: u8) -> Vec<(f64, Option<f64>)> {
    let mut tracker = harmonigraph_core::NoteTracker::new();
    for event in &take.events {
        if let CanonicalRecord::Baseline(b) = event {
            for voice in &b.voices {
                assert_eq!(voice.assignment.map(|a| a.t), Some(voice.actual_onset));
            }
        }
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
        snapshot(1, 4, &[struck(60, 10), struck(64, 160)], 256),
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
    assert!(take.incomplete.is_none());
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
        snapshot(1, 2, &[struck(60, 10), struck(62, 110)], 174),
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
/// pause warns nothing. A failure marker always warns.
#[test]
fn a_gap_warns_where_it_overlaps_a_run_and_a_failure_always_does() {
    let runs = [run(0, 2.0), end(128), run(256, 2.0 + 128.0 / RATE)];
    let read_with = |extra: Record| {
        let mut records = runs.to_vec();
        records.push(extra);
        read(&records).unwrap()
    };
    let paused = read_with(gap(1, 140, 200));
    assert!(paused.incomplete.is_none());
    let crossing = read_with(gap(1, 100, 300));
    assert!(crossing.incomplete.is_some());
    let span = crossing.events.iter().find_map(|e| match e {
        CanonicalRecord::Gap(g) => Some((g.t, g.through)),
        _ => None,
    });
    let (t, through) = span.unwrap();
    assert!((t - song(100, 0, 2.0)).abs() < 1e-9 && (through - song(128, 0, 2.0)).abs() < 1e-9);
    let failed = read_with(Record::Incomplete(IncompleteRecord {
        reason: canonical::GapReasonRecord::ProducerLost,
        ..Default::default()
    }));
    assert!(failed.incomplete.is_some());
}
