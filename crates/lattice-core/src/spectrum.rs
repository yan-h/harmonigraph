//! Pitch-class spectrum analysis: fold an audio signal's FFT onto the
//! 0..1200-cent axis the Spectral pane draws.
//!
//! Everything here is pure sample-in, buckets-out logic — no threads, no
//! clocks, no allocation after construction — so the shells can feed it
//! from wherever their audio comes from (the plugin's input bus, the
//! standalone's mock synth) and the whole pipeline stays unit-testable.
//! The FFT is a hand-rolled iterative radix-2 (the crate deliberately has
//! no dependencies); at 8192 points a few times per second it is nowhere
//! near a bottleneck.

/// Pitch-class resolution of the folded spectrum: 5-cent buckets.
pub const PC_BINS: usize = 240;

/// Analysis window length in samples (~0.17 s at 48 kHz — steady enough
/// for a meter, short enough to follow chord changes).
const FFT_SIZE: usize = 8192;

/// Fold range. Below ~55 Hz a bin is wider than a semitone and the fold
/// smears; above 5 kHz there is little tonal energy and much cymbal.
const FOLD_MIN_HZ: f32 = 55.0;
const FOLD_MAX_HZ: f32 = 5_000.0;

/// 12-TET C relative to A440 (261.6256 Hz): the 0-cent reference of the
/// pitch-class axis, matching how MIDI notes map with a zero C-offset.
const C_REF_HZ: f32 = 261.625_58;

/// Rolling analyzer: push mono samples as they arrive, ask for the folded
/// spectrum whenever the display wants a fresh frame.
pub struct SpectrumAnalyzer {
    sample_rate: f32,
    /// The most recent FFT_SIZE samples, as a circular buffer.
    ring: Vec<f32>,
    write: usize,
    /// Samples pushed since (re)configuration, saturating at FFT_SIZE.
    filled: usize,
    /// Hann window, precomputed.
    window: Vec<f32>,
    /// FFT scratch (allocated once).
    re: Vec<f32>,
    im: Vec<f32>,
}

impl SpectrumAnalyzer {
    pub fn new(sample_rate: f32) -> Self {
        let window = (0..FFT_SIZE)
            .map(|i| {
                let phase = std::f32::consts::TAU * i as f32 / FFT_SIZE as f32;
                0.5 * (1.0 - phase.cos())
            })
            .collect();
        SpectrumAnalyzer {
            sample_rate: sample_rate.max(1.0),
            ring: vec![0.0; FFT_SIZE],
            write: 0,
            filled: 0,
            window,
            re: vec![0.0; FFT_SIZE],
            im: vec![0.0; FFT_SIZE],
        }
    }

    /// Change the sample rate (host renegotiation). A change empties the
    /// buffer: mixing samples from two clocks would smear every peak.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        let sample_rate = sample_rate.max(1.0);
        if (sample_rate - self.sample_rate).abs() > f32::EPSILON {
            self.sample_rate = sample_rate;
            self.ring.fill(0.0);
            self.write = 0;
            self.filled = 0;
        }
    }

    /// Append mono samples (most recent last). Any chunk size works; only
    /// the trailing FFT_SIZE samples are kept.
    pub fn push_samples(&mut self, samples: &[f32]) {
        for &s in samples {
            self.ring[self.write] = s;
            self.write = (self.write + 1) % FFT_SIZE;
        }
        self.filled = (self.filled + samples.len()).min(FFT_SIZE);
    }

    /// The current pitch-class power spectrum, or None until a full
    /// window has been seen.
    ///
    /// Bucket values are absolute power: a full-scale sine contributes
    /// ~1.0 to its pitch class regardless of window position or sample
    /// rate, so successive frames are comparable and the display can
    /// apply a fixed mapping. Only local spectral peaks are folded — each
    /// deposits its main lobe's power at its parabolically refined
    /// frequency, split linearly between the two nearest buckets on the
    /// circular cent axis. (Folding every bin instead would smear each
    /// note across the +/-40 cents its skirt bins land on: at C4 one FFT
    /// bin is ~38 cents wide.)
    pub fn pitch_class_spectrum(&mut self) -> Option<[f32; PC_BINS]> {
        if self.filled < FFT_SIZE {
            return None;
        }

        // Unroll the ring into time order, windowed.
        for i in 0..FFT_SIZE {
            let src = (self.write + i) % FFT_SIZE;
            self.re[i] = self.ring[src] * self.window[i];
            self.im[i] = 0.0;
        }
        fft_in_place(&mut self.re, &mut self.im);

        // Amplitude normalization so a unit sine reads as ~1.0: |X| for a
        // real sine of amplitude A is A * sum(window) / 2.
        let window_sum: f32 = self.window.iter().sum();
        let norm = 2.0 / window_sum;

        let bin_hz = self.sample_rate / FFT_SIZE as f32;
        let lo = (FOLD_MIN_HZ / bin_hz).ceil() as usize;
        let hi = ((FOLD_MAX_HZ / bin_hz) as usize).min(FFT_SIZE / 2 - 2);

        let mag = |k: usize| (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt();

        let mut buckets = [0.0f32; PC_BINS];
        for k in lo..=hi {
            let m = mag(k);
            let (prev, next) = (mag(k - 1), mag(k + 1));
            // Peaks only; `>=` on one side so an exactly-between-bins tone
            // (two equal center bins) still registers once.
            if !(m > prev && m >= next) || m * norm < 1e-4 {
                continue;
            }
            // Parabolic refinement on log magnitude: sub-bin pitch from
            // the peak and its two neighbors.
            let mut bin = k as f32;
            if prev > 0.0 && next > 0.0 {
                let (a, b, c) = (prev.ln(), m.ln(), next.ln());
                let denom = a - 2.0 * b + c;
                if denom.abs() > f32::EPSILON {
                    bin += (0.5 * (a - c) / denom).clamp(-0.5, 0.5);
                }
            }
            let freq = bin * bin_hz;
            let cents = (1200.0 * (freq / C_REF_HZ).log2()).rem_euclid(1200.0);

            // Linear split across the two nearest 5-cent buckets (the axis
            // is circular: 1199.9 cents neighbors bucket 0).
            let pos = cents / (1200.0 / PC_BINS as f32);
            let base = pos.floor();
            let frac = pos - base;
            let b0 = (base as usize) % PC_BINS;
            let b1 = (b0 + 1) % PC_BINS;
            // The whole main lobe's power, not just the center bin's, so
            // the reading stays level as a tone drifts between bins.
            let power = (prev * prev + m * m + next * next) * norm * norm;
            buckets[b0] += power * (1.0 - frac);
            buckets[b1] += power * frac;
        }
        Some(buckets)
    }
}

/// Iterative radix-2 Cooley-Tukey, in place. Lengths are compile-time
/// powers of two here; debug_assert documents the requirement.
fn fft_in_place(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);

    // Bit-reversal permutation.
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    let mut len = 2;
    while len <= n {
        let step = std::f32::consts::TAU / len as f32;
        let half = len / 2;
        for start in (0..n).step_by(len) {
            for k in 0..half {
                // Recomputing sin/cos per butterfly is fine at this size
                // and call rate; a twiddle table would only add state.
                let angle = -step * k as f32;
                let (ws, wc) = angle.sin_cos();
                let (i, j) = (start + k, start + k + half);
                let (tr, ti) = (re[j] * wc - im[j] * ws, re[j] * ws + im[j] * wc);
                re[j] = re[i] - tr;
                im[j] = im[i] - ti;
                re[i] += tr;
                im[i] += ti;
            }
        }
        len *= 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed a sum of sines and return the folded spectrum.
    fn analyze(freqs_amps: &[(f32, f32)], sample_rate: f32) -> [f32; PC_BINS] {
        let mut analyzer = SpectrumAnalyzer::new(sample_rate);
        // Push in awkward chunk sizes to exercise the ring seam.
        let samples: Vec<f32> = (0..FFT_SIZE + 1234)
            .map(|i| {
                let t = i as f32 / sample_rate;
                freqs_amps
                    .iter()
                    .map(|(f, a)| a * (std::f32::consts::TAU * f * t).sin())
                    .sum()
            })
            .collect();
        for chunk in samples.chunks(701) {
            analyzer.push_samples(chunk);
        }
        analyzer.pitch_class_spectrum().expect("window filled")
    }

    fn peak_bucket(buckets: &[f32; PC_BINS]) -> usize {
        (0..PC_BINS).max_by(|&a, &b| buckets[a].total_cmp(&buckets[b])).unwrap()
    }

    fn bucket_of_cents(cents: f32) -> usize {
        ((cents / 5.0).round() as usize) % PC_BINS
    }

    /// Circular distance in buckets.
    fn dist(a: usize, b: usize) -> usize {
        let d = a.abs_diff(b);
        d.min(PC_BINS - d)
    }

    #[test]
    fn empty_analyzer_reports_nothing() {
        let mut analyzer = SpectrumAnalyzer::new(48_000.0);
        assert!(analyzer.pitch_class_spectrum().is_none());
        analyzer.push_samples(&vec![0.1; FFT_SIZE - 1]);
        assert!(analyzer.pitch_class_spectrum().is_none(), "one short of a window");
        analyzer.push_samples(&[0.1]);
        assert!(analyzer.pitch_class_spectrum().is_some());
    }

    #[test]
    fn a440_folds_to_900_cents() {
        // A above C: 1200*log2(440/261.6256) = 900 cents exactly in 12-TET.
        let buckets = analyze(&[(440.0, 0.8)], 48_000.0);
        let peak = peak_bucket(&buckets);
        assert!(
            dist(peak, bucket_of_cents(900.0)) <= 1,
            "peak at bucket {peak} ({}c), expected ~900c",
            peak * 5
        );
        // Absolute calibration: amplitude 0.8 -> power ~0.64 at the peak
        // (split across at most two buckets, windowing spreads a little).
        let total: f32 = buckets.iter().sum();
        assert!(
            (0.3..=1.0).contains(&total),
            "sine power should land near 0.64, got total {total}"
        );
    }

    #[test]
    fn dyad_shows_both_pitch_classes() {
        // 12-TET C4 + E4: 0 and 400 cents.
        let buckets = analyze(&[(261.6256, 0.5), (329.6276, 0.5)], 48_000.0);
        let c = bucket_of_cents(0.0);
        let e = bucket_of_cents(400.0);
        let floor = buckets.iter().sum::<f32>() / PC_BINS as f32;
        let near = |target: usize| {
            (0..PC_BINS)
                .filter(|&b| dist(b, target) <= 1)
                .map(|b| buckets[b])
                .fold(0.0f32, f32::max)
        };
        assert!(near(c) > floor * 20.0, "C peak missing");
        assert!(near(e) > floor * 20.0, "E peak missing");
        // And nothing comparable elsewhere (e.g. no image at 400+600c).
        let stray = (0..PC_BINS)
            .filter(|&b| dist(b, c) > 3 && dist(b, e) > 3)
            .map(|b| buckets[b])
            .fold(0.0f32, f32::max);
        assert!(stray < near(c) * 0.2, "stray energy {stray}");
    }

    #[test]
    fn octaves_fold_to_the_same_bucket() {
        // A3 + A4 + A5 all land on 900 cents.
        let buckets = analyze(&[(220.0, 0.4), (440.0, 0.4), (880.0, 0.4)], 48_000.0);
        let peak = peak_bucket(&buckets);
        assert!(dist(peak, bucket_of_cents(900.0)) <= 1);
        // The folded peak carries all three notes' power.
        let neighborhood: f32 = (0..PC_BINS)
            .filter(|&b| dist(b, peak) <= 1)
            .map(|b| buckets[b])
            .sum();
        assert!(neighborhood > 0.3, "expected stacked octave power, got {neighborhood}");
    }

    #[test]
    fn sample_rate_change_resets_the_window() {
        let mut analyzer = SpectrumAnalyzer::new(48_000.0);
        analyzer.push_samples(&vec![0.2; FFT_SIZE]);
        assert!(analyzer.pitch_class_spectrum().is_some());
        analyzer.set_sample_rate(44_100.0);
        assert!(
            analyzer.pitch_class_spectrum().is_none(),
            "stale samples must not be analyzed under a new clock"
        );
    }

    #[test]
    fn fft_matches_a_naive_dft_on_a_small_case() {
        let n = 16;
        let signal: Vec<f32> =
            (0..n).map(|i| (i as f32 * 0.7).sin() + 0.3 * (i as f32 * 2.1).cos()).collect();
        let mut re = signal.clone();
        let mut im = vec![0.0f32; n];
        fft_in_place(&mut re, &mut im);
        for k in 0..n {
            let (mut dr, mut di) = (0.0f64, 0.0f64);
            for (i, &s) in signal.iter().enumerate() {
                let angle = -std::f64::consts::TAU * (k * i) as f64 / n as f64;
                dr += f64::from(s) * angle.cos();
                di += f64::from(s) * angle.sin();
            }
            assert!(
                (re[k] as f64 - dr).abs() < 1e-3 && (im[k] as f64 - di).abs() < 1e-3,
                "bin {k}: fft ({}, {}) vs dft ({dr}, {di})",
                re[k],
                im[k]
            );
        }
    }
}
