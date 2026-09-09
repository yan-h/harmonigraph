//! Independent exported-factory fixtures each exercise the one process-wide
//! tuning session. Serialize entire fixtures, including recorder injection and
//! teardown, without locking any audio callback or interfering with
//! concurrency intentionally created inside one fixture.
use std::sync::{Mutex, MutexGuard};

static FIXTURE: Mutex<()> = Mutex::new(());

pub(crate) fn enter() -> MutexGuard<'static, ()> {
    let guard = FIXTURE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // A panicking fixture can leave a row claimed by a Tune that never reached
    // its destructor. The session outlives every fixture, so the next one
    // starts from an empty one rather than from whatever that left behind.
    crate::tuning::session::session().test_reset();
    guard
}
