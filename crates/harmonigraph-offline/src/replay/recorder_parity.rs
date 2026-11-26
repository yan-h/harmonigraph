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

/// Deltas with no sequence, lifetime or timing, which is what a reset carries:
/// a run at offset zero holds them, and both lanes draw the same roll.
#[test]
fn unsequenced_recorder_display_and_disk_replay_share_note_semantics() {
    use harmonigraph_core::{NoteEvent, NoteEventKind, NoteTracker, SourceId};
    use harmonigraph_record::testing;

    const ORIGIN: f64 = 10.0;
    let (mut recorder, mut capture) = testing::channel();
    capture.arm();
    assert!(recorder.is_armed());
    let path = parity_path("unsequenced");
    let mut writer = testing::FileWriter::new(&capture, path.clone());
    recorder.record(0.0, Some(0.0));
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
        recorder.publish_note(live.into()).expect_both();
    }

    let mut direct = NoteTracker::new();
    assert_eq!(capture.display_into(&mut direct, |_, _| {}), live_events.len());
    capture.stop();
    assert!(!recorder.is_armed());
    recorder.run_end(ORIGIN + 1.0);
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
    replay.advance_to(&mut state.runtime, ORIGIN + 1.0);
    assert!(replay.is_spent());
    assert_eq!(
        roll_snapshot(&direct, ORIGIN + 1.0, ORIGIN),
        roll_snapshot(&state.runtime.tracker, ORIGIN + 1.0, ORIGIN)
    );
    let actual = roll_snapshot(&state.runtime.tracker, ORIGIN + 1.0, ORIGIN);
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
    use harmonigraph_core::canonical::NoteDelta;
    NoteDelta { event, sequence, lifetime, pitch_microcents, assignment: None }
}

#[test]
fn canonical_recorder_display_and_disk_replay_share_gap_repair_and_routing() {
    use harmonigraph_core::canonical::{SourceBaseline, VoiceBaseline};
    use harmonigraph_core::{NoteEvent, NoteEventKind, NoteTracker, SourceId};
    use harmonigraph_record::publication::Lane;
    use harmonigraph_record::testing;

    const ORIGIN: f64 = 10.0;
    const LIFETIME: u64 = 61;
    let source = SourceId(1);
    let (mut recorder, mut capture) = testing::channel();
    capture.arm();
    assert!(recorder.is_armed());
    let path = parity_path("canonical");
    let mut writer = testing::FileWriter::new(&capture, path.clone());
    let configuration = harmonigraph_core::configuration::ConfigReducer::default().resolved();
    // One run whose presentation ORIGIN is song zero.
    recorder.record(ORIGIN, Some(0.0));
    recorder.configuration(ORIGIN, configuration);

    recorder
        .publish_note(accepted_exact(
            NoteEvent::on(ORIGIN + 0.125, source, 0, 60, 0.8),
            1,
            LIFETIME,
            Some(6_000_000_000),
        ))
        .expect_both();
    // The absurd f32 adapter value makes this assert the exact canonical field,
    // not merely that both consumers made the same mistake.
    recorder
        .publish_note(accepted_exact(
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
        ))
        .expect_both();
    recorder.publication_lost(ORIGIN + 0.375);
    let row = VoiceBaseline {
        note: 60,
        lifetime: LIFETIME,
        actual_onset: ORIGIN + 0.125,
        pitch_microcents: 6_037_500_000,
        velocity: 0.8,
        ..Default::default()
    };
    let repaired = SourceBaseline::new(source, ORIGIN + 0.5, 2, true, &[row]).unwrap();
    recorder.publish_baseline(Lane::Take, &repaired).unwrap();
    recorder.publish_baseline(Lane::Display, &repaired).unwrap();
    recorder
        .publish_note(accepted_exact(
            NoteEvent::off(ORIGIN + 0.625, source, 0, 60),
            3,
            LIFETIME,
            None,
        ))
        .expect_both();
    let empty = SourceBaseline::new(source, ORIGIN + 0.75, 3, true, &[]).unwrap();
    recorder.publish_baseline(Lane::Take, &empty).unwrap();
    recorder.publish_baseline(Lane::Display, &empty).unwrap();

    let mut direct = NoteTracker::new();
    assert_eq!(capture.display_into(&mut direct, |_, _| {}), 6);
    capture.stop();
    assert!(!recorder.is_armed());
    recorder.run_end(ORIGIN + 1.0);
    writer.stop();
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
    // The gap draws nothing: the note is held through it as one line, and the
    // later baseline restates the pitch it holds (#1798 F1).
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
            history_complete: true,
            end_pitch: 60.375,
            segments: vec![((0.125, 60.0), (0.25, 60.375)), ((0.25, 60.375), (0.625, 60.375))],
        }]
    );
    // Nothing is left sounding on either lane.
    for tracker in [&direct, &state.runtime.tracker] {
        assert_eq!(tracker.held_count(), 0);
    }

    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}

/// Movement is evaluated on recorded block time, independent of frame cadence.
/// The renderer's appearance replacement changes style, never captured motion.
#[test]
fn camera_records_override_replacement_and_replay_at_any_frame_cadence() {
    // Use the real writer's header version rather than a hand-maintained fixture.
    let path = parity_path("camera");
    let mut writer =
        harmonigraph_take::Writer::create(&path, &harmonigraph_take::Header::default()).unwrap();
    let run = harmonigraph_take::RunRecord::default();
    writer.write(&harmonigraph_take::Record::Run(run)).unwrap();
    for (t, values) in
        [(12.0, [0.4, 0.3, 12.0, 1.0, 0.0]), (12.125, [0.8, -0.3, 8.0, 12.25, -7.375])]
    {
        for (key, value) in ParamKey::CAMERA.into_iter().zip(values) {
            writer
                .write(&harmonigraph_take::Record::Param(harmonigraph_take::ParamRecord {
                    t,
                    id: key.id().into(),
                    value,
                }))
                .unwrap();
        }
    }
    writer.flush().unwrap();
    let take = Take::read(&path).unwrap();
    for cadence in [vec![12.2], vec![12.0, 12.05, 12.1, 12.15, 12.2], vec![12.0, 12.125, 12.2]] {
        let mut replay = Replay::new(take.clone());
        let mut state = PictureState::new(TextureFormat::Bgra8Unorm);
        state.appearance.camera.distance = 20.0;
        state.appearance.camera.projection = harmonigraph_scene::Projection::Cabinet;
        for t in cadence {
            replay.advance_to(&mut state.runtime, t);
            harmonigraph_ui::begin_frame(&mut state, &replay.params, t);
        }
        assert_eq!(state.appearance.camera_movement(), [0.8, -0.3, 8.0, 12.25, -7.375]);
        assert_eq!(state.appearance.camera.projection, harmonigraph_scene::Projection::Cabinet);
    }
    let mut old = take;
    old.params.clear();
    let mut replay = Replay::new(old);
    let mut state = PictureState::new(TextureFormat::Bgra8Unorm);
    state.appearance.camera.distance = 20.0;
    replay.advance_to(&mut state.runtime, 12.2);
    harmonigraph_ui::begin_frame(&mut state, &replay.params, 12.2);
    assert_eq!(state.appearance.camera.distance, 20.0);
    std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
}
