//! Independent exported-factory fixtures each exercise a process-wide registry
//! with its production four-Hub limit. Serialize entire fixtures, including
//! recorder injection and teardown, without locking any audio callback or
//! interfering with concurrency intentionally created inside one fixture.
use std::sync::{Mutex, MutexGuard};

static FIXTURE: Mutex<()> = Mutex::new(());

pub(crate) fn enter() -> MutexGuard<'static, ()> {
    FIXTURE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
