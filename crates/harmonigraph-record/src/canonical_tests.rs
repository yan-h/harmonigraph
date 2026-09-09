use super::*;
use harmonigraph_core::canonical::*;
use harmonigraph_core::confirmed::PitchProvenance;
use publication::Lane;

fn path(name: &str) -> std::path::PathBuf {
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-canonical-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    directory.join("capture.take")
}

fn accepted(event: NoteEvent, sequence: u64) -> NoteDelta {
    NoteDelta {
        assignment: None,
        partial_output: false,
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
        recorder.publish_note(event, route).expect_both();
    }
    let empty = SourceBaseline::new(source, 1, 2.0, 3, true, &[]).unwrap();
    recorder.publish_baseline(Lane::Take, &empty, route).unwrap();
    // Duplicated transfer must not write an already completed lifetime twice.
    for event in events {
        recorder.publish_note(event, route).expect_both();
    }
    recorder.publish_baseline(Lane::Take, &empty, route).unwrap();
    recorder.publish_note(accepted(NoteEvent::on(2.5, source, 0, 60, 0.7), 4), route).expect_both();
    // Explicit disarmed provenance remains unrecorded despite the active file.
    recorder
        .publish_note(
            NoteEvent::on(3.0, SourceId::DIRECT, 0, 90, 0.8).into(),
            publication::Route::default(),
        )
        .expect_both();
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
    assert_eq!(take.header.version, harmonigraph_take::FORMAT_VERSION);
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
    // One short of the ring: the last cell is reserved so the gap below has
    // somewhere to go without another callback, which is this test's subject.
    for i in 0..publication::PUBLICATION_RING - 1 {
        recorder
            .publish_note(
                NoteEvent::on(i as f64 / 48000.0, SourceId::DIRECT, 0, 60, 0.8).into(),
                route,
            )
            .expect_both();
    }
    assert_eq!(
        recorder.publication_free(),
        publication::Lanes::both(0),
        "the fixture must actually fill both lanes"
    );
    assert_eq!(
        recorder.publish_note(NoteEvent::off(1.0, SourceId::DIRECT, 0, 60).into(), route),
        publication::Lanes::both(Err(publication::PublishError::Lost))
    );
    // Source and musical state never require this writer to acknowledge output.
    // There are deliberately no further recorder/audio calls to deliver a gap:
    // this drain is the only one, and it is what makes the loss durable.
    writer.drain(&mut capture);
    // Changed with #712's export decision: the gap used to fail the take here.
    // It marks it instead, which is what the file below carries and what the
    // export warns from. Asserted BEFORE the drop, because the drop is a
    // separate failure and would hide this one.
    assert!(!writer.failed(), "a hole in the note history is not a recording failure");
    drop(recorder);
    writer.stop();
    writer.drain(&mut capture);
    // A producer that goes away with the take still open IS an ownership
    // failure, and still refuses — which is why nothing finishes here.
    assert!(writer.failed());
    assert!(writer.finished.is_none());
    let take = harmonigraph_take::Take::read(&file).unwrap();
    assert!(!take.truncated);
    let incomplete = take.incomplete.unwrap();
    assert_eq!((incomplete.first_publication, incomplete.last_publication), (4096, 4096));
    assert!(matches!(take.events.last(), Some(harmonigraph_take::CanonicalRecord::Gap(_))));
    assert_eq!(take.notes().count(), publication::PUBLICATION_RING - 1);
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
            for id in 1..=3 {
                let baseline =
                    SourceBaseline::new(SourceId::DIRECT, id, 11.0, 0, true, &[]).unwrap();
                let route = publication::Route {
                    address: Some(RecordAddress { epoch: 1, pass: 129 }),
                    time_offset: 0.0,
                };
                recorder.publish_baseline(Lane::Take, &baseline, route).unwrap();
                recorder.publish_baseline(Lane::Display, &baseline, route).unwrap();
                recorder
                    .publish_note(NoteEvent::on(11.0, SourceId::DIRECT, 0, 60, 0.8).into(), route)
                    .expect_both();
                writer.drain(&mut capture);
                let displayed = capture.display_events();
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

/// The prefix a pending `Start` still owns, and — in the second case — what a
/// Stop does once the recording really has failed.
///
/// The overflow no longer supplies that failure: #712 makes a hole in the note
/// history a marker on the file rather than a refusal, so the second case names
/// its own ownership failure. See
/// `an_overflowed_take_finalises_and_launches_the_render_it_was_stopped_with`
/// for what the overflow alone now does.
#[test]
fn real_worker_materializes_pending_start_before_accounting_a_recording_failure() {
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
        for id in 1..=2 {
            let baseline = SourceBaseline::new(SourceId::DIRECT, id, 0.0, 0, true, &[]).unwrap();
            recorder
                .publish_baseline(
                    Lane::Take,
                    &baseline,
                    publication::Route { address: Some(address), time_offset: 0.0 },
                )
                .unwrap();
        }
        for i in 0..publication::PUBLICATION_RING - 3 {
            recorder
                .publish_note(
                    NoteEvent::on(i as f64 / 48000.0, SourceId::DIRECT, 0, 60, 0.8).into(),
                    publication::Route { address: Some(address), time_offset: 0.0 },
                )
                .expect_both();
        }
        assert_eq!(
            recorder
                .publish_note(
                    NoteEvent::off(1.0, SourceId::DIRECT, 0, 60).into(),
                    publication::Route { address: Some(address), time_offset: 0.0 }
                )
                .take,
            Err(publication::PublishError::Lost)
        );
        if stop_after_failure {
            recorder.fail_configuration();
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
            4093,
            "pending Start still owns the complete successful prefix"
        );
        assert_eq!(
            take.events
                .iter()
                .filter(|record| matches!(record, harmonigraph_take::CanonicalRecord::Baseline(_)))
                .count(),
            2
        );
        assert_eq!(
            take.configurations.len(),
            1,
            "ordinary lane also waits for Start after failure"
        );
        let loss = take.incomplete.unwrap();
        assert_eq!((loss.first_publication, loss.last_publication), (4096, 4096));
        assert!(matches!(take.events.last(), Some(harmonigraph_take::CanonicalRecord::Gap(_))));
        std::fs::remove_dir_all(directory).unwrap();
    }
}

/// #712, finding 1: a take-lane overflow used to set `fence.failed`, so the
/// worker accounted a failure, threw the pending Stop away and never moved
/// `last_take`. Stop-and-render then had nothing to render, or rendered an
/// EARLIER take — the permissive renderer 5B built for exactly this file was
/// never reached.
///
/// The real worker, a real 4,096-cell overflow, and the real Stop the Video
/// pane sends. `renderer_path` points at nothing, so the launch is observable
/// (and instant) through the status line it leaves rather than by running a
/// GPU render: the message only exists if `spawn_render` was reached at all,
/// which is the assertion the old behaviour failed. That the resulting file
/// warns rather than refuses is
/// `a_take_missing_note_history_is_exported_with_a_warning` in
/// harmonigraph-offline; this is the half that gets it there.
#[test]
fn an_overflowed_take_finalises_and_launches_the_render_it_was_stopped_with() {
    let directory = path("worker-overflow-render").parent().unwrap().to_path_buf();
    let (mut recorder, control) = channel();
    recorder.enable_configuration();
    recorder.enable_canonical();
    *control.fence.test_directory.lock() = Some(directory.clone());
    let fence = control.fence.clone();
    let _resume_on_panic = WorkerPause(fence.clone());
    control.start(48000.0, String::new(), false);
    assert!(recorder.is_armed());
    let address = RecordAddress { epoch: 1, pass: 1 };
    let route = publication::Route { address: Some(address), time_offset: 0.0 };
    recorder.configuration_at(
        address,
        0.0,
        harmonigraph_core::configuration::ConfigReducer::default().resolved(),
    );
    // Published until the lane actually loses one rather than for a fixed count:
    // the REAL worker is draining concurrently, so how many cells it takes to
    // overflow is not a number this test may assume.
    let mut published = 0;
    let mut lost = false;
    for i in 0..publication::PUBLICATION_RING * 4 {
        let event = NoteEvent::on(i as f64 / 48000.0, SourceId::DIRECT, 0, 60, 0.8);
        match recorder.publish_note(event.into(), route).take {
            Ok(()) => published += 1,
            Err(publication::PublishError::Lost) => {
                lost = true;
                break;
            }
            other => panic!("unexpected publication outcome {other:?}"),
        }
    }
    assert!(lost, "the fixture must actually lose a report");
    assert!(published >= publication::PUBLICATION_RING - 1, "{published} reports were accepted");
    // The worker drains it; the pass closures below need cells, and losing one
    // of THOSE is an ownership failure that still refuses (see
    // `full_primary_publication_...` in harmonigraph-plugin).
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while recorder.publication_free().take < publication::PUBLICATION_RING - 1
        && std::time::Instant::now() < until
    {
        std::thread::yield_now();
    }
    assert!(!fence.failed.load(Ordering::Acquire), "the overflow alone must not fail the take");

    let config = harmonigraph_take::RenderConfig {
        renderer_path: directory.join("no-such-renderer").display().to_string(),
        ..Default::default()
    };
    control.stop(RenderRequest::from_config(&config));
    assert!(!recorder.is_armed());
    recorder.configuration_pass_complete(address);
    recorder.configuration_epoch_complete(1);
    recorder.source_pass_complete(address, 1.0);
    recorder.source_epoch_complete(1, 1.0);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while control.last_take().is_none() && std::time::Instant::now() < until {
        std::thread::yield_now();
    }
    let finished = control.last_take().expect("the overflowed take still finalises");
    assert!(!fence.failed.load(Ordering::Acquire));
    assert_eq!(finished, worker_take(&directory), "Render targets THIS take, not an earlier one");
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !control.status().contains("could not run") && std::time::Instant::now() < until {
        std::thread::yield_now();
    }
    assert!(
        control.status().contains("could not run"),
        "Stop-and-render reached the renderer: {}",
        control.status()
    );
    let take = harmonigraph_take::Take::read(&finished).unwrap();
    let loss = take.incomplete.expect("the missing history is on the file, not silent");
    assert_eq!(
        loss.first_publication, loss.last_publication,
        "exactly the one report that was lost"
    );
    assert_eq!(take.notes().count(), published, "and every surviving report is kept");
    drop(recorder);
    drop(control);
    fence.worker_after_empty.enabled.store(false, Ordering::Release);
    wait_for(&fence.worker_finished);
    std::fs::remove_dir_all(directory).unwrap();
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
        .publish_note(accepted(NoteEvent::on(0.01, SourceId(1), 0, 60, 0.8), 1), route)
        .expect_both();
    recorder
        .publish_note(accepted(NoteEvent::off(0.02, SourceId(1), 0, 60), 2), route)
        .expect_both();
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

#[test]
fn retired_producer_keeps_real_writer_alive_after_every_ui_control_is_dropped() {
    let directory = path("retired-producer").parent().unwrap().to_path_buf();
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
    fence.worker_after_stop.enabled.store(true, Ordering::Release);
    control.stop(None);
    assert!(!recorder.is_armed());
    recorder.configuration_pass_complete(address);
    recorder.configuration_epoch_complete(1);
    drop(control); // No UI, Control or external command sender survives.
    fence.worker_after_empty.enabled.store(false, Ordering::Release);
    wait_for(&fence.worker_after_stop.entered);
    fence.worker_after_empty.entered.store(false, Ordering::Release);
    fence.worker_after_empty.enabled.store(true, Ordering::Release);
    fence.worker_after_stop.enabled.store(false, Ordering::Release);
    wait_for(&fence.worker_after_empty.entered);
    assert!(!fence.worker_finished.load(Ordering::Acquire));
    let route = publication::Route { address: Some(address), time_offset: 0.0 };
    recorder
        .publish_note(accepted(NoteEvent::on(0.01, SourceId(1), 0, 60, 0.8), 1), route)
        .expect_both();
    recorder
        .publish_note(accepted(NoteEvent::off(0.02, SourceId(1), 0, 60), 2), route)
        .expect_both();
    recorder.source_pass_complete(address, 1.0);
    recorder.source_epoch_complete(1, 1.0);
    drop(recorder);
    fence.worker_after_empty.enabled.store(false, Ordering::Release);
    wait_for(&fence.worker_finished);
    let take = harmonigraph_take::Take::read(worker_take(&directory)).unwrap();
    assert!(take.incomplete.is_none());
    assert_eq!(take.notes().count(), 2);
    std::fs::remove_dir_all(directory).unwrap();
}

/// #712, the stage-5 review's remaining finding: a gap can reach the writer
/// before the file it belongs to exists, and its marker had nowhere to land.
///
/// The interleaving is a genuine race in production. A merged gap only reaches
/// the ring once a drain running CONCURRENTLY with the audio thread has freed
/// cells, and the worker polls `Start` outside that drain — so the same drain
/// consumes the gap with no file open, and `Lost` no longer fails the fence to
/// make the loss visible some other way. The drain is therefore done by hand
/// here, one item consumed and the next declined, which leaves exactly the two
/// free cells the racing worker leaves behind.
///
/// What the serial range proves is that the fixture reaches the case rather
/// than something next to it. `4097` is the ARMED publication, so the outage
/// starts on history the take about to open owns; merging a later disarmed
/// loss into it is what strips its address, and only an address-less gap goes
/// past the wait for a file. Keep the address instead and the merged outage
/// waits for its file like any addressed record, landing as an ordinary `Gap`
/// in the pass: the range reads `4097..=4098`, and the only thing the unplaced
/// path ever saw was the disarmed gap ahead of it.
#[test]
fn a_gap_with_no_file_open_marks_the_take_that_opens_after_it() {
    let (mut publisher, mut consumer) = publication::channel();
    let disarmed = publication::Route::default();
    let armed =
        publication::Route { address: Some(RecordAddress { epoch: 1, pass: 1 }), time_offset: 0.0 };
    let sounding =
        |i: usize| NoteEvent::on(i as f64 / 48000.0, SourceId::DIRECT, 0, 60, 0.8).into();
    for i in 0..publication::PUBLICATION_RING - 1 {
        publisher.note(sounding(i), disarmed).unwrap();
    }
    // The reserve takes the first outage. The second is ARMED history, and it
    // has nowhere to go: it is held on the audio thread until a cell frees.
    assert_eq!(publisher.note(sounding(0), disarmed), Err(publication::PublishError::Lost));
    assert_eq!(publisher.note(sounding(1), armed), Err(publication::PublishError::Lost));
    // One item consumed and the next declined — the two free cells a drain
    // running against the audio thread leaves behind mid-call.
    let mut seen = 0;
    consumer.drain(|_, _| {
        seen += 1;
        seen < 2
    });
    // A third loss, disarmed, merges into the held one. The two routes differ,
    // so the merged outage keeps NEITHER address — and that is what sends it
    // past the wait for an addressed file below.
    assert_eq!(publisher.note(sounding(2), disarmed), Err(publication::PublishError::Lost));

    let mut fanout = CanonicalFanout::default();
    let fence = RecordFence::default();
    let failure = FailureAccount::default();
    let file = path("unplaced-gap");
    // `Start` is queued and unpolled, so the writer holds no file at all.
    let mut nothing_open: Option<Open> = None;
    fanout.drain(&mut consumer, &mut nothing_open, &fence, &failure);
    assert!(
        !fence.failed.load(Ordering::Acquire),
        "a hole in the note history still does not refuse the take"
    );

    let status = Mutex::new(String::new());
    let mut opened =
        Open::create(harmonigraph_take::Header::default(), file.clone(), 1, None, &status).unwrap();
    opened.epoch = 1;
    let mut open = Some(opened);
    fanout.drain(&mut consumer, &mut open, &fence, &failure);
    let sealed = open.take().unwrap().finish().unwrap();
    let take = harmonigraph_take::Take::read(&sealed).unwrap();
    let loss =
        take.incomplete.expect("the gap that had no file marks the one that opened after it");
    assert_eq!(loss.reason, harmonigraph_take::canonical::GapReasonRecord::PublicationFull);
    assert_eq!(
        (loss.first_publication, loss.last_publication),
        (4096, 4098),
        "coalesced over both outages, and 4097 inside it is the armed publication"
    );
    assert!(!fence.failed.load(Ordering::Acquire));
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

/// The same repair through the REAL worker, and the end that matters: the
/// sealed take carries the warning.
///
/// `worker_after_empty` parks the worker inside the arm that found no command,
/// so everything published below reaches the ring before the loop's own drain
/// runs, and the `Start` queued below is not polled until the iteration after
/// it. The gap is drained against no file exactly as the racing case drains
/// it, and the take opened one iteration later is what has to carry it.
///
/// The gap here spans only the disarmed prefix, which is this repair's STATED
/// RESIDUAL rather than its motivating case: the take warns about a hole that
/// is not in its own history. Reaching the merged armed gap needs the
/// concurrent drain that
/// `a_gap_with_no_file_open_marks_the_take_that_opens_after_it` builds by hand.
#[test]
fn a_real_worker_carries_a_gap_it_drained_before_start_onto_the_take() {
    let directory = path("worker-gap-before-start").parent().unwrap().to_path_buf();
    let (mut recorder, control) = channel();
    recorder.enable_configuration();
    recorder.enable_canonical();
    *control.fence.test_directory.lock() = Some(directory.clone());
    let fence = control.fence.clone();
    let _resume_on_panic = WorkerPause(fence.clone());
    fence.worker_after_empty.enabled.store(true, Ordering::Release);
    wait_for(&fence.worker_after_empty.entered);
    let sounding =
        |i: usize| NoteEvent::on(i as f64 / 48000.0, SourceId::DIRECT, 0, 60, 0.8).into();
    // Only the take lane is under test, so only its outcome is asserted; the
    // editor's lane fills alongside it and is drained back to empty below.
    let mut displayed = control.take_display().expect("the display lane's consumer");
    for i in 0..publication::PUBLICATION_RING - 1 {
        recorder.publish_note(sounding(i), publication::Route::default()).take.unwrap();
    }
    assert_eq!(
        recorder.publish_note(sounding(0), publication::Route::default()).take,
        Err(publication::PublishError::Lost),
        "the fixture must actually lose a report"
    );
    displayed.drain(|_, _| true);
    let address = RecordAddress { epoch: 1, pass: 1 };
    control.start(48000.0, String::new(), false);
    assert!(recorder.is_armed());
    // Released into the drain that has no file, then into the poll that opens
    // one — in that order, because the pause is inside the arm that found no
    // command and the loop drains before it polls again. `recording to` is the
    // status only `Open::create` writes, so waiting for it waits for both.
    fence.worker_after_empty.enabled.store(false, Ordering::Release);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !control.status().starts_with("recording to") && std::time::Instant::now() < until {
        std::thread::yield_now();
    }
    assert!(control.status().starts_with("recording to"), "{}", control.status());
    recorder.configuration_at(
        address,
        0.0,
        harmonigraph_core::configuration::ConfigReducer::default().resolved(),
    );
    let route = publication::Route { address: Some(address), time_offset: 0.0 };
    recorder
        .publish_note(accepted(NoteEvent::on(0.01, SourceId(1), 0, 60, 0.8), 1), route)
        .expect_both();
    recorder
        .publish_note(accepted(NoteEvent::off(0.02, SourceId(1), 0, 60), 2), route)
        .expect_both();
    control.stop(None);
    assert!(!recorder.is_armed(), "the disarm boundary closes the producer's prefix");
    recorder.configuration_pass_complete(address);
    recorder.configuration_epoch_complete(1);
    recorder.source_pass_complete(address, 1.0);
    recorder.source_epoch_complete(1, 1.0);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while control.last_take().is_none() && std::time::Instant::now() < until {
        std::thread::yield_now();
    }
    let finished = control.last_take().expect("the take still finalises");
    assert!(!fence.failed.load(Ordering::Acquire), "and still is not a refusal");
    let take = harmonigraph_take::Take::read(&finished).unwrap();
    assert!(
        take.incomplete.is_some(),
        "the gap the worker drained before this file existed is on it, not lost"
    );
    assert_eq!(take.notes().count(), 2, "and the take it opened is otherwise whole");
    drop(recorder);
    drop(control);
    wait_for(&fence.worker_finished);
    std::fs::remove_dir_all(directory).unwrap();
}

/// #712, the stage-5 review's third round: a gap outliving the pass it landed
/// on, and the file that exports being one opened after it.
///
/// The interleaving the reviewer named, built here in the order the worker
/// runs it. `Start` is unpolled and `NewPass` is already queued, so the drain
/// that consumes the gap has no file — the round-2 case — and the ONE that
/// follows opens pass 1, marks it, and rolls straight over to pass 2 without
/// leaving the loop: `drain_with_boundaries` calls the fanout from
/// `before_new_pass`, ahead of `Open::next_pass`. Pass 2 is where the music
/// is, so pass 2 is what `Open::take_path` seals and hands to the renderer,
/// and pass 1 is not even retained by then.
///
/// What the range proves is that the fixture reaches the case rather than one
/// beside it: `4097` is the publication routed to PASS 2, and it is inside the
/// coalesced `4096..=4098` the take carries. So the marker on the exported
/// file is describing history that file owns, not only the disarmed prefix
/// ahead of it. Both closures are published and accepted, so nothing here is
/// standing in for a take that refused.
#[test]
fn a_gap_that_outlived_the_pass_it_marked_is_on_the_pass_that_exports() {
    let (mut publisher, mut consumer) = publication::channel();
    let disarmed = publication::Route::default();
    let first_pass = RecordAddress { epoch: 1, pass: 1 };
    let second_pass = RecordAddress { epoch: 1, pass: 2 };
    let routed_to_second = publication::Route { address: Some(second_pass), time_offset: 0.0 };
    let sounding =
        |i: usize| NoteEvent::on(i as f64 / 48000.0, SourceId::DIRECT, 0, 60, 0.8).into();
    for i in 0..publication::PUBLICATION_RING - 1 {
        publisher.note(sounding(i), disarmed).unwrap();
    }
    // The reserve takes the first outage. The second is history PASS 2 owns,
    // and it is held on the audio thread until a cell frees.
    assert_eq!(publisher.note(sounding(0), disarmed), Err(publication::PublishError::Lost));
    assert_eq!(publisher.note(sounding(1), routed_to_second), Err(publication::PublishError::Lost));
    let mut seen = 0;
    consumer.drain(|_, _| {
        seen += 1;
        seen < 2
    });
    // A third loss merges into the held one. The routes differ, so the merged
    // outage keeps NEITHER address and covers the pass-2 publication inside it.
    assert_eq!(publisher.note(sounding(2), disarmed), Err(publication::PublishError::Lost));

    let (mut entries, mut queued) = rtrb::RingBuffer::new(TAKE_RING_CAPACITY);
    entries.push(Entry::NewPass).expect("ring has room");
    let fence = RecordFence::default();
    fence.enabled.store(true, Ordering::Release);
    let failure = FailureAccount::default();
    let status = Mutex::new(String::new());
    let mut fanout = CanonicalFanout::default();
    let mut open: Option<Open> = None;
    let pump = |open: &mut Option<Open>,
                entries: &mut rtrb::Consumer<Entry>,
                consumer: &mut publication::Consumer,
                fanout: &mut CanonicalFanout| {
        let had =
            drain_with_boundaries(entries, None, open, &status, Some(&fence), &failure, |o| {
                fanout.drain(consumer, o, &fence, &failure);
            });
        fanout.drain(consumer, open, &fence, &failure);
        had
    };
    // `Start` is queued and unpolled. The entry ring is not even read while an
    // armed fence has no file, so `NewPass` is still waiting afterwards.
    assert!(!pump(&mut open, &mut queued, &mut consumer, &mut fanout));

    let file = path("rollover-gap");
    let mut opened =
        Open::create(harmonigraph_take::Header::default(), file.clone(), 1, None, &status).unwrap();
    opened.epoch = 1;
    opened.source_enabled = true;
    open = Some(opened);
    assert!(pump(&mut open, &mut queued, &mut consumer, &mut fanout));
    assert_eq!(open.as_ref().unwrap().pass, 2, "the queued NewPass rolled the recording over");

    // Pass 2 is the voiced one, and both passes close cleanly.
    publisher
        .note(accepted(NoteEvent::on(0.5, SourceId(1), 0, 60, 0.8), 1), routed_to_second)
        .expect("the lane drained, so the take's own note fits");
    publisher.pass_complete(first_pass, 1.0).expect("pass 1 closes");
    publisher.pass_complete(second_pass, 2.0).expect("pass 2 closes");
    publisher.epoch_complete(1, 2.0).expect("the epoch closes");
    entries.push(Entry::ConfigurationPassComplete(first_pass)).expect("ring has room");
    entries.push(Entry::ConfigurationPassComplete(second_pass)).expect("ring has room");
    entries.push(Entry::ProducerClosed(1)).expect("ring has room");
    entries.push(Entry::ConfigurationEpochComplete(1)).expect("ring has room");
    assert!(pump(&mut open, &mut queued, &mut consumer, &mut fanout));
    assert!(open.as_ref().unwrap().voiced, "pass 2 is the one holding the music");
    assert!(open.as_ref().unwrap().retained.is_empty(), "and pass 1 has already been sealed");

    let sealed = finish_ready(&mut open, 1, &fence).expect("the take seals");
    assert_eq!(
        sealed.file_name().unwrap(),
        std::path::Path::new("capture-2.take"),
        "the voiced later pass is what export selects"
    );
    let take = harmonigraph_take::Take::read(&sealed).unwrap();
    let loss = take.incomplete.expect("the gap the earlier pass took is on the pass that exports");
    assert_eq!(
        (loss.first_publication, loss.last_publication),
        (4096, 4098),
        "coalesced over both outages, and 4097 inside it is routed to pass 2"
    );
    assert!(!fence.failed.load(Ordering::Acquire), "a hole warns, it does not refuse");
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

#[test]
fn a_marker_flush_failure_refuses_stop_and_render() {
    let directory = path("marker-flush-failure").parent().unwrap().to_path_buf();
    let (mut recorder, control) = channel();
    recorder.enable_configuration();
    recorder.enable_canonical();
    let fence = control.fence.clone();
    *fence.test_directory.lock() = Some(directory.clone());
    *fence.test_marker_failure.lock() = Some(1);
    let _resume_on_panic = WorkerPause(fence.clone());
    fence.worker_after_empty.enabled.store(true, Ordering::Release);
    wait_for(&fence.worker_after_empty.entered);
    control.start(48000.0, String::new(), false);
    assert!(recorder.is_armed());
    let address = RecordAddress { epoch: 1, pass: 1 };
    let route = publication::Route { address: Some(address), time_offset: 0.0 };
    for i in 0..publication::PUBLICATION_RING - 1 {
        recorder
            .publish_note(
                NoteEvent::on(i as f64 / 48000.0, SourceId::DIRECT, 0, 60, 0.8).into(),
                route,
            )
            .take
            .unwrap();
    }
    assert_eq!(
        recorder.publish_note(NoteEvent::off(1.0, SourceId::DIRECT, 0, 60).into(), route).take,
        Err(publication::PublishError::Lost),
    );
    let config = harmonigraph_take::RenderConfig {
        renderer_path: directory.join("no-such-renderer").display().to_string(),
        ..Default::default()
    };
    control.stop(RenderRequest::from_config(&config));
    assert!(!recorder.is_armed());
    fence.worker_after_empty.enabled.store(false, Ordering::Release);
    wait_for(&fence.worker_stop_processed);
    assert!(fence.failed.load(Ordering::Acquire), "marker I/O failure must refuse export");
    wait_for(&fence.worker_failure_accounted);
    assert!(control.last_take().is_none(), "failed marker must not finalise as a complete take");
    let file = worker_take(&directory);
    let take = harmonigraph_take::Take::read(&file).unwrap();
    assert_eq!(take.notes().count(), publication::PUBLICATION_RING - 1);
    assert!(
        !std::fs::read_to_string(&file)
            .unwrap()
            .lines()
            .any(|line| line.starts_with("Incomplete(")),
        "the injected flush really failed to write the marker",
    );
    assert!(matches!(take.events.last(), Some(harmonigraph_take::CanonicalRecord::Gap(_))));
    let cause = control.status();
    assert!(cause.contains("cannot write incomplete marker"), "{cause}");
    assert!(cause.contains(&file.display().to_string()), "{cause}");
    assert!(cause.contains("Bad file descriptor"), "the OS write error remains visible: {cause}");
    control.tick(false, 0);
    assert_eq!(control.status(), cause, "UI refresh must retain the original I/O cause");
    assert!(!cause.contains("could not run"), "the renderer must not be launched");
    drop(recorder);
    drop(control);
    wait_for(&fence.worker_finished);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn carried_marker_failure_visits_retained_passes_and_keeps_the_first_error() {
    let file = path("carried-marker-failure");
    let status = Mutex::new(String::new());
    let mut current = Open::create(Default::default(), file.clone(), 3, None, &status).unwrap();
    current.fail_marker_on_pass = Some(3);
    for pass in 1..3 {
        let mut old = Open::create(Default::default(), file.clone(), pass, None, &status).unwrap();
        old.fail_marker_on_pass = Some(pass);
        current.retained.push(old);
    }
    let mut open = Some(current);
    let mut fanout = CanonicalFanout { unplaced: Some(Default::default()), ..Default::default() };
    let fence = RecordFence::default();
    let failure = FailureAccount::default();
    let (_publisher, mut consumer) = publication::channel();
    fanout.drain(&mut consumer, &mut open, &fence, &failure);
    assert!(fence.failed.load(Ordering::Acquire));
    let cause = fence.failure_message.lock().clone().unwrap();
    assert!(cause.contains("capture-3.take"), "first error must survive retained errors: {cause}");
    let current = open.as_ref().unwrap();
    for pass in std::iter::once(current).chain(current.retained.iter()) {
        assert_eq!(pass.fail_marker_on_pass, None, "every retained writer must be visited");
        assert!(pass.incomplete.is_none());
        assert!(harmonigraph_take::Take::read(pass.path()).unwrap().incomplete.is_none());
    }
    failure.account(&mut open, 1, &status, Some(&fence), Default::default());
    assert_eq!(fence.failure_message.lock().as_ref(), Some(&cause));
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

#[test]
fn rollover_marker_failure_keeps_the_old_owner_until_failure_accounting() {
    let file = path("rollover-marker-failure");
    let status = Mutex::new(String::new());
    let mut current = Open::create(Default::default(), file.clone(), 1, None, &status).unwrap();
    current.epoch = 1;
    current.mark_incomplete(Default::default()).unwrap();
    current.fail_marker_on_pass = Some(2);
    let mut open = Some(current);
    let fence = RecordFence::default();
    fence.intent.store(3, Ordering::Release);
    let failure = FailureAccount::default();
    let (mut producer, mut consumer) = rtrb::RingBuffer::new(1);
    producer.push(Entry::NewPass).unwrap();
    drain_with_boundaries(&mut consumer, None, &mut open, &status, Some(&fence), &failure, |_| {});
    assert!(fence.failed.load(Ordering::Acquire));
    assert!(failure.contains(1), "the previous epoch owner must reach failure accounting");
    assert!(open.is_none());
    assert!(fence.failure_message.lock().as_ref().unwrap().contains("capture-2.take"));
    assert!(harmonigraph_take::Take::read(&file).unwrap().incomplete.is_some());
    assert!(harmonigraph_take::Take::read(file.with_file_name("capture-2.take"))
        .unwrap()
        .incomplete
        .is_none());
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

#[test]
fn failure_accounting_latches_its_marker_io_error() {
    let file = path("accounting-marker-failure");
    let status = Mutex::new(String::new());
    let mut current = Open::create(Default::default(), file.clone(), 1, None, &status).unwrap();
    current.fail_marker_on_pass = Some(1);
    let mut open = Some(current);
    let fence = RecordFence::default();
    fence.fail(); // An audio-thread failure has no allocated message yet.
    FailureAccount::default().account(&mut open, 1, &status, Some(&fence), Default::default());
    assert_eq!(*status.lock(), CONFIGURATION_FAILURE);
    let cause = fence.failure_message.lock().clone().unwrap();
    assert!(cause.contains("cannot write incomplete marker"), "{cause}");
    assert!(cause.contains(&file.display().to_string()), "{cause}");
    assert!(cause.contains("Bad file descriptor"), "{cause}");
    assert!(open.is_none());
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}
