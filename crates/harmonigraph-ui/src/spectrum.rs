//! The audio-derived spectrum behind the Spectral pane: the analyzer that
//! turns incoming samples into columns, the store those columns age out of,
//! and the per-surface caches describing the uploaded heatmap texture.
//! Runtime-only — none of this is persisted.

use crate::SpectrumConfig;

/// One power value per pitch-spectrum bucket, the array the analyzer fills
/// and the pane draws. See [`harmonigraph_core::spectrum::SPECTRUM_BINS`].
pub(crate) type SpectrumBuckets = [f32; harmonigraph_core::spectrum::SPECTRUM_BINS];

/// Audio-derived pitch spectrum shown in the Spectral pane. The shell
/// feeds mono samples every frame from wherever its audio comes from
/// (plugin: input bus via a ring buffer; standalone: the mock synth); the
/// pane asks for a display refresh when it draws. Runtime-only.
pub struct AudioSpectrum {
    /// One analyzer per input channel, combined in the power domain — see
    /// [`ChannelBank`](harmonigraph_core::spectrum::ChannelBank).
    pub(crate) analyzer: harmonigraph_core::spectrum::ChannelBank,
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
    /// Smoothed rather than taken fresh, on exactly the reasoning behind the
    /// plugin's own `ClockMapper` (and with its constants). A shell drains its
    /// audio ring on frame boundaries but the ring fills in audio BLOCKS, so the
    /// number of samples a frame brings swings by a block either way while `now`
    /// advances by a frame — several ms of wobble in what any one batch implies
    /// about where sample 0 was. Stamping columns from a fresh estimate would
    /// pass that wobble straight into their spacing, which is what the sample
    /// grid exists to remove: at an 8 ms hop, +-5 ms of it is enough to leave a
    /// 12.8 ms slab empty. Smoothed, the grid is exactly even and still follows
    /// the shell clock.
    pub(crate) anchor: Option<f64>,
    /// When samples last arrived; the curve hides once the source stops
    /// (closed input bus, switched-off synth) rather than freezing.
    pub(crate) last_samples: Option<f64>,
    /// Timestamped raw spectra, one per FFT, for the spectrogram — oldest
    /// first. Raw (unsmoothed) so time isn't blurred across columns.
    /// Bounded by age and, by construction, by memory: see
    /// [`SpectrumHistory`] and
    /// [`AudioSpectrum::push_history`].
    pub(crate) history: SpectrumHistory,
    /// One per drawing surface — index 0 the docked Spectral pane (and the
    /// offline render), index 1 the Video pane's preview — so two live
    /// spectrograms in one frame don't overwrite each other's work.
    pub(crate) spectrogram: [SpectrogramSurface; 2],
}

/// One drawing surface's heatmap: the uploaded texture, and the three caches
/// that describe it.
///
/// They are held together because they describe ONE texture between them and
/// can only be trusted as a set — the cache validates the texture, the ring
/// records which slabs its columns hold, and either one kept without it is a
/// statement about pixels that no longer exist. As four parallel arrays they
/// were four chances to update three.
///
/// Runtime-only, never persisted, and each rebuilds itself from
/// [`AudioSpectrum::history`] when dropped, so `None` is always a safe state.
#[derive(Default)]
pub(crate) struct SpectrogramSurface {
    /// The heatmap's pixels, sampled with bilinear filtering so it reads as a
    /// smooth image rather than a mesh of interpolated triangles. Created
    /// lazily on first draw.
    pub(crate) tex: Option<egui::TextureHandle>,
    /// Validates [`Self::tex`]: while the key still matches, the whole build
    /// (aggregate -> colour -> upload) is skipped and only the scrolling quad
    /// is redrawn. See [`SpectrogramKey`].
    pub(crate) cache: Option<SpectrogramCache>,
    /// Live-only incremental aggregator: keeps the slab grid across frames so a
    /// rebuild folds only new columns instead of rescanning the whole window.
    /// See `spectrogram::SpectrogramAgg`.
    pub(crate) agg: Option<crate::spectrogram::SpectrogramAgg>,
    /// Which slabs [`Self::tex`]'s columns currently hold, so a new one can be
    /// written on its own instead of repainting the whole heatmap. `None`
    /// whenever the texture was built the full-width way, which now means only
    /// the offline whole-song render.
    pub(crate) ring: Option<crate::spectrogram::SpectrogramRing>,
    /// Times the ring has been restarted — re-blanked and every column
    /// repainted — by [`Restart`](crate::spectrogram::Restart) reason.
    /// Kept HERE rather than on the ring, which does not survive its own
    /// restart. Read out by the performance overlay beside the aggregator's
    /// rebuild count; see
    /// [`SpectrogramAgg::rebuilds`](crate::spectrogram::SpectrogramAgg::rebuilds).
    pub(crate) restarts: [u32; crate::spectrogram::Restart::COUNT],
    /// Whether the style is mid-gesture, which is when the image is built
    /// short and stretched — see
    /// [`StyleMotion`](crate::spectrogram::StyleMotion). Like the aggregator,
    /// it describes the style stream rather than the texture, so it survives
    /// [`release_texture`](Self::release_texture).
    pub(crate) motion: Option<crate::spectrogram::StyleMotion>,
}

impl SpectrogramSurface {
    /// Drop everything that describes the CURRENT context's texture, so the
    /// next draw uploads a fresh one.
    ///
    /// Three of the four go, and which three is the whole point of naming this:
    /// the cache validates the released texture, and the ring records which
    /// slabs its columns held — kept across a context change, the ring would
    /// write one fresh column into a brand new texture and call the other
    /// thousands valid, a heatmap of uninitialized memory. The aggregator is
    /// derived from the STORE rather than from the texture, so it survives; it
    /// is the one piece a new context does not invalidate.
    pub(crate) fn release_texture(&mut self) {
        self.tex = None;
        self.cache = None;
        self.ring = None;
    }
}

/// One column of the spectrogram, and the age-tiered store they live in — both
/// pure data, so they live in the core crate. See
/// [`harmonigraph_core::spectrogram`] for why a column is bytes of dB rather than
/// floats of power, and why old ones are merged.
pub use harmonigraph_core::spectrogram::{SpectrogramColumn, SpectrumHistory};

/// The inputs the built spectrogram image depends on. Equal keys mean the
/// uploaded texture is still valid, so `draw_spectrogram` skips the rebuild
/// (aggregate -> smooth -> color -> texture upload) and only redraws the quad,
/// which scrolls with `now` every frame regardless. The FFT refreshes at
/// ~20 Hz while the pane redraws at frame rate, so most frames hit.
///
/// Staleness-safe by construction — every way the image can change moves a
/// field: a fresh column moves `newest_bits` (even in a saturated ring, where
/// the count holds), the oldest column scrolling out of the window moves
/// `first`, a resize/zoom moves the layout fields, and a palette or Level
/// window change moves the colour fields. Floats compare by bit pattern so
/// equality is exact and free of NaN quirks.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpectrogramKey {
    /// Everything that decides a column's PIXELS, shared with the ring — which
    /// asks the same question about the same columns, one scroll at a time. See
    /// [`ColumnStyle`](crate::spectrogram::ColumnStyle).
    pub(crate) style: crate::spectrogram::ColumnStyle,
    /// ...and which columns are in the WINDOW, which is what a scroll changes
    /// and the ring survives.
    pub(crate) first: usize,
    pub(crate) cols_len: usize,
    pub(crate) newest_bits: u64,
    pub(crate) whole: bool,
}

/// A validated spectrogram build: its [`SpectrogramKey`] plus the scalars the
/// quad needs, so a cache hit can place the already-uploaded texture without
/// re-running the pixel pipeline. The texture itself stays in
/// [`SpectrogramSurface::tex`].
pub(crate) struct SpectrogramCache {
    pub(crate) key: SpectrogramKey,
    /// Where the build put its slabs in the texture, kept exactly as it
    /// computed them — a hit hands this straight back to the quad.
    pub(crate) layout: crate::spectrogram::TexLayout,
}

impl SpectrogramKey {
    /// Pack the image's inputs into a key: the style every column shares, plus
    /// which columns this build drew. `newest` is stored as a bit pattern so
    /// equality is exact.
    pub(crate) fn new(
        style: crate::spectrogram::ColumnStyle,
        first: usize,
        cols_len: usize,
        newest: f64,
        whole: bool,
    ) -> Self {
        SpectrogramKey { style, first, cols_len, newest_bits: newest.to_bits(), whole }
    }

    /// What the ring compares — the part of this key a scroll leaves alone.
    pub(crate) fn style(&self) -> &crate::spectrogram::ColumnStyle {
        &self.style
    }
}

impl SpectrogramCache {
    pub(crate) fn new(
        key: SpectrogramKey,
        layout: crate::spectrogram::TexLayout,
    ) -> Self {
        SpectrogramCache { key, layout }
    }

    /// Whether a freshly computed key matches this cached build's — i.e. the
    /// uploaded texture is still the right one to draw.
    pub(crate) fn matches(&self, key: &SpectrogramKey) -> bool {
        &self.key == key
    }

    /// The scalars the scrolling quad needs.
    pub(crate) fn geometry(&self) -> crate::spectrogram::TexLayout {
        self.layout
    }
}

/// The whole take's spectrogram, precomputed for the offline renderer's
/// playhead mode: every column of the full audio, laid out statically along
/// the time axis with a playhead at `now`, instead of the live scrolling
/// window. `Some` only in the offline renderer — the live ring
/// ([`AudioSpectrum::history`]) is bounded and cannot hold a whole song.
/// Runtime-only, never persisted (like
/// [`SharedState::learn_active`](crate::SharedState::learn_active)).
pub struct WholeSong {
    /// Take time at the near edge: the playhead sits here at the render's
    /// start.
    pub start: f64,
    /// Seconds spanned across the depth axis — the render's duration.
    pub span: f64,
    /// Every spectrogram column, oldest first.
    pub columns: Vec<SpectrogramColumn>,
    /// The whole take's notes, laid out from the start. The live tracker only
    /// holds notes replayed up to `now`, so the roll would otherwise fill in as
    /// the playhead reached them; the render wants the whole piece at once. Set
    /// by the offline renderer; empty in the spectrogram-only bounce preview.
    pub roll: harmonigraph_core::NoteRoll,
}

impl WholeSong {
    /// The shortest window the depth axis will map time across. A render can
    /// ask for less — one frame at 30 fps is 33 ms — and the axis draws this
    /// much regardless, so the picture reaches past what was asked for.
    pub const MIN_WINDOW: f64 = 0.05;

    /// Analyze the entire `samples` buffer, one raw column per hop,
    /// `time`-stamped in take time (`time_origin` is the take time of sample 0).
    /// Raw, exactly like the live store: the heatmap reads what was measured,
    /// and blurring one column into the next is not something it has done since
    /// temporal smoothing was dropped.
    ///
    /// The hop is the live one, EXCEPT that a long take stretches it: this build
    /// spans the whole song rather than a scrolling window, so its time axis is
    /// cut into `span / WHOLE_SONG_SLAB_CAP` slabs at best, and columns finer
    /// than that are aggregated away by the MAX the moment they are drawn. A
    /// three-minute take at the live rate would hold 22 500 columns (86 MB) to
    /// display 4096 of them. Scaling the hop to the slab keeps the same
    /// [`COLUMNS_PER_SLAB`](crate::spectrogram::COLUMNS_PER_SLAB) margin
    /// the live path has — every slab still gets a column, none goes empty — for
    /// a quarter of the memory.
    ///
    /// `samples` is INTERLEAVED, `channels` per frame, and the channels are
    /// combined exactly as the live path combines them — same
    /// [`ChannelBank`](harmonigraph_core::spectrum::ChannelBank), same power sum. That
    /// is the point of sharing the type rather than repeating the arithmetic: a
    /// render that summed its channels differently from the pane would differ
    /// from the look that was dialed in, and only for stereo-wide material, which
    /// is the hardest kind of difference to attribute.
    ///
    /// Pure: `(samples, channels, rate, config)` in, columns out, no clock or
    /// RNG, so a render built on it stays byte-identical between runs.
    pub fn precompute(
        samples: &[f32],
        channels: usize,
        sample_rate: f32,
        time_origin: f64,
        start: f64,
        span: f64,
        config: &SpectrumConfig,
    ) -> WholeSong {
        let mut analyzer = harmonigraph_core::spectrum::ChannelBank::new(sample_rate, channels);
        analyzer.set_fft_size(config.window.samples());
        analyzer.set_tapers(config.tapers.count());
        let channels = analyzer.channels();
        let sr = (sample_rate as f64).max(1.0);
        let hop = (span / crate::spectrogram::WHOLE_SONG_SLAB_CAP as f64
            / crate::spectrogram::COLUMNS_PER_SLAB)
            .max(AudioSpectrum::FFT_INTERVAL);
        let total = samples.len() / channels; // frames
        let mut columns = Vec::new();
        // Feed the buffer in one-hop chunks; once the window has filled every
        // hop yields a column, exactly as the live `push_samples` loop does.
        let (mut fed, mut k) = (0usize, 1usize);
        loop {
            let end = ((k as f64 * hop * sr).round() as usize).min(total);
            if end > fed {
                analyzer.push_frames(&samples[fed * channels..end * channels]);
                fed = end;
            }
            if let Some(power) = analyzer.power_sum() {
                // The middle of the window this spectrum measured, exactly as
                // the live path stamps it — `end` is where that window ENDS.
                // The take's notes are laid out from their own timestamps, so a
                // render is where a half-window offset would show up most: the
                // ribbons are placed perfectly and the heatmap would not be.
                let center = time_origin + end as f64 / sr - analyzer.window_center_offset();
                columns.push(SpectrogramColumn::from_power(center, &power));
            }
            if end >= total {
                break;
            }
            k += 1;
        }
        // The roll is filled in separately by the renderer (it needs the notes,
        // not the audio); the bounce preview leaves it empty.
        WholeSong { start, span, columns, roll: harmonigraph_core::NoteRoll::default() }
    }

    /// The columns the depth axis can actually draw: those stamped inside
    /// `[start, start + span]`.
    ///
    /// The heatmap's WIDTH comes from the columns the fold is handed, not from
    /// the span the plan sized its slab against:
    /// [`Plan::new`](crate::spectrogram::Plan::new) picks `bucket` from the
    /// window, while the module's `aggregate_slabs` gives every elapsed slab a
    /// texel between the first column and the last (both private to it, so
    /// named here rather than linked).
    /// Those agree only while the columns lie inside the window, and
    /// [`precompute`](Self::precompute) leaves them free to reach past it: it
    /// analyses the whole `samples` buffer
    /// whatever the render's `--start`/`--end`, so a ten-second window on a
    /// three-minute bounce arrives here with columns spanning the file and a
    /// bucket cut for ten seconds. Folding those built a texture around 14 000
    /// texels wide against a limit of 2048 — issue #367, the same
    /// `load_texture` assert as #333/#335, reached from the other axis.
    ///
    /// The window the depth axis actually maps time across:
    /// [`span`](Self::span) under [`MIN_WINDOW`](Self::MIN_WINDOW).
    ///
    /// A window of nothing maps every take time to one depth, so the axis puts
    /// a floor under it — and a render shorter than that floor therefore draws
    /// a region reaching past `start + span`. `TimeAxis::new` reads its
    /// whole-song window from here rather than restating the floor, because the
    /// two restatements drifting is exactly what left a 20 ms render's heatmap
    /// covering 64% of its region.
    pub fn window(&self) -> f64 {
        self.span.max(Self::MIN_WINDOW)
    }

    /// `window` is the axis' own, taken from the caller rather than from
    /// [`span`](Self::span), and they are NOT the same number: `TimeAxis::new`
    /// floors the window it maps time across, so a render shorter than that
    /// floor draws a depth region reaching past `start + span`. Trimming to
    /// `span` there drops columns that have a depth on screen and stops the
    /// heatmap part way down a region the rest of the pane keeps drawing — a
    /// 20 ms render leaves 36% of it bare. `build` hands over the very `f64`
    /// [`Plan::new`](crate::spectrogram::Plan::new) cut `bucket` from, so the
    /// trim and the slab cannot drift; two expressions of one window is what
    /// this takes an argument to avoid.
    ///
    /// Trimmed to that window EXACTLY, with no margin either side, and the
    /// tightness is what `spectrogram::slab_ceiling`'s slabs in hand are
    /// counted against. Nothing drawn is lost by it: the pane maps take time to
    /// depth through `TimeAxis::frac`, so a column outside
    /// `[start, start + window]` has no depth on screen, and the quad samples
    /// the texture by ABSOLUTE time (`u_drawn` over `t_origin`/`tex_span`), so
    /// dropping texels off the ends moves none of the ones that remain.
    pub fn drawn_columns(&self, window: f64) -> impl Iterator<Item = &SpectrogramColumn> {
        let (from, to) = (self.start, self.start + window);
        self.columns.iter().filter(move |c| c.time >= from && c.time <= to)
    }
}

impl Default for AudioSpectrum {
    fn default() -> Self {
        AudioSpectrum {
            analyzer: harmonigraph_core::spectrum::ChannelBank::new(48_000.0, 1),
            display: [0.0; harmonigraph_core::spectrum::SPECTRUM_BINS],
            frames_seen: 0,
            next_hop: 0,
            anchor: None,
            last_samples: None,
            history: SpectrumHistory::default(),
            spectrogram: [SpectrogramSurface::default(), SpectrogramSurface::default()],
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
    /// Forget the spectrogram textures, so the next draw uploads fresh ones
    /// into whatever context is current. See
    /// [`SharedState::release_context_resources`](crate::SharedState::release_context_resources).
    pub(crate) fn release_textures(&mut self) {
        for surface in &mut self.spectrogram {
            surface.release_texture();
        }
    }

    /// Seconds of AUDIO between FFTs (125 columns a second), measured in
    /// samples rather than on the shell clock — see
    /// [`push_samples`](Self::push_samples).
    ///
    /// The heatmap does not repaint itself for every column (see
    /// `write_ring`), so the cost of a column is O(pitch pixels) rather than
    /// O(pitch pixels x slabs) and the rate buys smoothness at the newest
    /// edge almost for free. It costs no REACH either: the store coarsens
    /// with age (see [`SpectrumHistory`]), so the rate sets the resolution of
    /// the recent stretch and barely touches how far back the heatmap goes.
    ///
    /// At 8 ms this is a picture setting and not an analysis one: the window
    /// is untouched, so what a column RESOLVES is unchanged, and overlapping
    /// that same window more finely just draws the time axis at 2.5x the
    /// resolution 20 ms reaches (via
    /// [`live_slab`](crate::spectrogram::live_slab), whose ladder is
    /// rungs of THIS interval, so the picture's grid tracks it).
    /// It costs 0.37 ms of FFT per column — 4.7% of a core, against 1.9% at
    /// 20 ms — and one more [`SpectrumHistory`] tier to hold the same reach.
    pub(crate) const FFT_INTERVAL: f64 = 0.008;
    /// How long after the last samples the curve keeps drawing.
    pub(crate) const HOLD_SECONDS: f64 = 0.5;
    /// Per-batch gain and restart threshold for the sample-count anchor (see
    /// the field). The plugin's `ClockMapper` solves the same problem for MIDI
    /// event times with the same two numbers.
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
        let batch = samples.len() / channels;
        if batch == 0 {
            return;
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
        let anchor = match self.anchor {
            Some(prev) if (candidate - prev).abs() <= Self::ANCHOR_SNAP => {
                prev + (candidate - prev) * Self::ANCHOR_SMOOTHING
            }
            _ => candidate,
        };
        self.anchor = Some(anchor);

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
            self.push_history(boundary - self.analyzer.window_center_offset(), &fresh);
        }
    }

    /// The curve to draw, or None while no audio is flowing. The levels are
    /// maintained per column by [`push_samples`](Self::push_samples); this
    /// only decides whether they are still live.
    pub fn display(&self, now: f64) -> Option<&SpectrumBuckets> {
        self.last_samples
            .is_some_and(|t| now - t <= Self::HOLD_SECONDS)
            .then_some(&self.display)
    }

    /// The most history ever kept, span-independent: the longest span the roll
    /// offers (`roll_seconds` max, 600 s) plus 10 s of headroom so a column is
    /// ready the instant the window reaches back to it. Nothing older is
    /// retained even at the maximum span.
    ///
    /// This is the ONLY thing that decides reach — no memory backstop binds
    /// first. Storing a bucket as a byte of dB and coarsening old columns
    /// (see [`SpectrumHistory`]) puts the full span at about 30 MB, so the
    /// cap can simply be the span. Keeping every column at full rate forever
    /// would instead cost 160 MB to reach only ~3.5 minutes at 50 Hz, drawing
    /// a heatmap over the recent stretch and bare roll beyond it.
    ///
    /// Raising it is cheap and sub-linear: another
    /// [`SpectrumHistory::COARSE_COLUMNS`] (~4 MB) doubles the reach. The unit
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

    /// Fallbacks taken across both surfaces since the plugin was opened: full
    /// re-aggregations of the window, and ring restarts.
    ///
    /// Both are CORRECT and both are expensive, which is the whole problem —
    /// they draw the right picture at many times the cost, so nothing on screen
    /// distinguishes a working cache from one that has quietly stopped. The
    /// overlay turns them into a rate, where "climbing" is the entire diagnosis.
    pub(crate) fn spectrogram_fallbacks(
        &self,
    ) -> (u32, [u32; crate::spectrogram::Restart::COUNT]) {
        self.spectrogram.iter().fold(
            (0, [0; crate::spectrogram::Restart::COUNT]),
            |(rebuilds, mut restarts), s| {
                for (total, surface) in restarts.iter_mut().zip(s.restarts) {
                    *total += surface;
                }
                (rebuilds + s.agg.as_ref().map_or(0, |a| a.rebuilds()), restarts)
            },
        )
    }

    /// Forget the spectrogram history (paired with clearing the roll).
    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// How far behind `now` the newest column sits even when nothing is wrong:
    /// half the analysis window, because that is where a spectrum belongs on a
    /// time axis (see
    /// [`window_center_offset`](harmonigraph_core::spectrum::SpectrumAnalyzer::window_center_offset)).
    ///
    /// The heatmap's near edge has to allow for this or it reads a perfectly
    /// healthy stream as stale and stops the strip short of the now-line — by
    /// 171 ms on the Precise window, which is a visible gap that widens and
    /// narrows as the window is changed.
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
