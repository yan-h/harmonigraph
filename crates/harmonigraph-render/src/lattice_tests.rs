//! Unit tests for the lattice renderer. The GPU-backed ones no-op when
//! no headless adapter is available (CI without a GPU); the headless
//! device and the render/readback round trip they share with `roll` and
//! `text` live in [`crate::gpu_harness`].

mod compose;
mod contract;
mod device;
mod fixtures;
mod glow_colour;
mod glow_markers;
mod glow_reach;
mod golden;
mod labels;
mod marks;
mod octaves;
mod quantization;
mod reload;
mod shadows;
mod shimmer;
mod ships;
mod sweep;
mod targets;
mod timing;

// Thread-local so concurrent GPU fixtures count only their own creations,
// including strips discarded before any frame can observe their handles.
thread_local! {
    pub(super) static INK_STRIP_CREATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
