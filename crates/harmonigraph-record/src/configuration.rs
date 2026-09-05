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
    /// One coherent off-thread intent: epoch in the high bits, armed in bit zero.
    pub intent: AtomicU64,
    pub finishing: AtomicBool,
    pub failed: AtomicBool,
    pub configuration_closed: AtomicU64,
}
impl RecordFence {
    pub fn epoch(&self) -> u64 {
        self.intent.load(Ordering::Acquire) >> 1
    }
    pub fn capture(&self) -> Option<u64> {
        let intent = self.intent.load(Ordering::Acquire);
        (intent & 1 != 0).then_some(intent >> 1)
    }
    pub fn fail(&self) {
        self.failed.store(true, Ordering::Release);
    }
}
