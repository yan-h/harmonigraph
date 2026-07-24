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

/// Seconds between frame-timer ticks when nothing says otherwise: ~67 Hz.
///
/// Deliberately not 1/60. The timer is a plain run-loop timer with no
/// relation to the display's vsync, so pacing it AT the refresh rate would
/// make ordinary jitter miss refreshes; sampling slightly faster than the
/// display means every refresh has a fresh frame ready.
pub const DEFAULT_FRAME_INTERVAL: f64 = 0.015;

/// Floor on [`Window::set_frame_interval`] (1000 Hz): past this the run loop
/// is doing nothing but service the timer.
pub const MIN_FRAME_INTERVAL: f64 = 0.001;

/// Ceiling on [`Window::set_frame_interval`] (1 Hz): a window that paints
/// less often than this reads as hung.
pub const MAX_FRAME_INTERVAL: f64 = 1.0;

pub use clipboard::*;
pub use event::*;
pub use mouse_cursor::MouseCursor;
pub use window::*;
pub use window_info::*;
pub use window_open_options::*;

pub(crate) mod wrappers;
