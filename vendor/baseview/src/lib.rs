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
    /// `fired` on `Instant`'s clock, read beside it: the anchor that puts
    /// any of these times on `Instant`'s clock, as `fired_at + (t - fired)`.
    pub fired_at: std::time::Instant,
    /// The refresh the callback was issued for.
    pub timestamp: f64,
    /// The refresh the frame is meant to be on screen by.
    pub target_timestamp: f64,
    /// Seconds between the display's refreshes.
    pub refresh: f64,
    /// The frame rate the frame was paced at: one frame every `divisor`th
    /// refresh, as set by [`Window::set_frame_divisor`].
    pub divisor: u32,
}

pub use clipboard::*;
pub use event::*;
pub use mouse_cursor::MouseCursor;
pub use window::*;
pub use window_info::*;
pub use window_open_options::*;

pub(crate) mod wrappers;
