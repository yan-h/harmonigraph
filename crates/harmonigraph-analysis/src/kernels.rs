//! How every pitch bucket reads the frame's transform: at its own frequency,
//! through the one listen every bucket shares.
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
//! constant-Q kernel. The listen is the same at every pitch, so it is shared
//! between the two: part of its Gaussian multiplies the frame before the
//! transform, which narrows every kernel ([`FIXED_KERNEL_SPREAD`]).
//!
//! **Each kernel is centred on the bucket's own frequency rather than on a
//! bin**, so a partial reads at its own pitch wherever it sits between bins.
//! That is the whole reason the kernels read this way: the resample of the bins
//! it replaced kept every peak on the bin it was measured at, which through the
//! old 4096-sample Fast frame drew a 55 Hz A1 more than a semitone sharp
//! (#1575).
//!
//! **How long each bucket listens** is the one thing the settings decide: one
//! Gaussian, the same at every pitch, set toward the frame's newest end
//! ([`FIXED_SPREAD`], [`FIXED_CENTRE`]). Every bucket listens about the same
//! instant, so the column is time-aligned across the axis and belongs where its
//! window's weight is centred ([`lag_samples`]).
//!
//! A listen that glided with pitch, long in the bass and short in the treble,
//! was tried as a fourth setting and removed: at the same lag no window it
//! could glide through drew a bass as sharp as Precise's without false peaks,
//! and a sharper treble did not earn the mode (#1600).

use std::f64::consts::PI;

use harmonigraph_core::spectrum::SPECTRUM_BINS;
use realfft::num_complex::Complex32;

use crate::BUCKET_FREQUENCIES;

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
///
/// Not every placement toward the newest end is clean, though: holding the
/// spread and moving the Gaussian draws lobes in bands, -60 to -67 dB with its
/// centre 0.38 to 0.45 of the listen from the newest sample, against 0.3 here
/// (#1600). Moving it nearer the newest end is clean and cuts the lag by up to
/// 14%, at skirts up to 1.6 times as wide 60 dB down.
pub(crate) const FIXED_SPREAD: f64 = 0.1729;

/// Where the fixed listen's Gaussian is centred, as a fraction of the frame from
/// its oldest sample: toward the newest end, which is what keeps the lag of a
/// frame half as long again as the old one's. See [`FIXED_SPREAD`].
pub(crate) const FIXED_CENTRE: f64 = 0.8;

/// The share of the fixed listen the kernels read: a Gaussian of this spread,
/// as a fraction of the frame, with the rest of the listen multiplied into the
/// frame's window before its transform ([`Kernels::window`]).
///
/// A Gaussian of spread `s` is the product of two about the same centre whose
/// spreads `a` and `b` satisfy `1/s^2 = 1/a^2 + 1/b^2`, and the fixed listen is
/// one Gaussian at every pitch, so it can be split anywhere between the frame
/// and the kernels and every bucket still reads through all of it. The wider
/// the kernels' share in time, the narrower they are in bins and the fewer taps
/// they take: here 1.06 bins where the whole listen's were 1.84,
/// which is 42% fewer taps, a quarter off the reads and 12 to 16% off a column
/// through each fixed frame.
///
/// What bounds it is the kernels' repeated copies ([`PAD`]), which spread with
/// the kernels' share and which only the frame's window keeps off the frame. At
/// 0.3 the nearest copy under that window comes to at most `1.5e-7` of the
/// listen's peak, every bucket reads what the whole listen did through the
/// kernels within 0.01 dB, and pitch and false peaks measure as well as they
/// did (Fast's loudest false peak beside a full-scale tone fell from -86 dB to
/// -111, the kernels clipping less at the bottom of the axis). At
/// 0.35 a partial reads up to 0.03 cents further off anywhere above 25 Hz, at
/// 0.4 up to half a cent through Balanced, and at 0.45 three cents (from 100 to
/// 500 Hz alone: 0.018, 0.17 and a cent, which
/// `the_fixed_listens_kernels_leave_every_peak_on_its_pitch` holds).
pub(crate) const FIXED_KERNEL_SPREAD: f64 = 0.3;

/// The frame a fixed window reads to listen as sharply as a plain Hann window
/// `listen` samples long: half as long again, by the choice of [`FIXED_SPREAD`]
/// and [`FIXED_CENTRE`]. Each is written out as 3·2^k rather than computed from
/// the pair, so it stays a size the FFT transforms fast.
///
/// # Panics
///
/// For any listen but the fixed resolutions' three: 4096, 8192 and 16,384.
pub const fn fixed_frame(listen: usize) -> usize {
    match listen {
        4096 => 6144,
        8192 => 12_288,
        16_384 => 24_576,
        _ => panic!("no fixed frame gives that listen"),
    }
}

/// How many frames long the transform is: the frame and a frame of zeros.
///
/// A kernel sampled at whole bins is the transform of the frame times a
/// Gaussian REPEATED every transform length, so a frame that filled the whole
/// transform would be read through the neighbouring copies' tails as well:
/// not a level error but a PITCH one.
///
/// The frame is placed so that the listen's centre is the transform's middle
/// ([`Kernels::offset`]), which keeps the copies equally far either side. With
/// a frame of zeros the nearest copy is 1.2 frames from the frame's oldest
/// sample, and the frame's window, which carries the rest of the listen
/// ([`FIXED_KERNEL_SPREAD`]), brings it to at most `1.5e-7` of the listen's
/// peak anywhere in the frame. Unpadded, the copy would sit a frame behind the
/// listen's centre, nearer than that centre to the oldest 0.3 of the frame,
/// and bend the window there at about 1% of its peak. Four times buys nothing
/// more.
pub(crate) const PAD: usize = 2;

/// How far out each kernel is kept, in its own standard deviations.
///
/// A kernel cut short is a reading through a window with ripples on it, and
/// its error is not a level but a false PEAK beside a loud low partial: at 4,
/// a full-scale 110 Hz tone through the Fast frame drew one at 72.8 Hz, -52 dB,
/// and full-scale 55 to 147 Hz tones drew one or two each through every fixed
/// frame. At 5 the loudest local maximum more than a semitone from any of them
/// is -124 dB, the transform's own single-precision floor, through all three;
/// 6 measured the same and costs a fifth more taps. Those were measured with
/// the kernels reading the whole fixed listen. Since they read a narrower share
/// of it ([`FIXED_KERNEL_SPREAD`]), what a cut-off tail drops lies under the
/// listen's own wider skirt, so it can lower a level but not raise a peak:
/// across tones every 0.113 semitone the loudest false maximum through Fast is
/// -111 dB, and -122 through Balanced and Precise.
const KERNEL_REACH: f64 = 5.0;

/// How far inside the usable bins a reading's centre has to sit, in standard
/// deviations of its whole listen's kernel, to be read at all.
///
/// A kernel clipped by the edge of the usable bins reads a lopsided slice of the
/// spectrum, and its peak walks away from the partial. From 2 the error stays
/// within 3.4 cents through the Fast frame, which is what this keeps; nearer
/// the edge the bucket reads as nothing, the truth about a frame too short to
/// place it. At
/// 48 kHz that is 20 to 22 Hz through the Fast frame, and 20 to 44 Hz through
/// the same frame at 96 kHz, which is why the UI doubles the frame there.
///
/// The whole listen's width and not the kernel's: what walks the reading off
/// is the listen's response crossing the edge, and under a fixed listen the
/// frame's window carries part of that response ([`FIXED_KERNEL_SPREAD`]).
/// Counted in the narrower kernel's own width, the clearance let the Fast frame
/// read 20 Hz tones up to 56 cents off.
const KERNEL_CLEARANCE: f64 = 2.0;

/// The sample of a frame of `fft_size` that every bucket's listen is centred
/// on: [`FIXED_CENTRE`] of the way to the newest sample.
pub(crate) fn listen_centre(fft_size: usize) -> usize {
    (FIXED_CENTRE * fft_size as f64).round() as usize
}

/// How many samples before the newest one a column taken from a frame of
/// `fft_size` belongs on a time axis: where its window's weight is centred,
/// the amplitude centroid of the Hann window times the listen's Gaussian.
///
/// The listen sits toward the newest end and the Hann window pulls its
/// centroid back toward the middle a little, which only the sum says by how
/// much.
pub(crate) fn lag_samples(fft_size: usize) -> f64 {
    let centroid = listen_centre(fft_size) as f64 + fixed_moments(fft_size).1;
    (fft_size - 1) as f64 - centroid
}

/// The listen's window — the frame's Hann window under the listen's Gaussian —
/// summed once for everything that asks about it: its sum, where its weight is
/// centred relative to the listen's centre, and its spread about that.
fn fixed_moments(fft_size: usize) -> (f64, f64, f64) {
    let centre = listen_centre(fft_size) as f64;
    let sigma = FIXED_SPREAD * fft_size as f64;
    let (mut sum, mut first, mut second) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..fft_size {
        let x = i as f64 - centre;
        let d = x / sigma;
        let w = f64::from(crate::hann_weight(i, fft_size)) * (-0.5 * d * d).exp();
        sum += w;
        first += w * x;
        second += w * x * x;
    }
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

/// How many bins [`Kernels::read`] sums at a time: eight floats, which leaves
/// no remainder loop for a kernel of a dozen taps to pay for.
const TAP_CHUNK: usize = 4;

/// One frequency a bucket is read at, over bins `k0..k0 + taps`: from
/// `weights[at..]`, each bin's sign-folded weight twice, once for its real part
/// and once for its imaginary part, so a reading is one straight run over the
/// spectrum's interleaved floats. `taps` is padded with zero weights to a whole
/// number of [`TAP_CHUNK`]s.
#[derive(Clone, Copy, Debug)]
struct Point {
    k0: u32,
    taps: u32,
    at: u32,
}

/// Which kernels each bucket reads, for one frame length and sample rate. Rebuilt by [`plan`](Self::plan) whenever any of those change, and read
/// unchanged by every column between.
pub(crate) struct Kernels {
    /// Per bucket, the range of [`points`](Self::points) it reads; empty is a
    /// bucket the frame cannot place, which reads as nothing.
    spans: Box<[(u32, u32); SPECTRUM_BINS]>,
    points: Vec<Point>,
    weights: Vec<f32>,
    /// The scale that puts a full-scale sine at 1.0 through the listen's
    /// window, the same for every bucket — see [`norm_power`].
    norm: f32,
    /// Where the frame starts in the padded transform — see
    /// [`offset`](Self::offset).
    offset: usize,
    /// See [`window`](Self::window).
    window: Vec<f32>,
    /// Its sum, which the frame's weighted mean is divided by.
    window_sum: f32,
}

impl Kernels {
    pub(crate) fn new() -> Self {
        Kernels {
            spans: Box::new([(0, 0); SPECTRUM_BINS]),
            points: Vec::new(),
            weights: Vec::new(),
            norm: 0.0,
            offset: 0,
            window: Vec::new(),
            window_sum: 0.0,
        }
    }

    /// What the frame is multiplied by before its transform, `fft_size` long:
    /// its Hann window times the share of the listen's Gaussian the kernels do
    /// not read ([`FIXED_KERNEL_SPREAD`]).
    pub(crate) fn window(&self) -> &[f32] {
        &self.window
    }

    /// The sum of [`window`](Self::window).
    pub(crate) fn window_sum(&self) -> f32 {
        self.window_sum
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

    /// Settle every bucket's kernels, and the frame's window, for a frame of
    /// `fft_size` at `sample_rate`.
    pub(crate) fn plan(&mut self, sample_rate: f32, fft_size: usize) {
        let n = fft_size as f64;
        // The padded transform's length, in which its bins are counted.
        let bins_n = (PAD * fft_size) as f64;
        let sr = f64::from(sample_rate);
        let (first, last) = usable_bins(fft_size);
        let centre = listen_centre(fft_size);
        self.offset = PAD * fft_size / 2 - centre;
        // The listen's whole Gaussian and the share of it the kernels read; the
        // share the frame carries, whose product with the kernels' share is the
        // whole listen; and the spread of the window the whole one makes with
        // the Hann window, and the scale that window owes.
        let (sigma, kernel_sigma) = (FIXED_SPREAD * n, FIXED_KERNEL_SPREAD * n);
        let carried = 1.0 / (1.0 / (sigma * sigma) - 1.0 / (kernel_sigma * kernel_sigma)).sqrt();
        let (sum, _, spread) = fixed_moments(fft_size);
        self.norm = norm_power(sum);
        self.window.clear();
        self.window.extend((0..fft_size).map(|i| {
            let d = (i as f64 - centre as f64) / carried;
            (f64::from(crate::hann_weight(i, fft_size)) * (-0.5 * d * d).exp()) as f32
        }));
        self.window_sum = self.window.iter().sum();
        let kernel_std = bins_n / (2.0 * PI * kernel_sigma);
        let clearance = KERNEL_CLEARANCE * bins_n / (2.0 * PI * sigma);
        let reach = KERNEL_REACH * kernel_std;
        // Unit sum over the WHOLE Gaussian, not the kept taps, so a kernel
        // clipped at the edge of the usable bins is not lifted.
        let scale = 1.0 / (kernel_std * (2.0 * PI).sqrt());
        self.points.clear();
        self.weights.clear();
        for (b, frequencies) in BUCKET_FREQUENCIES.iter().enumerate() {
            // Read at more than one frequency where the bucket is wider than the
            // response is, no further apart than half the response's spread:
            // one reading at the centre would dim a partial near the bucket's
            // edge by up to 3 dB at the top of the axis, and readings a whole
            // spread apart still dimmed one between them by 1.1 dB. At half a
            // spread the dip is 0.27 dB.
            let response_hz = sr / (2.0 * PI * spread);
            let (lower, upper) = (f64::from(frequencies.lower_hz), f64::from(frequencies.upper_hz));
            let readings = (2.0 * (upper - lower) / response_hz).ceil().max(1.0) as usize;
            let start = self.points.len() as u32;
            for r in 0..readings {
                let hz = lower + (upper - lower) * (r as f64 + 0.5) / readings as f64;
                let at = hz * bins_n / sr;
                if at < first as f64 + clearance || at > last as f64 - clearance {
                    continue;
                }
                let k0 = ((at - reach).ceil() as usize).max(first);
                let k1 = ((at + reach).floor() as usize).min(last);
                // Padded at the top up to the transform's last bin, Nyquist,
                // and the rest at the bottom. The last usable bin is PAD below
                // Nyquist and the first PAD above DC, so between them the two
                // ends always have room for the TAP_CHUNK - 1 at most.
                let padding = (TAP_CHUNK - (k1 + 1 - k0) % TAP_CHUNK) % TAP_CHUNK;
                let to = (k1 + padding).min(PAD * fft_size / 2);
                let from = k0 - (padding - (to - k1));
                let offset = self.weights.len() as u32;
                // The listen is centred on the transform's middle (see
                // `offset`), and (-1)^k moves the transform's time origin there.
                self.weights.extend((from..=to).flat_map(|k| {
                    let d = (at - k as f64) / kernel_std;
                    let g = if (k0..=k1).contains(&k) { scale * (-0.5 * d * d).exp() } else { 0.0 };
                    let w = (if k % 2 == 0 { g } else { -g }) as f32;
                    [w, w]
                }));
                self.points.push(Point {
                    k0: from as u32,
                    taps: (to + 1 - from) as u32,
                    at: offset,
                });
            }
            self.spans[b] = (start, self.points.len() as u32);
        }
    }

    /// Fill `out` from the frame's padded transform. A bucket read at several
    /// frequencies takes the loudest, so its level does not grow with how many
    /// readings it took, and 0 dB keeps meaning a full-scale sine.
    pub(crate) fn read(&self, spectrum: &[Complex32], out: &mut [f32; SPECTRUM_BINS]) {
        const FLOATS: usize = 2 * TAP_CHUNK;
        // SAFETY: num-complex documents `Complex<T>` as memory layout
        // compatible with `[T; 2]`, real part first, so a slice of them is
        // twice as many `f32`s, real parts at even indices.
        let floats: &[f32] =
            unsafe { std::slice::from_raw_parts(spectrum.as_ptr().cast(), 2 * spectrum.len()) };
        for (out, &(p0, p1)) in out.iter_mut().zip(self.spans.iter()) {
            let mut loudest = 0.0f32;
            for point in &self.points[p0 as usize..p1 as usize] {
                let len = 2 * point.taps as usize;
                let weights = &self.weights[point.at as usize..][..len];
                let bins = &floats[2 * point.k0 as usize..][..len];
                // Eight running sums rather than one, so the taps are summed
                // in vector lanes: one sum is a chain of dependent adds the
                // compiler may not reorder. Even lanes gather the real part
                // and odd ones the imaginary. Reading the kernels is about
                // half a column's cost.
                let mut sums = [0.0f32; FLOATS];
                let (bins, _) = bins.as_chunks::<FLOATS>();
                let (weights, _) = weights.as_chunks::<FLOATS>();
                for (x, w) in bins.iter().zip(weights) {
                    for lane in 0..FLOATS {
                        sums[lane] += x[lane] * w[lane];
                    }
                }
                // Written out: summing the lanes with `step_by` took the loop
                // above out of vector registers and doubled the whole read.
                // The pattern stops compiling if the lane count changes.
                let [r0, i0, r1, i1, r2, i2, r3, i3] = sums;
                let (re, im) = (r0 + r1 + r2 + r3, i0 + i1 + i2 + i3);
                loudest = loudest.max(re * re + im * im);
            }
            *out = loudest * self.norm;
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

#[cfg(test)]
mod tests {
    use harmonigraph_core::spectrum::{hz_to_midi, BINS_PER_SEMITONE, SPECTRUM_MIN_MIDI};

    use super::*;
    use crate::{AnalyzerConfig, SpectrumAnalyzer};

    const RATE: f32 = 48_000.0;

    /// The spread of a periodic Hann window of length `n`, as a fraction of `n`:
    /// `sqrt(1/12 - 1/(2 pi^2))`. A Gaussian of this many samples' spread listens
    /// over the same span of time as the Hann window, which is what lets a length
    /// in samples name both.
    const HANN_SPREAD: f64 = 0.180_773_6;

    fn analyze(config: AnalyzerConfig, hz: f32) -> [f32; SPECTRUM_BINS] {
        let mut analyzer = SpectrumAnalyzer::new(config);
        // The phase in f64: in f32 it loses enough precision over a frame to
        // draw a -74 dB false peak beside an 11 kHz tone.
        let step = std::f64::consts::TAU * f64::from(hz) / f64::from(config.sample_rate);
        let samples: Vec<f32> =
            (0..config.fft_size + 1234).map(|i| (step * i as f64).sin() as f32).collect();
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

    /// The contract every absolute dB rests on, held bucket by bucket: the
    /// listen's window is normalized, and a bucket wider than its response
    /// is read at several frequencies, half the response's spread apart, so a
    /// partial between two readings is dimmed by a quarter of a dB at most.
    #[test]
    fn a_full_scale_sine_reads_unity_across_the_axis() {
        // A partial on the boundary between two of the widest buckets, the one
        // place a single reading per bucket would be furthest from it.
        let top = &BUCKET_FREQUENCIES[SPECTRUM_BINS - 40];
        // The shortest and longest frames.
        for frame in [6144, 24_576] {
            let config = AnalyzerConfig { sample_rate: RATE, fft_size: frame };
            for hz in [30.0, 110.0, 440.0, 1999.0, 7531.0, 15_000.0, top.upper_hz] {
                let level = analyze(config, hz);
                let db = 10.0 * level[peak(&level)].log10();
                assert!((-0.35..0.1).contains(&db), "{hz} Hz through {frame}: {db:+.2} dB");
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
            let config = AnalyzerConfig { sample_rate: RATE, fft_size: n };
            for hz in [55.0f32, 82.41, 110.0, 146.83, 261.63] {
                let level = analyze(config, hz);
                let (at, db) = (1..SPECTRUM_BINS - 1)
                    .filter(|&b| level[b] > level[b - 1] && level[b] >= level[b + 1])
                    .filter(|&b| cents_off(b, hz).abs() > 100.0)
                    .map(|b| {
                        let f = &BUCKET_FREQUENCIES[b];
                        ((f.lower_hz * f.upper_hz).sqrt(), 10.0 * level[b].log10())
                    })
                    .fold((0.0, f32::NEG_INFINITY), |a, b| if b.1 > a.1 { b } else { a });
                assert!(
                    db < -90.0,
                    "{n}-sample frame, {hz} Hz: a false peak at {at:.1} Hz, {db:+.1} dB"
                );
            }
        }
    }

    /// Every frame `configure` accepts reads, however short: padding a kernel
    /// to whole [`TAP_CHUNK`]s at the bottom once overran bin 0 on a 12-sample
    /// frame, whose one kernel spans every usable bin.
    #[test]
    fn every_even_frame_pads_its_kernels_inside_the_transform() {
        for n in (4..=64).step_by(2) {
            let mut analyzer =
                SpectrumAnalyzer::new(AnalyzerConfig { sample_rate: RATE, fft_size: n });
            analyzer.push_samples(&vec![0.5; n]);
            analyzer.pitch_spectrum().expect("a full frame");
        }
    }

    /// Through every fixed frame a partial's peak sits within three hundredths
    /// of a cent of its pitch, far finer than a bucket, which only the kernels'
    /// share of the listen threatens ([`FIXED_KERNEL_SPREAD`]).
    ///
    /// Their repeated copies spread with that share, and a share too wide moves
    /// every peak by tenths of a cent before anything else notices: here 0.009
    /// cents at worst with the kernels reading the whole listen and at 0.3,
    /// 0.018 at 0.35, 0.17 at 0.4 and a cent at 0.45, while the check against a
    /// direct transform passes throughout. The peak is placed between buckets by
    /// a parabola through the three around it in dB. The tones stop at 500 Hz,
    /// above which the Precise frame reads each bucket at several frequencies
    /// and keeps the loudest, which places a peak only to a sixth of a cent
    /// whatever the share, and start at 100, below which the Fast frame's length
    /// rather than the share decides the error.
    #[test]
    fn the_fixed_listens_kernels_leave_every_peak_on_its_pitch() {
        for n in [6144, 12_288, 24_576] {
            let mut analyzer =
                SpectrumAnalyzer::new(AnalyzerConfig { sample_rate: RATE, fft_size: n });
            let mut midi = hz_to_midi(100.0);
            while midi < hz_to_midi(500.0) {
                let hz = 440.0 * 2f64.powf((f64::from(midi) - 69.0) / 12.0);
                // In f64, since an f32 phase is itself a few hundredths of a
                // cent of noise on a tone this long.
                let tone: Vec<f32> = (0..n)
                    .map(|i| (std::f64::consts::TAU * hz * i as f64 / f64::from(RATE)).sin() as f32)
                    .collect();
                analyzer.push_samples(&tone);
                let level = analyzer.pitch_spectrum().expect("a full frame");
                let p = peak(level);
                let [below, at, above] = [p - 1, p, p + 1].map(|b| level[b].log10());
                let between = 0.5 * (below - above) / (below - 2.0 * at + above);
                let off = cents_off(p, hz as f32) + between * 100.0 / BINS_PER_SEMITONE as f32;
                assert!(off.abs() < 0.03, "{n}-sample frame: {hz:.1} Hz peaks {off:+.3} cents off");
                midi += 0.33;
            }
        }
    }

    /// Each fixed frame listens as long as the listen it is asked for — the
    /// plain Hann window each fixed resolution was before #1575, and what the
    /// settings name it by — read off the window as the length of the Hann
    /// window with its spread in time. So a retuned [`FIXED_SPREAD`] or
    /// [`FIXED_CENTRE`] that changed the look fails here rather than leaving
    /// the settings describing the old one.
    #[test]
    fn each_fixed_frame_gives_the_listen_it_is_named_for() {
        for listen in [4096, 8192, 16_384] {
            let frame = fixed_frame(listen);
            let ratio = fixed_moments(frame).2 / HANN_SPREAD / listen as f64;
            assert!((ratio - 1.0).abs() < 0.01, "{frame}-sample frame: {ratio:.4} of {listen}");
        }
    }

    /// The shipped analyzer, ring wrap and padding included, against an
    /// independent f64 transform: each bucket read through its kernel is the
    /// frame, mean removed, through the Hann window times the listen's
    /// Gaussian, evaluated directly at the bucket's own frequency.
    ///
    /// That equality is the whole of what the kernels claim, and the two sides
    /// share nothing but the window and the normalization: one convolves the
    /// padded FFT, the other sums the frame in time. The listen's Gaussian sits
    /// off the frame's middle, which only the frame's placement in the
    /// transform keeps real.
    #[test]
    fn a_kernel_reads_the_frame_through_its_buckets_own_window() {
        for n in [6144, 12_288, 24_576] {
            let mut a = SpectrumAnalyzer::new(AnalyzerConfig { sample_rate: RATE, fft_size: n });
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
            let window: Vec<f64> = (0..n).map(|i| f64::from(crate::hann_weight(i, n))).collect();
            // The mean the analyzer removes, weighted by the window the frame
            // is actually multiplied by: preprocessing, not the claim.
            let frame_window = a.kernels.window();
            let mean = frame.iter().zip(frame_window).map(|(s, &w)| s * f64::from(w)).sum::<f64>()
                / frame_window.iter().map(|&w| f64::from(w)).sum::<f64>();
            let centre = listen_centre(n) as f64;
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
                let sigma = FIXED_SPREAD * n as f64;
                let (mut re, mut im) = (0.0, 0.0);
                for i in 0..n {
                    let d = (i as f64 - centre) / sigma;
                    let x = (frame[i] - mean) * window[i] * (-0.5 * d * d).exp();
                    let angle = std::f64::consts::TAU * hz * i as f64 / f64::from(RATE);
                    re += x * angle.cos();
                    im -= x * angle.sin();
                }
                let expected = (re * re + im * im) * f64::from(a.kernels.norm);
                let error = (f64::from(buckets[b]) - expected).abs() / loudest;
                assert!(error < 1e-4, "N={n}, bucket {b} at {hz:.1} Hz: scaled error {error:e}");
                checked += 1;
            }
            assert!(checked > 20, "N={n}: only {checked} buckets were compared");
        }
    }
}
