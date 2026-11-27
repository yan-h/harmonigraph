use super::*;
use harmonigraph_core::notes::NoteEvent;

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

fn until(ready: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready() && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(ready(), "the real worker did not get there");
}

/// One recorded block at presentation `at`, `origin` in the take's audio: its
/// run, a note played and released inside it, its audio, and its end.
fn record_block(recorder: &mut Recorder, at: f64, origin: f64, key: u8, level: f32) {
    let duration = 64.0 / 48_000.0;
    recorder.run(at, origin);
    for event in [
        NoteEvent::on(at, SourceId::DIRECT, 0, key, 0.8),
        NoteEvent::off(at + duration / 2.0, SourceId::DIRECT, 0, key),
    ] {
        recorder.publish_note(event.into()).expect_both();
    }
    recorder.audio(&mut std::iter::repeat_n(level, 128), 128, 48_000.0);
    recorder.run_end(at + duration);
}

/// What arrives once the take has sealed goes to no file and fails nothing.
#[test]
fn what_arrives_after_the_seal_is_dropped() {
    let file = path("after-seal");
    let (mut recorder, mut capture) = testing::channel();
    capture.arm();
    recorder.is_armed();
    let mut writer = testing::FileWriter::new(&capture, file.clone());
    record_block(&mut recorder, 1.0, 0.0, 60, 0.25);
    capture.stop();
    recorder.is_armed();
    writer.stop();
    writer.drain(&mut capture);
    let sealed = writer.finished.clone().expect("sealed");
    recorder.publish_note(NoteEvent::on(2.5, SourceId::DIRECT, 0, 64, 0.8).into()).expect_both();
    recorder.params(2.5, [0.5; ParamKey::ALL.len()]);
    writer.drain(&mut capture);
    assert!(!writer.failed());
    assert_eq!(harmonigraph_take::Take::read(&sealed).unwrap().notes().count(), 2);
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

/// A forward seek under On disarm records on into the take's one WAV, from
/// where its audio had got to (#1760 R3).
#[test]
fn a_forward_seek_records_on_into_the_one_wav() {
    let file = path("seek");
    let (mut recorder, mut capture) = testing::channel();
    capture.arm();
    assert!(recorder.is_armed());
    let mut writer = testing::FileWriter::new(&capture, file.clone());
    let duration = 64.0 / 48_000.0;
    let origin = recorder.observe_transport(5.0, true, duration).unwrap();
    record_block(&mut recorder, 1.0, origin, 60, 0.25);
    let resumed = recorder.observe_transport(30.0, true, duration).unwrap();
    assert_eq!(resumed, 5.0 + duration, "the seek continues the audio");
    record_block(&mut recorder, 1.0 + duration, resumed, 72, 0.75);
    capture.stop();
    writer.stop();
    writer.drain(&mut capture);
    let sealed = writer.finished.clone().expect("sealed");
    let take = harmonigraph_take::Take::read(&sealed).unwrap();
    let starts: Vec<_> = take
        .notes()
        .filter(|n| matches!(n.kind, harmonigraph_take::NoteKind::On { .. }))
        .map(|n| (n.note, n.t))
        .collect();
    assert_eq!(starts, [(60, 5.0), (72, 5.0 + duration)]);
    let wav = sealed.with_file_name(take.header.audio_file.unwrap());
    let bytes = std::fs::read(&wav).unwrap();
    assert_eq!(bytes.len() - 44, 256 * 4, "both blocks in one WAV");
    let siblings = std::fs::read_dir(file.parent().unwrap()).unwrap().count();
    assert_eq!(siblings, 2, "one take, one WAV");
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

/// A lost publication is a gap on the file, which warns, not a failure (#712);
/// the gap reaches the file without another audio callback.
#[test]
fn a_lost_publication_is_a_gap_on_the_file() {
    let (mut recorder, mut capture) = testing::channel();
    capture.arm();
    recorder.is_armed();
    let file = path("full");
    let mut writer = testing::FileWriter::new(&capture, file.clone());
    recorder.run(0.0, 0.0);
    // One short of the ring: the last cell is reserved for the gap.
    for i in 0..publication::PUBLICATION_RING - 1 {
        let event = NoteEvent::on(i as f64 / 48000.0, SourceId::DIRECT, 0, 60, 0.8);
        recorder.publish_note(event.into()).expect_both();
    }
    assert_eq!(
        recorder.publish_note(NoteEvent::off(0.5, SourceId::DIRECT, 0, 60).into()),
        publication::Lanes::both(Err(publication::PublishError::Lost))
    );
    writer.drain(&mut capture);
    assert!(!writer.failed(), "a hole in the note history is not a recording failure");
    recorder.run_end(1.0);
    capture.stop();
    writer.stop();
    writer.drain(&mut capture);
    let take = harmonigraph_take::Take::read(writer.finished.as_ref().unwrap()).unwrap();
    let gap = take.gap().unwrap();
    assert_eq!((gap.first, gap.last), (4096, 4096));
    assert!(!take.failed);
    // Every onset published reached the file. The release that was lost left
    // the key held at the take's end, where the reader releases it (#1803).
    let onsets = take.notes().filter(|n| matches!(n.kind, harmonigraph_take::NoteKind::On { .. }));
    assert_eq!(onsets.count(), publication::PUBLICATION_RING - 1);
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

/// A block longer than the audio ring behind a stalled writer fails the take:
/// the file is marked at once, and its Stop seals it unrendered.
#[test]
fn a_failed_take_is_marked_and_sealed_unrendered() {
    let file = path("overrun");
    let (mut recorder, mut capture) = testing::channel();
    capture.arm();
    recorder.is_armed();
    let mut writer = testing::FileWriter::new(&capture, file.clone());
    recorder.run(1.0, 0.0);
    let samples = AUDIO_RING_CAPACITY + 256;
    recorder.audio(&mut std::iter::repeat_n(0.5, samples), samples, 48_000.0);
    recorder.run_end(2.0);
    writer.drain(&mut capture);
    assert!(writer.failed());
    assert!(harmonigraph_take::Take::read(&file).unwrap().failed);
    capture.stop();
    writer.stop();
    writer.drain(&mut capture);
    assert!(writer.finished.is_none(), "a failed take is never rendered");
    std::fs::remove_dir_all(file.parent().unwrap()).unwrap();
}

/// A recorder dropped mid-take fails it: nothing will close its run.
#[test]
fn a_recorder_dropped_mid_take_fails_it() {
    let (mut recorder, capture) = testing::channel();
    capture.arm();
    recorder.is_armed();
    recorder.run(0.0, 0.0);
    let probe = testing::FileWriter::new(&capture, path("dropped"));
    drop(recorder);
    assert!(probe.failed());
    std::fs::remove_dir_all(path("dropped").parent().unwrap()).unwrap();
}

/// A recorder dropped after a Stop but before a block closed its run fails the
/// take too (#1776): the disconnect would otherwise seal it unfailed, with no
/// end to its run, and render it.
#[test]
fn a_recorder_dropped_stopped_with_its_run_open_fails_it() {
    let (mut recorder, capture) = testing::channel();
    capture.arm();
    recorder.is_armed();
    recorder.run(0.0, 0.0);
    capture.stop();
    let probe = testing::FileWriter::new(&capture, path("dropped-stopped"));
    drop(recorder);
    assert!(probe.failed());
    std::fs::remove_dir_all(path("dropped-stopped").parent().unwrap()).unwrap();
}

/// The real worker: a Stop seals once the run has ended and launches the render,
/// and Start is refused until it has.
#[test]
fn the_real_worker_seals_a_stopped_take_and_renders_it() {
    let directory = path("worker").parent().unwrap().to_path_buf();
    let (mut recorder, control) = worker_in(&directory);
    control.start(String::new());
    assert!(recorder.is_armed());
    let file = || {
        std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "take"))
    };
    until(|| file().is_some());
    record_block(&mut recorder, 1.0, 0.0, 60, 0.25);
    control.stop();
    assert!(!recorder.is_armed());
    control.start(String::new());
    assert!(!control.is_recording(), "Start waits for the previous take to seal");
    until(|| control.last_take().is_some());
    assert_eq!(control.last_take(), file());
    until(|| could_not_run(&control));
    let take = harmonigraph_take::Take::read(file().unwrap()).unwrap();
    assert!(!take.failed && take.gap().is_none());
    assert_eq!(take.notes().count(), 2);
    drop(recorder);
    drop(control);
    std::fs::remove_dir_all(directory).unwrap();
}
