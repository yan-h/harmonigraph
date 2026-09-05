//! Internal recording addresses and completion fences. Runtime identities never
//! enter the take format. A stop is intent; the producer and configuration owner
//! must both close its actual prefix before the writer can publish a finished file.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub const RECORD_PASSES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordAddress {
    pub epoch: u64,
    pub pass: u32,
}

#[derive(Default)]
pub(crate) struct RecordFence {
    pub enabled: AtomicBool,
    pub canonical_enabled: AtomicBool,
    /// One coherent off-thread intent: epoch in the high bits, armed in bit zero.
    pub intent: AtomicU64,
    pub finishing: AtomicBool,
    pub failed: AtomicBool,
    pub configuration_closed: AtomicU64,
    pub source_closed: AtomicU64,
    #[cfg(all(test, feature = "test-support"))]
    pub worker_after_empty: TestPause,
    #[cfg(all(test, feature = "test-support"))]
    pub worker_after_stop: TestPause,
    #[cfg(all(test, feature = "test-support"))]
    pub worker_finished: AtomicBool,
    #[cfg(all(test, feature = "test-support"))]
    pub test_directory: parking_lot::Mutex<Option<std::path::PathBuf>>,
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
        self.intent.load(Ordering::Acquire) >> 1
    }
    pub fn fail(&self) {
        self.failed.store(true, Ordering::Release);
    }
}
