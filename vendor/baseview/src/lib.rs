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

/// When the display asked for the frame `on_frame` is running: in seconds on
/// the media clock (`CACurrentMediaTime` on macOS), but for `fired_at`, which
/// is on `Instant`'s. See [`Window::frame_timing`].
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

impl FrameTiming {
    /// How far a target may lie from the fire and still be this frame's: past
    /// any panel's refresh, and the link's own silence threshold before the
    /// watchdog takes over.
    const MAX_TARGET_SECONDS: f64 = 0.1;

    /// `target_timestamp` on `Instant`'s clock, or `None` when the link's
    /// times put it further than a tenth of a second from the fire, or are not
    /// numbers. The times come from AppKit, and a consumer that never rewinds
    /// its clock would hold its picture still behind one target far ahead.
    pub fn target_at(&self) -> Option<std::time::Instant> {
        let ahead = self.target_timestamp - self.fired;
        if ahead.is_nan() || ahead.abs() > Self::MAX_TARGET_SECONDS {
            return None;
        }
        let span = std::time::Duration::from_secs_f64(ahead.abs());
        if ahead >= 0.0 {
            self.fired_at.checked_add(span)
        } else {
            self.fired_at.checked_sub(span)
        }
    }
}

pub use clipboard::*;
pub use event::*;
pub use mouse_cursor::MouseCursor;
pub use window::*;
pub use window_info::*;
pub use window_open_options::*;

pub(crate) mod wrappers;
