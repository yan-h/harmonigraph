//! Pitch spectrum analysis: map an audio signal's FFT onto the absolute
//! log-frequency (MIDI pitch) axis the Spectral pane draws, so every
//! partial displays at its actual pitch.
//!
//! Everything here is pure sample-in, buckets-out logic — no threads, no
//! clocks, no allocation after construction — so the shells can feed it
//! from wherever their audio comes from (the plugin's input bus, the
//! standalone's mock synth) and the whole pipeline stays unit-testable.
//! The FFT is a hand-rolled iterative radix-2 (the crate deliberately has
//! no dependencies); at 8192 points a few times per second it is nowhere
//! near a bottleneck.

/// The spectrum's pitch axis: MIDI notes [MIN, MAX), which is 20 Hz to
/// 20 kHz — the audible band, as every analyzer states it. The axis is linear
/// in MIDI pitch, i.e. logarithmic in frequency, so every octave gets equal
/// width.
///
/// Deliberately NOT whole octaves from a C. It used to be MIDI 12..132, ten
/// octaves C to C, which made the C gridlines land on the axis ends — tidy,
/// but it stopped at 16.7 kHz and left the top third of an octave of the
/// audible band unanalyzed. There is no C anywhere near 20 kHz (the next one
/// is 44 kHz), so covering the band means giving that tidiness up.
pub const SPECTRUM_MIN_MIDI: f32 = 15.486_82; // 20 Hz
pub const SPECTRUM_MAX_MIDI: f32 = 135.076_23; // 20 kHz
/// Axis resolution: 32 buckets per semitone (3.125 cents).
///
/// This is what sets how sharply a partial can be drawn, and it is the only
/// thing that does. The analyzer is peak-based: it finds each local maximum,
/// refines its position parabolically to well under an FFT bin, then splits
/// its power across the two nearest buckets — so a partial is always two
/// buckets wide and never wider. At the old 8 per semitone that was a 25-cent
/// floor, which reads as a fat blob the moment you zoom the pitch range in to
/// an octave or two. At 32 it is 6.25 cents.
pub const BINS_PER_SEMITONE: usize = 32;
/// Enough buckets to cover the axis, plus slack: the span is not a whole
/// number of semitones, and `pitch_spectrum` writes to `b0 + 1`, so the top
/// partial needs a bucket above the one it lands in or it would be dropped.
pub const SPECTRUM_BINS: usize =
    ((SPECTRUM_MAX_MIDI - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32) as usize + 2;

/// Normalized magnitude below which a spectral peak is treated as noise
/// and skipped entirely.
const PEAK_FLOOR: f32 = 1e-4;

/// Frequency of a (fractional) MIDI pitch at A440.
pub fn midi_to_hz(midi: f32) -> f32 {
    440.0 * ((midi - 69.0) / 12.0).exp2()
}

/// The inverse of [`midi_to_hz`]: the (fractional) MIDI pitch of `hz`.
pub fn hz_to_midi(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

/// Default analysis window length in samples (~0.17 s at 48 kHz — steady
/// enough for a meter, short enough to follow chord changes). At the axis
/// floor (~16 Hz) one FFT bin spans several semitones, so the lowest
/// octave reads coarse; that is inherent to the window length, not a bug.
/// [`SpectrumAnalyzer::set_fft_size`] trades response time against bass
/// precision at runtime.
pub const DEFAULT_FFT_SIZE: usize = 8192;

/// Rolling analyzer: push mono samples as they arrive, ask for the
/// spectrum whenever the display wants a fresh frame.
pub struct SpectrumAnalyzer {
    sample_rate: f32,
    fft_size: usize,
    /// The most recent `fft_size` samples, as a circular buffer.
    ring: Vec<f32>,
    write: usize,
    /// Samples pushed since (re)configuration, saturating at `fft_size`.
    filled: usize,
    /// Hann window, precomputed.
    window: Vec<f32>,
    /// FFT scratch (allocated per configuration).
    re: Vec<f32>,
    im: Vec<f32>,
}

impl SpectrumAnalyzer {
    pub fn new(sample_rate: f32) -> Self {
        let mut analyzer = SpectrumAnalyzer {
            sample_rate: sample_rate.max(1.0),
            fft_size: 0,
            ring: Vec::new(),
            write: 0,
            filled: 0,
            window: Vec::new(),
            re: Vec::new(),
            im: Vec::new(),
        };
        analyzer.configure(DEFAULT_FFT_SIZE);
        analyzer
    }

    /// (Re)allocate every buffer for `fft_size` and clear the window.
    fn configure(&mut self, fft_size: usize) {
        assert!(fft_size.is_power_of_two(), "radix-2 FFT needs a power of two");
        self.fft_size = fft_size;
        self.ring = vec![0.0; fft_size];
        self.write = 0;
        self.filled = 0;
        self.window = (0..fft_size)
            .map(|i| {
                let phase = std::f32::consts::TAU * i as f32 / fft_size as f32;
                0.5 * (1.0 - phase.cos())
            })
            .collect();
        self.re = vec![0.0; fft_size];
        self.im = vec![0.0; fft_size];
    }

    /// Change the analysis window length (a power of two): longer =
    /// sharper bass, slower response. A change empties the buffer.
    /// No-op at the current size, so calling every frame is fine.
    pub fn set_fft_size(&mut self, fft_size: usize) {
        if fft_size != self.fft_size {
            self.configure(fft_size);
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
    /// the trailing `fft_size` samples are kept.
    pub fn push_samples(&mut self, samples: &[f32]) {
        for &s in samples {
            self.ring[self.write] = s;
            self.write = (self.write + 1) % self.fft_size;
        }
        self.filled = (self.filled + samples.len()).min(self.fft_size);
    }

    /// The current power spectrum over the MIDI-pitch axis, or None until
    /// a full window has been seen.
    ///
    /// Bucket values are absolute power: a full-scale sine contributes
    /// ~1.0 at its pitch regardless of window position or sample rate,
    /// so successive frames are comparable and the display can apply a
    /// fixed mapping. Only local spectral peaks are deposited — each
    /// contributes its main lobe's power at its parabolically refined
    /// frequency, split linearly between the two nearest buckets. (Using
    /// every bin instead would smear each note across the width its
    /// skirt bins span: at C4 one FFT bin is ~38 cents wide.)
    pub fn pitch_spectrum(&mut self) -> Option<[f32; SPECTRUM_BINS]> {
        if self.filled < self.fft_size {
            return None;
        }

        // Unroll the ring into time order, windowed.
        for i in 0..self.fft_size {
            let src = (self.write + i) % self.fft_size;
            self.re[i] = self.ring[src] * self.window[i];
            self.im[i] = 0.0;
        }
        fft_in_place(&mut self.re, &mut self.im);

        // Amplitude normalization so a unit sine reads as ~1.0: |X| for a
        // real sine of amplitude A is A * sum(window) / 2.
        let window_sum: f32 = self.window.iter().sum();
        let norm = 2.0 / window_sum;

        let bin_hz = self.sample_rate / self.fft_size as f32;
        // Analyze the overlap of the pitch axis and what the FFT resolves
        // (skip DC and the first bin; stay clear of Nyquist).
        let lo = (midi_to_hz(SPECTRUM_MIN_MIDI) / bin_hz).ceil().max(2.0) as usize;
        let hi = ((midi_to_hz(SPECTRUM_MAX_MIDI) / bin_hz) as usize).min(self.fft_size / 2 - 2);

        let mag = |k: usize| (self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt();

        let mut buckets = [0.0f32; SPECTRUM_BINS];
        for k in lo..=hi {
            let m = mag(k);
            let (prev, next) = (mag(k - 1), mag(k + 1));
            // Peaks only; `>=` on one side so an exactly-between-bins tone
            // (two equal center bins) still registers once.
            if !(m > prev && m >= next) || m * norm < PEAK_FLOOR {
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
            let midi = hz_to_midi(freq);

            // Linear split across the two nearest buckets; partials
            // outside the axis are dropped, not wrapped.
            let pos = (midi - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32;
            if !(0.0..(SPECTRUM_BINS - 1) as f32).contains(&pos) {
                continue;
            }
            let base = pos.floor();
            let frac = pos - base;
            let b0 = base as usize;
            // The whole main lobe's power, not just the center bin's, so
            // the reading stays level as a tone drifts between bins.
            let power = (prev * prev + m * m + next * next) * norm * norm;
            buckets[b0] += power * (1.0 - frac);
            buckets[b0 + 1] += power * frac;
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
    fn analyze(freqs_amps: &[(f32, f32)], sample_rate: f32) -> [f32; SPECTRUM_BINS] {
        let mut analyzer = SpectrumAnalyzer::new(sample_rate);
        // Push in awkward chunk sizes to exercise the ring seam.
        let samples: Vec<f32> = (0..DEFAULT_FFT_SIZE + 1234)
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
        analyzer.pitch_spectrum().expect("window filled")
    }

    fn peak_bucket(buckets: &[f32; SPECTRUM_BINS]) -> usize {
        (0..SPECTRUM_BINS).max_by(|&a, &b| buckets[a].total_cmp(&buckets[b])).unwrap()
    }

    fn bucket_of_midi(midi: f32) -> usize {
        ((midi - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32).round() as usize
    }

    fn dist(a: usize, b: usize) -> usize {
        a.abs_diff(b)
    }

    #[test]
    fn empty_analyzer_reports_nothing() {
        let mut analyzer = SpectrumAnalyzer::new(48_000.0);
        assert!(analyzer.pitch_spectrum().is_none());
        analyzer.push_samples(&vec![0.1; DEFAULT_FFT_SIZE - 1]);
        assert!(analyzer.pitch_spectrum().is_none(), "one short of a window");
        analyzer.push_samples(&[0.1]);
        assert!(analyzer.pitch_spectrum().is_some());
    }

    #[test]
    fn a440_lands_on_a4() {
        let buckets = analyze(&[(440.0, 0.8)], 48_000.0);
        let peak = peak_bucket(&buckets);
        assert!(
            dist(peak, bucket_of_midi(69.0)) <= 1,
            "peak at bucket {peak}, expected A4 (bucket {})",
            bucket_of_midi(69.0)
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
    fn dyad_shows_both_pitches() {
        // 12-TET C4 + E4 (MIDI 60 and 64).
        let buckets = analyze(&[(261.6256, 0.5), (329.6276, 0.5)], 48_000.0);
        let c = bucket_of_midi(60.0);
        let e = bucket_of_midi(64.0);
        let floor = buckets.iter().sum::<f32>() / SPECTRUM_BINS as f32;
        let near = |target: usize| {
            (0..SPECTRUM_BINS)
                .filter(|&b| dist(b, target) <= 1)
                .map(|b| buckets[b])
                .fold(0.0f32, f32::max)
        };
        assert!(near(c) > floor * 20.0, "C4 peak missing");
        assert!(near(e) > floor * 20.0, "E4 peak missing");
        // And nothing comparable elsewhere.
        let stray = (0..SPECTRUM_BINS)
            .filter(|&b| dist(b, c) > 3 && dist(b, e) > 3)
            .map(|b| buckets[b])
            .fold(0.0f32, f32::max);
        assert!(stray < near(c) * 0.2, "stray energy {stray}");
    }

    #[test]
    fn octaves_show_as_separate_peaks() {
        // A3 + A4 + A5: distinct pitches a MIDI octave apart, no folding.
        let buckets = analyze(&[(220.0, 0.4), (440.0, 0.4), (880.0, 0.4)], 48_000.0);
        let floor = buckets.iter().sum::<f32>() / SPECTRUM_BINS as f32;
        for midi in [57.0, 69.0, 81.0] {
            let target = bucket_of_midi(midi);
            let level = (0..SPECTRUM_BINS)
                .filter(|&b| dist(b, target) <= 1)
                .map(|b| buckets[b])
                .fold(0.0f32, f32::max);
            assert!(level > floor * 20.0, "missing peak at MIDI {midi}");
        }
    }

    #[test]
    fn out_of_range_partials_are_dropped_not_wrapped() {
        // ~12.5 Hz sits below the axis floor (C-1 ~ 16.35 Hz); it must not
        // alias to some in-range bucket. (An inaudible test tone, but the
        // guard matters for subsonic rumble in real material.)
        let buckets = analyze(&[(12.5, 0.8)], 48_000.0);
        let total: f32 = buckets.iter().sum();
        assert!(total < 0.05, "sub-axis energy leaked in: {total}");
    }

    #[test]
    fn sample_rate_change_resets_the_window() {
        let mut analyzer = SpectrumAnalyzer::new(48_000.0);
        analyzer.push_samples(&vec![0.2; DEFAULT_FFT_SIZE]);
        assert!(analyzer.pitch_spectrum().is_some());
        analyzer.set_sample_rate(44_100.0);
        assert!(
            analyzer.pitch_spectrum().is_none(),
            "stale samples must not be analyzed under a new clock"
        );
    }

    #[test]
    fn set_fft_size_resets_the_window_and_noops_at_the_current_size() {
        let mut analyzer = SpectrumAnalyzer::new(48_000.0);
        analyzer.push_samples(&vec![0.2; DEFAULT_FFT_SIZE]);
        assert!(analyzer.pitch_spectrum().is_some());
        // A genuine size change empties the buffer.
        analyzer.set_fft_size(DEFAULT_FFT_SIZE * 2);
        assert!(analyzer.pitch_spectrum().is_none(), "resized window starts empty");
        // Refilling to the new length produces a spectrum again.
        analyzer.push_samples(&vec![0.2; DEFAULT_FFT_SIZE * 2]);
        assert!(analyzer.pitch_spectrum().is_some());
        // Setting the same size again is a no-op: the filled window survives.
        analyzer.set_fft_size(DEFAULT_FFT_SIZE * 2);
        assert!(analyzer.pitch_spectrum().is_some(), "no-op resize kept the window");
    }

    #[test]
    fn midi_to_hz_anchors_a440_and_doubles_each_octave() {
        assert!((midi_to_hz(69.0) - 440.0).abs() < 1e-2, "A4 = 440 Hz");
        assert!((midi_to_hz(57.0) - 220.0).abs() < 1e-2, "A3 = 220 Hz");
        assert!((midi_to_hz(81.0) - 880.0).abs() < 1e-2, "A5 = 880 Hz");
        assert!((midi_to_hz(60.0) - 261.6256).abs() < 0.1, "middle C ≈ 261.63 Hz");
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
