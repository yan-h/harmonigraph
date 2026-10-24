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
//! constant-Q kernel, with the Q set by the listen rather than fixed. A fixed
//! listen, the same at every pitch, is shared between the two: part of its
//! Gaussian multiplies the frame before the transform, which narrows every
//! kernel ([`FIXED_KERNEL_SPREAD`]).
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

/// The frame a [`Sweep`] reads every bucket's listen from, whose longest listen
/// is [`max_length`] of it.
pub const SWEEP_FRAME: usize = 16_384;

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

/// The share of the fixed listen the kernels read: a Gaussian of this spread,
/// as a fraction of the frame, with the rest of the listen multiplied into the
/// frame's window before its transform ([`Kernels::window`]).
///
/// A Gaussian of spread `s` is the product of two about the same centre whose
/// spreads `a` and `b` satisfy `1/s^2 = 1/a^2 + 1/b^2`, and the fixed listen is
/// one Gaussian at every pitch, so it can be split anywhere between the frame
/// and the kernels and every bucket still reads through all of it. Under a
/// [`Sweep`] each bucket's Gaussian is its own, so its kernels read all of it.
/// The wider the kernels' share in time, the narrower they are in bins and the
/// fewer taps they take: here 1.06 bins where the whole listen's were 1.84,
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
/// transform would be read through the neighbouring copies' tails as well. That
/// is not a level error but a PITCH one, and it grows as the listen approaches
/// the frame: on an unpadded transform the longest listen read a partial up to
/// 0.06 of a bin off. Padding moves the copies a whole frame further from the
/// listen's centre. Measured against known tones, twice is enough (under 0.2
/// cents above 50 Hz at a Sweep listen of 0.89 of the frame, longer than any
/// it now takes, [`SWEEP_LONGEST`]); four times buys nothing more.
///
/// The frame is placed so that the listen's centre is the transform's middle
/// ([`Kernels::offset`]), which keeps the copies equally far either side: for
/// the fixed listen the nearest one is 1.2 frames from the frame's oldest
/// sample, and the frame's window, which carries the rest of that listen
/// ([`FIXED_KERNEL_SPREAD`]), brings it to at most `1.5e-7` of the listen's
/// peak anywhere in the frame.
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
/// spectrum, and its peak walks away from the partial — at 1.7 of these, a
/// 20.6 Hz partial through a 4096-sample frame read 48 cents off. From 2 the
/// error stays within about 13 cents (3.4 through the Fast frame, whose fixed
/// listen is shorter than that frame's longest), which is what this keeps; nearer the edge the
/// bucket reads as nothing, the truth about a frame too short to place it. At
/// 48 kHz that is 20 to 22 Hz through the Fast frame, and 20 to 44 Hz through
/// it at 96 kHz. A Sweep instead lengthens a listen too short to clear the
/// edge ([`swept_sigma`]), so only a bucket that the longest listen cannot
/// place goes unread there.
///
/// The whole listen's width and not the kernel's: what walks the reading off
/// is the listen's response crossing the edge, and under a fixed listen the
/// frame's window carries part of that response ([`FIXED_KERNEL_SPREAD`]).
/// Counted in the narrower kernel's own width, the clearance let the Fast frame
/// read 20 Hz tones up to 56 cents off.
const KERNEL_CLEARANCE: f64 = 2.0;

/// How many bins of the padded transform, `bins_n` long, a reading through a
/// Gaussian of spread `sigma` has to sit inside the usable bins
/// ([`KERNEL_CLEARANCE`]). Inversely proportional to the spread, which is what
/// lets [`swept_sigma`] solve it for the spread that places a bucket.
fn clearance_bins(sigma: f64, bins_n: f64) -> f64 {
    KERNEL_CLEARANCE * bins_n / (2.0 * PI * sigma)
}

/// The longest listen a Sweep asks for, as a share of its frame, in
/// Hann-equivalent samples ([`max_length`]).
///
/// What bounds it is FALSE PEAKS, as for the fixed listen ([`FIXED_SPREAD`]),
/// which a Sweep cannot dodge the same way: every bucket's Gaussian has to sit
/// on the frame's middle to keep the column's lag the same at every pitch
/// ([`lag_samples`]), so the longest listen reaches both of the frame's ends,
/// and the Hann window's sidelobes come through between their nulls. Across
/// full-scale tones every 0.113 semitone at 48 kHz, with both ends at the
/// one length, the loudest local maximum more than a semitone from the tone
/// is -42.5 dB at 0.89 of the frame (where the narrowest kernel once bounded
/// it), -58 at 0.82, -79 at 0.73, -92 at 0.69, and -110.5 here, 5/8 of it:
/// clean under the lowest floor the levels can be set to, and about where
/// the fixed frames measure. 0.59 is -121.5 dB, but widens every bass peak.
const SWEEP_LONGEST: f64 = 5.0 / 8.0;

/// The longest listen, in Hann-equivalent samples, that a Sweep on a frame of
/// `fft_size` can ask for: [`SWEEP_LONGEST`] of it, so the 16,384-sample Sweep
/// frame reaches 10,240 samples.
pub fn max_length(fft_size: usize) -> f32 {
    (SWEEP_LONGEST * fft_size as f64) as f32
}

/// The spread of the frame's Hann envelope times a Gaussian of spread `sigma`
/// centred on it: spreads of two Gaussian-like envelopes combine as their
/// reciprocal squares.
fn effective_spread(sigma: f64, n: f64) -> f64 {
    let frame = HANN_SPREAD * n;
    (1.0 / (1.0 / (sigma * sigma) + 1.0 / (frame * frame))).sqrt()
}

/// The spread of the Gaussian that brings the frame's Hann envelope down to a
/// listen of `length` Hann-equivalent samples, on a frame of `n`: the inverse
/// of [`effective_spread`]. Finite for every length under the frame's own.
fn gaussian_for(length: f64, n: f64) -> f64 {
    let (tau, frame) = (length * HANN_SPREAD, HANN_SPREAD * n);
    1.0 / (1.0 / (tau * tau) - 1.0 / (frame * frame)).sqrt()
}

/// The spread of the Gaussian a bucket listens through under `sweep`, on a
/// frame of `fft_size` at `sample_rate`: the length the sweep asks for at the
/// bucket's centre, gliding log-linearly between its ends, unless that is too
/// short to place the bucket.
///
/// A short listen is a wide kernel, and a kernel that would cross the bottom
/// of the usable bins is not read ([`KERNEL_CLEARANCE`]). There the bucket
/// listens instead as long as placing its lowest frequency takes, up to the
/// longest listen. Left unread, a short bass listen cut the axis off with a
/// wall under which nothing read: at 48 kHz, 175 Hz at a 256-sample listen,
/// 75 Hz at 1024. At 48 kHz 20 Hz takes 4740 samples, at 96 kHz 9660, and
/// at 192 kHz more than the longest, so the bottom few buckets there stay
/// unread whatever the sweep.
fn swept_sigma(
    sweep: Sweep,
    fft_size: usize,
    sample_rate: f64,
    frequencies: &crate::BucketFrequencies,
) -> f64 {
    let n = fft_size as f64;
    let longest = f64::from(max_length(fft_size));
    let length = |l: f32| f64::from(l).clamp(f64::from(SWEEP_MIN_LENGTH), longest).ln();
    let hz = f64::from(frequencies.center_hz);
    let u = ((hz / 20.0).ln() / (20_000.0f64 / 20.0).ln()).clamp(0.0, 1.0);
    let (bass, treble) = (length(sweep.bass), length(sweep.treble));
    let asked = gaussian_for((bass + (treble - bass) * u).exp(), n);
    // The clearance `plan` asks of the lowest reading, solved for the spread.
    let bins_n = (PAD * fft_size) as f64;
    let room =
        f64::from(frequencies.lower_hz) * bins_n / sample_rate - usable_bins(fft_size).0 as f64;
    let placed = if room > 0.0 { clearance_bins(1.0, bins_n) / room } else { f64::INFINITY };
    asked.max(placed).min(gaussian_for(longest, n))
}

/// PROTOTYPE (#1600): a Sweep reads every bucket through its own gamma window
/// on an untapered frame instead of the frame's Hann window under a centred
/// Gaussian, when `HARMONIGRAPH_SWEEP_GAMMA` is set. Not for merging.
pub(crate) fn gamma() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("HARMONIGRAPH_SWEEP_GAMMA").is_some())
}

/// The gamma Sweep's longest listen on a frame of `fft_size`: half of it, so
/// the window's tail has died to about 1e-6 by the frame's oldest sample.
pub fn gamma_max_length(fft_size: usize) -> f64 {
    fft_size as f64 / 2.0
}

/// How far behind the newest sample every gamma window's centroid sits: half
/// the longest listen, as the fixed windows' is.
fn gamma_delay(fft_size: usize) -> f64 {
    0.502 * gamma_max_length(fft_size)
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
    if sweep && gamma() {
        return gamma_delay(fft_size) - 0.5;
    }
    let centre = listen_centre(fft_size, sweep) as f64;
    let centroid = if sweep { centre } else { centre + fixed_moments(fft_size).1 };
    (fft_size - 1) as f64 - centroid
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
    /// PROTOTYPE: complex taps, stored as two runs of `2 * taps` floats: the
    /// first sums to the reading's real part, the second to its imaginary.
    complex: bool,
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
            norm: Box::new([0.0; SPECTRUM_BINS]),
            offset: 0,
            window: Vec::new(),
            window_sum: 0.0,
        }
    }

    /// What the frame is multiplied by before its transform, `fft_size` long:
    /// its Hann window, under a fixed listen times the share of that listen's
    /// Gaussian the kernels do not read ([`FIXED_KERNEL_SPREAD`]).
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

    /// Settle every bucket's kernels, and the frame's window, listening along
    /// `sweep` or, without one, through the fixed listen. `hann` is the frame's
    /// Hann window, `fft_size` long, looked up rather than recomputed because a
    /// Sweep's plan reads it once per bucket at every slider tick.
    pub(crate) fn plan(
        &mut self,
        sweep: Option<Sweep>,
        sample_rate: f32,
        fft_size: usize,
        hann: &[f32],
    ) {
        if let Some(sweep) = sweep.filter(|_| gamma()) {
            return self.plan_gamma(sweep, sample_rate, fft_size);
        }
        let n = fft_size as f64;
        // The padded transform's length, in which its bins are counted.
        let bins_n = (PAD * fft_size) as f64;
        let sr = f64::from(sample_rate);
        let (first, last) = usable_bins(fft_size);
        let centre = listen_centre(fft_size, sweep.is_some());
        self.offset = PAD * fft_size / 2 - centre;
        let hann = |i: usize| f64::from(hann[i]);
        // The share of the fixed listen the frame carries: the Gaussian whose
        // product with the kernels' share is the whole listen.
        let carried = sweep.is_none().then(|| {
            let (whole, kernels) = (FIXED_SPREAD * n, FIXED_KERNEL_SPREAD * n);
            1.0 / (1.0 / (whole * whole) - 1.0 / (kernels * kernels)).sqrt()
        });
        self.window.clear();
        self.window.extend((0..fft_size).map(|i| {
            let gaussian = carried.map_or(1.0, |spread| {
                let d = (i as f64 - centre as f64) / spread;
                (-0.5 * d * d).exp()
            });
            (hann(i) * gaussian) as f32
        }));
        self.window_sum = self.window.iter().sum();
        // One listen at every pitch without a sweep, so its moments are summed
        // once: the whole Gaussian's spread and the kernels' share of it, the
        // spread of the window the whole one makes with the Hann window, and
        // the scale that window owes.
        let fixed = sweep.is_none().then(|| {
            let (sum, _, spread) = fixed_moments(fft_size);
            (FIXED_SPREAD * n, FIXED_KERNEL_SPREAD * n, spread, norm_power(sum))
        });
        self.points.clear();
        self.weights.clear();
        for (b, frequencies) in BUCKET_FREQUENCIES.iter().enumerate() {
            // The listen's whole Gaussian, and the share of it the kernel reads.
            let (sigma, kernel_sigma, spread, norm) = match (sweep, fixed) {
                (Some(sweep), _) => {
                    let sigma = swept_sigma(sweep, fft_size, sr, frequencies);
                    let [sum, _, _] = listen_moments(fft_size, centre, sigma, hann);
                    (sigma, sigma, effective_spread(sigma, n), norm_power(sum))
                }
                (None, fixed) => fixed.expect("summed without a sweep"),
            };
            let kernel_std = bins_n / (2.0 * PI * kernel_sigma);

            // Read at more than one frequency where the bucket is wider than the
            // response is, no further apart than half the response's spread:
            // one reading at the centre would dim a partial near the bucket's
            // edge by up to 3 dB at the top of the axis, and readings a whole
            // spread apart still dimmed one between them by 1.1 dB. At half a
            // spread the dip is 0.27 dB.
            let response_hz = sr / (2.0 * PI * spread);
            let (lower, upper) = (f64::from(frequencies.lower_hz), f64::from(frequencies.upper_hz));
            let readings = (2.0 * (upper - lower) / response_hz).ceil().max(1.0) as usize;
            let clearance = clearance_bins(sigma, bins_n);
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
                    complex: false,
                });
            }
            self.spans[b] = (start, self.points.len() as u32);
            self.norm[b] = norm;
        }
    }

    /// PROTOTYPE (#1600): every bucket of `sweep` reads the UNTAPERED frame
    /// through its own whole window, a gamma envelope `tau^(K-1) e^(-tau/mu)`
    /// with `tau` counted back from the newest sample, so its kernel is that
    /// window's transform: `(1 - i w mu)^(-K)` times the phase of where the
    /// frame sits in the transform (Brown and Puckette's spectral kernel). Each
    /// bucket's spread is its Hann-equivalent listen's, and every window's
    /// centroid sits [`gamma_delay`] behind the newest sample, so the column
    /// stays aligned across the axis. The spectrum of a gamma window falls
    /// away from its peak monotonically at every length: no sidelobes, so no
    /// cap but the frame's room for the window's tail.
    fn plan_gamma(&mut self, sweep: Sweep, sample_rate: f32, fft_size: usize) {
        let m = PAD * fft_size;
        let bins_n = m as f64;
        let sr = f64::from(sample_rate);
        let (first, last) = usable_bins(fft_size);
        self.offset = PAD * fft_size / 2 - fft_size / 2;
        self.window.clear();
        self.window.resize(fft_size, 1.0);
        self.window_sum = fft_size as f32;
        self.points.clear();
        self.weights.clear();
        let longest = gamma_max_length(fft_size);
        let delay = gamma_delay(fft_size);
        // Where tau = 0 falls in the transform: half a sample past the newest.
        let t0 = (self.offset + fft_size) as f64 - 0.5;
        let length = |l: f32| f64::from(l).clamp(f64::from(SWEEP_MIN_LENGTH), longest).ln();
        let (bass, treble) = (length(sweep.bass), length(sweep.treble));
        for (b, frequencies) in BUCKET_FREQUENCIES.iter().enumerate() {
            let hz = f64::from(frequencies.center_hz);
            let u = ((hz / 20.0).ln() / (20_000.0f64 / 20.0).ln()).clamp(0.0, 1.0);
            let asked = (bass + (treble - bass) * u).exp() * HANN_SPREAD;
            // The same placement floor as the Hann Sweep's (`swept_sigma`).
            let room = f64::from(frequencies.lower_hz) * bins_n / sr - first as f64;
            let placed =
                if room > 0.0 { clearance_bins(1.0, bins_n) / room } else { f64::INFINITY };
            let sigma = asked.max(placed).min(longest * HANN_SPREAD);
            let k = (delay / sigma).powi(2);
            let mu = sigma * sigma / delay;
            // Out to -130 dB of the kernel's peak, in bins either side.
            let reach = ((10f64.powf(13.0 / k) - 1.0).sqrt() / mu) * bins_n / (2.0 * PI);
            let clearance = clearance_bins(sigma, bins_n);
            let response_hz = sr / (2.0 * PI * sigma);
            let (lower, upper) = (f64::from(frequencies.lower_hz), f64::from(frequencies.upper_hz));
            let readings = (2.0 * (upper - lower) / response_hz).ceil().max(1.0) as usize;
            let start = self.points.len() as u32;
            for r in 0..readings {
                let f = lower + (upper - lower) * (r as f64 + 0.5) / readings as f64;
                let at = f * bins_n / sr;
                if at < first as f64 + clearance || at > last as f64 - clearance {
                    continue;
                }
                let k0 = ((at - reach).ceil() as usize).max(first);
                let k1 = ((at + reach).floor() as usize).min(last);
                let padding = (TAP_CHUNK - (k1 + 1 - k0) % TAP_CHUNK) % TAP_CHUNK;
                let to = (k1 + padding).min(PAD * fft_size / 2);
                let from = k0 - (padding - (to - k1));
                let theta = 2.0 * PI * f / sr;
                let taps: Vec<(f64, f64)> = (from..=to)
                    .map(|kk| {
                        if !(k0..=k1).contains(&kk) {
                            return (0.0, 0.0);
                        }
                        let w = theta - 2.0 * PI * kk as f64 / bins_n;
                        // (1 - i w mu)^(-K) = exp(-K ln(1 - i w mu)).
                        let (lr, li) = (0.5 * (w * mu).mul_add(w * mu, 1.0).ln(), -(w * mu).atan());
                        let (mag, phase) = ((-k * lr).exp(), -k * li - w * t0);
                        (mag * phase.cos() / bins_n, mag * phase.sin() / bins_n)
                    })
                    .collect();
                let at_weights = self.weights.len() as u32;
                // Real part: re*a - im*b; imaginary: re*b + im*a.
                self.weights.extend(taps.iter().flat_map(|&(a, bb)| [a as f32, -bb as f32]));
                self.weights.extend(taps.iter().flat_map(|&(a, bb)| [bb as f32, a as f32]));
                self.points.push(Point {
                    k0: from as u32,
                    taps: (to + 1 - from) as u32,
                    at: at_weights,
                    complex: true,
                });
            }
            self.spans[b] = (start, self.points.len() as u32);
            self.norm[b] = norm_power(1.0);
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
        for ((out, &(p0, p1)), norm) in out.iter_mut().zip(self.spans.iter()).zip(self.norm.iter())
        {
            let mut loudest = 0.0f32;
            for point in &self.points[p0 as usize..p1 as usize] {
                if point.complex {
                    let len = 2 * point.taps as usize;
                    let bins = &floats[2 * point.k0 as usize..][..len];
                    let dot = |weights: &[f32]| {
                        let mut sums = [0.0f32; FLOATS];
                        let (bins, _) = bins.as_chunks::<FLOATS>();
                        let (weights, _) = weights.as_chunks::<FLOATS>();
                        for (x, w) in bins.iter().zip(weights) {
                            for lane in 0..FLOATS {
                                sums[lane] += x[lane] * w[lane];
                            }
                        }
                        let [a, b, c, d, e, f, g, h] = sums;
                        (a + b) + (c + d) + (e + f) + (g + h)
                    };
                    let at = point.at as usize;
                    let re = dot(&self.weights[at..][..len]);
                    let im = dot(&self.weights[at + len..][..len]);
                    loudest = loudest.max(re * re + im * im);
                    continue;
                }
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
    /// The fresh sweep: the longest listen the frame allows at the bottom, and
    /// half of Fast at the top.
    const FRESH: Sweep = Sweep { bass: 10_240.0, treble: 2048.0 };

    fn config(sweep: Sweep) -> AnalyzerConfig {
        AnalyzerConfig { sample_rate: RATE, fft_size: FRAME, sweep: Some(sweep) }
    }

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

    /// A Sweep's longest listen draws no false peak beside a loud partial
    /// either (#1592). Centred on the frame, a listen that nears the frame's
    /// length reads the frame's own Hann sidelobes between their nulls: at 0.89
    /// of the frame, which the sliders reached, -42 dB a few semitones either
    /// side of every tone. Both ends at the longest, so every bucket listens
    /// that long, and tones from the bottom of the axis up.
    #[test]
    fn the_longest_sweep_raises_no_false_peak() {
        let longest = max_length(FRAME);
        let config = config(Sweep { bass: longest, treble: longest });
        for hz in [24.0f32, 31.0, 38.0, 55.0, 93.0, 211.0, 440.0] {
            let level = analyze(config, hz);
            let (at, db) = (1..SPECTRUM_BINS - 1)
                .filter(|&b| level[b] > level[b - 1] && level[b] >= level[b + 1])
                .filter(|&b| cents_off(b, hz).abs() > 100.0)
                .map(|b| (BUCKET_FREQUENCIES[b].center_hz, 10.0 * level[b].log10()))
                .fold((0.0, f32::NEG_INFINITY), |a, b| if b.1 > a.1 { b } else { a });
            assert!(db < -100.0, "{hz} Hz: a false peak at {at:.1} Hz, {db:+.1} dB");
        }
    }

    /// A short bass listen still reads the bass: where the listen asked for is
    /// too short to place a bucket clear of the bottom of the usable bins
    /// ([`KERNEL_CLEARANCE`]), the bucket listens as long as placing it takes.
    /// Left unread instead, a 256-sample bass listen read nothing under 175 Hz
    /// at 48 kHz, and the lowest bucket that did read drew a 60 Hz partial
    /// there, at -8 dB. At 96 kHz the bins are twice as wide, so it takes a
    /// listen twice as long, and the lowest buckets need most of the longest.
    #[test]
    fn a_short_bass_listen_still_places_every_partial() {
        let sweep = Some(Sweep { bass: SWEEP_MIN_LENGTH, treble: 2048.0 });
        for sample_rate in [48_000.0, 96_000.0] {
            let config = AnalyzerConfig { sample_rate, fft_size: FRAME, sweep };
            for hz in [21.0f32, 25.0, 40.0, 60.0, 100.0, 150.0] {
                let level = analyze(config, hz);
                let (off, db) = (cents_off(peak(&level), hz), 10.0 * level[peak(&level)].log10());
                assert!(off.abs() < 15.0, "{sample_rate} Hz, {hz} Hz reads {off:+.1} cents off");
                assert!((-0.35..0.1).contains(&db), "{sample_rate} Hz, {hz} Hz reads {db:+.2} dB");
            }
            assert!(
                analyze(config, 21.0)[0] > 0.0,
                "{sample_rate} Hz: the bottom bucket is unread"
            );
        }
    }

    /// Every frame `configure` accepts reads, however short: padding a kernel
    /// to whole [`TAP_CHUNK`]s at the bottom once overran bin 0 on a 12-sample
    /// frame, whose one kernel spans every usable bin.
    #[test]
    fn every_even_frame_pads_its_kernels_inside_the_transform() {
        for n in (4..=64).step_by(2) {
            let mut analyzer = SpectrumAnalyzer::new(AnalyzerConfig {
                sample_rate: RATE,
                fft_size: n,
                sweep: None,
            });
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
            let mut analyzer = SpectrumAnalyzer::new(AnalyzerConfig {
                sample_rate: RATE,
                fft_size: n,
                sweep: None,
            });
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

    /// The lengths are Hann-equivalent: a sweep's peak at either end is as wide
    /// as a single Hann window of the length the sweep asks for there, which is
    /// what lets the sliders be read against Fast, Balanced and Precise.
    ///
    /// The treble end is the shortest listen allowed, whose peak spans eight
    /// buckets at 17 kHz. A 1024-sample one there is 64 Hz wide against 31 Hz
    /// buckets, so its measured width is two buckets whatever the listen, and
    /// the same two for a sweep three times as long.
    #[test]
    fn the_listen_at_each_end_matches_a_hann_window_of_that_length() {
        let sweep = Sweep { bass: 10_240.0, treble: SWEEP_MIN_LENGTH };
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
            let window: Vec<f64> = (0..n).map(|i| f64::from(crate::hann_weight(i, n))).collect();
            // The mean the analyzer removes, weighted by the window the frame
            // is actually multiplied by: preprocessing, not the claim.
            let frame_window = a.kernels.window();
            let mean = frame.iter().zip(frame_window).map(|(s, &w)| s * f64::from(w)).sum::<f64>()
                / frame_window.iter().map(|&w| f64::from(w)).sum::<f64>();
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
                    swept_sigma(sweep, n, f64::from(RATE), f)
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

#[cfg(test)]
mod probe_gamma_impl {
    use super::*;
    use crate::{AnalyzerConfig, SpectrumAnalyzer};
    use harmonigraph_core::spectrum::{hz_to_midi, BINS_PER_SEMITONE, SPECTRUM_MIN_MIDI};

    const N: usize = 32_768;

    #[test]
    #[ignore]
    fn gamma_reads() {
        std::env::set_var("HARMONIGRAPH_SWEEP_GAMMA", "1");
        assert!(gamma());
        for rate in [48_000.0f32, 96_000.0] {
            let sweep = Sweep { bass: 16_384.0, treble: 2048.0 };
            let config = AnalyzerConfig { sample_rate: rate, fft_size: N, sweep: Some(sweep) };
            println!("rate {rate}: column lag {:.1} ms", config.column_lag() * 1000.0);
            // 1. Against a direct f64 read through each bucket's gamma window.
            let mut a = SpectrumAnalyzer::new(config);
            let input: Vec<f32> = (0..N + 997)
                .map(|i| {
                    let t = i as f64 / f64::from(rate);
                    ((std::f64::consts::TAU * 440.0 * t).sin() * 0.4
                        + (std::f64::consts::TAU * 7137.0 * t).cos() * 0.2
                        + (std::f64::consts::TAU * 31.0 * t).sin() * 0.3) as f32
                })
                .collect();
            a.push_samples(&input);
            let buckets = *a.pitch_spectrum().unwrap();
            let frame: Vec<f64> = (0..N).map(|i| f64::from(a.ring[(a.write + i) % N])).collect();
            let mean = frame.iter().sum::<f64>() / N as f64;
            let longest = gamma_max_length(N);
            let delay = gamma_delay(N);
            let mut worst = 0.0f64;
            for b in (0..SPECTRUM_BINS).step_by(97) {
                let (p0, p1) = a.kernels.spans[b];
                if p1 != p0 + 1 {
                    continue;
                }
                let fr = &BUCKET_FREQUENCIES[b];
                let hz = 0.5 * f64::from(fr.lower_hz + fr.upper_hz);
                let u = ((f64::from(fr.center_hz) / 20.0).ln() / 1000f64.ln()).clamp(0.0, 1.0);
                let asked =
                    (16_384f64.ln() + (2048f64.ln() - 16_384f64.ln()) * u).exp().min(longest)
                        * HANN_SPREAD;
                let bins_n = (PAD * N) as f64;
                let room =
                    f64::from(fr.lower_hz) * bins_n / f64::from(rate) - usable_bins(N).0 as f64;
                let sigma =
                    asked.max(clearance_bins(1.0, bins_n) / room).min(longest * HANN_SPREAD);
                let (k, mu) = ((delay / sigma).powi(2), sigma * sigma / delay);
                let w: Vec<f64> = (0..N)
                    .map(|i| {
                        let tau = (N - 1 - i) as f64 + 0.5;
                        ((k - 1.0) * tau.ln() - tau / mu).exp()
                    })
                    .collect();
                let s: f64 = w.iter().sum();
                let (mut re, mut im) = (0.0, 0.0);
                for i in 0..N {
                    let x = (frame[i] - mean) * w[i] / s;
                    let ang = std::f64::consts::TAU * hz * i as f64 / f64::from(rate);
                    re += x * ang.cos();
                    im -= x * ang.sin();
                }
                let direct = 4.0 * (re * re + im * im);
                let got = f64::from(buckets[b]);
                let db = 10.0 * (got.max(1e-30) / direct.max(1e-30)).log10();
                if direct > 1e-9 {
                    worst = worst.max(db.abs());
                }
            }
            println!("  kernel vs direct: worst {worst:.3} dB");
            // 2. False peaks, pitch, level through the analyzer.
            let (mut fp, mut cents, mut lmin) = (f64::NEG_INFINITY, 0.0f64, f64::INFINITY);
            let mut midi = 15.8f32;
            while midi < 128.0 {
                let hz = 440.0 * 2f32.powf((midi - 69.0) / 12.0);
                let step = std::f64::consts::TAU * f64::from(hz) / f64::from(rate);
                let s: Vec<f32> = (0..N).map(|i| (step * i as f64).sin() as f32).collect();
                a.push_samples(&s);
                let l = *a.pitch_spectrum().unwrap();
                let pk = (0..SPECTRUM_BINS).max_by(|&x, &y| l[x].total_cmp(&l[y])).unwrap();
                let c = |b: usize| {
                    100.0
                        * (SPECTRUM_MIN_MIDI + (b as f32 + 0.5) / BINS_PER_SEMITONE as f32
                            - hz_to_midi(hz))
                };
                cents = cents.max(f64::from(c(pk).abs()));
                lmin = lmin.min(10.0 * f64::from(l[pk]).log10());
                for b in 1..SPECTRUM_BINS - 1 {
                    if l[b] > l[b - 1] && l[b] >= l[b + 1] && c(b).abs() > 100.0 {
                        fp = fp.max(10.0 * f64::from(l[b]).log10());
                    }
                }
                midi += 0.37;
            }
            println!(
                "  tones: false peak {fp:.1} dB | pitch {cents:.1}c | lowest level {lmin:.2} dB"
            );
        }
    }
}

#[cfg(test)]
mod probe_gamma_cost {
    use super::*;
    use crate::{AnalyzerConfig, SpectrumAnalyzer};

    fn time(config: AnalyzerConfig) -> (f64, usize) {
        let mut a = SpectrumAnalyzer::new(config);
        let s: Vec<f32> = (0..config.fft_size)
            .map(|i| ((i as f32) * 0.0123).sin() * 0.3 + ((i as f32) * 0.7).sin() * 0.1)
            .collect();
        a.push_samples(&s);
        for _ in 0..20 {
            a.pitch_spectrum();
        }
        let t = std::time::Instant::now();
        let reps = 300;
        for _ in 0..reps {
            std::hint::black_box(a.pitch_spectrum());
        }
        let taps: usize =
            a.kernels.points.iter().map(|p| p.taps as usize * if p.complex { 2 } else { 1 }).sum();
        (t.elapsed().as_secs_f64() * 1e6 / reps as f64, taps)
    }

    #[test]
    #[ignore]
    fn cost() {
        let gamma_on = gamma();
        let rate = 48_000.0;
        if !gamma_on {
            let (us, taps) = time(AnalyzerConfig {
                sample_rate: rate,
                fft_size: 16_384,
                sweep: Some(Sweep { bass: 10_240.0, treble: 2048.0 }),
            });
            println!("today Sweep 10240/2048: {us:.0} us/column, {taps} tap-units");
            let (us, taps) = time(AnalyzerConfig {
                sample_rate: rate,
                fft_size: 16_384,
                sweep: Some(Sweep { bass: 10_240.0, treble: 256.0 }),
            });
            println!("today Sweep 10240/256: {us:.0} us/column, {taps} tap-units");
            let (us, taps) =
                time(AnalyzerConfig { sample_rate: rate, fft_size: 24_576, sweep: None });
            println!("Precise: {us:.0} us/column, {taps} tap-units");
        } else {
            let (us, taps) = time(AnalyzerConfig {
                sample_rate: rate,
                fft_size: 32_768,
                sweep: Some(Sweep { bass: 16_384.0, treble: 2048.0 }),
            });
            println!("gamma Sweep 16384/2048: {us:.0} us/column, {taps} tap-units");
            let (us, taps) = time(AnalyzerConfig {
                sample_rate: rate,
                fft_size: 32_768,
                sweep: Some(Sweep { bass: 16_384.0, treble: 256.0 }),
            });
            println!("gamma Sweep 16384/256: {us:.0} us/column, {taps} tap-units");
        }
    }
}
