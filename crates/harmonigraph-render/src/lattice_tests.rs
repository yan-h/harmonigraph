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
mod glow_union;
mod golden;
mod labels;
mod marks;
#[cfg(target_os = "macos")]
mod metal_assets;
#[cfg(target_os = "macos")]
mod metal_precompile;
mod occlusion;
mod octaves;
mod quantization;
mod reload;
mod shadow_staging;
mod shadows;
mod ships;
mod targets;
mod timing;

// Thread-local so concurrent GPU fixtures count only their own creations,
// including strips discarded before any frame can observe their handles.
thread_local! {
    pub(super) static INK_STRIP_CREATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
