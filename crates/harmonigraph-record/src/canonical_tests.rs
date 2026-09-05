use super::*;
use harmonigraph_core::canonical::*;
use harmonigraph_core::confirmed::PitchProvenance;

fn path(name: &str) -> std::path::PathBuf {
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-canonical-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    directory.join("capture.take")
}

fn accepted(event: NoteEvent, sequence: u64) -> NoteDelta {
    NoteDelta {
        event,
        sequence,
        lifetime: 1,
        provenance: PitchProvenance::AcceptedOutput,
        timing: Some(EventTiming {
            clock: ClockId { runtime_session: 7, epoch: 3 },
            input: (event.time * 48000.0) as i64 - 32,
            planned: None,
            sample: (event.time * 48000.0) as i64,
            sample_rate: 48000.0,
        }),
        pitch_microcents: None,
    }
}

#[test]
fn delayed_history_and_baseline_keep_original_pass_and_both_wav_tails() {
    let (mut recorder, mut capture) = testing::channel();
    recorder.enable_configuration();
    recorder.enable_canonical();
    capture.arm_audio();
    assert!(recorder.is_armed());
    let file = path("delayed");
    let mut writer = testing::FileWriter::new(
        &capture,
        file.clone(),
        Some(AudioSpec { sample_rate: 48000.0, channels: 2 }),
    );
    let first = RecordAddress { epoch: 1, pass: 1 };
    let second = RecordAddress { epoch: 1, pass: 2 };
    let config = harmonigraph_core::configuration::ConfigReducer::default().resolved();
    assert!(recorder.observe_transport(20.0, true));
    recorder.configuration_at(first, 20.0, config);
    recorder.mark_audio_start(20.0);
    recorder.audio(&mut std::iter::repeat_n(0.125, 96 * 2), 96 * 2);
    assert!(recorder.observe_transport(0.0, true));
    recorder.configuration_at(second, 0.0, config);
    recorder.mark_audio_start(0.0);
    recorder.audio(&mut std::iter::repeat_n(0.25, 32 * 2), 32 * 2);
    capture.stop();
    assert!(!recorder.is_armed());
    recorder.configuration_pass_complete(first);
    recorder.configuration_pass_complete(second);
    recorder.configuration_epoch_complete(1);
    writer.stop();
    writer.drain(&mut capture);
    assert!(writer.finished.is_none(), "producer/configuration closure cannot seal source history");
    assert!(!writer.failed());

    let source = SourceId(1);
    let events = [
        accepted(NoteEvent::on(1.0, source, 0, 60, 0.8), 1),
        accepted(
            NoteEvent {
                time: 1.25,
                source,
                channel: 0,
                note: 60,
                kind: NoteEventKind::Tuning { semitones: 0.25 },
            },
            2,
        ),
        accepted(NoteEvent::off(1.5, source, 0, 60), 3),
    ];
    let route = publication::Route { address: Some(first), time_offset: 19.0 };
    for event in events {
        recorder.publish_note(event, 10.0, route).unwrap();
    }
    let empty =
        SourceBaseline::new(source, 1, 2.0, 0.0, 3, true, &[], [ChannelBaseline::default(); 16])
            .unwrap();
    recorder.publish_baseline(1, &empty, 10.0, route).unwrap();
    // Duplicated transfer must not write an already completed lifetime twice.
    for event in events {
        recorder.publish_note(event, 10.0, route).unwrap();
    }
    recorder.publish_baseline(1, &empty, 10.0, route).unwrap();
    recorder
        .publish_note(accepted(NoteEvent::on(2.5, source, 0, 60, 0.7), 4), 10.0, route)
        .unwrap();
    // Explicit disarmed provenance remains unrecorded despite the active file.
    recorder
        .publish_note(
            NoteEvent::on(3.0, SourceId::DIRECT, 0, 90, 0.8).into(),
            10.0,
            publication::Route::default(),
        )
        .unwrap();
    writer.drain(&mut capture);
    assert!(writer.finished.is_none());
    recorder.source_pass_complete(first, 10.0);
    recorder.source_pass_complete(second, 10.0);
    recorder.source_epoch_complete(1, 10.0);
    writer.drain(&mut capture);
    assert!(!writer.failed());
    assert_eq!(
        writer.finished.as_ref(),
        Some(&file),
        "late voiced first pass is still the render target"
    );
    let take = harmonigraph_take::Take::read(&file).unwrap();
    assert_eq!(take.header.version, 4);
    assert!(take.incomplete.is_none());
    assert_eq!(take.events.len(), 5);
    assert_eq!(take.notes().map(|n| n.t).collect::<Vec<_>>(), [20.0, 20.25, 20.5, 21.5]);
    assert!(!take.notes().any(|n| n.note == 90));
    let mut tracker = harmonigraph_core::NoteTracker::new();
    for event in &take.events {
        event.apply(&mut tracker).unwrap();
    }
    assert_eq!(tracker.roll().notes().count(), 2);
    let second_file = file.with_file_name("capture-2.take");
    for (take_file, frames) in [(&file, 96), (&second_file, 32)] {
        let audio = std::fs::read(take_file.with_extension("wav")).unwrap();
        assert_eq!(u32::from_le_bytes(audio[40..44].try_into().unwrap()) as usize / 8, frames);
        assert_eq!((audio.len() - 44) / 8, frames);
    }
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

#[test]
fn real_publication_ring_loss_is_durable_after_the_last_callback() {
    let (mut recorder, mut capture) = testing::channel();
    recorder.enable_configuration();
    recorder.enable_canonical();
    capture.arm();
    recorder.is_armed();
    let file = path("full");
    let mut writer = testing::FileWriter::new(&capture, file.clone(), None);
    let address = RecordAddress { epoch: 1, pass: 1 };
    let route = publication::Route { address: Some(address), time_offset: 0.0 };
    for i in 0..publication::PUBLICATION_RING {
        recorder
            .publish_note(
                NoteEvent::on(i as f64 / 48000.0, SourceId::DIRECT, 0, 60, 0.8).into(),
                1.0,
                route,
            )
            .unwrap();
    }
    assert_eq!(
        recorder.publish_note(NoteEvent::off(1.0, SourceId::DIRECT, 0, 60).into(), 1.0, route),
        Err(publication::PublishError::Lost)
    );
    // Source and musical state never require this writer to acknowledge output.
    // There are deliberately no further recorder/audio calls to deliver a gap.
    drop(recorder);
    writer.stop();
    writer.drain(&mut capture);
    assert!(writer.failed());
    assert!(writer.finished.is_none());
    let take = harmonigraph_take::Take::read(&file).unwrap();
    assert!(!take.truncated);
    let incomplete = take.incomplete.unwrap();
    assert_eq!((incomplete.first_publication, incomplete.last_publication), (4097, 4097));
    assert!(matches!(take.events.last(), Some(harmonigraph_take::CanonicalRecord::Gap(_))));
    assert_eq!(take.notes().count(), publication::PUBLICATION_RING);
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

#[test]
fn all_128_passes_need_source_closure_before_the_129th_file() {
    for close_source in [false, true] {
        let (mut recorder, mut capture) = testing::channel();
        recorder.enable_configuration();
        recorder.enable_canonical();
        capture.arm();
        recorder.is_armed();
        let file = path(if close_source { "pass-reuse" } else { "pass-full" });
        let mut writer = testing::FileWriter::new(&capture, file.clone(), None);
        recorder.observe_transport(10.0, true);
        for _ in 1..RECORD_PASSES {
            recorder.observe_transport(0.0, true);
            recorder.observe_transport(10.0, true);
        }
        for pass in 1..=RECORD_PASSES as u32 {
            recorder.configuration_pass_complete(RecordAddress { epoch: 1, pass });
        }
        writer.drain(&mut capture);
        assert_eq!(writer.retained_passes(), RECORD_PASSES - 1);
        assert!(!writer.failed());
        if close_source {
            recorder.source_pass_complete(RecordAddress { epoch: 1, pass: 1 }, 10.0);
        }
        // Both lanes are queued before the worker runs. It must consume the
        // available source closure before judging a 129th required file.
        recorder.observe_transport(0.0, true);
        writer.drain(&mut capture);
        assert_eq!(writer.failed(), !close_source);
        if close_source {
            assert_eq!(writer.current_pass(), Some(129));
            assert_eq!(writer.retained_passes(), 127);
        } else {
            assert!(harmonigraph_take::Take::read(&file).unwrap().incomplete.is_some());
        }
        drop(writer);
        std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }
}
