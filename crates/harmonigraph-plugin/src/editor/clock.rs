/// One presentation clock for notes, audio columns and drawing. While audio
/// flows its sample time owns the clock, including faster-than-realtime bounces.
/// Moving the offset towards wall time each drain compresses history across
/// batches: old columns keep their dates while new ones move backwards.
pub(crate) struct ClockMapper {
    /// Initial delivery offset, held until callbacks pause or the source rewinds.
    pub(super) offset: Option<f64>,
    observed_audio: Option<f64>,
    observed_wall: f64,
    /// How far the drawn time runs ahead of the wall clock.
    lead: Option<Lead>,
    shown: f64,
}

/// The drawn time's lead over the wall clock: the best a recent heartbeat has
/// supported, rather than the latest one's.
///
/// A heartbeat is seen at the frame that drains it, not when its block was
/// due, and how long it waited for that frame varies block to block — by up
/// to a whole frame, since blocks and frames run at unrelated rates. Taking
/// each heartbeat's own lead made that wait the drawn time's: at 144 Hz over
/// 512-sample blocks it advanced 10.7, 6.9 and 3.7 ms on successive frames,
/// and anything scrolling on that clock moved in steps. The least-delayed
/// heartbeat is the one closest to when its block was due, so the best lead
/// over a few dozen blocks runs the picture at wall speed from that phase.
///
/// Recent, not ever: a lead the audio no longer supports — a short dropout
/// that lost audio time, or a sample clock running slow against the wall —
/// ages out of the window, and `shown` then holds until the audio catches up.
/// Kept as two buckets so the window costs nothing to slide; it spans between
/// one and three of them.
///
/// And never more than [`Lead::MAX_WAIT_SECONDS`] past the latest lead, which
/// is the most a heartbeat's wait for its frame can explain. A source running
/// slower than realtime — an overloaded engine, a slow export — supports a
/// lower lead with every block, and holding the best one ran the picture at
/// wall speed and froze it by turns, with new data landing a quarter second
/// short of the now-line. Capped, the drawn time follows such a source as
/// closely as it did when each heartbeat set it outright.
struct Lead {
    previous: f64,
    current: f64,
    latest: f64,
    since: f64,
}

impl Lead {
    /// Holds a few dozen 512-sample blocks, so one of them met its frame
    /// almost at once. At 4096 samples it holds only a handful and leaves
    /// about a millisecond of unevenness; a longer bucket would smooth that,
    /// and would also hold a lead the audio dropped for longer.
    const BUCKET_SECONDS: f64 = 0.25;

    /// A frame interval down to 20 fps, which covers every frame cap and the
    /// closed window's analyzer poll, plus delivery jitter.
    const MAX_WAIT_SECONDS: f64 = 0.05;

    fn new(lead: f64, wall: f64) -> Self {
        Lead { previous: lead, current: lead, latest: lead, since: wall }
    }

    fn push(&mut self, lead: f64, wall: f64) {
        let age = wall - self.since;
        if age >= 2.0 * Self::BUCKET_SECONDS {
            *self = Lead::new(lead, wall);
            return;
        }
        if age >= Self::BUCKET_SECONDS {
            (self.previous, self.current, self.since) = (self.current, lead, wall);
        } else {
            self.current = self.current.max(lead);
        }
        self.latest = lead;
    }

    fn get(&self) -> f64 {
        self.previous.max(self.current).min(self.latest + Self::MAX_WAIT_SECONDS)
    }
}

impl ClockMapper {
    /// Ignore ordinary callback jitter; a real callback pause leaves room for
    /// wall-clock fades before the source resumes. Transport stops usually keep
    /// streaming silence and need no special handling.
    const PAUSE_SECONDS: f64 = 1.0;

    pub fn new() -> Self {
        ClockMapper {
            offset: None,
            observed_audio: None,
            observed_wall: 0.0,
            lead: None,
            shown: 0.0,
        }
    }

    /// Observe a fresh audio heartbeat, independently of historical delivery.
    /// An idle polling pass must not re-anchor the last callback clock.
    pub fn observe(&mut self, newest_audio_time: f64, gui_now: f64) {
        if self.observed_audio == Some(newest_audio_time) {
            return;
        }
        let candidate = self.shown.max(gui_now) - newest_audio_time;
        let offset = match (self.offset, self.observed_audio) {
            (Some(offset), Some(previous)) if newest_audio_time >= previous => {
                let pause = (gui_now - self.observed_wall) - (newest_audio_time - previous);
                offset + if pause > Self::PAUSE_SECONDS { pause } else { 0.0 }
            }
            // A re-anchor moves the lead by however far it moved the offset,
            // so the leads before it describe a different mapping.
            _ => {
                self.lead = None;
                candidate
            }
        };
        self.offset = Some(offset);
        self.observed_audio = Some(newest_audio_time);
        self.observed_wall = gui_now;
        let lead = newest_audio_time + offset - gui_now;
        match &mut self.lead {
            Some(window) => window.push(lead, gui_now),
            None => self.lead = Some(Lead::new(lead, gui_now)),
        }
    }

    /// Follow fresh audio at wall speed; keep ageing when callbacks stop arriving.
    pub fn now(&mut self, wall_now: f64) -> f64 {
        let now = self.lead.as_ref().map_or(wall_now, |lead| wall_now + lead.get());
        // A lead that ages out of the window, or a short callback gap that
        // resumes behind an extrapolated frame, would step the drawn time
        // back. Hold until audio catches up rather than rewind histories
        // already pruned.
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

    const FRAME: f64 = 1.0 / 144.0;
    const BLOCK: f64 = 512.0 / 48_000.0;

    /// A 144 Hz window draining 512-sample blocks at 48 kHz, block `k` (whose
    /// last sample is due at `(k + 1) * BLOCK`) arriving at wall time
    /// `arrives(k)`. The two rates are incommensurate, so every frame finds
    /// its block waited a different length of time, and three seconds cycle
    /// through those phases many times.
    ///
    /// Returns, for each frame after the first half second, its drawn time
    /// and how far that was drawn past the newest audio it had.
    fn frames_at_144_hz(arrives: impl Fn(usize) -> f64) -> Vec<(f64, f64)> {
        let mut clock = ClockMapper::new();
        let mut newest = 0.0;
        let mut next = 0;
        let mut frames = Vec::new();
        for i in 1..432 {
            let wall = i as f64 * FRAME;
            while arrives(next) <= wall {
                next += 1;
                newest = next as f64 * BLOCK;
            }
            if next > 0 {
                clock.observe(newest, wall);
            }
            let now = clock.now(wall);
            if wall >= 0.5 {
                frames.push((now, now - (newest + clock.offset.unwrap())));
            }
        }
        frames
    }

    /// Realtime blocks, each delivered 2 ms after its last sample.
    fn on_time(k: usize) -> f64 {
        (k + 1) as f64 * BLOCK + 0.002
    }

    /// What used to move the notes in steps: each drain re-anchored the clock
    /// at the frame that found the block rather than when the block was due,
    /// so the drawn time advanced 10.7, 6.9 and 3.7 ms on successive 6.9 ms
    /// frames — the picture running at half and one and a half speed by turns.
    /// What is left is the window letting go of its best phase now and then,
    /// a few tenths of a millisecond.
    #[test]
    fn block_delivery_does_not_step_the_drawn_time() {
        let frames = frames_at_144_hz(on_time);
        let worst = frames.windows(2).map(|w| (w[1].0 - w[0].0 - FRAME).abs()).fold(0.0, f64::max);
        assert!(worst < 1e-3, "a frame advanced {worst:.4} s away from its own interval");
    }

    /// The audio still owns the clock: a stall that loses it 30 ms leaves the
    /// best lead from before it unsupported, and the window lets that go
    /// rather than keep drawing every later block 30 ms short of the now-line.
    #[test]
    fn a_lead_the_audio_lost_ages_out() {
        let stalled = |k| on_time(k) + if on_time(k) < 1.5 { 0.0 } else { 0.03 };
        let &(_, ahead) = frames_at_144_hz(stalled).last().unwrap();
        assert!(ahead < 0.015, "drawn {ahead:.3} s past the newest audio");
    }

    /// A source running at half speed — an overloaded engine, a slow export —
    /// supports a lower lead with every block. Holding the best one ran the
    /// picture at wall speed and froze it by turns, with new columns and notes
    /// landing a quarter second short of the now-line.
    #[test]
    fn a_slow_source_is_followed() {
        let frames = frames_at_144_hz(|k| 2.0 * on_time(k));
        let worst = frames.iter().map(|&(_, ahead)| ahead).fold(0.0, f64::max);
        assert!(worst < 0.1, "drawn {worst:.3} s past the newest audio");
    }

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
