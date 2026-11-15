//! The audio-derived spectrum behind the Spectral pane: the analyzer that
//! turns incoming samples into columns, and the store those columns age out
//! of. Separate surface storage describes the uploaded heatmap textures.
//! Runtime-only — none of this is persisted.

use crate::SpectrumConfig;

/// One power value per pitch-spectrum bucket, the array the analyzer fills
/// and the pane draws. See [`harmonigraph_core::spectrum::SPECTRUM_BINS`].
pub(crate) type SpectrumBuckets = [f32; harmonigraph_core::spectrum::SPECTRUM_BINS];

/// Audio-derived pitch spectrum shown in the Spectral pane: what the columns
/// a feeder's [`SpectrumAnalysis`] makes become. Each shell owns the analysis
/// over its own audio and dates it on its own clock (plugin: the selected
/// main/sidechain input, through its input worker; standalone: the mock synth;
/// offline: the take's soundtrack), and hands what it made here. Runtime-only.
pub struct AudioSpectrum {
    /// What the columns become: the curve, the history and their memos.
    pub(crate) store: SpectrumStore,
}

/// Samples in, raw columns out, on an exact grid of sample counts, dated from
/// the frame zero its feeder states. It never reads what it has produced, so
/// it runs where the store is not (#1559): the plugin's input worker owns its
/// own and analyzes off the editor's lock.
pub struct SpectrumAnalysis {
    /// One analyzer per input channel, combined in the power domain — see
    /// [`ChannelBank`](harmonigraph_analysis::ChannelBank).
    pub(crate) analyzer: harmonigraph_analysis::ChannelBank,
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

/// What [`AudioSpectrum`] keeps of the columns: the curve each column advances, the
/// history it joins, and the memo of the curve's fold.
pub(crate) struct SpectrumStore {
    /// Changed only when the smoothed grid changes. This identity lives beside
    /// the measurement keyed on it, so replacing the store, or resetting it
    /// with [`SpectrumStore::restart`], drops both together and no fold
    /// outlives the display it was measured from. Resetting the analysis alone
    /// does not, so whoever restarts it delivers a [`Delivery::Restart`] too.
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
    /// When the spectrum last held anything above silence, for
    /// [`Animating`](crate::Animating), which keeps drawing at full rate only
    /// while what that shows can still be in view or still settling. Samples
    /// arriving say nothing about it, because a host streams silence with its
    /// transport stopped.
    pub(crate) last_audible: Heard,
    /// The heatmap's history: every raw (unsmoothed) column folded into a
    /// slab per rung as it arrives, bounded by each rung's ring. See
    /// [`SlabPyramid`](crate::spectrogram::SlabPyramid) and
    /// [`SpectrumStore::keep`].
    pub(crate) pyramid: crate::spectrogram::SlabPyramid,
    /// Columns offered to the history, dropped or not: see
    /// [`AudioSpectrum::columns_taken`].
    columns_taken: u64,
    /// Clear display history presses, which decide whether a hand-over's
    /// columns join the history ([`AudioSpectrum::give`]).
    clears: u32,
    /// Columns the guard dropped, and the times it or a crash in the fold
    /// reset the history: see [`AudioSpectrum::guard_counts`].
    drops: u32,
    resets: u32,
    /// The lag of the analysis as of the newest batch's arrival, which the
    /// newest columns were made at unless a resolution change is still
    /// refilling the window: see [`AudioSpectrum::column_lag`].
    lag: f64,
}

/// The times of the newest columns that held anything above silence
/// ([`POWER_FLOOR`](harmonigraph_core::spectrogram::POWER_FLOOR)), one per
/// reading of the spectrum, `None` until one has.
///
/// Two, because the readings fall silent apart: the curve's release carries
/// it seconds past the sound, and a column the heatmap shows can leave the
/// curve under the floor. Stamping both on one clock made a Clear's silent
/// columns look heard for as long as the curve was still falling.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Heard {
    /// The newest raw column the history folded: what the heatmap scrolls.
    pub(crate) column: Option<f64>,
    /// The newest column after which the smoothed curve still stood above
    /// silence: what the curve, the Spiral and the lattice ring draw.
    pub(crate) curve: Option<f64>,
}

/// What the analysis hands the store, in the order it happened: a source
/// run starting over, a batch of audio arriving, or a column it produced.
/// The plugin's input worker makes them off the editor's lock and gives
/// them under it ([`AudioSpectrum::give`], #1559); the inline shells apply
/// each as it is made ([`AudioSpectrum::apply`]). They carry everything the
/// store learns from the analysis, so neither entry takes anything else from
/// it.
pub enum Delivery {
    /// The retained source run restarted: forget the curve, keep the history.
    Restart,
    /// A batch arrived, analyzed at a lag of `lag` seconds (see
    /// [`AudioSpectrum::column_lag`]), whose newest frame is at the shell time
    /// `newest`, which keeps the curve live even while the analyzer refills
    /// and makes no column.
    Arrival { newest: f64, lag: f64 },
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

/// One raw spectrum the analysis produced, for the store to take.
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

/// Every drawing surface's heatmap state, indexed by the surface id its copy of
/// the pane was drawn with: 0 the docked Spectral pane, 1 the Video pane's
/// preview, and offline the placement's index in the layout (see
/// [`draw_pane`](crate::draw_pane)).
///
/// Each surface reads a run of its own: the slab width comes off the pane's
/// own depth in points, so two placements at different sizes sharing one
/// surface would each replace the other's run every frame.
///
/// What stays SHARED is the analyzer and the slab pyramid it fills
/// ([`SpectrumStore::pyramid`]): there is one selected input and one stream of
/// columns, and a surface holds only the run it read out of them.
#[derive(Default)]
pub(crate) struct SpectrogramSurfaces(Vec<SpectrogramSurface>);

impl SpectrogramSurfaces {
    /// The surface's own state, empty on first sight of its id.
    ///
    /// A method on the field rather than on [`AudioSpectrum`], so a caller
    /// reading the shared pyramid can hold that borrow while this one is out.
    pub(crate) fn at(&mut self, surface: usize) -> &mut SpectrogramSurface {
        if self.0.len() <= surface {
            self.0.resize_with(surface + 1, SpectrogramSurface::default);
        }
        &mut self.0[surface]
    }

    /// Every surface an id has been asked for, for the readings and the
    /// teardowns that are about all of them at once.
    pub(crate) fn all(&self) -> impl Iterator<Item = &SpectrogramSurface> {
        self.0.iter()
    }
}

/// One drawing surface's heatmap: the immutable snapshot of the run it drew.
/// The renderer separately owns its uploaded copy.
///
/// Runtime-only, never persisted, and rebuilt from the pyramid when dropped,
/// so a default is always a safe state.
#[derive(Default)]
pub(crate) struct SpectrogramSurface {
    /// The run and palette reused across frames. See
    /// [`FoldedGrid`](crate::spectrogram::FoldedGrid).
    pub(crate) folded: crate::spectrogram::FoldedGrid,
    /// The pyramid level the previous frame drew at, which is what gives
    /// [`live_slab`](crate::spectrogram::live_slab)'s ladder its hysteresis —
    /// see [`Plan::new`](crate::spectrogram::Plan::new). `None` before the first frame.
    pub(crate) held_level: Option<usize>,
}

impl Default for SpectrumAnalysis {
    fn default() -> Self {
        SpectrumAnalysis {
            analyzer: harmonigraph_analysis::ChannelBank::new(Default::default(), 1),
            frames_seen: 0,
            next_hop: 0,
        }
    }
}

impl Default for AudioSpectrum {
    fn default() -> Self {
        AudioSpectrum {
            store: SpectrumStore {
                display_revision: 0,
                frame_fold: None,
                #[cfg(test)]
                fold_measurements: 0,
                display: [0.0; harmonigraph_core::spectrum::SPECTRUM_BINS],
                last_samples: None,
                last_audible: Heard::default(),
                pyramid: Default::default(),
                columns_taken: 0,
                clears: 0,
                drops: 0,
                resets: 0,
                // A default analyzer's, until an analysis hands its own over.
                lag: harmonigraph_analysis::AnalyzerConfig::default().column_lag(),
            },
        }
    }
}

impl SpectrumAnalysis {
    /// Analyze interleaved samples, `channels` to a frame, one spectrum per
    /// [`FFT_INTERVAL`](AudioSpectrum::FFT_INTERVAL) of audio in them, handing
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
    /// their neighbour (`JITTER_SLABS`), so a held column scrolled past about
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
        self.analyzer.configure(config.analyzer(sample_rate), channels);

        // FRAMES throughout: `samples` is interleaved, and a hop is an amount of
        // time. A partial frame at the end is left out, so the de-interleaving
        // in `push_frames` can never slip a channel.
        let channels = self.analyzer.channels();
        let batch = samples.len() / channels;
        if batch == 0 {
            return;
        }
        let sr = f64::from(sample_rate.max(1.0));
        let newest = origin + (self.frames_seen + batch as u64 - 1) as f64 / sr;
        deliver(Delivery::Arrival { newest, lag: self.lag() });
        let hop = ((AudioSpectrum::FFT_INTERVAL * sr).round() as u64).max(1);
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
            self.analyzer.push_frames(&samples[fed * channels..(fed + take) * channels]);
            self.frames_seen += take as u64;
            fed += take;
            if self.frames_seen < self.next_hop {
                break; // The batch ran out before the boundary.
            }
            self.next_hop = self.frames_seen + hop;
            let Some(fresh) = self.analyzer.power_sum() else { continue };

            // Stamped where the window it measured is centred, not at the
            // boundary itself — see `window_center_offset`. This is what lets a
            // ridge sit under the note ribbon that made it, which is the entire
            // point of drawing the two on one time axis. The boundary is where
            // the newest frame fed so far sits on the run's grid, so
            // consecutive columns are exactly `hop` frames apart.
            let boundary = origin + self.frames_seen.saturating_sub(1) as f64 / sr;
            let center = boundary - self.analyzer.window_center_offset();
            let column = AnalyzedColumn { time: center, power: fresh, step: hop as f64 / sr };
            deliver(Delivery::Column(Box::new(column)));
        }
    }

    /// The running analyzer's lag, which each [`Delivery::Arrival`] carries
    /// to the store: see [`AudioSpectrum::column_lag`].
    pub fn lag(&self) -> f64 {
        self.analyzer.window_center_offset()
    }

    /// Forget the retained run: the analyzer's window and the hop grid. The
    /// store's half is a [`Delivery::Restart`].
    pub fn restart(&mut self) {
        self.analyzer.restart();
        self.frames_seen = 0;
        self.next_hop = 0;
    }
}

impl SpectrumStore {
    /// Advance the curve by one analyzed column and, if `keep`, keep it as
    /// history.
    ///
    /// The curve's attack and release run here, per column, for the reason the
    /// FFT runs on a grid of sample counts ([`SpectrumAnalysis::feed_source`]):
    /// on the frame clock their time constants would depend on the frame rate.
    pub(crate) fn take(&mut self, column: AnalyzedColumn, config: &SpectrumConfig, keep: bool) {
        // Two coefficients, chosen per bucket by which way it is moving.
        // Derived from the hop actually in use rather than set on the bar,
        // so the times mean seconds at any hop this loop runs at.
        self.display_revision = self.display_revision.wrapping_add(1);
        let attack = hop_alpha(config.attack, column.step);
        let release = hop_alpha(config.release, column.step);
        let floor = harmonigraph_core::spectrogram::POWER_FLOOR;
        let (mut column_heard, mut curve_heard) = (false, false);
        for (shown, new) in self.display.iter_mut().zip(&column.power) {
            // Invalid input must not poison the carried smoothing state
            // after clean audio has replaced the analyzer's window.
            let new = if new.is_finite() { *new } else { 0.0 };
            // POWER, so "louder" is the same comparison in dB — the levels
            // are mapped through `loudness` well downstream of here.
            let alpha = if new > *shown { attack } else { release };
            *shown += (new - *shown) * alpha;
            column_heard |= new > floor;
            curve_heard |= *shown > floor;
        }
        if curve_heard {
            self.last_audible.curve = Some(column.time);
        }
        // Keep the RAW spectrum for the spectrogram (the smoothed
        // `display` would smear one column into the next). Retention is
        // span-INDEPENDENT (see `keep`): shrinking the span and widening it
        // again must not lose the history in between.
        if keep {
            let newest = self.pyramid.newest();
            let verdict = self.keep(column.time, &column.power, column_heard);
            debug_assert!(
                verdict == crate::spectrogram::Admit::Fold || backward_dates_expected(),
                "history dates must strictly increase: {} after {newest:?}",
                column.time,
            );
        }
    }

    /// Fold one raw spectrum into the history, through the guard at its one
    /// entry ([`SlabPyramid::push`](crate::spectrogram::SlabPyramid::push)):
    /// a column up to a second at or before the newest is dropped, and one
    /// further back resets the history and starts it again. Either is counted
    /// for the overlay, and `heard` stamps [`Heard::column`] only for a
    /// column that was folded. The guard's verdict.
    ///
    /// Retention is deliberately NOT keyed to the current span: trimming to the
    /// live `roll_seconds` meant shrinking the span popped columns off the
    /// front, and widening it again could never bring them back — the span
    /// control silently erased spectrogram history. Each rung keeps what the
    /// longest span can reach (see the pyramid's retention), and the heatmap
    /// reads back as far as the span asks.
    ///
    /// Every feeder dates a run from its stated frame zero
    /// ([`SpectrumAnalysis::feed_source`]), so a run's columns step forward by
    /// whole hops, and a new run starts after the old one: the plugin's clock
    /// offset only grows and a restarted analyzer's first column waits for a
    /// full frame; the offline renderer's one run is its soundtrack; and the
    /// standalone synth, after a stall, starts a new run whose first column
    /// dates after the last of the run before, however its resolution changed.
    /// So [`take`](Self::take) asserts the order in debug builds, and the
    /// guard is what release builds have instead.
    ///
    /// A crash inside the fold leaves the levels part-folded, which a
    /// write-once history would draw for as long as a level keeps its slabs,
    /// so it resets the history on the way out: the history is what cannot be
    /// trusted, and a crash anywhere else in the worker costs nothing.
    pub(crate) fn keep(
        &mut self,
        time: f64,
        power: &SpectrumBuckets,
        heard: bool,
    ) -> crate::spectrogram::Admit {
        use crate::spectrogram::Admit;
        self.columns_taken += 1;
        /// Resets the history if the fold it guards unwinds.
        struct ResetOnUnwind<'a>(&'a mut SpectrumStore);
        impl Drop for ResetOnUnwind<'_> {
            fn drop(&mut self) {
                if std::thread::panicking() {
                    self.0.resets = self.0.resets.saturating_add(1);
                    self.0.reset();
                }
            }
        }
        let guard = ResetOnUnwind(self);
        let verdict = guard.0.pyramid.push(time, power);
        drop(guard);
        match verdict {
            Admit::Drop => self.drops = self.drops.saturating_add(1),
            Admit::Reset => {
                // The pyramid emptied itself before folding the column.
                self.resets = self.resets.saturating_add(1);
                self.last_audible.column = None;
            }
            Admit::Fold => {}
        }
        if heard && verdict != Admit::Drop {
            self.last_audible.column = Some(time);
        }
        verdict
    }

    /// Empty the history, which moves the pyramid's generation on: what a
    /// Clear and a crash in the fold do, and the guard's reset does inside
    /// the pyramid. [`Heard::column`] goes with the columns it dated.
    fn reset(&mut self) {
        self.pyramid.clear();
        self.last_audible.column = None;
    }

    /// Apply one delivery, as [`AudioSpectrum::give`] describes.
    /// `keep` false moves the curve without keeping the column as history.
    fn apply(&mut self, delivery: Delivery, config: &SpectrumConfig, keep: bool) {
        match delivery {
            Delivery::Restart => self.restart(),
            Delivery::Arrival { newest, lag } => {
                self.last_samples = Some(newest);
                self.lag = lag;
            }
            Delivery::Column(column) => self.take(*column, config, keep),
        }
    }

    /// Forget the curve along with the run it was measured from; the history
    /// stays, and so does [`last_audible`](Self::last_audible), which may
    /// date a column still in it. See [`Delivery::Restart`].
    pub(crate) fn restart(&mut self) {
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
    /// Seconds of AUDIO between FFTs (125 columns a second), measured in
    /// samples rather than on the shell clock — see
    /// [`feed_source`](SpectrumAnalysis::feed_source).
    ///
    /// A column costs the slab it lands in and nothing else (see
    /// `spectrogram::FoldedGrid`), so the rate buys smoothness at the newest edge
    /// almost for free, and no REACH: each level of the slab pyramid keeps
    /// slabs, not columns, so the rate decides only what a column costs to
    /// fold.
    ///
    /// At 8 ms this is a picture setting and not an analysis one: the window
    /// is untouched, so what a column RESOLVES is unchanged, and overlapping
    /// that same window more finely just draws the time axis at 2.5x the
    /// resolution 20 ms reaches (via
    /// [`live_slab`](crate::spectrogram::live_slab), whose ladder is
    /// rungs of THIS interval, so the picture's grid tracks it).
    /// A column costs about 30 µs per channel through Fast at 48 kHz and 106
    /// through Precise (#1597's measurement) — up to 2.7% of a core in stereo,
    /// against 1.1% at 20 ms. The frame is a length of time
    /// ([`SpectrumWindow::frame`](crate::SpectrumWindow::frame)), so a column
    /// costs about twice that at 88.2 and 96 kHz and four times at 176.4 and
    /// 192.
    pub(crate) const FFT_INTERVAL: f64 = 0.008;
    /// How long after the last samples the curve keeps drawing.
    pub(crate) const HOLD_SECONDS: f64 = 0.5;

    /// Keep what an analysis made from audio taken when the history had been
    /// cleared [`clears`](Self::clears) times, in order; `config` is the one
    /// it analyzed under. The plugin's input worker takes its audio and
    /// analyzes it off the editor's lock, and gives the result under it.
    ///
    /// If the history has been cleared since, its columns still move the curve
    /// but do not join the history, which the Clear was asked to empty of
    /// everything before it. A reset by the guard is not a Clear: the columns
    /// after it still join the history it started again.
    ///
    /// Whether it brought sound back to a spectrum that had gone still: a
    /// column heard more than [`HOLD_SECONDS`](Self::HOLD_SECONDS) after the
    /// one heard before it on the same reading, or the first. By then every
    /// picture of it had stopped moving (the curve a hold after its last
    /// heard column, the heatmap later still), so a still window has to be
    /// asked to draw it. Sound that goes on asks nothing more: a picture
    /// that shows it keeps the window drawing, and one that does not has
    /// nothing new to draw.
    pub fn give(
        &mut self,
        clears: u32,
        deliveries: impl IntoIterator<Item = Delivery>,
        config: &SpectrumConfig,
    ) -> bool {
        let keep = clears == self.clears();
        let before = self.store.last_audible;
        for delivery in deliveries {
            self.store.apply(delivery, config, keep);
        }
        let after = self.store.last_audible;
        let resumed = |was: Option<f64>, heard: Option<f64>| match (was, heard) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(was), Some(heard)) => heard > was + Self::HOLD_SECONDS,
        };
        resumed(before.curve, after.curve) || resumed(before.column, after.column)
    }

    /// Keep one delivery as it is made: an inline shell's
    /// [`feed_source`](SpectrumAnalysis::feed_source) delivers here, where
    /// no Clear can come between the analysis and the store.
    pub fn apply(&mut self, delivery: Delivery, config: &SpectrumConfig) {
        self.store.apply(delivery, config, true);
    }

    /// The curve to draw, or None while no audio is flowing. The levels are
    /// maintained per column as each is delivered; this
    /// only decides whether they are still live.
    pub fn display(&self, now: f64) -> Option<&SpectrumBuckets> {
        let store = &self.store;
        store.last_samples.is_some_and(|t| now - t <= Self::HOLD_SECONDS).then_some(&store.display)
    }

    /// The most history ever kept, span-independent: the longest span the roll
    /// offers (`roll_seconds` max, 600 s) plus 10 s of headroom so a column is
    /// ready the instant the window reaches back to it. Each rung of the slab
    /// pyramid keeps this much (or its widest ring, whichever is less), and
    /// nothing older; see [`SlabPyramid`](crate::spectrogram::SlabPyramid).
    pub(crate) const HISTORY_MAX_SECONDS: f64 = 610.0;

    /// A column straight into the store, for a test that needs history
    /// without analyzing audio. See [`SpectrumStore::keep`].
    #[cfg(test)]
    pub(crate) fn push_history(&mut self, now: f64, power: &SpectrumBuckets) {
        let heard = power.iter().any(|p| *p > harmonigraph_core::spectrogram::POWER_FLOOR);
        self.store.keep(now, power, heard);
    }

    /// The slab pyramid the heatmap reads, every level folded up to the
    /// newest column.
    pub(crate) fn pyramid(&mut self) -> &mut crate::spectrogram::SlabPyramid {
        &mut self.store.pyramid
    }

    /// Forget the spectrogram history (paired with clearing the roll).
    pub fn clear_history(&mut self) {
        self.store.clears = self.store.clears.wrapping_add(1);
        self.store.reset();
    }

    /// How many times the history has been cleared, which a hand-over
    /// captures when it takes its audio: see [`give`](Self::give).
    pub fn clears(&self) -> u32 {
        self.store.clears
    }

    /// Which emptying of the history this is — a Clear, or a reset by the
    /// guard or by a crash in the fold — so the heatmap's colour memory can
    /// tell an emptied picture from one that scrolled on.
    pub fn history_epoch(&self) -> u32 {
        self.store.pyramid.generation()
    }

    /// Columns the history's guard dropped, and the times the history reset,
    /// since the store was made. Both read zero while every feeder dates its
    /// columns forward, which every shipped feeder does: the overlay shows
    /// them as totals, and anything but zero is a bug in the dating.
    pub(crate) fn guard_counts(&self) -> (u32, u32) {
        (self.store.drops, self.store.resets)
    }

    /// How far behind `now` the newest column sits even when nothing is wrong:
    /// where the weight of the window it was read through is centred, because
    /// that is where a spectrum belongs on a time axis (see
    /// [`AnalyzerConfig::column_lag`](harmonigraph_analysis::AnalyzerConfig::column_lag)).
    ///
    /// The heatmap allows for this delay when deciding whether columns are
    /// fresh enough to extend their newest color to the now-line. This also
    /// reports the lag of the analysis that made the newest columns, delivered
    /// with their batch, rather than the requested config's
    /// ([`SpectrumConfig::column_lag`]): the plugin's frame reads it while
    /// its input worker analyzes, and after a crash there replaced the
    /// analysis, until the next hand-over.
    pub fn column_lag(&self) -> f64 {
        self.store.lag
    }

    /// How many columns have been offered to the history since the store was
    /// made, including any its guard dropped: a count that only grows, so a
    /// Clear does not hide what a step kept, and nothing has to read the
    /// pyramid to tell.
    pub fn columns_taken(&self) -> u64 {
        self.store.columns_taken
    }

    /// The date of the newest column the history holds, `None` while it is
    /// empty.
    pub fn newest_column(&self) -> Option<f64> {
        self.store.pyramid.newest()
    }

    /// When the spectrum last held anything above silence: see [`Heard`].
    pub(crate) fn last_audible(&self) -> Heard {
        self.store.last_audible
    }

    /// When the smoothed display stops moving, which the curve, the Spiral and
    /// the lattice ring all draw: a column moves it once it lands, a
    /// [`column_lag`](Self::column_lag) after its own time, and nothing draws
    /// it [`HOLD_SECONDS`](Self::HOLD_SECONDS) after the samples stop. The
    /// display's release is inside the stamp it starts from
    /// ([`Heard::curve`]), so a curve still above silence counts as heard.
    /// `None` before anything has been.
    ///
    /// A bound from the stamp each landing column writes, beside the constants
    /// it is built from, rather than a second copy of them in the frame loop.
    pub(crate) fn moving_until(&self) -> Option<f64> {
        let heard = self.last_audible().curve?;
        Some(heard + self.column_lag() + Self::HOLD_SECONDS)
    }

    /// Whether the smoothed display is still moving at `now`: see
    /// [`moving_until`](Self::moving_until).
    pub(crate) fn moving(&self, now: f64) -> bool {
        self.moving_until().is_some_and(|until| until >= now)
    }
}

impl SpectrogramSurfaces {
    /// Full uploads of the slab grid across every surface since the plugin
    /// was opened.
    ///
    /// CORRECT and expensive, which is the whole problem — it draws the right
    /// picture at many times the cost, so nothing on screen distinguishes a
    /// working delta upload from one that has quietly stopped. The overlay
    /// turns it into a rate, where "climbing" is the entire diagnosis.
    pub(crate) fn full_uploads(&self) -> u32 {
        self.all().map(|s| s.folded.full_uploads()).sum()
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
        let store = &mut self.store;
        let key = (store.display_revision, width.to_bits());
        if store.frame_fold.as_ref().is_none_or(|(revision, bits, _)| (*revision, *bits) != key) {
            store.frame_fold = Some((
                key.0,
                key.1,
                crate::panes::spectral_fold::Fold::measure(&store.display, width),
            ));
            #[cfg(test)]
            {
                store.fold_measurements += 1;
            }
        }
        store.frame_fold.as_ref().map(|(_, _, fold)| fold.grid())
    }
}

/// Whether a test feeds the history backward dates on purpose, to reach the
/// guard: [`SpectrumStore::take`] asserts in debug builds that nothing else
/// does.
#[cfg(not(test))]
fn backward_dates_expected() -> bool {
    false
}

#[cfg(test)]
thread_local! {
    /// Set by [`expecting_backward_dates`].
    static BACKWARD_DATES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
fn backward_dates_expected() -> bool {
    BACKWARD_DATES.get()
}

/// Run `body` with [`SpectrumStore::take`]'s ordering assertion lifted, for a
/// test of the guard that stands in for it in release builds.
#[cfg(test)]
pub(crate) fn expecting_backward_dates<T>(body: impl FnOnce() -> T) -> T {
    BACKWARD_DATES.set(true);
    let out = body();
    BACKWARD_DATES.set(false);
    out
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
        assert_eq!(spectrum.store.fold_measurements, 1);

        // Ten 60 Hz frames, all inside HOLD_SECONDS so the curve is still
        // drawn, and none of them pushing audio. Under a key carrying `now`
        // each one measured the same numbers again.
        for frame in 1..=10 {
            let now = 1.0 + f64::from(frame) / 60.0;
            assert!(now - 1.0 < AudioSpectrum::HOLD_SECONDS, "the fixture must stay in hold");
            assert_eq!(spectrum.folded(now, 2.0).unwrap().as_slice(), first);
        }
        assert_eq!(spectrum.store.fold_measurements, 1, "a new frame is not a new measurement");

        // A new column at a new frame time still invalidates it: the next
        // batch of the run, which ends that much later.
        let later = 1.0 + tone(660.0).len() as f64 / 48_000.0;
        feed.feed(&mut spectrum, &tone(660.0), 1, 48_000.0, later, &cfg);
        let next = spectrum.folded(later, 2.0).unwrap().to_vec();
        assert_ne!(next, first, "new audio must be measured");
        assert_eq!(spectrum.store.fold_measurements, 2);
    }
}
