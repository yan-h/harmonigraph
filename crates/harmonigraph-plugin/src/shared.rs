//! A saved document the main thread owns (#1938): the tuning record and the
//! Lattice Map document. Audio never takes its lock: a reader's release can
//! take the parking bucket's lock while a writer is parked, so what audio
//! needs is published to it wait-free, by the document's owner, inside the
//! write.
//!
//! The lock stays private, so no guard outlives the closure handed to it, and
//! the door keeps one rule the type cannot spell: nothing inside a closure
//! calls the host or wakes it. A host may save from inside a call a frame
//! makes (`editor/lock.rs`), and that save reads the document on the same
//! thread; the lock is not reentrant, so a guard held across the host call
//! hangs the host's main thread. A closure still captures its scope, so a
//! per-thread depth checks the rule in debug builds: a door entered while
//! another is held, or a host call made from inside one (every call through
//! [`Checked`], the editor's one host, and every wake), fails an assertion.

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

/// The editor's host, checking before each call that no document door is
/// held ([`assert_unheld`]).
pub(crate) struct Checked(pub std::sync::Arc<dyn harmonigraph_clap::prelude::EditorHost>);
impl harmonigraph_clap::prelude::EditorHost for Checked {
    fn closed(&self) -> bool {
        assert_unheld();
        self.0.closed()
    }
    fn request_resize(&self, size: (u32, u32)) -> bool {
        assert_unheld();
        self.0.request_resize(size)
    }
    fn begin_set_parameter(&self, row: usize) {
        assert_unheld();
        self.0.begin_set_parameter(row)
    }
    fn set_parameter(&self, row: usize, plain: f32) {
        assert_unheld();
        self.0.set_parameter(row, plain)
    }
    fn end_set_parameter(&self, row: usize) {
        assert_unheld();
        self.0.end_set_parameter(row)
    }
}

#[derive(Debug, Default)]
pub(crate) struct Shared<T> {
    lock: RwLock<T>,
}

impl<T> Shared<T> {
    pub fn new(value: T) -> Self {
        Self { lock: RwLock::new(value) }
    }

    /// Read the document, off audio: the main thread, a frame or the input
    /// worker.
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
}
