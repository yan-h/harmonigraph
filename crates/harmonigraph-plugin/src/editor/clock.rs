/// One clock-domain mapping for notes and audio columns, and one presentation
/// clock for drawing them.
///
/// Source timestamps keep one fixed `offset`: moving it towards wall time on
/// every drain compresses history across batches, since old columns keep their
/// dates while new ones move backwards. The picture is different. In realtime
/// the audio frontier arrives a BLOCK at a time and the GUI observes it a FRAME
/// at a time; drawing directly at that frontier makes time advance in unequal
/// steps even though both clocks run at the same rate. Realtime presentation is
/// therefore wall-paced and only slews gently towards audio. Buffered and
/// offline hosts own presentation outright, so ahead-of-time processing still
/// advances at source speed.
pub(crate) struct ClockMapper {
    /// Initial delivery offset, held until callbacks pause or the source rewinds.
    pub(super) offset: Option<f64>,
    observed_audio: Option<f64>,
    observed_wall: f64,
    presented_wall: Option<f64>,
    shown: f64,
}

impl ClockMapper {
    /// Ignore ordinary callback jitter; a real callback pause leaves room for
    /// wall-clock fades before the source resumes. Transport stops usually keep
    /// streaming silence and need no special handling.
    const PAUSE_SECONDS: f64 = 1.0;
    /// Largest realtime speed correction, as a share of wall-clock progress.
    ///
    /// Enough to close ordinary hardware-clock drift without turning the
    /// block-stepped audio frontier back into block-stepped motion. It bounds a
    /// frame to 0.99x..=1.01x wall speed even when a callback lands just before
    /// one GUI frame and just after the next.
    const REALTIME_SLEW: f64 = 0.01;
    /// A larger phase error is a discontinuity, not ordinary callback/frame
    /// phasing. Re-anchor future timestamps before draining them rather than
    /// spending seconds slewing newly resumed notes through already-pruned time.
    const REALTIME_DISCONTINUITY_SECONDS: f64 = 0.1;

    pub fn new() -> Self {
        ClockMapper {
            offset: None,
            observed_audio: None,
            observed_wall: 0.0,
            presented_wall: None,
            shown: 0.0,
        }
    }

    /// Observe a fresh audio heartbeat, independently of historical delivery.
    /// An idle polling pass must not re-anchor the last callback clock.
    pub fn observe(&mut self, newest_audio_time: f64, gui_now: f64, source_owned: bool) {
        if self.observed_audio == Some(newest_audio_time) {
            return;
        }
        let candidate = self.shown.max(gui_now) - newest_audio_time;
        let rewound = self.observed_audio.is_some_and(|previous| newest_audio_time < previous);
        let mut offset = match (self.offset, self.observed_audio) {
            (Some(offset), Some(previous)) if newest_audio_time >= previous => {
                let pause = (gui_now - self.observed_wall) - (newest_audio_time - previous);
                offset + if pause > Self::PAUSE_SECONDS { pause } else { 0.0 }
            }
            _ => candidate,
        };

        if let (false, false, Some(presented_wall)) = (source_owned, rewound, self.presented_wall) {
            let presented_now = self.shown + (gui_now - presented_wall).max(0.0);
            let phase_error = newest_audio_time + offset - presented_now;
            if phase_error.abs() > Self::REALTIME_DISCONTINUITY_SECONDS {
                offset -= phase_error;
            }
        }
        self.offset = Some(offset);
        self.observed_audio = Some(newest_audio_time);
        self.observed_wall = gui_now;
        if rewound {
            self.presented_wall = None;
        }
    }

    /// Present at wall speed in realtime and at source speed when buffered or
    /// offline. Both keep ageing when callbacks stop arriving.
    pub fn now(&mut self, wall_now: f64, source_owned: bool) -> f64 {
        let source_now = match (self.observed_audio, self.offset) {
            (Some(audio), Some(offset)) => {
                audio + offset + (wall_now - self.observed_wall).max(0.0)
            }
            _ => wall_now,
        };

        let now = if source_owned {
            source_now
        } else if let Some(previous_wall) = self.presented_wall {
            let elapsed = (wall_now - previous_wall).max(0.0);
            let predicted = self.shown + elapsed;
            let correction = (source_now - predicted)
                .clamp(-Self::REALTIME_SLEW * elapsed, Self::REALTIME_SLEW * elapsed);
            predicted + correction
        } else {
            source_now
        };
        self.presented_wall = Some(wall_now);
        // Pruning is irreversible, so neither a source rewind nor a wall-clock
        // anomaly may move the picture back over history it has already shed.
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

    const REALTIME: bool = false;
    const SOURCE_OWNED: bool = true;

    #[test]
    fn preserves_intra_batch_spacing() {
        let mut clock = ClockMapper::new();
        // Audio clock at ~100s, GUI clock at ~7s.
        clock.observe(100.0, 7.0, SOURCE_OWNED);
        let a = clock.map(99.950, 7.0);
        let b = clock.map(99.995, 7.0);
        assert!((b - a - 0.045).abs() < 1e-9, "spacing lost: {a} {b}");
    }

    #[test]
    fn never_maps_into_the_future() {
        let mut clock = ClockMapper::new();
        clock.observe(100.0, 7.0, SOURCE_OWNED);
        assert!(clock.map(120.0, 7.0) <= 7.0);
    }

    #[test]
    fn source_rewind_preserves_the_display_clock() {
        let mut clock = ClockMapper::new();
        clock.observe(0.0, 0.0, SOURCE_OWNED);
        clock.observe(10.0, 1.0, SOURCE_OWNED);
        assert_eq!(clock.now(1.0, SOURCE_OWNED), 10.0);
        // Even a source rewind after a fast bounce starts at the visible
        // timeline, so new events cannot fall behind an already-pruned frame.
        clock.observe(0.1, 2.0, SOURCE_OWNED);
        let now = clock.now(2.0, SOURCE_OWNED);
        assert_eq!(now, 10.0);
        assert_eq!(clock.map(0.1, now), now);
    }

    #[test]
    fn callback_jitter_does_not_retime_history() {
        let mut clock = ClockMapper::new();
        clock.observe(100.0, 7.0, REALTIME); // offset -93
        assert_eq!(clock.now(7.0, REALTIME), 7.0);
        clock.observe(101.0, 8.01, REALTIME); // candidate -92.99: jitter, not a reset
        let mapped = clock.map(101.0, 9.0);
        assert_eq!(mapped, 8.0, "delivery jitter must not move already-stamped history");
        let shown = clock.now(8.01, REALTIME);
        assert_eq!(shown, 8.0);
    }

    /// A 512-sample heartbeat and a 144 Hz GUI never land at one cadence. The
    /// old clock restarted from the latest whole block and advanced each frame
    /// by 3.7, 6.9 or 10.7 ms; the geometry was sub-pixel, but the time handed
    /// to it was not. Every phase here must stay within the realtime slew bound.
    #[test]
    fn realtime_frames_stay_smooth_across_block_quantized_heartbeats() {
        const BLOCK: f64 = 512.0 / 48_000.0;
        const FRAME: f64 = 1.0 / 144.0;

        for phase in [0.0, 0.23 * BLOCK, 0.71 * BLOCK] {
            let mut clock = ClockMapper::new();
            let mut previous_audio = None;
            let mut previous_shown = None;
            for frame in 0..180 {
                let wall = frame as f64 * FRAME;
                let audio = ((wall + phase) / BLOCK).floor() * BLOCK;
                if previous_audio != Some(audio) {
                    clock.observe(audio, wall, REALTIME);
                    previous_audio = Some(audio);
                }
                let shown = clock.now(wall, REALTIME);
                if let Some(previous) = previous_shown {
                    let step = shown - previous;
                    let tolerance = 1e-12;
                    assert!(
                        step + tolerance >= (1.0 - ClockMapper::REALTIME_SLEW) * FRAME
                            && step - tolerance <= (1.0 + ClockMapper::REALTIME_SLEW) * FRAME,
                        "phase {phase:.6}, frame {frame}: presentation advanced {step:.9}s",
                    );
                }
                previous_shown = Some(shown);
            }
        }
    }

    #[test]
    fn realtime_resume_reanchors_before_new_events_are_mapped() {
        let mut clock = ClockMapper::new();
        clock.observe(0.0, 0.0, REALTIME);
        assert_eq!(clock.now(0.0, REALTIME), 0.0);
        assert_eq!(clock.now(1.5, REALTIME), 1.5);

        // Audio resumes after a half-second callback outage. This observation
        // precedes draining, so its events must land at the current picture
        // rather than half a second behind it and at risk of immediate pruning.
        clock.observe(1.01, 1.51, REALTIME);
        let mapped = clock.map(1.01, 1.51);
        assert!((mapped - 1.51).abs() < 1e-9, "resumed event mapped to {mapped}");
        assert!((clock.now(1.51, REALTIME) - 1.51).abs() < 1e-9);
    }

    #[test]
    fn presentation_clock_keeps_fading_after_a_fast_bounce_stops() {
        let mut clock = ClockMapper::new();
        clock.observe(0.0, 0.0, SOURCE_OWNED);
        clock.observe(10.0, 1.0, SOURCE_OWNED);
        assert_eq!(clock.now(1.0, SOURCE_OWNED), 10.0);
        assert_eq!(clock.now(3.0, SOURCE_OWNED), 12.0);
        // Idle polls must not reset the fade clock to the last callback.
        clock.observe(10.0, 3.0, SOURCE_OWNED);
        assert_eq!(clock.now(3.0, SOURCE_OWNED), 12.0);
        clock.observe(10.25, 3.25, SOURCE_OWNED);
        assert_eq!(clock.now(3.25, SOURCE_OWNED), 12.25, "resuming preserves the idle gap");
        assert_eq!(clock.offset, Some(2.0));
        assert_eq!(clock.now(3.75, SOURCE_OWNED), 12.75);
        clock.observe(10.5, 3.8, SOURCE_OWNED);
        assert_eq!(clock.now(3.8, SOURCE_OWNED), 12.75);
        clock.observe(11.0, 4.0, SOURCE_OWNED);
        assert_eq!(clock.now(4.0, SOURCE_OWNED), 13.0);
    }
}
