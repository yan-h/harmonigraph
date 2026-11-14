use super::*;
use harmonigraph_core::canonical::*;
use harmonigraph_core::confirmed::PitchProvenance;
use harmonigraph_core::notes::NoteEvent;
use publication::Lane;

/// Whether a [`worker_in`]'s render was reached: its job failed to start.
fn could_not_run(control: &Control) -> bool {
    control.export_jobs().iter().any(|job| job.detail.contains("could not run"))
}

fn path(name: &str) -> std::path::PathBuf {
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-canonical-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    directory.join("capture.take")
}

/// The clock `accepted` deltas are timed on.
const CLOCK: ClockId = ClockId { runtime_session: 7, epoch: 3 };

/// What the audio thread stamps an item played under `epoch`'s arm with.
fn armed(epoch: u64) -> publication::Span {
    publication::Span::at(CLOCK, 0, RecordIntent::armed(epoch))
}

/// The run a take that records from sample 0 on `CLOCK` opens with, at song 0.
fn opened(recorder: &mut Recorder) {
    recorder.run(1, 1, publication::Stamp { clock: CLOCK, input: 0 }, 0.0, 0.0);
}

/// An observed note as the Hub publishes one: timed where its input arrived,
/// which is its stamp.
fn stamped(event: NoteEvent) -> NoteDelta {
    let input = (event.time * 48_000.0) as i64;
    let timing =
        EventTiming { clock: CLOCK, input, planned: None, sample: input, sample_rate: 48_000.0 };
    NoteDelta { timing: Some(timing), ..event.into() }
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
            clock: CLOCK,
            input: (event.time * 48000.0) as i64 - 32,
            planned: None,
            sample: (event.time * 48000.0) as i64,
            sample_rate: 48000.0,
        }),
        pitch_microcents: None,
    }
}

/// A source may publish the last block's note after the transport restore.
/// Its stamp, taken when its input arrived, must survive that end, which
/// must not wait for MIDI.
#[test]
fn stopped_export_restore_keeps_the_unpublished_notes_original_stamp() {
    let (mut recorder, mut capture) = testing::channel();
    let (publisher, mut publications) = publication::channel();
    recorder.publication = publisher;
    capture.arm();
    assert!(recorder.is_armed());
    recorder.end_at_rewind.store(true, Ordering::Relaxed);
    let duration = 64.0 / 48_000.0;
    assert!(!recorder.observe_transport(5.0, false, duration).is_some());
    let origin = 5.0 + duration;
    assert!(recorder.observe_transport(origin, false, duration).is_some());
    let (epoch, pass) = recorder.armed_pass().unwrap();
    recorder.run(epoch, pass, Default::default(), origin, 0.0);
    recorder.audio(&mut std::iter::repeat_n(0.25, 128), 128);
    assert_eq!(recorder.latches.captured.load(Ordering::Relaxed), 0);
    assert!(!recorder.observe_transport(5.0, false, duration).is_some());
    assert_eq!(recorder.latches.end(), Some(End::Rewind));
    let note = accepted(NoteEvent::on(origin, SourceId(1), 0, 60, 0.8), 1);
    assert!(recorder.publish_note(note, armed(epoch)).take.is_ok());
    let mut notes = 0;
    publications.drain(|delivery, span| {
        if let publication::Delivery::Event(CanonicalEvent::Note(delta)) = delivery {
            assert_eq!(span, armed(epoch));
            assert_eq!(delta.event, note.event);
            notes += 1;
        }
        true
    });
    assert_eq!(notes, 1, "the delayed publication keeps its pre-restore stamp and timestamp");
    assert_eq!(recorder.latches.captured.load(Ordering::Relaxed), 1, "a note of the take");
    assert_eq!(capture.drain_audio(), vec![0.25; 128]);
}

/// History published late still lands in the pass whose run holds its input,
/// in the take's one file, and each pass's audio goes to its own WAV.
#[test]
fn delayed_history_and_baseline_keep_their_pass_and_both_wav_tails() {
    let (mut recorder, mut capture) = testing::channel();
    capture.arm();
    assert!(recorder.is_armed());
    let file = path("delayed");
    let mut writer = testing::FileWriter::new(&capture, file.clone());
    let at = |input| publication::Stamp { clock: CLOCK, input };
    let config = harmonigraph_core::configuration::ConfigReducer::default().resolved();
    assert!(recorder.observe_transport(20.0, true, 64.0 / 48_000.0).is_some());
    // Pass 1's run opens at input 0, presentation 0 and song 19: everything it
    // holds moves 19 s into the song.
    recorder.run(1, 1, at(0), 19.0, 0.0);
    recorder.configuration(1, 0.0, config);
    recorder.audio(&mut std::iter::repeat_n(0.125, 96 * 2), 96 * 2);
    assert!(recorder.observe_transport(0.0, true, 64.0 / 48_000.0).is_some());
    recorder.run_end(1, at(200_000));
    recorder.run(1, 2, at(200_000), 0.0, 200_000.0 / 48_000.0);
    recorder.configuration(1, 200_000.0 / 48_000.0, config);
    recorder.audio(&mut std::iter::repeat_n(0.25, 32 * 2), 32 * 2);
    capture.stop();
    assert!(!recorder.is_armed());
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
    for event in events {
        recorder.publish_note(event, armed(1)).expect_both();
    }
    let empty = SourceBaseline::new(source, 1, 2.0, 3, true, &[]).unwrap();
    let taken = publication::Span::at(CLOCK, 96_000, RecordIntent::armed(1));
    recorder.publish_baseline(Lane::Take, &empty, taken).unwrap();
    // Duplicated transfer must not write an already completed lifetime twice.
    for event in events {
        recorder.publish_note(event, armed(1)).expect_both();
    }
    recorder.publish_baseline(Lane::Take, &empty, taken).unwrap();
    recorder
        .publish_note(accepted(NoteEvent::on(2.5, source, 0, 60, 0.7), 4), armed(1))
        .expect_both();
    // Explicit disarmed provenance remains unrecorded despite the active file.
    recorder
        .publish_note(
            NoteEvent::on(3.0, SourceId::DIRECT, 0, 90, 0.8).into(),
            publication::Span::default(),
        )
        .expect_both();
    writer.drain(&mut capture);
    assert!(writer.finished.is_none());
    recorder.source_epoch_complete(1, 10.0);
    writer.drain(&mut capture);
    assert!(!writer.failed());
    assert_eq!(writer.finished.as_ref(), Some(&file), "every pass is in the take's one file");
    let take = harmonigraph_take::Take::read_pass(&file, Some(1)).unwrap();
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
    let last = harmonigraph_take::Take::read(&file).unwrap();
    assert_eq!((last.pass, last.notes().count()), (Some(2), 0), "late MIDI stays in pass 1");
    for (wav, frames) in [("capture.wav", 96), ("capture-2.wav", 32)] {
        let audio = std::fs::read(file.with_file_name(wav)).unwrap();
        assert_eq!(u32::from_le_bytes(audio[40..44].try_into().unwrap()) as usize / 8, frames);
        assert_eq!((audio.len() - 44) / 8, frames);
    }
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

#[test]
fn real_publication_ring_loss_is_durable_after_the_last_callback() {
    let (mut recorder, mut capture) = testing::channel();
    capture.arm();
    recorder.is_armed();
    opened(&mut recorder);
    let file = path("full");
    let mut writer = testing::FileWriter::new(&capture, file.clone());
    let route = armed(1);
    // One short of the ring: the last cell is reserved so the gap below has
    // somewhere to go without another callback, which is this test's subject.
    for i in 0..publication::PUBLICATION_RING - 1 {
        recorder
            .publish_note(
                stamped(NoteEvent::on(i as f64 / 48000.0, SourceId::DIRECT, 0, 60, 0.8)),
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
        recorder.publish_note(stamped(NoteEvent::off(1.0, SourceId::DIRECT, 0, 60)), route),
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
        self.0.worker_after_finish.enabled.store(false, Ordering::Release);
        self.0.worker_before_commands.enabled.store(false, Ordering::Release);
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
/// `an_overflowed_take_finalises_and_launches_its_render`
/// for what the overflow alone now does.
#[test]
fn real_worker_materializes_pending_start_before_accounting_a_recording_failure() {
    for stop_after_failure in [false, true] {
        let directory = path(if stop_after_failure { "worker-start-stop" } else { "worker-start" })
            .parent()
            .unwrap()
            .to_path_buf();
        let (mut recorder, control) = worker_in(&directory);
        let fence = control.fence.clone();
        let _resume_on_panic = WorkerPause(fence.clone());
        fence.worker_after_empty.enabled.store(true, Ordering::Release);
        wait_for(&fence.worker_after_empty.entered);
        control.start(48000.0, String::new());
        assert!(recorder.is_armed());
        opened(&mut recorder);
        recorder.configuration(
            1,
            0.0,
            harmonigraph_core::configuration::ConfigReducer::default().resolved(),
        );
        for id in 1..=2 {
            let baseline = SourceBaseline::new(SourceId::DIRECT, id, 0.0, 0, true, &[]).unwrap();
            recorder.publish_baseline(Lane::Take, &baseline, armed(1)).unwrap();
        }
        for i in 0..publication::PUBLICATION_RING - 3 {
            recorder
                .publish_note(
                    stamped(NoteEvent::on(i as f64 / 48000.0, SourceId::DIRECT, 0, 60, 0.8)),
                    armed(1),
                )
                .expect_both();
        }
        assert_eq!(
            recorder
                .publish_note(stamped(NoteEvent::off(1.0, SourceId::DIRECT, 0, 60)), armed(1))
                .take,
            Err(publication::PublishError::Lost)
        );
        if stop_after_failure {
            recorder.fail_configuration();
            fence.worker_after_empty.enabled.store(false, Ordering::Release);
            wait_for(&fence.worker_failure_accounted);
            fence.worker_after_stop.enabled.store(true, Ordering::Release);
            control.stop();
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
            assert!(control
                .export_jobs()
                .iter()
                .all(|job| job.state != harmonigraph_take::render::ExportStatus::Running));
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
/// pane sends. The renderer points at nothing ([`worker_in`]), so the launch is observable
/// (and instant) through the export job's detail rather than by running a
/// GPU render: the message only exists if `spawn_render` was reached at all,
/// which is the assertion the old behaviour failed. That the resulting file
/// warns rather than refuses is
/// `a_take_missing_note_history_is_exported_with_a_warning` in
/// harmonigraph-offline; this is the half that gets it there.
#[test]
fn an_overflowed_take_finalises_and_launches_its_render() {
    let directory = path("worker-overflow-render").parent().unwrap().to_path_buf();
    let (mut recorder, control) = worker_in(&directory);
    let fence = control.fence.clone();
    let _resume_on_panic = WorkerPause(fence.clone());
    control.start(48000.0, String::new());
    assert!(recorder.is_armed());
    opened(&mut recorder);
    let route = armed(1);
    recorder.configuration(
        1,
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
        match recorder.publish_note(stamped(event), route).take {
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

    control.stop();
    assert!(!recorder.is_armed());
    recorder.configuration_epoch_complete(1);
    recorder.source_epoch_complete(1, 1.0);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while control.last_take().is_none() && std::time::Instant::now() < until {
        std::thread::yield_now();
    }
    let finished = control.last_take().expect("the overflowed take still finalises");
    assert!(!fence.failed.load(Ordering::Acquire));
    assert_eq!(finished, worker_take(&directory), "Render targets THIS take, not an earlier one");
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !could_not_run(&control) && std::time::Instant::now() < until {
        std::thread::yield_now();
    }
    assert!(
        could_not_run(&control),
        "Stop-and-render reached the renderer: {:?}",
        control.export_jobs()
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
    let (mut recorder, control) = worker_in(&directory);
    let fence = control.fence.clone();
    let render = control.render.clone();
    let last_take = control.last_take.clone();
    let _resume_on_panic = WorkerPause(fence.clone());
    control.start(48000.0, String::new());
    assert!(recorder.is_armed());
    opened(&mut recorder);
    recorder.configuration(
        1,
        0.0,
        harmonigraph_core::configuration::ConfigReducer::default().resolved(),
    );
    fence.worker_after_stop.enabled.store(true, Ordering::Release);
    control.stop();
    assert!(!recorder.is_armed());
    recorder.configuration_epoch_complete(1);
    wait_for(&fence.worker_after_stop.entered);
    let route = armed(1);
    recorder
        .publish_note(accepted(NoteEvent::on(0.01, SourceId(1), 0, 60, 0.8), 1), route)
        .expect_both();
    recorder
        .publish_note(accepted(NoteEvent::off(0.02, SourceId(1), 0, 60), 2), route)
        .expect_both();
    recorder.source_epoch_complete(1, 1.0);
    drop(recorder);
    drop(control); // The next real worker poll observes Disconnected.
    fence.worker_after_stop.enabled.store(false, Ordering::Release);
    wait_for(&fence.worker_finished);
    // Writer completion can leave export staging active in this directory.
    render.shutdown();
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

/// Completion owns only its finishing message and publishes it before Start
/// is allowed again, even if the old completion handler has not run yet.
#[test]
fn completed_recording_releases_its_status_before_accepting_another_start() {
    for (case, newer) in [
        ("ordinary", None),
        ("split", None),
        ("retry", None),
        ("error", Some("cannot start renderer: test error")),
    ] {
        let directory = path(&format!("worker-status-{case}")).parent().unwrap().to_path_buf();
        let (mut recorder, control) = worker_in(&directory);
        let fence = control.fence.clone();
        let _resume_on_panic = WorkerPause(fence.clone());
        control.start(48000.0, String::new());
        assert!(recorder.is_armed());
        let at = |input| publication::Stamp { clock: CLOCK, input };
        recorder.run(1, 1, at(0), 0.0, 0.0);
        recorder.configuration(
            1,
            0.0,
            harmonigraph_core::configuration::ConfigReducer::default().resolved(),
        );
        let route = armed(1);
        recorder
            .publish_note(accepted(NoteEvent::on(0.01, SourceId(1), 0, 60, 0.8), 1), route)
            .expect_both();
        recorder
            .publish_note(accepted(NoteEvent::off(0.02, SourceId(1), 0, 60), 2), route)
            .expect_both();
        fence.worker_after_stop.enabled.store(true, Ordering::Release);
        fence.worker_after_finish.enabled.store(true, Ordering::Release);
        if case == "split" {
            // First ensure Start was consumed; otherwise the worker could
            // drain the split with Start, before it polls the queued Stop.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while std::fs::read_dir(&directory).map_or(true, |mut entries| entries.next().is_none())
            {
                assert!(std::time::Instant::now() < deadline, "Start did not open its file");
                std::thread::yield_now();
            }
            // Stop must be polled before the pending transport split drains.
            fence.worker_before_commands.enabled.store(true, Ordering::Release);
            wait_for(&fence.worker_before_commands.entered);
            recorder.run_end(1, at(48_000));
            recorder.run(1, 2, at(48_000), 0.0, 1.0);
        }
        control.stop();
        assert!(!recorder.is_armed());
        recorder.configuration_epoch_complete(1);
        if case == "split" {
            fence.worker_before_commands.enabled.store(false, Ordering::Release);
            fence.worker_before_commands.entered.store(false, Ordering::Release);
        }
        recorder.source_epoch_complete(1, 1.0);
        wait_for(&fence.worker_after_stop.entered);
        if case == "retry" {
            control.start(48000.0, String::new());
            assert!(!control.is_recording());
            assert!(control.status().starts_with("finishing the previous take"));
        }
        if let Some(message) = newer {
            *control.status.lock() = message.into();
        }
        fence.worker_after_stop.enabled.store(false, Ordering::Release);
        wait_for(&fence.worker_after_finish.entered);
        assert!(!fence.finishing.load(Ordering::Acquire));
        let file = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|file| file.extension().is_some_and(|ext| ext == "take"))
            .expect("the completed take");
        if case == "split" {
            let stem = file.file_stem().unwrap().to_str().unwrap();
            let second = file.with_file_name(format!("{stem}-2.wav"));
            assert!(second.exists(), "queued split did not open its pass");
        }
        let take = harmonigraph_take::Take::read_pass(&file, Some(1)).unwrap();
        assert!(take.incomplete.is_none());
        assert_eq!(take.notes().count(), 2, "fixture must finish a nonempty prefix");
        let selected = file;
        assert_eq!(control.status(), newer.unwrap_or_default());
        assert!(control.last_take().is_none(), "old completion handler is still parked");
        control.start(48000.0, String::new());
        assert!(control.is_recording());
        let armed = control.status();
        // Park at the next command poll: the old handler runs, but the new
        // Start has not opened a file or published its own writer message.
        fence.worker_before_commands.enabled.store(true, Ordering::Release);
        fence.worker_after_finish.enabled.store(false, Ordering::Release);
        wait_for(&fence.worker_before_commands.entered);
        assert_eq!(control.last_take(), Some(selected));
        assert_eq!(control.status(), armed, "old completion overwrote the new Start");
        // And the handler really launched this take's render.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !could_not_run(&control) && std::time::Instant::now() < until {
            std::thread::yield_now();
        }
        assert!(could_not_run(&control), "{:?}", control.export_jobs());
        assert_eq!(control.status(), armed, "the render wrote onto the record line");
        drop(recorder);
        drop(control);
        fence.worker_before_commands.enabled.store(false, Ordering::Release);
        wait_for(&fence.worker_finished);
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn retired_producer_keeps_real_writer_alive_after_every_ui_control_is_dropped() {
    let directory = path("retired-producer").parent().unwrap().to_path_buf();
    let (mut recorder, control) = worker_in(&directory);
    let fence = control.fence.clone();
    let render = control.render.clone();
    let _resume_on_panic = WorkerPause(fence.clone());
    fence.worker_after_empty.enabled.store(true, Ordering::Release);
    wait_for(&fence.worker_after_empty.entered);
    control.start(48000.0, String::new());
    assert!(recorder.is_armed());
    opened(&mut recorder);
    recorder.configuration(
        1,
        0.0,
        harmonigraph_core::configuration::ConfigReducer::default().resolved(),
    );
    fence.worker_after_stop.enabled.store(true, Ordering::Release);
    control.stop();
    assert!(!recorder.is_armed());
    recorder.configuration_epoch_complete(1);
    drop(control); // No UI, Control or external command sender survives.
    fence.worker_after_empty.enabled.store(false, Ordering::Release);
    wait_for(&fence.worker_after_stop.entered);
    fence.worker_after_empty.entered.store(false, Ordering::Release);
    fence.worker_after_empty.enabled.store(true, Ordering::Release);
    fence.worker_after_stop.enabled.store(false, Ordering::Release);
    wait_for(&fence.worker_after_empty.entered);
    assert!(!fence.worker_finished.load(Ordering::Acquire));
    let route = armed(1);
    recorder
        .publish_note(accepted(NoteEvent::on(0.01, SourceId(1), 0, 60, 0.8), 1), route)
        .expect_both();
    recorder
        .publish_note(accepted(NoteEvent::off(0.02, SourceId(1), 0, 60), 2), route)
        .expect_both();
    recorder.source_epoch_complete(1, 1.0);
    drop(recorder);
    fence.worker_after_empty.enabled.store(false, Ordering::Release);
    wait_for(&fence.worker_finished);
    // Writer completion can leave export staging active in this directory.
    render.shutdown();
    let take = harmonigraph_take::Take::read(worker_take(&directory)).unwrap();
    assert!(take.incomplete.is_none());
    assert_eq!(take.notes().count(), 2);
    std::fs::remove_dir_all(directory).unwrap();
}

/// #712's race, under #1760's rule: a gap can reach the writer before the file
/// it belongs to exists. An outage that lost armed history stays armed however
/// much disarmed history merges into it, so it waits for its take's `Start`
/// like any armed record, and lands in it; the gap of only disarmed history
/// ahead of it touched no take, and warns nothing.
///
/// The interleaving is a genuine race in production. A merged gap only reaches
/// the ring once a drain running CONCURRENTLY with the audio thread has freed
/// cells, and the worker polls `Start` outside that drain — so the same drain
/// meets the gap with no file open. The drain is therefore done by hand here,
/// one item consumed and the next declined, which leaves exactly the two free
/// cells the racing worker leaves behind.
///
/// What the serial range proves is that the fixture reaches the case rather
/// than something next to it. `4097` is the ARMED publication and `4098` the
/// disarmed one merged into it; `4096`, disarmed alone, is not on the take.
#[test]
fn an_armed_gap_with_no_file_open_waits_for_its_take() {
    let (mut publisher, mut consumer) = publication::channel();
    let disarmed = publication::Span::default();
    let armed = armed(1);
    let sounding =
        |i: usize| stamped(NoteEvent::on(i as f64 / 48000.0, SourceId::DIRECT, 0, 60, 0.8));
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
    // A third loss, disarmed, merges into the held one, which stays armed.
    assert_eq!(publisher.note(sounding(2), disarmed), Err(publication::PublishError::Lost));

    let mut fanout = CanonicalFanout::default();
    let fence = RecordFence::default();
    let failure = FailureAccount::default();
    let file = path("waiting-gap");
    // `Start` is queued and unpolled, so the writer holds no file at all.
    let mut nothing_open: Option<Recording> = None;
    fanout.drain(&mut consumer, &mut nothing_open, &fence, &failure);
    assert!(fanout.waiting_file, "the armed gap waits for its take");
    assert!(
        !fence.failed.load(Ordering::Acquire),
        "a hole in the note history still does not refuse the take"
    );

    let status = Mutex::new(String::new());
    let mut recording = Recording::create(
        harmonigraph_take::Header::default(),
        file.clone(),
        1,
        FIXTURE_SPEC,
        &status,
    )
    .unwrap();
    // The run the lost armed history was played in.
    let at = harmonigraph_take::StampRecord::new(CLOCK, 0);
    recording.run(harmonigraph_take::RunRecord { pass: 1, at, ..Default::default() }).unwrap();
    let mut open = Some(recording);
    fanout.drain(&mut consumer, &mut open, &fence, &failure);
    let sealed = open.take().unwrap().finish().unwrap();
    let take = harmonigraph_take::Take::read(&sealed).unwrap();
    let loss = take.incomplete.expect("the gap that had no file lands in its take");
    assert_eq!(loss.reason, harmonigraph_take::canonical::GapReasonRecord::PublicationFull);
    assert_eq!(
        (loss.first_publication, loss.last_publication),
        (4097, 4098),
        "the armed outage with what merged into it, and not the disarmed one before"
    );
    assert!(!fence.failed.load(Ordering::Acquire));
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

/// A loss of history played before the arm touched no recorded run, so the
/// take that opens after it does not warn: #712's stated residual, a warning
/// about a hole the take does not have, is gone (#1760, question 2).
///
/// `worker_after_empty` parks the worker inside the arm that found no command,
/// so everything published below reaches the ring before the loop's own drain
/// runs, and the `Start` queued below is not polled until the iteration after
/// it. The gap is drained against no file exactly as the racing case drains
/// it, and the take opened one iteration later is what must not carry it.
#[test]
fn a_loss_before_the_arm_leaves_the_take_that_follows_whole() {
    let directory = path("worker-gap-before-start").parent().unwrap().to_path_buf();
    let (mut recorder, control) = worker_in(&directory);
    let fence = control.fence.clone();
    let render = control.render.clone();
    let _resume_on_panic = WorkerPause(fence.clone());
    fence.worker_after_empty.enabled.store(true, Ordering::Release);
    wait_for(&fence.worker_after_empty.entered);
    let sounding =
        |i: usize| stamped(NoteEvent::on(i as f64 / 48000.0, SourceId::DIRECT, 0, 60, 0.8));
    // Only the take lane is under test, so only its outcome is asserted; the
    // editor's lane fills alongside it and is drained back to empty below.
    let mut displayed = control.take_display().expect("the display lane's consumer");
    for i in 0..publication::PUBLICATION_RING - 1 {
        recorder.publish_note(sounding(i), publication::Span::default()).take.unwrap();
    }
    assert_eq!(
        recorder.publish_note(sounding(0), publication::Span::default()).take,
        Err(publication::PublishError::Lost),
        "the fixture must actually lose a report"
    );
    displayed.drain(|_, _| true);
    control.start(48000.0, String::new());
    assert!(recorder.is_armed());
    opened(&mut recorder);
    // Released into the drain that has no file, then into the poll that opens
    // one — in that order, because the pause is inside the arm that found no
    // command and the loop drains before it polls again. `recording to` is the
    // status only `Recording::create` writes, so waiting for it waits for both.
    fence.worker_after_empty.enabled.store(false, Ordering::Release);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !control.status().starts_with("recording to") && std::time::Instant::now() < until {
        std::thread::yield_now();
    }
    assert!(control.status().starts_with("recording to"), "{}", control.status());
    recorder.configuration(
        1,
        0.0,
        harmonigraph_core::configuration::ConfigReducer::default().resolved(),
    );
    let route = armed(1);
    recorder
        .publish_note(accepted(NoteEvent::on(0.01, SourceId(1), 0, 60, 0.8), 1), route)
        .expect_both();
    recorder
        .publish_note(accepted(NoteEvent::off(0.02, SourceId(1), 0, 60), 2), route)
        .expect_both();
    control.stop();
    assert!(!recorder.is_armed(), "the disarm boundary closes the producer's prefix");
    recorder.configuration_epoch_complete(1);
    recorder.source_epoch_complete(1, 1.0);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while control.last_take().is_none() && std::time::Instant::now() < until {
        std::thread::yield_now();
    }
    let finished = control.last_take().expect("the take still finalises");
    assert!(!fence.failed.load(Ordering::Acquire), "and still is not a refusal");
    let take = harmonigraph_take::Take::read(&finished).unwrap();
    assert!(take.incomplete.is_none(), "the disarmed loss is no hole in this take");
    assert_eq!(take.notes().count(), 2, "and the take it opened is whole");
    drop(recorder);
    drop(control);
    wait_for(&fence.worker_finished);
    // Writer completion can leave export staging active in this directory.
    render.shutdown();
    std::fs::remove_dir_all(directory).unwrap();
}

/// #895, correctness item 4: with TWO gaps, the file's marker and the reader's
/// answer must be the same gap.
///
/// The two encodings of "this take is incomplete" used to coalesce in opposite
/// directions — `Recording::mark_incomplete` keeps the first, `Take::parse` took the
/// last line it saw — so the export warned about a range the file's own marker
/// did not hold. One gap cannot show that: the first and the last are the same
/// record, and the assertion passes under either rule. So the fixture carries
/// two gaps with DISJOINT ranges, and the count below is what proves it did.
///
/// Each gap is written as an ordinary `Gap` record as well as being marked,
/// because that is what the fanout does with an addressed one, and it is the
/// later `Gap` LINE that the reader used to prefer over the marker above it.
#[test]
fn a_second_gap_leaves_the_reader_naming_the_gap_the_file_marked() {
    use harmonigraph_take::canonical::{GapReasonRecord, GapRecord};
    let file = path("two-gaps");
    let status = Mutex::new(String::new());
    let mut open =
        Recording::create(Default::default(), file.clone(), 1, FIXTURE_SPEC, &status).unwrap();
    let first = harmonigraph_take::IncompleteRecord {
        first_publication: 7,
        last_publication: 9,
        reason: GapReasonRecord::PublicationFull,
    };
    let second = harmonigraph_take::IncompleteRecord {
        first_publication: 40,
        last_publication: 44,
        reason: GapReasonRecord::InvalidRecord,
    };
    for (record, t) in [(first, 1.0), (second, 2.0)] {
        let gap = GapRecord {
            source: None,
            t,
            through: t,
            first: record.first_publication,
            last: record.last_publication,
            reason: record.reason,
            inputs: None,
        };
        open.writer.canonical(harmonigraph_take::CanonicalRecord::Gap(gap)).unwrap();
        open.mark_incomplete(record).unwrap();
    }
    let sealed = open.finish().unwrap();

    let text = std::fs::read_to_string(&sealed).unwrap();
    let markers: Vec<_> = text.lines().filter(|line| line.starts_with("Incomplete(")).collect();
    assert_eq!(markers.len(), 1, "the writer marks the recording once: {markers:?}");
    let take = harmonigraph_take::Take::read(&sealed).unwrap();
    assert_eq!(
        take.events
            .iter()
            .filter(|record| matches!(record, harmonigraph_take::CanonicalRecord::Gap(_)))
            .count(),
        2,
        "the fixture must actually carry two gaps, or it proves nothing about which wins",
    );
    assert_eq!(
        take.incomplete,
        Some(first),
        "the reader names the gap the marker holds, not the last Gap line in the file",
    );
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

#[test]
fn failure_accounting_latches_its_marker_io_error() {
    let file = path("accounting-marker-failure");
    let status = Mutex::new(String::new());
    let mut current =
        Recording::create(Default::default(), file.clone(), 1, FIXTURE_SPEC, &status).unwrap();
    current.fail_marker = true;
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

#[test]
fn pump_flushes_buffered_records_after_both_lanes() {
    let file = path("pump-batched-flush");
    let status = Mutex::new(String::new());
    let mut recording =
        Recording::create(Default::default(), file.clone(), 1, FIXTURE_SPEC, &status).unwrap();
    recording
        .writer
        .param(harmonigraph_take::ParamRecord {
            t: 0.0,
            id: ParamKey::ALL[0].id().into(),
            value: 1.0,
        })
        .unwrap();
    assert!(
        harmonigraph_take::Take::read(&file).unwrap().params.is_empty(),
        "a small record stays buffered until the pump flushes"
    );
    let mut pump = Pump { open: Some(recording), ..Default::default() };
    let (mut producer, mut entries) = rtrb::RingBuffer::<Entry>::new(8);
    producer.push(Entry::Param { t: 1.0, key: 0, value: 99.0 }).unwrap();
    let (_audio, mut samples) = rtrb::RingBuffer::<f32>::new(8);
    let (mut publisher, mut publications) = publication::channel();
    publisher.note(NoteEvent::on(1.0, SourceId::DIRECT, 0, 60, 0.8).into(), armed(1)).unwrap();
    let fence = RecordFence::default();
    fence.arm(1);
    let pumped = pump.pass(&mut entries, &mut samples, &mut publications, &status, &fence, false);
    assert!(pumped.finished.is_none(), "the file needs no finalization to publish its bytes");
    assert!(!fence.failed.load(Ordering::Acquire));
    let take = harmonigraph_take::Take::read(&file).unwrap();
    assert_eq!(take.params.iter().map(|p| p.value).collect::<Vec<_>>(), [1.0, 99.0]);
    assert_eq!(take.notes().count(), 1, "flush follows the publication-lane drain too");
    drop(pump);
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

#[test]
fn pump_flush_failure_is_accounted_before_stop() {
    let file = path("pump-flush-failure");
    let status = Mutex::new(String::new());
    let mut recording =
        Recording::create(Default::default(), file.clone(), 1, FIXTURE_SPEC, &status).unwrap();
    recording.writer.make_read_only_for_test(&file).unwrap();
    let mut pump = Pump { open: Some(recording), ..Default::default() };
    let (mut producer, mut entries) = rtrb::RingBuffer::<Entry>::new(8);
    producer.push(Entry::Param { t: 0.0, key: 0, value: 0.5 }).unwrap();
    let (_audio, mut samples) = rtrb::RingBuffer::<f32>::new(8);
    let (_publisher, mut publications) = publication::channel();
    let fence = RecordFence::default();
    fence.arm(1);
    let pumped = pump.pass(&mut entries, &mut samples, &mut publications, &status, &fence, false);
    assert!(pumped.finished.is_none());
    assert!(fence.failed.load(Ordering::Acquire));
    assert!(pump.failure.contains(1));
    assert!(pump.open.is_none(), "the failed file is accounted without waiting for Stop");
    let cause = fence.failure_message.lock().clone().unwrap();
    assert!(cause.contains("cannot flush"), "{cause}");
    assert!(cause.contains(&file.display().to_string()), "{cause}");
    assert!(cause.contains("Bad file descriptor"), "{cause}");
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

/// What the owner states for one recorded block at `input`: its run, opened at
/// `origin` in the pass's audio, a note played and released inside it, and
/// its audio. The Hub's clock here is the input sample's own time.
fn record_block(recorder: &mut Recorder, input: i64, origin: f64, key: u8, level: f32) {
    let duration = 64.0 / 48_000.0;
    let (epoch, pass) = recorder.armed_pass().unwrap();
    let presentation = input as f64 / 48_000.0;
    let at = |input| publication::Stamp { clock: CLOCK, input };
    recorder.run(epoch, pass, at(input), origin, presentation);
    let timed = |event: NoteEvent| NoteDelta {
        timing: Some(EventTiming {
            clock: CLOCK,
            input,
            planned: None,
            sample: input,
            sample_rate: 48_000.0,
        }),
        ..event.into()
    };
    for event in [
        NoteEvent::on(presentation, SourceId::DIRECT, 0, key, 0.8),
        NoteEvent::off(presentation + duration / 2.0, SourceId::DIRECT, 0, key),
    ] {
        recorder.publish_note(timed(event), armed(epoch)).expect_both();
    }
    recorder.audio(&mut std::iter::repeat_n(level, 128), 128);
    recorder.run_end(epoch, at(input + 64));
}

fn wav_samples(path: &std::path::Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap();
    bytes[44..].as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()
}

fn song_times(take: &harmonigraph_take::Take) -> Vec<f64> {
    take.notes().map(|n| (n.t * 1e9).round() / 1e9).collect()
}

/// A stopped seek must survive parked observations, but an ordinary pause
/// cannot insert a pass or move either block's samples relative to its notes:
/// the resume's run opens where the audio got to.
#[test]
fn forward_seeks_split_real_audio_at_the_new_notes_origin() {
    let duration = 64.0 / 48_000.0;
    let rounded = |t: f64| (t * 1e9).round() / 1e9;
    for (name, paused, seek) in [
        ("playing-forward-seek", false, 10.0),
        ("stopped-forward-seek", true, 10.0),
        ("ordinary-pause", true, duration),
    ] {
        let file = path(name);
        let (mut recorder, mut capture) = testing::channel();
        capture.arm();
        assert!(recorder.is_armed());
        let mut writer = testing::FileWriter::new(&capture, file.clone());
        let origin = recorder.observe_transport(0.0, true, duration).unwrap();
        record_block(&mut recorder, 0, origin, 60, 0.25);
        writer.drain(&mut capture);
        if paused {
            for _ in 0..3 {
                assert!(recorder.observe_transport(0.0, false, duration).is_none());
            }
            if seek == 10.0 {
                for _ in 0..3 {
                    assert!(recorder.observe_transport(seek, false, duration).is_none());
                }
            }
            writer.drain(&mut capture);
            assert_eq!(writer.pass(), Some(1), "a parked seek opens no pass");
        }
        let origin = recorder.observe_transport(seek, true, duration).unwrap();
        let split = seek == 10.0;
        assert_eq!(recorder.armed_pass().unwrap().1, if split { 2 } else { 1 });
        assert_eq!(origin, if split { seek } else { duration }, "the audio's own position");
        record_block(&mut recorder, 640, origin, 72, 0.75);
        capture.stop();
        assert!(!recorder.is_armed());
        recorder.configuration_epoch_complete(1);
        recorder.source_epoch_complete(1, 11.0);
        writer.stop();
        writer.drain(&mut capture);
        assert!(!writer.failed());
        assert_eq!(writer.finished, Some(file.clone()), "one file holds every pass");
        let take = harmonigraph_take::Take::read(&file).unwrap();
        assert!(take.incomplete.is_none());
        let second_wav = file.with_file_name("capture-2.wav");
        if split {
            let first = harmonigraph_take::Take::read_pass(&file, Some(1)).unwrap();
            assert_eq!(first.audio_start, Some(0.0));
            assert_eq!(song_times(&first), [0.0, rounded(duration / 2.0)]);
            assert_eq!(wav_samples(&file.with_extension("wav")), vec![0.25; 128]);
            assert_eq!((take.pass, take.audio_start), (Some(2), Some(seek)));
            assert_eq!(song_times(&take), [seek, rounded(seek + duration / 2.0)]);
            assert_eq!(wav_samples(&second_wav), vec![0.75; 128]);
        } else {
            assert_eq!((take.passes.as_slice(), take.audio_start), (&[1][..], Some(0.0)));
            let expected = [0.0, duration / 2.0, duration, duration + duration / 2.0];
            assert_eq!(song_times(&take), expected.map(rounded));
            assert_eq!(
                wav_samples(&file.with_extension("wav")),
                [vec![0.25; 128], vec![0.75; 128]].concat()
            );
            assert!(!second_wav.exists());
        }
        std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    }
}

#[test]
fn an_audio_only_last_pass_renders_after_a_pass_with_midi() {
    let file = path("audio-only-last-pass");
    let (mut recorder, mut capture) = testing::channel();
    capture.arm();
    assert!(recorder.is_armed());
    let mut writer = testing::FileWriter::new(&capture, file.clone());
    let duration = 64.0 / 48_000.0;
    let origin = recorder.observe_transport(10.0, true, duration).unwrap();
    record_block(&mut recorder, 0, origin, 60, 0.25);
    let origin = recorder.observe_transport(0.0, true, duration).unwrap();
    let (epoch, pass) = recorder.armed_pass().unwrap();
    assert_eq!(pass, 2);
    let at = publication::Stamp { clock: CLOCK, input: 64 };
    recorder.run(epoch, pass, at, origin, 64.0 / 48_000.0);
    recorder.audio(&mut std::iter::repeat_n(0.75, 128), 128);
    capture.stop();
    assert!(!recorder.is_armed());
    recorder.configuration_epoch_complete(epoch);
    recorder.source_epoch_complete(epoch, 11.0);
    writer.stop();
    writer.drain(&mut capture);
    assert!(!writer.failed());
    assert_eq!(writer.finished, Some(file.clone()));
    let take = harmonigraph_take::Take::read(&file).unwrap();
    assert!(take.incomplete.is_none());
    assert_eq!(take.pass, Some(2), "the last pass renders");
    assert_eq!(take.notes().count(), 0, "no held note accidentally makes this a MIDI pass");
    assert_eq!(take.audio_start, Some(0.0));
    assert_eq!(take.audio_file.as_deref(), Some("capture-2.wav"));
    assert_eq!(wav_samples(&file.with_file_name("capture-2.wav")), vec![0.75; 128]);
    let first = harmonigraph_take::Take::read_pass(&file, Some(1)).unwrap();
    assert_eq!(first.notes().count(), 2);
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

/// A writer stalled while a pass records more audio than its ring holds fails
/// the take, and the pass that lost the audio says so.
#[test]
fn a_failure_inside_a_pass_marks_that_pass() {
    let file = path("closed-failure");
    let (mut recorder, mut capture) = testing::channel();
    capture.arm();
    assert!(recorder.is_armed());
    let mut writer = testing::FileWriter::new(&capture, file.clone());
    let duration = 64.0 / 48_000.0;
    let at = |input| publication::Stamp { clock: CLOCK, input };
    let origin = recorder.observe_transport(0.0, true, duration).unwrap();
    record_block(&mut recorder, 0, origin, 60, 0.25);
    writer.drain(&mut capture);
    // The writer stalls, and pass 1's next block is longer than the audio ring.
    let origin = recorder.observe_transport(duration, true, duration).unwrap();
    let (epoch, pass) = recorder.armed_pass().unwrap();
    assert_eq!(pass, 1);
    recorder.run(epoch, pass, at(64), origin, 64.0 / 48_000.0);
    let samples = AUDIO_RING_CAPACITY + 256;
    recorder.audio(&mut std::iter::repeat_n(0.5, samples), samples);
    recorder.run_end(epoch, at(128));
    // A forward seek opens pass 2.
    let origin = recorder.observe_transport(10.0, true, duration).unwrap();
    assert_eq!(recorder.armed_pass().unwrap().1, 2);
    record_block(&mut recorder, 640, origin, 72, 0.75);
    writer.drain(&mut capture);
    assert!(writer.failed(), "the fixture must fail the take inside pass 1");
    let first = harmonigraph_take::Take::read_pass(&file, Some(1)).unwrap();
    assert!(first.incomplete.is_some(), "pass 1 lost audio and must say so");
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

/// A failure marker names no pass, so a failure in pass 2 marks pass 1 too,
/// though everything pass 1 recorded reached the file (#1766).
#[test]
fn a_failure_marks_every_pass_of_the_take() {
    let file = path("late-failure");
    let (mut recorder, mut capture) = testing::channel();
    capture.arm();
    assert!(recorder.is_armed());
    let mut writer = testing::FileWriter::new(&capture, file.clone());
    let duration = 64.0 / 48_000.0;
    let origin = recorder.observe_transport(0.0, true, duration).unwrap();
    record_block(&mut recorder, 0, origin, 60, 0.25);
    let origin = recorder.observe_transport(10.0, true, duration).unwrap();
    record_block(&mut recorder, 640, origin, 72, 0.75);
    writer.drain(&mut capture);
    recorder.fail_configuration();
    writer.drain(&mut capture);
    assert!(writer.failed());
    let first = harmonigraph_take::Take::read_pass(&file, Some(1)).unwrap();
    assert!(first.incomplete.is_some(), "the take failed, so pass 1 warns too");
    assert_eq!(first.notes().count(), 2);
    let second = harmonigraph_take::Take::read_pass(&file, Some(2)).unwrap();
    assert!(second.incomplete.is_some(), "pass 2 was open when it failed");
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

/// An item of a take the writer already sealed played in no run its closure
/// could prove — a pause's late input, say — so it is dropped. Held at the
/// lane's head instead, it waited for a file that never opens and failed the
/// next take at its first drain, until the plugin was reloaded.
#[test]
fn an_item_of_a_sealed_take_is_dropped_rather_than_failing_the_next() {
    let file = path("after-seal");
    let (mut recorder, mut capture) = testing::channel();
    capture.arm();
    assert!(recorder.is_armed());
    let mut writer = testing::FileWriter::new(&capture, file.clone());
    let origin = recorder.observe_transport(0.0, true, 64.0 / 48_000.0).unwrap();
    record_block(&mut recorder, 0, origin, 60, 0.25);
    capture.stop();
    assert!(!recorder.is_armed());
    recorder.configuration_epoch_complete(1);
    recorder.source_epoch_complete(1, 1.0);
    writer.stop();
    writer.drain(&mut capture);
    assert_eq!(writer.finished.as_ref(), Some(&file), "take 1 sealed");
    let late = accepted(NoteEvent::on(2.0, SourceId::DIRECT, 0, 64, 0.8), 3);
    recorder.publish_note(late, armed(1)).expect_both();
    writer.drain(&mut capture);
    capture.arm();
    assert!(recorder.is_armed());
    let mut next = testing::FileWriter::new(&capture, path("after-seal-2"));
    next.drain(&mut capture);
    assert!(!next.failed(), "take 1's late item must not fail take 2");
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
    std::fs::remove_dir_all(path("after-seal-2").parent().unwrap()).unwrap();
}
