//! How every pitch bucket reads the frame's transform: at its own frequency,
//! through its own listen.
//!
//! **One transform, and a small kernel per bucket.** Reading the frame through
//! a window is multiplying it by that window, and multiplying in time is
//! convolving in frequency. So each column takes one ordinary transform of the
//! whole frame, and each bucket then reads it through its own kernel: its
//! window's transform, sampled at the frame's bins about the bucket's own
//! frequency. A long window is a narrow kernel, so the bass costs a few taps
//! and the treble a few hundred per bucket. This is Brown and Puckette's
//! frequency-domain constant-Q kernel, with the Q set by the listen rather than
//! fixed.
//!
//! **Each kernel is centred on the bucket's own frequency rather than on a
//! bin**, so a partial reads at its own pitch wherever it sits between bins.
//! That is the whole reason every window reads this way, and not only a
//! [`Sweep`]: the resample of the bins it replaced kept every peak on the bin it
//! was measured at, which through the old 4096-sample Fast frame drew a 55 Hz A1
//! more than a semitone sharp (#1575).
//!
//! **Where and how long each bucket listens** is the one thing the settings
//! decide, and the two kinds of setting read differently.
//!
//! A fixed window listens through one Gaussian, the same at every pitch, set
//! toward the frame's newest end under the frame's Hann window
//! ([`FIXED_SPREAD`], [`FIXED_CENTRE`]). Being one listen, it is split between
//! the frame and the kernels: part of its Gaussian multiplies the frame before
//! the transform ([`FIXED_KERNEL_SPREAD`]), and the rest is the kernels, whose
//! transform is another Gaussian, real and `PAD * fft_size / (2 pi sigma)` bins
//! wide.
//!
//! A [`Sweep`] gives every bucket a listen of its own, gliding log-linearly from
//! [`Sweep::bass`] at 20 Hz to [`Sweep::treble`] at 20 kHz, with no bands and so
//! no seams between them. A fixed window cannot suit both ends of a pitch axis:
//! low partials sit a few hertz apart and need a long listen to be told apart,
//! high ones are far apart and want a short one so onsets stay sharp. Each
//! bucket reads the UNTAPERED frame through a gamma window,
//! `tau^(K-1) e^(-tau/mu)` with `tau` counted back from the newest sample (the
//! gammatone filter's envelope), and its kernel is that whole window's
//! transform, `(1 - i omega mu)^(-K)`: complex, and falling away from its peak
//! with no sidelobes at any length. The frame has no edge of its own but its
//! oldest sample, where the longest window's tail is down to a millionth of its
//! peak ([`max_length`]).
//!
//! That is what replaced Gaussians centred under the frame's Hann window, the
//! way a fixed window listens (#1600). A centred listen that nears the frame's
//! length reads the Hann window's sidelobes between their nulls, so those
//! capped the longest listen at 5/8 of the frame; moved off the middle, to
//! share one instant without the cap, they drew the sidelobes in bands of
//! lengths that a sweep's bass glides through.
//!
//! What a gamma window gives up is its far skirt. It rises from the newest
//! sample as `tau^(K-1)`, and the longer the window the smaller `K` and the
//! heavier its skirt far from its peak, where a Gaussian's falls away fastest.
//! Where a sweep lengthens toward the bass, the bottom of the axis then hears a
//! loud tone an octave or two up more than the buckets above it do, a faint
//! shelf at 20 to 25 Hz: -113 dB through the fresh sweep at 48 kHz, and -97 at
//! 96 kHz, where a sample is half as long (the centred listens drew -100).
//!
//! Either way every bucket's window has its weight centred on the same instant,
//! so the column is time-aligned across the axis and belongs there
//! ([`lag_samples`]). Under a Sweep, a short treble window is a narrow bump
//! about the longest one's centroid ([`SWEEP_CENTROID`]) and a long bass window
//! a wide, lopsided one about the same instant, so a longer listen in the bass
//! costs no misalignment, only latency.

use std::f64::consts::PI;

use harmonigraph_core::spectrum::SPECTRUM_BINS;
use realfft::num_complex::Complex32;

use crate::BUCKET_FREQUENCIES;

/// The listen lengths at the two ends of the pitch axis, each in samples of an
/// equivalent Hann window: what a single Hann window that long would spread its
/// energy over in time. So `4096` at either end listens as sharply as Fast,
/// whose frame is longer but whose listen is a 4096-sample Hann's.
///
/// Each bucket listens through a gamma window with that spread in time
/// ([`HANN_SPREAD`] of the length), and every window's weight is centred on
/// the same instant ([`SWEEP_CENTROID`]). A peak is then as wide at half power
/// as the Hann window's, to within 5%: the gamma's response is a Gaussian's
/// near its peak, a hair wider the more lopsided the window, which the longest
/// listen's is most.
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
/// is [`max_length`] of it: twice as long, the room the longest gamma window's
/// tail needs.
pub const SWEEP_FRAME: usize = 32_768;

/// The spread of a periodic Hann window of length `n`, as a fraction of `n`:
/// `sqrt(1/12 - 1/(2 pi^2))`. A Gaussian or a gamma window of this many
/// samples' spread listens over the same span of time as the Hann window, which
/// is what lets a length in samples name all three.
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
/// [`Sweep`] each bucket's window is its own, so its kernel reads all of it.
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
/// A kernel sampled at whole bins is the transform of the frame times a window
/// REPEATED every transform length, so a frame that filled the whole transform
/// would be read through the neighbouring copies' tails as well. That is not a
/// level error but a PITCH one, and it grows as the listen approaches the
/// frame: on an unpadded transform a centred Gaussian listen of 0.89 of the
/// frame read a partial up to 0.06 of a bin off, and twice the frame brought it
/// under 0.2 cents above 50 Hz; four times bought nothing more.
///
/// For the fixed listen the frame is placed so that the listen's centre is the
/// transform's middle ([`Kernels::offset`]), which keeps the copies equally far
/// either side: the nearest one is 1.2 frames from the frame's oldest sample,
/// and the frame's window, which carries the rest of that listen
/// ([`FIXED_KERNEL_SPREAD`]), brings it to at most `1.5e-7` of the listen's
/// peak anywhere in the frame. A Sweep's gamma window is nothing before its
/// newest sample, so the only copy that reaches the frame is the one a whole
/// transform older, two frames back, past four times the longest window's
/// centroid, where its tail is under `1e-17` of its peak.
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
///
/// A Sweep's kernels are cut by their depth instead ([`SWEEP_KERNEL_DEPTH`]),
/// since a gamma window's transform falls as a power of the distance rather
/// than a Gaussian.
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

/// The longest listen, in Hann-equivalent samples, that a Sweep on a frame of
/// `fft_size` can ask for: half of it, so the 32,768-sample Sweep frame reaches
/// 16,384 samples, Precise's listen.
///
/// What bounds it is the frame's oldest sample, the one edge an untapered frame
/// has. The longest window's tail is down to about a millionth of its peak
/// there, -120 dB, an edge too small to draw anything.
pub fn max_length(fft_size: usize) -> f32 {
    (fft_size / 2) as f32
}

/// Where every Sweep window's weight is centred behind the newest sample, as a
/// share of the longest listen ([`max_length`]).
///
/// One instant for every bucket keeps the column aligned across the axis,
/// and it is the column's lag ([`lag_samples`]): a hair over half the longest
/// listen puts it at 171 ms for the 16,384-sample listen at 48 kHz, as far
/// behind as Precise. Nearer the newest sample, the longest window would keep
/// its spread with a smaller `K`, rising more abruptly from that sample with a
/// heavier skirt, and a longer tail toward the frame's oldest end.
const SWEEP_CENTROID: f64 = 0.502;

/// How many samples behind the newest one every Sweep window on a frame of
/// `fft_size` has its weight centred ([`SWEEP_CENTROID`]).
fn sweep_centroid(fft_size: usize) -> f64 {
    SWEEP_CENTROID * f64::from(max_length(fft_size))
}

/// The shape `(K, mu)` of the gamma window `tau^(K-1) e^(-tau/mu)` whose spread
/// is `sigma` samples and whose weight is centred `centroid` samples back: a
/// gamma's mean is `K mu` and its variance `K mu^2`.
fn gamma_shape(sigma: f64, centroid: f64) -> (f64, f64) {
    ((centroid / sigma).powi(2), sigma * sigma / centroid)
}

/// How deep a Sweep's kernels are kept: out to where the window's transform
/// has fallen to this share of its peak, an amplitude.
///
/// A gamma window's transform falls away from its peak as
/// `(1 + omega^2 mu^2)^(-K/2)`, without lobes, so where it is cut is a depth
/// rather than a count of spreads ([`KERNEL_REACH`]).
const SWEEP_KERNEL_DEPTH: f64 = 1e-7;

/// The spread of the gamma window a bucket listens through under `sweep`, on a
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
    let longest = f64::from(max_length(fft_size));
    let length = |l: f32| f64::from(l).clamp(f64::from(SWEEP_MIN_LENGTH), longest).ln();
    let hz = f64::from(frequencies.center_hz);
    let u = ((hz / 20.0).ln() / (20_000.0f64 / 20.0).ln()).clamp(0.0, 1.0);
    let (bass, treble) = (length(sweep.bass), length(sweep.treble));
    let asked = HANN_SPREAD * (bass + (treble - bass) * u).exp();
    // The clearance `plan` asks of the lowest reading, solved for the spread.
    let bins_n = (PAD * fft_size) as f64;
    let room =
        f64::from(frequencies.lower_hz) * bins_n / sample_rate - usable_bins(fft_size).0 as f64;
    let placed = if room > 0.0 { clearance_bins(1.0, bins_n) / room } else { f64::INFINITY };
    asked.max(placed).min(HANN_SPREAD * longest)
}

/// The sample of a frame of `fft_size` that the fixed listen's Gaussian is
/// centred on: [`FIXED_CENTRE`] of the way to the newest sample.
fn fixed_centre(fft_size: usize) -> usize {
    (FIXED_CENTRE * fft_size as f64).round() as usize
}

/// How many samples before the newest one a column taken from a frame of
/// `fft_size` belongs on a time axis: where its window's weight is centred.
///
/// Under a Sweep every bucket's window is centred [`SWEEP_CENTROID`] of the
/// longest listen back, counted from half a sample past the newest sample,
/// where the gamma window starts. The fixed listen sits toward the newest end
/// and the Hann window pulls its centroid back toward the middle a little,
/// which only the sum says by how much.
pub(crate) fn lag_samples(fft_size: usize, sweep: bool) -> f64 {
    if sweep {
        return sweep_centroid(fft_size) - 0.5;
    }
    let centroid = fixed_centre(fft_size) as f64 + fixed_moments(fft_size).1;
    (fft_size - 1) as f64 - centroid
}

/// The fixed listen's window — the frame's Hann window under the fixed
/// Gaussian — summed once for everything that asks about it: its sum, where its
/// weight is centred relative to the listen's centre, and its spread about that.
fn fixed_moments(fft_size: usize) -> (f64, f64, f64) {
    let centre = fixed_centre(fft_size) as f64;
    let spread = FIXED_SPREAD * fft_size as f64;
    let [sum, first, second] = (0..fft_size).fold([0.0f64; 3], |[sum, first, second], i| {
        let d = i as f64 - centre;
        let w = f64::from(crate::hann_weight(i, fft_size)) * (-0.5 * (d / spread).powi(2)).exp();
        [sum + w, first + w * d, second + w * d * d]
    });
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

/// The floats in a [`TAP_CHUNK`] of bins, real and imaginary parts interleaved.
const FLOATS: usize = 2 * TAP_CHUNK;

/// One frequency a bucket is read at, over bins `k0..k0 + taps`, from
/// `weights[at..]`, each a straight run over the spectrum's interleaved floats.
/// `taps` is padded with zero weights to a whole number of [`TAP_CHUNK`]s.
///
/// A real kernel (the fixed listen's) is one run of `2 * taps` floats, each
/// bin's sign-folded weight twice, once for its real part and once for its
/// imaginary part. A complex one (a Sweep's) is two: the first sums to the
/// reading's real part and the second to its imaginary part.
#[derive(Clone, Copy, Debug)]
struct Point {
    k0: u32,
    taps: u32,
    at: u32,
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
    /// under a fixed listen its Hann window times the share of that listen's
    /// Gaussian the kernels do not read ([`FIXED_KERNEL_SPREAD`]); under a
    /// Sweep nothing, ones, since every bucket's whole window is in its kernel.
    pub(crate) fn window(&self) -> &[f32] {
        &self.window
    }

    /// The sum of [`window`](Self::window).
    pub(crate) fn window_sum(&self) -> f32 {
        self.window_sum
    }

    /// Where the frame's oldest sample goes in the padded transform.
    ///
    /// The transform's time origin is its first sample. The fixed listen's
    /// kernels read the frame through a Gaussian centred on the transform's
    /// middle, because a shift of half the transform is a factor of `(-1)^k`
    /// on bin `k`, which is real: the weights stay real and a tap is two
    /// multiplies rather than a complex product's four. Moving the frame
    /// rather than the Gaussian is what keeps that true for a listen off the
    /// frame's middle. A Sweep's kernels are complex whatever the frame's
    /// place, and carry it in their phase, so its frame starts the transform.
    pub(crate) fn offset(&self) -> usize {
        self.offset
    }

    /// Settle every bucket's kernels, and the frame's window, listening along
    /// `sweep` or, without one, through the fixed listen.
    pub(crate) fn plan(&mut self, sweep: Option<Sweep>, sample_rate: f32, fft_size: usize) {
        self.points.clear();
        self.weights.clear();
        match sweep {
            Some(sweep) => self.plan_sweep(sweep, f64::from(sample_rate), fft_size),
            None => self.plan_fixed(f64::from(sample_rate), fft_size),
        }
    }

    /// The fixed listen: one Gaussian, the same at every pitch, its share
    /// ([`FIXED_KERNEL_SPREAD`]) read as real kernels and the rest multiplied
    /// into the frame's Hann window.
    fn plan_fixed(&mut self, sr: f64, fft_size: usize) {
        let n = fft_size as f64;
        // The padded transform's length, in which its bins are counted.
        let bins_n = (PAD * fft_size) as f64;
        let centre = fixed_centre(fft_size);
        self.offset = PAD * fft_size / 2 - centre;
        // The listen's whole Gaussian and the kernels' share of it, and the
        // share the frame carries: the Gaussian whose product with the
        // kernels' share is the whole listen.
        let (sigma, kernel_sigma) = (FIXED_SPREAD * n, FIXED_KERNEL_SPREAD * n);
        let carried = 1.0 / (1.0 / (sigma * sigma) - 1.0 / (kernel_sigma * kernel_sigma)).sqrt();
        self.window.clear();
        self.window.extend((0..fft_size).map(|i| {
            let d = (i as f64 - centre as f64) / carried;
            (f64::from(crate::hann_weight(i, fft_size)) * (-0.5 * d * d).exp()) as f32
        }));
        self.window_sum = self.window.iter().sum();
        // The spread of the window the whole Gaussian makes with the Hann
        // window, and the scale that window owes.
        let (sum, _, spread) = fixed_moments(fft_size);
        let norm = norm_power(sum);
        let kernel_std = bins_n / (2.0 * PI * kernel_sigma);
        let reach = KERNEL_REACH * kernel_std;
        // Unit sum over the WHOLE Gaussian, not the kept taps, so a kernel
        // clipped at the edge of the usable bins is not lifted.
        let scale = 1.0 / (kernel_std * (2.0 * PI).sqrt());
        for (b, frequencies) in BUCKET_FREQUENCIES.iter().enumerate() {
            let start = self.points.len() as u32;
            for at in readings(frequencies, sigma, spread, sr, fft_size) {
                let (from, kept, to) = taps(at, reach, fft_size);
                let offset = self.weights.len() as u32;
                // The listen is centred on the transform's middle (see
                // `offset`), and (-1)^k moves the transform's time origin there.
                self.weights.extend((from..=to).flat_map(|k| {
                    let d = (at - k as f64) / kernel_std;
                    let g = if kept.contains(&k) { scale * (-0.5 * d * d).exp() } else { 0.0 };
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

    /// A Sweep: every bucket reads the untapered frame through its own gamma
    /// window, so its kernel is that window's transform. With `tau` counted
    /// back from half a sample past the newest sample, `t0` in the transform,
    /// the window `tau^(K-1) e^(-tau/mu)` scaled to unit area transforms to
    /// `(1 - i omega mu)^(-K)` times the phase `e^(-i omega t0)` of where it
    /// starts, at `omega` from the reading's frequency.
    fn plan_sweep(&mut self, sweep: Sweep, sr: f64, fft_size: usize) {
        let bins_n = (PAD * fft_size) as f64;
        self.offset = 0;
        self.window.clear();
        self.window.resize(fft_size, 1.0);
        self.window_sum = fft_size as f32;
        let centroid = sweep_centroid(fft_size);
        let t0 = fft_size as f64 - 0.5;
        // Unit area, so a full-scale sine reads `1/2` at its own frequency as
        // through any window of unit sum; the transform's own `1/bins_n` is
        // folded in.
        let norm = norm_power(1.0);
        // The transform's last bin, Nyquist.
        let half = PAD * fft_size / 2;
        let mut kernel = Vec::new();
        for (b, frequencies) in BUCKET_FREQUENCIES.iter().enumerate() {
            let sigma = swept_sigma(sweep, fft_size, sr, frequencies);
            let (k, mu) = gamma_shape(sigma, centroid);
            // Where the kernel's magnitude falls to its depth, in bins.
            let reach = (SWEEP_KERNEL_DEPTH.powf(-2.0 / k) - 1.0).sqrt() / mu * bins_n / (2.0 * PI);
            let start = self.points.len() as u32;
            for at in readings(frequencies, sigma, sigma, sr, fft_size) {
                let (lo, hi) = ((at - reach).ceil() as isize, (at + reach).floor() as isize);
                // Taps below DC or above Nyquist read a bin of the real
                // frame's transform mirrored, its conjugate, so they fold onto
                // that bin rather than being cut off. A bass kernel's skirt
                // reaches below DC, where an untapered frame carries a loud
                // tone's leakage: cut off there, the bottom buckets read up to
                // 1.7e-4 of the loudest bucket's amplitude off their own
                // windows at 96 kHz, about 8 dB on a -50 dB skirt.
                let fold = |j: isize| match j {
                    j if j < 0 => (j.unsigned_abs(), true),
                    j if j as usize > half => ((2 * half).saturating_sub(j as usize), true),
                    j => (j as usize, false),
                };
                let clamp = |j: isize| j.clamp(0, half as isize) as usize;
                let (from, to) = (clamp(lo).min(fold(hi).0), clamp(hi).max(fold(lo).0));
                // Padded at the top up to Nyquist and the rest at the bottom.
                let padding = (TAP_CHUNK - (to + 1 - from) % TAP_CHUNK) % TAP_CHUNK;
                let top = (to + padding).min(half);
                let (from, to) = (from.saturating_sub(padding - (top - to)), top);
                // Per bin, the four weights the two runs give it.
                kernel.clear();
                kernel.resize(to + 1 - from, [0.0f64; 4]);
                for j in lo..=hi {
                    let omega = 2.0 * PI * (at - j as f64) / bins_n;
                    // (1 - i omega mu)^(-K) = exp(-K ln(1 - i omega mu)).
                    let x = omega * mu;
                    let magnitude = (-0.5 * k * x.mul_add(x, 1.0).ln()).exp() / bins_n;
                    let phase = k * x.atan() - omega * t0;
                    let (a, b) = (magnitude * phase.cos(), magnitude * phase.sin());
                    // A bin `x + iy` through a tap `a + ib` reads `ax - by`
                    // real and `bx + ay` imaginary; through its conjugate,
                    // `ax + by` and `bx - ay`.
                    let (bin, mirrored) = fold(j);
                    let sign = if mirrored { -1.0 } else { 1.0 };
                    let w = &mut kernel[bin - from];
                    w[0] += a;
                    w[1] -= sign * b;
                    w[2] += b;
                    w[3] += sign * a;
                }
                let offset = self.weights.len() as u32;
                self.weights.extend(kernel.iter().flat_map(|w| [w[0] as f32, w[1] as f32]));
                self.weights.extend(kernel.iter().flat_map(|w| [w[2] as f32, w[3] as f32]));
                self.points.push(Point {
                    k0: from as u32,
                    taps: (to + 1 - from) as u32,
                    at: offset,
                    complex: true,
                });
            }
            self.spans[b] = (start, self.points.len() as u32);
            self.norm[b] = norm;
        }
    }

    /// Fill `out` from the frame's padded transform. A bucket read at several
    /// frequencies takes the loudest, so its level does not grow with how many
    /// readings it took, and 0 dB keeps meaning a full-scale sine.
    pub(crate) fn read(&self, spectrum: &[Complex32], out: &mut [f32; SPECTRUM_BINS]) {
        // SAFETY: num-complex documents `Complex<T>` as memory layout
        // compatible with `[T; 2]`, real part first, so a slice of them is
        // twice as many `f32`s, real parts at even indices.
        let floats: &[f32] =
            unsafe { std::slice::from_raw_parts(spectrum.as_ptr().cast(), 2 * spectrum.len()) };
        for ((out, &(p0, p1)), norm) in out.iter_mut().zip(self.spans.iter()).zip(self.norm.iter())
        {
            let mut loudest = 0.0f32;
            for point in &self.points[p0 as usize..p1 as usize] {
                let len = 2 * point.taps as usize;
                let at = point.at as usize;
                let bins = &floats[2 * point.k0 as usize..][..len];
                // Written out: summing the lanes with `step_by` took the loop
                // in `lanes` out of vector registers and doubled the whole
                // read. The patterns stop compiling if the lane count changes.
                let (re, im) = if point.complex {
                    let [a, b, c, d, e, f, g, h] = lanes(bins, &self.weights[at..][..len]);
                    let real = (a + b) + (c + d) + (e + f) + (g + h);
                    let [a, b, c, d, e, f, g, h] = lanes(bins, &self.weights[at + len..][..len]);
                    (real, (a + b) + (c + d) + (e + f) + (g + h))
                } else {
                    // Even lanes gather the real part and odd ones the
                    // imaginary.
                    let [r0, i0, r1, i1, r2, i2, r3, i3] = lanes(bins, &self.weights[at..][..len]);
                    (r0 + r1 + r2 + r3, i0 + i1 + i2 + i3)
                };
                loudest = loudest.max(re * re + im * im);
            }
            *out = loudest * norm;
        }
    }
}

/// `bins` times `weights`, float by float, summed into eight running sums
/// rather than one, so the taps are summed in vector lanes: one sum is a chain
/// of dependent adds the compiler may not reorder. Reading the kernels is about
/// half a column's cost.
#[inline(always)]
fn lanes(bins: &[f32], weights: &[f32]) -> [f32; FLOATS] {
    let mut sums = [0.0f32; FLOATS];
    let (bins, _) = bins.as_chunks::<FLOATS>();
    let (weights, _) = weights.as_chunks::<FLOATS>();
    for (x, w) in bins.iter().zip(weights) {
        for lane in 0..FLOATS {
            sums[lane] += x[lane] * w[lane];
        }
    }
    sums
}

/// The frequencies of bucket `frequencies` a kernel is read at, in bins of the
/// padded transform of a frame of `fft_size` at `sr`: those of them clear of
/// the edges of the usable bins by the clearance a listen of spread `sigma`
/// needs ([`KERNEL_CLEARANCE`]), no further apart than half the spread of the
/// response of a window of spread `spread`.
///
/// One reading at the centre would dim a partial near the bucket's edge by up
/// to 3 dB at the top of the axis, and readings a whole spread apart still
/// dimmed one between them by 1.1 dB. At half a spread the dip is 0.27 dB.
fn readings(
    frequencies: &crate::BucketFrequencies,
    sigma: f64,
    spread: f64,
    sr: f64,
    fft_size: usize,
) -> impl Iterator<Item = f64> {
    let bins_n = (PAD * fft_size) as f64;
    let (first, last) = usable_bins(fft_size);
    let clearance = clearance_bins(sigma, bins_n);
    let response_hz = sr / (2.0 * PI * spread);
    let (lower, upper) = (f64::from(frequencies.lower_hz), f64::from(frequencies.upper_hz));
    let count = (2.0 * (upper - lower) / response_hz).ceil().max(1.0) as usize;
    (0..count)
        .map(move |r| (lower + (upper - lower) * (r as f64 + 0.5) / count as f64) * bins_n / sr)
        .filter(move |&at| at >= first as f64 + clearance && at <= last as f64 - clearance)
}

/// The bins a kernel read at bin `at` and kept to `reach` bins either side
/// spans in the padded transform of a frame of `fft_size`: `from..=to`, padded
/// to whole [`TAP_CHUNK`]s, and the bins of it inside the usable ones that are
/// given a weight.
fn taps(at: f64, reach: f64, fft_size: usize) -> (usize, std::ops::RangeInclusive<usize>, usize) {
    let (first, last) = usable_bins(fft_size);
    let k0 = ((at - reach).ceil() as usize).max(first);
    let k1 = ((at + reach).floor() as usize).min(last);
    // Padded at the top up to the transform's last bin, Nyquist, and the rest
    // at the bottom. The last usable bin is PAD below Nyquist and the first PAD
    // above DC, so between them the two ends always have room for the
    // TAP_CHUNK - 1 at most.
    let padding = (TAP_CHUNK - (k1 + 1 - k0) % TAP_CHUNK) % TAP_CHUNK;
    let to = (k1 + padding).min(PAD * fft_size / 2);
    let from = k0 - (padding - (to - k1));
    (from, k0..=k1, to)
}

/// The scale that puts a full-scale sine at 1.0 through a window whose samples
/// sum to `sum`, as a power.
///
/// A real sine of amplitude `A` transforms, through a window `w`, to
/// `|X| = A * sum(w) / 2` at its own frequency, so `(2/sum(w))^2` returns `A^2`;
/// here `w` is the bucket's whole window. That is the contract the rest of the
/// plugin rests on — 0 dB is a full-scale sine at every pitch, which is what
/// makes the Level window's ends absolute dB, what the tilt pivots against,
/// and what makes the audio ring's Gate a fixed position rather than a
/// drifting one. The padding adds nothing to the sum, being zeros.
fn norm_power(sum: f64) -> f32 {
    (4.0 / (sum * sum)) as f32
}

#[cfg(test)]
mod tests {
    use harmonigraph_core::spectrum::{hz_to_midi, BINS_PER_SEMITONE, SPECTRUM_MIN_MIDI};

    use super::*;
    use crate::{AnalyzerConfig, SpectrumAnalyzer};

    const RATE: f32 = 48_000.0;
    const FRAME: usize = SWEEP_FRAME;
    /// The fresh sweep: the longest listen the frame allows at the bottom, and
    /// half of Fast at the top.
    const FRESH: Sweep = Sweep { bass: 16_384.0, treble: 2048.0 };

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
    /// either. Every gamma window's spectrum falls away from its peak without
    /// lobes, so what could draw one is an edge: the frame's oldest sample,
    /// where the longest window's tail is a millionth of its peak, and the
    /// kernels' depth ([`SWEEP_KERNEL_DEPTH`]). Both ends at the longest, so
    /// every bucket listens that long, tones from the bottom of the axis up,
    /// at the three common rates. The loudest is -122 dB, the transform's own
    /// single-precision floor.
    ///
    /// Under the Hann window the Sweep's listens used to be centred in, the
    /// same listen drew the frame's sidelobes at -42 dB (#1592), which is what
    /// capped it at 5/8 of the frame.
    #[test]
    fn the_longest_sweep_raises_no_false_peak() {
        let longest = max_length(FRAME);
        let sweep = Some(Sweep { bass: longest, treble: longest });
        for sample_rate in [44_100.0, 48_000.0, 96_000.0] {
            let config = AnalyzerConfig { sample_rate, fft_size: FRAME, sweep };
            for hz in [24.0f32, 31.0, 38.0, 55.0, 93.0, 211.0, 440.0] {
                let level = analyze(config, hz);
                let (at, db) = (1..SPECTRUM_BINS - 1)
                    .filter(|&b| level[b] > level[b - 1] && level[b] >= level[b + 1])
                    .filter(|&b| cents_off(b, hz).abs() > 100.0)
                    .map(|b| (BUCKET_FREQUENCIES[b].center_hz, 10.0 * level[b].log10()))
                    .fold((0.0, f32::NEG_INFINITY), |a, b| if b.1 > a.1 { b } else { a });
                assert!(
                    db < -100.0,
                    "{sample_rate} Hz, {hz} Hz: a false peak at {at:.1} Hz, {db:+.1} dB"
                );
            }
        }
    }

    /// A short bass listen still reads the bass: where the listen asked for is
    /// too short to place a bucket clear of the bottom of the usable bins
    /// ([`KERNEL_CLEARANCE`]), the bucket listens as long as placing it takes.
    /// Left unread instead, a 256-sample bass listen read nothing under 175 Hz
    /// at 48 kHz, and the lowest bucket that did read drew a 60 Hz partial
    /// there, at -8 dB. At 96 kHz the bins are twice as wide, so it takes a
    /// listen twice as long: 9660 samples at 20 Hz, under the longest.
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
    /// A gamma window's half-power width is a Gaussian's of the same spread,
    /// 1.47 bins of the length against Hann's 1.44, a hair wider the more
    /// lopsided the window is: 4.3% at 24 Hz, near the longest listen, and 0.3%
    /// at 17 kHz. A gamma given the whole length as its spread rather than
    /// [`HANN_SPREAD`] of it is five times too narrow.
    ///
    /// The treble end is the shortest listen allowed, whose peak spans eight
    /// buckets at 17 kHz. A 1024-sample one there is 64 Hz wide against 31 Hz
    /// buckets, so its measured width is two buckets whatever the listen, and
    /// the same two for a sweep three times as long.
    #[test]
    fn the_listen_at_each_end_matches_a_hann_window_of_that_length() {
        let sweep = Sweep { bass: 16_384.0, treble: SWEEP_MIN_LENGTH };
        for hz in [24.0f32, 17_000.0] {
            let u = (hz / 20.0).ln() / 1000.0f32.ln();
            let length = (sweep.bass.ln() + (sweep.treble.ln() - sweep.bass.ln()) * u).exp();
            // Hann's half-power width is 1.44 bins of its own length.
            let hann = 1.44 * RATE / length;
            let width = width_hz(&analyze(config(sweep), hz));
            assert!(
                (width / hann - 1.0).abs() < 0.08,
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
    /// frame, mean removed, through that bucket's own window, evaluated
    /// directly at the bucket's own frequency. That window is the Hann window
    /// times the fixed Gaussian through a fixed frame, and under a sweep the
    /// bucket's own gamma window, scaled to unit sum, on the untapered frame.
    ///
    /// That equality is the whole of what the kernels claim, and the two sides
    /// share nothing but the window and the normalization: one convolves the
    /// padded FFT, the other sums the frame in time. The fixed frames put the
    /// Gaussian off the frame's middle, which only the frame's placement in the
    /// transform keeps real; the sweep's kernels are complex, carrying where
    /// each window sits in their phase, and its bass end is its widest, most
    /// lopsided window.
    ///
    /// The error is an amplitude, as a share of the loudest bucket's, so a
    /// quiet bucket's counts as much as a loud one's: what a bass kernel's
    /// taps below DC fold onto is the frame's leakage from the loud 31 Hz tone,
    /// which only an untapered frame has, and reads at most -50 dB. Cut off at
    /// the bottom of the bins instead of folded, or folded without being
    /// conjugated, the bottom bucket read 1.7e-4 of the loudest off at 96 kHz,
    /// against at most 6e-7 through any frame here; at 48 kHz the same cut
    /// kernel read only 1e-6 off.
    #[test]
    fn a_kernel_reads_the_frame_through_its_buckets_own_window() {
        let fresh = Some(FRESH);
        for (n, sweep, rate) in [
            (6144, None, RATE),
            (12_288, None, RATE),
            (24_576, None, RATE),
            (FRAME, fresh, RATE),
            (FRAME, fresh, 96_000.0),
        ] {
            let mut a =
                SpectrumAnalyzer::new(AnalyzerConfig { sample_rate: rate, fft_size: n, sweep });
            let input: Vec<_> = (0..n + 997)
                .map(|i| {
                    let t = i as f64 / f64::from(rate);
                    ((std::f64::consts::TAU * 440.0 * t).sin() * 0.4
                        + (std::f64::consts::TAU * 7137.0 * t).cos() * 0.2
                        + (std::f64::consts::TAU * 31.0 * t).sin() * 0.3
                        + 0.05
                        + if i % 113 == 0 { 0.3 } else { 0.0 }) as f32
                })
                .collect();
            a.push_samples(&input);
            let buckets = *a.pitch_spectrum().expect("a full window plus wrap");
            assert_eq!(a.write, 997);
            let frame: Vec<f64> = (0..n).map(|i| f64::from(a.ring[(a.write + i) % n])).collect();
            // The mean the analyzer removes, weighted by the window the frame
            // is actually multiplied by: preprocessing, not the claim.
            let frame_window = a.kernels.window();
            let mean = frame.iter().zip(frame_window).map(|(s, &w)| s * f64::from(w)).sum::<f64>()
                / frame_window.iter().map(|&w| f64::from(w)).sum::<f64>();
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
                let window: Vec<f64> = match sweep {
                    None => {
                        let (centre, sigma) = (fixed_centre(n) as f64, FIXED_SPREAD * n as f64);
                        (0..n)
                            .map(|i| {
                                let d = (i as f64 - centre) / sigma;
                                f64::from(crate::hann_weight(i, n)) * (-0.5 * d * d).exp()
                            })
                            .collect()
                    }
                    Some(sweep) => {
                        let sigma = swept_sigma(sweep, n, f64::from(rate), f);
                        let (k, mu) = gamma_shape(sigma, sweep_centroid(n));
                        // About its peak, so even a short window's power of
                        // tau stays in range.
                        let mode = (k - 1.0) * mu;
                        let gamma: Vec<f64> = (0..n)
                            .map(|i| {
                                let tau = (n - 1 - i) as f64 + 0.5;
                                ((k - 1.0) * (tau / mode).ln() - (tau - mode) / mu).exp()
                            })
                            .collect();
                        let sum: f64 = gamma.iter().sum();
                        gamma.iter().map(|w| w / sum).collect()
                    }
                };
                let (mut re, mut im) = (0.0, 0.0);
                for i in 0..n {
                    let x = (frame[i] - mean) * window[i];
                    let angle = std::f64::consts::TAU * hz * i as f64 / f64::from(rate);
                    re += x * angle.cos();
                    im -= x * angle.sin();
                }
                let expected = (re * re + im * im) * f64::from(a.kernels.norm[b]);
                let error = (f64::from(buckets[b]).sqrt() - expected.sqrt()).abs() / loudest.sqrt();
                assert!(
                    error < 1e-5,
                    "N={n}, {rate} Hz, {sweep:?}, bucket {b} at {hz:.1} Hz: scaled error {error:e}"
                );
                checked += 1;
            }
            assert!(
                checked > 20,
                "N={n}, {rate} Hz, {sweep:?}: only {checked} buckets were compared"
            );
        }
    }
}

/// Measurements behind the Sweep's numbers, run by hand with `--ignored
/// --nocapture` (release, for the timings) rather than by CI.
#[cfg(test)]
mod probes {
    use harmonigraph_core::spectrum::{hz_to_midi, BINS_PER_SEMITONE, SPECTRUM_MIN_MIDI};

    use super::*;
    use crate::{AnalyzerConfig, SpectrumAnalyzer};

    const SWEEPS: [Sweep; 3] = [
        Sweep { bass: 16_384.0, treble: 2048.0 },
        Sweep { bass: 16_384.0, treble: 256.0 },
        Sweep { bass: 8192.0, treble: 2048.0 },
    ];

    /// Through the analyzer, full-scale tones every 0.37 semitone across the
    /// whole axis: the loudest local maximum more than a semitone from the
    /// tone, how far the peak sits from it, and the lowest level it reads.
    ///
    /// Every false peak it finds is at the bottom of the axis, from 20 to 25
    /// Hz, beside a loud tone one to two octaves up, and a measured skirt
    /// rather than a numerical error: a long gamma window's skirt is heavier
    /// than a short one's, so where the sweep lengthens toward the bass the
    /// bottom buckets hear a far tone more than the buckets above them do. A
    /// sample is half as long at 96 kHz, which brings the skirt up most there.
    #[test]
    #[ignore]
    fn probe_sweep_reads() {
        for sample_rate in [44_100.0f32, 48_000.0, 96_000.0] {
            for sweep in SWEEPS {
                let config =
                    AnalyzerConfig { sample_rate, fft_size: SWEEP_FRAME, sweep: Some(sweep) };
                let mut analyzer = SpectrumAnalyzer::new(config);
                // The loudest false peak, the tone it was beside and where it was.
                let (mut false_peak, mut cents, mut lowest) = ((-999.0, 0.0, 0.0), 0.0f32, 0.0f32);
                let mut midi = SPECTRUM_MIN_MIDI + 0.3;
                while midi < 128.0 {
                    let hz = 440.0 * 2f64.powf((f64::from(midi) - 69.0) / 12.0);
                    let step = std::f64::consts::TAU * hz / f64::from(sample_rate);
                    let tone: Vec<f32> =
                        (0..SWEEP_FRAME).map(|i| (step * i as f64).sin() as f32).collect();
                    analyzer.push_samples(&tone);
                    let level = analyzer.pitch_spectrum().expect("a full frame");
                    let off = |b: usize| {
                        100.0
                            * (SPECTRUM_MIN_MIDI + (b as f32 + 0.5) / BINS_PER_SEMITONE as f32
                                - hz_to_midi(hz as f32))
                    };
                    let peak =
                        (0..SPECTRUM_BINS).max_by(|&a, &b| level[a].total_cmp(&level[b])).unwrap();
                    cents = cents.max(off(peak).abs());
                    lowest = lowest.min(10.0 * level[peak].log10());
                    for b in 1..SPECTRUM_BINS - 1 {
                        let db = 10.0 * level[b].log10();
                        if level[b] > level[b - 1]
                            && level[b] >= level[b + 1]
                            && off(b).abs() > 100.0
                            && db > false_peak.0
                        {
                            false_peak = (db, hz, BUCKET_FREQUENCIES[b].center_hz);
                        }
                    }
                    midi += 0.37;
                }
                let (db, tone, at) = false_peak;
                println!(
                    "{sample_rate} Hz, {sweep:?}: false peak {db:+.1} dB at {at:.1} Hz beside \
                     {tone:.1}, pitch {cents:.1} cents, lowest level {lowest:+.2} dB"
                );
            }
        }
    }

    /// What a column and a replan cost, per channel, against Precise.
    #[test]
    #[ignore]
    fn probe_sweep_cost() {
        let time = |config: AnalyzerConfig| {
            let mut analyzer = SpectrumAnalyzer::new(config);
            let audio: Vec<f32> = (0..config.fft_size)
                .map(|i| ((i as f32) * 0.0123).sin() * 0.3 + ((i as f32) * 0.7).sin() * 0.1)
                .collect();
            analyzer.push_samples(&audio);
            for _ in 0..20 {
                analyzer.pitch_spectrum();
            }
            let reps = 300;
            let start = std::time::Instant::now();
            for _ in 0..reps {
                std::hint::black_box(analyzer.pitch_spectrum());
            }
            let column = start.elapsed().as_secs_f64() * 1e6 / f64::from(reps);
            let start = std::time::Instant::now();
            for _ in 0..10 {
                analyzer.kernels.plan(config.sweep, config.sample_rate, config.fft_size);
            }
            let plan = start.elapsed().as_secs_f64() * 1e3 / 10.0;
            let taps: usize = analyzer
                .kernels
                .points
                .iter()
                .map(|p| p.taps as usize * if p.complex { 2 } else { 1 })
                .sum();
            println!(
                "{:?}, {} frame: {column:.0} us a column, {plan:.1} ms a plan, {taps} taps",
                config.sweep, config.fft_size
            );
        };
        time(AnalyzerConfig { sample_rate: 48_000.0, fft_size: fixed_frame(16_384), sweep: None });
        for sweep in SWEEPS {
            time(AnalyzerConfig {
                sample_rate: 48_000.0,
                fft_size: SWEEP_FRAME,
                sweep: Some(sweep),
            });
        }
    }
}
