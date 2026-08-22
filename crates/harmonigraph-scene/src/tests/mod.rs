//! Unit tests for the scene layer, one module per thing under test.
//!
//! [`harness`] holds the scene builder and node lookups more than one of
//! them needs; everything else is grouped by what it exercises.

mod harness;
mod nodes;
mod plus;
mod marks;
mod sheet;
mod camera;
mod scroll;
mod trail;
mod hue_space;
mod shimmer;
mod gradient_cache;
