//! The Sweep estimator: one window whose length glides with pitch (#1560).
//!
//! A fixed window cannot suit both ends of a pitch axis. Low partials sit a few
//! hertz apart and need a long listen to be told apart; high ones are far apart
//! and want a short one so onsets stay sharp. Sweep gives every bucket its own
//! length, gliding log-linearly from [`Sweep::bass`] at 20 Hz to
//! [`Sweep::treble`] at 20 kHz, with no bands and so no seams between them.
//!
//! **It costs one transform, not one per length.** Shortening a window is
//! multiplying the frame by a narrower envelope, and multiplying in time is
//! convolving in frequency. So each column takes the ordinary transform of the
//! whole frame through the configured tapers, and each bucket then reads it
//! through its own small kernel: the frame times a Gaussian of spread `sigma`
//! is the frame's transform convolved with that Gaussian's transform, which is
//! another Gaussian, `fft_size / (2 pi sigma)` bins wide. A wide Gaussian in
//! time is a few taps in frequency, so the bass costs almost nothing and the
//! treble a few dozen taps per bucket. This is Brown and Puckette's
//! frequency-domain constant-Q kernel, with the Q set by the two lengths rather
//! than fixed.
//!
//! Each kernel is centred on the bucket's own frequency rather than on a bin, so
//! a partial reads at its own pitch wherever it sits between bins. The single
//! window's reconstruction (`reconstruct`) keeps a peak on the bin it was
//! measured at, which at 4096 samples puts a 55 Hz A1 more than a semitone sharp.
//!
//! The frame is centred on the same instant at every pitch, so the column is
//! time-aligned across the axis and belongs half the FRAME before its newest
//! sample, exactly as a single window of `fft_size` does. The bass's longer
//! listen therefore costs no misalignment, only the frame's own latency.

use std::f64::consts::PI;

use harmonigraph_core::spectrum::SPECTRUM_BINS;
use realfft::num_complex::Complex32;

use crate::{usable_bins, BUCKET_FREQUENCIES};

/// The listen lengths at the two ends of the pitch axis, each in samples of an
/// equivalent Hann window: what a single Hann window that long would spread its
/// energy over in time. So `4096` at either end is the Fast setting's listen
/// there, whatever the frame is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sweep {
    /// At 20 Hz, the bottom of the axis.
    pub bass: f32,
    /// At 20 kHz, the top.
    pub treble: f32,
}

/// The shortest listen either end accepts, in samples.
pub const SWEEP_MIN_LENGTH: f32 = 256.0;

/// The spread of a periodic Hann window of length `n`, as a fraction of `n`:
/// `sqrt(1/12 - 1/(2 pi^2))`. A Gaussian of this many samples' spread listens
/// over the same span of time as the Hann window, which is what lets a length
/// in samples name both.
const HANN_SPREAD: f64 = 0.180_773_6;

/// The narrowest kernel, in bins: a Gaussian sampled at whole bins reads a
/// frequency BETWEEN them faithfully only while it is at least this wide. It is
/// what bounds the longest listen a frame can give — see [`max_length`].
const MIN_KERNEL_STD: f64 = 0.6;

/// How far out each kernel is kept, in its own standard deviations. The tail
/// beyond carries `exp(-8)` of the peak, under a ten-thousandth of the weight.
const KERNEL_REACH: f64 = 4.0;

/// The longest listen, in Hann-equivalent samples, that a frame of `fft_size`
/// can give: the frame's own Hann envelope times the widest Gaussian whose
/// kernel is still [`MIN_KERNEL_STD`] bins wide. About 0.83 of the frame, so a
/// 16384-sample frame reaches about 13,500 samples in the bass.
pub fn max_length(fft_size: usize) -> f32 {
    let n = fft_size as f64;
    let widest = n / (2.0 * PI * MIN_KERNEL_STD);
    (effective_spread(widest, n) / HANN_SPREAD) as f32
}

/// The spread of the frame's Hann envelope times a Gaussian of spread `sigma`:
/// spreads of two Gaussian-like envelopes combine as their reciprocal squares.
fn effective_spread(sigma: f64, n: f64) -> f64 {
    let frame = HANN_SPREAD * n;
    (1.0 / (1.0 / (sigma * sigma) + 1.0 / (frame * frame))).sqrt()
}

/// One frequency a bucket is read at: `taps` sign-folded weights from
/// `weights[at..]`, applied to bins `k0..k0 + taps`.
#[derive(Clone, Copy, Debug)]
struct Point {
    k0: u32,
    taps: u32,
    at: u32,
}

/// Which kernels each bucket reads, for one sweep, frame length, sample rate and
/// taper set. Rebuilt by [`plan`](Self::plan) whenever any of those change, and
/// read unchanged by every column between.
pub(crate) struct SweepPlan {
    /// Per bucket, the range of [`points`](Self::points) it reads; empty is a
    /// bucket outside the usable bins, which reads as nothing.
    spans: Box<[(u32, u32); SPECTRUM_BINS]>,
    points: Vec<Point>,
    weights: Vec<f32>,
    /// Per bucket, the scale that puts a full-scale sine at 1.0 through that
    /// bucket's own window — the counterpart of `taper_norm_power`, which a
    /// shortened window needs bucket by bucket.
    norm: Box<[f32; SPECTRUM_BINS]>,
}

impl SweepPlan {
    pub(crate) fn new() -> Self {
        SweepPlan {
            spans: Box::new([(0, 0); SPECTRUM_BINS]),
            points: Vec::new(),
            weights: Vec::new(),
            norm: Box::new([0.0; SPECTRUM_BINS]),
        }
    }

    /// Settle every bucket's kernels. `tapers` is the analyzer's flat taper
    /// table, `taper_count` of them each `fft_size` long.
    pub(crate) fn plan(
        &mut self,
        sweep: Sweep,
        sample_rate: f32,
        fft_size: usize,
        tapers: &[f32],
        taper_count: usize,
    ) {
        let n = fft_size as f64;
        let sr = f64::from(sample_rate);
        let (first, last) = usable_bins(fft_size);
        let longest = f64::from(max_length(fft_size));
        let length = |l: f32| f64::from(l).clamp(f64::from(SWEEP_MIN_LENGTH), longest);
        let (bass, treble) = (length(sweep.bass).ln(), length(sweep.treble).ln());
        let span = (20_000.0f64 / 20.0).ln();
        let frame = HANN_SPREAD * n;
        self.points.clear();
        self.weights.clear();
        for (b, frequencies) in BUCKET_FREQUENCIES.iter().enumerate() {
            let centre = f64::from(frequencies.center_hz);
            let u = ((centre / 20.0).ln() / span).clamp(0.0, 1.0);
            // The listen this bucket asks for, as a spread, and the Gaussian
            // that brings the frame's own envelope down to it — never wider
            // than the narrowest kernel allows, which `longest` already folds in.
            let tau = (bass + (treble - bass) * u).exp() * HANN_SPREAD;
            let inverse = (1.0 / (tau * tau) - 1.0 / (frame * frame)).max(0.0);
            let widest = n / (2.0 * PI * MIN_KERNEL_STD);
            let sigma = if inverse > 0.0 { (1.0 / inverse.sqrt()).min(widest) } else { widest };
            let kernel_std = n / (2.0 * PI * sigma);

            // Read at more than one frequency where the bucket is wider than the
            // response is: one reading at the centre would dim a partial near
            // the bucket's edge, by up to 3 dB at the top of the axis.
            let response_hz = sr / (2.0 * PI * effective_spread(sigma, n));
            let (lower, upper) = (f64::from(frequencies.lower_hz), f64::from(frequencies.upper_hz));
            let readings = ((upper - lower) / response_hz).ceil().max(1.0) as usize;
            let start = self.points.len() as u32;
            for r in 0..readings {
                let hz = lower + (upper - lower) * (r as f64 + 0.5) / readings as f64;
                let at = hz * n / sr;
                if at < first as f64 || at > last as f64 {
                    continue;
                }
                let reach = KERNEL_REACH * kernel_std;
                let k0 = ((at - reach).ceil() as usize).max(first);
                let k1 = ((at + reach).floor() as usize).min(last);
                // Unit sum over the WHOLE Gaussian, not the kept taps, so a
                // kernel clipped at the edge of the usable bins is not lifted.
                let scale = 1.0 / (kernel_std * (2.0 * PI).sqrt());
                let offset = self.weights.len() as u32;
                for k in k0..=k1 {
                    let d = (at - k as f64) / kernel_std;
                    // The frame's time origin is its first sample and the
                    // envelope's is its middle; (-1)^k moves the one to the
                    // other, so the kernel stays real.
                    let sign = if k % 2 == 0 { 1.0 } else { -1.0 };
                    self.weights.push((sign * scale * (-0.5 * d * d).exp()) as f32);
                }
                self.points.push(Point { k0: k0 as u32, taps: (k1 + 1 - k0) as u32, at: offset });
            }
            self.spans[b] = (start, self.points.len() as u32);
            self.norm[b] = norm_power(tapers, fft_size, taper_count, sigma);
        }
    }

    /// Fill `out` from the frame's transforms, `taper_count` of them laid end to
    /// end, `bins` each. A bucket read at several frequencies takes the loudest,
    /// so its level does not grow with how many readings it took.
    pub(crate) fn read(
        &self,
        spectra: &[Complex32],
        bins: usize,
        taper_count: usize,
        out: &mut [f32; SPECTRUM_BINS],
    ) {
        for ((out, &(p0, p1)), norm) in out.iter_mut().zip(self.spans.iter()).zip(self.norm.iter())
        {
            let mut loudest = 0.0f32;
            for point in &self.points[p0 as usize..p1 as usize] {
                let weights = &self.weights[point.at as usize..(point.at + point.taps) as usize];
                let mut power = 0.0f32;
                for t in 0..taper_count {
                    let from = t * bins + point.k0 as usize;
                    let bins = &spectra[from..from + weights.len()];
                    let mut acc = Complex32::new(0.0, 0.0);
                    for (x, &w) in bins.iter().zip(weights) {
                        acc += x * w;
                    }
                    power += acc.norm_sqr();
                }
                loudest = loudest.max(power);
            }
            *out = loudest * norm;
        }
    }
}

/// The scale that puts a full-scale sine at 1.0 through the tapers each shaped
/// by a Gaussian of spread `sigma` about the frame's middle — the reasoning of
/// `taper_norm_power`, with each taper's sum taken under the Gaussian.
///
/// The sum is taken every `STRIDE` samples. It runs once per bucket per plan,
/// which is every slider tick, and a strided sum of a smooth integrand that
/// vanishes at both ends is spectrally accurate: its error is the Gaussian's
/// transform at the stride's own frequency, `exp(-2 pi^2 sigma^2 / STRIDE^2)`,
/// which at the shortest listen allowed ([`SWEEP_MIN_LENGTH`], a spread of 46
/// samples) is under `exp(-160)`.
fn norm_power(tapers: &[f32], n: usize, taper_count: usize, sigma: f64) -> f32 {
    const STRIDE: usize = 16;
    let centre = n / 2;
    // The Gaussian at whole multiples of the stride out from the middle, by
    // recurrence rather than an `exp` per sample: each step's ratio is the last
    // one's times a constant.
    let s = STRIDE as f64;
    let q = (-s * s / (sigma * sigma)).exp();
    let mut response = 0.0f64;
    for taper in tapers.chunks_exact(n).take(taper_count) {
        let mut sum = f64::from(taper[centre]);
        let (mut g, mut ratio) = (1.0f64, (-s * s / (2.0 * sigma * sigma)).exp());
        let mut d = STRIDE;
        while d <= centre {
            g *= ratio;
            ratio *= q;
            let below = f64::from(taper[centre - d]);
            let above = if centre + d < n { f64::from(taper[centre + d]) } else { 0.0 };
            sum += g * (below + above);
            d += STRIDE;
        }
        sum *= s;
        response += sum * sum;
    }
    (4.0 / response) as f32
}

#[cfg(test)]
mod tests {
    use harmonigraph_core::spectrum::{hz_to_midi, BINS_PER_SEMITONE, SPECTRUM_MIN_MIDI};

    use super::*;
    use crate::{AnalyzerConfig, SpectrumAnalyzer};

    const RATE: f32 = 48_000.0;
    const FRAME: usize = 16_384;
    /// The fresh sweep: as long as the frame allows at the bottom, and half of
    /// Fast at the top, which is Fast's listen by 2 kHz.
    const FRESH: Sweep = Sweep { bass: 13_500.0, treble: 2048.0 };

    fn config(sweep: Sweep, tapers: usize) -> AnalyzerConfig {
        AnalyzerConfig { sample_rate: RATE, fft_size: FRAME, tapers, sweep: Some(sweep) }
    }

    fn analyze(config: AnalyzerConfig, hz: f32) -> [f32; SPECTRUM_BINS] {
        let mut analyzer = SpectrumAnalyzer::new(config);
        let samples: Vec<f32> = (0..config.fft_size + 1234)
            .map(|i| (std::f32::consts::TAU * hz * i as f32 / RATE).sin())
            .collect();
        for chunk in samples.chunks(701) {
            analyzer.push_samples(chunk);
        }
        *analyzer.pitch_spectrum().expect("window filled")
    }

    fn peak(buckets: &[f32; SPECTRUM_BINS]) -> usize {
        (0..SPECTRUM_BINS).max_by(|&a, &b| buckets[a].total_cmp(&buckets[b])).unwrap()
    }

    fn cents_off(bucket: usize, hz: f32) -> f32 {
        let midi = SPECTRUM_MIN_MIDI + (bucket as f32 + 0.5) / BINS_PER_SEMITONE as f32;
        100.0 * (midi - hz_to_midi(hz))
    }

    /// Half-power width of the peak, in Hz.
    fn width_hz(buckets: &[f32; SPECTRUM_BINS]) -> f32 {
        let top = buckets[peak(buckets)];
        let lit: Vec<_> = (0..SPECTRUM_BINS).filter(|&b| buckets[b] >= 0.5 * top).collect();
        let edges = &BUCKET_FREQUENCIES;
        edges[*lit.last().unwrap()].upper_hz - edges[lit[0]].lower_hz
    }

    /// Each kernel is centred on its bucket's own frequency, so a low partial
    /// lands at its own pitch. A single window's reconstruction keeps a peak on
    /// the bin it was measured at: at 4096 samples that is 58.6 Hz for this
    /// 55 Hz A1, more than a semitone sharp.
    #[test]
    fn a_low_partial_reads_at_its_own_pitch() {
        for hz in [41.2, 55.0, 61.7, 98.0] {
            let off = cents_off(peak(&analyze(config(FRESH, 1), hz)), hz);
            assert!(off.abs() < 5.0, "{hz} Hz reads {off:+.1} cents off");
        }
    }

    /// The contract every absolute dB rests on, held bucket by bucket: each
    /// bucket's own window is normalized, and a bucket wider than its response
    /// is read at several frequencies so a partial near its edge is not dimmed.
    #[test]
    fn a_full_scale_sine_reads_unity_across_the_axis() {
        // A partial on the boundary between two of the widest buckets, the one
        // place a single reading per bucket would be furthest from it.
        let top = &BUCKET_FREQUENCIES[SPECTRUM_BINS - 40];
        for tapers in [1, 3] {
            for hz in [30.0, 110.0, 440.0, 1999.0, 7531.0, 15_000.0, top.upper_hz] {
                let level = analyze(config(FRESH, tapers), hz);
                let db = 10.0 * level[peak(&level)].log10();
                assert!((-1.2..0.2).contains(&db), "{hz} Hz, {tapers} tapers: {db:+.2} dB");
            }
        }
    }

    /// The lengths are Hann-equivalent: a sweep's peak at either end is as wide
    /// as a single Hann window of the length the sweep asks for there, which is
    /// what lets the sliders be read against Fast, Balanced and Precise.
    #[test]
    fn the_listen_at_each_end_matches_a_hann_window_of_that_length() {
        let sweep = Sweep { bass: 12_000.0, treble: 1024.0 };
        for hz in [24.0f32, 17_000.0] {
            let u = (hz / 20.0).ln() / 1000.0f32.ln();
            let length = (sweep.bass.ln() + (sweep.treble.ln() - sweep.bass.ln()) * u).exp();
            // Hann's half-power width is 1.44 bins of its own length.
            let hann = 1.44 * RATE / length;
            let width = width_hz(&analyze(config(sweep, 1), hz));
            assert!(
                (width / hann - 1.0).abs() < 0.2,
                "{hz} Hz: {width:.2} Hz wide, a {length:.0}-sample Hann is {hann:.2}"
            );
        }
    }

    /// A sweep's lengths decide only the kernels, so moving them keeps the audio
    /// already in the frame: a slider drag keeps drawing. Leaving the sweep
    /// changes how the frame is read and nothing about the frame either.
    #[test]
    fn changing_the_sweep_keeps_the_window() {
        // A treble partial, where the two sweeps' listens differ by an octave,
        // so a reading through the old kernels cannot pass for the new one.
        let audio: Vec<f32> =
            (0..FRAME).map(|i| (std::f32::consts::TAU * 3000.0 * i as f32 / RATE).sin()).collect();
        let read = |analyzer: &mut SpectrumAnalyzer| *analyzer.pitch_spectrum().expect("a window");
        let other = Sweep { bass: 8000.0, treble: 4096.0 };
        let mut analyzer = SpectrumAnalyzer::new(config(FRESH, 1));
        analyzer.push_samples(&audio);
        let before = read(&mut analyzer);
        analyzer.configure(config(other, 1));
        let after = read(&mut analyzer);
        let mut fresh = SpectrumAnalyzer::new(config(other, 1));
        fresh.push_samples(&audio);
        assert_eq!(after, read(&mut fresh), "the window was kept but read through the old sweep");
        assert_ne!(after, before, "the fixture does not tell the two sweeps apart");
        analyzer.configure(AnalyzerConfig { sweep: None, ..config(FRESH, 1) });
        assert!(analyzer.pitch_spectrum().is_some(), "leaving the sweep dropped the window");
    }
}
