mod clipboard;
mod event;
mod keyboard;
mod mouse_cursor;
mod window;
mod window_info;
mod window_open_options;

pub(crate) mod platform;

#[cfg(feature = "opengl")]
pub mod gl;

/// When the display asked for the frame `on_frame` is running, in seconds on
/// the media clock (`CACurrentMediaTime` on macOS). See
/// [`Window::frame_timing`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameTiming {
    /// When the callback actually ran.
    pub fired: f64,
    /// The refresh the callback was issued for.
    pub timestamp: f64,
    /// The refresh the frame is meant to be on screen by.
    pub target_timestamp: f64,
    /// Seconds between the display's refreshes.
    pub refresh: f64,
}

pub use clipboard::*;
pub use event::*;
pub use mouse_cursor::MouseCursor;
pub use window::*;
pub use window_info::*;
pub use window_open_options::*;

pub(crate) mod wrappers;
