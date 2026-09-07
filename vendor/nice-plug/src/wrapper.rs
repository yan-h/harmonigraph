//! Wrappers for different plugin types. Each wrapper has an entry point macro that you can pass the
//! name of a type that implements `Plugin` to. The macro will handle the rest.

pub mod clap;
pub(crate) mod state;
pub(crate) mod util;

#[cfg(feature = "standalone")]
pub mod standalone;
#[cfg(feature = "vst3")]
pub mod vst3;

// This is used by the wrappers.
pub use util::setup_logger;

#[cfg(all(debug_assertions, feature = "clap-boundary-tests"))]
pub mod allocation_probe;
