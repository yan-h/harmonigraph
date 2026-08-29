//! Unit tests for the lattice renderer. The GPU-backed ones no-op when
//! no headless adapter is available (CI without a GPU); the headless
//! device and the render/readback round trip they share with `roll` and
//! `text` live in [`crate::gpu_harness`].

mod clearing;
mod compose;
mod contract;
mod device;
mod fixtures;
mod glow_colour;
mod glow_markers;
mod glow_meld;
mod glow_reach;
mod glow_standoff;
mod golden;
mod labels;
mod marks;
mod octaves;
mod shimmer;
mod ships;
mod sweep;
mod timing;
