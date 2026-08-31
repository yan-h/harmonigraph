//! Unit tests for the UI shell, one module per thing under test.
//!
//! Split by subject rather than by file: a suite here exercises the whole
//! shell through `root_ui` and a dock, so what it covers is a behaviour
//! (folding, persistence, the settings column) rather than one module's
//! surface. [`harness`] holds what more than one of them needs.
//!
//! Two of the modules here are support rather than suites. [`probe`] is
//! visible to the WHOLE crate, because the pane and widget test modules live
//! beside what they test (`panes/spectral/tests.rs`, `panes/lattice.rs`'s own
//! `mod tests`, `widgets/probe.rs`) and ask the same questions the suites here
//! do: what a themed context draws, and what a pane paints into a rect.
//! [`harness`] answers the ones only a whole dock can, and nothing outside
//! this directory has a dock.

mod fold;
mod harness;
mod labels;
mod lattice;
mod perf;
mod persist;
pub(crate) mod probe;
mod profile;
mod scale;
mod settings;
mod shell;
mod spectral;
mod spectrum;
mod zoom_memory;
