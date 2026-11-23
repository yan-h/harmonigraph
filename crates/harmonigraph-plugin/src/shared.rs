//! A saved document the main thread owns and audio copies (#1938).
//!
//! The lock stays private, so no guard outlives the closure handed to it, and
//! the door keeps one rule the type cannot spell: nothing inside a closure
//! calls the host or wakes it. A host may save from inside a call a frame
//! makes (`editor/lock.rs`), and that save reads the document on the same
//! thread; the lock is not reentrant, so a guard held across the host call
//! hangs the host's main thread. A closure still captures its scope, so a
//! per-thread depth checks the rule in debug builds: a door entered while
//! another is held, or a host call made from inside one
//! ([`assert_unheld`]), fails an assertion.

use parking_lot::RwLock;
#[cfg(debug_assertions)]
use std::cell::Cell;

#[cfg(debug_assertions)]
thread_local! {
    /// How many document closures this thread is inside; debug builds only.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Inside a document closure until dropped. Counts in debug builds alone, so
/// a release build's doors, audio's among them, pay nothing for the check.
struct Held;
impl Held {
    fn enter() -> Self {
        #[cfg(debug_assertions)]
        DEPTH.with(|depth| {
            assert_eq!(depth.get(), 0, "a document door entered inside another's closure");
            depth.set(depth.get() + 1);
        });
        Held
    }
}
impl Drop for Held {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        DEPTH.with(|depth| depth.set(depth.get() - 1));
    }
}

/// Called before a call into the host: none is made from inside a document
/// closure. Debug builds only.
pub(crate) fn assert_unheld() {
    #[cfg(debug_assertions)]
    assert_eq!(DEPTH.with(Cell::get), 0, "a host call from inside a document closure");
}

#[derive(Debug, Default)]
pub(crate) struct Shared<T> {
    lock: RwLock<T>,
}

impl<T> Shared<T> {
    /// Read the document. Main thread or the editor's threads, never audio.
    pub fn read<R>(&self, read: impl FnOnce(&T) -> R) -> R {
        let _held = Held::enter();
        read(&self.lock.read())
    }

    /// Change the document. The caller marks the project modified, or wakes
    /// the host, once this has returned.
    pub fn write<R>(&self, write: impl FnOnce(&mut T) -> R) -> R {
        let _held = Held::enter();
        write(&mut self.lock.write())
    }

    /// Audio's read: `None` while a writer holds the document, when the
    /// callback keeps what it copied last.
    pub fn try_read<R>(&self, read: impl FnOnce(&T) -> R) -> Option<R> {
        let _held = Held::enter();
        self.lock.try_read().map(|document| read(&document))
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    /// A door entered inside another's closure, which would hang were it a
    /// write, fails the check rather than reaching the lock.
    #[test]
    #[should_panic(expected = "a document door entered inside another's closure")]
    fn a_nested_door_fails_the_depth_check() {
        let document = Shared::<u8>::default();
        document.read(|_| document.read(|_| ()));
    }

    /// So does a host call from inside one; outside, it passes.
    #[test]
    fn a_host_call_from_inside_a_closure_fails_the_depth_check() {
        let document = Shared::<u8>::default();
        assert_unheld();
        let inside = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            document.write(|_| assert_unheld())
        }));
        assert!(inside.is_err());
        assert_unheld();
    }
}
