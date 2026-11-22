//! The one presentation clock: where each callback starts and ends in
//! presentation seconds, and the mapping from steady samples to them, for the
//! live audio, the tape, notes, reset markers and a take's runs (#1885).
//!
//! Presentation time advances only by the audio `play` processed. A callback
//! that starts anywhere but where the last processed one ended (the host's
//! counter skipped ahead, or the callback between processed no audio) pins a
//! new segment at the time reached, so the samples nobody processed take no
//! picture time and the notes after them sit on their audio. Each activation
//! pins afresh at its first valid callback, so a counter that went back across
//! one costs nothing either.
//!
//! The steady clock itself stays the host's, run on across a restart by the
//! shell: a Tune's records are matched to the Hub's callbacks on it, and each
//! instance closing its own gaps would set them apart.

use harmonigraph_clap::plugin::Boundary;

/// Steady samples to presentation seconds: one segment, from the first sample
/// of the processed run it was pinned at. A sample before the segment maps to
/// its start, so the mapping never goes back and every sample from before the
/// pin, a gap's included, lands at or before what follows it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Mapping {
    /// The steady sample the segment starts at, and its presentation time.
    pin: Option<(i64, f64)>,
    rate: f64,
}

impl Mapping {
    #[cfg(test)]
    pub fn pinned(start: i64, time: f64, rate: f64) -> Self {
        Self { pin: Some((start, time)), rate }
    }

    pub fn at(self, sample: i64) -> f64 {
        let Some((start, time)) = self.pin else { return 0.0 };
        if self.rate <= 0.0 {
            return time;
        }
        // In f64: a caller may pass `i64::MIN` for "never registered".
        time + (sample as f64 - start as f64).max(0.0) / self.rate
    }
}

#[derive(Default)]
pub(crate) struct PresentationClock {
    mapping: Mapping,
    /// Where the last processed callback ended, in presentation seconds.
    end: f64,
    /// The steady sample just past the last processed callback.
    ended: Option<i64>,
    /// This callback's first steady sample, once `begin` has seen it valid.
    block: Option<i64>,
}

impl PresentationClock {
    /// A new activation, perhaps at a new rate, pins afresh at its first
    /// valid callback, where the previous one left off.
    pub fn activate(&mut self, rate: f64) {
        *self = Self { mapping: Mapping { pin: None, rate }, end: self.end, ..Self::default() };
    }

    /// A callback's boundary. A valid one that does not start where the last
    /// processed one ended pins the segment there. An invalid one's steady
    /// time may be CLAP's -1 for "unknown": it pins nothing and processes
    /// nothing.
    pub fn begin(&mut self, boundary: &Boundary) {
        self.block = boundary.valid.then_some(boundary.steady_time);
        if boundary.valid && self.ended != Some(boundary.steady_time) {
            self.mapping.pin = Some((boundary.steady_time, self.end));
        }
    }

    /// `play` processed `frames` from this callback's start.
    pub fn processed(&mut self, frames: usize) {
        if let Some(start) = self.block {
            let ended = start.saturating_add(frames as i64);
            self.ended = Some(ended);
            self.end = self.mapping.at(ended);
        }
    }

    pub fn mapping(&self) -> Mapping {
        self.mapping
    }

    /// This callback's start, or where the clock stands without a valid one.
    pub fn start(&self) -> f64 {
        self.block.map_or(self.end, |start| self.mapping.at(start))
    }

    /// Where the last processed callback ended: the take lanes' clock, and
    /// the time a host reset is stamped at, which the next callback's notes
    /// start at or after (#1881).
    pub fn end(&self) -> f64 {
        self.end
    }

    #[cfg(test)]
    pub fn set_end(&mut self, end: f64) {
        self.end = end;
    }

    /// One processed callback of `frames` at `rate`, running on from the
    /// last: for a test that calls `play` without the shell's boundary.
    #[cfg(test)]
    pub fn step(&mut self, frames: usize, rate: f64) {
        if self.mapping.rate != rate {
            self.activate(rate);
        }
        let steady_time = self.ended.unwrap_or(0);
        let frames = frames as u32;
        self.begin(&Boundary {
            steady_time,
            frames,
            transport: None,
            valid: true,
            input_lost: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boundary(steady_time: i64) -> Boundary {
        Boundary { steady_time, frames: 64, transport: None, valid: true, input_lost: false }
    }

    /// A counter that skips ahead inside an activation, or a callback that
    /// processed nothing, takes no presentation time; a sample from before
    /// the pin maps to it.
    #[test]
    fn unprocessed_samples_take_no_presentation_time() {
        let mut clock = PresentationClock::default();
        clock.activate(64.0);
        for start in [0, 64, 128] {
            clock.begin(&boundary(start));
            clock.processed(64);
        }
        assert_eq!(clock.end(), 3.0);
        // Not processed: the next callback starts past where audio ended.
        clock.begin(&boundary(192));
        assert_eq!(clock.start(), 3.0);
        assert_eq!(clock.end(), 3.0);
        clock.begin(&boundary(10_000));
        assert_eq!(clock.start(), 3.0, "the skip closes");
        let mapping = clock.mapping();
        assert_eq!(mapping.at(10_064), 4.0);
        assert_eq!(mapping.at(5_000), 3.0, "a sample in the gap lands at its end");
        clock.processed(64);
        assert_eq!(clock.end(), 4.0);
        // An activation runs on from there, wherever its counter starts.
        clock.activate(128.0);
        clock.begin(&boundary(0));
        clock.processed(64);
        assert_eq!(clock.end(), 4.5);
    }
}
