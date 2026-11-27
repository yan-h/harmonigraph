//! The audio-derived spectrum behind the Spectral pane: the analyzer that
//! turns incoming samples into columns, and what those columns become, the
//! curve and the history. What each drawing surface holds of the history is
//! [`SpectrogramSurfaces`](crate::spectrogram::SpectrogramSurfaces), beside
//! the heatmap it feeds.
//! Runtime-only — none of this is persisted.

use crate::SpectrumConfig;

use harmonigraph_core::spectrum::SpectrumBuckets;

/// Audio-derived pitch spectrum shown in the Spectral pane: what the columns
/// a feeder's [`SpectrumAnalysis`] makes become. Each shell owns the analysis
/// over its own audio and dates it on its own clock (plugin: the selected
/// main/sidechain input, through its input worker; standalone: the mock synth;
/// offline: the take's soundtrack), and hands what it made here: the curve
/// each column advances, the history it joins, and the memo of the curve's
/// fold. Runtime-only.
pub struct AudioSpectrum {
    /// Changed only when the smoothed grid changes. This identity lives beside
    /// the measurement keyed on it, so resetting it with
    /// [`AudioSpectrum::restart`] drops both together and no fold outlives
    /// the display it was measured from. Resetting the analysis alone does
    /// not, so whoever restarts it delivers a [`Delivery::Restart`] too.
    display_revision: u64,
    /// One lazy Fold, keyed on everything its value is measured from and
    /// nothing else. `Fold::measure` is a pure function of `display` and the
    /// sanitized width: `display_revision` stands for the first — the only two
    /// writes to `display` bump it on the adjacent line — and the width is
    /// carried by bits. Geometry, palettes and TIME are not inputs; a frame
    /// with no new column measures the same numbers the last one did, so `now`
    /// only decided how often the memo was thrown away.
    frame_fold: Option<(u64, u32, crate::panes::spectral_fold::Fold)>,
    #[cfg(test)]
    pub(crate) fold_measurements: usize,
    /// Smoothed display buckets (power; the pane maps to height).
    pub(crate) display: SpectrumBuckets,
    /// When samples last arrived; the curve hides once the source stops
    /// (silent/unrouted input, switched-off synth) rather than freezing.
    pub(crate) last_samples: Option<f64>,
    /// When the spectrum last held anything that draws, for
    /// [`Animating`](crate::Animating), which asks for frames only while what
    /// that shows can still be in view or still settling. Samples
    /// arriving say nothing about it, because a host streams silence with its
    /// transport stopped.
    pub(crate) last_audible: Heard,
    /// The heatmap's history: every raw (unsmoothed) column folded into a
    /// slab per rung as it arrives, bounded by each rung's ring. See
    /// [`SlabPyramid`](crate::spectrogram::SlabPyramid) and
    /// [`AudioSpectrum::keep`].
    pub(crate) pyramid: crate::spectrogram::SlabPyramid,
    /// Columns the history has taken: see [`AudioSpectrum::columns_taken`].
    #[cfg(any(test, feature = "testing"))]
    columns_taken: u64,
}

/// Samples in, raw columns out, on an exact grid of sample counts, dated from
/// the frame zero its feeder states. It never reads what it has produced, so
/// it runs where the spectrum is not (#1559): the plugin's input worker owns its
/// own and analyzes off the editor's lock.
#[derive(Default)]
pub struct SpectrumAnalysis {
    /// One analyzer per input channel, combined in the power domain — see
    /// [`ChannelBank`](harmonigraph_analysis::ChannelBank). Built at the first
    /// feed, at the settings and format it brings, rather than at a default
    /// the feed would throw away: a bank's kernels cost the main thread about
    /// 0.68 ms per instance.
    pub(crate) analyzer: Option<harmonigraph_analysis::ChannelBank>,
    /// FRAMES pushed since this run started, and the count at which the
    /// next FFT falls due. The column grid is a function of these two and
    /// nothing else — see [`feed_source`](Self::feed_source).
    ///
    /// Frames, not samples: a stereo stream carries two samples per instant, and
    /// a hop is an amount of TIME. Counting samples would halve the hop the
    /// moment the input went stereo.
    pub(crate) frames_seen: u64,
    pub(crate) next_hop: u64,
}

/// When each reading of the spectrum last held anything, `None` until one
/// has.
///
/// Two, because the readings fall silent apart: the curve's release carries
/// it seconds past the sound, and a column the heatmap shows can leave the
/// curve under the floor. Stamping both on one clock made a Clear's silent
/// columns look heard for as long as the curve was still falling.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Heard {
    /// The newest raw column the history folded that the heatmap draws: one
    /// with a bucket over the faintest dB the volume colours draw at its pitch
    /// ([`draws`]), the level the heatmap clamps each bucket at before any
    /// effect reads it. What the heatmap scrolls.
    pub(crate) column: Option<f64>,
    /// Until when the columns arriving move what is drawn of the smoothed
    /// display (the curve, the Spiral and the lattice ring) and of the
    /// heatmap's fresh edge, which asks for every frame while they do. On
    /// shell time, off the newest frame of each batch, which is what
    /// [`display`](AudioSpectrum::display) holds the curve from, rather than
    /// off the column, which sits a resolution's lag before them.
    ///
    /// A column that leaves something drawn, in itself or in the curve it
    /// moves ([`AudioSpectrum::take`]), puts this
    /// [`HOLD_SECONDS`](AudioSpectrum::HOLD_SECONDS) past its batch, where the
    /// curve hides if no samples follow. One that leaves nothing drawn brings
    /// it back to its batch: the curve has gone flat under silence still
    /// arriving, and each column after it draws the same nothing (#1917 Q2).
    pub(crate) curve_until: Option<f64>,
}

/// How far under the faintest dB a reading draws
/// ([`faintest_drawn_db`](crate::panes::spectral::axes::faintest_drawn_db)) a
/// bucket still counts as drawn.
///
/// The test reads each bucket's own power at its own pitch, while a reading
/// takes the tilt at the pitch it draws: the curve's and the Spiral's
/// footprint mean at its pixel's centre, at most half a bucket (1/64 of a
/// semitone) from the buckets it averages, and the lattice ring's Fold at the
/// centre of a Gaussian over its neighbours. At the steepest tilt (6 dB per
/// octave) the first lifts a reading by under 0.01 dB, and so does the Fold
/// at its widest (`SPECTRAL_WIDTH_MAX`, 50 cents, with taps out to three of
/// them): a Gaussian mean of neighbours each at its own floor stands
/// `exp(σ²k²/2)` over the centre's, with `σ` in buckets and `k` the tilt's
/// slope in the natural log of power per bucket.
/// The curve falls through the margin in about 35 ms at the default release.
const DRAWN_MARGIN_DB: f32 = 1.0;

/// Whether some bucket of `columns` holds a power that draws: over
/// [`DRAWN_MARGIN_DB`] under `faintest`, the faintest dB its readings draw at
/// the bucket's pitch, and over
/// [`POWER_FLOOR`](harmonigraph_core::spectrogram::POWER_FLOOR), which every
/// reading takes for silence. `peak` is the loudest bucket of any of them.
/// The one test of "drawn" both readings of the spectrum stamp ([`Heard`]).
///
/// The faintest dB is affine in pitch and the buckets are evenly spaced in
/// it, so the faintest powers are a geometric run, walked by one multiply a
/// bucket, and walked only while `peak` is over the faintest of them all:
/// silence, which a host streams with its transport stopped, walks nothing.
/// Columns land under the editor's lock.
fn draws(faintest: impl Fn(f32) -> f32, peak: f32, columns: &[&SpectrumBuckets]) -> bool {
    use harmonigraph_core::spectrum::{bucket_pitch, SPECTRUM_BINS};
    let silence = harmonigraph_core::spectrogram::POWER_FLOOR;
    let db = |bucket| f64::from(faintest(bucket_pitch(bucket)) - DRAWN_MARGIN_DB);
    let (first, last) = (db(0), db(SPECTRUM_BINS - 1));
    if peak <= (10f64.powf(first.min(last) / 10.0) as f32).max(silence) {
        return false;
    }
    let ratio = 10f64.powf((last - first) / 10.0 / (SPECTRUM_BINS - 1) as f64);
    let mut faintest = 10f64.powf(first / 10.0);
    for bucket in 0..SPECTRUM_BINS {
        let at = (faintest as f32).max(silence);
        if columns.iter().any(|column| column[bucket] > at && column[bucket].is_finite()) {
            return true;
        }
        faintest *= ratio;
    }
    false
}

/// What the analysis hands [`AudioSpectrum`], in the order it happened: a
/// source run starting over, a batch of audio arriving, or a column it
/// produced. The plugin's input worker makes them off the editor's lock and
/// gives them under it ([`AudioSpectrum::give`], #1559); the inline shells
/// apply each as it is made ([`AudioSpectrum::apply`]). They carry everything
/// the spectrum learns from the analysis, so neither entry takes anything
/// else from it.
pub enum Delivery {
    /// The retained source run restarted: forget the curve, keep the history.
    Restart,
    /// A batch arrived whose newest frame is at the shell time `newest`,
    /// which keeps the curve live even while the analyzer refills and makes
    /// no column.
    Arrival { newest: f64 },
    /// Boxed, because a block shorter than the 8 ms between columns delivers
    /// an arrival and seldom a column: at 64-frame blocks and 48 kHz six of
    /// seven deliveries are arrivals, and a list whose every slot was sized
    /// for a column (15.3 KB) kept 31 MB after one step that found the
    /// plugin's audio ring full.
    Column(Box<AnalyzedColumn>),
}

impl Delivery {
    /// The column this delivers, if it is one.
    pub fn column(&self) -> Option<&AnalyzedColumn> {
        match self {
            Delivery::Column(column) => Some(column),
            _ => None,
        }
    }
}

/// One raw spectrum the analysis produced, for [`AudioSpectrum`] to take.
#[derive(Clone)]
pub struct AnalyzedColumn {
    /// Shell time its window listened at: its amplitude centroid, which is the
    /// column lag before the newest sample it read.
    pub(crate) time: f64,
    pub(crate) power: SpectrumBuckets,
    /// Seconds of audio since the column before, which the curve's time
    /// constants are converted at.
    pub(crate) step: f64,
}

impl AnalyzedColumn {
    /// Shell time its window listened at.
    pub fn time(&self) -> f64 {
        self.time
    }

    /// Raw power per pitch bucket, as analyzed.
    pub fn power(&self) -> &[f32] {
        &self.power
    }
}

impl Default for AudioSpectrum {
    fn default() -> Self {
        AudioSpectrum {
            display_revision: 0,
            frame_fold: None,
            #[cfg(test)]
            fold_measurements: 0,
            display: [0.0; harmonigraph_core::spectrum::SPECTRUM_BINS],
            last_samples: None,
            last_audible: Heard::default(),
            pyramid: Default::default(),
            #[cfg(any(test, feature = "testing"))]
            columns_taken: 0,
        }
    }
}

impl SpectrumAnalysis {
    /// Analyze interleaved samples, `channels` to a frame, one spectrum per
    /// [`FFT_INTERVAL`](crate::spectrogram::FFT_INTERVAL) of audio in them, handing
    /// `deliver` the batch's arrival and then each column it completes.
    ///
    /// `origin` is frame zero of this run on the shell clock, which the
    /// feeder states from its own: the plugin maps its presentation clock
    /// once per take, the offline renderer counts frames from the start of
    /// the soundtrack, and the standalone synth counts the frames it rendered.
    /// So a column's date is a function of the samples before it and nothing
    /// else, and however the feeder chunks a run, the columns come out the
    /// same. A feeder whose clock moves under the run restarts it
    /// ([`restart`](Self::restart)) and states the new run's origin.
    ///
    /// The FFT runs here, on a grid of sample counts, rather than on a grid
    /// of frames. That is the whole point:
    /// the old gate (`now - last_fft >= FFT_INTERVAL`, evaluated once per UI
    /// pass) could only fire ON a frame boundary, so a 20 ms interval on a 60 Hz
    /// display fired every 33.3 ms — SLOWER than the 32 ms slabs the heatmap was
    /// cutting the window into. Slabs went empty and were filled by duplicating
    /// their neighbour, so a held column scrolled past about
    /// once a second; the columns that did arrive sat at a phase inside their
    /// slab that drifted with the frame clock; and capping the frame rate
    /// coarsened the picture in proportion. Counting samples makes the column
    /// grid exact, evenly spaced, and independent of how often — or how evenly —
    /// the shell draws.
    ///
    /// One call therefore costs as many FFTs as the audio it is handed contains
    /// hops. Normally that is a frame's worth (two or three), and the worst
    /// case is a batch as large as the shell's audio ring — 1.37 s of stereo
    /// at 48 kHz in the plugin, 170 columns, some 10 ms through Fast and 36
    /// through Precise — reachable only by a drain starved for that long,
    /// which then gets its heatmap back-filled with audio that really did
    /// happen. The ring holds samples, not time, so a higher rate's fewer
    /// columns cost what its longer frame adds back, and a lower rate's are
    /// more: 54 ms through Precise at 32 kHz.
    pub fn feed_source(
        &mut self,
        samples: &[f32],
        channels: usize,
        sample_rate: f32,
        origin: f64,
        config: &SpectrumConfig,
        mut deliver: impl FnMut(Delivery),
    ) {
        // A changed rate empties the analyzers' rings, so nothing comes out
        // until they have refilled; a changed frame carries what the new one
        // can use, so only a longer one waits, for the rest of it. The hop grid
        // keeps its phase across any gap rather than restarting on it.
        let settings = config.analyzer(sample_rate);
        let analyzer = match &mut self.analyzer {
            Some(analyzer) => {
                analyzer.configure(settings, channels);
                analyzer
            }
            None => {
                self.analyzer.insert(harmonigraph_analysis::ChannelBank::new(settings, channels))
            }
        };

        // FRAMES throughout: `samples` is interleaved, and a hop is an amount of
        // time. A partial frame at the end is left out, so the de-interleaving
        // in `push_frames` can never slip a channel.
        let channels = analyzer.channels();
        let batch = samples.len() / channels;
        if batch == 0 {
            return;
        }
        let sr = f64::from(sample_rate.max(1.0));
        let newest = origin + (self.frames_seen + batch as u64 - 1) as f64 / sr;
        deliver(Delivery::Arrival { newest });
        let hop = ((crate::spectrogram::FFT_INTERVAL * sr).round() as u64).max(1);
        // Columns fall on multiples of `hop` frames from the start of the
        // stream. Left at zero the first boundary would be frame 1 and every one
        // after it a frame early, which is harmless but makes the grid
        // impossible to state (or to test) in whole hops.
        if self.next_hop == 0 {
            self.next_hop = hop;
        }

        let mut fed = 0usize; // frames
        while fed < batch {
            // Feed exactly up to the next hop boundary, so a spectrum is taken
            // at every multiple of `hop` frames and nowhere else. `max(1)`
            // keeps the loop moving if a sample-rate change ever leaves the
            // boundary behind us; the next line puts the grid back on its feet.
            let want = self.next_hop.saturating_sub(self.frames_seen).max(1) as usize;
            let take = want.min(batch - fed);
            analyzer.push_frames(&samples[fed * channels..(fed + take) * channels]);
            self.frames_seen += take as u64;
            fed += take;
            if self.frames_seen < self.next_hop {
                break; // The batch ran out before the boundary.
            }
            self.next_hop = self.frames_seen + hop;
            let Some(fresh) = analyzer.power_sum() else { continue };

            // Stamped where the window it measured is centred, not at the
            // boundary itself — see `window_center_offset`. This is what lets a
            // ridge sit under the note ribbon that made it, which is the entire
            // point of drawing the two on one time axis. The boundary is where
            // the newest frame fed so far sits on the run's grid, so
            // consecutive columns are exactly `hop` frames apart.
            let boundary = origin + self.frames_seen.saturating_sub(1) as f64 / sr;
            let center = boundary - analyzer.window_center_offset();
            let column = AnalyzedColumn { time: center, power: fresh, step: hop as f64 / sr };
            deliver(Delivery::Column(Box::new(column)));
        }
    }

    /// The running analyzer's lag: how far behind the newest sample it read
    /// a column is dated. What a shell's tests read to tell which resolution
    /// it analyzed under; a frame reads [`SpectrumConfig::live_lag`].
    #[cfg(any(test, feature = "testing"))]
    pub fn lag(&self) -> f64 {
        self.analyzer.as_ref().expect("the lag of an analysis fed nothing").window_center_offset()
    }

    /// Forget the retained run: the analyzer's window and the hop grid. The
    /// spectrum's half is a [`Delivery::Restart`].
    pub fn restart(&mut self) {
        if let Some(analyzer) = &mut self.analyzer {
            analyzer.restart();
        }
        self.frames_seen = 0;
        self.next_hop = 0;
    }
}

impl AudioSpectrum {
    /// Advance the curve by one analyzed column and keep it as history.
    ///
    /// The curve's attack and release run here, per column, for the reason the
    /// FFT runs on a grid of sample counts ([`SpectrumAnalysis::feed_source`]):
    /// on the frame clock their time constants would depend on the frame rate.
    ///
    /// Answers the batch it came in if it drew, in itself or in the curve.
    fn take(&mut self, column: AnalyzedColumn, config: &SpectrumConfig) -> Option<f64> {
        // Two coefficients, chosen per bucket by which way it is moving.
        // Derived from the hop actually in use rather than set on the bar,
        // so the times mean seconds at any hop this loop runs at.
        self.display_revision = self.display_revision.wrapping_add(1);
        let attack = hop_alpha(config.attack, column.step);
        let release = hop_alpha(config.release, column.step);
        let (mut loudest, mut peak) = (0.0_f32, 0.0_f32);
        for (shown, new) in self.display.iter_mut().zip(&column.power) {
            // Invalid input must not poison the carried smoothing state
            // after clean audio has replaced the analyzer's window.
            let new = if new.is_finite() { *new } else { 0.0 };
            // POWER, so "louder" is the same comparison in dB — the levels
            // are mapped through `loudness` well downstream of here.
            let alpha = if new > *shown { attack } else { release };
            *shown += (new - *shown) * alpha;
            loudest = loudest.max(new);
            peak = peak.max(*shown);
        }
        use crate::panes::spectral::axes::{faintest_coloured_db, faintest_drawn_db};
        // The heatmap reads the raw column through the volume colours alone.
        let column_drawn =
            draws(|midi| faintest_coloured_db(config, midi), loudest, &[&column.power]);
        // Every batch delivers its arrival before its columns.
        let mut drew = None;
        if let Some(batch) = self.last_samples {
            // The raw column as well as the curve: the heatmap's fresh edge
            // draws it while the curve's attack has yet to lift the display.
            let drawn = column_drawn
                || draws(
                    |midi| faintest_drawn_db(config, midi),
                    loudest.max(peak),
                    &[&column.power, &self.display],
                );
            drew = drawn.then_some(batch);
            let until = &mut self.last_audible.curve_until;
            *until = if drawn {
                Some(batch + Self::HOLD_SECONDS)
            } else {
                until.map(|until| until.min(batch))
            };
        }
        // Keep the RAW spectrum for the spectrogram (the smoothed
        // `display` would smear one column into the next).
        self.keep(column.time, &column.power, column_drawn);
        drew
    }

    /// Fold one raw spectrum into the history, through its one entry
    /// ([`SlabPyramid::push`](crate::spectrogram::SlabPyramid::push), which
    /// asserts that dates only move forward), and stamp [`Heard::column`] if
    /// it was `drawn`.
    ///
    /// Retention is deliberately NOT keyed to the current span: trimming to the
    /// live `roll_seconds` meant shrinking the span popped columns off the
    /// front, and widening it again could never bring them back — the span
    /// control silently erased spectrogram history. Each rung keeps the
    /// longest span's reach or its ring, whichever is fewer slabs (see the
    /// pyramid's retention), and the heatmap reads back as far as the span
    /// asks.
    fn keep(&mut self, time: f64, power: &SpectrumBuckets, drawn: bool) {
        #[cfg(any(test, feature = "testing"))]
        {
            self.columns_taken += 1;
        }
        self.pyramid.push(time, power);
        if drawn {
            self.last_audible.column = Some(time);
        }
    }

    /// Forget the curve along with the run it was measured from; the history
    /// stays, and so does [`last_audible`](Self::last_audible), which may
    /// date a column still in it. See [`Delivery::Restart`].
    fn restart(&mut self) {
        self.last_samples = None;
        self.display.fill(0.0);
        self.display_revision = self.display_revision.wrapping_add(1);
        self.frame_fold = None;
    }
}

/// The one-step coefficient of an exponential approach with time constant
/// `seconds`, taken `dt` seconds at a time.
///
/// `1 - exp(-dt/tau)`, which is what makes a TIME the thing set and the
/// coefficient the thing derived: the same `seconds` is the same filter at any
/// step size, where a raw coefficient silently means a different filter as soon
/// as the step changes.
///
/// The two degenerate cases mean opposite things and are worth keeping apart:
///
/// - **No time PASSED** holds, returning 0. This is what a pane drawn twice in
///   one frame hits — the docked lattice and the Video tab's preview, off one
///   clock — and landing on the target there would run the filter at twice its
///   speed whenever both are on screen.
/// - **No time ASKED FOR** lands, returning 1. That is the bar's own off
///   position, and a non-finite time takes it too rather than answering NaN
///   into every bucket.
///
/// A time long enough to freeze the display is not caught here and is not
/// meant to be: the coefficient it asks for rounds to 0 in f32, which is a
/// filter that never arrives. What keeps it off this function is
/// [`SpectrumConfig::sanitize`](crate::SpectrumConfig), which fits a
/// deserialized time to the bar's own range.
pub(crate) fn hop_alpha(seconds: f32, dt: f64) -> f32 {
    // A NaN clock holds rather than lands, on the same argument the zero step
    // does: a step nobody can measure is not evidence that time passed. An
    // INFINITE one falls through and lands, which the arithmetic below reaches
    // on its own.
    if dt.is_nan() || dt <= 0.0 {
        return 0.0;
    }
    if !seconds.is_finite() || seconds <= 0.0 {
        return 1.0;
    }
    1.0 - (-dt / f64::from(seconds)).exp() as f32
}

impl AudioSpectrum {
    /// How long after the last samples the curve keeps drawing.
    pub(crate) const HOLD_SECONDS: f64 = 0.5;

    /// Keep what an analysis made, in order; `config` is the one it analyzed
    /// under. The plugin's input worker takes its audio and analyzes it off
    /// the editor's lock, and gives the result under it.
    ///
    /// A Clear pressed while the worker analyzes does not reach the columns
    /// it is making: they join the emptied history, at most one step's audio
    /// from just before the press (the worker polls every 4 ms while the
    /// window is open), as if the press had come that much sooner.
    ///
    /// Whether it brought sound back to a picture of the spectrum that had
    /// gone still, each by its own stamp ([`Heard`]), so a still window has to
    /// be asked to draw it:
    ///
    /// - The smoothed display: a column that draws for it, from a batch past
    ///   the bound it moved until before the hand-over
    ///   ([`Heard::curve_until`]), or the first.
    /// - The heatmap: a column it draws, more than
    ///   [`HOLD_SECONDS`](Self::HOLD_SECONDS) after the one it last drew, or
    ///   the first. Its own, since the curve's bound keeps moving through a
    ///   passage only the curve draws, and with the curve off screen nothing
    ///   asks for frames while it does. Its reach after a column is a Span at
    ///   least, longer than the hold, so it has gone still by then.
    ///
    /// Sound that goes on asks nothing more: a picture that shows it keeps
    /// the window drawing, and one that does not has nothing new to draw.
    pub fn give(
        &mut self,
        deliveries: impl IntoIterator<Item = Delivery>,
        config: &SpectrumConfig,
    ) -> bool {
        let Heard { column, curve_until } = self.last_audible;
        let mut back = false;
        for delivery in deliveries {
            let drawn = self.land(delivery, config);
            back |= drawn.is_some_and(|batch| curve_until.is_none_or(|still| batch > still));
        }
        let heatmap = match (column, self.last_audible.column) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(was), Some(drawn)) => drawn > was + Self::HOLD_SECONDS,
        };
        back || heatmap
    }

    /// Keep one delivery as it is made: an inline shell's
    /// [`feed_source`](SpectrumAnalysis::feed_source) delivers here.
    pub fn apply(&mut self, delivery: Delivery, config: &SpectrumConfig) {
        self.land(delivery, config);
    }

    /// Keep one delivery, and the batch a column that draws came in.
    fn land(&mut self, delivery: Delivery, config: &SpectrumConfig) -> Option<f64> {
        match delivery {
            Delivery::Restart => self.restart(),
            Delivery::Arrival { newest } => self.last_samples = Some(newest),
            Delivery::Column(column) => return self.take(*column, config),
        }
        None
    }

    /// The curve to draw, or None while no audio is flowing. The levels are
    /// maintained per column as each is delivered; this
    /// only decides whether they are still live.
    pub fn display(&self, now: f64) -> Option<&SpectrumBuckets> {
        self.last_samples.is_some_and(|t| now - t <= Self::HOLD_SECONDS).then_some(&self.display)
    }

    /// A column straight into the history, for a test that needs history
    /// without analyzing audio. See [`AudioSpectrum::keep`].
    #[cfg(test)]
    /// Stamped as drawn under the fresh look's colours.
    pub(crate) fn push_history(&mut self, now: f64, power: &SpectrumBuckets) {
        let config = SpectrumConfig::default();
        let faintest = |midi| crate::panes::spectral::axes::faintest_coloured_db(&config, midi);
        let loudest = power.iter().copied().fold(0.0, f32::max);
        self.keep(now, power, draws(faintest, loudest, &[power]));
    }

    /// The slab pyramid the heatmap reads, every level folded up to the
    /// newest column.
    pub(crate) fn pyramid(&mut self) -> &mut crate::spectrogram::SlabPyramid {
        &mut self.pyramid
    }

    /// Forget the spectrogram history (paired with clearing the roll), and
    /// [`Heard::column`] with the columns it dated.
    pub fn clear_history(&mut self) {
        self.pyramid.clear();
        self.last_audible.column = None;
    }

    /// Which Clear of the history this is, so the heatmap's colour memory can
    /// tell an emptied picture from one that scrolled on.
    pub fn history_epoch(&self) -> u32 {
        self.pyramid.generation()
    }

    /// How many columns the history has taken since the spectrum was made: a
    /// count that only grows, so a Clear does not hide what a step kept, and
    /// nothing has to read the pyramid to tell.
    #[cfg(any(test, feature = "testing"))]
    pub fn columns_taken(&self) -> u64 {
        self.columns_taken
    }

    /// The date of the newest column the history holds, `None` while it is
    /// empty.
    pub fn newest_column(&self) -> Option<f64> {
        self.pyramid.newest()
    }

    /// When each reading of the spectrum last drew anything: see [`Heard`].
    pub(crate) fn last_audible(&self) -> Heard {
        self.last_audible
    }

    /// When the smoothed display stops moving, which the curve, the Spiral and
    /// the lattice ring all draw: [`Heard::curve_until`], which each landing
    /// column writes. While the columns leave something drawn it is
    /// [`HOLD_SECONDS`](Self::HOLD_SECONDS) after their samples, the same
    /// bound [`display`](Self::display) hides the curve at, and the display's
    /// release is inside it; once one leaves nothing drawn it is that column's
    /// samples. `None` before anything has been drawn.
    pub(crate) fn moving_until(&self) -> Option<f64> {
        self.last_audible().curve_until
    }

    /// Whether the smoothed display is still moving at `now`: see
    /// [`moving_until`](Self::moving_until).
    pub(crate) fn moving(&self, now: f64) -> bool {
        self.moving_until().is_some_and(|until| until >= now)
    }
}

impl AudioSpectrum {
    /// The folded grid, or `None` while no audio is flowing.
    ///
    /// `now` decides only whether there is a curve to fold at all — the hold
    /// window in [`display`](Self::display) — and never what the fold measures,
    /// so it is not part of the memo's key. It was, and that made the memo a
    /// one-frame one: it restarted on every repaint, which is often enough to
    /// hide anything the carry-forward path gets wrong.
    pub(crate) fn folded(&mut self, now: f64, width: f32) -> Option<&SpectrumBuckets> {
        self.display(now)?;
        let key = (self.display_revision, width.to_bits());
        if self.frame_fold.as_ref().is_none_or(|(revision, bits, _)| (*revision, *bits) != key) {
            self.frame_fold = Some((
                key.0,
                key.1,
                crate::panes::spectral_fold::Fold::measure(&self.display, width),
            ));
            #[cfg(test)]
            {
                self.fold_measurements += 1;
            }
        }
        self.frame_fold.as_ref().map(|(_, _, fold)| fold.grid())
    }
}

/// A test shell's feed: `samples` through `analysis` into `spectrum` as the
/// next stretch of a run whose frame zero is at `origin`, each delivery
/// applied as it is made, and the columns it made, as a delivery tap.
#[cfg(test)]
pub(crate) fn feed(
    spectrum: &mut AudioSpectrum,
    analysis: &mut SpectrumAnalysis,
    samples: &[f32],
    channels: usize,
    sample_rate: f32,
    origin: f64,
    config: &SpectrumConfig,
) -> Vec<AnalyzedColumn> {
    let mut columns = Vec::new();
    analysis.feed_source(samples, channels, sample_rate, origin, config, |delivery| {
        columns.extend(delivery.column().cloned());
        spectrum.apply(delivery, config)
    });
    columns
}

/// A test shell's feeder for a run fed in several batches: its analysis, and
/// the run's frame zero, put where the first batch's newest frame is heard at
/// the `now` it is fed at. Later batches are dated by counting frames from
/// there, as every shipped feeder dates them, whatever `now` they are fed at.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestFeed {
    analysis: SpectrumAnalysis,
    origin: Option<f64>,
}

#[cfg(test)]
impl TestFeed {
    /// The next batch of the run: [`feed`].
    pub(crate) fn feed(
        &mut self,
        spectrum: &mut AudioSpectrum,
        samples: &[f32],
        channels: usize,
        sample_rate: f32,
        now: f64,
        config: &SpectrumConfig,
    ) -> Vec<AnalyzedColumn> {
        let frames = (samples.len() / channels.max(1)).saturating_sub(1);
        let origin =
            *self.origin.get_or_insert(now - frames as f64 / f64::from(sample_rate.max(1.0)));
        feed(spectrum, &mut self.analysis, samples, channels, sample_rate, origin, config)
    }
}

/// One batch through a fresh analysis, dated so its newest frame was heard at
/// `now`, for a test that analyzes one stretch of audio: [`TestFeed`].
#[cfg(test)]
pub(crate) fn heard_at(
    spectrum: &mut AudioSpectrum,
    samples: &[f32],
    channels: usize,
    sample_rate: f32,
    now: f64,
    config: &SpectrumConfig,
) -> Vec<AnalyzedColumn> {
    TestFeed::default().feed(spectrum, samples, channels, sample_rate, now, config)
}

#[cfg(test)]
mod tests {
    use super::AudioSpectrum;
    use crate::SpectrumConfig;

    /// The behavior `now` in the key made unreachable: a repaint that brought
    /// no new column reuses the measurement instead of taking it again.
    ///
    /// Beside its siblings in `tests::spectrum`, which cover the key's other
    /// two inputs (a width edit, and audio pushed at the same shell time,
    /// which the revision sees and a clock in the key would not)
    /// and its hold-window early return.
    #[test]
    fn a_fold_outlives_a_frame_that_brought_no_new_column() {
        let cfg = SpectrumConfig::default();
        let mut spectrum = AudioSpectrum::default();
        let tone = |frequency: f32| {
            (0..cfg.window.frame(48_000.0) * 2)
                .map(|i| (std::f32::consts::TAU * frequency * i as f32 / 48_000.0).sin() * 0.5)
                .collect::<Vec<_>>()
        };
        let mut feed = crate::spectrum::TestFeed::default();
        feed.feed(&mut spectrum, &tone(440.0), 1, 48_000.0, 1.0, &cfg);
        let first = spectrum.folded(1.0, 2.0).unwrap().to_vec();
        assert!(first.iter().any(|v| *v > 0.0), "the fixture must contain measured energy");
        assert_eq!(spectrum.fold_measurements, 1);

        // Ten 60 Hz frames, all inside HOLD_SECONDS so the curve is still
        // drawn, and none of them pushing audio. Under a key carrying `now`
        // each one measured the same numbers again.
        for frame in 1..=10 {
            let now = 1.0 + f64::from(frame) / 60.0;
            assert!(now - 1.0 < AudioSpectrum::HOLD_SECONDS, "the fixture must stay in hold");
            assert_eq!(spectrum.folded(now, 2.0).unwrap().as_slice(), first);
        }
        assert_eq!(spectrum.fold_measurements, 1, "a new frame is not a new measurement");

        // A new column at a new frame time still invalidates it: the next
        // batch of the run, which ends that much later.
        let later = 1.0 + tone(660.0).len() as f64 / 48_000.0;
        feed.feed(&mut spectrum, &tone(660.0), 1, 48_000.0, later, &cfg);
        let next = spectrum.folded(later, 2.0).unwrap().to_vec();
        assert_ne!(next, first, "new audio must be measured");
        assert_eq!(spectrum.fold_measurements, 2);
    }
}
