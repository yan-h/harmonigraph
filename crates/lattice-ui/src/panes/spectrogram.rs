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
use lattice_core::spectrogram::{db_of, BucketDb};
use lattice_core::spectrum::{BINS_PER_SEMITONE, SPECTRUM_BINS, SPECTRUM_MIN_MIDI};
use lattice_scene::FrameParams;

use super::spectral::{spectrogram_level_db, Axes, PitchScale, TimeAxis};
use crate::{SharedState, SpectrogramColor, SpectrumConfig};

/// Most time slabs a live window is ever cut into, whatever the pane's size —
/// and so, with the window, the FINEST slab any given moment can be drawn into.
/// That is what [`SpectrumHistory`](lattice_core::SpectrumHistory) sizes its
/// tiers against: a column of age `a` is only on screen when the window is at
/// least `a` long, so it never needs storing finer than `a / LIVE_SLAB_CAP`.
pub(crate) const LIVE_SLAB_CAP: f32 = 512.0;
/// The same for the offline whole-song build, which spans an entire take rather
/// than a scrolling window and so wants more of them.
const WHOLE_SONG_SLAB_CAP: f32 = 4096.0;
/// Never subdivide finer than the data arrives (~20 ms FFT period, plus a
/// little for frame jitter). A shorter bucket leaves empty buckets between
/// columns, and the texture's linear time axis assumes evenly-spaced slabs —
/// gaps there stretch the edge columns into flat streaks (short spans).
/// Tracks `AudioSpectrum::FFT_INTERVAL` at the same 1.6x ratio it always had.
pub(crate) const MIN_BUCKET: f64 = 0.032;

/// A run of empty slabs this short is sampling jitter rather than a stall in
/// the analyzer, and holds the previous column instead of reading as silence.
///
/// The analyzer produces a column roughly every 20 ms and the narrowest slab
/// is 32 ms, so the two are within a factor of two of each other: one long
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

/// Where the visible slabs sit in the uploaded texture, and what pitch range
/// its rows cover — everything the quad's `u`/`v` need.
///
/// One shape for both builds. A full-width build parks the visible slabs at
/// `x0 = 0` in a texture exactly `tex_w = w` wide, where the mapping collapses
/// to the plain `(t - t_origin) / tex_span`; the ring parks them at a rotating
/// offset inside a wider texture. Keeping one formula means the scrolling quad
/// has no idea which built it.
#[derive(Clone, Copy)]
pub(crate) struct TexLayout {
    /// Absolute time at the left edge of the first visible slab.
    pub(crate) t_origin: f64,
    /// Seconds the visible slabs span.
    pub(crate) tex_span: f64,
    /// Pitch-axis fraction of the first and last row.
    pub(crate) t0: f32,
    pub(crate) tn: f32,
    /// Texel x of the first visible slab, and the texture's full width.
    pub(crate) x0: f32,
    pub(crate) tex_w: f32,
}

/// Which slabs the uploaded texture's columns currently hold.
///
/// The heatmap's columns are indexed by SLAB KEY — `floor(time / bucket)`, a
/// function of absolute time alone (see [`SlabGrid::fold`]) — so a column keeps
/// its identity as the window scrolls past it. That is what lets a new column
/// be written on its own: everything older is already correct where it sits,
/// and only the newest slab (still accumulating its MAX) plus any slab that
/// just appeared need repainting.
///
/// **Every column is written twice**, `capacity` texels apart in a texture
/// `2 * capacity` wide. A ring read as a ring wraps, and a wrapped run needs
/// two quads with a seam between them that bilinear filtering then blends
/// across. Written twice, any run of at most `capacity` slabs is contiguous
/// somewhere in the texture, so the existing single quad still works and the
/// seam cannot be sampled. The cost is a second `set_partial` of one column —
/// still O(rows), against O(rows × slabs) for the repaint it replaces.
pub(crate) struct SpectrogramRing {
    /// Slabs held; the texture is twice this wide.
    capacity: usize,
    /// Everything that decides a pixel's colour, or which buckets a row reads.
    /// A change to any of it invalidates every column at once.
    style: RingStyle,
    /// Newest slab key written, and the oldest still valid.
    written_through: i64,
    oldest_valid: i64,
}

/// The part of [`crate::SpectrogramKey`] that outlives a scroll: everything
/// except which columns are in the window. Two builds sharing a `RingStyle`
/// can share columns; anything else has to start over.
#[derive(Clone, PartialEq)]
struct RingStyle {
    rows: usize,
    bucket_bits: u64,
    scale_min_bits: u32,
    scale_span_bits: u32,
    cfg: SpectrumConfig,
    frame: FrameParams,
}

impl SpectrogramRing {
    /// Texel x of a slab key. The `+ capacity` twin is written by the caller.
    fn x_of(&self, key: i64) -> usize {
        key.rem_euclid(self.capacity as i64) as usize
    }
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
    let col_cap = if whole.is_some() { WHOLE_SONG_SLAB_CAP } else { LIVE_SLAB_CAP };
    let target_cols = (depth_span * axes.depth_len()).round().clamp(2.0, col_cap) as usize;
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

    let layout = match reused {
        Some(layout) => layout,
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
            let (centers, power) = match whole {
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

            // The offline whole-song build keeps the full-width path: its
            // column set is fixed and already cached after the first frame, so
            // there is nothing for a ring to save.
            let ring_able = whole.is_none();
            let layout = if ring_able {
                let style = RingStyle {
                    rows: h,
                    bucket_bits: bucket.to_bits(),
                    scale_min_bits: scale.min_midi.to_bits(),
                    scale_span_bits: scale.span.to_bits(),
                    cfg,
                    frame,
                };
                // Sized off the WINDOW, not off how much history has arrived:
                // a capacity that grew with the column count would change on
                // almost every frame and rebuild the very thing it caches.
                // +2 so a guard column fits on each side of the visible run
                // without overwriting a column the run still needs.
                let capacity = ((window / bucket).ceil() as usize + 2).max(w + 2);
                write_ring(
                    painter.ctx(),
                    spectrum,
                    surface,
                    style,
                    capacity,
                    &cfg,
                    &bins,
                    &power,
                    (centers[0] / bucket).floor() as i64,
                    w,
                );
                let ring = spectrum.spectrogram_ring[surface].as_ref();
                let x0 = ring.map_or(0.0, |r| r.x_of((centers[0] / bucket).floor() as i64) as f32);
                let tex_w = ring.map_or(w as f32, |r| (r.capacity * 2) as f32);
                TexLayout { t_origin, tex_span, t0, tn, x0, tex_w }
            } else {
                // The full-width build owns the whole texture, so any ring
                // bookkeeping describing it is now a lie about which slabs its
                // columns hold.
                spectrum.spectrogram_ring[surface] = None;
                // Build and upload the image (pixel (x = slab, y = bin), y = 0 low pitch).
                let pixels = fill_pixels(&cfg, w, &bins, &power);
                let image = egui::ColorImage::new([w, h], pixels);
                let opts = egui::TextureOptions::LINEAR; // bilinear + ClampToEdge
                match &mut spectrum.spectrogram_tex[surface] {
                    Some(handle) => handle.set(image, opts),
                    slot => *slot = Some(painter.ctx().load_texture("spectrogram", image, opts)),
                }
                TexLayout { t_origin, tex_span, t0, tn, x0: 0.0, tex_w: w as f32 }
            };
            spectrum.spectrogram_cache[surface] = Some(crate::SpectrogramCache::new(
                key, t_origin, tex_span, t0, tn, layout.x0, layout.tex_w,
            ));
            layout
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
    // Slabs occupy texels `x0 .. x0 + visible` of a `tex_w`-wide texture; for a
    // full-width build that is the whole thing and this collapses to the plain
    // `(t - t_origin) / tex_span`.
    //
    // Deliberately NOT clamped. `u` has to stay a straight function of time —
    // that continuity is what makes the image track the notes and killed the
    // per-slab stutter, because these are VERTEX UVs and the fragment scale is
    // interpolated between them. Pinning one end to the edge texel freezes it
    // for part of every slab and slides it for the rest, which shows up as the
    // whole heatmap twitching once per slab. The data-less slivers are handled
    // where they belong, in the data: `write_ring` keeps a duplicate of each
    // edge column just outside the run, which is what ClampToEdge did for free
    // before the texture became a ring.
    let u_at = |d: f32| u_of(&layout, time.time_at(d), bucket);
    let v_at = |p: f32| (p - layout.t0) / (layout.tn - layout.t0);
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
        (time.depth_of(layout.t_origin), time.depth_of(layout.t_origin + layout.tex_span))
    } else {
        // Plus the lag every healthy column has by construction: it is stamped
        // at the middle of the window it measured, so the newest one is always
        // half a window old. Without that term the strip stops short of the
        // now-line whenever the window is long, and the gap changes size with
        // the window setting.
        const FRESH: f64 = 0.12;
        let stale_after = FRESH + spectrum.column_lag();
        let near = if now - newest <= stale_after { split } else { time.depth_of(newest) };
        (near, time.depth_of(layout.t_origin))
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
) -> (Vec<f64>, Vec<BucketDb>) {
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
///
/// Held in the same dB bytes the columns are stored in: MAX is order-preserving
/// under the encoding, so aggregating in it is exact, and the grid — which is
/// rebuilt and cloned every frame — stays a quarter the size.
#[derive(Default, Clone)]
struct SlabGrid {
    centers: Vec<f64>,
    power: Vec<BucketDb>,
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
                        self.power.resize(self.power.len() + nb, 0);
                    }
                }
                self.centers.push((key as f64 + 0.5) * bucket);
                self.power.resize(self.power.len() + nb, 0);
                self.cur_key = Some(key);
                true
            }
            // First column (None), or time running backwards (Some, key < k, a
            // transport jump): start a fresh row rather than fill a negative
            // gap. Only the backward case breaks the incremental invariant.
            other => {
                self.cur_key = Some(key);
                self.centers.push((key as f64 + 0.5) * bucket);
                self.power.resize(self.power.len() + nb, 0);
                other.is_none()
            }
        };
        let base = self.power.len() - nb;
        for (k, &(from, to)) in bin_idx.iter().enumerate() {
            let mut p = self.power[base + k];
            for src in from..to {
                if col.db[src] > p {
                    p = col.db[src];
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
        history: &crate::SpectrumHistory,
        first: usize,
        bucket: f64,
        bin_idx: &[(usize, usize)],
    ) {
        self.grid = SlabGrid::default();
        for col in history.iter_from(first) {
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
        history: &crate::SpectrumHistory,
        first: usize,
        bucket: f64,
        bin_idx: &[(usize, usize)],
    ) -> (Vec<f64>, Vec<BucketDb>) {
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
            for col in history.iter_from(start) {
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
                    *v = 0;
                }
                for c in history.iter_from(first) {
                    if (c.time / bucket).floor() as i64 != t {
                        break;
                    }
                    for (k, &(from, to)) in bin_idx.iter().enumerate() {
                        for src in from..to {
                            if c.db[src] > self.grid.power[k] {
                                self.grid.power[k] = c.db[src];
                            }
                        }
                    }
                }
            }
        }
        (self.grid.centers.clone(), self.grid.power.clone())
    }
}


/// Texture `u` for an absolute time.
///
/// A straight line in `t`, with no clamping, and it must stay that way: these
/// are vertex UVs, so the scale every fragment samples at is interpolated
/// between the quad's corners. Bending or pinning either end changes the whole
/// image's scale, and doing it for only part of each slab — which is what
/// clamping to the edge texel does, since `now` crosses the last texel center
/// mid-slab — makes the heatmap twitch once per slab.
fn u_of(layout: &TexLayout, t: f64, bucket: f64) -> f32 {
    let slabs = ((t - layout.t_origin) / bucket) as f32;
    (layout.x0 + slabs) / layout.tex_w
}

/// Bring the ring's texture up to date for the visible slabs, allocating or
/// restarting it when it cannot be carried forward.
///
/// Only two columns can ever be stale: the newest slab, which is still
/// accumulating its MAX as columns fold in, and any slab that appeared since
/// the last call. Everything older already sits at the right texel with the
/// right pixels — that is the whole point of keying columns by absolute time.
#[allow(clippy::too_many_arguments)]
fn write_ring(
    ctx: &egui::Context,
    spectrum: &mut crate::AudioSpectrum,
    surface: usize,
    style: RingStyle,
    capacity: usize,
    cfg: &SpectrumConfig,
    bins: &[Bin],
    power: &[BucketDb],
    first_key: i64,
    visible: usize,
) {
    let h = bins.len();
    let tex_w = capacity * 2;
    let last_key = first_key + visible as i64 - 1;
    let opts = egui::TextureOptions::LINEAR; // bilinear + ClampToEdge

    // Carry the ring forward only when it describes THIS texture, in this
    // style, and still holds columns the visible run continues from. A gap
    // (the window jumped, history was cleared) would leave never-written
    // texels between the old columns and the new ones.
    let usable = match (&spectrum.spectrogram_ring[surface], &spectrum.spectrogram_tex[surface]) {
        (Some(ring), Some(_)) => {
            ring.capacity == capacity
                && ring.style == style
                && first_key >= ring.oldest_valid
                && first_key <= ring.written_through + 1
        }
        _ => false,
    };

    if !usable {
        // A fresh texture starts black, so a column never written reads as
        // silence rather than as whatever the allocation happened to contain.
        let blank = egui::ColorImage::new([tex_w, h], vec![Color32::BLACK; tex_w * h]);
        match &mut spectrum.spectrogram_tex[surface] {
            Some(handle) => handle.set(blank, opts),
            slot => *slot = Some(ctx.load_texture("spectrogram", blank, opts)),
        }
        spectrum.spectrogram_ring[surface] = Some(SpectrogramRing {
            capacity,
            style,
            // Nothing written yet: the loop below then paints every visible
            // slab rather than trusting a column that was never uploaded.
            written_through: first_key - 1,
            oldest_valid: first_key,
        });
    }

    let (Some(ring), Some(tex)) =
        (&mut spectrum.spectrogram_ring[surface], &mut spectrum.spectrogram_tex[surface])
    else {
        return;
    };

    // Start at the last column written, not past it: that slab was uploaded
    // mid-accumulation and may have gained energy since.
    let start = ring.written_through.max(first_key);
    for key in start..=last_key {
        let i = (key - first_key) as usize;
        let column = fill_column(cfg, bins, &power[i * h..(i + 1) * h]);
        let image = egui::ColorImage::new([1, h], column);
        let x = ring.x_of(key);
        tex.set_partial([x, 0], image.clone(), opts);
        // The twin, `capacity` texels along, is what keeps any run of at most
        // `capacity` slabs contiguous — see [`SpectrogramRing`].
        tex.set_partial([x + capacity, 0], image, opts);
    }

    // Duplicate each edge column just outside the run. The quad reaches a
    // little past the data at both ends — the sub-slab between the newest slab
    // and `now`, and the bit before the oldest slab's center — and a sampler
    // set to ClampToEdge only clamps at the TEXTURE edge, which inside a ring
    // is somewhere else entirely. Without these the slivers would sample a
    // column from a whole window ago. Half a texel is all the quad overruns,
    // so one column each side covers it.
    let last_i = visible - 1;
    for (key, slab) in
        [(first_key - 1, 0usize), (last_key + 1, last_i)]
    {
        let column = fill_column(cfg, bins, &power[slab * h..(slab + 1) * h]);
        let image = egui::ColorImage::new([1, h], column);
        let x = ring.x_of(key);
        tex.set_partial([x, 0], image.clone(), opts);
        tex.set_partial([x + capacity, 0], image, opts);
    }

    ring.written_through = last_key;
    // Anything older than a full lap has been overwritten by the columns above.
    // The far guard sits one before the run, so it is the oldest texel in use.
    ring.oldest_valid = ring.oldest_valid.max(last_key - capacity as i64 + 2);
}

/// One slab's column of the heatmap, bottom (lowest bin) first — the pixels
/// [`fill_pixels`] would put in that column, for a build that writes columns
/// one at a time.
fn fill_column(cfg: &SpectrumConfig, bins: &[Bin], slab: &[BucketDb]) -> Vec<Color32> {
    bins.iter()
        .zip(slab)
        .map(|(bin, &p)| cell_color(cfg.spectrogram_color, bin_level(cfg, p, bin.midi)))
        .collect()
}

/// One cell's 0..1 loudness from its stored byte.
///
/// Deliberately unguarded. A shortcut here — answering anything under -90 dB
/// as flat silence without consulting the mapping — is tempting on the grounds
/// that it "would land at 0 anyway" and that it saves a `log10` for the many
/// empty buckets of a typical spectrum. Neither half holds: the dB window
/// drags down to -120 dB, which makes -90 dB a perfectly visible tenth of the
/// way up the ramp, and the columns are dB already, so there is no `log10` to
/// skip. What such a shortcut actually does is cut the ramp off at -90 dB —
/// the faintest colour dropping straight to black, with the whole quiet end of
/// a wide window missing behind the cliff.
fn bin_level(cfg: &SpectrumConfig, bucket: BucketDb, midi: f32) -> f32 {
    spectrogram_level_db(cfg, db_of(bucket), midi)
}

/// The heatmap image, row-major `pixel(x = slab, y = bin)` at `[y * w + x]`,
/// with `y = 0` the lowest bin. `power` is the flat `w * bins.len()` grid from
/// [`aggregate_rows`]. Opaque throughout — silence is the ramp's dark end, so
/// the plane is filled rather than see-through.
fn fill_pixels(cfg: &SpectrumConfig, w: usize, bins: &[Bin], power: &[BucketDb]) -> Vec<Color32> {
    let h = bins.len();
    let mut pixels = vec![Color32::BLACK; w * h];
    for x in 0..w {
        let base = x * h;
        for (y, bin) in bins.iter().enumerate() {
            let level = bin_level(cfg, power[base + y], bin.midi);
            pixels[y * w + x] = cell_color(cfg.spectrogram_color, level);
        }
    }
    pixels
}

/// A cell's opaque color: `level` (0..1 loudness) mapped through the chosen
/// ramp. The ramp's dark end is black, matching the region's black bed (laid
/// down in `spectral_pane`), so silence recedes while energy stands out — the
/// overall opacity is applied once, as the quad's tint. Shared with the spectrum
/// curve so the two read in the same scheme.
pub(super) fn cell_color(kind: SpectrogramColor, level: f32) -> Color32 {
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
        SpectrogramColor::Magma => ramp(
            t,
            &[[0, 0, 0], [40, 15, 85], [140, 30, 110], [230, 90, 60], [255, 225, 190]],
        ),
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
        let mut power = [0.0f32; SPECTRUM_BINS];
        for &(i, p) in energy {
            power[i] = p;
        }
        crate::SpectrogramColumn::from_power(time, &power)
    }

    /// The stored byte a power lands on — what the aggregation compares, and
    /// so what the aggregation tests below assert against.
    fn q(power: f32) -> BucketDb {
        lattice_core::spectrogram::quantize(power)
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
        assert_eq!(power[0], q(1.0), "the short note's peak survives");
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
        assert_eq!(power[0], q(1.0), "the column before the gap");
        assert_eq!(&power[1..4], [0, 0, 0], "the gap reads as silence, not as a smear");
        assert_eq!(power[4], q(0.5), "the column after it");
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
        assert_eq!(power[1], q(1.0), "and holds the column before it");
        assert_eq!(power[2], q(0.5));
    }

    /// Time running backwards (a transport jump) starts a fresh row rather
    /// than trying to fill a negative gap — which would be an enormous loop,
    /// or a silent no-row.
    #[test]
    fn columns_going_back_in_time_still_get_a_row() {
        let cols = [col(10.0, &[(5, 1.0)]), col(1.0, &[(5, 0.5)])];
        let (centers, power) = aggregate_rows(cols.iter(), &[(5, 6)], 0.25);
        assert_eq!(centers.len(), 2);
        assert_eq!(power[1], q(0.5), "the rewound column landed in its own row");
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
        let w = 2;
        let bins = [
            Bin { idx: 10, end: 11, midi: 40.0, t: 0.1 },
            Bin { idx: 11, end: 12, midi: 41.0, t: 0.2 },
            Bin { idx: 12, end: 13, midi: 42.0, t: 0.3 },
        ];
        let mut power = vec![0; w * bins.len()]; // row-major [slab][bin]
        power[bins.len() + 2] = q(1.0); // slab 1, bin 2 loud
        let px = fill_pixels(&cfg, w, &bins, &power);
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

    /// Storing a bucket as a byte of dB is a memory decision, and it is only
    /// allowed to be one: the colour a cell ends up must be the colour the
    /// power itself would have produced, to within half the grid step it was
    /// put on. Anything wider would be a look change wearing an optimization's
    /// clothes. (The step itself was judged by eye against a sixteen-bit store
    /// and found invisible; this is what keeps it from drifting after.)
    #[test]
    fn quantizing_a_bucket_does_not_move_its_colour() {
        use super::super::spectral::{power_db, spectrogram_level_db};
        let mut cfg = SpectrumConfig::default();
        let tolerance =
            0.5 * lattice_core::spectrogram::DB_STEP / (cfg.ceiling_db - cfg.floor_db) + 1e-6;
        for own_range in [false, true] {
            cfg.spectrogram_own_range = own_range;
            for tilt in [0.0, 3.0, -3.0] {
                cfg.tilt = tilt;
                for midi in [20.0f32, 60.0, 100.0, 130.0] {
                    for power in [1e-8f32, 1e-6, 1e-4, 1e-2, 0.1, 0.5, 1.0, 4.0] {
                        let exact = spectrogram_level_db(&cfg, power_db(power), midi);
                        let stored = bin_level(&cfg, q(power), midi);
                        assert!(
                            (stored - exact).abs() <= tolerance,
                            "power {power} at MIDI {midi} (tilt {tilt}, own range \
                             {own_range}): {exact} exact vs {stored} stored",
                        );
                    }
                }
            }
        }
        // And silence stays exactly silent rather than creeping up off the
        // quantizer's floor, whatever the dB window is set to.
        cfg.spectrogram_own_range = true;
        cfg.spectrogram_floor_db = -120.0;
        assert_eq!(bin_level(&cfg, 0, 60.0), 0.0, "an empty bucket must read as silence");
    }

    /// The quiet end of the ramp must FADE to black, not fall off a cliff into
    /// it. A shortcut answering everything under -90 dB as silence is
    /// invisible while the dB window bottoms out above that, and becomes a
    /// hard edge — faintest colour straight to black — as soon as the window
    /// can be dragged below it. Nothing between two adjacent stored bytes may
    /// move the level by more than the step between them.
    #[test]
    fn the_quiet_end_of_the_ramp_fades_instead_of_cutting_off() {
        let mut cfg = SpectrumConfig {
            spectrogram_own_range: true,
            spectrogram_ceiling_db: 0.0,
            ..SpectrumConfig::default()
        };
        for floor in [-60.0f32, -90.0, -100.0, -120.0] {
            cfg.spectrogram_floor_db = floor;
            // One stored step, as a fraction of the window it is drawn in; the
            // levels either side of any stored byte may differ by that and no
            // more.
            let step = lattice_core::spectrogram::DB_STEP / (cfg.spectrogram_ceiling_db - floor);
            for bucket in 0..BucketDb::MAX {
                let here = bin_level(&cfg, bucket, 60.0);
                let next = bin_level(&cfg, bucket + 1, 60.0);
                assert!(
                    next - here <= step * 1.001 && next >= here,
                    "floor {floor}: byte {bucket} ({here}) -> {next} jumps by {}, \
                     one step is {step}",
                    next - here,
                );
            }
            // And the bottom byte is black at every window, so silence still
            // recedes into the region's bed rather than glowing.
            assert_eq!(bin_level(&cfg, 0, 60.0), 0.0, "floor {floor}: silence must be black");
        }
    }

    #[test]
    fn cells_are_opaque_and_run_dark_to_bright() {
        let quiet = cell_color(SpectrogramColor::Heat, 0.0);
        let loud = cell_color(SpectrogramColor::Heat, 1.0);
        // Opaque throughout (opacity is applied as the quad tint, not here).
        assert_eq!(quiet.a(), 255);
        assert_eq!(loud.a(), 255);
        // Silence is the dark end; loud is brighter.
        let lum = |c: Color32| c.r() as u32 + c.g() as u32 + c.b() as u32;
        assert_eq!(lum(quiet), 0, "silence is the ramp's black end");
        assert!(lum(loud) > lum(quiet));
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
        let bin_idx = [(4usize, 6usize), (6, 10), (10, 11)]; // 3 rows, varied ranges
        let bucket = 0.25;
        let window_span = 1.0;
        // Exercises: cluster (0.30, 0.31), 1-slab gap (0.55->0.80 is 1 apart;
        // 0.80->1.60 is a multi-slab gap), then steady scroll.
        let times: [f64; 14] = [
            0.05, 0.10, 0.30, 0.31, 0.55, 0.80, 1.60, 1.62, 1.90, 2.15, 2.40, 2.65, 2.90, 3.15,
        ];

        let mut agg = SpectrogramAgg::new();
        let mut history = crate::SpectrumHistory::default();
        for (i, &t) in times.iter().enumerate() {
            // Per-column, per-bin energy, so a wrong slab or a stale hold surfaces
            // as a value mismatch, not just a shape one.
            let e = [(4, 0.1 * (i as f32 + 1.0)), (7, 0.05 * i as f32), (10, 1.0 - 0.03 * i as f32)];
            history.push(col(t, &e));
            // Trim the store, so `first` indices shift under the aggregator.
            history.trim_older_than(t - (window_span + 0.5));
            let oldest = t - window_span;
            let first = history.partition_point(|c| c.time < oldest).saturating_sub(1);
            let inc = agg.window(&history, first, bucket, &bin_idx);
            let bat = aggregate_rows(history.iter_from(first), &bin_idx, bucket);
            assert_eq!(inc, bat, "incremental != batch at step {i} (t={t})");
        }

        // A layout change (new bucket) must fall back to a rebuild — still exact.
        let now = *times.last().unwrap();
        let first = history.partition_point(|c| c.time < now - window_span).saturating_sub(1);
        let inc = agg.window(&history, first, 0.4, &bin_idx);
        let bat = aggregate_rows(history.iter_from(first), &bin_idx, 0.4);
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
        // And the color inputs: a palette, dB window or contrast change (cfg)
        // or a gradient-range change (frame) would recolor every pixel.
        let mut cfg2 = cfg;
        cfg2.spectrogram_gamma += 0.1;
        vary(crate::SpectrogramKey::new(100, 0.1, 40.0, 48.0, 3, 200, 5.0, false, cfg2, frame));
        let mut frame2 = frame;
        frame2.brightest_pitch += 1.0;
        vary(crate::SpectrogramKey::new(100, 0.1, 40.0, 48.0, 3, 200, 5.0, false, cfg, frame2));
    }

    /// A column written on its own must be pixel-for-pixel what the
    /// whole-image build would have put in that column. If these two ever
    /// disagree, the live heatmap and an offline render of the same audio stop
    /// looking alike, since which one runs is exactly what tells them apart.
    #[test]
    fn one_column_matches_the_whole_image_build() {
        let cfg = SpectrumConfig::default();
        let bins = [
            Bin { idx: 0, end: 1, midi: 40.0, t: 0.0 },
            Bin { idx: 1, end: 2, midi: 52.0, t: 0.5 },
            Bin { idx: 2, end: 3, midi: 64.0, t: 1.0 },
        ];
        let h = bins.len();
        // Two slabs of three bins, slab-major: [slab][bin].
        let power = [q(1e-3), q(0.0), q(1e-6), q(4e-2), q(1e-5), q(0.0)];
        let w = power.len() / h;

        let whole = fill_pixels(&cfg, w, &bins, &power);
        for slab in 0..w {
            let column = fill_column(&cfg, &bins, &power[slab * h..(slab + 1) * h]);
            for (y, pixel) in column.iter().enumerate() {
                assert_eq!(*pixel, whole[y * w + slab], "slab {slab}, bin {y}");
            }
        }
    }

    /// Columns are placed by absolute slab key, and every one is written twice
    /// so a run never straddles the wrap. Any `capacity` consecutive keys must
    /// therefore land on `capacity` consecutive texels somewhere in the
    /// double-width texture.
    #[test]
    fn a_full_window_is_contiguous_wherever_it_starts() {
        let capacity = 8;
        let ring = SpectrogramRing {
            capacity,
            style: RingStyle {
                rows: 3,
                bucket_bits: 0.1f64.to_bits(),
                scale_min_bits: 40.0f32.to_bits(),
                scale_span_bits: 48.0f32.to_bits(),
                cfg: SpectrumConfig::default(),
                frame: FrameParams::default(),
            },
            written_through: 0,
            oldest_valid: 0,
        };
        // Start the run at every phase of the ring, including ones that wrap.
        for first in -3i64..20 {
            let x0 = ring.x_of(first);
            for step in 0..capacity as i64 {
                let key = first + step;
                // The twin at `+ capacity` is what makes this hold across the
                // wrap; without it `x_of` alone would jump back to 0.
                let placed = ring.x_of(key);
                let contiguous = x0 + step as usize;
                assert!(
                    placed == contiguous || placed + capacity == contiguous,
                    "key {key} at texel {placed} is not {contiguous} (run from {first})",
                );
                assert!(contiguous < capacity * 2, "run ran off the texture");
            }
        }
    }

    /// Negative slab keys happen: the shell clock starts at zero and the
    /// window reaches back before it. A plain `%` would hand back a negative
    /// texel and panic on the upload.
    #[test]
    fn slab_keys_before_zero_still_land_inside_the_texture() {
        let ring = SpectrogramRing {
            capacity: 8,
            style: RingStyle {
                rows: 3,
                bucket_bits: 0.1f64.to_bits(),
                scale_min_bits: 40.0f32.to_bits(),
                scale_span_bits: 48.0f32.to_bits(),
                cfg: SpectrumConfig::default(),
                frame: FrameParams::default(),
            },
            written_through: 0,
            oldest_valid: 0,
        };
        for key in -20i64..0 {
            assert!(ring.x_of(key) < 8, "key {key} fell outside the ring");
        }
    }

    /// The time -> texture mapping has to be a straight line, including across
    /// the slab boundary the newest column sits on. Clamping it to the edge
    /// texel (an early attempt at filling the data-less slivers) pinned it for
    /// part of every slab and let it slide for the rest, and since these are
    /// VERTEX UVs that rescaled the whole image once per slab — visible as the
    /// heatmap jittering. The slivers are filled with guard columns instead.
    #[test]
    fn the_time_to_texture_mapping_is_a_straight_line() {
        let bucket = 0.08;
        let layout = TexLayout {
            t_origin: 100.0,
            tex_span: 8.0 * bucket,
            t0: 0.0,
            tn: 1.0,
            // A run parked mid-ring, as it is for all but one lap in eight.
            x0: 5.0,
            tex_w: 16.0,
        };
        let step = bucket / 4.0;
        let at = |i: i32| u_of(&layout, layout.t_origin + i as f64 * step, bucket);

        // Equal steps in time, equal steps in u — everywhere, including past
        // BOTH ends of the run where the quad reaches for its slivers.
        let expected = at(1) - at(0);
        assert!(expected > 0.0, "u must advance with time");
        for i in -4..40 {
            let moved = at(i + 1) - at(i);
            assert!(
                (moved - expected).abs() < 1e-6,
                "step {i} bent the mapping: expected {expected}, got {moved}",
            );
        }
    }
}
