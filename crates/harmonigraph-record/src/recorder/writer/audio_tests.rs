//! Real-worker audio failures, independent of note-publication recovery.

use super::*;
use nice_assert_no_alloc::assert_no_alloc;

#[global_allocator]
static ALLOCATOR: nice_assert_no_alloc::AllocDisabler = nice_assert_no_alloc::AllocDisabler;

const PREFIX: [f32; 4] = [0.5, -0.5, 0.25, -0.25];

fn wait_for(description: &str, ready: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready() {
        assert!(std::time::Instant::now() < deadline, "recording worker stalled: {description}");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

struct Worker {
    recorder: Option<Recorder>,
    control: Option<Control>,
    fence: Arc<RecordFence>,
    directory: std::path::PathBuf,
}

impl Worker {
    fn ordinary(name: &str) -> Self {
        let directory = std::env::temp_dir()
            .join(format!("harmonigraph-boundary-{}-{name}", std::process::id()));
        let (recorder, control) = channel();
        let fence = control.fence.clone();
        *fence.test_directory.lock() = Some(directory.clone());
        let worker = Self { recorder: Some(recorder), control: Some(control), fence, directory };
        worker.control.as_ref().unwrap().start(48_000.0, String::new(), true);
        wait_for("ordinary Start", || worker.find_wav().is_some());
        worker.fence.worker_before_commands.enabled.store(true, Ordering::Release);
        wait_for("before command poll", || {
            worker.fence.worker_before_commands.entered.load(Ordering::Acquire)
        });
        worker
    }

    fn start(name: &str, fenced: bool, fail_finish: bool, pending_start: bool) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "harmonigraph-audio-{}-{name}-{fenced}-{pending_start}",
            std::process::id()
        ));
        let (recorder, control) = channel();
        if fenced {
            recorder.enable_configuration();
        }
        let fence = control.fence.clone();
        *fence.test_directory.lock() = Some(directory.clone());
        *fence.test_wav_limit.lock() = Some(2);
        fence.test_wav_finish_failure.store(fail_finish, Ordering::Release);
        let mut worker =
            Self { recorder: Some(recorder), control: Some(control), fence, directory };
        let fence = &worker.fence;
        // Park AFTER the empty command poll. In the pending case, the next
        // drain must see both the audio and its alignment before it sees Start.
        fence.worker_after_empty.enabled.store(true, Ordering::Release);
        wait_for("empty command poll", || fence.worker_after_empty.entered.load(Ordering::Acquire));
        worker.control.as_ref().unwrap().start(48_000.0, String::new(), true);
        if !pending_start {
            fence.worker_after_empty.enabled.store(false, Ordering::Release);
            wait_for("Start opened WAV", || worker.find_wav().is_some());
        }
        let recorder = worker.recorder.as_mut().unwrap();
        assert_no_alloc(|| {
            assert!(recorder.is_armed());
            recorder.mark_audio_start(0.25);
            recorder.audio(&mut PREFIX.into_iter(), PREFIX.len());
            if !fenced {
                recorder.finish_callback();
            }
        });
        fence.worker_after_empty.enabled.store(false, Ordering::Release);
        // The limit only becomes reachable once the two-frame prefix is on
        // disk. Empty command visits alone do not prove that it was retained.
        wait_for("two-frame WAV prefix", || {
            worker.find_wav().is_some_and(|path| std::fs::metadata(path).unwrap().len() == 60)
        });
        worker.assert_prefix();
        assert!(!worker.fence.failed.load(Ordering::Acquire), "valid prefix was written");
        worker
    }

    fn stop_with_render(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let program = self.directory.join("renderer");
        std::fs::write(&program, "#!/bin/sh\ntouch \"$0.invoked\"\n").unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        self.control.as_ref().unwrap().stop(Some(RenderRequest {
            program,
            audio: None,
            align: None,
            appearance: None,
            size: [16, 16],
            playhead: None,
        }));
        let recorder = self.recorder.as_mut().unwrap();
        assert_no_alloc(|| {
            assert!(!recorder.is_armed());
            if self.fence.enabled.load(Ordering::Acquire) {
                recorder.configuration_pass_complete(RecordAddress { epoch: 1, pass: 1 });
                recorder.configuration_epoch_complete(1);
            } else {
                recorder.finish_callback();
            }
        });
        // A failure may already be accounted before Stop. Wait for this
        // command's worker iteration too, so its render request is exercised.
        wait_for("Stop processed", || self.fence.worker_stop_processed.load(Ordering::Acquire));
    }

    fn assert_failed(&self, cause: &str) {
        wait_for("failure accounted", || {
            self.fence.worker_failure_accounted.load(Ordering::Acquire)
        });
        let control = self.control.as_ref().unwrap();
        assert!(self.fence.failed.load(Ordering::Acquire));
        assert!(self.fence.worker_failure_accounted.load(Ordering::Acquire));
        assert!(!self.fence.finishing.load(Ordering::Acquire));
        assert!(!self.recorder.as_ref().unwrap().wants_audio());
        let message = control.status();
        assert!(
            message.contains(cause)
                && message.contains(".wav")
                && message.contains("no render started"),
            "{message}"
        );
        control.tick(true, 100);
        assert_eq!(control.status(), message, "GUI refresh preserves the actual error");
        assert!(control.last_take().is_none());
        assert!(!self.directory.join("renderer.invoked").exists());
    }

    fn wav(&self) -> std::path::PathBuf {
        self.find_wav().expect("worker opened WAV")
    }

    fn find_wav(&self) -> Option<std::path::PathBuf> {
        std::fs::read_dir(&self.directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|extension| extension == "wav"))
    }

    fn assert_prefix(&self) {
        let bytes = std::fs::read(self.wav()).unwrap();
        let samples: Vec<_> = bytes[44..]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(samples, PREFIX, "WAV preserves every initial sample");
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.fence.worker_before_commands.enabled.store(false, Ordering::Release);
        self.fence.worker_after_empty.enabled.store(false, Ordering::Release);
        self.recorder.take();
        self.control.take();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !self.fence.worker_finished.load(Ordering::Acquire)
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn wav_samples(path: &std::path::Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize, bytes.len() - 44);
    bytes[44..].chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect()
}

#[test]
fn ordinary_queued_stop_preserves_the_audio_prefix() {
    let mut worker = Worker::ordinary("queued-stop");
    let recorder = worker.recorder.as_mut().unwrap();
    assert_no_alloc(|| {
        assert!(recorder.is_armed());
        recorder.mark_audio_start(0.25);
        recorder.audio(&mut PREFIX.into_iter(), PREFIX.len());
        recorder.finish_callback();
    });
    worker.control.as_ref().unwrap().stop(None);
    worker.fence.worker_before_commands.enabled.store(false, Ordering::Release);
    wait_for("ordinary Stop", || worker.control.as_ref().unwrap().last_take().is_some());
    assert_eq!(wav_samples(&worker.wav()), PREFIX);
}

#[test]
fn ordinary_queued_rollover_keeps_each_pass_audio() {
    let mut worker = Worker::ordinary("queued-rollover");
    let first = worker.wav();
    let recorder = worker.recorder.as_mut().unwrap();
    assert_no_alloc(|| {
        assert!(recorder.is_armed());
        assert!(recorder.observe_transport(1.0, true, 64.0 / 48_000.0));
        recorder.mark_audio_start(1.0);
        recorder.audio(&mut PREFIX.into_iter(), PREFIX.len());
        recorder.finish_callback();
        assert!(recorder.is_armed());
        assert!(recorder.observe_transport(0.0, true, 64.0 / 48_000.0));
        recorder.mark_audio_start(0.0);
        recorder.audio(&mut [0.75, -0.75].into_iter(), 2);
        recorder.finish_callback();
    });
    worker.fence.worker_before_commands.enabled.store(false, Ordering::Release);
    let second =
        first.with_file_name(format!("{}-2.wav", first.file_stem().unwrap().to_str().unwrap()));
    wait_for("loop audio written", || std::fs::metadata(&second).is_ok_and(|m| m.len() >= 52));
    worker.control.as_ref().unwrap().stop(None);
    wait_for("rollover Stop", || worker.control.as_ref().unwrap().last_take().is_some());
    assert_eq!((wav_samples(&first), wav_samples(&second)), (PREFIX.to_vec(), vec![0.75, -0.75]));
}

#[test]
fn wav_limit_is_reported_and_never_renders_an_incomplete_take() {
    for fenced in [false, true] {
        for pending_start in [false, true] {
            let mut worker = Worker::start("size-limit", fenced, false, pending_start);
            assert_no_alloc(|| {
                worker.recorder.as_mut().unwrap().audio(&mut [1.0, -1.0].into_iter(), 2)
            });
            worker.assert_failed("RIFF size limit");
            worker.stop_with_render();
            worker.assert_failed("RIFF size limit");
            let bytes = std::fs::read(worker.wav()).unwrap();
            assert_eq!(bytes.len(), 44 + 16);
            assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 36 + 16);
            assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 16);
            worker.assert_prefix();
            let take = harmonigraph_take::Take::read(worker.wav().with_extension("take")).unwrap();
            assert_eq!(take.header.audio_start, Some(0.25), "pending Start retains alignment too");
        }
    }
}

#[test]
fn wav_finalization_failure_is_accounted_without_producer_disconnect() {
    for fenced in [false, true] {
        let mut worker = Worker::start("finalize", fenced, true, true);
        worker.stop_with_render();
        worker.assert_failed("WAV finalization failure");
    }
}
