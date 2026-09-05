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
            // Refusal terminates recording ownership, not display publication.
            // Three successive uses of one two-slot bank prove reclamation.
            for id in 1..=3 {
                let baseline = SourceBaseline::new(
                    SourceId::DIRECT,
                    id,
                    11.0,
                    0.0,
                    0,
                    true,
                    &[],
                    [ChannelBaseline::default(); 16],
                )
                .unwrap();
                let route = publication::Route {
                    address: Some(RecordAddress { epoch: 1, pass: 129 }),
                    time_offset: 0.0,
                };
                recorder.publish_baseline(0, &baseline, 11.0, route).unwrap();
                recorder
                    .publish_note(
                        NoteEvent::on(11.0, SourceId::DIRECT, 0, 60, 0.8).into(),
                        11.0,
                        route,
                    )
                    .unwrap();
                writer.drain(&mut capture);
                let displayed = writer.display_events();
                assert_eq!(displayed.len(), 2);
                assert!(
                    matches!(&displayed[0], harmonigraph_take::CanonicalRecord::Baseline(frame) if frame.id == id)
                );
                assert!(displayed[1].note().is_some());
            }
        }
        drop(writer);
        std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }
}

fn wait_for(flag: &AtomicBool) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !flag.load(Ordering::Acquire) && std::time::Instant::now() < until {
        std::thread::yield_now();
    }
    assert!(flag.load(Ordering::Acquire), "real recording worker did not reach deterministic seam");
}

struct WorkerPause(Arc<RecordFence>);
impl Drop for WorkerPause {
    fn drop(&mut self) {
        self.0.worker_after_empty.enabled.store(false, Ordering::Release);
        self.0.worker_after_stop.enabled.store(false, Ordering::Release);
    }
}
fn worker_take(directory: &std::path::Path) -> std::path::PathBuf {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "take"))
        .expect("actual worker created take")
}

#[test]
fn real_worker_materializes_pending_start_before_accounting_publication_loss() {
    for stop_after_failure in [false, true] {
        let directory = path(if stop_after_failure { "worker-start-stop" } else { "worker-start" })
            .parent()
            .unwrap()
            .to_path_buf();
        let (mut recorder, control) = channel();
        recorder.enable_configuration();
        recorder.enable_canonical();
        *control.fence.test_directory.lock() = Some(directory.clone());
        let fence = control.fence.clone();
        let _resume_on_panic = WorkerPause(fence.clone());
        fence.worker_after_empty.enabled.store(true, Ordering::Release);
        wait_for(&fence.worker_after_empty.entered);
        control.start(48000.0, String::new(), false);
        assert!(recorder.is_armed());
        let address = RecordAddress { epoch: 1, pass: 1 };
        recorder.configuration_at(
            address,
            0.0,
            harmonigraph_core::configuration::ConfigReducer::default().resolved(),
        );
        let bank_released = recorder.publication.bank_observer();
        for id in 1..=2 {
            let baseline = SourceBaseline::new(
                SourceId::DIRECT,
                id,
                0.0,
                0.0,
                0,
                true,
                &[],
                [ChannelBaseline::default(); 16],
            )
            .unwrap();
            recorder
                .publish_baseline(
                    0,
                    &baseline,
                    1.0,
                    publication::Route { address: Some(address), time_offset: 0.0 },
                )
                .unwrap();
        }
        for i in 0..publication::PUBLICATION_RING - 2 {
            recorder
                .publish_note(
                    NoteEvent::on(i as f64 / 48000.0, SourceId::DIRECT, 0, 60, 0.8).into(),
                    1.0,
                    publication::Route { address: Some(address), time_offset: 0.0 },
                )
                .unwrap();
        }
        assert_eq!(
            recorder.publish_note(
                NoteEvent::off(1.0, SourceId::DIRECT, 0, 60).into(),
                1.0,
                publication::Route { address: Some(address), time_offset: 0.0 }
            ),
            Err(publication::PublishError::Lost)
        );
        if stop_after_failure {
            fence.worker_after_empty.enabled.store(false, Ordering::Release);
            wait_for(&fence.worker_failure_accounted);
            fence.worker_after_stop.enabled.store(true, Ordering::Release);
            control.stop(RenderRequest::from_config(&harmonigraph_take::RenderConfig::default()));
            wait_for(&fence.worker_after_stop.entered);
            fence.worker_after_empty.entered.store(false, Ordering::Release);
            fence.worker_after_empty.enabled.store(true, Ordering::Release);
            fence.worker_after_stop.enabled.store(false, Ordering::Release);
            wait_for(&fence.worker_after_empty.entered);
            assert!(
                !fence.finishing.load(Ordering::Acquire),
                "accounted failed Stop is terminal without another callback or disconnect"
            );
            assert_eq!(*control.status.lock(), CONFIGURATION_FAILURE);
            assert!(!control.is_recording());
            assert!(control.last_take.lock().is_none());
            assert_eq!(control.progress.in_flight.load(Ordering::Acquire), 0);
        }
        drop(recorder); // No rescue callback or producer operation follows.
        drop(control);
        fence.worker_after_empty.enabled.store(false, Ordering::Release);
        wait_for(&fence.worker_finished);
        let take = harmonigraph_take::Take::read(worker_take(&directory)).unwrap();
        assert_eq!(
            take.notes().count(),
            4094,
            "pending Start still owns the complete successful prefix"
        );
        assert_eq!(
            take.events
                .iter()
                .filter(|record| matches!(record, harmonigraph_take::CanonicalRecord::Baseline(_)))
                .count(),
            2
        );
        assert!(
            bank_released(),
            "both primary baseline payloads returned after disk/display copies"
        );
        assert_eq!(
            take.configurations.len(),
            1,
            "ordinary lane also waits for Start after failure"
        );
        let loss = take.incomplete.unwrap();
        assert_eq!((loss.first_publication, loss.last_publication), (4097, 4097));
        assert!(matches!(take.events.last(), Some(harmonigraph_take::CanonicalRecord::Gap(_))));
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn real_worker_disconnect_finishes_the_stop_after_its_last_source_closure() {
    let directory = path("worker-stop").parent().unwrap().to_path_buf();
    let (mut recorder, control) = channel();
    recorder.enable_configuration();
    recorder.enable_canonical();
    *control.fence.test_directory.lock() = Some(directory.clone());
    let fence = control.fence.clone();
    let last_take = control.last_take.clone();
    let _resume_on_panic = WorkerPause(fence.clone());
    control.start(48000.0, String::new(), false);
    assert!(recorder.is_armed());
    let address = RecordAddress { epoch: 1, pass: 1 };
    recorder.configuration_at(
        address,
        0.0,
        harmonigraph_core::configuration::ConfigReducer::default().resolved(),
    );
    fence.worker_after_stop.enabled.store(true, Ordering::Release);
    control.stop(None);
    assert!(!recorder.is_armed());
    recorder.configuration_pass_complete(address);
    recorder.configuration_epoch_complete(1);
    wait_for(&fence.worker_after_stop.entered);
    let route = publication::Route { address: Some(address), time_offset: 0.0 };
    recorder
        .publish_note(accepted(NoteEvent::on(0.01, SourceId(1), 0, 60, 0.8), 1), 1.0, route)
        .unwrap();
    recorder
        .publish_note(accepted(NoteEvent::off(0.02, SourceId(1), 0, 60), 2), 1.0, route)
        .unwrap();
    recorder.source_pass_complete(address, 1.0);
    recorder.source_epoch_complete(1, 1.0);
    drop(recorder);
    drop(control); // The next real worker poll observes Disconnected.
    fence.worker_after_stop.enabled.store(false, Ordering::Release);
    wait_for(&fence.worker_finished);
    let file = worker_take(&directory);
    let take = harmonigraph_take::Take::read(&file).unwrap();
    assert!(
        take.incomplete.is_none(),
        "all three publication closures completed before disconnect"
    );
    assert_eq!(take.notes().count(), 2);
    assert_eq!(*last_take.lock(), Some(file));
    assert!(!fence.failed.load(Ordering::Acquire));
    std::fs::remove_dir_all(directory).unwrap();
}
