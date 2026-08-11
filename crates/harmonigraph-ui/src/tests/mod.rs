//! Unit tests for the UI shell, one module per thing under test.
//!
//! Split by subject rather than by file: a suite here exercises the whole
//! shell through `root_ui` and a dock, so what it covers is a behaviour
//! (folding, persistence, the settings column) rather than one module's
//! surface. [`harness`] holds what more than one of them needs.
//!
//! Two of the modules here are support rather than suites, and they are
//! visible to the WHOLE crate rather than to this directory: the pane test
//! modules live beside the panes (`panes/spectral/tests.rs`,
//! `panes/lattice.rs`'s own `mod tests`) and ask the same two questions the
//! suites here do — what a themed context draws, and what a pane paints into
//! a rect. [`probe`] answers those for every test module in the crate;
//! [`harness`] answers the ones only a whole dock can.

pub(crate) mod harness;
pub(crate) mod probe;
mod persist;
mod shell;
mod lattice;
mod spectrum;
mod labels;
mod spectral;
mod fold;
mod settings;
mod perf;
mod scale;
mod profile;
