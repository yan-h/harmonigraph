//! The arm intent and the failure flag.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// What the GUI last asked of the audio thread: arm a take epoch, or disarm it.
///
/// Packed into one word — the epoch above bit zero, armed in bit zero — so
/// [`RecordFence`] publishes both in a single atomic store the audio thread
/// reads without a lock. This type is the only place that knows the packing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecordIntent(u64);

impl RecordIntent {
    /// The largest epoch the packing has room for.
    pub const MAX_EPOCH: u64 = u64::MAX >> 1;
    const ARMED: u64 = 1;

    pub const fn armed(epoch: u64) -> Self {
        Self(epoch << 1 | Self::ARMED)
    }
    pub const fn epoch(self) -> u64 {
        self.0 >> 1
    }
    pub const fn is_armed(self) -> bool {
        self.0 & Self::ARMED != 0
    }
}

#[derive(Default)]
pub(crate) struct RecordFence {
    /// A [`RecordIntent`], reached only through the methods below.
    intent: AtomicU64,
    pub finishing: AtomicBool,
    pub failed: AtomicBool,
    /// First worker-side failure detail. Audio-thread failures only set `failed`.
    pub failure_message: parking_lot::Mutex<Option<String>>,
    #[cfg(feature = "test-support")]
    pub worker_after_empty: TestPause,
    #[cfg(feature = "test-support")]
    pub worker_before_commands: TestPause,
    #[cfg(feature = "test-support")]
    pub worker_finished: AtomicBool,
    #[cfg(all(test, feature = "test-support"))]
    pub worker_stop_processed: AtomicBool,
    #[cfg(feature = "test-support")]
    pub test_directory: parking_lot::Mutex<Option<std::path::PathBuf>>,
    #[cfg(feature = "test-support")]
    pub test_wav_limit: parking_lot::Mutex<Option<u64>>,
    #[cfg(feature = "test-support")]
    pub test_wav_finish_failure: AtomicBool,
    #[cfg(feature = "test-support")]
    pub boundary_pause: TestPause,
}

#[cfg(feature = "test-support")]
#[derive(Default)]
pub(crate) struct TestPause {
    pub enabled: AtomicBool,
    pub entered: AtomicBool,
}
#[cfg(feature = "test-support")]
impl TestPause {
    pub fn reach(&self) {
        if self.enabled.load(Ordering::Acquire) {
            self.entered.store(true, Ordering::Release);
            while self.enabled.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }
        }
    }
}
impl RecordFence {
    pub fn intent(&self) -> RecordIntent {
        RecordIntent(self.intent.load(Ordering::Acquire))
    }
    pub fn epoch(&self) -> u64 {
        self.intent().epoch()
    }
    pub fn arm(&self, epoch: u64) {
        self.intent.store(RecordIntent::armed(epoch).0, Ordering::Release);
    }
    /// Clear the armed bit and keep the epoch, whatever it is by now.
    pub fn disarm(&self) {
        self.intent.fetch_and(!RecordIntent::ARMED, Ordering::AcqRel);
    }
    pub fn fail(&self) {
        self.failed.store(true, Ordering::Release);
    }

    /// What the status line says of the failure: the worker's cause, or the
    /// generic line for one the audio thread found.
    pub fn failure_status(&self) -> String {
        let message = self.failure_message.lock().clone();
        message.unwrap_or_else(|| crate::recorder::CONFIGURATION_FAILURE.into())
    }

    /// The next take starts with no failure: Start only, once the last take
    /// has sealed.
    pub fn clear_failure(&self) {
        *self.failure_message.lock() = None;
        self.failed.store(false, Ordering::Release);
    }

    /// Worker only: preserve the concrete cause across GUI status refreshes.
    pub fn fail_with_message(&self, message: String) {
        self.failure_message
            .lock()
            .get_or_insert_with(|| format!("recording incomplete: {message}; no render started"));
        self.fail();
    }
}
