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
    fn queued(name: &str) -> Self {
        let directory = std::env::temp_dir()
            .join(format!("harmonigraph-boundary-{}-{name}", std::process::id()));
        let (recorder, control) = channel();
        let fence = control.fence.clone();
        *fence.test_directory.lock() = Some(directory.clone());
        let worker = Self { recorder: Some(recorder), control: Some(control), fence, directory };
        worker.control.as_ref().unwrap().start(48_000.0, String::new());
        wait_for("Start", || worker.find_wav().is_some());
        worker.fence.worker_before_commands.enabled.store(true, Ordering::Release);
        wait_for("before command poll", || {
            worker.fence.worker_before_commands.entered.load(Ordering::Acquire)
        });
        worker
    }

    fn start(name: &str, fail_finish: bool, pending_start: bool) -> Self {
        let directory = std::env::temp_dir()
            .join(format!("harmonigraph-audio-{}-{name}-{pending_start}", std::process::id()));
        let (recorder, control) = channel();
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
        worker.control.as_ref().unwrap().start(48_000.0, String::new());
        if !pending_start {
            fence.worker_after_empty.enabled.store(false, Ordering::Release);
            wait_for("Start opened WAV", || worker.find_wav().is_some());
        }
        let recorder = worker.recorder.as_mut().unwrap();
        assert_no_alloc(|| {
            assert!(recorder.is_armed());
            recorder.mark_audio_start(0.25);
            recorder.audio(&mut PREFIX.into_iter(), PREFIX.len());
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
        self.control.as_ref().unwrap().stop(RenderRequest {
            program,
            appearance: None,
            size: Some([16, 16]),
            notice: None,
        });
        assert_no_alloc(|| self.close(1));
        // A failure may already be accounted before Stop. Wait for this
        // command's worker iteration too, so its render request is exercised.
        wait_for("Stop processed", || self.fence.worker_stop_processed.load(Ordering::Acquire));
    }

    /// What the configuration owner and the Hub publish once a stopped take's
    /// next callback observes the disarm: every lane closes over the first
    /// `passes` passes of the one epoch these fixtures record.
    fn close(&mut self, passes: u32) {
        let recorder = self.recorder.as_mut().unwrap();
        assert!(!recorder.is_armed());
        for pass in 1..=passes {
            let address = RecordAddress { epoch: 1, pass };
            recorder.configuration_pass_complete(address);
            recorder.source_pass_complete(address, 1.0);
        }
        recorder.configuration_epoch_complete(1);
        recorder.source_epoch_complete(1, 1.0);
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
        let render = self.control.as_ref().map(|control| control.render.clone());
        self.control.take();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !self.fence.worker_finished.load(Ordering::Acquire)
            && std::time::Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if let Some(render) = render {
            render.shutdown();
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
fn queued_stop_preserves_the_audio_prefix() {
    let mut worker = Worker::queued("queued-stop");
    let recorder = worker.recorder.as_mut().unwrap();
    assert_no_alloc(|| {
        assert!(recorder.is_armed());
        recorder.mark_audio_start(0.25);
        recorder.audio(&mut PREFIX.into_iter(), PREFIX.len());
    });
    worker.control.as_ref().unwrap().stop(unlaunchable_render(&worker.directory));
    worker.close(1);
    worker.fence.worker_before_commands.enabled.store(false, Ordering::Release);
    wait_for("queued Stop", || worker.control.as_ref().unwrap().last_take().is_some());
    assert_eq!(wav_samples(&worker.wav()), PREFIX);
}

#[test]
fn queued_rollover_keeps_each_pass_audio() {
    let mut worker = Worker::queued("queued-rollover");
    let first = worker.wav();
    let recorder = worker.recorder.as_mut().unwrap();
    assert_no_alloc(|| {
        assert!(recorder.is_armed());
        assert!(recorder.observe_transport(1.0, true, 64.0 / 48_000.0));
        recorder.mark_audio_start(1.0);
        recorder.audio(&mut PREFIX.into_iter(), PREFIX.len());
        assert!(recorder.is_armed());
        assert!(recorder.observe_transport(0.0, true, 64.0 / 48_000.0));
        recorder.mark_audio_start(0.0);
        recorder.audio(&mut [0.75, -0.75].into_iter(), 2);
    });
    worker.fence.worker_before_commands.enabled.store(false, Ordering::Release);
    let second =
        first.with_file_name(format!("{}-2.wav", first.file_stem().unwrap().to_str().unwrap()));
    wait_for("loop audio written", || std::fs::metadata(&second).is_ok_and(|m| m.len() >= 52));
    worker.control.as_ref().unwrap().stop(unlaunchable_render(&worker.directory));
    worker.close(2);
    wait_for("rollover Stop", || worker.control.as_ref().unwrap().last_take().is_some());
    assert_eq!((wav_samples(&first), wav_samples(&second)), (PREFIX.to_vec(), vec![0.75, -0.75]));
}

#[test]
fn wav_limit_is_reported_and_never_renders_an_incomplete_take() {
    for pending_start in [false, true] {
        let mut worker = Worker::start("size-limit", false, pending_start);
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

#[test]
fn wav_finalization_failure_is_accounted_without_producer_disconnect() {
    let mut worker = Worker::start("finalize", true, true);
    worker.stop_with_render();
    worker.assert_failed("WAV finalization failure");
}

/// A one-file trigger ends the contiguous prefix at a seek and explains that
/// cutoff without poisoning recording ownership or hiding renderer warnings.
#[test]
fn a_forward_seek_finishes_one_file_with_a_notice_and_allows_another_take() {
    use std::os::unix::fs::PermissionsExt;
    for (paused, stop_before_seek) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut worker = Worker::queued(&format!("seek-notice-{paused}-{stop_before_seek}"));
        let control = worker.control.as_ref().unwrap();
        control.set_end_at_rewind(true);
        let program = worker.directory.join("renderer");
        // spawn_render passes take, --out, output, --size, dimensions.
        std::fs::write(
            &program,
            "#!/bin/sh\nprintf 'video' > \"$3\"\nprintf 'warning: fixture warning\\n' >&2\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let request = || RenderRequest {
            program: program.clone(),
            appearance: None,
            size: Some([16, 16]),
            notice: None,
        };
        let recorder = worker.recorder.as_mut().unwrap();
        let duration = 64.0 / 48_000.0;
        assert_no_alloc(|| {
            assert!(recorder.is_armed());
            assert!(recorder.observe_transport(0.0, true, duration));
            recorder.mark_audio_start(0.0);
            recorder.audio(&mut std::iter::repeat_n(0.25, 128), 128);
            if paused {
                assert!(!recorder.observe_transport(0.0, false, duration));
                assert!(!recorder.observe_transport(10.0, false, duration));
                assert!(!recorder.observe_transport(10.0, false, duration));
            }
            // This callback was admitted before the GUI Stop. It owns its
            // transport observation even if the intent changes meanwhile.
            assert!(recorder.is_armed());
        });
        if stop_before_seek {
            assert!(!control.has_ended(), "Stop has no completion reason to snapshot yet");
            control.stop(request());
        }
        assert_no_alloc(|| {
            assert!(!recorder.observe_transport(10.0, true, duration));
            assert!(recorder.configuration_address().is_none());
        });
        assert!(control.has_ended());
        assert_eq!(control.latches.end(), Some(End::ForwardSeek));
        if !stop_before_seek {
            control.stop(request());
        }
        assert_no_alloc(|| worker.close(1));
        worker.fence.worker_before_commands.enabled.store(false, Ordering::Release);
        wait_for("successful seek render", || {
            worker.control.as_ref().unwrap().status().starts_with("rendered ")
        });
        let control = worker.control.as_ref().unwrap();
        let status = control.status();
        assert!(status.contains("take ended before a forward transport seek"), "{status}");
        assert!(status.contains("warning: fixture warning"), "{status}");
        assert!(!worker.fence.failed.load(Ordering::Acquire));
        let take_path = control.last_take().unwrap();
        let take = harmonigraph_take::Take::read(&take_path).unwrap();
        assert!(take.incomplete.is_none());
        assert_eq!(take.header.audio_start, Some(0.0));
        assert_eq!(wav_samples(&take_path.with_extension("wav")), vec![0.25; 128]);
        assert!(!Pass::path_for(&take_path, 2).exists());
        // Successful completion releases the ordinary Start gate. A gesture
        // must not leave a permanent failure requiring the plugin to reload.
        control.start(48_000.0, String::new());
        assert!(control.is_recording(), "{}", control.status());
        assert!(!control.has_ended());
        let recorder = worker.recorder.as_mut().unwrap();
        assert!(recorder.is_armed());
        let next = recorder.configuration_address().unwrap();
        control.stop(unlaunchable_render(&worker.directory));
        assert_no_alloc(|| {
            assert!(!recorder.is_armed());
            recorder.configuration_pass_complete(next);
            recorder.source_pass_complete(next, 12.0);
            recorder.configuration_epoch_complete(next.epoch);
            recorder.source_epoch_complete(next.epoch, 12.0);
        });
        wait_for("second take finalized", || !worker.fence.finishing.load(Ordering::Acquire));
        assert!(!worker.fence.failed.load(Ordering::Acquire));
    }
}
