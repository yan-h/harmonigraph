//! Unit tests for the scene layer, one module per thing under test.
//!
//! [`harness`] holds the scene builder and node lookups more than one of
//! them needs; everything else is grouped by what it exercises.

mod camera;
mod glow_curve;
mod gradient_cache;
mod harness;
mod hue_space;
mod marks;
mod nodes;
mod plus;
mod scroll;
mod sheet;
mod shimmer;
mod trail;
