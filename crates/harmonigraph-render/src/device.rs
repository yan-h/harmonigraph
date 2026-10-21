//! The one device descriptor every shell and test in this workspace asks for.
//! The plugin, the standalone harness, the offline renderer and the GPU tests
//! all take it from [`device_descriptor`], so a feature or limit added here
//! reaches each of them, and a test never runs with features or limits
//! production doesn't ask for. Which adapter each picks is separate: tests go
//! through `test_gpu_adapter`, the shells through their own instance. (The
//! vendored crates' own tests build their devices themselves.)
//!
//! Every optional feature is granted wherever the adapter offers it, so on
//! the CI Metal adapter no test reaches a feature's fallback, and none
//! covers the timestamp-free path today. A test that should reach one would
//! narrow the descriptor: `descriptor.required_features -= wgpu::Features::...`.

use crate::wgpu;

/// Requested wherever the adapter advertises them, never required:
/// `request_device` FAILS on an unsupported feature, and a missing optional
/// capability must cost a readout or a fallback, not a plugin that won't open.
/// Code that uses one checks `device.features()` at the point of use.
///
/// `TIMESTAMP_QUERY` lets the performance overlay report GPU time; where it is
/// not granted the overlay says "n/a".
const OPTIONAL_FEATURES: wgpu::Features = wgpu::Features::TIMESTAMP_QUERY;

/// The descriptor for every device this project creates: the adapter's own
/// limits, plus each [`OPTIONAL_FEATURES`] entry the adapter offers.
///
/// A caller that cannot work without one of the optional features checks
/// `required_features` on the result and bails, rather than building its own.
pub fn device_descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        label: Some("harmonigraph device"),
        required_features: adapter.features() & OPTIONAL_FEATURES,
        required_limits: device_limits(adapter),
        ..Default::default()
    }
}

/// Request the adapter's limits in every rendering shell. The lattice's three
/// RGBA16F attachments plus R16F local transmission require 26 bytes/sample;
/// supported Apple silicon and the measured CI Metal adapter provide at least 64.
/// Older adapters are unsupported; there is no second shadow pass to fall back to.
fn device_limits(adapter: &wgpu::Adapter) -> wgpu::Limits {
    let limits = adapter.limits();
    assert!(
        limits.max_color_attachment_bytes_per_sample >= 26,
        "the lattice requires at least 26 color attachment bytes per sample"
    );
    limits
}
