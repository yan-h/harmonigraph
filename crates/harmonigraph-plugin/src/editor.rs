//! A nice-plug `Editor` implementation running egui on egui-baseview's
//! **wgpu** backend (nice-plug-egui defaults to OpenGL, which is deprecated
//! on macOS and can't host our wgpu paint callbacks; it also doesn't opt in
//! to host->plugin resizing). Adapted from nice-plug-egui's editor glue
//! (ISC licensed).

mod clock;
mod frame;
pub(crate) mod input;
mod persist;
mod shared;
#[cfg(all(feature = "startup-probe", target_os = "macos"))]
pub(crate) mod startup_probe;
mod window;

pub use persist::EguiState;
pub use shared::EditorShared;
pub use window::create;

/// The surface format egui-baseview's wgpu backend will pick. It isn't
/// exposed through its API, so we mirror the choice egui-wgpu makes: the
/// first supported non-sRGB 8-bit format, which is Bgra8Unorm on both Metal
/// and DX12. If the lattice pane ever panics with a pipeline/surface format
/// mismatch on some exotic setup, this is the knob (the real fix is
/// upstreaming RenderState access in egui-baseview).
pub(crate) const ASSUMED_SURFACE_FORMAT: harmonigraph_render::wgpu::TextureFormat =
    harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm;

/// Editor size on first open, in logical pixels.
pub(crate) const DEFAULT_SIZE: (u32, u32) = (1000, 700);
/// Smallest size accepted from a host resize.
///
/// The width is not this editor's own number: it is the floor the pane layout
/// dials to as well, so it comes from `harmonigraph_ui::shell` rather than
/// being restated here — see
/// [`MIN_WINDOW_WIDTH`](harmonigraph_ui::shell::MIN_WINDOW_WIDTH) for what a
/// window and a layout holding two different floors would cost. The height is
/// nobody else's business.
const MIN_SIZE: (u32, u32) = (harmonigraph_ui::shell::MIN_WINDOW_WIDTH as u32, 300);

#[cfg(test)]
mod tests {
    use super::MIN_SIZE;

    /// The floor this window is held to and the floor the pane layout dials to
    /// are one number, and the cast into window pixels is where they could
    /// stop being one: a floor with a fraction in it would leave the window
    /// stopping a fraction below what the layout believes, and the layout
    /// banking a difference the window will never give back (see
    /// `harmonigraph_ui::shell::MIN_WINDOW_WIDTH`).
    #[test]
    fn the_window_floor_is_exactly_the_floor_the_layout_dials_to() {
        assert_eq!(MIN_SIZE.0 as f32, harmonigraph_ui::shell::MIN_WINDOW_WIDTH);
    }
}
