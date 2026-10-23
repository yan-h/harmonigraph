//! How every pitch bucket reads the frame's transform: at its own frequency,
//! through its own listen.
//!
//! **One transform, and a small kernel per bucket.** Shortening a window is
//! multiplying the frame by a narrower envelope, and multiplying in time is
//! convolving in frequency. So each column takes the ordinary transform of the
//! whole frame through its Hann window, and each bucket then reads it through
//! its own kernel: the frame times a Gaussian of spread `sigma` is the frame's
//! transform convolved with that Gaussian's transform, which is another
//! Gaussian, `PAD * fft_size / (2 pi sigma)` bins wide. A wide Gaussian in time
//! is a few taps in frequency, so the bass costs almost nothing and the treble
//! a few dozen taps per bucket. This is Brown and Puckette's frequency-domain
//! constant-Q kernel, with the Q set by the listen rather than fixed.
//!
//! **Each kernel is centred on the bucket's own frequency rather than on a
//! bin**, so a partial reads at its own pitch wherever it sits between bins.
//! That is the whole reason every window reads this way, and not only a
//! [`Sweep`]: the resample of the bins it replaced kept every peak on the bin it
//! was measured at, which through the old 4096-sample Fast frame drew a 55 Hz A1
//! more than a semitone sharp (#1575).
//!
//! **Where and how long each bucket listens** is the one thing the settings
//! decide. A fixed window listens through one Gaussian, the same at every
//! pitch, set toward the frame's newest end ([`FIXED_SPREAD`],
//! [`FIXED_CENTRE`]). A [`Sweep`] listens about the frame's middle, gliding
//! log-linearly from [`Sweep::bass`] at 20 Hz to [`Sweep::treble`] at 20 kHz,
//! with no bands and so no seams between them. A fixed window cannot suit both
//! ends of a pitch axis: low partials sit a few hertz apart and need a long
//! listen to be told apart, high ones are far apart and want a short one so
//! onsets stay sharp.
//!
//! Either way every bucket listens about the same instant, so the column is
//! time-aligned across the axis and belongs where its window's weight is
//! centred ([`lag_samples`]). A longer listen in the bass under a Sweep
//! therefore costs no misalignment, only the frame's own latency.

use std::f64::consts::PI;

use harmonigraph_core::spectrum::SPECTRUM_BINS;
use realfft::num_complex::Complex32;

use crate::BUCKET_FREQUENCIES;

/// The listen lengths at the two ends of the pitch axis, each in samples of an
/// equivalent Hann window: what a single Hann window that long would spread its
/// energy over in time. So `4096` at either end listens as sharply as Fast,
/// whose frame is longer but whose listen is a 4096-sample Hann's.
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

/// The fixed windows' listen: a Gaussian of this spread, as a fraction of the
/// frame, centred at [`FIXED_CENTRE`], under the frame's Hann window.
///
/// The pair is chosen so that a fixed window looks and lags as the plain Hann
/// window of two thirds of its frame did, which is what Fast, Balanced and
/// Precise were before every bucket read at its own pitch: a peak as wide at
/// half power as that Hann window's, and the column's lag within one percent
/// of it (43.0, 86.1 and 172.2 ms at 48 kHz, where the old 4096, 8192 and
/// 16,384-sample Hann windows lagged 42.7, 85.3 and 170.7). A partial reads
/// within 3.4 cents of its pitch from 22 to 25 Hz through the Fast frame and
/// within half a cent above 50 Hz, its level within a quarter of a dB, and a
/// note's release tail is about 15% longer to -40 dB (38 ms where it was 33
/// through Fast), because the Gaussian tapers more gently than a Hann window.
///
/// What it is chosen against is FALSE PEAKS. A listen centred on the frame and
/// as long as the frame allows reads the frame's own Hann sidelobes, which a
/// reading at every bucket's own frequency now samples between the nulls: a
/// full-scale partial drew bumps at -42.5 dB a few semitones either side. The
/// old bin readings hid the same sidelobes only by landing on the nulls.
/// Shortening the centred listen to 0.75 of the frame cleaned them and widened
/// every peak by 28%; doubling the frame instead doubles the live lag. Put
/// toward the newest end, the Gaussian has died away long before the frame's
/// oldest sample, and the Hann window's own end is the only edge the product
/// has, so its spectrum falls away from the peak without lobes.
pub(crate) const FIXED_SPREAD: f64 = 0.1729;

/// Where the fixed listen's Gaussian is centred, as a fraction of the frame from
/// its oldest sample: toward the newest end, which is what keeps the lag of a
/// frame half as long again as the old one's. See [`FIXED_SPREAD`].
pub(crate) const FIXED_CENTRE: f64 = 0.8;

/// How many frames long the transform is: the frame and a frame of zeros.
///
/// A kernel sampled at whole bins is the transform of the frame times a
/// Gaussian REPEATED every transform length, so a frame that filled the whole
/// transform would be read through the neighbouring copies' tails as well. That
/// is not a level error but a PITCH one, and it grows as the listen approaches
/// the frame: on an unpadded transform the longest listen read a partial up to
/// 0.06 of a bin off. Padding moves the copies a whole frame further from the
/// listen's centre. Measured against known tones, twice is enough (under 0.2
/// cents above 50 Hz at the longest Sweep listen); four times buys nothing more.
///
/// The frame is placed so that the listen's centre is the transform's middle
/// ([`Kernels::offset`]), which keeps the copies equally far either side: for
/// the fixed listen the nearest one is 1.2 frames from the frame's oldest
/// sample, seven of its spreads, where it weighs `3e-11` of its peak.
pub(crate) const PAD: usize = 2;

/// The narrowest kernel, in bins of the padded transform, and what bounds the
/// longest listen a Sweep can ask for ([`max_length`]): a narrower kernel is a
/// wider Gaussian in time, whose repeated copies (see [`PAD`]) reach back into
/// the frame. At this width the longest listen is 0.89 of the frame and reads
/// within 0.2 cents; at 0.75 of a bin it is 0.92 of the frame and errs by 0.7.
/// The fixed listen's kernel is 1.84 bins wide, well clear of it.
const MIN_KERNEL_STD: f64 = 0.9;

/// How far out each kernel is kept, in its own standard deviations.
///
/// A kernel cut short is a reading through a window with ripples on it, and
/// its error is not a level but a false PEAK beside a loud low partial: at 4,
/// a full-scale 110 Hz tone through the Fast frame drew one at 72.8 Hz, -52 dB,
/// and full-scale 55 to 147 Hz tones drew one or two each through every fixed
/// frame. At 5 the loudest local maximum more than a semitone from any of them
/// is -124 dB, the transform's own single-precision floor, through all three;
/// 6 measured the same and costs a fifth more taps.
const KERNEL_REACH: f64 = 5.0;

/// How far inside the usable bins a reading's centre has to sit, in its
/// kernel's standard deviations, to be read at all.
///
/// A kernel clipped by the edge of the usable bins reads a lopsided slice of the
/// spectrum, and its peak walks away from the partial — at 1.7 of these, a
/// 20.6 Hz partial through a 4096-sample frame read 48 cents off. From 2 the
/// error stays within about 13 cents (3.4 through the Fast frame, whose fixed
/// listen is shorter than that frame's longest), which is what this keeps; nearer the edge the
/// bucket reads as nothing, the truth about a frame too short to place it. At
/// 48 kHz that is 20 to 22 Hz through the Fast frame, and 20 to 44 Hz through
/// it at 96 kHz.
const KERNEL_CLEARANCE: f64 = 2.0;

/// The longest listen, in Hann-equivalent samples, that a Sweep on a frame of
/// `fft_size` can ask for: the frame's own Hann envelope times the widest
/// Gaussian whose kernel is still [`MIN_KERNEL_STD`] bins wide. 0.89 of the
/// frame, so the 16384-sample Sweep frame reaches about 14,600 samples.
pub fn max_length(fft_size: usize) -> f32 {
    let n = fft_size as f64;
    (effective_spread(widest(n), n) / HANN_SPREAD) as f32
}

/// The widest Gaussian, as a spread in samples, whose kernel on the padded
/// transform of a frame of `n` is still [`MIN_KERNEL_STD`] bins wide.
fn widest(n: f64) -> f64 {
    PAD as f64 * n / (2.0 * PI * MIN_KERNEL_STD)
}

/// The spread of the frame's Hann envelope times a Gaussian of spread `sigma`
/// centred on it: spreads of two Gaussian-like envelopes combine as their
/// reciprocal squares.
fn effective_spread(sigma: f64, n: f64) -> f64 {
    let frame = HANN_SPREAD * n;
    (1.0 / (1.0 / (sigma * sigma) + 1.0 / (frame * frame))).sqrt()
}

/// The spread of the Gaussian a bucket centred at `hz` listens through under
/// `sweep`, on a frame of `fft_size`: the one that brings the frame's own Hann
/// envelope down to the length the sweep asks for there, never wider than the
/// narrowest kernel allows, which [`max_length`] already folds in.
fn swept_sigma(sweep: Sweep, fft_size: usize, hz: f64) -> f64 {
    let n = fft_size as f64;
    let longest = f64::from(max_length(fft_size));
    let length = |l: f32| f64::from(l).clamp(f64::from(SWEEP_MIN_LENGTH), longest).ln();
    let u = ((hz / 20.0).ln() / (20_000.0f64 / 20.0).ln()).clamp(0.0, 1.0);
    let (bass, treble) = (length(sweep.bass), length(sweep.treble));
    let tau = (bass + (treble - bass) * u).exp() * HANN_SPREAD;
    let frame = HANN_SPREAD * n;
    let inverse = (1.0 / (tau * tau) - 1.0 / (frame * frame)).max(0.0);
    let widest = widest(n);
    if inverse > 0.0 {
        (1.0 / inverse.sqrt()).min(widest)
    } else {
        widest
    }
}

/// The sample of a frame of `fft_size` that every bucket's listen is centred
/// on: the middle under a Sweep, and [`FIXED_CENTRE`] of the way to the newest
/// sample for a fixed window.
pub(crate) fn listen_centre(fft_size: usize, sweep: bool) -> usize {
    if sweep {
        fft_size / 2
    } else {
        (FIXED_CENTRE * fft_size as f64).round() as usize
    }
}

/// How many samples before the newest one a column taken from a frame of
/// `fft_size` belongs on a time axis: where its window's weight is centred,
/// the amplitude centroid of the Hann window times the listen's Gaussian.
///
/// Under a Sweep every bucket's Gaussian is centred on the frame's middle, as
/// the Hann window is, so the centroid is the middle by symmetry whatever each
/// bucket's spread. The fixed listen sits toward the newest end and the Hann
/// window pulls its centroid back toward the middle a little, which only the
/// sum says by how much.
pub(crate) fn lag_samples(fft_size: usize, sweep: bool) -> f64 {
    let centre = listen_centre(fft_size, sweep) as f64;
    let centroid = if sweep { centre } else { centre + fixed_moments(fft_size).1 };
    (fft_size - 1) as f64 - centroid
}

/// The fixed listen through a frame of `fft_size`, as the length of the plain
/// Hann window with its spread in time: two thirds of the frame, to a fifth of a
/// percent, by the choice of [`FIXED_SPREAD`] and [`FIXED_CENTRE`].
///
/// Read off the window rather than restated, so that what the settings call
/// each fixed resolution follows the pair if it is ever retuned.
pub fn fixed_listen(fft_size: usize) -> f32 {
    (fixed_moments(fft_size).2 / HANN_SPREAD) as f32
}

/// The fixed listen's window — the frame's Hann window under the fixed
/// Gaussian — summed once for everything that asks about it: its sum, where its
/// weight is centred relative to the listen's centre, and its spread about that.
fn fixed_moments(fft_size: usize) -> (f64, f64, f64) {
    let centre = listen_centre(fft_size, false);
    let hann = |i: usize| f64::from(crate::hann_weight(i, fft_size));
    let [sum, first, second] =
        listen_moments(fft_size, centre, FIXED_SPREAD * fft_size as f64, hann);
    let mean = first / sum;
    (sum, mean, (second / sum - mean * mean).sqrt())
}

/// The bins of the padded transform of a frame of `fft_size` a kernel may read,
/// inclusive: from the frame's own first bin above DC to its last below Nyquist.
///
/// Bin 1 is usable because the frame's mean is removed before the transform
/// (`pitch_spectrum`), so DC leaves nothing there to read; reading it is what
/// put a 22 Hz partial within 13 cents through a 4096-sample frame, where
/// stopping a bin higher put it 3.5 semitones off.
fn usable_bins(fft_size: usize) -> (usize, usize) {
    (PAD, PAD * (fft_size / 2 - 1))
}

/// One frequency a bucket is read at: `taps` sign-folded weights from
/// `weights[at..]`, applied to bins `k0..k0 + taps`.
#[derive(Clone, Copy, Debug)]
struct Point {
    k0: u32,
    taps: u32,
    at: u32,
}

/// Which kernels each bucket reads, for one listen, frame length and sample
/// rate. Rebuilt by [`plan`](Self::plan) whenever any of those change, and read
/// unchanged by every column between.
pub(crate) struct Kernels {
    /// Per bucket, the range of [`points`](Self::points) it reads; empty is a
    /// bucket the frame cannot place, which reads as nothing.
    spans: Box<[(u32, u32); SPECTRUM_BINS]>,
    points: Vec<Point>,
    weights: Vec<f32>,
    /// Per bucket, the scale that puts a full-scale sine at 1.0 through that
    /// bucket's own window — see [`norm_power`].
    norm: Box<[f32; SPECTRUM_BINS]>,
    /// Where the frame starts in the padded transform — see
    /// [`offset`](Self::offset).
    offset: usize,
}

impl Kernels {
    pub(crate) fn new() -> Self {
        Kernels {
            spans: Box::new([(0, 0); SPECTRUM_BINS]),
            points: Vec::new(),
            weights: Vec::new(),
            norm: Box::new([0.0; SPECTRUM_BINS]),
            offset: 0,
        }
    }

    /// Where the frame's oldest sample goes in the padded transform: wherever
    /// puts the listen's centre at the transform's middle.
    ///
    /// The transform's time origin is its first sample. Each kernel reads the
    /// frame through a Gaussian centred on the transform's middle, because a
    /// shift of half the transform is a factor of `(-1)^k` on bin `k`, which
    /// is real: the weights stay real and a tap is two multiplies rather than
    /// a complex product's four. Moving the frame rather than the Gaussian is
    /// what keeps that true for a listen off the frame's middle.
    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    /// Settle every bucket's kernels, listening along `sweep` or, without one,
    /// through the fixed listen. `window` is the analyzer's Hann window,
    /// `fft_size` long.
    pub(crate) fn plan(
        &mut self,
        sweep: Option<Sweep>,
        sample_rate: f32,
        fft_size: usize,
        window: &[f32],
    ) {
        let n = fft_size as f64;
        // The padded transform's length, in which its bins are counted.
        let bins_n = (PAD * fft_size) as f64;
        let sr = f64::from(sample_rate);
        let (first, last) = usable_bins(fft_size);
        let centre = listen_centre(fft_size, sweep.is_some());
        self.offset = PAD * fft_size / 2 - centre;
        let window = |i: usize| f64::from(window[i]);
        // One listen at every pitch without a sweep, so its moments are summed
        // once: the Gaussian's spread, the spread of the window it makes with
        // the Hann window, and the scale that window owes.
        let fixed = sweep.is_none().then(|| {
            let (sum, _, spread) = fixed_moments(fft_size);
            (FIXED_SPREAD * n, spread, norm_power(sum))
        });
        self.points.clear();
        self.weights.clear();
        for (b, frequencies) in BUCKET_FREQUENCIES.iter().enumerate() {
            let (sigma, spread, norm) = match (sweep, fixed) {
                (Some(sweep), _) => {
                    let sigma = swept_sigma(sweep, fft_size, f64::from(frequencies.center_hz));
                    let [sum, _, _] = listen_moments(fft_size, centre, sigma, window);
                    (sigma, effective_spread(sigma, n), norm_power(sum))
                }
                (None, fixed) => fixed.expect("summed without a sweep"),
            };
            let kernel_std = bins_n / (2.0 * PI * sigma);

            // Read at more than one frequency where the bucket is wider than the
            // response is, no further apart than half the response's spread:
            // one reading at the centre would dim a partial near the bucket's
            // edge by up to 3 dB at the top of the axis, and readings a whole
            // spread apart still dimmed one between them by 1.1 dB. At half a
            // spread the dip is 0.27 dB.
            let response_hz = sr / (2.0 * PI * spread);
            let (lower, upper) = (f64::from(frequencies.lower_hz), f64::from(frequencies.upper_hz));
            let readings = (2.0 * (upper - lower) / response_hz).ceil().max(1.0) as usize;
            let clearance = KERNEL_CLEARANCE * kernel_std;
            let reach = KERNEL_REACH * kernel_std;
            // Unit sum over the WHOLE Gaussian, not the kept taps, so a kernel
            // clipped at the edge of the usable bins is not lifted.
            let scale = 1.0 / (kernel_std * (2.0 * PI).sqrt());
            let start = self.points.len() as u32;
            for r in 0..readings {
                let hz = lower + (upper - lower) * (r as f64 + 0.5) / readings as f64;
                let at = hz * bins_n / sr;
                if at < first as f64 + clearance || at > last as f64 - clearance {
                    continue;
                }
                let k0 = ((at - reach).ceil() as usize).max(first);
                let k1 = ((at + reach).floor() as usize).min(last);
                let offset = self.weights.len() as u32;
                // The listen is centred on the transform's middle (see
                // `offset`), and (-1)^k moves the transform's time origin there.
                self.weights.extend((k0..=k1).map(|k| {
                    let d = (at - k as f64) / kernel_std;
                    let g = scale * (-0.5 * d * d).exp();
                    (if k % 2 == 0 { g } else { -g }) as f32
                }));
                self.points.push(Point { k0: k0 as u32, taps: (k1 + 1 - k0) as u32, at: offset });
            }
            self.spans[b] = (start, self.points.len() as u32);
            self.norm[b] = norm;
        }
    }

    /// Fill `out` from the frame's padded transform. A bucket read at several
    /// frequencies takes the loudest, so its level does not grow with how many
    /// readings it took, and 0 dB keeps meaning a full-scale sine.
    pub(crate) fn read(&self, spectrum: &[Complex32], out: &mut [f32; SPECTRUM_BINS]) {
        const LANES: usize = 4;
        for ((out, &(p0, p1)), norm) in out.iter_mut().zip(self.spans.iter()).zip(self.norm.iter())
        {
            let mut loudest = 0.0f32;
            for point in &self.points[p0 as usize..p1 as usize] {
                let weights = &self.weights[point.at as usize..(point.at + point.taps) as usize];
                let bins = &spectrum[point.k0 as usize..point.k0 as usize + weights.len()];
                // Four running sums a side rather than one, so the taps are
                // summed four at a time in vector lanes: one sum is a chain
                // of dependent adds the compiler may not reorder. Reading the
                // kernels is most of a column's cost.
                let (mut re, mut im) = ([0.0f32; LANES], [0.0f32; LANES]);
                let (bins, bins_rest) = bins.as_chunks::<LANES>();
                let (weights, weights_rest) = weights.as_chunks::<LANES>();
                for (x, w) in bins.iter().zip(weights) {
                    for lane in 0..LANES {
                        re[lane] += x[lane].re * w[lane];
                        im[lane] += x[lane].im * w[lane];
                    }
                }
                for (lane, (x, &w)) in bins_rest.iter().zip(weights_rest).enumerate() {
                    re[lane] += x.re * w;
                    im[lane] += x.im * w;
                }
                let (re, im) = (re.iter().sum::<f32>(), im.iter().sum::<f32>());
                loudest = loudest.max(re * re + im * im);
            }
            *out = loudest * norm;
        }
    }
}

/// The scale that puts a full-scale sine at 1.0 through a window whose samples
/// sum to `sum`, as a power.
///
/// A real sine of amplitude `A` transforms, through a window `w`, to
/// `|X| = A * sum(w) / 2` at its own frequency, so `(2/sum(w))^2` returns `A^2`;
/// here `w` is the Hann window under the bucket's Gaussian. That is the
/// contract the rest of the plugin rests on — 0 dB is a full-scale sine at
/// every pitch, which is what makes the Level window's ends absolute dB, what
/// the tilt pivots against, and what makes the audio ring's Gate a fixed
/// position rather than a drifting one. The padding adds nothing to the sum,
/// being zeros.
fn norm_power(sum: f64) -> f32 {
    (4.0 / (sum * sum)) as f32
}

/// The window `w` of a frame of `n` under a Gaussian of spread `sigma` about
/// sample `centre`: its sum, and its first and second moments about `centre`,
/// which say where the window's weight sits and how widely it spreads.
///
/// The sums are taken every `STRIDE` samples. They run once per bucket per
/// plan under a Sweep, which is every slider tick, and a strided sum of a
/// smooth integrand that vanishes at both ends is spectrally accurate: its
/// error is the Gaussian's transform at the stride's own frequency,
/// `exp(-2 pi^2 sigma^2 / STRIDE^2)`, which at the shortest listen allowed
/// ([`SWEEP_MIN_LENGTH`], a spread of 46 samples) is under `exp(-160)`. The
/// moments' integrands are the same one times a polynomial, as smooth.
fn listen_moments(n: usize, centre: usize, sigma: f64, w: impl Fn(usize) -> f64) -> [f64; 3] {
    const STRIDE: usize = 16;
    // The Gaussian at whole multiples of the stride out from the centre, by
    // recurrence rather than an `exp` per sample: each step's ratio is the last
    // one's times a constant.
    let s = STRIDE as f64;
    let q = (-s * s / (sigma * sigma)).exp();
    let mut sum = w(centre);
    let (mut first, mut second) = (0.0f64, 0.0f64);
    let (mut g, mut ratio) = (1.0f64, (-s * s / (2.0 * sigma * sigma)).exp());
    let mut d = STRIDE;
    while d <= centre.max(n - 1 - centre) {
        g *= ratio;
        ratio *= q;
        let below = if d <= centre { w(centre - d) } else { 0.0 };
        let above = if centre + d < n { w(centre + d) } else { 0.0 };
        let x = d as f64;
        sum += g * (below + above);
        first += g * x * (above - below);
        second += g * x * x * (below + above);
        d += STRIDE;
    }
    [sum * s, first * s, second * s]
}

#[cfg(test)]
mod tests {
    use harmonigraph_core::spectrum::{hz_to_midi, BINS_PER_SEMITONE, SPECTRUM_MIN_MIDI};

    use super::*;
    use crate::{AnalyzerConfig, SpectrumAnalyzer};

    const RATE: f32 = 48_000.0;
    const FRAME: usize = 16_384;
    /// The fresh sweep: the look picked in #1560 at the bottom, a little under
    /// the 14,600 samples the frame allows there, and half of Fast at the top.
    const FRESH: Sweep = Sweep { bass: 13_500.0, treble: 2048.0 };

    fn config(sweep: Sweep) -> AnalyzerConfig {
        AnalyzerConfig { sample_rate: RATE, fft_size: FRAME, sweep: Some(sweep) }
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
    /// lands at its own pitch under a sweep, where every bucket's listen is a
    /// different length, as it does through a fixed one
    /// (`a_partial_reads_at_its_own_pitch_through_every_frame`).
    #[test]
    fn a_low_partial_reads_at_its_own_pitch() {
        for hz in [41.2, 55.0, 61.7, 98.0] {
            let off = cents_off(peak(&analyze(config(FRESH), hz)), hz);
            assert!(off.abs() < 5.0, "{hz} Hz reads {off:+.1} cents off");
        }
    }

    /// The contract every absolute dB rests on, held bucket by bucket: each
    /// bucket's own window is normalized, and a bucket wider than its response
    /// is read at several frequencies, half the response's spread apart, so a
    /// partial between two readings is dimmed by a quarter of a dB at most.
    #[test]
    fn a_full_scale_sine_reads_unity_across_the_axis() {
        // A partial on the boundary between two of the widest buckets, the one
        // place a single reading per bucket would be furthest from it.
        let top = &BUCKET_FREQUENCIES[SPECTRUM_BINS - 40];
        // The sweep, and the fixed listen through the shortest and longest of
        // the fixed frames: the fixed listen is off the frame's middle, so its
        // window's sum is a different one.
        for (frame, listen) in [(FRAME, Some(FRESH)), (6144, None), (24_576, None)] {
            let config = AnalyzerConfig { sample_rate: RATE, fft_size: frame, sweep: listen };
            for hz in [30.0, 110.0, 440.0, 1999.0, 7531.0, 15_000.0, top.upper_hz] {
                let level = analyze(config, hz);
                let db = 10.0 * level[peak(&level)].log10();
                assert!(
                    (-0.35..0.1).contains(&db),
                    "{hz} Hz through {frame}, {listen:?}: {db:+.2} dB"
                );
            }
        }
    }

    /// A loud low partial draws no false one beside it ([`KERNEL_REACH`]).
    ///
    /// A kernel cut off too near its centre reads through a window with
    /// ripples on it, which draws a partial that is not there a few semitones
    /// from a loud one: at four spreads, -52 dB beside a full-scale 110 Hz tone
    /// through the Fast frame. A local maximum is what the eye reads as a
    /// partial, so that is what this looks for, more than a semitone from the
    /// tone and above -90 dB, through every fixed frame.
    #[test]
    fn a_loud_partial_raises_no_false_peak() {
        for n in [6144, 12_288, 24_576] {
            let config = AnalyzerConfig { sample_rate: RATE, fft_size: n, sweep: None };
            for hz in [55.0f32, 82.41, 110.0, 146.83, 261.63] {
                let level = analyze(config, hz);
                let (at, db) = (1..SPECTRUM_BINS - 1)
                    .filter(|&b| level[b] > level[b - 1] && level[b] >= level[b + 1])
                    .filter(|&b| cents_off(b, hz).abs() > 100.0)
                    .map(|b| (BUCKET_FREQUENCIES[b].center_hz, 10.0 * level[b].log10()))
                    .fold((0.0, f32::NEG_INFINITY), |a, b| if b.1 > a.1 { b } else { a });
                assert!(
                    db < -90.0,
                    "{n}-sample frame, {hz} Hz: a false peak at {at:.1} Hz, {db:+.1} dB"
                );
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
            let width = width_hz(&analyze(config(sweep), hz));
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
        let mut analyzer = SpectrumAnalyzer::new(config(FRESH));
        analyzer.push_samples(&audio);
        let before = read(&mut analyzer);
        analyzer.configure(config(other));
        let after = read(&mut analyzer);
        let mut fresh = SpectrumAnalyzer::new(config(other));
        fresh.push_samples(&audio);
        assert_eq!(after, read(&mut fresh), "the window was kept but read through the old sweep");
        assert_ne!(after, before, "the fixture does not tell the two sweeps apart");
        // Leaving the sweep moves the listen's centre and so the frame's place in
        // the transform: a stale offset would still read SOMETHING, so compare.
        let fixed = AnalyzerConfig { sweep: None, ..config(FRESH) };
        analyzer.configure(fixed);
        let left = read(&mut analyzer);
        let mut fresh = SpectrumAnalyzer::new(fixed);
        fresh.push_samples(&audio);
        assert_eq!(left, read(&mut fresh), "leaving the sweep kept the window but not its read");
    }

    /// The fixed listen is as long as two thirds of its frame, the plain Hann
    /// window each fixed resolution was before #1575 and what the settings name
    /// it by, so a retuned [`FIXED_SPREAD`] or [`FIXED_CENTRE`] that changed
    /// the look fails here rather than leaving the settings describing the
    /// old one.
    #[test]
    fn the_fixed_listen_is_two_thirds_of_its_frame() {
        for n in [6144, 12_288, 24_576] {
            let ratio = fixed_listen(n) / (n as f32 * 2.0 / 3.0);
            assert!((ratio - 1.0).abs() < 0.01, "{n}-sample frame: {ratio:.4} of two thirds");
        }
    }

    /// The shipped analyzer, ring wrap and padding included, against an
    /// independent f64 transform: each bucket read through its kernel is the
    /// frame, mean removed, through the Hann window times that bucket's own
    /// Gaussian, evaluated directly at the bucket's own frequency.
    ///
    /// That equality is the whole of what the kernels claim, and the two sides
    /// share nothing but the window and the normalization: one convolves the
    /// padded FFT, the other sums the frame in time. The fixed frames put the
    /// Gaussian off the frame's middle, which only the frame's placement in the
    /// transform keeps real; the sweep's bass end is its widest Gaussian, the
    /// case the padding exists for.
    #[test]
    fn a_kernel_reads_the_frame_through_its_buckets_own_window() {
        for (n, sweep) in [(6144, None), (12_288, None), (24_576, None), (FRAME, Some(FRESH))] {
            let mut a =
                SpectrumAnalyzer::new(AnalyzerConfig { sample_rate: RATE, fft_size: n, sweep });
            let input: Vec<_> = (0..n + 997)
                .map(|i| {
                    let t = i as f64 / f64::from(RATE);
                    ((std::f64::consts::TAU * 440.0 * t).sin() * 0.4
                        + (std::f64::consts::TAU * 7137.0 * t).cos() * 0.2
                        + 0.05
                        + if i % 113 == 0 { 0.3 } else { 0.0 }) as f32
                })
                .collect();
            a.push_samples(&input);
            let buckets = *a.pitch_spectrum().expect("a full window plus wrap");
            assert_eq!(a.write, 997);
            let frame: Vec<f64> = (0..n).map(|i| f64::from(a.ring[(a.write + i) % n])).collect();
            let window: Vec<f64> = a.window.iter().map(|&w| f64::from(w)).collect();
            let mean = frame.iter().zip(&window).map(|(s, w)| s * w).sum::<f64>()
                / window.iter().sum::<f64>();
            let centre = listen_centre(n, sweep.is_some()) as f64;
            let loudest = buckets.iter().copied().fold(0.0f32, f32::max) as f64;
            let mut checked = 0;
            for b in (0..SPECTRUM_BINS).step_by(53) {
                // Buckets read at one frequency, which is then the middle of
                // the bucket; the top of the axis is read at several.
                let (p0, p1) = a.kernels.spans[b];
                if p1 != p0 + 1 {
                    continue;
                }
                let f = &BUCKET_FREQUENCIES[b];
                let hz = 0.5 * f64::from(f.lower_hz + f.upper_hz);
                let sigma = sweep.map_or(FIXED_SPREAD * n as f64, |sweep| {
                    swept_sigma(sweep, n, f64::from(f.center_hz))
                });
                let (mut re, mut im) = (0.0, 0.0);
                for i in 0..n {
                    let d = (i as f64 - centre) / sigma;
                    let x = (frame[i] - mean) * window[i] * (-0.5 * d * d).exp();
                    let angle = std::f64::consts::TAU * hz * i as f64 / f64::from(RATE);
                    re += x * angle.cos();
                    im -= x * angle.sin();
                }
                let expected = (re * re + im * im) * f64::from(a.kernels.norm[b]);
                let error = (f64::from(buckets[b]) - expected).abs() / loudest;
                assert!(
                    error < 1e-4,
                    "N={n}, {sweep:?}, bucket {b} at {hz:.1} Hz: scaled error {error:e}"
                );
                checked += 1;
            }
            assert!(checked > 20, "N={n}, {sweep:?}: only {checked} buckets were compared");
        }
    }
}
