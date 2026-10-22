/// One presentation clock for notes, audio columns and drawing. While audio
/// flows its sample time owns the clock, including faster-than-realtime bounces.
/// Moving the offset towards wall time each drain compresses history across
/// batches: old columns keep their dates while new ones move backwards.
pub(crate) struct ClockMapper {
    /// Initial delivery offset, held until callbacks pause or the source rewinds.
    pub(super) offset: Option<f64>,
    observed_audio: Option<f64>,
    observed_wall: f64,
    /// When the clock was last read, fresh or not. A heartbeat waited at most
    /// from then for the read that found it.
    drained: f64,
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
/// A bucket is as long as it takes to see a heartbeat that met its frame
/// almost at once, which depends on the block and frame lengths together:
/// the waits spread over the shorter of the two, and one heartbeat arrives
/// per the longer, so a window of `T` seconds finds a best wait within about
/// `block * frame / T`. A fixed quarter second held a few dozen 512-sample
/// blocks but only three of 4096 samples, and each rollover then let go of
/// the best phase and stepped a 66.7 ms frame by up to 10.7 ms.
///
/// And never more than its own wait for its frame, plus delivery jitter, past
/// the latest lead, which is all that wait can explain. A source running
/// slower than realtime — an overloaded engine, a slow export — supports a
/// lower lead with every block, and holding the best one ran the picture at
/// wall speed and froze it by turns, with new data landing a quarter second
/// short of the now-line. Capped, the drawn time follows such a source as
/// closely as it did when each heartbeat set it outright.
struct Lead {
    previous: f64,
    current: f64,
    /// The highest lead the latest heartbeat's wait can explain.
    ceiling: f64,
    since: f64,
    /// How long the current bucket runs, set when it opens.
    bucket: f64,
}

impl Lead {
    /// Below a quarter second the window would gain nothing: the best of a
    /// few dozen 512-sample blocks already met its frame almost at once.
    const MIN_BUCKET_SECONDS: f64 = 0.25;

    /// The longest a lead the audio dropped is held for, at up to three
    /// buckets. Large blocks at the slowest frame rate reach it, and their few
    /// phases repeat within it: 4096 samples against 66.7 ms every 2.1 s.
    const MAX_BUCKET_SECONDS: f64 = 3.0;

    /// How close to the best possible wait the window's best one should be.
    /// Losing that much at a rollover is a step of that much.
    const RESOLUTION_SECONDS: f64 = 0.0005;

    /// The longest drain gap taken as a heartbeat's wait for its frame. The
    /// slowest frame rate, a quarter of 60 Hz, drains every 66.7 ms and the
    /// closed window's analyzer every 20 ms; a longer gap is a hitch, and
    /// taken whole it would let the ceiling hold a lead the audio dropped.
    const MAX_WAIT_SECONDS: f64 = 0.1;

    /// How much later one block may be delivered than another, past the wait
    /// for its frame, before the ceiling holds the drawn time back.
    const JITTER_SECONDS: f64 = 0.02;

    /// A heartbeat of `lead`, drained `gap` after the drain before it and
    /// `advance` of audio past the heartbeat before it.
    fn new(lead: f64, wall: f64, gap: f64, advance: f64) -> Self {
        Lead {
            previous: lead,
            current: lead,
            ceiling: lead + gap + Self::JITTER_SECONDS,
            since: wall,
            bucket: Self::bucket(gap, advance),
        }
    }

    /// The advance stands in for the block length, which it equals unless a
    /// drain found several blocks, and then overstates.
    fn bucket(gap: f64, advance: f64) -> f64 {
        (gap * advance / Self::RESOLUTION_SECONDS)
            .clamp(Self::MIN_BUCKET_SECONDS, Self::MAX_BUCKET_SECONDS)
    }

    fn push(&mut self, lead: f64, wall: f64, gap: f64, advance: f64) {
        let age = wall - self.since;
        if age >= 2.0 * self.bucket {
            *self = Lead::new(lead, wall, gap, advance);
            return;
        }
        if age >= self.bucket {
            (self.previous, self.current, self.since) = (self.current, lead, wall);
            self.bucket = Self::bucket(gap, advance);
        } else {
            self.current = self.current.max(lead);
        }
        self.ceiling = lead + gap + Self::JITTER_SECONDS;
    }

    fn get(&self) -> f64 {
        self.previous.max(self.current).min(self.ceiling)
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
            drained: 0.0,
            lead: None,
            shown: 0.0,
        }
    }

    /// Observe a fresh audio heartbeat, independently of historical delivery.
    /// An idle polling pass must not re-anchor the last callback clock.
    pub fn observe(&mut self, newest_audio_time: f64, gui_now: f64) {
        let gap =
            (gui_now - std::mem::replace(&mut self.drained, gui_now)).min(Lead::MAX_WAIT_SECONDS);
        if self.observed_audio == Some(newest_audio_time) {
            return;
        }
        let advance = self.observed_audio.map_or(0.0, |previous| newest_audio_time - previous);
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
            Some(window) => window.push(lead, gui_now, gap, advance),
            None => self.lead = Some(Lead::new(lead, gui_now, gap, advance)),
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

    /// A window drawing every `frame` seconds for `seconds`, draining blocks
    /// of `block` seconds, block `k` (whose last sample is due at
    /// `(k + 1) * block`) arriving at wall time `arrives(k)`. Incommensurate
    /// rates make every frame find its block waited a different length of
    /// time, and the run cycles through those phases many times.
    ///
    /// Returns, for each frame after the first second — long enough for the
    /// window to have seen the best phase — its drawn time and how far that
    /// was drawn past the newest audio it had.
    fn frames_at(
        frame: f64,
        block: f64,
        seconds: f64,
        arrives: impl Fn(usize) -> f64,
    ) -> Vec<(f64, f64)> {
        let mut clock = ClockMapper::new();
        let mut newest = 0.0;
        let mut next = 0;
        let mut frames = Vec::new();
        for i in 1..(seconds / frame) as usize {
            let wall = i as f64 * frame;
            while arrives(next) <= wall {
                next += 1;
                newest = next as f64 * block;
            }
            if next > 0 {
                clock.observe(newest, wall);
            }
            let now = clock.now(wall);
            if wall >= 1.0 {
                frames.push((now, now - (newest + clock.offset.unwrap())));
            }
        }
        frames
    }

    /// Realtime blocks, each delivered 2 ms after its last sample.
    fn on_time(block: f64) -> impl Fn(usize) -> f64 {
        move |k| (k + 1) as f64 * block + 0.002
    }

    /// How far the worst frame's advance departed from its own interval.
    fn worst_step(frames: &[(f64, f64)], frame: f64) -> f64 {
        frames.windows(2).map(|w| (w[1].0 - w[0].0 - frame).abs()).fold(0.0, f64::max)
    }

    /// What used to move the notes in steps: each drain re-anchored the clock
    /// at the frame that found the block rather than when the block was due,
    /// so the drawn time advanced 10.7, 6.9 and 3.7 ms on successive 6.9 ms
    /// frames — the picture running at half and one and a half speed by turns.
    /// What is left is the window letting go of its best phase now and then,
    /// a few tenths of a millisecond.
    #[test]
    fn block_delivery_does_not_step_the_drawn_time() {
        let worst = worst_step(&frames_at(FRAME, BLOCK, 3.0, on_time(BLOCK)), FRAME);
        assert!(worst < 1e-3, "a frame advanced {worst:.4} s away from its own interval");
    }

    /// The audio still owns the clock: a stall that loses it 30 ms leaves the
    /// best lead from before it unsupported, and the window lets that go
    /// rather than keep drawing every later block 30 ms short of the now-line.
    #[test]
    fn a_lead_the_audio_lost_ages_out() {
        let on_time = on_time(BLOCK);
        let stalled = |k| on_time(k) + if on_time(k) < 1.5 { 0.0 } else { 0.03 };
        let &(_, ahead) = frames_at(FRAME, BLOCK, 3.0, stalled).last().unwrap();
        assert!(ahead < 0.015, "drawn {ahead:.3} s past the newest audio");
    }

    /// A source running at half speed — an overloaded engine, a slow export —
    /// supports a lower lead with every block. Holding the best one ran the
    /// picture at wall speed and froze it by turns, with new columns and notes
    /// landing a quarter second short of the now-line.
    #[test]
    fn a_slow_source_is_followed() {
        let on_time = on_time(BLOCK);
        let frames = frames_at(FRAME, BLOCK, 3.0, |k| 2.0 * on_time(k));
        let worst = frames.iter().map(|&(_, ahead)| ahead).fold(0.0, f64::max);
        assert!(worst < 0.1, "drawn {worst:.3} s past the newest audio");
    }

    /// Large blocks at a slow frame rate: 4096 samples at 48 kHz is 85 ms, so
    /// a quarter second held three heartbeats, too few for one of them to
    /// have met its frame at once. Every rollover let go of the best phase and
    /// stepped a 66.7 ms frame by up to 10.7 ms, and a 16.7 ms one by 8.7 ms.
    /// Twelve seconds roll the longest window over several times.
    #[test]
    fn large_blocks_at_a_slow_frame_rate_do_not_step_the_drawn_time() {
        let block = 4096.0 / 48_000.0;
        for frame in [1.0 / 15.0, 1.0 / 60.0] {
            let worst = worst_step(&frames_at(frame, block, 12.0, on_time(block)), frame);
            assert!(worst < 1.5e-3, "a {frame:.4} s frame advanced {worst:.4} s away from it");
        }
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
