//! Off-audio fixture accounting. The underlying allocation guard still checks
//! every allocation/deallocation; production builds do not include this probe.
use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
#[cfg(feature = "assert_process_allocs")]
use nice_assert_no_alloc::AllocDisabler as BaseAllocator;
#[cfg(not(feature = "assert_process_allocs"))]
use std::alloc::System as BaseAllocator;

#[derive(Clone, Copy, Debug, Default)]
pub struct AllocationMeasure {
    pub allocated: usize,
    pub deallocated: usize,
    pub allocations: usize,
    pub deallocations: usize,
    pub peak: isize,
    pub retained: isize,
}
thread_local! {
    static MEASURE: Cell<Option<AllocationMeasure>> = const { Cell::new(None) };
}
fn account(allocate: usize, deallocate: usize, allocations: usize, deallocations: usize) {
    let _ = MEASURE.try_with(|cell| {
        if let Some(mut value) = cell.get() {
            value.allocated += allocate; value.deallocated += deallocate;
            value.allocations += allocations; value.deallocations += deallocations;
            value.retained += allocate as isize - deallocate as isize;
            value.peak = value.peak.max(value.retained);
            cell.set(Some(value));
        }
    });
}
pub(crate) struct MeasuredAllocator;
unsafe impl GlobalAlloc for MeasuredAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { BaseAllocator.alloc(layout) };
        if !pointer.is_null() { account(layout.size(), 0, 1, 0); }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { BaseAllocator.alloc_zeroed(layout) };
        if !pointer.is_null() { account(layout.size(), 0, 1, 0); }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { BaseAllocator.dealloc(pointer, layout); }
        account(0, layout.size(), 0, 1);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let result = unsafe { BaseAllocator.realloc(pointer, layout, size) };
        if !result.is_null() { account(size, layout.size(), 1, 1); }
        result
    }
}

/// Measures actual GlobalAlloc requests on this thread, including temporary
/// allocations and frees. Other threads and allocator-private overhead are
/// deliberately excluded. This is neither RSS nor an audio callback timer.
pub fn measure_allocations<R>(operation: impl FnOnce() -> R) -> (R, AllocationMeasure) {
    struct Stop;
    impl Drop for Stop { fn drop(&mut self) { MEASURE.with(|cell| cell.set(None)); } }
    MEASURE.with(|cell| { assert!(cell.get().is_none()); cell.set(Some(AllocationMeasure::default())); });
    let stop = Stop;
    let result = operation();
    let measurement = MEASURE.with(|cell| cell.get().unwrap());
    drop(stop);
    (result, measurement)
}
