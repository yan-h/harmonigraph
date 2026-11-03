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
/// A heartbeat is seen at the drain that finds it, not when its block was
/// due, and how long it waited for that drain varies block to block — by up
/// to a whole drain interval, since blocks and drains run at unrelated rates.
/// Drains were frames when this was measured; since #1566 they are the input
/// worker's steps, every 4 ms with the window open and 20 ms with it shut,
/// which shrinks the wait this window exists to absorb. Taking
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
/// A bucket is as long as it takes to see a heartbeat that met its drain
/// almost at once, which depends on the drain and block lengths together:
/// the waits spread over the shorter of the two, and one heartbeat arrives
/// per the longer, so a window of `T` seconds finds a best wait within about
/// `drain * block / T`. At the worker's 4 ms that is a quarter second below
/// 1,500-sample blocks and 0.68 s at 4096. It was sized from the frame
/// length while frames drained, when a fixed quarter second held only three
/// 4096-sample blocks and each rollover stepped a 66.7 ms frame by up to
/// 10.7 ms; at 4 ms even that fixed quarter second steps one by about 1.3 ms,
/// and the sizing mostly serves the frame drains left when the worker fails
/// to spawn (a shut window draws nothing for its 20 ms drains to step).
///
/// And never more than its own wait for its drain, plus delivery jitter, past
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
    /// few dozen 512-sample blocks already met its drain almost at once.
    const MIN_BUCKET_SECONDS: f64 = 0.25;

    /// The longest a lead the audio dropped is held for, at up to three
    /// buckets. With the window open, at 4 ms drains, a drain must find over
    /// 0.375 s of audio to reach it — a hitch, which also lengthens the gap,
    /// or a bounce running far faster than realtime, whose advance overstates
    /// the block. With it shut, at 20 ms, blocks over about 3,500 samples
    /// reach it as well.
    /// Without it a bounce's advance would hold a dropped lead for minutes
    /// (`a_bounce_holds_a_dropped_lead_for_seconds_not_minutes`).
    const MAX_BUCKET_SECONDS: f64 = 3.0;

    /// How close to the best possible wait the window's best one should be.
    /// Losing that much at a rollover is a step of that much.
    const RESOLUTION_SECONDS: f64 = 0.0005;

    /// The longest drain gap taken as a heartbeat's wait for its drain. The
    /// input worker drains every 4 ms with the window open and every 20 ms
    /// with it shut; a longer gap is a hitch, and taken whole it would let the
    /// ceiling hold a lead the audio dropped.
    const MAX_WAIT_SECONDS: f64 = 0.1;

    /// How much later one block may be delivered than another, past the wait
    /// for its drain, before the ceiling holds the drawn time back. A 4096-
    /// sample block's processing can vary by tens of milliseconds, and 20 ms
    /// stepped a 144 Hz picture by 25 ms when one in ten came 20–50 ms late.
    const JITTER_SECONDS: f64 = 0.045;

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
                if pause > Self::PAUSE_SECONDS {
                    self.lead = None;
                    offset + pause
                } else {
                    offset
                }
            }
            // A re-anchor moves the lead by however far it moved the offset,
            // so the leads before it describe a different mapping. So does a
            // pause, by the difference between the waits of the heartbeats
            // either side of it.
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

    /// The input worker's cadence with the window open: `background::POLL`
    /// plus a little for the step itself. Its drains observe the clock; frames
    /// only read it. Not 4 ms exactly: that divides 512-sample blocks into
    /// three phases, and a run would never see the rest.
    const DRAIN: f64 = 0.00413;

    /// A window drawing every `frame` seconds for `seconds`, its worker
    /// draining every `drain` seconds, blocks of `block` seconds, block `k`
    /// (whose last sample is due at `(k + 1) * block`) arriving at wall time
    /// `arrives(k)`. Incommensurate rates make every drain find its block
    /// waited a different length of time, and the run cycles through those
    /// phases many times.
    ///
    /// Returns, for each frame after the first half second, its drawn time
    /// and how far that was drawn past the newest audio it had.
    fn frames_at(
        frame: f64,
        block: f64,
        seconds: f64,
        drain: f64,
        arrives: impl Fn(usize) -> f64,
    ) -> Vec<(f64, f64)> {
        let mut clock = ClockMapper::new();
        let mut newest = 0.0;
        let mut next = 0;
        let mut drains = 0;
        let mut frames = Vec::new();
        for i in 1..(seconds / frame) as usize {
            let wall = i as f64 * frame;
            while (drains + 1) as f64 * drain <= wall {
                drains += 1;
                let at = drains as f64 * drain;
                while arrives(next) <= at {
                    next += 1;
                    newest = next as f64 * block;
                }
                if next > 0 {
                    clock.observe(newest, at);
                }
            }
            let now = clock.now(wall);
            if wall >= 0.5 {
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
    /// at the one that found the block rather than when the block was due, so
    /// the drawn time advanced 10.7, 6.9 and 3.7 ms on successive 6.9 ms
    /// frames — the picture running at half and one and a half speed by turns.
    /// What is left is the window letting go of its best phase now and then, a
    /// few tenths of a millisecond for 512-sample blocks and about a
    /// millisecond for 4096, at every frame rate a display link runs at.
    /// Twelve seconds roll the longest window over several times. The frames
    /// of the first second are skipped, while the window first fills.
    ///
    /// Drained as the two real configurations drain. The input worker with
    /// the window open drains every [`DRAIN`], whatever the frame rate. The
    /// frame-drain fallback, when the worker's thread could not be spawned
    /// (`EditorShared::frame_drains`), drains once per frame, just before the
    /// frame reads the clock: at 60 and 15 fps that is a 17 and 67 ms gap,
    /// which is what sizes the lead's buckets past their minimum. A fixed
    /// quarter second bucket does as well as the sized one at [`DRAIN`], but
    /// steps 4096-sample blocks by 8.7 ms at 60 fps and 10.7 ms at 15, where
    /// the sized one steps 0.7 and 0.0 ms; these limits refuse the former. A
    /// worker shut with the window drains at its own slower cadence, but then
    /// no frame is drawn to step, so that pairing is not a row. 512-sample
    /// blocks are not run through the fallback: at 60 fps they step 0.67 ms,
    /// which does not fit the 0.5 ms limit, and the fallback is the rare path.
    #[test]
    fn block_delivery_does_not_step_the_drawn_time() {
        // A drain of `None` is the frame-drain fallback: one per frame.
        let (fast, slow) = ([FRAME, 1.0 / 60.0, 1.0 / 15.0], [1.0 / 60.0, 1.0 / 15.0]);
        for (drain, samples, limit, frames) in [
            (Some(DRAIN), 512.0, 5e-4, &fast[..]),
            (Some(DRAIN), 4096.0, 1.5e-3, &fast[..]),
            (None, 4096.0, 1.5e-3, &slow[..]),
        ] {
            let block = samples / 48_000.0;
            for &frame in frames {
                let every = drain.unwrap_or(frame);
                let frames = frames_at(frame, block, 12.0, every, on_time(block));
                let worst = worst_step(&frames[(0.5 / frame) as usize..], frame);
                assert!(
                    worst < limit,
                    "{samples} samples drained every {every:.4} s at {frame:.4} s a frame \
                     advanced {worst:.4} s away from it"
                );
            }
        }
    }

    /// The audio still owns the clock: a stall that loses it 30 ms leaves the
    /// best lead from before it unsupported, and the window lets that go
    /// rather than keep drawing every later block 30 ms short of the now-line.
    #[test]
    fn a_lead_the_audio_lost_ages_out() {
        let on_time = on_time(BLOCK);
        let stalled = |k| on_time(k) + if on_time(k) < 1.5 { 0.0 } else { 0.03 };
        let &(_, ahead) = frames_at(FRAME, BLOCK, 3.0, DRAIN, stalled).last().unwrap();
        assert!(ahead < 0.015, "drawn {ahead:.3} s past the newest audio");
    }

    /// A drain that finds a second of audio — a bounce at a few hundred times
    /// realtime, or a hitch — opens a bucket sized from that advance, and a
    /// lead the audio then drops is held as long as the bucket runs. The
    /// ceiling on a bucket keeps that to seconds: sized from the advance alone,
    /// this one would hold 20 ms past the audio for eight.
    #[test]
    fn a_bounce_holds_a_dropped_lead_for_seconds_not_minutes() {
        let mut lead = super::Lead::new(0.02, 0.0, DRAIN, 1.0);
        let mut wall = 0.0;
        while wall < 4.0 {
            wall += DRAIN;
            lead.push(0.0, wall, DRAIN, BLOCK);
        }
        assert!(lead.get() < 0.001, "drawn {:.3} s past the audio after {wall:.1} s", lead.get());
    }

    /// A source running at half speed — an overloaded engine, a slow export —
    /// supports a lower lead with every block. Holding the best one ran the
    /// picture at wall speed and froze it by turns, with new columns and notes
    /// landing a quarter second short of the now-line.
    #[test]
    fn a_slow_source_is_followed() {
        let on_time = on_time(BLOCK);
        let frames = frames_at(FRAME, BLOCK, 3.0, DRAIN, |k| 2.0 * on_time(k));
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
