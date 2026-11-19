//! Harmonigraph's CLAP shell: one [`Plugin`][prelude::Plugin] behind a CLAP
//! entry point, [`nice_export_clap!()`]. The trait states the order of every
//! call into the plugin; the crate's `README.md` maps what harmonigraph
//! changed of the nice-plug it grew from.
//!
//! - Parameters are rows of one [`Table`][prelude::Table]; a row's string id
//!   is what a save stores its value under, so it never changes.
//! - Saved settings beside the parameters belong to the plugin's
//!   [`Setup`][wrapper::clap::setup::Setup].
//! - The [`debug`] macros log, and assert only in debug builds.

#![allow(clippy::type_complexity)]

// nice-plug-core's root, merged in: its modules sit at this crate's root, where
// nice-plug's `pub use nice_plug_core::*` used to put them.
pub mod audio_setup;
pub mod buffer;
pub mod context;
pub mod editor;
pub mod events;
pub mod midi;
pub mod params;
pub mod plugin;
pub mod util;

/// Everything you'll need to use nice-plug. Import this with `use nice_plug::prelude::*;`.
pub mod prelude;
pub mod wrapper;

// Last, as in nice-plug-core, so the `#[macro_use]` scope reaches no module above.
// These macros are also in the crate root and in the prelude, but having the module itself be pub
// as well makes it easy to import _just_ the macros without using `#[macro_use] extern crate nice-plug-core;`
#[macro_use]
pub mod debug;

/// A re-export of the `log` crate for use in the debug macros. This should not be used directly.
pub use log;
