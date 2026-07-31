//! Unit tests for the UI shell, one module per thing under test.
//!
//! Split by subject rather than by file: a suite here exercises the whole
//! shell through `root_ui` and a dock, so what it covers is a behaviour
//! (folding, persistence, the settings column) rather than one module's
//! surface. [`harness`] holds what more than one of them needs.

mod harness;
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
