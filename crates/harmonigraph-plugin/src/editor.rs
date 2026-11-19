//! Harmonigraph's editor, the shell's `Editor`, running egui on egui-baseview's
//! **wgpu** backend (nice-plug-egui defaults to OpenGL, which is deprecated
//! on macOS and can't host our wgpu paint callbacks; it also doesn't opt in
//! to host->plugin resizing). Adapted from nice-plug-egui's editor glue
//! (ISC licensed).

mod clock;
mod frame;
pub(crate) mod gestures;
pub(crate) mod input;
mod lock;
mod persist;
mod pop_out;
mod shared;
#[cfg(all(feature = "startup-probe", target_os = "macos"))]
pub(crate) mod startup_probe;
mod window;

pub(crate) use lock::open_window_session;
pub use lock::EditorLock;
pub(crate) use persist::SavedWindow;
pub use persist::{EguiState, UiState};
pub use shared::EditorShared;
pub use window::create;
pub(crate) use window::{graphics_config, start_graphics};

/// The surface format egui-baseview's wgpu backend will pick. It isn't
/// exposed through its API, so we mirror the choice egui-wgpu makes: the
/// first supported non-sRGB 8-bit format, which is Bgra8Unorm on both Metal
/// and DX12. If the lattice pane ever panics with a pipeline/surface format
/// mismatch on some exotic setup, this is the knob (upstreaming RenderState
/// access in egui-baseview would remove the need to mirror it; egui-baseview
/// is pinned from egui-plugin-window, whose `PATCHES.md` holds its patches).
pub(crate) const ASSUMED_SURFACE_FORMAT: harmonigraph_render::wgpu::TextureFormat =
    harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm;

/// Editor size on first open, in logical pixels.
pub(crate) const DEFAULT_SIZE: (u32, u32) = (1000, 700);
/// Both shells use the same minimum window size.
const MIN_SIZE: (u32, u32) = (
    harmonigraph_ui::shell::MIN_WINDOW_SIZE.x as u32,
    harmonigraph_ui::shell::MIN_WINDOW_SIZE.y as u32,
);
/// The largest the editor's own window may be, in logical points: Apple
/// silicon's texture limit, 16,384 pixels a side, at a Retina display's scale
/// of 2. Past it egui-baseview refuses the surface, and the picture would stop
/// following the window, so the window stops where the picture already did,
/// and no display reaches it. The host's window has no maximum of ours.
const MAX_SIZE: (u32, u32) = (8192, 8192);
/// Whether the editor can open in a window of its own on this platform
/// (`pop_out`); elsewhere it stays in the host's.
const OWN_WINDOW_OFFERED: bool = cfg!(target_os = "macos");
