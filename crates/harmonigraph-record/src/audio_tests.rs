//! Real-worker audio failures, independent of note-publication recovery.

use super::*;

struct Worker {
    recorder: Option<Recorder>,
    control: Option<Control>,
    fence: Arc<RecordFence>,
    directory: std::path::PathBuf,
}

impl Worker {
    fn start(name: &str, fenced: bool, fail_finish: bool) -> Self {
        let directory = std::env::temp_dir()
            .join(format!("harmonigraph-audio-{}-{name}-{fenced}", std::process::id()));
        let (mut recorder, control) = channel();
        if fenced {
            recorder.enable_configuration();
        }
        let fence = control.fence.clone();
        *fence.test_directory.lock() = Some(directory.clone());
        *fence.test_wav_limit.lock() = Some(2);
        fence.test_wav_finish_failure.store(fail_finish, Ordering::Release);
        control.start(48_000.0, String::new(), true);
        assert!(recorder.is_armed());
        recorder.mark_audio_start(0.0);
        recorder.audio(&mut [0.5, -0.5, 0.25, -0.25].into_iter(), 4);
        let worker = Self { recorder: Some(recorder), control: Some(control), fence, directory };
        worker.settle();
        assert!(!worker.fence.failed.load(Ordering::Acquire), "valid prefix was written");
        worker
    }

    fn settle(&self) {
        let visits = self.fence.worker_empty_visits.load(Ordering::Acquire);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while self.fence.worker_empty_visits.load(Ordering::Acquire) < visits + 2 {
            assert!(std::time::Instant::now() < deadline, "recording worker stalled");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
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
            ui_state: None,
            size: [16, 16],
            playhead: None,
        }));
        let recorder = self.recorder.as_mut().unwrap();
        assert!(!recorder.is_armed());
        if self.fence.enabled.load(Ordering::Acquire) {
            recorder.configuration_pass_complete(RecordAddress { epoch: 1, pass: 1 });
            recorder.configuration_epoch_complete(1);
        }
        self.settle();
    }

    fn assert_failed(&self, cause: &str) {
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
        std::fs::read_dir(&self.directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|extension| extension == "wav"))
            .unwrap()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
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

#[test]
fn wav_limit_is_reported_and_never_renders_an_incomplete_take() {
    for fenced in [false, true] {
        let mut worker = Worker::start("size-limit", fenced, false);
        worker.recorder.as_mut().unwrap().audio(&mut [1.0, -1.0].into_iter(), 2);
        worker.settle();
        worker.assert_failed("RIFF size limit");
        worker.stop_with_render();
        worker.assert_failed("RIFF size limit");
        let bytes = std::fs::read(worker.wav()).unwrap();
        assert_eq!(bytes.len(), 44 + 16);
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 36 + 16);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 16);
        assert_eq!(f32::from_le_bytes(bytes[44..48].try_into().unwrap()), 0.5);
    }
}

#[test]
fn wav_finalization_failure_is_accounted_without_producer_disconnect() {
    for fenced in [false, true] {
        let mut worker = Worker::start("finalize", fenced, true);
        worker.stop_with_render();
        worker.assert_failed("WAV finalization failure");
    }
}
