//! What the Tape tab shows, and what a click in it asks (#1719).
//!
//! The plugin's input worker fills a [`TapeView`] under the editor lock once a
//! step and takes its request. A shell with no tape leaves it at its default,
//! which says there is none.

/// What a click in the Tape tab asks of the plugin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TapeRequest {
    /// Audition from these song seconds.
    Cue(f64),
    /// End the audition.
    Stop,
}

/// The host's bar and beat at one song position, for the ruler. Every field
/// is the host's own: positions in quarter notes, bars numbered from 0. The
/// plugin builds one only with a positive tempo and a meter, defaulting to
/// 4/4.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Meter {
    pub beats: f64,
    /// Quarter notes per minute.
    pub tempo: f64,
    /// Where the bar holding `beats` starts, and its number.
    pub bar_start: f64,
    pub bar: i32,
    pub numerator: i32,
    pub denominator: i32,
}

impl Meter {
    /// A bar's length in quarter notes.
    fn bar_length(&self) -> f64 {
        f64::from(self.numerator) * 4.0 / f64::from(self.denominator)
    }

    /// Quarter notes from this anchor, at `seconds` from it.
    fn beats_after(&self, seconds: f64) -> f64 {
        self.beats + seconds * self.tempo / 60.0
    }

    /// Seconds from this anchor to quarter note `beats`.
    fn seconds_to(&self, beats: f64) -> f64 {
        (beats - self.beats) * 60.0 / self.tempo
    }
}

/// A stretch recorded by one pass, with no gap in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TapeSpan {
    /// Song seconds.
    pub start: f64,
    pub end: f64,
    /// Warm at its start: recorded after enough run-up, or from the song's
    /// start. A cold pass that plays on long enough turns warm, and the
    /// warm part is a span of its own.
    pub warm: bool,
    pub recorded: std::time::Instant,
}

#[derive(Clone, Debug, Default)]
pub struct TapeView {
    /// A tape exists here: the plugin has one, other shells do not.
    pub present: bool,
    /// The analysis input is Main and the host is not rendering offline: the
    /// only times the tape sounds. It records whenever the analysis input is
    /// Main, an export included.
    pub enabled: bool,
    /// The transport rolls, so a cue would be cut at once.
    pub rolling: bool,
    /// The song position the host last reported, in seconds. While stopped
    /// it is where playback stopped: Bitwig does not report a playhead moved
    /// while stopped.
    pub position: Option<f64>,
    /// The song seconds sounding now, while an audition plays.
    pub heard: Option<f64>,
    /// Memory the recording holds.
    pub bytes: usize,
    /// What is recorded, in song order.
    pub spans: Vec<TapeSpan>,
    /// The host's meter where recordings started, about a second apart, by
    /// song seconds and in song order. The newest recording's wins where two
    /// cover the same stretch.
    pub meters: Vec<(f64, Meter)>,
    /// A click the plugin has not taken yet. A newer click replaces it.
    pub request: Option<TapeRequest>,
}

impl TapeView {
    /// The anchor that governs `seconds`: the last at or before it, else the
    /// first, both read forward or back at their own tempo.
    fn anchor(&self, seconds: f64) -> Option<(f64, Meter)> {
        let after = self.meters.partition_point(|(at, _)| *at <= seconds);
        self.meters.get(after.saturating_sub(1)).copied()
    }

    /// How long a bar lasts at `seconds`.
    pub fn bar_seconds(&self, seconds: f64) -> Option<f64> {
        let (_, meter) = self.anchor(seconds)?;
        Some(meter.seconds_to(meter.beats + meter.bar_length()))
    }

    /// The bar (from 1, as Bitwig numbers them) and the beat in it (from 1,
    /// counted in the time signature's own note value) at `seconds`.
    pub fn bar_at(&self, seconds: f64) -> Option<(i32, i32)> {
        let (at, meter) = self.anchor(seconds)?;
        let beats = meter.beats_after(seconds - at);
        let length = meter.bar_length();
        let bars = ((beats - meter.bar_start) / length).floor();
        let into = beats - meter.bar_start - bars * length;
        let beat = (into * f64::from(meter.denominator) / 4.0).floor() as i32;
        Some((meter.bar + bars as i32 + 1, beat + 1))
    }

    /// Every bar line in `from..to`: its song seconds and its bar number,
    /// from 1. Each anchor rules from where it starts to where the next does;
    /// the first rules back to `from` as well.
    pub fn bar_lines(&self, from: f64, to: f64) -> Vec<(f64, i32)> {
        let mut lines = Vec::new();
        let meters = &self.meters;
        for (i, &(at, meter)) in meters.iter().enumerate() {
            let start = if i == 0 { from } else { at.max(from) };
            let end = meters.get(i + 1).map_or(to, |(next, _)| next.min(to));
            if start >= end {
                continue;
            }
            let length = meter.bar_length();
            let first = ((meter.beats_after(start - at) - meter.bar_start) / length).ceil() as i64;
            let last = ((meter.beats_after(end - at) - meter.bar_start) / length).ceil() as i64;
            for k in first..last {
                let seconds = at + meter.seconds_to(meter.bar_start + k as f64 * length);
                lines.push((seconds, meter.bar + k as i32 + 1));
            }
        }
        lines
    }

    /// Where recording coverage ends, in song seconds.
    pub fn end(&self) -> Option<f64> {
        self.spans.last().map(|span| span.end)
    }

    /// The span holding `seconds`.
    pub fn span_at(&self, seconds: f64) -> Option<&TapeSpan> {
        let after = self.spans.partition_point(|span| span.start <= seconds);
        self.spans.get(after.checked_sub(1)?).filter(|span| span.end > seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 4/4 at 120: a bar every two seconds.
    fn common(at: f64, bar: i32) -> (f64, Meter) {
        let beats = at * 2.0;
        let meter = Meter {
            beats,
            tempo: 120.0,
            bar_start: f64::from(bar) * 4.0,
            bar,
            numerator: 4,
            denominator: 4,
        };
        (at, meter)
    }

    #[test]
    fn bars_follow_each_anchor_and_a_tempo_change_moves_the_lines_after_it() {
        let mut view =
            TapeView { meters: vec![common(0.0, 0), common(1.0, 0)], ..Default::default() };
        assert_eq!(view.bar_at(0.0), Some((1, 1)));
        assert_eq!(view.bar_at(2.6), Some((2, 2)));
        assert_eq!(view.bar_lines(0.5, 6.5), [(2.0, 2), (4.0, 3), (6.0, 4)]);

        // From bar 3 (4 s) the song doubles its tempo: a bar a second.
        let (_, mut fast) = common(4.0, 2);
        fast.tempo = 240.0;
        view.meters.push((4.0, fast));
        assert_eq!(view.bar_lines(0.5, 6.5), [(2.0, 2), (4.0, 3), (5.0, 4), (6.0, 5)]);
        assert_eq!(view.bar_at(5.5), Some((4, 3)));
    }

    #[test]
    fn a_three_eight_bar_counts_eighths_and_no_meter_draws_no_bars() {
        // 3/8 at 120 quarters a minute: a bar is 1.5 quarters, 0.75 s.
        let meter = Meter {
            beats: 0.0,
            tempo: 120.0,
            bar_start: 0.0,
            bar: 0,
            numerator: 3,
            denominator: 8,
        };
        let view = TapeView { meters: vec![(0.0, meter)], ..Default::default() };
        assert_eq!(view.bar_lines(0.1, 2.0), [(0.75, 2), (1.5, 3)]);
        assert_eq!(view.bar_at(0.5), Some((1, 3)));
        assert_eq!(TapeView::default().bar_at(1.0), None);
        assert!(TapeView::default().bar_lines(0.0, 10.0).is_empty());
    }
}
