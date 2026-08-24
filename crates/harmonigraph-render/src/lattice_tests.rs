//! Unit tests for the lattice renderer. The GPU-backed ones no-op when
//! no headless adapter is available (CI without a GPU); the headless
//! device and the render/readback round trip they share with `roll` and
//! `text` live in [`crate::gpu_harness`].

mod fixtures;
mod contract;
mod device;
mod shimmer;
mod sweep;
mod marks;
mod clearing;
mod octaves;
mod compose;
mod ships;
mod labels;
mod glow_reach;
mod glow_markers;
mod glow_standoff;
mod glow_colour;
mod glow_meld;
