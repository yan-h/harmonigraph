use backtrace::Backtrace;
use std::cmp;
use std::os::raw::c_char;

use crate::util::permit_alloc;

pub(crate) mod buffer_management;
#[cfg(debug_assertions)]
pub(crate) mod context_checks;

// `clap-boundary-tests` REPLACES the guard below rather than adding to it: it
// installs `MeasuredAllocator`, whose base is `AllocDisabler` only when
// `assert_process_allocs` is on and `std::alloc::System` otherwise. Enabling
// the boundary tests alone therefore counts every allocation an audio callback
// makes and permits all of them — a suite that looks like it measures a guarded
// callback while guarding nothing. The pairing is not expressible as a Cargo
// feature dependency here without editing the vendored manifest, so it is held
// at compile time instead.
#[cfg(all(feature = "clap-boundary-tests", not(feature = "assert_process_allocs")))]
compile_error!(
    "The 'clap-boundary-tests' feature replaces the 'assert_process_allocs' global allocator with a counting one, whose base allocator is the plain system allocator unless 'assert_process_allocs' is also enabled. Enable both features together, or neither."
);

#[cfg(all(
    debug_assertions,
    feature = "assert_process_allocs",
    not(feature = "clap-boundary-tests")
))]
#[global_allocator]
static A: nice_assert_no_alloc::AllocDisabler = nice_assert_no_alloc::AllocDisabler;

#[cfg(all(debug_assertions, feature = "clap-boundary-tests"))]
#[global_allocator]
static MEASURED: super::allocation_probe::MeasuredAllocator =
    super::allocation_probe::MeasuredAllocator;

/// A Rabin fingerprint based string hash for parameter ID strings.
pub fn hash_param_id(id: &str) -> u32 {
    let mut hash: u32 = 0;
    for char in id.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(char as u32);
    }

    // In VST3 the last bit is reserved for parameters provided by the host
    // https://developer.steinberg.help/display/VST/Parameters+and+Automation
    hash &= !(1 << 31);

    hash
}

/// The equivalent of the `strlcpy()` C function. Copy `src` to `dest` as a null-terminated
/// C-string. If `dest` does not have enough capacity, add a null terminator at the end to prevent
/// buffer overflows.
pub fn strlcpy(dest: &mut [c_char], src: &str) {
    if dest.is_empty() {
        return;
    }

    let src_bytes: &[u8] = src.as_bytes();
    // NOTE: `c_char` is i8 on x86 based archs, and u8 on AArch64. There this line won't do
    //       anything.
    let src_bytes_signed: &[c_char] = unsafe { &*(src_bytes as *const [u8] as *const [c_char]) };

    // Make sure there's always room for a null terminator
    let copy_len = cmp::min(dest.len() - 1, src.len());
    dest[..copy_len].copy_from_slice(&src_bytes_signed[..copy_len]);
    dest[copy_len] = 0;
}

/// Clamp an input event's timing to the buffer length. Emits a debug assertion failure if it was
/// out of bounds.
#[inline]
pub fn clamp_input_event_timing(timing: u32, total_buffer_len: u32) -> u32 {
    // If `total_buffer_len == 0`, then 0 is a valid timing
    let last_valid_index = total_buffer_len.saturating_sub(1);

    crate::nice_debug_assert!(
        timing <= last_valid_index,
        "Input event is out of bounds, will be clamped to the buffer's size"
    );

    timing.min(last_valid_index)
}

/// Clamp an output event's timing to the buffer length. Emits a debug assertion failure if it was
/// out of bounds.
#[inline]
pub fn clamp_output_event_timing(timing: u32, total_buffer_len: u32) -> u32 {
    let last_valid_index = total_buffer_len.saturating_sub(1);

    crate::nice_debug_assert!(
        timing <= last_valid_index,
        "Output event is out of bounds, will be clamped to the buffer's size"
    );

    timing.min(last_valid_index)
}

/// Set up the logger so that the `nice_*!()` logging and assertion macros log output to a
/// centralized location and panics also get written there. By default this logs to STDERR. If a
/// Windows debugger is attached, then messages will be sent there instead. This uses
/// [nice-log](https://github.com/BillyDM/nice-log). See the readme there for more information.
///
/// In short, nice-log's behavior can be controlled by setting the `NICE_LOG` environment variable to:
///
/// - `stderr`, in which case the log output always gets written to STDERR.
/// - `windbg` (only on Windows), in which case the output always gets logged using
///   `OutputDebugString()`.
/// - A file path, in which case the output gets appended to the end of that file which will be
///   created if necessary.
pub fn setup_logger() {
    let log_level =
        if cfg!(debug_assertions) { log::LevelFilter::Trace } else { log::LevelFilter::Info };

    let logger_builder = nice_log::LoggerBuilder::new(log_level)
        .filter_module("cosmic_text::buffer")
        .filter_module("cosmic_text::shape")
        .filter_module("selectors::matching");

    // Always show the module in debug builds, makes it clearer where messages are coming from and
    // it helps set up filters
    #[cfg(debug_assertions)]
    let logger_builder = logger_builder.always_show_module_path();

    // In release builds there are some more logging messages from libraries that are not relevant
    // to the end user that can be filtered out
    #[cfg(not(debug_assertions))]
    let logger_builder = logger_builder.filter_module("cosmic_text::font::system::std");

    let logger_set = logger_builder.build_global().is_ok();
    if logger_set {
        log_panics();
    }
}

/// This is copied from same as the `log_panics` crate, but it's wrapped in `permit_alloc()`.
/// Otherwise logging panics will trigger `assert_no_alloc` as this also allocates.
fn log_panics() {
    std::panic::set_hook(Box::new(|info| {
        permit_alloc(|| {
            // All of this is directly copied from `permit_no_alloc`, except that `error!()` became
            // `nice_error!()` and `Shim` has been inlined
            let backtrace = Backtrace::new();

            let thread = std::thread::current();
            let thread = thread.name().unwrap_or("unnamed");

            let msg = match info.payload().downcast_ref::<&'static str>() {
                Some(s) => *s,
                None => match info.payload().downcast_ref::<String>() {
                    Some(s) => &**s,
                    None => "Box<Any>",
                },
            };

            match info.location() {
                Some(location) => {
                    crate::nice_error!(
                        target: "panic", "thread '{}' panicked at '{}': {}:{}\n{:?}",
                        thread,
                        msg,
                        location.file(),
                        location.line(),
                        backtrace
                    );
                }
                None => {
                    crate::nice_error!(
                        target: "panic",
                        "thread '{}' panicked at '{}'\n{:?}",
                        thread,
                        msg,
                        backtrace
                    )
                }
            }
        })
    }));
}

/// A wrapper around the entire process function, including the plugin wrapper parts. This sets up
/// `assert_no_alloc` if needed.
pub fn process_wrapper<T, F: FnOnce() -> T>(f: F) -> T {
    #[cfg(all(debug_assertions, feature = "assert_process_allocs"))]
    return nice_assert_no_alloc::assert_no_alloc(f);

    #[cfg(not(all(debug_assertions, feature = "assert_process_allocs")))]
    return f();
}

#[cfg(test)]
mod miri {
    use std::ffi::CStr;

    use super::*;

    #[test]
    fn strlcpy_normal() {
        let mut dest = [0; 256];
        strlcpy(&mut dest, "Hello, world!");

        assert_eq!(unsafe { CStr::from_ptr(dest.as_ptr()) }.to_str(), Ok("Hello, world!"));
    }

    #[test]
    fn strlcpy_overflow() {
        let mut dest = [0; 6];
        strlcpy(&mut dest, "Hello, world!");

        assert_eq!(unsafe { CStr::from_ptr(dest.as_ptr()) }.to_str(), Ok("Hello"));
    }
}
