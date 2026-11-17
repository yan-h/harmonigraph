#![allow(unsafe_code)]

//! A set of safe wrappers around C or platform APIs.
//!
//! This module is designed to contain all unsafe code necessary for baseview to work, where safe
//! APIs do not yet exist in the Rust ecosystem (e.g. parts of AppKit), or are not practical to use.
//!
//! These wrappers are not designed to fully encapsulate their respective APIs, only the bits used
//! by baseview.
//!
//! However, all of these APIs should always be sound (i.e. no UB can be triggered by safe code).
//! Otherwise, this should be considered a bug and reported accordingly.

#[cfg(target_os = "macos")]
pub mod appkit;
