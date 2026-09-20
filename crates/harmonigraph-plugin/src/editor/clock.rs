/// One presentation clock for notes, audio columns and drawing. While audio
/// flows its sample time owns the clock, including faster-than-realtime bounces.
/// Moving the offset towards wall time each drain compresses history across
/// batches: old columns keep their dates while new ones move backwards.
pub(crate) struct ClockMapper {
    /// Initial delivery offset, held until callbacks pause or the source rewinds.
    pub(super) offset: Option<f64>,
    observed_audio: Option<f64>,
    observed_wall: f64,
    shown: f64,
}

impl ClockMapper {
    /// Ignore ordinary callback jitter; a real callback pause leaves room for
    /// wall-clock fades before the source resumes. Transport stops usually keep
    /// streaming silence and need no special handling.
    const PAUSE_SECONDS: f64 = 1.0;

    pub fn new() -> Self {
        ClockMapper { offset: None, observed_audio: None, observed_wall: 0.0, shown: 0.0 }
    }

    /// Observe a fresh audio heartbeat, independently of historical delivery.
    /// An idle polling pass must not re-anchor the last callback clock.
    pub fn observe(&mut self, newest_audio_time: f64, gui_now: f64) {
        if self.observed_audio == Some(newest_audio_time) {
            return;
        }
        let candidate = self.shown.max(gui_now) - newest_audio_time;
        self.offset = Some(match (self.offset, self.observed_audio) {
            (Some(offset), Some(previous)) if newest_audio_time >= previous => {
                let pause = (gui_now - self.observed_wall) - (newest_audio_time - previous);
                offset + if pause > Self::PAUSE_SECONDS { pause } else { 0.0 }
            }
            _ => candidate,
        });
        self.observed_audio = Some(newest_audio_time);
        self.observed_wall = gui_now;
    }

    /// Follow fresh audio exactly; keep ageing when callbacks stop arriving.
    pub fn now(&mut self, wall_now: f64) -> f64 {
        let now = match (self.observed_audio, self.offset) {
            (Some(audio), Some(offset)) => {
                audio + offset + (wall_now - self.observed_wall).max(0.0)
            }
            _ => wall_now,
        };
        // A short callback gap can resume behind an extrapolated frame. Hold
        // until audio catches up rather than rewind histories already pruned.
        self.shown = self.shown.max(now);
        self.shown
    }

    /// Map an audio timestamp to GUI time (clamped: never in the future).
    #[cfg(test)]
    pub fn map(&self, audio_time: f64, gui_now: f64) -> f64 {
        match self.offset {
            Some(offset) => (audio_time + offset).min(gui_now),
            None => gui_now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ClockMapper;

    #[test]
    fn preserves_intra_batch_spacing() {
        let mut clock = ClockMapper::new();
        // Audio clock at ~100s, GUI clock at ~7s.
        clock.observe(100.0, 7.0);
        let a = clock.map(99.950, 7.0);
        let b = clock.map(99.995, 7.0);
        assert!((b - a - 0.045).abs() < 1e-9, "spacing lost: {a} {b}");
    }

    #[test]
    fn never_maps_into_the_future() {
        let mut clock = ClockMapper::new();
        clock.observe(100.0, 7.0);
        assert!(clock.map(120.0, 7.0) <= 7.0);
    }

    #[test]
    fn source_rewind_preserves_the_display_clock() {
        let mut clock = ClockMapper::new();
        clock.observe(0.0, 0.0);
        clock.observe(10.0, 1.0);
        assert_eq!(clock.now(1.0), 10.0);
        // Even a source rewind after a fast bounce starts at the visible
        // timeline, so new events cannot fall behind an already-pruned frame.
        clock.observe(0.1, 2.0);
        let now = clock.now(2.0);
        assert_eq!(now, 10.0);
        assert_eq!(clock.map(0.1, now), now);
    }

    #[test]
    fn callback_jitter_does_not_retime_history() {
        let mut clock = ClockMapper::new();
        clock.observe(100.0, 7.0); // offset -93
        clock.observe(101.0, 8.1); // candidate -92.9: jitter, not a reset
        let mapped = clock.map(101.0, 9.0);
        assert_eq!(mapped, 8.0, "delivery jitter must not move already-stamped history");
        assert_eq!(clock.now(8.1), 8.0);
    }

    #[test]
    fn presentation_clock_keeps_fading_after_a_fast_bounce_stops() {
        let mut clock = ClockMapper::new();
        clock.observe(0.0, 0.0);
        clock.observe(10.0, 1.0);
        assert_eq!(clock.now(1.0), 10.0);
        assert_eq!(clock.now(3.0), 12.0);
        // Idle polls must not reset the fade clock to the last callback.
        clock.observe(10.0, 3.0);
        assert_eq!(clock.now(3.0), 12.0);
        clock.observe(10.25, 3.25);
        assert_eq!(clock.now(3.25), 12.25, "resuming preserves the idle gap");
        assert_eq!(clock.offset, Some(2.0));
        // A shorter callback gap holds the drawn time until audio catches up.
        assert_eq!(clock.now(3.75), 12.75);
        clock.observe(10.5, 3.8);
        assert_eq!(clock.now(3.8), 12.75);
        clock.observe(11.0, 4.0);
        assert_eq!(clock.now(4.0), 13.0);
    }
}
