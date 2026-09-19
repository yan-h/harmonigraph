//! The audio-derived spectrum behind the Spectral pane: the analyzer that
//! turns incoming samples into columns, the store those columns age out of,
//! Separate surface storage describes the uploaded heatmap textures.
//! Runtime-only — none of this is persisted.

use crate::SpectrumConfig;

/// One power value per pitch-spectrum bucket, the array the analyzer fills
/// and the pane draws. See [`harmonigraph_core::spectrum::SPECTRUM_BINS`].
pub(crate) type SpectrumBuckets = [f32; harmonigraph_core::spectrum::SPECTRUM_BINS];

/// Audio-derived pitch spectrum shown in the Spectral pane. The shell
/// feeds samples every tick from wherever its audio comes from
/// (plugin: selected main/sidechain input via a ring buffer; standalone: the
/// mock synth). Analysis advances on the sample clock without drawing. Runtime-only.
pub struct AudioSpectrum {
    /// Changed only when the smoothed grid changes. This identity lives beside
    /// the measurement keyed on it, so replacing or resetting the analyzer
    /// drops both together and no fold outlives the display it was measured
    /// from.
    display_revision: u64,
    /// One lazy Fold, keyed on everything its value is measured from and
    /// nothing else. `Fold::measure` is a pure function of `display` and the
    /// clamped width: `display_revision` stands for the first — the only two
    /// writes to `display` bump it on the adjacent line — and the width is
    /// carried by bits. Geometry, palettes and TIME are not inputs; a frame
    /// with no new column measures the same numbers the last one did, so `now`
    /// only decided how often the memo was thrown away.
    frame_fold: Option<(u64, u32, crate::panes::spectral_fold::Fold)>,
    #[cfg(test)]
    pub(crate) fold_measurements: usize,
    /// One analyzer per input channel, combined in the power domain — see
    /// [`ChannelBank`](harmonigraph_analysis::ChannelBank).
    pub(crate) analyzer: harmonigraph_analysis::ChannelBank,
    /// Smoothed display buckets (power; the pane maps to height).
    pub(crate) display: SpectrumBuckets,
    /// FRAMES pushed since this analyzer was made, and the count at which the
    /// next FFT falls due. The column grid is a function of these two and
    /// nothing else — see [`push_samples`](AudioSpectrum::push_samples).
    ///
    /// Frames, not samples: a stereo stream carries two samples per instant, and
    /// a hop is an amount of TIME. Counting samples would halve the hop the
    /// moment the input went stereo.
    pub(crate) frames_seen: u64,
    pub(crate) next_hop: u64,
    /// Shell time of sample 0: what turns a sample count into a timestamp.
    ///
    /// Arrival-dated input from [`push_samples`](Self::push_samples) smooths
    /// this estimate rather than taking it fresh. A shell drains its
    /// audio ring on frame boundaries but the ring fills in audio BLOCKS, so the
    /// number of samples a frame brings swings by a block either way while `now`
    /// advances by a frame — several ms of wobble in what any one batch implies
    /// about where sample 0 was. Stamping columns from a fresh estimate would
    /// pass that wobble straight into their spacing, which is what the sample
    /// grid exists to remove: at an 8 ms hop, +-5 ms of it is enough to leave a
    /// 12.8 ms slab empty. Smoothed, the grid is exactly even and still follows
    /// the shell clock. The plugin instead supplies exact source timestamps
    /// through [`push_source_samples`](Self::push_source_samples), which uses
    /// its shared presentation-clock mapping without this smoothing.
    pub(crate) anchor: Option<f64>,
    /// When samples last arrived; the curve hides once the source stops
    /// (silent/unrouted input, switched-off synth) rather than freezing.
    pub(crate) last_samples: Option<f64>,
    /// Timestamped raw spectra, one per FFT, for the spectrogram — oldest
    /// first. Raw (unsmoothed) so time isn't blurred across columns.
    /// Bounded by age and, by construction, by memory: see
    /// [`SpectrumHistory`] and
    /// [`AudioSpectrum::push_history`].
    pub(crate) history: SpectrumHistory,
}

/// Every drawing surface's heatmap state, indexed by the surface id its copy of
/// the pane was drawn with: 0 the docked Spectral pane, 1 the Video pane's
/// preview, and offline the placement's index in the layout (see
/// [`draw_pane`](crate::draw_pane)).
///
/// Grown on first sight of an id rather than fixed at the two the editor draws,
/// because a hand-written offline layout names as many Spectral placements as
/// it likes and each one folds a grid of its own: the slab width a fold settles
/// on comes off the pane's own depth in points, so two placements at different
/// sizes sharing one surface would each invalidate the other's grid every
/// frame.
///
/// What stays SHARED is the analyzer and the column ring it fills
/// ([`AudioSpectrum::history`]): there is one selected input and one stream of
/// columns, and a surface holds only what it folded out of them.
#[derive(Default)]
pub(crate) struct SpectrogramSurfaces(Vec<SpectrogramSurface>);

impl SpectrogramSurfaces {
    /// The surface's own state, empty on first sight of its id.
    ///
    /// A method on the field rather than on [`AudioSpectrum`], so a caller
    /// folding new columns can hold the shared `history` borrow while this one
    /// is out.
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

    pub(crate) fn all_mut(&mut self) -> impl Iterator<Item = &mut SpectrogramSurface> {
        self.0.iter_mut()
    }
}

/// One drawing surface's heatmap: the slab grid it folds, and the statement of
/// what the GPU holds of it.
///
/// Runtime-only, never persisted, and each half rebuilds itself from
/// [`AudioSpectrum::history`] when dropped, so a default is always a safe
/// state.
#[derive(Default)]
pub(crate) struct SpectrogramSurface {
    /// Incremental aggregator: keeps the slab grid across frames so a
    /// rebuild folds only new columns instead of rescanning the whole window.
    /// See `spectrogram::SpectrogramAgg`.
    pub(crate) agg: Option<crate::spectrogram::SpectrogramAgg>,
    /// What the GPU's copy of that grid holds, so a frame can send the slabs
    /// that moved instead of the run. See
    /// [`GpuGrid`](crate::spectrogram::GpuGrid).
    pub(crate) gpu: crate::spectrogram::GpuGrid,
    /// The slab width the previous frame drew at, which is what gives
    /// [`live_slab`](crate::spectrogram::live_slab)'s ladder its hysteresis —
    /// see [`Plan::new`](crate::spectrogram::Plan::new). `None` before the first frame.
    pub(crate) held_bucket: Option<f64>,
}

/// One column of the spectrogram, and the age-tiered store they live in — both
/// pure data, so they live in the core crate. See
/// [`harmonigraph_core::spectrogram`] for why a column is bytes of dB rather than
/// floats of power, and why old ones are merged.
pub use harmonigraph_core::spectrogram::{SpectrogramColumn, SpectrumHistory};

impl Default for AudioSpectrum {
    fn default() -> Self {
        AudioSpectrum {
            display_revision: 0,
            frame_fold: None,
            #[cfg(test)]
            fold_measurements: 0,
            analyzer: harmonigraph_analysis::ChannelBank::new(48_000.0, 1),
            display: [0.0; harmonigraph_core::spectrum::SPECTRUM_BINS],
            frames_seen: 0,
            next_hop: 0,
            anchor: None,
            last_samples: None,
            history: SpectrumHistory::default(),
        }
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
    /// [`push_samples`](Self::push_samples).
    ///
    /// A column costs the slab it lands in and nothing else (see
    /// `spectrogram::GpuGrid`), so the rate buys smoothness at the newest edge
    /// almost for free. It costs no REACH either: the store coarsens
    /// with age (see [`SpectrumHistory`]), so the rate sets the resolution of
    /// the recent stretch and barely touches how far back the heatmap goes.
    ///
    /// At 8 ms this is a picture setting and not an analysis one: the window
    /// is untouched, so what a column RESOLVES is unchanged, and overlapping
    /// that same window more finely just draws the time axis at 2.5x the
    /// resolution 20 ms reaches (via
    /// [`live_slab`](crate::spectrogram::live_slab), whose ladder is
    /// rungs of THIS interval, so the picture's grid tracks it).
    /// It costs 0.23 ms of FFT per column — 2.9% of a core, against 1.2% at
    /// 20 ms — and one more [`SpectrumHistory`] tier to hold the same reach.
    pub(crate) const FFT_INTERVAL: f64 = 0.008;
    /// How long after the last samples the curve keeps drawing.
    pub(crate) const HOLD_SECONDS: f64 = 0.5;
    /// Per-batch gain and restart threshold for the arrival-dated sample-count
    /// anchor (see the field). Source-dated plugin audio bypasses both.
    pub(crate) const ANCHOR_SMOOTHING: f64 = 0.05;
    pub(crate) const ANCHOR_SNAP: f64 = 1.0;

    /// Feed mono samples from the shell, analyzing one spectrum per
    /// [`FFT_INTERVAL`](Self::FFT_INTERVAL) of audio in them. `now` is the shell
    /// clock also passed to [`root_ui`](crate::root_ui), and dates the NEWEST sample of the
    /// batch — which is what a shell draining its audio ring at frame time
    /// means by it.
    ///
    /// The FFT runs here, on a grid of sample counts, rather than in
    /// [`display`](Self::display) on a grid of frames. That is the whole point:
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
    /// The smoothing and peak-hold decay of the CURVE moved here with it, for
    /// the same reason: both are per-column, so leaving them on the frame clock
    /// would have made their time constants frame-rate dependent.
    ///
    /// One call therefore costs as many FFTs as the audio it is handed contains
    /// hops, where the old one cost exactly one. Normally that is a frame's
    /// worth (two or three), and the worst case is a batch as large as the
    /// shell's audio ring — 1.37 s in the plugin, 170 columns, ~60 ms — reachable
    /// only by an editor that has been closed or stalled for that long, which
    /// then gets its heatmap back-filled with audio that really did happen.
    pub fn push_samples(
        &mut self,
        samples: &[f32],
        channels: usize,
        sample_rate: f32,
        now: f64,
        config: &SpectrumConfig,
    ) {
        if samples.is_empty() {
            return;
        }
        self.push_sample_chunks(
            samples.len() / channels.max(1),
            channels,
            sample_rate,
            now,
            config,
            |feed| {
                feed(samples);
                Ok::<_, std::convert::Infallible>(())
            },
        )
        .unwrap();
    }

    /// Start a new retained source run after loss, reset or a format change.
    /// Keep historical columns, but never combine samples across the boundary.
    pub fn restart_source(&mut self, channels: usize, sample_rate: f32) {
        self.analyzer = harmonigraph_analysis::ChannelBank::new(sample_rate, channels);
        self.frames_seen = 0;
        self.next_hop = 0;
        self.anchor = None;
        self.last_samples = None;
        self.display.fill(0.0);
        self.display_revision = self.display_revision.wrapping_add(1);
        self.frame_fold = None;
    }

    /// Feed complete source frames on an exact sample grid. `origin` is frame
    /// zero of this retained run converted to the shell clock by its owner.
    /// Unlike arrival-dated input, it needs no second clock estimate/smoothing.
    /// The shell may update its conversion once per drain, shared with notes;
    /// physical callback/chunk boundaries never influence that conversion.
    pub fn push_source_samples(
        &mut self,
        samples: &[f32],
        channels: usize,
        sample_rate: f32,
        origin: f64,
        config: &SpectrumConfig,
    ) {
        let batch = samples.len() / channels.max(1);
        if batch == 0 {
            return;
        }
        let newest = origin
            + (self.frames_seen + batch as u64).saturating_sub(1) as f64
                / f64::from(sample_rate.max(1.0));
        self.feed_sample_chunks(
            batch,
            channels,
            sample_rate,
            newest,
            config,
            Some(origin),
            |feed| {
                feed(samples);
                Ok::<_, std::convert::Infallible>(())
            },
        )
        .unwrap();
    }

    /// Feed one logical batch through bounded physical chunks. Update the clock
    /// anchor once from the complete batch's newest frame, exactly as
    /// `push_samples` does: I/O boundaries must not add smoothing steps or move
    /// column timestamps. The producer supplies exactly `batch` complete frames
    /// in order; an error aborts its caller rather than using a partial render.
    pub fn push_sample_chunks<E>(
        &mut self,
        batch: usize,
        channels: usize,
        sample_rate: f32,
        now: f64,
        config: &SpectrumConfig,
        consume: impl FnOnce(&mut dyn FnMut(&[f32])) -> Result<(), E>,
    ) -> Result<(), E> {
        self.feed_sample_chunks(batch, channels, sample_rate, now, config, None, consume)
    }

    #[allow(clippy::too_many_arguments)]
    fn feed_sample_chunks<E>(
        &mut self,
        batch: usize,
        channels: usize,
        sample_rate: f32,
        now: f64,
        config: &SpectrumConfig,
        source_origin: Option<f64>,
        consume: impl FnOnce(&mut dyn FnMut(&[f32])) -> Result<(), E>,
    ) -> Result<(), E> {
        // Any of the four empties the analyzers' rings, so nothing comes out
        // until they have refilled. The hop grid keeps its phase across that gap
        // rather than restarting on it.
        self.analyzer.set_channels(channels);
        self.analyzer.set_fft_size(config.window.samples());
        self.analyzer.set_tapers(config.tapers.count());
        self.analyzer.set_sample_rate(sample_rate);
        self.last_samples = Some(now);

        // FRAMES throughout: `samples` is interleaved, and a hop is an amount of
        // time. A partial frame at the end is left for the next batch, so the
        // de-interleaving in `push_frames` can never slip a channel.
        let channels = self.analyzer.channels();
        if batch == 0 {
            return Ok(());
        }
        let sr = f64::from(sample_rate.max(1.0));
        let hop = ((Self::FFT_INTERVAL * sr).round() as u64).max(1);
        // Columns fall on multiples of `hop` frames from the start of the
        // stream. Left at zero the first boundary would be frame 1 and every one
        // after it a frame early, which is harmless but makes the grid
        // impossible to state (or to test) in whole hops.
        if self.next_hop == 0 {
            self.next_hop = hop;
        }

        // Re-anchor the frame count on the shell clock: the last frame of this
        // batch is at `now`. Smoothed, so the columns below are evenly spaced;
        // snapped when the estimate moves further than any wobble could, which
        // is a stream that restarted — a transport jump, a sample-rate change
        // (the count is re-divided by a different rate, so the anchor moves by
        // minutes), or the first batch after the pane was switched on.
        let total = self.frames_seen + batch as u64;
        let candidate = now - total.saturating_sub(1) as f64 / sr;
        let anchor = source_origin.unwrap_or_else(|| match self.anchor {
            Some(prev) if (candidate - prev).abs() <= Self::ANCHOR_SNAP => {
                prev + (candidate - prev) * Self::ANCHOR_SMOOTHING
            }
            _ => candidate,
        });
        self.anchor = Some(anchor);

        consume(&mut |samples| {
            let batch = samples.len() / channels;
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

                // Two coefficients, chosen per bucket by which way it is moving.
                // Derived from the hop actually in use rather than set on the bar,
                // so the times mean seconds at any hop this loop runs at.
                self.display_revision = self.display_revision.wrapping_add(1);
                let step = hop as f64 / sr;
                let attack = hop_alpha(config.attack, step);
                let release = hop_alpha(config.release, step);
                for (shown, new) in self.display.iter_mut().zip(&fresh) {
                    // POWER, so "louder" is the same comparison in dB — the levels
                    // are mapped through `loudness` well downstream of here.
                    let alpha = if *new > *shown { attack } else { release };
                    *shown += (new - *shown) * alpha;
                }
                // Keep the RAW spectrum for the spectrogram (the smoothed
                // `display` would smear one column into the next). Retention is
                // span-INDEPENDENT (see `push_history`): shrinking the span and
                // widening it again must not lose the history in between.
                //
                // Stamped at the middle of the window it measured, not at the
                // boundary itself — see `window_center_offset`. This is what lets a
                // ridge sit under the note ribbon that made it, which is the entire
                // point of drawing the two on one time axis. The boundary is where
                // the newest frame fed so far sits on the anchored grid, so
                // consecutive columns are exactly `hop` frames apart.
                let boundary = anchor + self.frames_seen.saturating_sub(1) as f64 / sr;
                let center = boundary - self.analyzer.window_center_offset();
                // A source clock conversion can correct backwards while old
                // history remains on its previous mapping. The incremental
                // heatmap requires strictly increasing dates. Omit overlapping
                // history until mapped time passes its tail, without re-dating
                // samples or skipping any FFT/curve progression above. Offline
                // arrival-dated feeding retains its existing append behavior.
                if source_origin.is_none() || self.history.back().is_none_or(|c| center > c.time) {
                    self.push_history(center, &fresh);
                }
            }
        })
    }

    /// The curve to draw, or None while no audio is flowing. The levels are
    /// maintained per column by [`push_samples`](Self::push_samples); this
    /// only decides whether they are still live.
    pub fn display(&self, now: f64) -> Option<&SpectrumBuckets> {
        self.last_samples.is_some_and(|t| now - t <= Self::HOLD_SECONDS).then_some(&self.display)
    }

    /// The most history ever kept, span-independent: the longest span the roll
    /// offers (`roll_seconds` max, 600 s) plus 10 s of headroom so a column is
    /// ready the instant the window reaches back to it. Nothing older is
    /// retained even at the maximum span.
    ///
    /// This is the ONLY thing that decides reach — no memory backstop binds
    /// first. Storing linear-power sums and coarsening old columns
    /// (see [`SpectrumHistory`]) puts the full span at about 120 MiB, so the
    /// cap can simply be the span. Keeping every column at full rate forever
    /// would instead cost 160 MB to reach only ~3.5 minutes at 50 Hz, drawing
    /// a heatmap over the recent stretch and bare roll beyond it.
    ///
    /// Raising it is cheap and sub-linear: another
    /// [`SpectrumHistory::COARSE_COLUMNS`] (~15 MiB) doubles the reach. The unit
    /// test `spectrum_history_reaches_the_retention_cap` is what keeps the
    /// structure sized for whatever this says.
    pub(crate) const HISTORY_MAX_SECONDS: f64 = 610.0;

    /// Append one raw spectrum to the store, trimming anything past
    /// `HISTORY_MAX_SECONDS` of age. The store bounds its own memory (older
    /// columns merge, and its last tier's overflow is dropped), so there is no
    /// separate column-count backstop to keep in step with the FFT rate.
    ///
    /// Retention is deliberately NOT keyed to the current span: trimming to the
    /// live `roll_seconds` meant shrinking the span popped columns off the
    /// front, and widening it again could never bring them back — the span
    /// control silently erased spectrogram history. The heatmap simply reads
    /// back as far as the span asks; anything it isn't showing yet stays in the
    /// store until it ages past the cap.
    pub(crate) fn push_history(&mut self, now: f64, power: &SpectrumBuckets) {
        self.history.push(SpectrogramColumn::from_power(now, power));
        self.history.trim_older_than(now - Self::HISTORY_MAX_SECONDS);
    }

    /// The spectrogram columns, oldest first. Empty until audio has flowed.
    pub fn history(&self) -> &SpectrumHistory {
        &self.history
    }

    /// Forget the spectrogram history (paired with clearing the roll).
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// How far behind `now` the newest column sits even when nothing is wrong:
    /// half the analysis window, because that is where a spectrum belongs on a
    /// time axis (see
    /// [`window_center_offset`](harmonigraph_analysis::SpectrumAnalyzer::window_center_offset)).
    ///
    /// The heatmap deliberately does NOT allow for this, and stopping the strip
    /// short of the now-line by exactly this much is the picture it wants — 171
    /// ms of bed on the Precise window, narrowing with the window and with the
    /// Span. Everything nearer than the newest column is unmeasured, and
    /// covering it meant holding that column flat across the gap: the
    /// analyzer's current spectrum drawn as though it were history, which is
    /// what it looked like (#914). The argument lives on `strip_depths`, in the
    /// Spectral pane's spectrogram — named rather than linked, because this is
    /// a public item and that one is the pane's own.
    ///
    /// What still reads this is the analysis window ITSELF, observed from
    /// outside: nothing else about the running FFT is visible on this type, so
    /// half of it is the one readout that says which window the analyzer is
    /// actually on rather than which one the config asked for. The
    /// background-analysis tests (#324) assert a restored project's Window
    /// reached the thread through it, and the offline renderer's column-grid
    /// test pins its stamps against it.
    pub fn column_lag(&self) -> f64 {
        self.analyzer.window_center_offset()
    }

    /// Whether audio has arrived within the hold window — i.e. the spectrum is
    /// still live. Drives continuous repaint so the curve and spectrogram stay
    /// smooth even when no MIDI is animating the frame. Reads true only while
    /// samples are actually arriving (the shell pushes them when the spectrum is
    /// shown), so it idles cleanly once audio stops.
    pub fn is_flowing(&self, now: f64) -> bool {
        self.last_samples.is_some_and(|t| now - t <= Self::HOLD_SECONDS)
    }
}

impl SpectrogramSurfaces {
    /// Forget what the GPU holds of the spectrogram grids, so the next draw
    /// uploads them whole into whatever context is current. See
    /// [`PictureState::release_context_resources`](crate::PictureState::release_context_resources).
    ///
    /// The aggregators survive: they are derived from the STORE rather than
    /// from anything the GPU allocated, and are the one piece a new context does
    /// not invalidate.
    pub(crate) fn release_gpu_grids(&mut self) {
        for surface in self.all_mut() {
            surface.gpu.release();
        }
    }
    /// Fallbacks taken across every surface since the plugin was opened: full
    /// re-aggregations of the window, and full uploads of the grid.
    ///
    /// Both are CORRECT and both are expensive, which is the whole problem —
    /// they draw the right picture at many times the cost, so nothing on screen
    /// distinguishes a working cache from one that has quietly stopped. The
    /// overlay turns them into a rate, where "climbing" is the entire diagnosis.
    pub(crate) fn spectrogram_fallbacks(&self) -> (u32, u32) {
        self.all().fold((0, 0), |(rebuilds, uploads), s| {
            (rebuilds + s.agg.as_ref().map_or(0, |a| a.rebuilds()), uploads + s.gpu.full_uploads())
        })
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
        let width = crate::panes::spectral_fold::Fold::clamped_width(width);
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

#[cfg(test)]
mod tests {
    use super::AudioSpectrum;
    use crate::SpectrumConfig;

    /// The behavior `now` in the key made unreachable: a repaint that brought
    /// no new column reuses the measurement instead of taking it again.
    ///
    /// Beside its siblings in `tests::spectrum`, which cover the key's other
    /// two inputs (a width edit and same-frame audio) and its hold-window
    /// early return.
    #[test]
    fn a_fold_outlives_a_frame_that_brought_no_new_column() {
        let cfg = SpectrumConfig::default();
        let mut spectrum = AudioSpectrum::default();
        let tone = |frequency: f32| {
            (0..cfg.window.samples() * 2)
                .map(|i| (std::f32::consts::TAU * frequency * i as f32 / 48_000.0).sin() * 0.5)
                .collect::<Vec<_>>()
        };
        spectrum.push_samples(&tone(440.0), 1, 48_000.0, 1.0, &cfg);
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

        // A new column at a new frame time still invalidates it.
        spectrum.push_samples(&tone(660.0), 1, 48_000.0, 1.2, &cfg);
        let next = spectrum.folded(1.25, 2.0).unwrap().to_vec();
        assert_ne!(next, first, "new audio must be measured");
        assert_eq!(spectrum.fold_measurements, 2);
    }
}
