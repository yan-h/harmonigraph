//! Pitch spectrum analysis: map an audio signal's FFT onto the absolute
//! log-frequency (MIDI pitch) axis the Spectral pane draws, so every
//! partial displays at its actual pitch.
//!
//! Everything here is pure sample-in, buckets-out logic — no threads, no
//! clocks, no allocation after construction — so the shells can feed it
//! from wherever their audio comes from (the plugin's input bus, the
//! standalone's mock synth) and the whole pipeline stays unit-testable.
//! The FFT is a hand-rolled iterative radix-2 (the crate deliberately has
//! no dependencies) over a real-input packing, so a window of `n` real samples
//! costs a complex transform of `n / 2` (see [`untangle_real_power`]). It is
//! not incidental work: the Spectral pane asks for a column every 8 ms
//! (`AudioSpectrum::FFT_INTERVAL`) PER CHANNEL, so a stereo input at 8192
//! points runs 250 transforms a second — and a DAW keeps that fed with silence
//! as much as with audio, so the cost is continuous rather than only while
//! something plays. At ~0.043 ms each that is ~1.1% of a core — it was ~1.6%
//! before the transform stopped computing its twiddles, and `fft_bench`'s
//! `fft_in_place + untangle (a column's)` row is the number to re-read it off
//! rather than the bare transform's. Which is why
//! both the packing and `fft_in_place`'s twiddle handling are written for the
//! call rate this actually sees rather than the one a spectrum analyzer sounds
//! like it should have.

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

/// The most tapers an estimate averages: 8.
///
/// The ceiling is the sine family's own bandwidth, not arithmetic or memory. A
/// `count`-taper estimate resolves `(count + 1)` bins rather than the ~4 a Hann
/// window does, so at 8 the main lobe is over twice as wide as the picture is
/// drawn against — and the ring reads a 200¢ window whose bottom octave already
/// fills at one taper. Past a handful the variance a taper removes costs more
/// pitch than the reading has to give.
pub const MAX_TAPERS: usize = 8;

/// How far up the spectrum magnitudes are taken: a few bins past the crossover
/// below which a pitch bucket is narrower than the FFT's bin spacing, and so the
/// spectrum between the bins has to be reconstructed rather than sampled.
///
/// The CROSSOVER is a fixed BIN INDEX whatever the sample rate and the window
/// length are, which is what makes any of this a constant: a bucket spans a
/// constant RATIO of its own
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
    /// The tapers, precomputed: [`taper_count`](Self::tapers) of them laid end
    /// to end, each `fft_size` long. See [`build_tapers`].
    tapers: Vec<f32>,
    /// How many of them, so the flat `tapers` can be walked without dividing.
    taper_count: usize,
    /// The scale that puts a full-scale sine at 1.0, precomputed alongside the
    /// tapers it is a property of — see [`taper_norm_power`].
    norm_power: f32,
    /// FFT scratch, HALF the window long: the real window is PACKED into a
    /// complex signal of half the length, transformed at that length, and
    /// untangled back into bins — see [`untangle_real_power`].
    re: Vec<f32>,
    im: Vec<f32>,
    /// The untangle's twiddles, one per bin of the half transform. Precomputed
    /// with the buffers it is sized against, because computing them live hands
    /// the packing's whole saving back — see [`build_untangle_twiddles`].
    ///
    /// [`fft_in_place`] reads it too, at a stride per stage: the transform's
    /// twiddles are a SUBSET of these, so buying this table bought that one.
    untangle_twiddles: Vec<(f32, f32)>,
    /// Power per bin, SUMMED across the tapers by
    /// [`pitch_spectrum`](SpectrumAnalyzer::pitch_spectrum) — the one place the
    /// taper count stops being visible, since everything downstream of it reads
    /// one number per bin however many looks went into it. Half the window
    /// long: the bins above Nyquist mirror the ones below and no branch reads
    /// them.
    bin_power: Vec<f32>,
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
            tapers: Vec::new(),
            taper_count: 0,
            norm_power: 0.0,
            re: Vec::new(),
            im: Vec::new(),
            untangle_twiddles: Vec::new(),
            bin_power: Vec::new(),
            bin_mag: Vec::new(),
        };
        analyzer.configure(DEFAULT_FFT_SIZE, 1);
        analyzer
    }

    /// (Re)allocate every buffer for `fft_size` and `taper_count`, and clear
    /// the window.
    fn configure(&mut self, fft_size: usize, taper_count: usize) {
        assert!(
            fft_size >= 4 && fft_size.is_power_of_two(),
            "a power of two, four up: the packing halves it before the radix-2 FFT sees it"
        );
        let taper_count = taper_count.clamp(1, MAX_TAPERS);
        self.fft_size = fft_size;
        self.taper_count = taper_count;
        self.ring = vec![0.0; fft_size];
        self.write = 0;
        self.filled = 0;
        self.tapers = build_tapers(fft_size, taper_count);
        self.norm_power = taper_norm_power(&self.tapers, fft_size);
        self.re = vec![0.0; fft_size / 2];
        self.im = vec![0.0; fft_size / 2];
        self.untangle_twiddles = build_untangle_twiddles(fft_size);
        self.bin_power = vec![0.0; fft_size / 2];
        self.bin_mag = vec![0.0; fft_size / 2];
    }

    /// Change the analysis window length (a power of two): longer =
    /// sharper bass, slower response. A change empties the buffer.
    /// No-op at the current size, so calling every frame is fine.
    pub fn set_fft_size(&mut self, fft_size: usize) {
        if fft_size != self.fft_size {
            self.configure(fft_size, self.taper_count);
        }
    }

    /// Change how many tapers the estimate averages: more = a steadier reading
    /// of the same audio, at one FFT apiece and a wider main lobe. A change
    /// empties the buffer, exactly as a window-length change does — the tapers
    /// are what the buffer is read THROUGH, so a spectrum measured half under
    /// one set and half under another is a measurement of neither. No-op at the
    /// current count, so calling every frame is fine.
    pub fn set_tapers(&mut self, taper_count: usize) {
        if taper_count.clamp(1, MAX_TAPERS) != self.taper_count {
            self.configure(self.fft_size, taper_count);
        }
    }

    /// How many tapers the estimate currently averages.
    pub fn tapers(&self) -> usize {
        self.taper_count
    }

    /// Seconds of audio one spectrum is measured over.
    ///
    /// A spectrum is not an instant: it is everything that happened across
    /// this much time, weighted toward the middle by whatever tapers are in
    /// use. Anything placing a spectrum on a time axis has to know how wide it
    /// is — see [`window_center_offset`](Self::window_center_offset).
    pub fn window_seconds(&self) -> f64 {
        f64::from(self.fft_size as u32) / f64::from(self.sample_rate)
    }

    /// How far BEFORE the newest sample the spectrum it produces belongs on a
    /// time axis: half the window.
    ///
    /// The analyzer always looks BACKWARD — `pitch_spectrum` reads the most
    /// recent `fft_size` samples — so a spectrum taken at time `t` describes
    /// `[t - window, t]`, and the tapers weight the middle of that most.
    /// Stamping it `t` would draw every sound half a window later than it
    /// happened, which is the whole of its energy placed at the moment the
    /// LAST of it arrived.
    ///
    /// HALF the window at every taper count, because what decides the centroid
    /// is that the set is SYMMETRIC and not what shape it has: one Hann window
    /// and a sum of sine tapers are different weightings, and both are their
    /// own mirror about the middle.
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

        // One transform per taper, summed into `bin_power`. The tapers are
        // independent LOOKS at one window of audio rather than more audio, so
        // what this loop buys is a steadier reading of the same 171 ms and not
        // a longer one; `build_tapers` carries why that is worth an FFT.
        //
        // The sum stays a sum — the mean's divisor is folded into
        // `taper_norm_power` instead, so no pass over the bins exists only to
        // divide by a constant.
        self.bin_power.fill(0.0);
        // Usable bins: skip DC and bin 1 (where the window's own leakage
        // dominates) and stay clear of Nyquist. Anything the axis asks for
        // outside this reads as nothing, which is the truth — a 4096-point
        // window at 48 kHz cannot see 20 Hz at all. It is also the only range
        // the untangle below fills, so a widening here is work as well as axis.
        let half = self.fft_size / 2;
        let (first, last) = (2usize, half - 2);
        for k in 0..self.taper_count {
            let taper = k * self.fft_size;
            // Unroll the ring into time order, tapered, and PACKED: the even
            // samples become the real part of a half-length complex signal and
            // the odd ones its imaginary part, so a real window of `fft_size`
            // costs a complex transform of `half` plus an O(n) untangle.
            for j in 0..half {
                let even = (self.write + 2 * j) % self.fft_size;
                let odd = (self.write + 2 * j + 1) % self.fft_size;
                self.re[j] = self.ring[even] * self.tapers[taper + 2 * j];
                self.im[j] = self.ring[odd] * self.tapers[taper + 2 * j + 1];
            }
            fft_in_place(&mut self.re, &mut self.im, &self.untangle_twiddles);
            untangle_real_power(
                &self.re,
                &self.im,
                &self.untangle_twiddles,
                first,
                last,
                &mut self.bin_power,
            );
        }

        // Amplitude normalization so a unit sine reads as ~1.0, precomputed
        // with the tapers it is a property of (`taper_norm_power`). The buckets
        // are POWER and every branch below produces `|X|^2` without ever taking
        // a root, so it is squared there rather than here.
        let norm_power = self.norm_power;

        let bin_hz = self.sample_rate / self.fft_size as f32;
        let half_bucket = 0.5 / BINS_PER_SEMITONE as f32;

        // Magnitudes for the reconstructing branch alone (hence
        // [`INTERP_BIN_CEILING`] rather than the whole spectrum). Once per BIN
        // and not once per bucket: below the crossover the axis is finer than
        // the FFT, by a hundredfold and more at the bottom of it, so a great
        // many buckets read the same four bins and each would otherwise pay for
        // the same four square roots.
        let mag_to = (INTERP_BIN_CEILING + 1).min(last + 1);
        for k in first..mag_to {
            self.bin_mag[k] = self.bin_power[k].sqrt();
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
                (k0..=k1).fold(0.0f32, |acc, k| acc.max(self.bin_power[k]))
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
/// safe here, and they are not a refinement — an ordinary cubic is unusable.
/// The transform's NULLS are why. A partial sitting exactly on a bin puts a
/// Hann window's zeros on every other bin around it, so runs of three zero bins
/// beside a nonzero one are the ordinary case rather than a contrived one — and
/// a Catmull-Rom through `(0, 0, 0, x)` reaches `-2x/27` at a third of the way
/// across. That is a NEGATIVE magnitude, which the caller squares into a bright
/// phantom sitting exactly where the transform is silent.
///
/// Shape-preserving tangents bound every value inside the pair it sits between,
/// so no reconstruction can invent a level neither bin holds — and in particular
/// none can invent one below zero, which is the case a magnitude cannot survive.
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
    // Reads past the top HOLD the last magnitude, which is the same boundary
    // the caller writes below its first usable bin — the four-point form wants
    // an outer sample the measured range does not have at either end, and a
    // held endpoint gives it one with a zero tangent, which is what "the curve
    // stops here" should look like. Insurance at THIS end rather than a live
    // path: the caller's own bound keeps `k + 2` inside the filled range at
    // every supported rate and window length, with bins to spare.
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

    /// How many tapers every channel's estimate averages — see
    /// [`SpectrumAnalyzer::set_tapers`]. One setting for the bank, because the
    /// channels are combined per bucket by [`power_sum`](Self::power_sum) and
    /// two channels measured through different estimators do not add up to a
    /// reading of anything.
    pub fn set_tapers(&mut self, taper_count: usize) {
        for analyzer in &mut self.per_channel {
            analyzer.set_tapers(taper_count);
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

/// The `count` tapers for a window of `n` samples, laid end to end.
///
/// ## Why more than one window
///
/// A spectrum measured through ONE window is a chi-squared estimate on two
/// degrees of freedom, whose standard deviation equals its mean — ±5.6 dB on
/// every bucket of every column, forever. Lengthening the window does not touch
/// that; it trades time resolution for frequency resolution and leaves the
/// variance where it is. Averaging columns barely touches it either, because at
/// an 8 ms hop through a 171 ms window consecutive columns are 95% the same
/// audio, so there is almost nothing independent to average until the filter is
/// longer than the window.
///
/// Averaging over ORTHOGONAL tapers is the way to get independent looks at one
/// window of audio. `count` of them give ~`2 * count` degrees of freedom, so the
/// noise on a bucket falls as `1/sqrt(count)`: 5.6 dB at one taper, 2.7 dB at
/// three. That is the speckle, and it is bought with FFTs rather than with
/// latency, which is what separates this from a smoothing filter.
///
/// ## One taper is Hann; more than one is the sine family
///
/// The two settings are two whole estimators rather than a family with Hann at
/// its head, because the variance reduction rests on the tapers being mutually
/// orthogonal and Hann is orthogonal to none of the sine tapers. Keeping Hann
/// at `count == 1` is what makes the single-taper picture the one this analyzer
/// draws with no toggle at all — the same window, and (see [`taper_norm_power`])
/// the same arithmetic under it.
///
/// The sine (Riedel-Sidorenko) tapers are the closed form
/// `sqrt(2/(n+1)) * sin(pi * (k+1) * (i+1) / (n+1))`, which is why they are here
/// rather than the DPSS/Slepian set the literature leads with: DPSS needs an
/// eigenproblem solved per window length, these need a sine, and the variance
/// they remove is within a few percent of each other at the counts worth
/// offering. They are unit-energy by construction, so no taper is louder than
/// another.
fn build_tapers(n: usize, count: usize) -> Vec<f32> {
    if count <= 1 {
        return (0..n)
            .map(|i| {
                let phase = std::f32::consts::TAU * i as f32 / n as f32;
                0.5 * (1.0 - phase.cos())
            })
            .collect();
    }
    let scale = (2.0 / (n as f32 + 1.0)).sqrt();
    let mut tapers = Vec::with_capacity(count * n);
    for k in 0..count {
        let order = (k + 1) as f32;
        tapers.extend((0..n).map(|i| {
            let phase = std::f32::consts::PI * order * (i as f32 + 1.0) / (n as f32 + 1.0);
            scale * phase.sin()
        }));
    }
    tapers
}

/// The scale that puts a full-scale sine at 1.0, for `tapers` of `n` samples
/// each — applied to the power SUMMED over them, not to the mean.
///
/// A real sine of amplitude `A` at a bin centre transforms, through a taper `w`,
/// to `|X| = A * sum(w) / 2`. So the summed power over the tapers is
/// `(A/2)^2 * sum_k (sum_n w_k)^2`, and dividing that into `4` returns `A^2`.
///
/// **`2/sum(w)` does not generalize, and the reason is worth stating**: the
/// sine tapers of even order are odd-symmetric and sum to ~0, so a
/// per-taper amplitude normalization divides by nothing on half of them. The
/// sum of squared sums is the same quantity read over the whole set, and it is
/// finite for every count.
///
/// What this holds is the contract the rest of the plugin rests on — 0 dB is a
/// full-scale sine at every pitch, which is what makes the Level window's ends
/// absolute dB, what the tilt pivots against, and what makes the audio ring's
/// Gate a fixed position rather than a drifting one. What it does NOT hold is
/// the NOISE FLOOR, which reads higher as tapers are added (4.72 dB by three):
/// a line spread across a wider main lobe has to be scaled back up to reach 1.0,
/// and flat noise comes up with it. That is a real cost in contrast, it is the
/// estimator's and not this function's, and
/// `the_noise_floor_reads_higher_as_tapers_are_added` measures it.
///
/// The single-taper case is written as `(2/sum)^2` rather than as `4/sum^2` so
/// that it is bit-identical to a Hann analyzer and not merely equal to one:
/// the two orders round differently in f32, and the toggle's off position is
/// worth having cost exactly nothing.
fn taper_norm_power(tapers: &[f32], n: usize) -> f32 {
    if tapers.len() <= n {
        let sum: f32 = tapers.iter().sum();
        let norm = 2.0 / sum;
        return norm * norm;
    }
    let response: f32 = tapers
        .chunks(n)
        .map(|taper| {
            let sum: f32 = taper.iter().sum();
            sum * sum
        })
        .sum();
    4.0 / response
}

/// The twiddles [`untangle_real_power`] reads: `e^(-i * tau * k / n)` for every
/// bin `k` of the half transform, where `n` is the REAL window length.
///
/// [`fft_in_place`] reads the SAME table for its stage twiddles, at a stride
/// per stage — the transform runs at `n / 2`, and its angles are every second
/// entry of this one, subsampled again for each earlier stage. So the table is
/// built for the untangle and the transform gets its own for nothing; the
/// derivation is on `fft_in_place`.
///
/// A TABLE rather than a `sin_cos` per bin, which is what makes the packing pay
/// for itself: the untangle visits `n/2 - 3` bins, so an angle computed at each
/// one would put back all of the trig the packing had just saved. That was the
/// whole of the argument when the transform still computed its own angles
/// beside this; now that it reads these, the table is paid for once and cashed
/// TWICE — once by the untangle it was built for, and once by a transform that
/// computes no angles at all. What it costs is state, one table per window
/// length, built where every other buffer sized on `fft_size` is.
///
/// `(sin, cos)` in that order, which is `f32::sin_cos`'s own, so nothing has to
/// reorder the pair between building an entry here and destructuring one in
/// [`untangle_real_power`] or [`fft_in_place`] — the two readers of this table
/// and, since the transform's `sin_cos` went away, the only places the order
/// is visible at all.
fn build_untangle_twiddles(n: usize) -> Vec<(f32, f32)> {
    (0..n / 2)
        .map(|k| {
            let angle = -std::f32::consts::TAU * k as f32 / n as f32;
            angle.sin_cos()
        })
        .collect()
}

/// The power of bins `first..=last` of the transform of `2 * re.len()` REAL
/// samples, ADDED to `power`: the untangle half of the real-input packing.
///
/// `re`/`im` hold the half-length complex transform `Z` of the packed signal
/// (even samples real, odd samples imaginary). `Z[k]` and `Z[half - k]` between
/// them carry the transforms of the even- and odd-indexed subsequences, whose
/// conjugate-symmetric and antisymmetric parts are those two subsequences'
/// spectra `E[k]` and `O[k]`; one twiddle recombines them into the real
/// spectrum's bin, `X[k] = E[k] + W^k O[k]`. The half each of those carries is
/// factored out to the `0.25` on the squared magnitude below — one exact
/// multiply on a value being scaled anyway, rather than four on the parts.
///
/// **`1 <= first` and `last < half` is the range the form is VALID on**, not a
/// convenience. DC and Nyquist are the two bins whose conjugate partner is
/// themselves: they fold together into one entry of `Z`, come out as
/// `Re Z[0] + Im Z[0]` and `Re Z[0] - Im Z[0]`, and reach neither the twiddle
/// nor an index of their own. `pitch_spectrum` reads neither — see the
/// `(first, last)` it passes — so nothing here computes them, and the loop
/// stays a loop.
fn untangle_real_power(
    re: &[f32],
    im: &[f32],
    twiddles: &[(f32, f32)],
    first: usize,
    last: usize,
    power: &mut [f32],
) {
    let half = re.len();
    debug_assert!(im.len() == half && twiddles.len() == half);
    debug_assert!(first >= 1 && last < half, "DC and Nyquist are not this form's to compute");
    for k in first..=last {
        let (ws, wc) = twiddles[k];
        let (ar, ai) = (re[k], im[k]);
        let (cr, ci) = (re[half - k], im[half - k]);
        // `2E[k]` and `2i * O[k]`, which is the pair with its partner's
        // conjugate added and subtracted.
        let (sr, si) = (ar + cr, ai - ci);
        let (dr, di) = (ar - cr, ai + ci);
        let xr = sr + wc * di + ws * dr;
        let xi = si + ws * di - wc * dr;
        power[k] += 0.25 * (xr * xr + xi * xi);
    }
}

/// Iterative radix-2 Cooley-Tukey, in place. Lengths are compile-time
/// powers of two here; debug_assert documents the requirement.
///
/// `twiddles` is [`build_untangle_twiddles`]`(2 * re.len())` — the very table
/// the analyzer already holds for its untangle, passed in rather than rebuilt,
/// because at the length a column transforms the two are the SAME SET. The
/// untangle's table is `e^(-i * tau * t / 2n)` for `t` in `0..n`; a stage at
/// length `len` wants `e^(-i * tau * k / len)` for `k` in `0..len / 2`, and
/// `k / len` is `(k * n / (len / 2)) / 2n`, so the stage's twiddle is entry
/// `k * stride` at `stride = n / (len / 2)`. Every stage is a subsampling of
/// the last one, and the last one is every second entry of the untangle's.
///
/// So the transform's twiddles cost no state at all here: the table is built
/// once in `configure`, where `fft_size` changes, for a pass that needed it
/// anyway. See [`build_untangle_twiddles`] for the table itself.
fn fft_in_place(re: &mut [f32], im: &mut [f32], twiddles: &[(f32, f32)]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two() && im.len() == n);
    debug_assert!(
        twiddles.len() == n,
        "one entry per point of the transform: `build_untangle_twiddles(2 * n)`"
    );

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
        let half = len / 2;
        // The table read replaced a `sin_cos` per (stage, `k`), which was most
        // of what this cost — the butterflies are a handful of multiplies and
        // sin_cos is a libm call — and it is 1.26x on the transform at n = 4096
        // (0.0503 ms to 0.0399 ms, `examples/fft_bench.rs`).
        //
        // **The `k` loop stays OUTSIDE the block loop, and that is measured
        // rather than left over.** It was put there to amortize the libm call
        // over a stage's blocks, so the obvious follow-through is to undo the
        // hoist now that there is no call to amortize and let each block walk
        // `re`/`im` in order. That is 6% SLOWER (0.0423 ms against 0.0399 ms):
        // the twiddle stays in a register across the whole inner loop this way,
        // where block-major reloads it per butterfly, and that beats the
        // locality it gives up. `fft, table, k inside the blocks` in the bench
        // is the row, kept so the next reader does not have to re-derive it.
        //
        // The result is bit-identical to BOTH spellings it replaced, which is
        // the bar and not a nicety: a transform that is only CLOSE moves every
        // pixel of a render, so a take rendered by two builds stops matching
        // itself and one shot of a multi-shot video no longer cuts against its
        // siblings. Two things make it exact. The blocks and the `k`s of one
        // stage touch disjoint pairs, so any order over them is the same
        // arithmetic on the same values. And the angle rounds once to the same
        // f32 either way: `TAU / len` is exact for a power-of-two `len`, so the
        // inline form was `round(TAU * k / len)`, while the table rounds
        // `TAU * t` and then divides by `2n`, which is exact because it is a
        // power of two — and `t = k * stride` is chosen to make `t / 2n` the
        // same real number as `k / len`.
        //
        // Equal angles are NOT the whole of it, and the missing half is worth
        // writing down because it would read as a mystery. Two spellings also
        // have to be EVALUATED the same way: with a compile-time-constant
        // operand LLVM folds the call at build time, and that fold is not
        // obliged to match the libm the same call reaches at runtime. Measured
        // on the pinned toolchain (1.92, aarch64-apple-darwin, where
        // `f32::sin_cos` lowers to one `___sincosf_stret`): over the 2048
        // distinct angles of the `k / 4096` grid, given as literals so they
        // certainly fold, exactly one comes out 1 ULP apart in `cos` from the
        // runtime call — `-TAU * 49/512`, at opt-level 2 and 3 alike.
        //
        // The transform is immune by construction, and more so than before:
        // there is now exactly ONE `sin_cos` in this file outside the tests,
        // in `build_untangle_twiddles`, and `configure` drives it with a
        // runtime `fft_size`, so its operand is never a constant and the
        // untangle and the transform can no longer disagree about an angle
        // they both read from one table.
        //
        // Where the exposure sits is the TEST, which deliberately puts a
        // foldable evaluation next to a called one, and `[profile.dev]
        // opt-level = 2` means `cargo test` is optimized. It is clean today,
        // and not by luck: the sweep entries small enough for LLVM to unroll
        // (`n` = 2, 4, 16, so tables of 4, 8 and 32) hold no divergent angle,
        // while the first one that diverges needs a table of 512. The two
        // conditions do not currently overlap. A compiler bump could move
        // either, so if that test ever goes red by a single ULP, this is what
        // it is before anything else is suspected.
        //
        // `reusing_a_stages_twiddles_does_not_move_a_single_bit` in this module
        // holds that, and holds it alone: the offline determinism tests render
        // twice from ONE build and compare the runs, so they catch
        // nondeterminism and are blind to drift.
        let stride = n / half;
        for k in 0..half {
            let (ws, wc) = twiddles[k * stride];
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
                freqs_amps.iter().map(|(f, a)| a * (std::f32::consts::TAU * f * t).sin()).sum()
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
    /// between holds — and in particular never a negative one.
    ///
    /// The property that makes a cubic usable at all here, and the one an
    /// ordinary cubic fails. The named case below is the one that matters: a
    /// partial sitting exactly ON a bin puts a Hann window's zeros on every
    /// other bin around it, and a Catmull-Rom through three of those and a
    /// fourth nonzero bin goes NEGATIVE — a magnitude the caller squares into a
    /// phantom where the transform is silent. The sweep after it covers every
    /// shape three steps can make, so a form that only misbehaves on the steep
    /// ones is caught too.
    #[test]
    fn the_reconstruction_never_overshoots_the_bins_it_reads() {
        // A null beside a rise: `(0, 0, 0, x)`. A Catmull-Rom reaches -2x/27
        // here, a third of the way across; anything shape-preserving is pinned
        // to the zeros it sits between.
        for x in [0.05f32, 0.4, 1.0] {
            for i in 0..=20 {
                let v = reconstruct(&[0.0, 0.0, 0.0, x], 1, i as f32 / 20.0);
                assert_eq!(v, 0.0, "a null beside a rise of {x} reconstructed {v}");
            }
        }

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
            // A MARGIN, not just "better". This is the only test that fails if
            // `reconstruct` is reverted to a straight line — the continuity and
            // overshoot tests both pass for one, a line being continuous through
            // its knots and bounded by them. Reverted, the two quantities here
            // become the same one computed twice (f32 FFT against f64 direct
            // transform, agreeing to about 1e-6), and a bare `<` would then be
            // decided by rounding noise. The worst measured ratio is 0.86.
            assert!(
                rms(mine) < 0.9 * rms(chord_sq),
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
    fn analyze_stereo(
        left: impl Fn(f32) -> f32,
        right: impl Fn(f32) -> f32,
    ) -> [f32; SPECTRUM_BINS] {
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

    /// The packing computes the same spectrum a DFT of the whole real signal
    /// does, over every bin it is defined for.
    ///
    /// Through the path `pitch_spectrum` runs and not the transform alone —
    /// pack, half-length FFT, untangle — because the packing is where an error
    /// this test can see would be: a wrong twiddle denominator, a partner read
    /// from the wrong end, a conjugate the wrong way round. In POWER, which is
    /// all [`untangle_real_power`] produces and all the analyzer ever reads.
    ///
    /// The bins are `1..=half - 1`, which is the whole of the untangle's
    /// domain: its two ends read their conjugate partner from the far end of
    /// the half transform, and the middle one (`n/4`) is its own partner, so a
    /// fixture this size reaches both edge cases and the self-conjugate one.
    /// DC and Nyquist are outside the form and outside what the analyzer
    /// reads — see [`untangle_real_power`].
    #[test]
    fn the_packed_real_transform_matches_a_naive_dft_on_a_small_case() {
        let n = 16;
        let half = n / 2;
        let signal: Vec<f32> =
            (0..n).map(|i| (i as f32 * 0.7).sin() + 0.3 * (i as f32 * 2.1).cos()).collect();
        let mut re: Vec<f32> = signal.iter().step_by(2).copied().collect();
        let mut im: Vec<f32> = signal.iter().skip(1).step_by(2).copied().collect();
        let twiddles = build_untangle_twiddles(n);
        fft_in_place(&mut re, &mut im, &twiddles);
        let mut power = vec![0.0f32; half];
        untangle_real_power(&re, &im, &twiddles, 1, half - 1, &mut power);
        for (k, &p) in power.iter().enumerate().skip(1) {
            let (mut dr, mut di) = (0.0f64, 0.0f64);
            for (i, &s) in signal.iter().enumerate() {
                let angle = -std::f64::consts::TAU * (k * i) as f64 / n as f64;
                dr += f64::from(s) * angle.cos();
                di += f64::from(s) * angle.sin();
            }
            let dft = dr * dr + di * di;
            assert!(
                (f64::from(p) - dft).abs() < 1e-3,
                "bin {k}: packed power {p} vs dft power {dft}"
            );
        }
    }

    /// [`fft_in_place`] reuses one twiddle across a stage's blocks, and takes
    /// it from a strided read of the untangle's table rather than a `sin_cos`.
    /// Sound only if it agrees with the spellings it replaced BIT FOR BIT,
    /// which is a stricter bar than the tolerance the naive DFT test above
    /// holds: a transform that is merely *close* moves every pixel of a
    /// render, so a take rendered by two builds stops matching itself and one
    /// shot of a multi-shot video no longer cuts against its siblings.
    ///
    /// Nothing else in the tree pins that. Both offline determinism tests
    /// render twice from ONE build and compare the runs to each other, so they
    /// are invariant to any change in what the FFT computes — they catch
    /// nondeterminism, not drift. This is the test that would fail.
    ///
    /// Two references, because there are two claims and each needs its own.
    /// [`per_block_twiddles`] recomputes the angle inside the butterfly loop,
    /// so against it the table's INDEXING is what is under test — a stride off
    /// by a factor of two reads a real twiddle from the wrong stage and shows
    /// here rather than as a rounding difference. [`hoisted_twiddles`] is the
    /// shape this shipped before the table, with `k` outside the block loop, so
    /// against it the loop ORDER is what is under test. Both are kept rather
    /// than deleted with the change, because the claim is about the three
    /// agreeing and there is otherwise nothing to compare against.
    ///
    /// The sweep runs on TRANSFORM lengths, not window lengths — `n` here is
    /// what `fft_in_place` is handed, which is half the real window, so the
    /// table each entry builds is `build_untangle_twiddles(2 * n)`.
    ///
    /// Its ends are the degenerate stages. At `n = 2` the transform is one
    /// stage that is a single block AND a single twiddle, and its stride
    /// (`n / half` = 2) is never multiplied by a nonzero `k`, so that entry
    /// cannot catch a stride error at all — it checks the table is long enough
    /// and the indexing does not panic. `n = 4` is the smallest length where a
    /// stride is actually exercised, at `k = 1` of the last stage.
    ///
    /// The middle covers every length the UI can reach: `SpectrumWindow`
    /// offers 4096, 8192 and 16384 samples, which transform at 2048, 4096 and
    /// 8192, and all three are here. `16384` is one step past what the enum
    /// offers (it would be a 32768-sample window), kept as headroom for a
    /// fourth entry rather than as a claim that something asks for it.
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

        /// The shape this shipped before the table: one `sin_cos` per (stage,
        /// `k`), hoisted out of the block loop so a stage paid for its angles
        /// once rather than once per block.
        fn hoisted_twiddles(re: &mut [f32], im: &mut [f32]) {
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

        for n in [2usize, 4, 16, 256, 2048, 4096, DEFAULT_FFT_SIZE, 16384] {
            let signal: Vec<f32> =
                (0..n).map(|i| (i as f32 * 0.017).sin() * 0.7 + (i as f32 * 0.11).cos()).collect();
            let (mut ar, mut ai) = (signal.clone(), vec![0.0f32; n]);
            let (mut br, mut bi) = (signal.clone(), vec![0.0f32; n]);
            let (mut cr, mut ci) = (signal.clone(), vec![0.0f32; n]);
            // The table the analyzer holds for a real window of `2 * n`, which
            // is the window whose transform runs at `n`.
            let twiddles = build_untangle_twiddles(2 * n);
            fft_in_place(&mut ar, &mut ai, &twiddles);
            per_block_twiddles(&mut br, &mut bi);
            hoisted_twiddles(&mut cr, &mut ci);
            // Compared as bits, not by `==`: the point is that not one ULP
            // moved, and float equality would also call two NaNs unequal.
            let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<u32>>();
            assert_eq!(bits(&ar), bits(&br), "real part differs at n = {n}");
            assert_eq!(bits(&ai), bits(&bi), "imaginary part differs at n = {n}");
            assert_eq!(bits(&ar), bits(&cr), "real part differs from the hoisted order at n = {n}");
            assert_eq!(
                bits(&ai),
                bits(&ci),
                "imaginary part differs from the hoisted order at n = {n}"
            );
        }
    }

    // ---- Tapers -----------------------------------------------------------

    /// Deterministic white noise. These tests measure the VARIANCE of an
    /// estimate, so the samples have to be random and the run has to repeat
    /// exactly — a flaky variance test is worse than none, since the number it
    /// is asserting about is the one that moves.
    struct Noise(u32);

    impl Noise {
        fn sample(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
        }
    }

    fn mean(values: &[f32]) -> f32 {
        values.iter().sum::<f32>() / values.len() as f32
    }

    fn std_dev(values: &[f32]) -> f32 {
        let m = mean(values);
        (values.iter().map(|v| (v - m) * (v - m)).sum::<f32>() / values.len() as f32).sqrt()
    }

    /// One bucket's level in dB across `trials` INDEPENDENT windows of white
    /// noise. Independent because each push replaces the window whole, which is
    /// the one condition under which the spread below is the estimator's own
    /// and not the overlap's.
    fn noise_levels(tapers: usize, trials: usize, bucket: usize) -> Vec<f32> {
        let sample_rate = 48_000.0f32;
        let window = 1024;
        let mut analyzer = SpectrumAnalyzer::new(sample_rate);
        analyzer.set_fft_size(window);
        analyzer.set_tapers(tapers);
        let mut noise = Noise(0x1234_5678);
        (0..trials)
            .map(|_| {
                let samples: Vec<f32> = (0..window).map(|_| noise.sample()).collect();
                analyzer.push_samples(&samples);
                let buckets = analyzer.pitch_spectrum().expect("a whole window went in");
                10.0 * buckets[bucket].max(1e-12).log10()
            })
            .collect()
    }

    /// A bucket in the middle of the axis at the 1024-point window: 2 kHz is
    /// well above the first usable bin and well under Nyquist at every rate the
    /// tests use.
    fn mid_axis_bucket() -> usize {
        bucket_of_midi(hz_to_midi(2000.0))
    }

    /// The toggle's off position is the analyzer with no toggle in it — same
    /// window, same arithmetic, not merely the same answer to three decimals.
    #[test]
    fn one_taper_is_the_hann_window_and_the_arithmetic_under_it() {
        let n = 256;
        let tapers = build_tapers(n, 1);
        assert_eq!(tapers.len(), n, "one taper is one window long");
        for (i, w) in tapers.iter().enumerate() {
            let phase = std::f32::consts::TAU * i as f32 / n as f32;
            assert_eq!(w.to_bits(), (0.5 * (1.0 - phase.cos())).to_bits(), "taper[{i}]");
        }
        // `(2/sum)^2` and `4/sum^2` are the same algebra and different f32, and
        // this is the one that keeps a single-taper render matching a build
        // with no tapers in it at all.
        let sum: f32 = tapers.iter().sum();
        let norm = 2.0 / sum;
        assert_eq!(taper_norm_power(&tapers, n).to_bits(), (norm * norm).to_bits());
    }

    /// The contract every absolute dB in the plugin rests on: 0 dB is a
    /// full-scale sine, whatever the estimator underneath. If this drifts, the
    /// Level window's ends stop meaning dB, the tilt pivots against nothing,
    /// and the audio ring's Gate quietly selects a different set of nodes.
    #[test]
    fn a_full_scale_sine_reads_unity_at_every_taper_count() {
        for tapers in 1..=MAX_TAPERS {
            let sample_rate = 48_000.0f32;
            let mut analyzer = SpectrumAnalyzer::new(sample_rate);
            analyzer.set_tapers(tapers);
            let samples: Vec<f32> = (0..DEFAULT_FFT_SIZE + 1234)
                .map(|i| {
                    let t = i as f32 / sample_rate;
                    (std::f32::consts::TAU * 2000.0 * t).sin()
                })
                .collect();
            for chunk in samples.chunks(701) {
                analyzer.push_samples(chunk);
            }
            let buckets = analyzer.pitch_spectrum().expect("window filled");
            let peak = buckets[peak_bucket(&buckets)];
            let db = 10.0 * peak.max(1e-12).log10();
            assert!(db.abs() < 1.5, "{tapers} tapers read a full-scale sine at {db:.2} dB, not 0");
        }
    }

    /// The whole point: more tapers, a steadier reading of the same audio.
    /// Theory says the spread falls as `1/sqrt(count)` — 5.6 dB at one taper,
    /// 2.7 dB at three — and the bar is set loose of that because what is being
    /// defended is the DIRECTION and the rough size, not the constant.
    #[test]
    fn more_tapers_steady_a_bucket_against_noise() {
        let bucket = mid_axis_bucket();
        let one = std_dev(&noise_levels(1, 240, bucket));
        let three = std_dev(&noise_levels(3, 240, bucket));
        let five = std_dev(&noise_levels(5, 240, bucket));
        assert!(three < one * 0.8, "three tapers: {three:.2} dB against one's {one:.2} dB");
        assert!(five < three, "five tapers: {five:.2} dB against three's {three:.2} dB");
    }

    /// The cost, measured rather than argued about: holding a full-scale sine
    /// at 0 dB means a line spread over a wider main lobe is scaled back up to
    /// reach it, and flat noise comes up with it. So the picture's CONTRAST
    /// between a partial and the floor narrows as tapers are added, which is
    /// what a reader of the ring's Gate sees as more nodes opening at one
    /// setting.
    ///
    /// The bound is a range and not a point: it is the estimator's property,
    /// and pinning it to two decimals would fail on a taper-family change that
    /// is otherwise exactly what this test wants to allow.
    #[test]
    fn the_noise_floor_reads_higher_as_tapers_are_added() {
        let bucket = mid_axis_bucket();
        let one = mean(&noise_levels(1, 240, bucket));
        let three = mean(&noise_levels(3, 240, bucket));
        let rise = three - one;
        assert!(
            (1.0..6.0).contains(&rise),
            "three tapers lift the noise floor {rise:.2} dB, outside the 1..6 dB this trades"
        );
    }

    /// The table the two tests above assert loose bounds on, printed in full,
    /// plus what a column costs at each count. Asserts nothing and is
    /// `#[ignore]`d: it is here because choosing a taper count is a judgement
    /// against three numbers that move together, and rebuilding the harness to
    /// see them is the expensive part.
    ///
    /// `cargo test -p harmonigraph-core -- --ignored --nocapture the_taper_table`
    #[test]
    #[ignore]
    fn the_taper_table() {
        let bucket = mid_axis_bucket();
        let base = mean(&noise_levels(1, 480, bucket));
        eprintln!("\n tapers |  noise sd | floor vs 1 | ms/column @8192");
        eprintln!("--------|-----------|------------|----------------");
        for tapers in 1..=MAX_TAPERS {
            let levels = noise_levels(tapers, 480, bucket);
            let sample_rate = 48_000.0f32;
            let mut analyzer = SpectrumAnalyzer::new(sample_rate);
            analyzer.set_tapers(tapers);
            analyzer.push_samples(&vec![0.2; DEFAULT_FFT_SIZE]);
            let started = std::time::Instant::now();
            let columns = 20;
            for _ in 0..columns {
                std::hint::black_box(analyzer.pitch_spectrum());
            }
            let ms = started.elapsed().as_secs_f64() * 1000.0 / f64::from(columns);
            eprintln!(
                "   {tapers}    |  {:5.2} dB |  {:+5.2} dB  |     {ms:6.3}",
                std_dev(&levels),
                mean(&levels) - base
            );
        }
        eprintln!();
    }

    /// The same contract [`set_fft_size`](SpectrumAnalyzer::set_fft_size) holds,
    /// and for the same reason: the tapers are what the buffer is read through,
    /// so a change has to drop what was measured through the old set rather
    /// than blend the two.
    #[test]
    fn set_tapers_resets_the_window_and_noops_at_the_current_count() {
        let mut analyzer = SpectrumAnalyzer::new(48_000.0);
        analyzer.push_samples(&vec![0.2; DEFAULT_FFT_SIZE]);
        assert!(analyzer.pitch_spectrum().is_some());
        assert_eq!(analyzer.tapers(), 1, "one taper with no toggle touched");

        analyzer.set_tapers(1);
        assert!(analyzer.pitch_spectrum().is_some(), "no-op at the current count");

        analyzer.set_tapers(3);
        assert_eq!(analyzer.tapers(), 3);
        assert!(analyzer.pitch_spectrum().is_none(), "a change empties the window");
        analyzer.push_samples(&vec![0.2; DEFAULT_FFT_SIZE]);
        assert!(analyzer.pitch_spectrum().is_some());

        // Out of range clamps rather than panicking or allocating the moon: a
        // hand-edited blob reaches this through the config, and every count
        // still has to come out as a spectrum somebody can see.
        analyzer.set_tapers(0);
        assert_eq!(analyzer.tapers(), 1);
        analyzer.set_tapers(usize::MAX);
        assert_eq!(analyzer.tapers(), MAX_TAPERS);
    }

    /// Every channel of a bank measures through the same estimator, which is
    /// what makes [`power_sum`](ChannelBank::power_sum) an addition of
    /// comparable things: two channels on different taper counts have
    /// different noise floors and different effective kernels, and a sum of
    /// those is a reading of nothing.
    ///
    /// The second channel is the one worth asserting — a fan-out that set only
    /// the first would leave a stereo bank half-converted and a mono one
    /// perfectly correct, which is the shape a mono test cannot see.
    #[test]
    fn every_channel_of_a_bank_measures_through_the_same_tapers() {
        let mut bank = ChannelBank::new(48_000.0, 2);
        bank.set_tapers(3);
        for (channel, analyzer) in bank.per_channel.iter().enumerate() {
            assert_eq!(analyzer.tapers(), 3, "channel {channel} kept its own taper count");
        }

        // A rebuild is the other door in: it drops the bank for a fresh one, so
        // the count goes back to the default and the caller has to re-set it.
        bank.set_channels(1);
        assert_eq!(bank.per_channel[0].tapers(), 1, "a rebuilt bank kept a stale count");
    }
}
