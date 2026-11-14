//! A cue played slower than recorded, with its pitch held (#1719 slice 5):
//! the store's audio through Signalsmith Stretch, on the input worker.
//!
//! Output frame `j` of a cue is song frame `start + j * speed`, the same
//! mapping the deck counts the song by, so the notes, the cursor and the
//! readout follow what is heard. The stretcher is primed with the audio
//! before the spot ([`Stretched::new`]) and its pre-roll is dropped, so the
//! first frame out is the spot itself rather than the stretcher's latency.

use signalsmith_stretch::Stretch;

use super::store::{Rendered, Store};

pub(crate) struct Stretched {
    stretch: Stretch,
    channels: usize,
    speed: f64,
    /// The song frame the cue starts at: output frame 0.
    start: i64,
    /// Output frames the stretcher has made, its dropped pre-roll included.
    made: u64,
    /// Pre-roll still to drop, and all of it.
    skip: usize,
    dropped: u64,
    /// The song frame the next input starts at.
    fed: i64,
    /// Where coverage ends, once the input has reached it.
    end: Option<i64>,
    input: Vec<f32>,
    output: Vec<f32>,
}

impl Stretched {
    /// A cue from song frame `start` at `speed` (below 1).
    pub(crate) fn new(store: &Store, start: i64, speed: f64) -> Stretched {
        let channels = store.channels;
        let mut stretch = Stretch::preset_default(channels as u32, store.rate as u32);
        let lead = stretch.input_latency() as i64;
        let skip = stretch.output_latency();
        // At least a block and an interval of the audio before the spot, as
        // the library asks, ending `lead` past it: its processing time is
        // then the spot, `lead` behind the input.
        let preroll = 3 * lead;
        let mut input = vec![0.0; preroll as usize * channels];
        let from = start + lead - preroll;
        read(store, from, &mut input);
        // Coverage that ends inside the look-ahead ends the cue there, and
        // what lies past the gap is no part of it.
        let mut ahead = vec![0.0; lead as usize * channels];
        let covered = store.render(start, &mut ahead);
        let end = covered.ended.then_some(start + covered.frames as i64);
        if let Some(end) = end {
            input[(end - from) as usize * channels..].fill(0.0);
        }
        stretch.seek(&input, speed);
        Stretched {
            stretch,
            channels,
            speed,
            start,
            made: 0,
            skip,
            dropped: skip as u64,
            fed: start + lead,
            end,
            input,
            output: Vec::new(),
        }
    }

    /// The next frames of the cue into `out`, as many as it holds or as are
    /// left before the end of coverage.
    pub(crate) fn render(&mut self, store: &Store, out: &mut [f32]) -> Rendered {
        let ch = self.channels;
        let want = out.len() / ch;
        let made = self.made + (self.skip + want) as u64;
        let input = (made as f64 * self.speed).round() as i64
            - (self.made as f64 * self.speed).round() as i64;
        self.input.clear();
        self.input.resize(input.max(0) as usize * ch, 0.0);
        if self.end.is_none() {
            let rendered = store.render(self.fed, &mut self.input);
            if rendered.ended {
                self.end = Some(self.fed + rendered.frames as i64);
            }
        }
        self.fed += input;
        self.output.clear();
        self.output.resize((self.skip + want) * ch, 0.0);
        self.stretch.process(&self.input, &mut self.output);
        self.made = made;
        let skipped = std::mem::take(&mut self.skip);
        // The cue's frames out before this call; the last to sound is the one
        // that reaches the end of coverage.
        let emitted = self.made - self.dropped - want as u64;
        let (frames, ended) = match self.end {
            Some(end) => {
                let last = ((end - self.start) as f64 / self.speed).floor().max(0.0) as u64;
                let left = last.saturating_sub(emitted) as usize;
                (left.min(want), left <= want)
            }
            None => (want, false),
        };
        out[..frames * ch].copy_from_slice(&self.output[skipped * ch..(skipped + frames) * ch]);
        Rendered { frames, ended }
    }
}

/// Song frames `from..` into `out`, silent wherever nothing is recorded.
fn read(store: &Store, from: i64, out: &mut [f32]) {
    let ch = store.channels;
    let end = from + (out.len() / ch) as i64;
    let mut frame = from;
    while frame < end {
        let at = (frame - from) as usize * ch;
        let rendered = store.render(frame, &mut out[at..]);
        frame += rendered.frames as i64;
        if rendered.ended {
            frame = store.next_start(frame).map_or(end, |next| next.min(end));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::*;
    use crate::tape::store::{Chunk, Stamp};

    const RATE: f32 = 48_000.0;

    /// Three seconds of song: silence, then 440 Hz from 1.5 s. An onset to
    /// time and a pitch to hold.
    fn store() -> Store {
        let frames = 3 * RATE as usize;
        let samples = (0..frames)
            .map(|i| {
                let t = i as f32 / RATE;
                if t >= 1.5 {
                    0.5 * (TAU * 440.0 * t).sin()
                } else {
                    0.0
                }
            })
            .collect();
        let stamp = Stamp { pass: 1, when: std::time::Instant::now(), pass_start: 0 };
        let mut store = Store::new(RATE, 1);
        store.insert(Chunk { start: 0, end: frames as i64, stamp, data_start: 0, samples });
        store
    }

    /// Everything a cue from song second 1 sounds at `speed`, in blocks the
    /// size the worker asks for.
    fn heard(store: &Store, speed: f64) -> Vec<f32> {
        let mut stretched = Stretched::new(store, RATE as i64, speed);
        let (mut heard, mut block) = (Vec::new(), vec![0.0; 1024]);
        loop {
            let rendered = stretched.render(store, &mut block);
            heard.extend_from_slice(&block[..rendered.frames]);
            if rendered.ended {
                return heard;
            }
            assert!(heard.len() < 10 * RATE as usize, "coverage never ended");
        }
    }

    #[test]
    fn a_slowed_cue_starts_at_its_spot_holds_its_pitch_and_ends_with_coverage() {
        let store = store();
        for speed in [0.75, 0.5, 0.25] {
            let heard = heard(&store, speed);
            let seconds = |frames: usize| frames as f64 / f64::from(RATE);
            // The two seconds of song after the spot, slowed.
            assert!((seconds(heard.len()) - 2.0 / speed).abs() < 1e-3, "{speed}: {}", heard.len());
            // Past a 2x stretch the library randomises phases, unseeded
            // (`maxCleanStretch` in signalsmith-stretch.h): over 30 runs at
            // 0.25x an onset landed 44 ms early to 13 ms late and the tone read
            // 439 to 444 Hz. Down to 2x both are exact on every run.
            let (early, off) = if speed < 0.5 { (0.08, 6) } else { (0.01, 1) };
            // The onset half a second of song in: the stretcher's latency is
            // primed and dropped, not heard as a delay.
            let onset = seconds(heard.iter().position(|s| s.abs() > 0.25).expect("the tone"));
            assert!((onset - 0.5 / speed).abs() < early, "{speed}: onset at {onset} s");
            // A steady second of the tone, well clear of its onset and the
            // fade at the end of coverage, still at 440 Hz.
            let from = ((0.5 / speed + 0.25) * f64::from(RATE)) as usize;
            let steady = &heard[from..from + RATE as usize];
            let rises = steady.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
            assert!(rises.abs_diff(440) <= off, "{speed}: {rises} Hz");
        }
    }

    /// Coverage that ends 20 ms after the spot, inside the stretcher's
    /// look-ahead, ends the cue there; the recording past the gap is never
    /// read into it.
    #[test]
    fn coverage_ending_inside_the_look_ahead_ends_the_cue_there() {
        let chunk = |start: f64, end: f64, pass, value: f32| {
            let (start, end) = ((start * RATE as f64) as i64, (end * RATE as f64) as i64);
            let stamp = Stamp { pass, when: std::time::Instant::now(), pass_start: start };
            let samples = vec![value; (end - start) as usize];
            Chunk { start, end, stamp, data_start: start, samples }
        };
        let mut store = Store::new(RATE, 1);
        store.insert(chunk(0.0, 1.02, 1, 0.0));
        store.insert(chunk(1.03, 2.0, 2, 0.9));
        let heard = heard(&store, 0.5);
        assert!((heard.len() as f64 / f64::from(RATE) - 0.04).abs() < 1e-3, "{}", heard.len());
        let loudest = heard.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(loudest < 1e-3, "the recording past the gap leaked in at {loudest}");
    }
}
