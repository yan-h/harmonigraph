//! Recorder boundary parity, through the real display consumer and writer pump.
//! This does not exercise the DAW/host/Hub or the writer command thread, nor
//! establish audio, pixel, envelope, or arbitrary replay-cadence equivalence.

use super::*;
use harmonigraph_render::wgpu::TextureFormat;

#[derive(Debug, PartialEq)]
struct RollSnapshot {
    source: harmonigraph_core::SourceId,
    lifetime: Option<u64>,
    channel: u8,
    note: u8,
    start: f64,
    end: Option<f64>,
    observed_until: Option<f64>,
    history_complete: bool,
    end_pitch: f32,
    segments: Vec<((f64, f32), (f64, f32))>,
}

fn roll_snapshot(
    tracker: &harmonigraph_core::NoteTracker,
    now: f64,
    clock_origin: f64,
) -> Vec<RollSnapshot> {
    tracker
        .roll()
        .notes()
        .map(|n| RollSnapshot {
            source: n.source,
            lifetime: n.lifetime,
            channel: n.channel,
            note: n.note,
            start: n.start - clock_origin,
            end: n.end.map(|t| t - clock_origin),
            observed_until: n.observed_until.map(|t| t - clock_origin),
            history_complete: n.history_complete,
            end_pitch: n.end_pitch(),
            segments: n
                .segments(now)
                .map(|((a, p), (b, q))| ((a - clock_origin, p), (b - clock_origin, q)))
                .collect(),
        })
        .collect()
}

fn parity_path(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir()
        .join(format!("harmonigraph-replay-parity-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    directory.join("capture.take")
}

#[test]
fn ordinary_recorder_display_and_disk_replay_share_note_semantics() {
    use harmonigraph_core::{NoteEvent, NoteEventKind, NoteTracker, SourceId};
    use harmonigraph_record::testing;

    const ORIGIN: f64 = 10.0;
    let (mut recorder, mut capture) = testing::channel();
    capture.arm();
    assert!(recorder.is_armed());
    let path = parity_path("ordinary");
    let mut writer = testing::FileWriter::new(&capture, path.clone(), None);
    let live_events = [
        NoteEvent::on(ORIGIN + 0.125, SourceId::DIRECT, 0, 60, 0.8),
        NoteEvent {
            time: ORIGIN + 0.25,
            source: SourceId::DIRECT,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Tuning { semitones: 0.25 },
        },
        NoteEvent::off(ORIGIN + 0.375, SourceId::DIRECT, 0, 60),
        NoteEvent::on(ORIGIN + 0.5, SourceId::DIRECT, 0, 64, 0.7),
        NoteEvent::source_reset(ORIGIN + 0.625, SourceId::DIRECT),
    ];
    for live in live_events {
        // Ordinary MIDI uses separate display publication and take-time input.
        recorder.publish_note(live.into(), Default::default()).expect_both();
        recorder.note(live.time - ORIGIN, live.source, live.channel, live.note, live.kind);
    }

    let mut direct = NoteTracker::new();
    assert_eq!(capture.display_into(&mut direct, |_, _| {}), live_events.len());
    recorder.finish_callback();
    capture.stop();
    assert!(!recorder.is_armed());
    recorder.finish_callback();
    writer.stop();
    writer.drain(&mut capture);
    assert!(!writer.failed());
    assert_eq!(writer.finished.as_ref(), Some(&path));

    let take = Take::read(&path).unwrap();
    assert!(!take.truncated);
    assert!(take.incomplete.is_none());
    assert_eq!(take.events.len(), live_events.len());
    let mut replay = Replay::new(take);
    let mut state = PictureState::new(TextureFormat::Bgra8Unorm);
    replay.advance_to(&mut state.runtime, 1.0);
    assert!(replay.is_spent());
    assert_eq!(
        roll_snapshot(&direct, ORIGIN + 1.0, ORIGIN),
        roll_snapshot(&state.runtime.tracker, 1.0, 0.0)
    );
    let actual = roll_snapshot(&state.runtime.tracker, 1.0, 0.0);
    assert_eq!(
        actual,
        [
            RollSnapshot {
                source: SourceId::DIRECT,
                lifetime: None,
                channel: 0,
                note: 60,
                start: 0.125,
                end: Some(0.375),
                observed_until: None,
                history_complete: true,
                end_pitch: 60.25,
                segments: vec![((0.125, 60.0), (0.25, 60.25)), ((0.25, 60.25), (0.375, 60.25))],
            },
            RollSnapshot {
                source: SourceId::DIRECT,
                lifetime: None,
                channel: 0,
                note: 64,
                start: 0.5,
                end: Some(0.625),
                observed_until: None,
                history_complete: true,
                end_pitch: 64.0,
                segments: vec![((0.5, 64.0), (0.625, 64.0))],
            },
        ]
    );
    assert_eq!(direct.held_count(), 0);
    assert_eq!(state.runtime.tracker.held_count(), 0);

    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

fn accepted_exact(
    event: harmonigraph_core::NoteEvent,
    sequence: u64,
    lifetime: u64,
    pitch_microcents: Option<i64>,
) -> harmonigraph_core::canonical::NoteDelta {
    use harmonigraph_core::canonical::{ClockId, EventTiming, NoteDelta};
    use harmonigraph_core::confirmed::PitchProvenance;
    NoteDelta {
        event,
        sequence,
        lifetime,
        provenance: PitchProvenance::AcceptedOutput,
        timing: Some(EventTiming {
            clock: ClockId { runtime_session: 7, epoch: 3 },
            input: (event.time * 48_000.0) as i64 - 32,
            planned: None,
            sample: (event.time * 48_000.0) as i64,
            sample_rate: 48_000.0,
        }),
        pitch_microcents,
        assignment: None,
        partial_output: false,
    }
}

#[test]
fn canonical_recorder_display_and_disk_replay_share_gap_repair_and_routing() {
    use harmonigraph_core::canonical::{ClockId, EventTiming, SourceBaseline, VoiceBaseline};
    use harmonigraph_core::confirmed::PitchProvenance;
    use harmonigraph_core::{NoteEvent, NoteEventKind, NoteTracker, SourceId};
    use harmonigraph_record::configuration::RecordAddress;
    use harmonigraph_record::publication::{Lane, Route};
    use harmonigraph_record::testing;

    const ORIGIN: f64 = 10.0;
    const LIFETIME: u64 = 61;
    let source = SourceId(1);
    let address = RecordAddress { epoch: 1, pass: 1 };
    let route = Route { address: Some(address), time_offset: -ORIGIN };
    let (mut recorder, mut capture) = testing::channel();
    recorder.enable_configuration();
    recorder.enable_canonical();
    capture.arm();
    assert!(recorder.is_armed());
    let path = parity_path("canonical");
    let mut writer = testing::FileWriter::new(&capture, path.clone(), None);
    let configuration = harmonigraph_core::configuration::ConfigReducer::default().resolved();
    recorder.configuration_at(address, 0.0, configuration);

    recorder
        .publish_note(
            accepted_exact(
                NoteEvent::on(ORIGIN + 0.125, source, 0, 60, 0.8),
                1,
                LIFETIME,
                Some(6_000_000_000),
            ),
            route,
        )
        .expect_both();
    // The absurd f32 adapter value makes this assert the exact canonical field,
    // not merely that both consumers made the same mistake.
    recorder
        .publish_note(
            accepted_exact(
                NoteEvent {
                    time: ORIGIN + 0.25,
                    source,
                    channel: 0,
                    note: 60,
                    kind: NoteEventKind::Tuning { semitones: 99.0 },
                },
                2,
                LIFETIME,
                Some(6_037_500_000),
            ),
            route,
        )
        .expect_both();
    recorder.publication_lost(ORIGIN + 0.375, route);
    let row = VoiceBaseline {
        note: 60,
        lifetime: LIFETIME,
        input_onset: ORIGIN + 0.125,
        actual_onset: ORIGIN + 0.125,
        onset: Some(EventTiming {
            clock: ClockId { runtime_session: 7, epoch: 3 },
            input: ((ORIGIN + 0.125) * 48_000.0) as i64 - 32,
            planned: None,
            sample: ((ORIGIN + 0.125) * 48_000.0) as i64,
            sample_rate: 48_000.0,
        }),
        pitch_microcents: 6_037_500_000,
        onset_pitch_microcents: 6_000_000_000,
        velocity: 0.8,
        provenance: PitchProvenance::AcceptedOutput,
        ..Default::default()
    };
    let repaired = SourceBaseline::new(source, 1, ORIGIN + 0.5, 2, true, &[row]).unwrap();
    recorder.publish_baseline(Lane::Take, &repaired, route).unwrap();
    recorder.publish_baseline(Lane::Display, &repaired, route).unwrap();
    recorder
        .publish_note(
            accepted_exact(NoteEvent::off(ORIGIN + 0.625, source, 0, 60), 3, LIFETIME, None),
            route,
        )
        .expect_both();
    let empty = SourceBaseline::new(source, 2, ORIGIN + 0.75, 3, true, &[]).unwrap();
    recorder.publish_baseline(Lane::Take, &empty, route).unwrap();
    recorder.publish_baseline(Lane::Display, &empty, route).unwrap();

    let mut direct = NoteTracker::new();
    assert_eq!(capture.display_into(&mut direct, |_, _| {}), 6);
    recorder.finish_callback();
    capture.stop();
    assert!(!recorder.is_armed());
    writer.stop();
    writer.drain(&mut capture);
    assert!(!writer.failed());
    assert!(writer.finished.is_none(), "ordinary closure does not complete configuration/source");
    recorder.configuration_pass_complete(address);
    recorder.configuration_epoch_complete(1);
    writer.drain(&mut capture);
    assert!(!writer.failed());
    assert!(writer.finished.is_none(), "configuration closure does not complete the source");
    recorder.source_pass_complete(address, ORIGIN + 0.75);
    recorder.source_epoch_complete(1, ORIGIN + 0.75);
    recorder.finish_callback();
    writer.drain(&mut capture);
    assert!(!writer.failed());
    assert_eq!(writer.finished.as_ref(), Some(&path));

    let take = Take::read(&path).unwrap();
    assert!(!take.truncated);
    assert_eq!(take.configurations.len(), 1);
    assert_eq!(take.configurations[0].t, 0.0);
    assert_eq!(take.configurations[0].resolved(), configuration);
    let event_times: Vec<_> = take.events.iter().map(|event| event.time()).collect();
    assert_eq!(event_times, [0.125, 0.25, 0.375, 0.5, 0.625, 0.75]);
    assert!(take.incomplete.is_some(), "the induced publication gap survives disk");
    let mut replay = Replay::new(take);
    let mut state = PictureState::new(TextureFormat::Bgra8Unorm);
    replay.advance_to(&mut state.runtime, 1.0);
    assert!(replay.is_spent());
    assert_eq!(state.runtime.replayed_configuration, Some(configuration));
    assert_eq!(
        roll_snapshot(&direct, ORIGIN + 1.0, ORIGIN),
        roll_snapshot(&state.runtime.tracker, 1.0, 0.0)
    );
    let actual = roll_snapshot(&state.runtime.tracker, 1.0, 0.0);
    // A later baseline repairs final pitch even if disk decoding loses exact
    // tuning. The pre-gap trajectory must retain it, and must not bridge loss.
    assert_eq!(
        actual,
        [RollSnapshot {
            source,
            lifetime: Some(LIFETIME),
            channel: 0,
            note: 60,
            start: 0.125,
            end: Some(0.625),
            observed_until: None,
            history_complete: false,
            end_pitch: 60.375,
            segments: vec![
                ((0.125, 60.0), (0.25, 60.375)),
                ((0.25, 60.375), (0.375, 60.375)),
                ((0.5, 60.375), (0.625, 60.375)),
            ],
        }]
    );
    for (tracker, origin) in [(&direct, ORIGIN), (&state.runtime.tracker, 0.0)] {
        assert_eq!(tracker.held_count(), 0);
        assert!(tracker.source_current_certain(source));
        let baseline = tracker.source_baseline(source).unwrap();
        assert_eq!(
            (
                baseline.source,
                baseline.id,
                baseline.time - origin,
                baseline.output_cut,
                baseline.participating
            ),
            (source, 2, 0.75, 3, true),
        );
        assert!(baseline.voices().is_empty());
        let gaps = tracker.publication_gaps();
        assert_eq!(gaps.len(), 1);
        assert_eq!((gaps[0].time - origin, gaps[0].through - origin), (0.375, 0.375));
        assert_eq!(gaps[0].source, None);
        assert_eq!(gaps[0].reason, harmonigraph_core::canonical::GapReason::PublicationFull);
    }

    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
