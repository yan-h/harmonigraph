//! Our own Rust heap, counted by the binary's global allocator.
//!
//! The process footprint the memory row also shows is the HOST's: in a shared
//! plug-in host it holds every other plugin loaded beside us (#1512). Each Rust
//! plugin binary has a global allocator of its own, so one that counts sees
//! only this binary's allocations — not another plugin's, and not what the
//! system frameworks allocate on our behalf.
//!
//! What it counts is bytes REQUESTED and not yet freed, not bytes resident: a
//! large buffer that is allocated but never touched counts in full, and the
//! allocator's own overhead and fragmentation count not at all. It is a total
//! with no owners.
//!
//! A binary opts in by installing [`Counting`] as its `#[global_allocator]`
//! over [`RUST_HEAP`]. One that does not reads [`rust_heap`] as `None`, and the
//! row leaves the heap out rather than claiming zero.

use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// Live bytes requested through a [`Counting`] allocator.
pub struct HeapCounter(AtomicUsize);

impl HeapCounter {
    pub const fn new() -> Self {
        HeapCounter(AtomicUsize::new(0))
    }

    /// Bytes requested and not yet freed.
    pub fn live(&self) -> usize {
        self.0.load(Relaxed)
    }
}

impl Default for HeapCounter {
    fn default() -> Self {
        Self::new()
    }
}

/// The counter the memory row reads. A binary's global allocator counts into
/// it; nothing else should.
pub static RUST_HEAP: HeapCounter = HeapCounter::new();

/// This binary's live Rust heap, or `None` when no [`Counting`] allocator is
/// installed over [`RUST_HEAP`].
///
/// Zero means "not installed" rather than "empty": the Rust runtime has
/// allocated before any frame is drawn, so an installed counter is never at
/// zero by the time anything reads it.
pub fn rust_heap() -> Option<u64> {
    match RUST_HEAP.live() {
        0 => None,
        bytes => Some(bytes as u64),
    }
}

/// A global allocator that counts what it hands out into a [`HeapCounter`]
/// and leaves the allocating itself to `base`.
///
/// Relaxed atomics and nothing else: no lock, no thread-local, nothing that
/// allocates. The count is a level read once a second, so it needs no ordering
/// against the memory it describes.
pub struct Counting<A> {
    base: A,
    counter: &'static HeapCounter,
}

impl<A> Counting<A> {
    pub const fn new(base: A, counter: &'static HeapCounter) -> Self {
        Counting { base, counter }
    }
}

unsafe impl<A: GlobalAlloc> GlobalAlloc for Counting<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { self.base.alloc(layout) };
        if !pointer.is_null() {
            self.counter.0.fetch_add(layout.size(), Relaxed);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { self.base.alloc_zeroed(layout) };
        if !pointer.is_null() {
            self.counter.0.fetch_add(layout.size(), Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { self.base.dealloc(pointer, layout) };
        self.counter.0.fetch_sub(layout.size(), Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let moved = unsafe { self.base.realloc(pointer, layout, size) };
        // A failed realloc leaves the old block in place, still counted.
        if !moved.is_null() {
            if size >= layout.size() {
                self.counter.0.fetch_add(size - layout.size(), Relaxed);
            } else {
                self.counter.0.fetch_sub(layout.size() - size, Relaxed);
            }
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::System;

    /// Every entry point moves the count by exactly what it requested or
    /// released, and a realloc by the difference in both directions.
    ///
    /// Driven through the trait directly, over a counter of its own: this test
    /// binary's global allocator is the system one, so nothing else moves it.
    #[test]
    fn the_count_follows_every_alloc_dealloc_and_realloc() {
        static COUNTER: HeapCounter = HeapCounter::new();
        let heap = Counting::new(System, &COUNTER);
        let layout = |size| Layout::from_size_align(size, 16).unwrap();
        unsafe {
            let a = heap.alloc(layout(1000));
            assert_eq!(COUNTER.live(), 1000, "alloc");
            let b = heap.alloc_zeroed(layout(24));
            assert_eq!(COUNTER.live(), 1024, "alloc_zeroed");
            let a = heap.realloc(a, layout(1000), 4000);
            assert_eq!(COUNTER.live(), 4024, "realloc that grows");
            let a = heap.realloc(a, layout(4000), 100);
            assert_eq!(COUNTER.live(), 124, "realloc that shrinks");
            heap.dealloc(b, layout(24));
            assert_eq!(COUNTER.live(), 100, "dealloc");
            heap.dealloc(a, layout(100));
            assert_eq!(COUNTER.live(), 0, "everything returned");
        }
    }
}
