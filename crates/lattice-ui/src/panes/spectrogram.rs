//! The Spectral pane's spectrogram: a frequency-vs-time heatmap of the
//! analyzed audio, drawn in the roll's depth region on the roll's own time
//! axis. A column of spectral energy therefore lines up with the note
//! ribbons that made it — the same pitch axis across, the same `now`-anchored
//! time along, so what you hear and what you played read against each other.
//!
//! It's a layer under the roll, not a pane. The heatmap is built into a small
//! image — one pixel per (time slab, pitch pixel) — uploaded as a texture and
//! sampled with bilinear filtering, so it reads as one smooth, filled image
//! rather than a mesh of flat cells (which looked blocky) or interpolated
//! triangles (which floated and creased). Geometry still comes from
//! [`Axes`](super::spectral::Axes), so it turns and flips with the pane, and
//! its dB intensity scale is shared with the spectrum curve via
//! [`loudness`](super::spectral::loudness) so "loud" means the same in both.

use egui::Color32;
use lattice_core::spectrum::{BINS_PER_SEMITONE, SPECTRUM_BINS, SPECTRUM_MIN_MIDI};
use lattice_scene::{channel_color, FrameParams};

use super::spectral::{loudness, Axes, PitchScale, TimeAxis};
use super::PITCH_RAMP_CHANNEL;
use crate::{SharedState, SpectrogramColor, SpectrumConfig};

/// Bin power at or below this is treated as flat silence — skips the `log10`
/// in the intensity map for the many empty buckets, without changing the look.
const NEAR_ZERO: f32 = 1e-9;

/// A run of empty slabs this short is sampling jitter rather than a stall in
/// the analyzer, and holds the previous column instead of reading as silence.
///
/// The analyzer produces a column roughly every 50 ms and the narrowest slab
/// is 80 ms, so the two are within a factor of two of each other: one long
/// frame is all it takes to leave a slab with nothing in it. A real stall —
/// switching the FFT window empties the ring for a window's worth of samples,
/// 341 ms at 48 kHz on Precise — is many slabs wide and genuinely was silence
/// as far as the analyzer is concerned.
const JITTER_SLABS: i64 = 1;

/// Draw the spectrogram across the roll's depth region (`split..1`), sharing
/// the roll's `depth_of` time mapping so its columns register with the notes.
///
/// Builds the heatmap into a `[time slab x pitch bin]` image, (re)uploads it
/// to `spectrum.spectrogram_tex`, then stretches it over the region as a
/// single bilinear-filtered quad — smooth in both axes, and opaque (silence is
/// the ramp's dark end, not transparent) so the plane is a filled image rather
/// than bright patches floating on the background.
/// One row of the heatmap image: the source buckets it draws from (`idx`
/// covers `idx..end`), its center MIDI pitch, and that pitch's fraction `t` up
/// the pitch axis.
///
/// A row is a PIXEL of the pitch axis, not a bucket. Zoomed in, several rows
/// share one bucket — that is the resolution the analyzer has, and asking for
/// more of it is what the bucket count is for. Zoomed out, a row takes the max
/// over the buckets that fall in it: the axis holds thousands of buckets and
/// the pane a few hundred pixels, so one row per bucket would build an image
/// far taller than the screen (and, at 32 buckets per semitone, taller than
/// the GPU will allocate) only for the sampler to throw the detail away.
struct Bin {
    idx: usize,
    end: usize,
    midi: f32,
    t: f32,
}

pub(super) fn draw_spectrogram(
    painter: &egui::Painter,
    axes: &Axes,
    scale: &PitchScale,
    state: &mut SharedState,
    split: f32,
    now: f64,
    // Which texture slot to build into (0 the docked pane / offline render, 1
    // the Render preview) — two live spectrograms in a frame need their own.
    surface: usize,
) {
    // Small copies, so `state.spectrum` is then free to take mutably (its
    // texture handle) without fighting the config/frame reads.
    let cfg = state.spectrum_config;
    let frame = state.frame_params;
    // Shared time<->depth mapping: a `now`-anchored scrolling window live, or
    // the whole take laid out statically (offline playhead mode).
    let time = TimeAxis::new(state, split, now);
    let whole = state.whole_song.as_ref();
    let spectrum = &mut state.spectrum;
    let opacity = cfg.spectrogram_opacity.clamp(0.0, 1.0);
    // Columns come from the precomputed whole-take set (playhead mode) or the
    // live ring.
    let enough = match whole {
        Some(ws) => ws.columns.len() >= 2,
        None => spectrum.history().len() >= 2,
    };
    if opacity <= 0.0 || !enough {
        return;
    }
    let depth_span = 1.0 - split;
    let window = time.window();
    let oldest = time.oldest();

    // ---- Layout the image is built on (rows x time-slabs) --------------------
    // Both feed the cache key below AND the rebuild, so they're computed up
    // front. `rows` is one per pitch pixel; `bucket` is the time-slab width.
    let max_rows = painter.ctx().input(|i| i.max_texture_side).max(64);
    let rows = (axes.pitch_len().round() as usize).clamp(2, max_rows);
    // One image column per output depth pixel; whole-song spans the entire take,
    // so it needs a higher cap than the live window.
    let col_cap = if whole.is_some() { 4096.0 } else { 512.0 };
    let target_cols = (depth_span * axes.depth_len()).round().clamp(2.0, col_cap) as usize;
    // Never subdivide finer than the data arrives (~50 ms FFT period, plus a
    // little for frame jitter). A shorter bucket leaves empty buckets between
    // columns, and the texture's linear time axis assumes evenly-spaced slabs —
    // gaps there stretch the edge columns into flat streaks (short spans).
    const MIN_BUCKET: f64 = 0.08;
    let bucket = (window / target_cols as f64).max(MIN_BUCKET);

    // ---- Data identity -------------------------------------------------------
    // `first` is the oldest in-window column (live); it advances as the window
    // scrolls a column off the far end. `newest` moves whenever a fresh column
    // arrives — catching it even in a saturated ring, where the count holds
    // steady. Whole-song draws the entire fixed set, so `first` is 0.
    let (first, cols_len, newest) = match whole {
        Some(ws) => (0, ws.columns.len(), ws.columns.last().map_or(now, |c| c.time)),
        None => {
            let hist = spectrum.history();
            let first = hist.partition_point(|c| c.time < oldest).saturating_sub(1);
            (first, hist.len(), hist.back().map_or(now, |c| c.time))
        }
    };

    // The heatmap pixels are a pure function of these; if none has changed since
    // the uploaded texture was built, the whole rebuild below is dead work.
    let key = crate::SpectrogramKey::new(
        rows,
        bucket,
        scale.min_midi,
        scale.span,
        first,
        cols_len,
        newest,
        whole.is_some(),
        cfg,
        frame,
    );

    // Fast path: the built image is still valid — reuse the uploaded texture and
    // its geometry; only the scrolling quad below is recomputed (with `now`).
    let reused = match &spectrum.spectrogram_cache[surface] {
        Some(c) if c.matches(&key) && spectrum.spectrogram_tex[surface].is_some() => Some(c.geometry()),
        _ => None,
    };

    let (t_origin, tex_span, t0, tn) = match reused {
        Some(geometry) => geometry,
        None => {
            // The image's rows: one per pixel of the pitch axis, never more
            // buckets than the axis holds and never a taller image than the GPU
            // will take. A row's pitch span maps back to a run of source
            // buckets, which it reduces by MAX when it covers several (a peak
            // must not be lost to averaging) and simply repeats when it covers
            // less than one. A bucket of slack on each side lets the filtering
            // carry the visible range cleanly to its edges.
            let bin_semis = 1.0 / BINS_PER_SEMITONE as f32;
            let margin = (bin_semis / scale.span).min(0.5);
            let bucket_of = |t: f32| {
                let midi = scale.min_midi + t * scale.span;
                (((midi - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32).floor() as isize)
                    .clamp(0, SPECTRUM_BINS as isize - 1) as usize
            };
            let bins: Vec<Bin> = (0..rows)
                .map(|r| {
                    // The row's own slice of the visible pitch range, widened by
                    // the margin so the edge rows reach past the range like the
                    // buckets did.
                    let span = 1.0 + 2.0 * margin;
                    let t0 = -margin + span * r as f32 / rows as f32;
                    let t1 = -margin + span * (r + 1) as f32 / rows as f32;
                    let (idx, last) = (bucket_of(t0), bucket_of(t1));
                    let t = 0.5 * (t0 + t1);
                    Bin {
                        idx,
                        end: (last + 1).max(idx + 1).min(SPECTRUM_BINS),
                        midi: scale.min_midi + t * scale.span,
                        t,
                    }
                })
                .collect();

            // Aggregate the in-window columns into one image column per depth
            // pixel by a FIXED time grid, MAX within each slab (keeps a short
            // note's peak and pins it against the scroll — see `aggregate_rows`).
            let bin_idx: Vec<(usize, usize)> = bins.iter().map(|b| (b.idx, b.end)).collect();
            let (centers, mut power) = match whole {
                // Offline whole-song: fixed column set, already cached after the
                // first frame — a plain batch aggregate.
                Some(ws) => aggregate_rows(ws.columns.iter(), &bin_idx, bucket),
                // Live: fold only the new column(s) into the kept slab grid
                // instead of rescanning the whole window every rebuild. `hist`
                // and the aggregator are disjoint fields of `spectrum`.
                None => {
                    let hist = &spectrum.history;
                    let agg = spectrum.spectrogram_agg[surface].get_or_insert_with(SpectrogramAgg::new);
                    agg.window(hist, first, bucket, &bin_idx)
                }
            };
            let (w, h) = (centers.len(), bins.len());
            if w < 2 {
                return;
            }
            // Optional temporal smoothing: average out fast beating/chorus wobble.
            smooth_time(&mut power, w, h, cfg.spectrogram_smoothing);

            // The image covers absolute time `[t_origin, t_origin + w*bucket]` —
            // the oldest slab's start to the newest slab's end. Its texel
            // centers sit at the slab centers, so `u = (t - t_origin) / span`
            // places time exactly.
            let t_origin = centers[0] - 0.5 * bucket;
            let tex_span = w as f64 * bucket;
            let (t0, tn) = (bins[0].t, bins[h - 1].t);
            if tex_span < 1e-9 || (tn - t0).abs() < 1e-6 {
                return;
            }

            // Build and upload the image (pixel (x = slab, y = bin), y = 0 low pitch).
            let pixels = fill_pixels(&cfg, &frame, w, &bins, &power);
            let image = egui::ColorImage::new([w, h], pixels);
            let opts = egui::TextureOptions::LINEAR; // bilinear + ClampToEdge
            match &mut spectrum.spectrogram_tex[surface] {
                Some(handle) => handle.set(image, opts),
                slot => *slot = Some(painter.ctx().load_texture("spectrogram", image, opts)),
            }
            spectrum.spectrogram_cache[surface] =
                Some(crate::SpectrogramCache::new(key, t_origin, tex_span, t0, tn));
            (t_origin, tex_span, t0, tn)
        }
    };

    let Some(tex) = &spectrum.spectrogram_tex[surface] else { return };

    // Map a screen depth to the texture's time axis CONTINUOUSLY, through the
    // roll's own `now`-anchored depth<->time relation (unclamped, the inverse
    // of `depth_of`). This is the fix for the image not tracking the notes and
    // for the per-slab stutter: `u` now slides with `now` frame to frame,
    // exactly as a note ribbon at the same depth does, instead of being pinned
    // to the slab endpoints (which jump a whole slab at a time). `v` maps pitch
    // to the bin rows. UVs run past 0..1 in the thin slivers with no data (the
    // sub-slab at `now`, the bit beyond the oldest slab); ClampToEdge fills
    // those from the edge column so the plane stays whole.
    let u_at = |d: f32| ((time.time_at(d) - t_origin) / tex_span) as f32;
    let v_at = |p: f32| (p - t0) / (tn - t0);
    let tint = Color32::from_white_alpha((opacity * 255.0) as u8);
    let vert =
        |p: f32, d: f32| egui::epaint::Vertex { pos: axes.at(p, d), uv: egui::pos2(u_at(d), v_at(p)), color: tint };

    // The quad only spans the depths the data actually reaches, so the drawn
    // strip GROWS from the now-line as history accumulates rather than being
    // stretched to fill the whole region. Without the far cap, clearing the
    // spectrogram (or startup) left a handful of fresh columns ClampToEdge-
    // smeared across everything as trails.
    //
    // Near edge: to the split while fresh columns keep arriving, but stopping
    // at the newest data once it goes stale — most visibly when switching the
    // window algorithm, which empties the ring for a window's worth of samples
    // and would otherwise smear the last pre-switch slice over the growing gap.
    // The grace keeps the ordinary ~one-FFT lag from opening a flickering
    // sliver. Far edge: the oldest slab's depth, which is 1 once history spans
    // the window (depth_of clamps there) and nearer while it is still filling.
    let (d_near, d_far) = if time.whole_song() {
        // The whole take is present from the first frame, so the strip fills the
        // region edge to edge; only the playhead moves.
        (time.depth_of(t_origin), time.depth_of(t_origin + tex_span))
    } else {
        const FRESH: f64 = 0.12;
        let near = if now - newest <= FRESH { split } else { time.depth_of(newest) };
        (near, time.depth_of(t_origin))
    };

    // One quad over pitch [0,1] x depth [d_near, d_far]; GPU bilinear-samples it.
    let mut mesh = egui::Mesh::with_texture(tex.id());
    mesh.vertices.push(vert(0.0, d_far)); // far, low
    mesh.vertices.push(vert(1.0, d_far)); // far, high
    mesh.vertices.push(vert(1.0, d_near)); // near, high
    mesh.vertices.push(vert(0.0, d_near)); // near, low
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Group `columns` (oldest first) into time-slabs of `bucket` seconds, taking
/// the MAX over each slab AND over each output row's run of source buckets
/// (`bin_idx` gives the `start..end` a row draws from). Returns each slab's
/// center time and a flat row-major power grid (`rows * nb`).
///
/// MAX on both axes for the same reason: a spectrogram cell answers "was there
/// anything here", and averaging a bright thin partial with its silent
/// neighbours answers "not much".
///
/// The slab a column lands in is `floor(time / bucket)` — a function of
/// absolute time alone, so it doesn't move as columns scroll off the far end
/// of the ring. That, plus MAX (rather than dropping samples), is what stops a
/// short, bright note from flickering: its peak is kept and stays in one
/// slowly-scrolling slab instead of blinking in and out with the sampling.
fn aggregate_rows<'a>(
    columns: impl Iterator<Item = &'a crate::SpectrogramColumn>,
    bin_idx: &[(usize, usize)],
    bucket: f64,
) -> (Vec<f64>, Vec<f32>) {
    let mut grid = SlabGrid::default();
    for col in columns {
        grid.fold(col, bin_idx, bucket);
    }
    (grid.centers, grid.power)
}

/// The growing slab grid the spectrogram image is built from: `centers[i]` is
/// slab `i`'s center time and `power` is the flat row-major `[slab][bin]` MAX
/// grid (`slab * nb + bin`). [`fold`](SlabGrid::fold) is the single per-column
/// step both [`aggregate_rows`] (batch, from scratch) and [`SpectrogramAgg`]
/// (incremental, live) drive — so the two can never disagree.
#[derive(Default, Clone)]
struct SlabGrid {
    centers: Vec<f64>,
    power: Vec<f32>,
    cur_key: Option<i64>,
}

impl SlabGrid {
    /// Fold one column (columns arrive oldest-first) into the grid, appending
    /// slabs and MAXing the column into the current one. Returns `false` iff
    /// the column ran BACKWARDS in time relative to the current slab — batch
    /// ignores the result (it just starts a fresh row, as before), while the
    /// incremental aggregator treats it as a broken invariant and rebuilds.
    fn fold(&mut self, col: &crate::SpectrogramColumn, bin_idx: &[(usize, usize)], bucket: f64) -> bool {
        let nb = bin_idx.len();
        let key = (col.time / bucket).floor() as i64;
        let forward = match self.cur_key {
            Some(k) if k == key => true,
            // A slab with no columns in it STILL gets a row, so the grid stays
            // one row per slab of elapsed time. The texture's time axis is
            // uniform — `u_at` maps time linearly across `w * bucket` — so
            // skipping an empty slab makes the rows either side of it
            // neighbouring texels, and the quad then stretches that pair over
            // the whole silent stretch: one flat color as wide as the silence.
            // Analysis stalls do happen (switching the FFT window empties the
            // ring for a window's worth of samples), and that band was the
            // result. Silence is what the analyzer actually had.
            Some(k) if key > k => {
                let empty = key - k - 1;
                for slot in (k + 1)..key {
                    self.centers.push((slot as f64 + 0.5) * bucket);
                    if empty <= JITTER_SLABS {
                        // Hold the previous column: at this width one empty
                        // slab is just a long frame, and painting it black
                        // would leave a stripe of false silence scrolling
                        // across the display for the rest of the window.
                        self.power.extend_from_within(self.power.len() - nb..);
                    } else {
                        self.power.resize(self.power.len() + nb, 0.0);
                    }
                }
                self.centers.push((key as f64 + 0.5) * bucket);
                self.power.resize(self.power.len() + nb, 0.0);
                self.cur_key = Some(key);
                true
            }
            // First column (None), or time running backwards (Some, key < k, a
            // transport jump): start a fresh row rather than fill a negative
            // gap. Only the backward case breaks the incremental invariant.
            other => {
                self.cur_key = Some(key);
                self.centers.push((key as f64 + 0.5) * bucket);
                self.power.resize(self.power.len() + nb, 0.0);
                other.is_none()
            }
        };
        let base = self.power.len() - nb;
        for (k, &(from, to)) in bin_idx.iter().enumerate() {
            let mut p = self.power[base + k];
            for src in from..to {
                if col.power[src] > p {
                    p = col.power[src];
                }
            }
            self.power[base + k] = p;
        }
        forward
    }
}

/// Live-only incremental spectrogram aggregation. `aggregate_rows` re-scans
/// EVERY in-window column on each ~20 Hz rebuild — O(columns-in-window), which
/// grows with the roll Span and is the residual creep the texture cache didn't
/// remove. This keeps the slab grid across frames instead: a rebuild folds only
/// the newly-arrived column(s) and drops the scrolled-out front, so its cost is
/// O(new columns), independent of how much history has accumulated.
///
/// It reproduces `aggregate_rows` EXACTLY: the shared [`SlabGrid::fold`] gives
/// identical slab values, and the front is trimmed to the same first slab batch
/// would start at. A layout change (bucket/bins), a backward transport jump, or
/// a window that jumped outside the kept grid falls back to a full rebuild —
/// each of which is just `aggregate_rows` again, so correctness never rides on
/// the fast path alone. The offline whole-song path does NOT use this (its
/// column set is fixed and already cached after the first frame).
pub(crate) struct SpectrogramAgg {
    grid: SlabGrid,
    bucket_bits: u64,
    bin_idx: Vec<(usize, usize)>,
    /// Time of the newest column already folded; the next update folds only
    /// columns past it.
    last_time: f64,
}

impl SpectrogramAgg {
    fn new() -> Self {
        SpectrogramAgg {
            grid: SlabGrid::default(),
            bucket_bits: 0,
            bin_idx: Vec::new(),
            last_time: f64::NEG_INFINITY,
        }
    }

    /// Re-fold the whole window from scratch (== `aggregate_rows(history[first..])`).
    fn rebuild(
        &mut self,
        history: &std::collections::VecDeque<crate::SpectrogramColumn>,
        first: usize,
        bucket: f64,
        bin_idx: &[(usize, usize)],
    ) {
        self.grid = SlabGrid::default();
        for col in history.iter().skip(first) {
            self.grid.fold(col, bin_idx, bucket);
        }
        self.bucket_bits = bucket.to_bits();
        self.bin_idx.clear();
        self.bin_idx.extend_from_slice(bin_idx);
        self.last_time = history.back().map_or(f64::NEG_INFINITY, |c| c.time);
    }

    /// The window's `(centers, power)`, maintained incrementally. `first` is the
    /// oldest in-window column index (as `draw_spectrogram` computes it), so the
    /// window's first slab is `floor(history[first].time / bucket)` — exactly
    /// where batch would start.
    fn window(
        &mut self,
        history: &std::collections::VecDeque<crate::SpectrogramColumn>,
        first: usize,
        bucket: f64,
        bin_idx: &[(usize, usize)],
    ) -> (Vec<f64>, Vec<f32>) {
        let target = history.get(first).map(|c| (c.time / bucket).floor() as i64);
        let newest = history.back().map_or(f64::NEG_INFINITY, |c| c.time);
        let layout_same = self.bucket_bits == bucket.to_bits() && self.bin_idx == bin_idx;
        // The fast path is valid only when: the layout is unchanged, we have a
        // prior grid, time hasn't gone backwards, and the window's first slab
        // still sits inside the grid we kept (front..=back). Anything else is a
        // full rebuild, which is always correct.
        let can_increment = layout_same
            && self.grid.cur_key.is_some()
            && newest >= self.last_time
            && target.zip(self.grid.centers.first()).zip(self.grid.cur_key).is_some_and(
                |((t, &front_center), back)| {
                    let front = (front_center / bucket).floor() as i64;
                    t >= front && t <= back
                },
            );

        if !can_increment {
            self.rebuild(history, first, bucket, bin_idx);
        } else {
            // Fold only columns newer than the last we folded.
            let start = history.partition_point(|c| c.time <= self.last_time);
            let mut forward = true;
            for col in history.iter().skip(start) {
                if !self.grid.fold(col, bin_idx, bucket) {
                    forward = false;
                    break;
                }
                self.last_time = col.time;
            }
            if !forward {
                // A mid-stream backward jump broke the grid; rebuild clean.
                self.rebuild(history, first, bucket, bin_idx);
            } else if let Some(t) = target {
                let nb = bin_idx.len();
                // Drop front slabs that have scrolled out of the window, leaving
                // the same first slab batch would produce.
                while self.grid.centers.len() > 1 {
                    let front = (self.grid.centers[0] / bucket).floor() as i64;
                    if front >= t {
                        break;
                    }
                    self.grid.centers.remove(0);
                    self.grid.power.drain(0..nb);
                }
                // The window's first slab is PARTIAL: batch folds only columns
                // from `first` onward, so an earlier column sharing this slab —
                // which we MAXed in before it fell behind `first` as the window
                // scrolled — must not count. Recompute just this one slab from
                // the in-window columns (a handful, so still O(1) per frame).
                for v in &mut self.grid.power[0..nb] {
                    *v = 0.0;
                }
                for c in history.iter().skip(first) {
                    if (c.time / bucket).floor() as i64 != t {
                        break;
                    }
                    for (k, &(from, to)) in bin_idx.iter().enumerate() {
                        for src in from..to {
                            if c.power[src] > self.grid.power[k] {
                                self.grid.power[k] = c.power[src];
                            }
                        }
                    }
                }
            }
        }
        (self.grid.centers.clone(), self.grid.power.clone())
    }
}

/// Smooth `power` (flat `rows * nb`, row-major over time slabs) along time,
/// in place. `smoothing` in `0..1` sets the strength: 0 leaves it untouched,
/// toward 1 blends ever more of each column into its neighbors. Runs an EMA
/// forward then backward, so the smoothing is symmetric (zero-phase) and a
/// peak doesn't drift in either time direction.
fn smooth_time(power: &mut [f32], rows: usize, nb: usize, smoothing: f32) {
    let a = 1.0 - smoothing.clamp(0.0, 0.95);
    if a >= 1.0 || rows < 2 {
        return;
    }
    for row in 1..rows {
        let (i, prev) = (row * nb, (row - 1) * nb);
        for k in 0..nb {
            power[i + k] += (1.0 - a) * (power[prev + k] - power[i + k]);
        }
    }
    for row in (0..rows - 1).rev() {
        let (i, next) = (row * nb, (row + 1) * nb);
        for k in 0..nb {
            power[i + k] += (1.0 - a) * (power[next + k] - power[i + k]);
        }
    }
}

/// The heatmap image, row-major `pixel(x = slab, y = bin)` at `[y * w + x]`,
/// with `y = 0` the lowest bin. `power` is the flat `w * bins.len()` grid from
/// [`aggregate_rows`]. Opaque throughout — silence is the ramp's dark end, so
/// the plane is filled rather than see-through.
fn fill_pixels(
    cfg: &SpectrumConfig,
    frame: &FrameParams,
    w: usize,
    bins: &[Bin],
    power: &[f32],
) -> Vec<Color32> {
    let h = bins.len();
    let mut pixels = vec![Color32::BLACK; w * h];
    for x in 0..w {
        let base = x * h;
        for (y, bin) in bins.iter().enumerate() {
            let p = power[base + y];
            let level = if p <= NEAR_ZERO { 0.0 } else { loudness(cfg, p, bin.midi) };
            pixels[y * w + x] = cell_color(cfg.spectrogram_color, level, bin.midi, frame);
        }
    }
    pixels
}

/// A cell's opaque color: `level` (0..1 loudness) mapped through the chosen
/// ramp. The ramp's dark end is black, matching the region's black bed (laid
/// down in `spectral_pane`), so silence recedes while energy stands out — the
/// overall opacity is applied once, as the quad's tint. Shared with the spectrum
/// curve so the two read in the same scheme.
pub(super) fn cell_color(kind: SpectrogramColor, level: f32, midi: f32, frame: &FrameParams) -> Color32 {
    let t = level.clamp(0.0, 1.0);
    let rgb = match kind {
        SpectrogramColor::Mono => ramp(t, &[[0, 0, 0], [255, 255, 255]]),
        SpectrogramColor::Heat => ramp(
            t,
            &[[0, 0, 0], [90, 0, 20], [200, 40, 20], [240, 150, 30], [255, 240, 180]],
        ),
        SpectrogramColor::Ice => ramp(
            t,
            &[[0, 0, 0], [10, 20, 70], [20, 70, 180], [60, 180, 220], [220, 250, 255]],
        ),
        SpectrogramColor::Aurora => ramp(
            t,
            &[[0, 0, 0], [50, 10, 90], [30, 90, 120], [40, 170, 100], [230, 230, 60]],
        ),
        SpectrogramColor::Pitch => {
            // The lattice's own pitch color, scaled toward black by loudness so
            // quiet cells stay dark while keeping their hue.
            let c = channel_color(PITCH_RAMP_CHANNEL, midi, frame.darkest_pitch, frame.brightest_pitch);
            let s = t.sqrt(); // lift the low end a touch so faint pitches still show
            [
                (c.x.clamp(0.0, 1.0) * s * 255.0) as u8,
                (c.y.clamp(0.0, 1.0) * s * 255.0) as u8,
                (c.z.clamp(0.0, 1.0) * s * 255.0) as u8,
            ]
        }
    };
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

/// Sample an evenly-spaced list of RGB stops at `t` in `0..1`, linearly
/// interpolating between the two nearest.
fn ramp(t: f32, stops: &[[u8; 3]]) -> [u8; 3] {
    let n = stops.len();
    if n == 1 {
        return stops[0];
    }
    let x = t.clamp(0.0, 1.0) * (n - 1) as f32;
    let i = (x.floor() as usize).min(n - 2);
    let f = x - i as f32;
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f) as u8;
    [
        lerp(stops[i][0], stops[i + 1][0]),
        lerp(stops[i][1], stops[i + 1][1]),
        lerp(stops[i][2], stops[i + 1][2]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_core::spectrum::SPECTRUM_BINS;

    /// A column at `time` with the given (bin, power) energy, rest silent.
    fn col(time: f64, energy: &[(usize, f32)]) -> crate::SpectrogramColumn {
        let mut power = Box::new([0.0f32; SPECTRUM_BINS]);
        for &(i, p) in energy {
            power[i] = p;
        }
        crate::SpectrogramColumn { time, power }
    }

    #[test]
    fn a_short_loud_column_keeps_its_peak_through_aggregation() {
        // A brief loud note between two quiet columns, all in one slab. MAX
        // must keep the peak — the flicker came from dropping this thin, bright
        // sample.
        let cols = [
            col(0.00, &[(5, 0.001)]),
            col(0.02, &[(5, 1.0)]), // the short note
            col(0.04, &[(5, 0.002)]),
        ];
        let (centers, power) = aggregate_rows(cols.iter(), &[(5, 6)], 1.0);
        assert_eq!(centers.len(), 1, "one slab of width 1.0 s holds all three");
        assert_eq!(power[0], 1.0, "the short note's peak survives");
    }

    /// A stall in the analyzer leaves a hole in the column stream — switching
    /// the FFT window empties its ring for a whole window's worth of samples,
    /// and nothing is measured until it refills. Every slab of elapsed time
    /// still needs its row: the texture's time axis is uniform, so without
    /// them the columns either side of the hole become neighbouring texels and
    /// the quad stretches that pair across the entire silent stretch — a band
    /// of one flat color, exactly as wide as the stall.
    #[test]
    fn a_gap_in_the_columns_keeps_a_row_for_every_silent_slab() {
        // Two columns a second apart, in quarter-second slabs: four slabs of
        // silence between them.
        let cols = [col(0.0, &[(5, 1.0)]), col(1.0, &[(5, 0.5)])];
        let (centers, power) = aggregate_rows(cols.iter(), &[(5, 6)], 0.25);
        assert_eq!(centers.len(), 5, "one row per slab, silent ones included");
        assert_eq!(power[0], 1.0, "the column before the gap");
        assert_eq!(&power[1..4], [0.0, 0.0, 0.0], "the gap reads as silence, not as a smear");
        assert_eq!(power[4], 0.5, "the column after it");
        // Evenly spaced centers are exactly what the texture mapping assumes.
        for pair in centers.windows(2) {
            assert!((pair[1] - pair[0] - 0.25).abs() < 1e-9, "slabs must stay uniform: {centers:?}");
        }
    }

    /// One missed slab is a long frame, not a stall, and holds the previous
    /// column. Painting it black would mean every stutter left a stripe of
    /// false silence scrolling across the display for the rest of the window
    /// — trading a rare artifact for a routine one.
    #[test]
    fn a_single_missed_slab_holds_instead_of_going_black() {
        // Columns half a second apart in quarter-second slabs: one slab empty.
        let cols = [col(0.0, &[(5, 1.0)]), col(0.5, &[(5, 0.5)])];
        let (centers, power) = aggregate_rows(cols.iter(), &[(5, 6)], 0.25);
        assert_eq!(centers.len(), 3, "the empty slab still gets its row");
        assert_eq!(power[1], 1.0, "and holds the column before it");
        assert_eq!(power[2], 0.5);
    }

    /// Time running backwards (a transport jump) starts a fresh row rather
    /// than trying to fill a negative gap — which would be an enormous loop,
    /// or a silent no-row.
    #[test]
    fn columns_going_back_in_time_still_get_a_row() {
        let cols = [col(10.0, &[(5, 1.0)]), col(1.0, &[(5, 0.5)])];
        let (centers, power) = aggregate_rows(cols.iter(), &[(5, 6)], 0.25);
        assert_eq!(centers.len(), 2);
        assert_eq!(power[1], 0.5, "the rewound column landed in its own row");
    }

    #[test]
    fn a_slab_is_anchored_to_absolute_time_not_ring_position() {
        // The same note must land in the same slab whether or not older columns
        // are present — otherwise scrolling would shift it and it would shimmer.
        // A note at t=2.6 sits in slab floor(2.6)=2.
        let with_old = [col(0.1, &[(0, 0.1)]), col(2.6, &[(0, 0.5)])];
        let (c_full, _) = aggregate_rows(with_old.iter(), &[(0, 1)], 1.0);
        let just_note = [col(2.6, &[(0, 0.5)])];
        let (c_scrolled, _) = aggregate_rows(just_note.iter(), &[(0, 1)], 1.0);
        assert!(c_full.contains(&2.5), "slab center is 2.5 with old columns");
        assert!(c_scrolled.contains(&2.5), "and still 2.5 after they scroll off");
    }

    #[test]
    fn fill_pixels_places_energy_at_the_right_slab_and_bin() {
        // Two slabs, three bins. Put a loud value in slab x=1, bin y=2 and
        // check it lands at pixel [y*w + x] and nowhere else is bright.
        let cfg = SpectrumConfig::default();
        let frame = FrameParams::default();
        let w = 2;
        let bins = [
            Bin { idx: 10, end: 11, midi: 40.0, t: 0.1 },
            Bin { idx: 11, end: 12, midi: 41.0, t: 0.2 },
            Bin { idx: 12, end: 13, midi: 42.0, t: 0.3 },
        ];
        let mut power = vec![0.0f32; w * bins.len()]; // row-major [slab][bin]
        power[bins.len() + 2] = 1.0; // slab 1, bin 2 loud
        let px = fill_pixels(&cfg, &frame, w, &bins, &power);
        let lum = |c: Color32| c.r() as u32 + c.g() as u32 + c.b() as u32;
        let loud = px[2 * w + 1]; // y=2, x=1
        assert!(lum(loud) > 0, "the loud cell should carry color");
        // Every other pixel is silence -> the ramp's dark end.
        for (i, &c) in px.iter().enumerate() {
            if i != 2 * w + 1 {
                assert_eq!(lum(c), 0, "pixel {i} should be dark, got {c:?}");
            }
        }
    }

    #[test]
    fn cells_are_opaque_and_run_dark_to_bright() {
        let frame = FrameParams::default();
        let quiet = cell_color(SpectrogramColor::Heat, 0.0, 60.0, &frame);
        let loud = cell_color(SpectrogramColor::Heat, 1.0, 60.0, &frame);
        // Opaque throughout (opacity is applied as the quad tint, not here).
        assert_eq!(quiet.a(), 255);
        assert_eq!(loud.a(), 255);
        // Silence is the dark end; loud is brighter.
        let lum = |c: Color32| c.r() as u32 + c.g() as u32 + c.b() as u32;
        assert_eq!(lum(quiet), 0, "silence is the ramp's black end");
        assert!(lum(loud) > lum(quiet));
    }

    #[test]
    fn smoothing_off_is_a_no_op_and_on_spreads_a_spike_without_drift() {
        // One bin (nb=1), a spike well inside a long run of slabs so the
        // forward+backward passes reach a symmetric interior (edge rows aside).
        let n = 15;
        let center = 7;
        let mut base = vec![0.0f32; n];
        base[center] = 1.0;

        // Off: untouched.
        let mut p = base.clone();
        smooth_time(&mut p, n, 1, 0.0);
        assert_eq!(p, base);

        // On: the spike spreads into both neighbors, the peak drops, and the
        // two sides match — the forward+backward passes leave no time drift.
        let mut p = base.clone();
        smooth_time(&mut p, n, 1, 0.7);
        assert!(p[center] < 1.0, "peak drops as it spreads");
        assert!(p[center - 1] > 0.0 && p[center + 1] > 0.0, "reaches both neighbors");
        assert!((p[center - 1] - p[center + 1]).abs() < 1e-3, "no time-direction drift");
        assert!(p[center - 1] > p[center - 2], "monotonic falloff away from the peak");
    }

    #[test]
    fn ramp_hits_its_endpoints_and_midpoint() {
        let stops = [[0, 0, 0], [100, 100, 100], [200, 200, 200]];
        assert_eq!(ramp(0.0, &stops), [0, 0, 0]);
        assert_eq!(ramp(1.0, &stops), [200, 200, 200]);
        assert_eq!(ramp(0.5, &stops), [100, 100, 100]);
        assert_eq!(ramp(0.25, &stops), [50, 50, 50]);
    }

    /// The incremental aggregator must produce EXACTLY what a from-scratch
    /// `aggregate_rows` over the window would, at every step — otherwise the
    /// live spectrogram would drift from what the batch/offline path draws. This
    /// walks a column stream with same-slab clusters, a one-slab jitter gap
    /// (hold-previous), a multi-slab gap (zeros), steady scroll (so `first`
    /// advances and the front trims), a ring trim (so indices shift), and a
    /// bucket change (a forced rebuild), comparing byte-for-byte each step.
    #[test]
    fn incremental_aggregation_matches_batch_step_for_step() {
        use std::collections::VecDeque;
        let bin_idx = [(4usize, 6usize), (6, 10), (10, 11)]; // 3 rows, varied ranges
        let bucket = 0.25;
        let window_span = 1.0;
        // Exercises: cluster (0.30, 0.31), 1-slab gap (0.55->0.80 is 1 apart;
        // 0.80->1.60 is a multi-slab gap), then steady scroll.
        let times: [f64; 14] = [
            0.05, 0.10, 0.30, 0.31, 0.55, 0.80, 1.60, 1.62, 1.90, 2.15, 2.40, 2.65, 2.90, 3.15,
        ];

        let mut agg = SpectrogramAgg::new();
        let mut history: VecDeque<crate::SpectrogramColumn> = VecDeque::new();
        for (i, &t) in times.iter().enumerate() {
            // Per-column, per-bin energy, so a wrong slab or a stale hold surfaces
            // as a value mismatch, not just a shape one.
            let e = [(4, 0.1 * (i as f32 + 1.0)), (7, 0.05 * i as f32), (10, 1.0 - 0.03 * i as f32)];
            history.push_back(col(t, &e));
            // Trim the ring, so `first` indices shift under the aggregator.
            while history.front().is_some_and(|c| c.time < t - (window_span + 0.5)) {
                history.pop_front();
            }
            let oldest = t - window_span;
            let first = history.partition_point(|c| c.time < oldest).saturating_sub(1);
            let inc = agg.window(&history, first, bucket, &bin_idx);
            let bat = aggregate_rows(history.iter().skip(first), &bin_idx, bucket);
            assert_eq!(inc, bat, "incremental != batch at step {i} (t={t})");
        }

        // A layout change (new bucket) must fall back to a rebuild — still exact.
        let now = *times.last().unwrap();
        let first = history.partition_point(|c| c.time < now - window_span).saturating_sub(1);
        let inc = agg.window(&history, first, 0.4, &bin_idx);
        let bat = aggregate_rows(history.iter().skip(first), &bin_idx, 0.4);
        assert_eq!(inc, bat, "incremental != batch after a bucket change");
    }

    /// The cache reuses the uploaded texture only while its key matches, so the
    /// key must move for EVERY input the pixels depend on — otherwise a change
    /// would leave a stale image on screen. Identical inputs must compare equal
    /// (the common case, a cache hit); each varied input must not.
    #[test]
    fn spectrogram_key_is_sensitive_to_every_input() {
        let cfg = SpectrumConfig::default();
        let frame = FrameParams::default();
        let base =
            || crate::SpectrogramKey::new(100, 0.1, 40.0, 48.0, 3, 200, 5.0, false, cfg, frame);
        // Same inputs -> equal: this is the hit that skips the rebuild.
        assert_eq!(base(), base());
        // Every layout / data field participates.
        let vary = |k: crate::SpectrogramKey| assert_ne!(base(), k);
        vary(crate::SpectrogramKey::new(101, 0.1, 40.0, 48.0, 3, 200, 5.0, false, cfg, frame)); // rows
        vary(crate::SpectrogramKey::new(100, 0.2, 40.0, 48.0, 3, 200, 5.0, false, cfg, frame)); // bucket
        vary(crate::SpectrogramKey::new(100, 0.1, 41.0, 48.0, 3, 200, 5.0, false, cfg, frame)); // scale min
        vary(crate::SpectrogramKey::new(100, 0.1, 40.0, 49.0, 3, 200, 5.0, false, cfg, frame)); // scale span
        vary(crate::SpectrogramKey::new(100, 0.1, 40.0, 48.0, 4, 200, 5.0, false, cfg, frame)); // first (scroll)
        vary(crate::SpectrogramKey::new(100, 0.1, 40.0, 48.0, 3, 201, 5.0, false, cfg, frame)); // count
        vary(crate::SpectrogramKey::new(100, 0.1, 40.0, 48.0, 3, 200, 6.0, false, cfg, frame)); // newest
        vary(crate::SpectrogramKey::new(100, 0.1, 40.0, 48.0, 3, 200, 5.0, true, cfg, frame)); // whole
        // And the color inputs: a palette/floor/smoothing change (cfg) or a
        // gradient-range change (frame) would recolor every pixel.
        let mut cfg2 = cfg;
        cfg2.spectrogram_smoothing += 0.1;
        vary(crate::SpectrogramKey::new(100, 0.1, 40.0, 48.0, 3, 200, 5.0, false, cfg2, frame));
        let mut frame2 = frame;
        frame2.brightest_pitch += 1.0;
        vary(crate::SpectrogramKey::new(100, 0.1, 40.0, 48.0, 3, 200, 5.0, false, cfg, frame2));
    }
}
