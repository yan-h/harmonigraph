//! Rolling audio analysis on Harmonigraph's dependency-free pitch axis.
//!
//! Each channel owns a planned RealFFT real-input transform (a half-size
//! complex FFT plus postprocessing), with reusable input/output/scratch. One
//! column is one transform of the latest frame through a Hann window and part
//! of the listen, zero-padded to twice its length, and every pitch bucket then
//! reads that transform at its OWN frequency through a small kernel, which
//! carries the rest of the listen — see [`kernels`]. Live and
//! offline consumers share all of it verbatim. Exact-zero retained windows
//! still emit zero columns without transforming, so history and release
//! continue advancing.

use harmonigraph_core::spectrum::{
    midi_to_hz, BINS_PER_SEMITONE, SPECTRUM_BINS, SPECTRUM_MIN_MIDI,
};

mod kernels;
pub use kernels::fixed_frame;

/// The default frame length (~0.26 s at 48 kHz), the UI's Balanced setting;
/// its fresh one is Fast. The UI changes it through
/// [`SpectrumAnalyzer::configure`], trading response time against bass
/// precision at runtime. At the axis floor (20 Hz) a frame this long still
/// resolves only a few semitones, so the lowest octave reads wide; that is
/// inherent to the frame length, not a bug.
///
/// Not a power of two: a fixed window listens through about two thirds of its
/// frame (see [`fixed_frame`]), so the frame is half as long again as the
/// 8192-sample Hann window Balanced is as sharp as.
pub const DEFAULT_FFT_SIZE: usize = fixed_frame(8192);

/// All inputs that determine an analyzer's estimator. Adopt them together so
/// a caller never builds resources for intermediate settings.
#[derive(Clone, Copy, Debug)]
pub struct AnalyzerConfig {
    pub sample_rate: f32,
    /// The FRAME: how much audio one column is taken from, and so how long a
    /// listen it can give. The listen is about two thirds of it.
    pub fft_size: usize,
}

impl AnalyzerConfig {
    /// How far BEFORE the newest sample a column belongs on a time axis, in
    /// seconds: where the weight of the window it is read through is centred.
    ///
    /// The analyzer always looks BACKWARD — `pitch_spectrum` reads the most
    /// recent `fft_size` samples — so a column taken at time `t` describes
    /// audio before `t`, weighted most where its window is. Stamping it `t`
    /// would draw every sound late by this much, the whole of its energy
    /// placed at the moment the LAST of it arrived.
    ///
    /// The window's amplitude centroid, which is about where a tone that starts
    /// or stops reads half its level: within a hundredth of the frame, the
    /// window's half-weight point being a little newer. It is the same at every pitch
    /// because every bucket listens about the same instant, so one lag places
    /// the whole column. The listen sits toward the frame's newest end, so the
    /// lag is about a third of the frame, as long as the plain Hann window it
    /// is as sharp as.
    ///
    /// The one source of it: the analyzer stamps its columns with this and an
    /// export reads this far past each frame, so the two agree by construction.
    pub fn column_lag(&self) -> f64 {
        kernels::lag_samples(self.fft_size) / f64::from(self.sample_rate.max(1.0))
    }
}

impl Default for AnalyzerConfig {
    fn default() -> Self {
        Self { sample_rate: 48_000.0, fft_size: DEFAULT_FFT_SIZE }
    }
}

/// The pitch axis is fixed; only its conversion from Hz to FFT bins changes
/// with the analyzer configuration. Share these frequencies across analyzers.
struct BucketFrequencies {
    lower_hz: f32,
    upper_hz: f32,
}

static BUCKET_FREQUENCIES: std::sync::LazyLock<[BucketFrequencies; SPECTRUM_BINS]> =
    std::sync::LazyLock::new(|| {
        let half_bucket = 0.5 / BINS_PER_SEMITONE as f32;
        std::array::from_fn(|b| {
            let midi = SPECTRUM_MIN_MIDI + (b as f32 + 0.5) / BINS_PER_SEMITONE as f32;
            BucketFrequencies {
                lower_hz: midi_to_hz(midi - half_bucket),
                upper_hz: midi_to_hz(midi + half_bucket),
            }
        })
    });

/// Rolling analyzer: push mono samples as they arrive, ask for the
/// spectrum whenever the display wants a fresh frame.
pub struct SpectrumAnalyzer {
    sample_rate: f32,
    fft_size: usize,
    /// The most recent `fft_size` samples, as a circular buffer.
    ring: Vec<f32>,
    write: usize,
    /// Samples the ring holds, pushed or carried in by a frame change (see
    /// [`configure`](Self::configure)), saturating at `fft_size`.
    filled: usize,
    /// A plan and its reusable buffers, owned by this channel, for a transform
    /// [`kernels::PAD`] frames long. The planner is dropped after
    /// configuration; no cross-channel or global plan cache.
    fft: Option<std::sync::Arc<dyn realfft::RealToComplex<f32>>>,
    fft_input: Vec<f32>,
    fft_output: Vec<realfft::num_complex::Complex32>,
    fft_scratch: Vec<realfft::num_complex::Complex32>,
    /// The pitch buckets [`pitch_spectrum`](SpectrumAnalyzer::pitch_spectrum)
    /// hands back, owned here rather than returned by value: at 32 buckets to
    /// the semitone one spectrum is 15.3 KB, and returning it copied that much
    /// per channel per column for nothing.
    ///
    /// SCRATCH and not a cache: it has no key, because every call rewrites it
    /// whole — `pitch_spectrum` writes every bucket, a silent one as zero, so
    /// none can be left reading the previous column. Nothing reads it that did
    /// not just write it.
    buckets: Box<[f32; SPECTRUM_BINS]>,
    /// Which kernels each bucket reads, derived from exactly the frame and the
    /// rate, and replanned by [`configure`](Self::configure) whenever either
    /// changes, so there is no key to check per column.
    kernels: kernels::Kernels,
    /// [`AnalyzerConfig::column_lag`] for the same two, settled with the
    /// kernels.
    lag: f64,
}

impl SpectrumAnalyzer {
    pub fn new(config: AnalyzerConfig) -> Self {
        let mut analyzer = SpectrumAnalyzer {
            sample_rate: config.sample_rate.max(1.0),
            fft_size: 0,
            ring: Vec::new(),
            write: 0,
            filled: 0,
            fft: None,
            fft_input: Vec::new(),
            fft_output: Vec::new(),
            fft_scratch: Vec::new(),
            buckets: Box::new([0.0; SPECTRUM_BINS]),
            kernels: kernels::Kernels::new(),
            lag: 0.0,
        };
        analyzer.configure(config);
        analyzer
    }

    /// Adopt the final estimator, rebuilding only what its changed inputs
    /// decide. A changed rate requires a fresh audio window, since samples at
    /// one rate say nothing at another. A changed frame at the same rate
    /// carries the newest samples the old window held, as many as the new one
    /// takes, so a frequency-resolution switch keeps drawing: a shorter frame
    /// is ready at once, and a longer one fills the rest (#1584). Unchanged
    /// settings preserve readiness and allocate nothing.
    pub fn configure(&mut self, config: AnalyzerConfig) {
        let AnalyzerConfig { sample_rate, fft_size } = config;
        assert!(
            fft_size >= 4 && fft_size % 2 == 0,
            "an even frame, four up: a real-input transform needs an even length, and realfft \
             plans any even one, which is what lets a frame be half as long again as its listen"
        );
        let sample_rate = sample_rate.max(1.0);
        let size_changed = fft_size != self.fft_size;
        let rate_changed = (sample_rate - self.sample_rate).abs() > f32::EPSILON;
        if !size_changed && !rate_changed {
            return;
        }
        // The newest samples the new frame takes, none across a rate change.
        // The new ring holds them oldest first from slot 0, exactly where a
        // fresh analyzer fed them would, so it reads them identically and fills
        // on from there. The old ring in time order is `older` then `newer`.
        let carried = if rate_changed { 0 } else { self.filled.min(fft_size) };
        let (newer, older) = self.ring.split_at(self.write);
        let from_newer = carried.min(newer.len());
        let from_older = carried - from_newer;
        let mut ring = vec![0.0; fft_size];
        ring[..from_older].copy_from_slice(&older[older.len() - from_older..]);
        ring[from_older..carried].copy_from_slice(&newer[newer.len() - from_newer..]);
        self.ring = ring;
        self.write = carried % fft_size;
        self.filled = carried;
        self.fft_size = fft_size;
        if rate_changed {
            self.sample_rate = sample_rate;
        }
        if size_changed {
            // No global cache or retained planner: each channel owns its plan.
            let fft =
                realfft::RealFftPlanner::<f32>::new().plan_fft_forward(fft_size * kernels::PAD);
            self.fft_input = fft.make_input_vec();
            self.fft_output = fft.make_output_vec();
            self.fft_scratch = fft.make_scratch_vec();
            self.fft = Some(fft);
        }
        self.kernels.plan(self.sample_rate, self.fft_size);
        self.lag =
            AnalyzerConfig { sample_rate: self.sample_rate, fft_size: self.fft_size }.column_lag();
    }

    /// How far BEFORE the newest sample the spectrum it produces belongs on a
    /// time axis, in seconds: [`AnalyzerConfig::column_lag`] for the settings
    /// it is running.
    pub fn window_center_offset(&self) -> f64 {
        self.lag
    }

    /// Forget the retained audio, keeping the plan, the window and the
    /// kernels — none of which the window's CONTENTS decide.
    ///
    /// This is what a source restart owes (see
    /// [`ChannelBank::restart`](ChannelBank::restart)): the analyzer must not
    /// read a spectrum across the boundary, and nothing about a new run
    /// changes which transform it is read through.
    fn clear_window(&mut self) {
        self.ring.fill(0.0);
        self.write = 0;
        self.filled = 0;
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
    /// Borrowed from the analyzer's own [`buckets`](Self::buckets) rather than
    /// returned by value, because one spectrum is 15.3 KB and every caller
    /// reads it in place. The next call overwrites it.
    ///
    /// Bucket values are absolute power: a full-scale sine reads ~1.0 at
    /// its pitch, so successive frames are comparable and the display can
    /// apply a fixed mapping.
    ///
    /// Every bucket is filled — this reads the whole spectrum onto the
    /// log-pitch axis, rather than depositing only the peaks, so broadband
    /// sound draws as a floor and an envelope rather than as speckle. Each
    /// bucket is read AT ITS OWN FREQUENCY ([`kernels`]), so a partial is drawn
    /// at its pitch wherever it falls between the transform's bins. This
    /// replaced a resample of the bins themselves, which kept every peak on the
    /// bin it was measured at: through the Fast window that drew a 55 Hz A1 a
    /// semitone sharp and lit A♯ on the lattice brighter than A (#1575).
    ///
    /// The frame's MEAN, weighted by the window it is multiplied by
    /// ([`kernels::Kernels::window`]), is removed before the transform. A
    /// constant offset otherwise reaches the bottom of the axis through the
    /// skirt of the listen's own response, which the kernels read down to the
    /// first bin above DC (−21 dB for a 0.5 offset through the Fast frame);
    /// without it, the kernels could not reach as close to DC as they do.
    pub fn pitch_spectrum(&mut self) -> Option<&[f32; SPECTRUM_BINS]> {
        if self.filled < self.fft_size {
            return None;
        }

        // Inspect the retained window, not just the newest input block. Return
        // a measurement so callers keep advancing history and display decay.
        if self.ring.iter().all(|sample| *sample == 0.0) {
            self.buckets.fill(0.0);
            return Some(&self.buckets);
        }

        // The ring in time order is its tail from `write` (oldest first) and
        // then its head up to `write`: two straight runs rather than an index
        // wrapped per sample, which the compiler cannot vectorize.
        let (newer, older) = self.ring.split_at(self.write);
        let (window_older, window_newer) = self.kernels.window().split_at(older.len());
        let weighted = older.iter().zip(window_older).map(|(s, w)| s * w).sum::<f32>()
            + newer.iter().zip(window_newer).map(|(s, w)| s * w).sum::<f32>();
        let mean = weighted / self.kernels.window_sum();
        // The frame sits where its listen's centre is the padded transform's
        // middle, so the kernels stay real (see `Kernels::offset`). The padding
        // is rewritten every column, because realfft uses its input as scratch.
        let offset = self.kernels.offset();
        let (before, rest) = self.fft_input.split_at_mut(offset);
        let (frame, after) = rest.split_at_mut(self.fft_size);
        before.fill(0.0);
        after.fill(0.0);
        let (input_older, input_newer) = frame.split_at_mut(older.len());
        for ((x, s), w) in input_older.iter_mut().zip(older).zip(window_older) {
            *x = (s - mean) * w;
        }
        for ((x, s), w) in input_newer.iter_mut().zip(newer).zip(window_newer) {
            *x = (s - mean) * w;
        }
        self.fft
            .as_ref()
            .unwrap()
            .process_with_scratch(&mut self.fft_input, &mut self.fft_output, &mut self.fft_scratch)
            .unwrap();
        self.kernels.read(&self.fft_output, &mut self.buckets);
        Some(&self.buckets)
    }
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
}

impl ChannelBank {
    /// A bank for `channels` channels (at least one).
    pub fn new(config: AnalyzerConfig, channels: usize) -> ChannelBank {
        ChannelBank {
            per_channel: (0..channels.max(1)).map(|_| SpectrumAnalyzer::new(config)).collect(),
            scratch: Vec::new(),
        }
    }

    pub fn channels(&self) -> usize {
        self.per_channel.len()
    }

    /// All channels measure through the same final estimator. A layout change
    /// starts every channel empty; samples from one layout say nothing about
    /// the next. New channels are built at the requested settings directly.
    pub fn configure(&mut self, config: AnalyzerConfig, channels: usize) {
        let channels = channels.max(1);
        if channels != self.per_channel.len() {
            self.per_channel = (0..channels).map(|_| SpectrumAnalyzer::new(config)).collect();
        } else {
            for analyzer in &mut self.per_channel {
                analyzer.configure(config);
            }
        }
    }

    /// Forget retained audio at a source boundary, keeping every estimator
    /// resource. The next feed adopts its final format with [`Self::configure`].
    pub fn restart(&mut self) {
        for analyzer in &mut self.per_channel {
            analyzer.clear_window();
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

    /// How far before the newest frame a column belongs on a time axis. Every
    /// channel shares its settings, so this is the bank's as much as any one
    /// analyzer's.
    pub fn window_center_offset(&self) -> f64 {
        self.per_channel[0].window_center_offset()
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
    ///
    /// BY VALUE, where [`pitch_spectrum`](SpectrumAnalyzer::pitch_spectrum) is
    /// borrowed — deliberately, and not an oversight to fix next. The per-
    /// channel copies it removed were one per channel per column; this one is
    /// one per column, and its caller holds the result across a `&mut self`
    /// call of its own (the history push), so borrowing here would cost that
    /// caller a restructure to buy a single 15.3 KB copy at the column rate.
    pub fn power_sum(&mut self) -> Option<[f32; SPECTRUM_BINS]> {
        let mut total = [0.0f32; SPECTRUM_BINS];
        for analyzer in &mut self.per_channel {
            let channel = analyzer.pitch_spectrum()?;
            for (sum, p) in total.iter_mut().zip(channel) {
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

/// Sample `i` of a periodic Hann window of `n` samples, the one definition of
/// the window: the frame's window is built from it
/// ([`kernels::Kernels::window`]), and the listen's sums read it too, so they
/// describe the same Hann window the frame is multiplied by (times the share of
/// the listen the frame carries).
fn hann_weight(i: usize, n: usize) -> f32 {
    let phase = std::f32::consts::TAU * i as f32 / n as f32;
    0.5 * (1.0 - phase.cos())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_core::spectrum::hz_to_midi;

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
        let mut analyzer = SpectrumAnalyzer::new(AnalyzerConfig { sample_rate, fft_size });
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
        *analyzer.pitch_spectrum().expect("window filled")
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
        let mut analyzer = SpectrumAnalyzer::new(Default::default());
        assert!(analyzer.pitch_spectrum().is_none());
        analyzer.push_samples(&vec![0.1; DEFAULT_FFT_SIZE - 1]);
        assert!(analyzer.pitch_spectrum().is_none(), "one short of a window");
        analyzer.push_samples(&[0.1]);
        assert!(analyzer.pitch_spectrum().is_some());
    }

    #[test]
    fn silent_windows_keep_readiness_retained_audio_and_resume() {
        const N: usize = 4096;
        let mut analyzer =
            SpectrumAnalyzer::new(AnalyzerConfig { fft_size: N, ..Default::default() });
        analyzer.push_samples(&vec![-0.0; N - 1]);
        assert!(analyzer.pitch_spectrum().is_none(), "zero input still needs a full window");
        analyzer.push_samples(&[0.0]);
        assert!(analyzer.pitch_spectrum().unwrap().iter().all(|x| x.to_bits() == 0));

        // A nonzero sample at the physical end of the ring, but off the
        // final Hann endpoint whose window would erase it.
        let mut near_end = vec![0.0; N];
        near_end[N - 2] = 1.0;
        analyzer.push_samples(&near_end);
        assert!(analyzer.pitch_spectrum().unwrap().iter().any(|x| *x > 0.0));
        // Time order crosses the ring seam while older audio remains.
        analyzer.push_samples(&[0.0; 17]);
        assert!(analyzer.pitch_spectrum().unwrap().iter().any(|x| *x > 0.0));
        analyzer.push_samples(&vec![0.0; N]);
        assert!(analyzer.pitch_spectrum().unwrap().iter().all(|x| x.to_bits() == 0));

        // Quiet audio must resume after the shortcut without reading
        // skipped scratch or being mistaken for exact silence.
        let quiet: Vec<_> = (0..N).map(|i| 1e-10 * (i as f32 * 0.05).sin()).collect();
        analyzer.push_samples(&quiet);
        assert!(analyzer.pitch_spectrum().unwrap().iter().any(|x| *x > 0.0));
        analyzer.push_samples(&vec![-0.0; N]);
        assert!(analyzer.pitch_spectrum().unwrap().iter().all(|x| x.to_bits() == 0));
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

    /// The bottom of the axis draws at every frame length, not just the one
    /// the rest of these tests use.
    ///
    /// Where the kernels can read STARTS at a fixed BIN of the frame (1, the
    /// first above DC, and a kernel's clearance above that, 2.8 bins in all)
    /// and therefore at a moving FREQUENCY: about 22 Hz through the Fast frame
    /// at 48 kHz, but 44 Hz through the same 6144 samples at 96 kHz, which the
    /// UI no longer asks for but the analyzer still takes. So the buckets that
    /// read the first usable bins are only on the axis AT ALL at the shorter
    /// frames and the higher rates, and a bound that quietly excludes them is
    /// invisible at 12288/48 kHz — which is every other test here.
    ///
    /// The failure this pins is a band of the axis reading as silence with the
    /// tone in it: two static lines with a hole between them, and the hole does
    /// not move when the tone does.
    #[test]
    fn the_lowest_buckets_draw_at_every_window_length() {
        for (fft_size, sr) in [(6144, 48_000.0f32), (6144, 96_000.0), (12_288, 96_000.0)] {
            let bin_hz = sr / fft_size as f32;
            // Three bins up, just inside where the kernels start to read.
            let hz = 3.0 * bin_hz;
            let buckets = analyze_with(fft_size, &[(hz, 0.8)], sr);
            let at = bucket_of_midi(hz_to_midi(hz));
            let near = (at.saturating_sub(2)..=(at + 2).min(SPECTRUM_BINS - 1))
                .map(|b| buckets[b])
                .fold(0.0f32, f32::max);
            assert!(
                near > 1e-6,
                "{fft_size}-point window at {sr} Hz: a tone at {hz:.1} Hz \
                 (bin 3) left bucket {at} and its neighbours silent",
            );
        }
    }

    /// Every frame draws a partial at its own pitch, the bass included (#1575).
    ///
    /// Reading the transform's bins kept each peak on the bin it was measured
    /// at, which through the Fast frame drew a 55 Hz A1 more than a semitone
    /// sharp and lit A♯ on the lattice brighter than A. The tones here are
    /// where that was worst, none of them on a bin of any of the three fixed
    /// frames.
    #[test]
    fn a_partial_reads_at_its_own_pitch_through_every_frame() {
        for fft_size in [6144, 12_288, 24_576] {
            for hz in [27.5f32, 41.2, 55.0, 110.0, 220.0] {
                let peak = peak_bucket(&analyze_with(fft_size, &[(hz, 0.8)], 48_000.0));
                let midi = SPECTRUM_MIN_MIDI + (peak as f32 + 0.5) / BINS_PER_SEMITONE as f32;
                let cents = 100.0 * (midi - hz_to_midi(hz));
                // A bucket is 3.125 cents, so a peak in the right one is within
                // half of that; one bucket either side is rounding, not place.
                assert!(
                    cents.abs() <= 100.0 / BINS_PER_SEMITONE as f32,
                    "{fft_size}-sample frame: {hz} Hz peaks {cents:+.1} cents off",
                );
            }
        }
    }

    /// A constant offset stays off the axis through every frame.
    ///
    /// The kernels read down to the first bin above DC, where the skirt of the
    /// listen's own response still carries a DC offset onto the bottom of the
    /// axis — −21 dB for this one through the Fast frame — unless the frame's
    /// mean is removed first, which is what this pins.
    #[test]
    fn a_dc_offset_stays_off_the_axis() {
        for fft_size in [6144, 12_288, 24_576] {
            let mut analyzer =
                SpectrumAnalyzer::new(AnalyzerConfig { fft_size, ..Default::default() });
            analyzer.push_samples(&vec![0.5; fft_size + 777]);
            let spectrum = analyzer.pitch_spectrum().expect("window filled");
            let loudest = spectrum.iter().fold(0.0f32, |a, &b| a.max(b));
            assert!(loudest < 1e-9, "{fft_size}-sample frame: DC read {loudest:e} somewhere");
        }
    }

    /// A column belongs where its window listens ([`AnalyzerConfig::column_lag`]):
    /// a tone that starts at a known sample reads half its level in the column
    /// stamped there, through every fixed frame, to within a
    /// hundredth of the frame. Not exactly: half the level is where half the
    /// window's weight is over the tone, its median, and the fixed window is
    /// lopsided enough to put that 0.6, 1.3 and 2.8 ms newer than the centroid
    /// the column is stamped at, so its onsets read that much early.
    ///
    /// Half the frame, which is where every column was stamped while every
    /// window was centred on its frame, would put a fixed window's onsets 21 to
    /// 84 ms early, its listen being toward the frame's newest end.
    #[test]
    fn a_column_is_stamped_where_its_window_listens() {
        let rate = 48_000.0f32;
        for fft_size in [6144, 12_288, 24_576] {
            let config = AnalyzerConfig { sample_rate: rate, fft_size };
            let lag = config.column_lag() * f64::from(rate);
            let onset = fft_size;
            let tone: Vec<f32> = (0..3 * fft_size)
                .map(|i| match i < onset {
                    true => 0.0,
                    false => (std::f32::consts::TAU * 1000.0 * i as f32 / rate).sin(),
                })
                .collect();
            let at = bucket_of_midi(hz_to_midi(1000.0));
            let mut analyzer = SpectrumAnalyzer::new(config);
            // From a fiftieth of the frame before the expected crossing, a
            // column every eight samples until the tone reads half its
            // full-scale level.
            let slack = fft_size / 50;
            let mut fed = (onset as f64 + lag) as usize - slack;
            analyzer.push_samples(&tone[..fed]);
            loop {
                let level = analyzer.pitch_spectrum().expect("window filled")[at - 2..=at + 2]
                    .iter()
                    .fold(0.0f32, |a, &b| a.max(b));
                if level.sqrt() >= 0.5 {
                    break;
                }
                assert!(fed < onset + lag as usize + slack, "{fft_size}: never crossed");
                analyzer.push_samples(&tone[fed..fed + 8]);
                fed += 8;
            }
            let stamped = (fed - 1) as f64 - lag;
            let early_ms = (onset as f64 - stamped) / f64::from(rate) * 1e3;
            let frame_ms = fft_size as f64 / f64::from(rate) * 1e3;
            assert!(
                early_ms.abs() < frame_ms / 100.0,
                "{fft_size}: the onset reads {early_ms:+.2} ms off"
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
        // With a shorter frame too, which a frame change alone would carry into.
        for fft_size in [DEFAULT_FFT_SIZE, 6144] {
            let mut analyzer = SpectrumAnalyzer::new(Default::default());
            analyzer.push_samples(&vec![0.2; DEFAULT_FFT_SIZE]);
            assert!(analyzer.pitch_spectrum().is_some());
            analyzer.configure(AnalyzerConfig { sample_rate: 44_100.0, fft_size });
            assert!(
                analyzer.pitch_spectrum().is_none(),
                "{fft_size}: stale samples must not be analyzed under a new clock"
            );
        }
    }

    /// The bucket plan is derived from the sample rate, so an analyzer that
    /// changed rate must read exactly as one built at the new rate — including
    /// a half rate, where the axis runs to Nyquist and the top bucket is clamped.
    #[test]
    fn a_sample_rate_change_reads_exactly_like_a_fresh_analyzer() {
        for rate in [96_000.0, 22_050.0] {
            let samples: Vec<f32> = (0..DEFAULT_FFT_SIZE + 333)
                .map(|i| {
                    let t = i as f32 / rate;
                    0.5 * (std::f32::consts::TAU * 440.0 * t).sin()
                        + 0.2 * (std::f32::consts::TAU * 9_000.0 * t).sin()
                        + if i % 97 == 0 { 0.3 } else { 0.0 }
                })
                .collect();
            let mut changed = SpectrumAnalyzer::new(Default::default());
            let plan = changed.fft.clone().unwrap();
            changed.configure(AnalyzerConfig { sample_rate: rate, ..Default::default() });
            assert!(std::sync::Arc::ptr_eq(&plan, changed.fft.as_ref().unwrap()));
            changed.push_samples(&samples);
            let mut fresh =
                SpectrumAnalyzer::new(AnalyzerConfig { sample_rate: rate, ..Default::default() });
            fresh.push_samples(&samples);
            let expected = *fresh.pitch_spectrum().unwrap();
            assert!(expected.iter().filter(|p| **p > 0.0).count() > SPECTRUM_BINS / 2);
            assert_eq!(changed.pitch_spectrum().unwrap(), &expected, "at {rate} Hz");
        }
    }

    /// Two tones and a click train at 48 kHz: enough structure that a sample
    /// carried to the wrong place reads differently.
    fn mixture(len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f32 / 48_000.0;
                0.5 * (std::f32::consts::TAU * 440.0 * t).sin()
                    + 0.2 * (std::f32::consts::TAU * 9_000.0 * t).sin()
                    + if i % 97 == 0 { 0.3 } else { 0.0 }
            })
            .collect()
    }

    /// A longer frame carries the shorter one's samples and waits only for the
    /// rest of itself, then reads exactly what a fresh analyzer fed the same
    /// newest samples reads (#1584). The shorter ring has wrapped, so the
    /// samples carried cross its seam, and the new ones land after them.
    #[test]
    fn a_longer_frame_fills_the_rest_and_noops_at_the_current_size() {
        let samples = mixture(9000 + DEFAULT_FFT_SIZE - 6144);
        let (before, after) = samples.split_at(9000);
        let mut analyzer =
            SpectrumAnalyzer::new(AnalyzerConfig { fft_size: 6144, ..Default::default() });
        analyzer.push_samples(before);
        assert!(analyzer.pitch_spectrum().is_some());
        analyzer.configure(Default::default());
        analyzer.push_samples(&after[..after.len() - 1]);
        assert!(analyzer.pitch_spectrum().is_none(), "one short of the longer frame");
        analyzer.push_samples(&after[after.len() - 1..]);
        let mut fresh = SpectrumAnalyzer::new(Default::default());
        fresh.push_samples(&samples[samples.len() - DEFAULT_FFT_SIZE..]);
        let expected = *fresh.pitch_spectrum().unwrap();
        assert_eq!(analyzer.pitch_spectrum().expect("the rest arrived"), &expected);
        // Setting the same size again is a no-op: the filled window survives.
        analyzer.configure(Default::default());
        assert!(analyzer.pitch_spectrum().is_some(), "no-op resize kept the window");
    }

    /// A switch to a shorter frame draws on the next column (#1584), from the
    /// newest samples the longer one held, reading exactly what a fresh
    /// analyzer fed those samples reads. The longer ring has wrapped, so the
    /// samples carried cross its seam.
    #[test]
    fn a_shorter_frame_draws_at_once_from_the_samples_it_carries() {
        let samples = mixture(30_000);
        let mut switched =
            SpectrumAnalyzer::new(AnalyzerConfig { fft_size: 24_576, ..Default::default() });
        switched.push_samples(&samples);
        switched.configure(AnalyzerConfig { fft_size: 6144, ..Default::default() });
        let mut fresh =
            SpectrumAnalyzer::new(AnalyzerConfig { fft_size: 6144, ..Default::default() });
        fresh.push_samples(&samples[samples.len() - 6144..]);
        let expected = *fresh.pitch_spectrum().unwrap();
        assert!(expected.iter().filter(|p| **p > 0.0).count() > SPECTRUM_BINS / 2);
        assert_eq!(switched.pitch_spectrum().expect("a whole frame carried"), &expected);
    }

    /// The bottom of the axis sits too near DC for a short frame to place —
    /// 20 Hz is under two of its kernel's widths above the Fast frame's first
    /// usable bin at 48 kHz — so those buckets are skipped rather than
    /// measured, and skipped has to read as silence.
    ///
    /// It is a test rather than an obvious property because
    /// [`pitch_spectrum`](SpectrumAnalyzer::pitch_spectrum) fills a RETAINED
    /// buffer: a longer window does reach those buckets, so shortening it is a
    /// column that writes them followed by a column that does not, and without
    /// the zeroing the second column would report the first one's levels. The
    /// window length is a setting on the Analyzer bar, so that is a dial away.
    #[test]
    fn shortening_the_window_darkens_the_buckets_it_can_no_longer_reach() {
        let mut analyzer = SpectrumAnalyzer::new(Default::default());
        // 20 Hz: the axis floor, and a bucket the Fast frame cannot place.
        let tone: Vec<f32> = (0..30_000)
            .map(|i| 0.5 * (std::f32::consts::TAU * 20.0 * i as f32 / 48_000.0).sin())
            .collect();
        analyzer.configure(AnalyzerConfig { fft_size: 24_576, ..Default::default() });
        analyzer.push_samples(&tone);
        let lit = analyzer.pitch_spectrum().expect("window filled")[0];
        assert!(lit > 0.0, "fixture must light the bottom bucket at the long window");
        // Read from the carried samples alone, which still hold the tone.
        analyzer.configure(AnalyzerConfig { fft_size: 6144, ..Default::default() });
        assert_eq!(
            analyzer.pitch_spectrum().expect("window filled")[0],
            0.0,
            "a bucket the window cannot reach reported the previous window's level",
        );
    }

    /// Feed a stereo pair through a bank and return the combined spectrum.
    fn analyze_stereo(
        left: impl Fn(f32) -> f32,
        right: impl Fn(f32) -> f32,
    ) -> [f32; SPECTRUM_BINS] {
        let sr = 48_000.0f32;
        let mut bank =
            ChannelBank::new(AnalyzerConfig { sample_rate: sr, ..Default::default() }, 2);
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

    /// A changed sample rate requires a fresh window, while the transform,
    /// which the rate does not decide, survives. Refilling must agree with a
    /// fresh analyzer.
    #[test]
    fn changing_the_rate_retains_the_plan_but_requires_a_fresh_window() {
        let mut analyzer = SpectrumAnalyzer::new(Default::default());
        analyzer.push_samples(&vec![0.2; DEFAULT_FFT_SIZE]);
        assert!(analyzer.pitch_spectrum().is_some());
        let plan = analyzer.fft.clone().unwrap();

        analyzer.configure(Default::default());
        assert!(analyzer.pitch_spectrum().is_some(), "no-op at the current settings");

        let config = AnalyzerConfig { sample_rate: 44_100.0, ..Default::default() };
        analyzer.configure(config);
        assert!(std::sync::Arc::ptr_eq(&plan, analyzer.fft.as_ref().unwrap()));
        assert!(analyzer.pitch_spectrum().is_none(), "a change empties the window");
        let samples: Vec<_> = (0..DEFAULT_FFT_SIZE + 333)
            .map(|i| (i as f32 * 0.07).sin() * 0.5 + if i % 97 == 0 { 0.3 } else { 0.0 })
            .collect();
        analyzer.push_samples(&samples[..DEFAULT_FFT_SIZE - 1]);
        assert!(analyzer.pitch_spectrum().is_none(), "requires a whole new window");
        analyzer.push_samples(&samples[DEFAULT_FFT_SIZE - 1..]);
        let mut fresh = SpectrumAnalyzer::new(config);
        fresh.push_samples(&samples);
        assert_eq!(analyzer.pitch_spectrum().unwrap(), fresh.pitch_spectrum().unwrap());
    }

    /// A joint format/estimator change builds every channel at the final
    /// settings and discards audio from the old layout, including channel zero.
    #[test]
    fn joint_configuration_matches_a_fresh_bank_in_every_channel() {
        let mut bank = ChannelBank::new(Default::default(), 1);
        bank.push_frames(&vec![0.2; DEFAULT_FFT_SIZE]);
        assert!(bank.power_sum().is_some());
        let config = AnalyzerConfig { sample_rate: 22_050.0, fft_size: 4096 };
        // First replace the mono layout; then update both retained channels.
        for config in [config, AnalyzerConfig { sample_rate: 44_100.0, ..config }] {
            bank.configure(config, 2);
            assert_eq!(bank.channels(), 2);
            assert!(bank.power_sum().is_none());
            let samples: Vec<_> = (0..config.fft_size + 333)
                .flat_map(|i| [(i as f32 * 0.11).sin() * 0.5, (i as f32 * 0.23).cos() * 0.3])
                .collect();
            let prefix = (config.fft_size - 1) * 2;
            bank.push_frames(&samples[..prefix]);
            assert!(bank
                .per_channel
                .iter_mut()
                .all(|analyzer| analyzer.pitch_spectrum().is_none()));
            bank.push_frames(&samples[prefix..]);
            let mut fresh = ChannelBank::new(config, 2);
            fresh.push_frames(&samples);
            for (actual, expected) in bank.per_channel.iter_mut().zip(&mut fresh.per_channel) {
                assert_eq!(actual.pitch_spectrum().unwrap(), expected.pitch_spectrum().unwrap());
            }
            assert_eq!(bank.power_sum().unwrap(), fresh.power_sum().unwrap());
        }
    }

    /// A restart must clear audio even when its format and estimator stay the same.
    ///
    /// Emptying the windows used to be a side effect of replacing the bank
    /// wholesale; now it is the whole job, while the transform the windows are
    /// read through deliberately survives. Both halves are load-bearing: a
    /// window that carried over would analyze one spectrum from two runs, and a
    /// configuration that did not carry over would be rebuilt by the next feed
    /// anyway, which is the work this exists to stop doing.
    #[test]
    fn a_restart_empties_the_windows_and_keeps_the_configuration() {
        let mut bank = ChannelBank::new(AnalyzerConfig { fft_size: 4096, ..Default::default() }, 2);
        bank.push_frames(&vec![0.2; 4096 * 2]);
        assert!(bank.power_sum().is_some(), "fixture must fill both windows first");

        bank.restart();
        assert!(bank.power_sum().is_none(), "a restart must not analyze across the boundary");
        for (channel, analyzer) in bank.per_channel.iter().enumerate() {
            assert_eq!(analyzer.fft_size, 4096, "channel {channel} lost its window length");
        }

        // And the run resumes at that same configuration, without the caller
        // having to re-set anything.
        bank.push_frames(&vec![0.2; 4096 * 2]);
        assert!(bank.power_sum().is_some());
    }
}
