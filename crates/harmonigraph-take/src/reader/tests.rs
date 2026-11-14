use harmonigraph_core::canonical::{SourceBaseline, VoiceBaseline};
use harmonigraph_core::{NoteEvent, SourceId};

use crate::canonical::{BaselineRecord, DeltaRecord, GapRecord, TimingRecord};
use crate::*;

const RATE: f64 = 48_000.0;
/// The Hub's delay in these fixtures: one 64-sample block.
const DELAY: i64 = 64;

/// Presentation time of `input` on the clock epoch `epoch`. A clock cut
/// restarts the sample count, not the presentation clock.
fn presentation(epoch: u64, input: i64) -> f64 {
    10.0 + epoch as f64 + input as f64 / RATE
}

fn at(epoch: u64, input: i64) -> StampRecord {
    StampRecord { runtime_session: 1, clock_epoch: epoch, input }
}

fn run(pass: u32, epoch: u64, input: i64, song: f64) -> Record {
    Record::Run(RunRecord {
        pass,
        at: at(epoch, input),
        song,
        presentation: presentation(epoch, input),
        audio_file: Some(format!("take-{pass}.wav")),
    })
}

fn end(epoch: u64, input: i64) -> Record {
    Record::RunEnd(RunEndRecord { at: at(epoch, input) })
}

/// A delta played at `input` on the DIRECT row, sounding `DELAY` later.
fn delta(sequence: u64, key: u8, on: bool, epoch: u64, input: i64) -> Record {
    let time = presentation(epoch, input + DELAY);
    let event = if on {
        NoteEvent::on(time, SourceId::DIRECT, 0, key, 0.8)
    } else {
        NoteEvent::off(time, SourceId::DIRECT, 0, key)
    };
    let mut delta = DeltaRecord::from(harmonigraph_core::canonical::NoteDelta::from(event));
    delta.sequence = sequence;
    delta.timing = Some(TimingRecord {
        runtime_session: 1,
        clock_epoch: epoch,
        input,
        planned: Some(input + DELAY),
        sample: input + DELAY,
        sample_rate: RATE,
    });
    Record::Canonical(CanonicalRecord::Delta(delta))
}

/// A run's opening snapshot of the DIRECT row, taken at `input`, holding `held`.
fn snapshot(id: u64, cut: u64, held: &[u8], epoch: u64, input: i64) -> Record {
    let time = presentation(epoch, input);
    let voices: Vec<_> = held
        .iter()
        .map(|&note| VoiceBaseline {
            note,
            velocity: 0.8,
            pitch_microcents: i64::from(note) * 100_000_000,
            input_onset: time,
            actual_onset: time,
            ..Default::default()
        })
        .collect();
    let frame = SourceBaseline::new(SourceId::DIRECT, id, time, cut, true, &voices).unwrap();
    let mut record = BaselineRecord::from(&frame);
    record.at = Some(at(epoch, input));
    Record::Canonical(CanonicalRecord::Baseline(Box::new(record)))
}

fn gap(first: u64, from: StampRecord, to: StampRecord) -> Record {
    let time = |at: StampRecord| presentation(at.clock_epoch, at.input);
    Record::Canonical(CanonicalRecord::Gap(GapRecord {
        t: time(from),
        through: time(to),
        first,
        last: first,
        inputs: Some([from, to]),
        ..Default::default()
    }))
}

fn read(records: &[Record], pass: Option<u32>) -> Result<Take, ReadError> {
    let mut text = ron::to_string(&Record::Header(Header::default())).unwrap();
    for record in records {
        text.push('\n');
        text.push_str(&ron::to_string(record).unwrap());
    }
    Take::parse_pass(std::io::Cursor::new(text), pass)
}

/// `(key, on, song time)` for every delta the pass draws, in order.
fn drawn(take: &Take) -> Vec<(u8, bool, f64)> {
    take.events
        .iter()
        .filter_map(|event| match event {
            CanonicalRecord::Delta(d) => {
                Some((d.event.note, matches!(d.event.kind, NoteKind::On { .. }), d.event.t))
            }
            _ => None,
        })
        .collect()
}

/// Each run moves by its one offset into the song, so a resume continues the
/// pass's audio; what played in the pause, and a snapshot taken there, belong
/// to no run and are dropped, and the resume's own snapshot restates the note.
#[test]
fn a_pass_holds_its_runs_on_the_song_and_drops_its_pause() {
    let paused = 128.0 / RATE;
    let take = read(
        &[
            run(1, 0, 0, 2.0),
            delta(1, 60, true, 0, 10),
            end(0, 128),
            delta(2, 62, true, 0, 160),
            snapshot(1, 2, &[60, 62], 0, 192),
            run(1, 0, 256, 2.0 + paused),
            snapshot(2, 2, &[60, 62], 0, 256),
            delta(3, 60, false, 0, 300),
        ],
        None,
    )
    .unwrap();
    assert_eq!(take.passes, [1]);
    assert_eq!((take.audio_file.as_deref(), take.audio_start), (Some("take-1.wav"), Some(2.0)));
    let song = |input: i64, run_start: i64, run_song: f64| {
        run_song + (input + DELAY - run_start) as f64 / RATE
    };
    let notes = drawn(&take);
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert_eq!((notes[0].0, notes[0].1), (60, true));
    assert!((notes[0].2 - song(10, 0, 2.0)).abs() < 1e-9);
    assert_eq!((notes[1].0, notes[1].1), (60, false));
    assert!((notes[1].2 - song(300, 256, 2.0 + paused)).abs() < 1e-9);
    let snapshots: Vec<_> = take
        .events
        .iter()
        .filter_map(|e| match e {
            CanonicalRecord::Baseline(b) => Some((b.id, b.t)),
            _ => None,
        })
        .collect();
    assert_eq!(snapshots.len(), 1, "only the resume's own snapshot: {snapshots:?}");
    assert_eq!(snapshots[0].0, 2);
    assert!((snapshots[0].1 - (2.0 + paused)).abs() < 1e-9);
    assert!(take.incomplete.is_none());
}

/// The last pass by default, any other on request, and a refusal naming what
/// the file holds for a pass it does not. A file with no `Run` is one pass.
#[test]
fn the_last_pass_reads_unless_another_is_asked_for() {
    let records = [
        run(1, 0, 0, 5.0),
        delta(1, 60, true, 0, 0),
        end(0, 64),
        run(2, 0, 64, 1.0),
        snapshot(1, 1, &[60], 0, 64),
        delta(2, 60, false, 0, 100),
    ];
    let last = read(&records, None).unwrap();
    assert_eq!((last.passes.as_slice(), last.pass), (&[1, 2][..], Some(2)));
    assert_eq!(last.audio_start, Some(1.0));
    assert_eq!(drawn(&last).iter().map(|n| n.1).collect::<Vec<_>>(), [false]);
    let first = read(&records, Some(1)).unwrap();
    assert_eq!((first.pass, first.audio_start), (Some(1), Some(5.0)));
    assert_eq!(drawn(&first).iter().map(|n| n.1).collect::<Vec<_>>(), [true]);
    let missing = read(&records, Some(3)).unwrap_err();
    assert!(matches!(&missing, ReadError::NoPass { asked: 3, passes } if passes == &[1, 2]));

    // A file with no `Run` is one pass. The standalone's plain note reads; a
    // stamped record was played where nothing recorded, an armed take whose
    // transport never rolled, and is dropped.
    let plain =
        NoteRecord { t: 0.5, note: 64, kind: NoteKind::On { velocity: 0.8 }, ..Default::default() };
    let unrouted = [Record::Note(plain), delta(1, 60, true, 0, 0)];
    let take = read(&unrouted, None).unwrap();
    assert_eq!((take.passes.as_slice(), take.pass), (&[1][..], Some(1)));
    assert_eq!(take.notes().collect::<Vec<_>>(), [plain]);
    let missing = read(&unrouted, Some(2)).unwrap_err();
    assert!(matches!(&missing, ReadError::NoPass { asked: 2, passes } if passes == &[1]));
}

/// Note history without a stamp has no place in a file that routes by them,
/// so it is refused rather than placed by guess.
#[test]
fn unstamped_history_beside_a_run_is_refused() {
    let plain =
        NoteRecord { t: 0.5, note: 64, kind: NoteKind::On { velocity: 0.8 }, ..Default::default() };
    let unstamped = NoteEvent::on(0.5, SourceId::DIRECT, 0, 64, 0.8).into();
    for (line, record) in [
        Record::Note(plain),
        Record::Canonical(CanonicalRecord::from_event(
            harmonigraph_core::canonical::CanonicalEvent::Note(unstamped),
        )),
    ]
    .into_iter()
    .enumerate()
    {
        let error = read(&[run(1, 0, 0, 0.0), record], None).unwrap_err();
        assert!(matches!(error, ReadError::InvalidCanonical(3)), "case {line}: {error}");
    }
}

/// A record played in the last D of a run sounds after it, past the next
/// run's opening snapshot, which covers it. Under the audio origin it is
/// late by less than its own delay, so the snapshot lifts level with it and
/// the pass reads whole (#1760's `r3-parked-at-start`, read in place).
#[test]
fn a_snapshot_below_a_record_sounding_past_its_run_lifts_level_with_it() {
    // The release's input is 4 samples before the run's end: it sounds 60
    // samples into the next run's song, after that run's snapshot.
    let resumed = run(1, 0, 256, 2.0 + 128.0 / RATE);
    let records =
        [run(1, 0, 0, 2.0), delta(1, 60, true, 0, 0), delta(2, 60, false, 0, 124), end(0, 128)];
    let take = read(
        &[records.as_slice(), &[resumed, snapshot(1, 2, &[], 0, 256), delta(3, 64, true, 0, 300)]]
            .concat(),
        None,
    )
    .expect("the lifted snapshot sorts after the release it covers");
    assert!(take.incomplete.is_none(), "a lift inside the bound is a pause, not a loss");
    let release = drawn(&take)[1].2;
    let lifted = take.events.iter().find_map(|e| match e {
        CanonicalRecord::Baseline(b) => Some(b.t),
        _ => None,
    });
    assert_eq!(lifted, Some(release));
    // Without the lift this fixture is refused, which is what makes it reach.
    assert!(release > 2.0 + 128.0 / RATE);
}

/// A voice a resume's snapshot restates draws from where it was struck: by the
/// run that holds its own onset when one does, and from the resume when none
/// does (struck in the pause). Here the history before the snapshot is lost,
/// so the snapshot is all that draws either note.
#[test]
fn a_restated_voice_draws_from_the_run_its_onset_was_played_in() {
    let voice = |note: u8, input: i64| {
        let onset = presentation(0, input + DELAY);
        VoiceBaseline {
            note,
            velocity: 0.8,
            pitch_microcents: i64::from(note) * 100_000_000,
            input_onset: presentation(0, input),
            actual_onset: onset,
            onset: Some(harmonigraph_core::canonical::EventTiming {
                clock: harmonigraph_core::canonical::ClockId { runtime_session: 1, epoch: 0 },
                input,
                planned: None,
                sample: input + DELAY,
                sample_rate: RATE,
            }),
            ..Default::default()
        }
    };
    let time = presentation(0, 256);
    let frame =
        SourceBaseline::new(SourceId::DIRECT, 1, time, 0, true, &[voice(60, 10), voice(62, 160)])
            .unwrap();
    let mut restated = BaselineRecord::from(&frame);
    restated.at = Some(at(0, 256));
    let resumed = 2.0 + 128.0 / RATE;
    let take = read(
        &[
            run(1, 0, 0, 2.0),
            end(0, 128),
            run(1, 0, 256, resumed),
            Record::Canonical(CanonicalRecord::Baseline(Box::new(restated))),
        ],
        None,
    )
    .unwrap();
    let mut tracker = harmonigraph_core::NoteTracker::new();
    for event in &take.events {
        event.apply(&mut tracker).unwrap();
    }
    let start = |key| tracker.roll().notes().find(|n| n.note == key).unwrap().start;
    assert!((start(60) - (2.0 + 74.0 / RATE)).abs() < 1e-9, "struck in run 1: {}", start(60));
    assert!((start(62) - resumed).abs() < 1e-9, "struck in the pause: {}", start(62));
}

/// A record that would need more lift than its maximum's own lateness is a
/// fault in the stamps: here the resume's song lags the audio, as the host's
/// position did before the audio origin. It still reads, and the pass warns.
#[test]
fn a_lift_past_its_bound_reads_and_warns() {
    let lagging = run(1, 0, 256, 2.0);
    let take = read(
        &[
            run(1, 0, 0, 2.0),
            delta(1, 60, true, 0, 0),
            delta(2, 60, false, 0, 124),
            end(0, 128),
            lagging,
            snapshot(1, 2, &[], 0, 256),
        ],
        None,
    )
    .expect("a breach lifts anyway, so the file stays valid");
    let marker = take.incomplete.expect("and the pass says it is not whole");
    assert_eq!(marker.reason, canonical::GapReasonRecord::InvalidRecord);
}

/// A gap marks the passes whose runs its inputs touch, across a clock cut too,
/// and a gap that touched no run marks nothing: the loss was of history no
/// pass recorded.
#[test]
fn a_gap_marks_only_the_passes_its_inputs_touch() {
    let records = [
        run(1, 0, 0, 0.0),
        end(0, 128),
        // A loss while paused, before the next pass.
        gap(5, at(0, 140), at(0, 150)),
        run(2, 1, 0, 9.0),
        end(1, 64),
        // From pass 1's run on the old clock, across the cut, into pass 2's.
        gap(9, at(0, 100), at(1, 10)),
        run(3, 1, 128, 20.0),
    ];
    let marked = |pass| read(&records, Some(pass)).unwrap().incomplete.map(|m| m.first_publication);
    assert_eq!(marked(1), Some(9));
    assert_eq!(marked(2), Some(9));
    assert_eq!(marked(3), None, "the pause's gap touched no run");
    // Each pass draws the gap only over its own runs, each end moved by the
    // run it falls in: pass 1 does not grow into pass 2's time.
    let span = |pass| {
        let take = read(&records, Some(pass)).unwrap();
        take.events.iter().find_map(|event| match event {
            CanonicalRecord::Gap(gap) => Some((gap.t, gap.through)),
            _ => None,
        })
    };
    let (t, through) = span(1).unwrap();
    assert!((t - 100.0 / RATE).abs() < 1e-9 && (through - 128.0 / RATE).abs() < 1e-9);
    let (t, through) = span(2).unwrap();
    assert!((t - 9.0).abs() < 1e-9 && (through - (9.0 + 10.0 / RATE)).abs() < 1e-9);
}

/// A failure below a pass's `PassClosed` line touched only the passes after
/// it, so an earlier pass still renders without a warning.
#[test]
fn a_failure_after_a_pass_closed_leaves_that_pass_whole() {
    let records = [
        run(1, 0, 0, 0.0),
        end(0, 64),
        run(2, 0, 64, 4.0),
        Record::PassClosed(1),
        Record::Incomplete(IncompleteRecord {
            reason: canonical::GapReasonRecord::ProducerLost,
            ..Default::default()
        }),
    ];
    assert!(read(&records, Some(1)).unwrap().incomplete.is_none());
    assert!(read(&records, Some(2)).unwrap().incomplete.is_some());
}
