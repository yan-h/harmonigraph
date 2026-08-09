//! Pitch spectrum analysis: map an audio signal's FFT onto the absolute
//! log-frequency (MIDI pitch) axis the Spectral pane draws, so every
//! partial displays at its actual pitch.
//!
//! Everything here is pure sample-in, buckets-out logic — no threads, no
//! clocks, no allocation after construction — so the shells can feed it
//! from wherever their audio comes from (the plugin's input bus, the
//! standalone's mock synth) and the whole pipeline stays unit-testable.
//! The FFT is a hand-rolled iterative radix-2 (the crate deliberately has
//! no dependencies), and it is not incidental work: the Spectral pane asks
//! for a column every 8 ms (`AudioSpectrum::FFT_INTERVAL`) PER CHANNEL, so a
//! stereo input at 8192 points runs 250 transforms a second — and a DAW keeps
//! that fed with silence as much as with audio, so the cost is continuous
//! rather than only while something plays. At ~0.11 ms each that is ~3% of a
//! core, which is why `fft_in_place` is written for the call rate it actually
//! sees rather than the one a spectrum analyzer sounds like it should have.

/// The spectrum's pitch axis: MIDI notes [MIN, MAX), which is 20 Hz to
/// 20 kHz — the audible band, as every analyzer states it. The axis is linear
/// in MIDI pitch, i.e. logarithmic in frequency, so every octave gets equal
/// width.
///
/// Deliberately NOT whole octaves from a C. MIDI 12..132 — ten octaves C to
/// C — would land the C labels on the axis ends, which is tidy, but it
/// stops at 16.7 kHz and leaves the top third of an octave of the audible
/// band unanalyzed. There is no C anywhere near 20 kHz (the next one is
/// 44 kHz), so covering the band means giving that tidiness up.
pub const SPECTRUM_MIN_MIDI: f32 = 15.486_82; // 20 Hz
pub const SPECTRUM_MAX_MIDI: f32 = 135.076_23; // 20 kHz
/// Axis resolution: 32 buckets per semitone (3.125 cents).
///
/// The grid the magnitude spectrum is resampled onto. It is finer than the
/// FFT resolves anywhere below ~3.2 kHz, which is deliberate: the extra rows
/// cost almost nothing and they let the axis be zoomed right in without the
/// grid itself becoming the thing you see. What the FFT can actually
/// distinguish is set by the window length, not by this.
pub const BINS_PER_SEMITONE: usize = 32;
/// Enough buckets to cover the axis, plus slack: the span is not a whole
/// number of semitones, and `pitch_spectrum` writes to `b0 + 1`, so the top
/// partial needs a bucket above the one it lands in or it would be dropped.
pub const SPECTRUM_BINS: usize =
    ((SPECTRUM_MAX_MIDI - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32) as usize + 2;

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
/// floor (20 Hz) one FFT bin spans several semitones, so the lowest
/// octave reads coarse; that is inherent to the window length, not a bug.
/// [`SpectrumAnalyzer::set_fft_size`] trades response time against bass
/// precision at runtime.
pub const DEFAULT_FFT_SIZE: usize = 8192;

/// The bin below which a pitch bucket is narrower than the FFT's bin spacing, so
/// the spectrum between the bins has to be reconstructed rather than sampled.
///
/// A fixed BIN INDEX whatever the sample rate and window length are, which is
/// what makes it a constant at all: a bucket spans a constant RATIO of its own
/// frequency (`ln 2 / (12 * BINS_PER_SEMITONE)` of it) while a bin spans a
/// constant number of Hz, so the two are equal where `f / bin_hz =
/// 12 * BINS_PER_SEMITONE / ln 2` — and `f / bin_hz` IS the bin coordinate, with
/// `bin_hz` cancelling out of both sides. At 32 buckets to the semitone that is
/// bin 554, which at 48 kHz is 3.2 kHz through an 8192-point window and 6.5 kHz
/// through a 4096-point one.
///
/// It decides only how much of the spectrum is reduced to magnitudes up front,
/// never which branch a bucket takes — that is still settled per bucket, by the
/// bins that actually fall inside it. So the slack is there to cover the buckets
/// straddling the crossover, and costs a few square roots.
const INTERP_BIN_CEILING: usize =
    ((12 * BINS_PER_SEMITONE) as f32 / std::f32::consts::LN_2) as usize + 8;

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
    /// Magnitude per bin, filled by
    /// [`pitch_spectrum`](SpectrumAnalyzer::pitch_spectrum) for the bins its
    /// reconstructing branch reads — see [`INTERP_BIN_CEILING`]. Only the low
    /// end is ever written; the rest is allocated once so the fill never has to
    /// grow it.
    bin_mag: Vec<f32>,
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
            bin_mag: Vec::new(),
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
        self.bin_mag = vec![0.0; fft_size / 2];
    }

    /// Change the analysis window length (a power of two): longer =
    /// sharper bass, slower response. A change empties the buffer.
    /// No-op at the current size, so calling every frame is fine.
    pub fn set_fft_size(&mut self, fft_size: usize) {
        if fft_size != self.fft_size {
            self.configure(fft_size);
        }
    }

    /// Seconds of audio one spectrum is measured over.
    ///
    /// A spectrum is not an instant: it is everything that happened across
    /// this much time, Hann-weighted toward the middle. Anything placing a
    /// spectrum on a time axis has to know how wide it is — see
    /// [`window_center_offset`](Self::window_center_offset).
    pub fn window_seconds(&self) -> f64 {
        f64::from(self.fft_size as u32) / f64::from(self.sample_rate)
    }

    /// How far BEFORE the newest sample the spectrum it produces belongs on a
    /// time axis: half the window.
    ///
    /// The analyzer always looks BACKWARD — `pitch_spectrum` reads the most
    /// recent `fft_size` samples — so a spectrum taken at time `t` describes
    /// `[t - window, t]`, and the Hann window weights the middle of that most.
    /// Stamping it `t` would draw every sound half a window later than it
    /// happened, which is the whole of its energy placed at the moment the
    /// LAST of it arrived.
    pub fn window_center_offset(&self) -> f64 {
        0.5 * self.window_seconds()
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
    /// Bucket values are absolute power: a full-scale sine reads ~1.0 at
    /// its pitch, so successive frames are comparable and the display can
    /// apply a fixed mapping.
    ///
    /// Every bucket is filled — this resamples the whole magnitude
    /// spectrum onto the log-pitch axis, rather than depositing only the
    /// peaks. A bucket wider than the FFT's bin spacing takes the loudest
    /// bin inside it; a narrower one (which is most of the axis: buckets
    /// and bins are equal width at [`INTERP_BIN_CEILING`], and below that
    /// the bucket is finer) reconstructs the spectrum between the bins —
    /// see [`reconstruct`]. MAX and not a sum, so the level a bucket
    /// reports doesn't grow with how many bins happen to fall in it and
    /// 0 dB keeps meaning a full-scale sine at every pitch.
    ///
    /// This replaced a peak-only fill, which deposited each local maximum
    /// at a parabolically refined frequency across the two nearest
    /// buckets. That drew every partial as a thin, exactly-placed line —
    /// but a spectrum is not only its partials. Broadband sound has no
    /// peaks to find, so noise, breath and cymbals came out as flickering
    /// speckle; there was no noise floor to read dynamics against, no
    /// spectral envelope, and quiet content vanished at a hard threshold
    /// instead of fading. The cost of the change is width: a partial is
    /// now as wide as the window's main lobe, which at C4 spans about a
    /// semitone, where before it was two buckets wide wherever it sat.
    /// The refinement had been hiding how little the FFT actually
    /// resolves down low; this shows it.
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

        // The buckets are POWER and every branch below produces `|X|^2` without
        // ever taking a root, so the normalization is squared once here instead.
        let norm_power = norm * norm;

        let bin_hz = self.sample_rate / self.fft_size as f32;
        let power = |re: &[f32], im: &[f32], k: usize| re[k] * re[k] + im[k] * im[k];
        // Usable bins: skip DC and bin 1 (where the window's own leakage
        // dominates) and stay clear of Nyquist. Anything the axis asks for
        // outside this reads as nothing, which is the truth — a 4096-point
        // window at 48 kHz cannot see 20 Hz at all.
        let (first, last) = (2usize, self.fft_size / 2 - 2);
        let half_bucket = 0.5 / BINS_PER_SEMITONE as f32;

        // Magnitudes for the reconstructing branch alone (hence
        // [`INTERP_BIN_CEILING`] rather than the whole spectrum). Once per BIN
        // and not once per bucket: below the crossover the axis is finer than
        // the FFT, by a hundredfold and more at the bottom of it, so a great
        // many buckets read the same four bins and each would otherwise pay for
        // the same four square roots.
        let mag_to = (INTERP_BIN_CEILING + 1).min(last + 1);
        for k in first..mag_to {
            self.bin_mag[k] = power(&self.re, &self.im, k).sqrt();
        }
        // The bin below the first usable one, HELD at its value rather than
        // measured. `reconstruct` reads one bin either side of the pair it
        // interpolates, so a bucket sitting on the first usable bin needs a
        // sample below it — and the two candidates are both wrong: bin 1 is
        // where the window's own leakage dominates, which is why `first` starts
        // above it, and leaving the entry unwritten reads whatever the previous
        // call left there. Holding the endpoint is the third answer, and it
        // makes the tangent there zero, which is what "the curve stops here"
        // should look like.
        if mag_to > first {
            self.bin_mag[first - 1] = self.bin_mag[first];
        }
        let bin_mag = &self.bin_mag[..mag_to];

        let mut buckets = [0.0f32; SPECTRUM_BINS];
        for (b, out) in buckets.iter_mut().enumerate() {
            let midi = SPECTRUM_MIN_MIDI + (b as f32 + 0.5) / BINS_PER_SEMITONE as f32;
            // The bucket's own frequency band, in bins.
            let x0 = midi_to_hz(midi - half_bucket) / bin_hz;
            let x1 = midi_to_hz(midi + half_bucket) / bin_hz;
            // The top is CLAMPED rather than required to be in range. A bucket
            // whose upper edge reaches past the last usable bin still CONTAINS
            // usable bins, and the loudest of those is what it means; rejecting
            // it sent it to the branch below instead, which is a bucket wider
            // than a bin asking to be read BETWEEN two of them. Only reachable
            // where the axis runs to Nyquist, so at half rates.
            let (k0, k1) = (x0.ceil(), x1.floor().min(last as f32));
            let p = if k1 >= k0 && k0 >= first as f32 {
                // Wider than the bin spacing: the loudest bin it contains.
                let (k0, k1) = (k0 as usize, k1 as usize);
                (k0..=k1).fold(0.0f32, |acc, k| acc.max(power(&self.re, &self.im, k)))
            } else {
                // Narrower: reconstruct the spectrum between the bins either
                // side of the bucket's center, so the log axis comes out smooth
                // instead of combed where it outruns the FFT.
                let x = midi_to_hz(midi) / bin_hz;
                let k = x.floor();
                // Exactly the pair being read between, and no wider: the cubic
                // wants a bin either side of that pair too, but it takes those
                // by holding the endpoint where the usable range runs out
                // rather than by demanding them. Requiring them instead put the
                // bottom of the axis outside its own analyzer — at the Fast
                // window the first two usable bins span 23 to 35 Hz, and the
                // seven semitones between them went dark.
                if k < first as f32 || k + 1.0 > last as f32 {
                    continue;
                }
                let k = k as usize;
                let m = reconstruct(bin_mag, k, x - k as f32);
                m * m
            };
            *out = p * norm_power;
        }
        Some(buckets)
    }
}

/// The spectrum's magnitude between bins `k` and `k + 1`, `t` of the way across:
/// a SHAPE-PRESERVING cubic through the four bins `k - 1 ..= k + 2` of
/// `bin_mag`.
///
/// A straight line between the two bins is the obvious form and reads visibly
/// wrong, because it is the CHORD of a curve that is convex almost everywhere
/// across a main lobe: it under-reads between every pair of bins and then snaps
/// back onto the curve at each of them. That is a facet per bin along the whole
/// stretch of the axis the FFT does not resolve — which is most of it — and it
/// is worst exactly where the axis is finest.
///
/// **Monotone tangents** (Fritsch-Carlson: the harmonic mean of the secants
/// either side, zeroed wherever they disagree in sign) are what make a cubic
/// safe here, and they are not a refinement — an ordinary cubic is unusable. A
/// main lobe's skirt drops steeply enough between adjacent bins that a form with
/// negative weights rings: a Catmull-Rom through these same four bins draws a
/// partial sitting ON a bin as a ring 1.42 dB ABOVE it with a notch at its true
/// peak, which is a shape the sound does not have. Shape-preserving tangents
/// bound every value inside the pair it sits between, so no reconstruction can
/// invent a level neither bin holds.
///
/// Continuity across the knots is the other half, and rules out the textbook
/// three-point parabola about the NEAREST bin: right for locating a peak, wrong
/// for drawing a curve, being discontinuous wherever the nearest bin changes —
/// it would trade a facet at every bin for a STEP at every half-bin. This passes
/// through the bins and matches slope across them.
///
/// **In magnitude, and that is measured rather than assumed.** dB is the
/// tempting domain, being the one the picture is drawn in and the one a main
/// lobe is nearly parabolic in, and it is indeed slightly better ACROSS THE TOP
/// of a ridge. It is catastrophic anywhere else: a windowed sinusoid's transform
/// has true zeros between bins, and a zero in dB is a pole, so at a partial
/// sitting exactly on a bin — where a Hann window nulls at every OTHER bin — the
/// reconstruction of its skirt collapses. Against the exact windowed transform,
/// over the buckets within 25 dB of the peak, RMS error in dB:
///
/// | partial's offset from a bin | 0 | 0.2 | 0.35 | 0.5 |
/// |---|---|---|---|---|
/// | straight line, in magnitude | 2.16 | 2.13 | 1.85 | 1.62 |
/// | this, in magnitude | **0.54** | **1.30** | **1.38** | **1.39** |
/// | this, in power | 2.64 | 2.25 | 1.94 | 1.84 |
/// | this, in dB | 81.63 | 1.46 | 1.22 | 1.25 |
///
/// Magnitude is the only one of the three better than a straight line at every
/// offset. `the_reconstruction_beats_a_straight_line_between_bins` is that table
/// as an assertion.
///
/// What none of them do is undo the window's scalloping loss: a partial exactly
/// between two bins reads up to 1.42 dB under its true height, because that is
/// what the bins either side of it report and nothing between them says
/// otherwise. Recovering it means reaching ABOVE the bins, which is the ringing
/// above — and 1.42 dB is 2% of the display's default range, against a ring that
/// is a feature the sound does not have.
fn reconstruct(bin_mag: &[f32], k: usize, t: f32) -> f32 {
    // The upper pair is clamped rather than guarded because the only caller
    // reaching the end of the array is a bucket at the very top of the
    // reconstructing range, where the two bins beyond it are its own neighbours
    // to within far less than the picture resolves.
    let mag = |i: usize| bin_mag[i.min(bin_mag.len() - 1)];
    let (a, b, c, d) = (mag(k - 1), mag(k), mag(k + 1), mag(k + 2));
    // Zero at a turning point, so an extremum stays where the bins put it.
    let tangent = |p: f32, q: f32| if p * q > 0.0 { 2.0 * p * q / (p + q) } else { 0.0 };
    let (m0, m1) = (tangent(b - a, c - b), tangent(c - b, d - c));
    let (t2, t3) = (t * t, t * t * t);
    debug_assert!((0.0..=1.0).contains(&t), "the cubic is only monotone on its own interval");
    // Cubic Hermite on the unit interval: value and tangent at each end.
    b * (2.0 * t3 - 3.0 * t2 + 1.0)
        + m0 * (t3 - 2.0 * t2 + t)
        + c * (3.0 * t2 - 2.0 * t3)
        + m1 * (t3 - t2)
}

/// One [`SpectrumAnalyzer`] per input channel, combined into the single
/// spectrum the display draws — see [`power_sum`](ChannelBank::power_sum).
///
/// The channels are analyzed SEPARATELY and combined afterwards, in the power
/// domain, rather than mixed to mono first. Mixing first is a sum of waveforms,
/// so anything out of phase between the channels partially or completely
/// CANCELS: a wide pad, a Haas-delayed double, a decorrelated reverb tail —
/// audible, and missing from the picture. In an analyzer whose whole job is to
/// show which pitches are sounding, a partial that vanishes because of stereo
/// phase is the worst available answer, so the mixdown is gone.
///
/// Any channel count works, one included (where this is exactly a bare
/// `SpectrumAnalyzer`), so nothing has to branch on mono vs stereo.
pub struct ChannelBank {
    per_channel: Vec<SpectrumAnalyzer>,
    /// One channel's samples, de-interleaved. Reused across pushes.
    scratch: Vec<f32>,
    sample_rate: f32,
}

impl ChannelBank {
    /// A bank for `channels` channels (at least one).
    pub fn new(sample_rate: f32, channels: usize) -> ChannelBank {
        ChannelBank {
            per_channel: (0..channels.max(1)).map(|_| SpectrumAnalyzer::new(sample_rate)).collect(),
            scratch: Vec::new(),
            sample_rate,
        }
    }

    pub fn channels(&self) -> usize {
        self.per_channel.len()
    }

    /// Match the bank to the incoming channel count. A change rebuilds the
    /// analyzers, which empties their windows — the same reset a sample-rate or
    /// window change causes, and for the same reason: samples from one layout
    /// say nothing about the next.
    pub fn set_channels(&mut self, channels: usize) {
        let channels = channels.max(1);
        if channels != self.per_channel.len() {
            *self = ChannelBank::new(self.sample_rate, channels);
        }
    }

    pub fn set_fft_size(&mut self, fft_size: usize) {
        for analyzer in &mut self.per_channel {
            analyzer.set_fft_size(fft_size);
        }
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        for analyzer in &mut self.per_channel {
            analyzer.set_sample_rate(sample_rate);
        }
    }

    /// Append INTERLEAVED frames (`channels()` samples per frame, most recent
    /// last). A partial frame at the end is ignored rather than shifting every
    /// later channel by one, which would silently swap the channels for good.
    pub fn push_frames(&mut self, interleaved: &[f32]) {
        let n = self.per_channel.len();
        if n == 1 {
            // The common case, and no de-interleaving to do.
            self.per_channel[0].push_samples(interleaved);
            return;
        }
        let frames = interleaved.len() / n;
        for (c, analyzer) in self.per_channel.iter_mut().enumerate() {
            self.scratch.clear();
            self.scratch.extend((0..frames).map(|f| interleaved[f * n + c]));
            analyzer.push_samples(&self.scratch);
        }
    }

    /// Seconds of audio one spectrum is measured over, and how far before the
    /// newest frame it belongs on a time axis. Every channel shares a window, so
    /// these are the bank's as much as any one analyzer's.
    pub fn window_center_offset(&self) -> f64 {
        self.per_channel[0].window_center_offset()
    }

    pub fn window_seconds(&self) -> f64 {
        self.per_channel[0].window_seconds()
    }

    /// The channels' MEAN POWER per bucket — the total energy at each pitch,
    /// wherever it sits in the stereo image — or None until every channel has
    /// seen a full window.
    ///
    /// Mean and not sum, so the scale the whole display rests on does not
    /// depend on the channel count: a full-scale sine centered in the image
    /// reads 0 dB, and mono input reads exactly what a single analyzer reads.
    /// A plain sum would lift everything 3 dB and put a centered full-scale
    /// sine above the top of every range bar.
    ///
    /// What this buys, deliberately: level is independent of pan. A sine
    /// panned hard left reads the same 3 dB below center that it would at any
    /// other pan position, where a mono mixdown puts it 6 dB down — and an
    /// anti-phase pair, which a mixdown erases entirely, reads at full level.
    pub fn power_sum(&mut self) -> Option<[f32; SPECTRUM_BINS]> {
        let mut total = [0.0f32; SPECTRUM_BINS];
        for analyzer in &mut self.per_channel {
            let channel = analyzer.pitch_spectrum()?;
            for (sum, p) in total.iter_mut().zip(&channel) {
                *sum += p;
            }
        }
        let gain = 1.0 / self.per_channel.len() as f32;
        for sum in &mut total {
            *sum *= gain;
        }
        Some(total)
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
        // The twiddle loop sits OUTSIDE the block loop, so a stage computes its
        // `half` sin_cos values once each instead of once per block. That is
        // 8191 calls across an n = 8192 transform against 53248 the other way
        // round, and the difference is most of what the FFT costs: the
        // butterflies themselves are a handful of multiplies, sin_cos is a
        // libm call, and this order is 2.1x faster on the transform — about
        // 1.7x through `pitch_spectrum`, which does the bucketing as well.
        //
        // The blocks within a stage are independent, so this is the same
        // arithmetic on the same values in a different order — equal bit for
        // bit, not merely within tolerance. Bits are the bar because a
        // transform that is only CLOSE moves every pixel of a render, so a
        // take rendered by two builds stops matching itself and one shot of a
        // multi-shot video no longer cuts against its siblings. The test that
        // holds this is `reusing_a_stages_twiddles_does_not_move_a_single_bit`
        // in this module, and it holds it alone: the offline determinism tests
        // render twice from ONE build and compare the runs, so they catch
        // nondeterminism and are blind to drift.
        //
        // A twiddle TABLE is faster again — another ~19% — and is the thing to
        // reach for if this ever matters more. It is bit-identical too,
        // measured across n = 2..16384: `TAU / len` is exact for a power-of-two
        // `len` and dividing by `n` is exact, so both spellings of the angle
        // round the same real value once. What it costs is state — the table
        // has to be rebuilt in `configure`, which is the one place `fft_size`
        // changes, alongside `ring`, `window`, `re` and `im`.
        for k in 0..half {
            let angle = -step * k as f32;
            let (ws, wc) = angle.sin_cos();
            for start in (0..n).step_by(len) {
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
        analyze_with(DEFAULT_FFT_SIZE, freqs_amps, sample_rate)
    }

    /// [`analyze`] at a chosen window length — the Fast and Precise settings
    /// reach parts of the axis the default one does not, because where the
    /// usable bins START is a fixed BIN and so a moving frequency.
    fn analyze_with(
        fft_size: usize,
        freqs_amps: &[(f32, f32)],
        sample_rate: f32,
    ) -> [f32; SPECTRUM_BINS] {
        let mut analyzer = SpectrumAnalyzer::new(sample_rate);
        analyzer.set_fft_size(fft_size);
        // Push in awkward chunk sizes to exercise the ring seam.
        let samples: Vec<f32> = (0..fft_size + 1234)
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
        // Absolute calibration, which the whole display scale rests on:
        // amplitude 0.8 reads as power ~0.64 (that is 0 dB for a full-scale
        // sine) in the bucket at its pitch. Checked at the PEAK, not as a
        // total: a dense spectrum samples a continuous curve onto a grid
        // finer than the FFT resolves, so summing it is meaningless.
        assert!(
            (0.5..=0.8).contains(&buckets[peak]),
            "amplitude 0.8 should read ~0.64 at its pitch, got {}",
            buckets[peak]
        );
    }

    /// The bottom of the axis draws at every window length, not just the one
    /// the rest of these tests use.
    ///
    /// Where the usable bins START is a fixed BIN (2, below which the window's
    /// own leakage dominates) and therefore a moving FREQUENCY: 23 Hz through an
    /// 8192-point window at 48 kHz, but 47 Hz through a 4096-point one, and
    /// 94 Hz at 96 kHz. So the buckets that read the first usable bins are only
    /// on the axis AT ALL at the shorter windows and the higher rates, and a
    /// bound that quietly excludes them is invisible at 8192/48 kHz — which is
    /// every other test here.
    ///
    /// The failure this pins is a band of the axis reading as silence with the
    /// tone in it: a run of buckets returning nothing while the buckets either
    /// side of them, which reach the same bins through the other branch, stay
    /// lit. Two static lines with a hole between them, and the hole does not
    /// move when the tone does.
    #[test]
    fn the_lowest_buckets_draw_at_every_window_length() {
        for (fft_size, sr) in [(4096, 48_000.0f32), (4096, 96_000.0), (8192, 96_000.0)] {
            let bin_hz = sr / fft_size as f32;
            // A tone on the first usable bin, which is where this band sits.
            let hz = 2.5 * bin_hz;
            let buckets = analyze_with(fft_size, &[(hz, 0.8)], sr);
            let at = bucket_of_midi(hz_to_midi(hz));
            let near = (at.saturating_sub(2)..=(at + 2).min(SPECTRUM_BINS - 1))
                .map(|b| buckets[b])
                .fold(0.0f32, f32::max);
            assert!(
                near > 1e-6,
                "{fft_size}-point window at {sr} Hz: a tone at {hz:.1} Hz \
                 (bin 2.5) left bucket {at} and its neighbours silent",
            );
        }
    }

    /// The reconstruction is continuous where it changes which bins it reads,
    /// and passes through them.
    ///
    /// The first rules out the textbook three-point parabola about the NEAREST
    /// bin, which is discontinuous at every half-bin and would trade the facet
    /// it was brought in to remove for a step of its own. The second is what
    /// stops the first from being satisfied by a form that agrees with itself
    /// and with nothing else.
    #[test]
    fn the_reconstruction_has_no_seam_at_a_bin() {
        // Curvature and asymmetry either side of every knot, so a form with a
        // seam in it has nowhere to hide.
        let bins: Vec<f32> = (0..12).map(|k| ((k * k * 7 % 23) as f32) * 0.1).collect();
        for k in 1..bins.len() - 3 {
            let end = reconstruct(&bins, k, 1.0);
            let start = reconstruct(&bins, k + 1, 0.0);
            assert_eq!(end, start, "a seam at bin {}", k + 1);
            assert!((end - bins[k + 1]).abs() <= 1e-6, "bin {} reads {end}", k + 1);
        }
    }

    /// The reconstruction never invents a level neither of the bins it sits
    /// between holds.
    ///
    /// The property that makes a cubic usable at all here, and the one an
    /// ordinary cubic fails: through the same four bins, a Catmull-Rom draws a
    /// partial sitting ON a bin as a ring 1.42 dB above it with a notch at its
    /// true peak. Swept over every shape three steps can make, so a form that
    /// only rings on steep ones is caught too.
    #[test]
    fn the_reconstruction_never_overshoots_the_bins_it_reads() {
        // Every shape three steps can make, up and down, gentle and cliff-edged
        // — so a form that only rings on the steep ones is caught too.
        let steps = [0.0f32, 0.001, 0.05, 0.4, 1.0];
        let signed = |v: f32, up: bool| if up { v } else { -v };
        for &p in &steps {
            for &q in &steps {
                for &r in &steps {
                    for dirs in 0..8u8 {
                        let up = |i: u8| dirs & (1 << i) == 0;
                        let mut bins = [1.0f32; 4];
                        for (i, &d) in [p, q, r].iter().enumerate() {
                            bins[i + 1] = bins[i] + signed(d, up(i as u8));
                        }
                        if bins.iter().any(|v| *v < 0.0) {
                            continue; // magnitudes only
                        }
                        let (lo, hi) = (bins[1].min(bins[2]), bins[1].max(bins[2]));
                        for i in 0..=20 {
                            let v = reconstruct(&bins, 1, i as f32 / 20.0);
                            assert!(
                                v >= lo - 1e-5 && v <= hi + 1e-5,
                                "{bins:?} reconstructed {v} outside {lo}..{hi}",
                            );
                        }
                    }
                }
            }
        }
    }

    /// The reconstruction tracks the exact windowed transform BETTER THAN A
    /// STRAIGHT LINE between the bins, at every offset a partial can sit at.
    ///
    /// This is the whole claim [`reconstruct`] makes, and the table in its docs
    /// is this measurement written out. The reference is the analyzed window's
    /// own transform evaluated at each bucket's exact frequency — the curve the
    /// FFT sampled and the reconstruction is trying to put back — rather than a
    /// second approximation of it.
    ///
    /// Judged over the buckets within `RIDGE_DB` of the peak, which is the ridge
    /// as the picture draws it. Further down, the transform dives into nulls
    /// that fall BETWEEN bins: nothing reading only the bins can reconstruct
    /// those, so every candidate is equally wrong about a stretch that is drawn
    /// as silence either way, and including it would compare noise.
    #[test]
    fn the_reconstruction_beats_a_straight_line_between_bins() {
        const RIDGE_DB: f64 = -25.0;
        let sr = 48_000.0f64;
        let n = DEFAULT_FFT_SIZE;
        let bin_hz = sr / n as f64;
        let amp = 0.8f64;
        for &off in &[0.0f64, 0.2, 0.35, 0.5] {
            let f = (75.0 + off) * bin_hz;
            let got = analyze(&[(f as f32, amp as f32)], sr as f32);
            // The exact windowed transform at any bin coordinate, over the same
            // 8192 samples the analyzer kept (`analyze` pushes 1234 past them).
            let exact = |x: f64| -> f64 {
                let (mut re, mut im) = (0.0, 0.0);
                for i in 0..n {
                    let w = 0.5 * (1.0 - (std::f64::consts::TAU * i as f64 / n as f64).cos());
                    let s = amp * (std::f64::consts::TAU * f * (i + 1234) as f64 / sr).sin();
                    let ang = -std::f64::consts::TAU * x * i as f64 / n as f64;
                    re += w * s * ang.cos();
                    im += w * s * ang.sin();
                }
                let norm = 4.0 / n as f64;
                (re * re + im * im) * norm * norm
            };
            let (mut mine, mut chord_sq, mut count) = (0.0f64, 0.0f64, 0usize);
            let peak = peak_bucket(&got);
            let lowest = peak - 40;
            for (i, &reading) in got[lowest..=peak + 40].iter().enumerate() {
                let b = lowest + i;
                let midi = SPECTRUM_MIN_MIDI + (b as f32 + 0.5) / BINS_PER_SEMITONE as f32;
                let x = midi_to_hz(midi) as f64 / bin_hz;
                let truth = exact(x);
                if 10.0 * (truth / (amp * amp)).log10() < RIDGE_DB {
                    continue;
                }
                let (k, frac) = (x.floor(), x - x.floor());
                // What a straight line between the two bins would have said.
                let chord = exact(k).sqrt() * (1.0 - frac) + exact(k + 1.0).sqrt() * frac;
                let err_db = |v: f64| (10.0 * (v / truth).log10()).powi(2);
                mine += err_db(reading as f64);
                chord_sq += err_db(chord * chord);
                count += 1;
            }
            let rms = |s: f64| (s / count as f64).sqrt();
            assert!(
                rms(mine) < rms(chord_sq),
                "at offset {off} from a bin: {:.2} dB RMS over {count} buckets, \
                 against a straight line's {:.2} dB",
                rms(mine),
                rms(chord_sq),
            );
        }
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
        // And the gap between them is a valley, not a third note. A dense
        // spectrum has skirts either side of every partial, so the test is
        // that the midpoint sits well below both peaks — not that it is
        // empty, which only a peak-picking analyzer could promise.
        let mid = buckets[(c + e) / 2];
        assert!(mid < near(c) * 0.2, "C4 and E4 are not separated: midpoint {mid}");
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
        // ~12.5 Hz sits below the axis floor (20 Hz); it must not alias to
        // some in-range bucket. (An inaudible test tone, but the guard
        // matters for subsonic rumble in real material.)
        //
        // Its skirt DOES reach the bottom of the axis, and honestly so — a
        // dense spectrum draws what the FFT saw, and the window's leakage
        // from a loud subsonic tone genuinely lands there. What must not
        // happen is a peak somewhere else, which is what wrapping would look
        // like: the axis above the very bottom stays quiet.
        let buckets = analyze(&[(12.5, 0.8)], 48_000.0);
        let above = bucket_of_midi(SPECTRUM_MIN_MIDI + 12.0);
        let stray = buckets[above..].iter().fold(0.0f32, |a, &b| a.max(b));
        assert!(stray < 0.01, "sub-axis energy appeared an octave up: {stray}");
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

    /// Feed a stereo pair through a bank and return the combined spectrum.
    fn analyze_stereo(left: impl Fn(f32) -> f32, right: impl Fn(f32) -> f32) -> [f32; SPECTRUM_BINS] {
        let sr = 48_000.0f32;
        let mut bank = ChannelBank::new(sr, 2);
        let interleaved: Vec<f32> = (0..DEFAULT_FFT_SIZE + 1234)
            .flat_map(|i| {
                let t = i as f32 / sr;
                [left(t), right(t)]
            })
            .collect();
        // Awkward chunk sizes, always whole frames, to exercise the seam.
        for chunk in interleaved.chunks(700 * 2) {
            bank.push_frames(chunk);
        }
        bank.power_sum().expect("both windows filled")
    }

    /// THE reason the channels are not mixed to mono before analysis: an
    /// anti-phase pair is loud, and a waveform sum erases it completely. A
    /// display whose job is to show which pitches are sounding cannot answer
    /// "nothing" for a tone that is plainly audible.
    #[test]
    fn an_anti_phase_pair_reads_at_full_level_where_a_mixdown_would_see_silence() {
        let a4 = 440.0;
        let sine = move |t: f32| 0.8 * (std::f32::consts::TAU * a4 * t).sin();
        let anti = analyze_stereo(sine, move |t| -sine(t));
        let peak = peak_bucket(&anti);
        assert!(
            dist(peak, bucket_of_midi(69.0)) <= 1,
            "the anti-phase tone should still peak at A4, got bucket {peak}",
        );

        // Same level as the in-phase pair: power adds, phase does not enter.
        let together = analyze_stereo(sine, sine);
        let (a, b) = (anti[peak], together[peak]);
        assert!((a - b).abs() < b * 0.05, "anti-phase read {a}, in-phase {b}");

        // And a mono mixdown of the same signal really is silence, so the test
        // above is measuring the fix and not a property the old path also had.
        let mixed = analyze(&[], 48_000.0); // silence: (sine + -sine) / 2
        assert!(mixed[peak] < b * 1e-3, "the mixdown was not silent: {}", mixed[peak]);
    }

    /// The scale every range bar and dB floor rests on must not move: a
    /// full-scale sine centered in the image still reads ~0 dB, exactly as it did
    /// when this was one analyzer over a mono mixdown. What DOES change is that
    /// level no longer depends on pan.
    #[test]
    fn the_power_sum_keeps_the_full_scale_calibration_and_drops_pan_from_it() {
        let a4 = 440.0;
        let sine = move |t: f32| 0.8 * (std::f32::consts::TAU * a4 * t).sin();
        let silent = |_: f32| 0.0;

        let centered = analyze_stereo(sine, sine);
        let peak = peak_bucket(&centered);
        assert!(
            (0.5..=0.8).contains(&centered[peak]),
            "amplitude 0.8 centered should read ~0.64 (0 dB), got {}",
            centered[peak],
        );

        // Hard left: half the energy, i.e. 3 dB down, at any pan position. The
        // old mixdown halved the AMPLITUDE instead and so read 6 dB down.
        let left = analyze_stereo(sine, silent);
        let ratio = left[peak] / centered[peak];
        assert!((ratio - 0.5).abs() < 0.05, "hard left read {ratio:.3} of centered, want 0.50");

        // A mono stream is untouched by any of this: one channel in, one
        // analyzer, the same numbers it always produced.
        let mono = analyze(&[(a4, 0.8)], 48_000.0);
        assert!(
            (mono[peak] - centered[peak]).abs() < centered[peak] * 0.01,
            "mono {} vs centered stereo {}",
            mono[peak],
            centered[peak],
        );
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

    /// [`fft_in_place`] computes each stage's twiddles once and reuses them
    /// across that stage's blocks. Sound only if it agrees with the per-block
    /// order BIT FOR BIT, which is a stricter bar than the tolerance the naive
    /// DFT test above holds: a transform that is merely *close* moves every
    /// pixel of a render, so a take rendered by two builds stops matching
    /// itself and one shot of a multi-shot video no longer cuts against its
    /// siblings.
    ///
    /// Nothing else in the tree pins that. Both offline determinism tests
    /// render twice from ONE build and compare the runs to each other, so they
    /// are invariant to any change in what the FFT computes — they catch
    /// nondeterminism, not drift. This is the test that would fail.
    ///
    /// The per-block order is kept here as the reference rather than deleted
    /// with the change, because the claim is *about* the two orders agreeing
    /// and there is otherwise nothing to compare against.
    #[test]
    fn reusing_a_stages_twiddles_does_not_move_a_single_bit() {
        /// Radix-2 with the twiddle recomputed inside the butterfly loop —
        /// the same arithmetic on the same values, with the k and start loops
        /// the other way round.
        fn per_block_twiddles(re: &mut [f32], im: &mut [f32]) {
            let n = re.len();
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

        // 2 and 4 are the degenerate stages (one block, or one twiddle); the
        // rest are every window `SpectrumWindow::samples` can ask for.
        for n in [2usize, 4, 16, 256, 4096, DEFAULT_FFT_SIZE, 16384] {
            let signal: Vec<f32> = (0..n)
                .map(|i| (i as f32 * 0.017).sin() * 0.7 + (i as f32 * 0.11).cos())
                .collect();
            let (mut ar, mut ai) = (signal.clone(), vec![0.0f32; n]);
            let (mut br, mut bi) = (signal.clone(), vec![0.0f32; n]);
            fft_in_place(&mut ar, &mut ai);
            per_block_twiddles(&mut br, &mut bi);
            // Compared as bits, not by `==`: the point is that not one ULP
            // moved, and float equality would also call two NaNs unequal.
            let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<u32>>();
            assert_eq!(bits(&ar), bits(&br), "real part differs at n = {n}");
            assert_eq!(bits(&ai), bits(&bi), "imaginary part differs at n = {n}");
        }
    }
}
