//! The allocation escape hatch the logging macros and the wrapper share.

/// Temporarily allow allocations within `func` if the shell was configured with the
/// `assert_process_allocs` feature.
#[cfg(all(debug_assertions, feature = "assert_process_allocs"))]
pub fn permit_alloc<T, F: FnOnce() -> T>(func: F) -> T {
    nice_assert_no_alloc::permit_alloc(func)
}

/// Temporarily allow allocations within `func` if the shell was configured with the
/// `assert_process_allocs` feature.
#[cfg(not(all(debug_assertions, feature = "assert_process_allocs")))]
pub fn permit_alloc<T, F: FnOnce() -> T>(func: F) -> T {
    func()
}
