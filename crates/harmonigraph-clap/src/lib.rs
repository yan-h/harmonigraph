//! Harmonigraph's CLAP shell: one [`Plugin`][prelude::Plugin] behind a CLAP
//! entry point, [`export_clap!()`]. The trait states the order of every
//! call into the plugin.
//!
//! The crate grew from nice-plug 0.1.10 and nice-plug-core 0.1.5, which
//! harmonigraph first vendored and then owned (#1814); `PATCHES.md` says where
//! they came from and how an upgrade is made. nice-plug-core's modules sit at
//! the crate root, where nice-plug's `pub use nice_plug_core::*` used to put
//! them, and its logging and assertion macros are the `shell_*!` family.
//! No tuning or sequencing policy belongs here.
//!
//! - Parameters are rows of one [`Table`][prelude::Table]; a row's string id
//!   is what a save stores its value under, so it never changes.
//! - Saved settings beside the parameters belong to the plugin's
//!   [`Setup`][wrapper::clap::setup::Setup].
//! - The [`debug`] macros log, and assert only in debug builds.
//! - `tests/` drive an exported fixture plugin through `clap-sys`: the process
//!   callback, flush and editor writes (`clap_boundary.rs`) and the auxiliary
//!   bounds (`clap_auxiliary.rs`). They run in `ci.sh`'s workspace test gate
//!   with the `clap-boundary-tests` feature, which also turns on
//!   `assert_process_allocs`. The `testing` module is the one CLAP host those
//!   fixtures and harmonigraph-plugin's entry-point suites share.

#![allow(clippy::type_complexity)]

// nice-plug-core's root, merged in: its modules sit at this crate's root, where
// nice-plug's `pub use nice_plug_core::*` used to put them.
pub mod audio_setup;
pub mod editor;
pub mod events;
pub mod params;
pub mod plugin;
pub mod util;

/// Everything a plugin needs from the shell. Import this with `use harmonigraph_clap::prelude::*;`.
pub mod prelude;
/// A CLAP host for the fixtures that drive a plugin through its entry point.
#[cfg(feature = "clap-boundary-tests")]
pub mod testing;
pub mod wrapper;

// Last, as in nice-plug-core, so the `#[macro_use]` scope reaches no module above.
// These macros are also in the crate root and in the prelude, but having the module itself be pub
// as well makes it easy to import _just_ the macros without using `#[macro_use] extern crate harmonigraph_clap;`
#[macro_use]
pub mod debug;

/// A re-export of the `log` crate for use in the debug macros. This should not be used directly.
pub use log;
