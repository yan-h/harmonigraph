//! Internal recording addresses and completion fences. Runtime identities never
//! enter the take format. Stop is intent; the producer and any optional
//! configuration/source owners must close their prefixes before finalization.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub const RECORD_PASSES: usize = 128;

// Ordinary callbacks and Stop share one atomic order. The exposed intent
// keeps its existing epoch/armed shape; this private bit is never returned.
pub(crate) const CALLBACK_ACTIVE: u64 = 1 << 63;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordAddress {
    pub epoch: u64,
    pub pass: u32,
}

#[derive(Default)]
pub(crate) struct RecordFence {
    pub enabled: AtomicBool,
    pub canonical_enabled: AtomicBool,
    /// Epoch in bits 1..62, armed in bit zero; CALLBACK_ACTIVE arbitrates
    /// ordinary callbacks with Stop without changing the exposed intent shape.
    pub intent: AtomicU64,
    pub finishing: AtomicBool,
    pub failed: AtomicBool,
    /// First worker-side failure detail. Audio-thread failures only set `failed`.
    pub failure_message: parking_lot::Mutex<Option<String>>,
    pub configuration_closed: AtomicU64,
    pub source_closed: AtomicU64,
    /// The joined plugin still owns actual source history not yet published.
    /// Only its retired publication owner may release this failure-close hold.
    pub retirement_hold: AtomicBool,
    #[cfg(feature = "test-support")]
    pub worker_after_empty: TestPause,
    #[cfg(feature = "test-support")]
    pub worker_before_commands: TestPause,
    #[cfg(feature = "test-support")]
    pub worker_after_stop: TestPause,
    #[cfg(feature = "test-support")]
    pub worker_finished: AtomicBool,
    #[cfg(feature = "test-support")]
    pub worker_empty_visits: AtomicU64,
    #[cfg(feature = "test-support")]
    pub worker_before_retirement_check: TestPause,
    #[cfg(all(test, feature = "test-support"))]
    pub worker_failure_accounted: AtomicBool,
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
    #[cfg(feature = "test-support")]
    pub producer_close_pause: TestPause,
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
    pub fn epoch(&self) -> u64 {
        (self.intent.load(Ordering::Acquire) & !CALLBACK_ACTIVE) >> 1
    }
    pub fn fail(&self) {
        self.failed.store(true, Ordering::Release);
    }

    /// Worker only: preserve the concrete cause across GUI status refreshes.
    pub fn fail_with_message(&self, message: String) {
        self.failure_message
            .lock()
            .get_or_insert_with(|| format!("recording incomplete: {message}; no render started"));
        self.fail();
    }
}
