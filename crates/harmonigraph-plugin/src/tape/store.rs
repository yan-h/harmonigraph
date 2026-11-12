//! The tape's recording: chunks of the mix keyed on song position, each from
//! one pass, under the warm/cold rule of #1719.
//!
//! Positions are song FRAMES at the store's rate (song seconds times the
//! rate, rounded). Chunks never overlap. A chunk's authoritative range is
//! `start..end`; its samples may run up to [`FADE_SECONDS`] past either end where a
//! newer winner trimmed it, which is what a seam between two passes crossfades
//! through.
//!
//! One recording per chunk. Keeping a cold pass beside a warm one (the
//! tail-difference idea in #1719) would be a second map of the same shape,
//! decided by [`beats`] per layer instead of by a single winner; nothing here
//! assumes there is only one map.

use std::collections::BTreeMap;

/// How far a trimmed chunk keeps its samples past the cut, and how long a seam
/// or an edge fades: 5 ms.
pub(crate) const FADE_SECONDS: f64 = 0.005;

/// [`FADE_SECONDS`] in frames at `rate`, and never none: the one length every
/// fade, seam and kept margin of the tape uses.
pub(crate) fn fade_frames(rate: f64) -> usize {
    (FADE_SECONDS * rate).ceil().max(1.0) as usize
}

/// How long a pass must have played before its audio counts as warm. A guess
/// good for reverb tails and bus state; see "Warm and cold" in #1719.
pub(crate) const SETTLE_SECONDS: f64 = 10.0;

/// A pass that started this close to the song's start is exact from its first
/// sample, however short its run-up.
pub(crate) const SONG_START_SECONDS: f64 = 0.05;

/// Two cold passes that started closer than this are the same start played
/// again, and the newer recording wins.
const SAME_RUN_UP_SECONDS: f64 = 0.05;

/// No memory setting yet (#1719): one generous cap, about 46 minutes of song
/// in stereo at 48 kHz, dropping the oldest-recorded chunks first.
pub(crate) const CAP_BYTES: usize = 1 << 30;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stamp {
    /// Which pass recorded it: chunks of one pass join without a seam.
    pub pass: u64,
    /// When it was recorded: for the console, and the cap drops the earliest
    /// first.
    pub when: std::time::Instant,
    /// The song frame the pass started at. Run-up and warmth are computed
    /// from it at the frame asked about, never stored per chunk: a run-up
    /// taken at each chunk's own first frame made two passes with different
    /// starts trade places every chunk, since their chunks are cut at
    /// different frames.
    pub pass_start: i64,
}

impl Stamp {
    /// Song seconds the pass had played before `frame`.
    pub(crate) fn run_up(&self, frame: i64, rate: f32) -> f64 {
        (frame - self.pass_start) as f64 / f64::from(rate)
    }

    /// The pass started at the song's start, so all of it is exact.
    pub(crate) fn is_from_start(&self, rate: f32) -> bool {
        self.pass_start as f64 <= SONG_START_SECONDS * f64::from(rate)
    }

    pub(crate) fn warm(&self, frame: i64, rate: f32) -> bool {
        self.is_from_start(rate) || self.run_up(frame, rate) >= SETTLE_SECONDS
    }
}

/// Whether a `new` recording replaces an `old` one from song frame `at`, the
/// first frame they share.
///
/// Warm audio replaces anything, the newer warm audio being the fresher.
/// Cold audio never replaces warm audio. Between two cold passes the one that
/// started earlier has the longer run-up at every frame, so it wins; the same
/// start played again wins as the fresher.
pub(crate) fn beats(new: Stamp, old: Stamp, at: i64, rate: f32) -> bool {
    if new.warm(at, rate) {
        return true;
    }
    let same_start = SAME_RUN_UP_SECONDS * f64::from(rate);
    !old.warm(at, rate) && new.pass_start as f64 <= old.pass_start as f64 + same_start
}

pub(crate) struct Chunk {
    pub start: i64,
    pub end: i64,
    pub stamp: Stamp,
    /// The first frame `samples` holds: at most `start`, and no more than
    /// [`FADE_SECONDS`] before it.
    pub data_start: i64,
    /// Interleaved, `channels` per frame.
    pub samples: Vec<f32>,
}

impl Chunk {
    fn data_end(&self, channels: usize) -> i64 {
        self.data_start + (self.samples.len() / channels) as i64
    }

    fn bytes(&self) -> usize {
        self.samples.len() * std::mem::size_of::<f32>()
    }

    /// The sample at song frame `frame`, channel `channel`, if held.
    fn at(&self, frame: i64, channel: usize, channels: usize) -> Option<f32> {
        let index = frame - self.data_start;
        (index >= 0 && frame < self.data_end(channels))
            .then(|| self.samples[index as usize * channels + channel])
    }

    /// A copy of `from..to`, keeping up to `margin` frames of samples past
    /// either cut.
    fn keep(&self, from: i64, to: i64, margin: i64, channels: usize) -> Chunk {
        let data_start = (from - margin).max(self.data_start);
        let data_end = (to + margin).min(self.data_end(channels));
        let first = (data_start - self.data_start) as usize * channels;
        let last = (data_end - self.data_start) as usize * channels;
        Chunk {
            start: from,
            end: to,
            stamp: self.stamp,
            data_start,
            samples: self.samples[first..last].to_vec(),
        }
    }
}

/// What [`Store::render`] filled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rendered {
    pub frames: usize,
    /// Recorded coverage stops at the end of what was filled.
    pub ended: bool,
}

pub(crate) struct Store {
    pub rate: f32,
    pub channels: usize,
    chunks: BTreeMap<i64, Chunk>,
    bytes: usize,
    cap: usize,
    /// Chunks dropped for the cap since the store was made or cleared.
    pub evicted: u64,
}

impl Store {
    pub(crate) fn new(rate: f32, channels: usize) -> Store {
        Store::with_cap(rate, channels, CAP_BYTES)
    }

    pub(crate) fn with_cap(rate: f32, channels: usize, cap: usize) -> Store {
        Store { rate, channels, chunks: BTreeMap::new(), bytes: 0, cap, evicted: 0 }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(test)]
    pub(crate) fn chunks(&self) -> impl Iterator<Item = &Chunk> {
        self.chunks.values()
    }

    fn margin(&self) -> i64 {
        fade_frames(f64::from(self.rate)) as i64
    }

    fn put(&mut self, chunk: Chunk) {
        if chunk.end > chunk.start {
            self.bytes += chunk.bytes();
            self.chunks.insert(chunk.start, chunk);
        }
    }

    fn take(&mut self, start: i64) -> Chunk {
        let chunk = self.chunks.remove(&start).expect("a listed chunk");
        self.bytes -= chunk.bytes();
        chunk
    }

    /// The chunk holding song frame `frame`.
    fn holding(&self, frame: i64) -> Option<&Chunk> {
        self.chunks.range(..=frame).next_back().map(|(_, c)| c).filter(|c| c.end > frame)
    }

    /// Commit a newly recorded chunk, which wins or loses each overlap by
    /// [`beats`]. A loser keeps what lies outside the winner, with margin.
    pub(crate) fn insert(&mut self, chunk: Chunk) {
        let (a, b) = (chunk.start, chunk.end);
        let margin = self.margin();
        let channels = self.channels;
        let overlapping: Vec<i64> = self
            .chunks
            .range(..b)
            .rev()
            .take_while(|(_, old)| old.end > a)
            .map(|(&start, _)| start)
            .collect();
        let mut lost = Vec::new();
        for start in overlapping {
            let old = self.take(start);
            if beats(chunk.stamp, old.stamp, old.start.max(a), self.rate) {
                if old.start < a {
                    self.put(old.keep(old.start, a, margin, channels));
                }
                if old.end > b {
                    self.put(old.keep(b, old.end, margin, channels));
                }
            } else {
                lost.push((old.start.max(a), old.end.min(b)));
                self.put(old);
            }
        }
        if lost.is_empty() {
            self.put(chunk);
        } else {
            lost.sort_unstable();
            let mut cursor = a;
            for (from, to) in lost {
                if from > cursor {
                    self.put(chunk.keep(cursor, from, margin, channels));
                }
                cursor = cursor.max(to);
            }
            if cursor < b {
                self.put(chunk.keep(cursor, b, margin, channels));
            }
        }
        while self.bytes > self.cap {
            let Some(oldest) = self.chunks.values().min_by_key(|c| c.stamp.when).map(|c| c.start)
            else {
                break;
            };
            self.take(oldest);
            self.evicted += 1;
        }
    }

    /// The recording's stamp at `frame`, if anything is recorded there.
    pub(crate) fn stamp_at(&self, frame: i64) -> Option<Stamp> {
        self.holding(frame).map(|c| c.stamp)
    }

    /// Fill `out` (interleaved, [`Self::channels`] per frame) from song frame
    /// `from` on, as far as coverage runs unbroken.
    ///
    /// A pure function of position, so a crossfade or fade that a read
    /// boundary cuts through comes out the same on both sides of it:
    /// - a seam between two passes crossfades over [`FADE_SECONDS`] through
    ///   whichever side kept margin past the seam, or dips through silence if
    ///   neither did;
    /// - the last [`FADE_SECONDS`] before the end of coverage fade out.
    pub(crate) fn render(&self, from: i64, out: &mut [f32]) -> Rendered {
        let ch = self.channels;
        let limit = from + (out.len() / ch) as i64;
        let fade = self.margin();
        // The unbroken chain of chunks from `from`, one past `limit` if it
        // continues there, so a fade whose window reaches back into the range
        // is seen.
        let mut chain: Vec<&Chunk> = Vec::new();
        let Some(first) = self.holding(from) else {
            return Rendered { frames: 0, ended: true };
        };
        // The predecessor, for a seam window that reaches past `from`.
        let before = self
            .chunks
            .range(..first.start)
            .next_back()
            .map(|(_, c)| c)
            .filter(|c| c.end == first.start);
        chain.push(first);
        let mut coverage_end = None;
        loop {
            let last = chain[chain.len() - 1];
            if last.end > limit + fade {
                break;
            }
            match self.chunks.get(&last.end) {
                Some(next) => chain.push(next),
                None => {
                    coverage_end = Some(last.end);
                    break;
                }
            }
        }
        let end = coverage_end.map_or(limit, |e| e.min(limit));
        let frames = (end - from).max(0) as usize;
        for f in from..end {
            let chunk = chain.iter().find(|c| c.end > f).expect("chain covers the range");
            let i = (f - from) as usize * ch;
            for c in 0..ch {
                out[i + c] = chunk.at(f, c, ch).expect("inside its range");
            }
        }
        let mut seams: Vec<(&Chunk, &Chunk)> = Vec::new();
        if let Some(before) = before {
            seams.push((before, first));
        }
        seams.extend(chain.windows(2).map(|w| (w[0], w[1])));
        for (a, b) in seams {
            if a.stamp.pass == b.stamp.pass {
                continue;
            }
            let s = a.end;
            let (lo, hi, dip) = if a.data_end(ch) >= s + fade {
                (s, s + fade, false)
            } else if b.data_start <= s - fade {
                (s - fade, s, false)
            } else {
                (s - fade, s + fade, true)
            };
            for f in lo.max(from)..hi.min(end) {
                let i = (f - from) as usize * ch;
                for c in 0..ch {
                    out[i + c] = if dip {
                        if f < s {
                            a.at(f, c, ch).unwrap_or(0.0) * (s - f) as f32 / fade as f32
                        } else {
                            b.at(f, c, ch).unwrap_or(0.0) * (f - s + 1) as f32 / fade as f32
                        }
                    } else {
                        let w = (f - lo) as f32 / (hi - lo) as f32;
                        a.at(f, c, ch).unwrap_or(0.0) * (1.0 - w)
                            + b.at(f, c, ch).unwrap_or(0.0) * w
                    };
                }
            }
        }
        if let Some(e) = coverage_end {
            for f in (e - fade).max(from)..end {
                let gain = (e - f) as f32 / fade as f32;
                let i = (f - from) as usize * ch;
                out[i..i + ch].iter_mut().for_each(|s| *s *= gain);
            }
        }
        Rendered { frames, ended: coverage_end.is_some_and(|e| e <= limit) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 1000.0;

    /// The frames of the settling time.
    const SETTLE: i64 = (SETTLE_SECONDS * RATE as f64) as i64;

    /// Pass `pass`, started at song frame `pass_start`, recorded in pass order.
    fn stamp(pass: u64, pass_start: i64) -> Stamp {
        let when = std::time::Instant::now() + std::time::Duration::from_secs(pass);
        Stamp { pass, when, pass_start }
    }

    /// A mono chunk whose every sample is `value`.
    fn chunk(start: i64, end: i64, stamp: Stamp, value: f32) -> Chunk {
        Chunk { start, end, stamp, data_start: start, samples: vec![value; (end - start) as usize] }
    }

    fn owners(store: &Store) -> Vec<(i64, i64, u64)> {
        store.chunks().map(|c| (c.start, c.end, c.stamp.pass)).collect()
    }

    #[test]
    fn warm_replaces_anything_and_cold_only_fills_gaps_or_beats_a_shorter_run_up() {
        // Judged at a frame 20 s into the song.
        let at = 2 * SETTLE;
        let warm = stamp(1, at - SETTLE);
        let cold = stamp(2, at - 1000);
        let colder = stamp(3, at - 500);
        let from_start = stamp(4, 0);
        let beats = |new, old| beats(new, old, at, RATE);
        assert!(warm.warm(at, RATE) && from_start.warm(at, RATE) && !cold.warm(at, RATE));
        assert!(beats(warm, cold) && beats(warm, warm) && beats(from_start, warm));
        assert!(!beats(cold, warm) && !beats(cold, from_start));
        assert!(beats(cold, colder) && !beats(colder, cold));
        // The same spot played again: the fresher recording wins.
        assert!(beats(stamp(5, at - 1000), cold));
    }

    #[test]
    fn a_cold_pass_fills_around_warm_audio_and_a_warm_one_overwrites_it() {
        let mut store = Store::new(RATE, 1);
        // 20 s into the song, so a pass can be warm without starting at zero.
        let o = 2 * SETTLE;
        store.insert(chunk(o + 1000, o + 2000, stamp(1, o + 1000 - SETTLE), 1.0));
        // Cold over the warm chunk's tail and past it: only the gap is taken.
        store.insert(chunk(o + 1500, o + 2500, stamp(2, o + 1500), 2.0));
        assert_eq!(owners(&store), [(o + 1000, o + 2000, 1), (o + 2000, o + 2500, 2)]);
        // The loser kept margin before the seam, for the crossfade.
        let margin = (FADE_SECONDS * f64::from(RATE)).ceil() as i64;
        assert_eq!(store.chunks().nth(1).unwrap().data_start, o + 2000 - margin);
        // Warm through the middle splits the old warm chunk around it.
        store.insert(chunk(o + 1200, o + 1400, stamp(3, o + 1200 - SETTLE), 3.0));
        assert_eq!(
            owners(&store),
            [
                (o + 1000, o + 1200, 1),
                (o + 1200, o + 1400, 3),
                (o + 1400, o + 2000, 1),
                (o + 2000, o + 2500, 2)
            ]
        );
    }

    #[test]
    fn the_cap_drops_the_oldest_recorded_chunks_first() {
        let bytes = 1000 * std::mem::size_of::<f32>();
        let mut store = Store::with_cap(RATE, 1, 2 * bytes);
        for pass in [3, 1, 2] {
            let start = pass as i64 * 1000;
            store.insert(chunk(start, start + 1000, stamp(pass, 0), 0.0));
        }
        assert_eq!(owners(&store), [(2000, 3000, 2), (3000, 4000, 3)]);
        assert_eq!(store.evicted, 1);
        assert!(store.bytes() <= 2 * bytes);
    }

    #[test]
    fn rendering_crosses_a_seam_through_the_losers_margin_and_fades_at_the_end() {
        let mut store = Store::new(RATE, 1);
        store.insert(chunk(0, 1000, stamp(1, 0), 1.0));
        store.insert(chunk(500, 1500, stamp(2, 500), 2.0));
        let fade = (FADE_SECONDS * f64::from(RATE)).ceil() as usize;
        let mut whole = vec![0.0; 2000];
        let rendered = store.render(0, &mut whole);
        assert_eq!(rendered, Rendered { frames: 1500, ended: true });
        // The second pass kept margin before 1000, so the seam fades into it.
        assert_eq!(whole[1000 - fade - 1], 1.0);
        assert!(whole[1000 - fade / 2] > 1.0 && whole[1000 - fade / 2] < 2.0);
        assert_eq!(whole[1000], 2.0);
        // The edge of coverage fades out.
        assert!(whole[1499] < 2.0 / fade as f32 + f32::EPSILON);
        // Reading in pieces that cut through both windows gives the same audio.
        let mut pieces = Vec::new();
        let mut at = 0;
        loop {
            let mut out = vec![0.0; 3];
            let r = store.render(at, &mut out);
            pieces.extend_from_slice(&out[..r.frames]);
            at += r.frames as i64;
            if r.ended {
                break;
            }
        }
        assert_eq!(pieces, whole[..1500]);
    }

    #[test]
    fn a_seam_with_no_margin_on_either_side_dips_through_silence() {
        let mut store = Store::new(RATE, 1);
        store.insert(chunk(0, 1000, stamp(1, 0), 1.0));
        store.insert(chunk(1000, 2000, stamp(2, 1000), 1.0));
        let mut out = vec![0.0; 2000];
        store.render(0, &mut out);
        let fade = (FADE_SECONDS * f64::from(RATE)).ceil() as usize;
        assert_eq!(out[1000 - fade - 1], 1.0);
        assert!(out[999] < 0.5 && out[1000] < 0.5);
        assert_eq!(out[1000 + fade], 1.0);
        // Nothing recorded at the cue: an immediate end.
        assert_eq!(store.render(5000, &mut out), Rendered { frames: 0, ended: true });
    }
}
