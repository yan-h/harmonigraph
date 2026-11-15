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
    /// the refresh of any panel at 10 Hz or faster.
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

#[cfg(test)]
mod tests {
    use super::FrameTiming;
    use std::time::{Duration, Instant};

    /// A target ahead of the fire or behind it (a callback over a refresh
    /// late) lands that far from `fired_at`; one a second off, or not a
    /// number, is not this frame's.
    #[test]
    fn a_target_lands_beside_its_fire_or_not_at_all() {
        let fired_at = Instant::now() + Duration::from_secs(1);
        let timing = |target_timestamp| FrameTiming {
            fired: 500.0,
            fired_at,
            timestamp: 499.995,
            target_timestamp,
            refresh: 1.0 / 144.0,
            divisor: 1,
        };
        let at = |target| timing(target).target_at();
        assert_eq!(at(500.004), Some(fired_at + Duration::from_secs_f64(0.004)));
        assert_eq!(at(499.998), Some(fired_at - Duration::from_secs_f64(0.002)));
        assert_eq!(at(501.0), None);
        assert_eq!(at(f64::NAN), None);
    }
}
