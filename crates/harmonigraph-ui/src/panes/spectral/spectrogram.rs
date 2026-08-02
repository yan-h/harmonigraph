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
//! [`Axes`](super::axes::Axes), so it turns and flips with the pane, and
//! its dB intensity scale is shared with the spectrum curve via
//! [`loudness`](super::axes::loudness) so "loud" means the same in both.

use egui::Color32;
use harmonigraph_core::spectrogram::{db_of, BucketDb};
use harmonigraph_core::spectrum::{BINS_PER_SEMITONE, SPECTRUM_BINS, SPECTRUM_MIN_MIDI};

use super::axes::{loudness_db, Axes, PitchScale, TimeAxis};
use crate::{SharedState, SpectrogramColor, SpectrumConfig};

/// Most time slabs a live window is ever cut into, whatever the pane's size —
/// and so, with the window, the FINEST slab any given moment can be drawn into.
/// That is what [`SpectrumHistory`](harmonigraph_core::SpectrumHistory) sizes its
/// tiers against: a column of age `a` is only on screen when the window is at
/// least `a` long, so it never needs storing finer than `a / LIVE_SLAB_CAP`.
///
/// Raising this is not free: the store's tiers have to keep up with it (see
/// [`SpectrumHistory::COARSE_COLUMNS`](harmonigraph_core::SpectrumHistory::COARSE_COLUMNS),
/// which must be at least as large), so 512 -> 1024 took the store from 17 to
/// 30 MB. What it buys is the DEFAULT span: at 512 the pane would open on a
/// 32 ms slab where 1024 gives it 16 ms, so the cap and not the data was setting
/// the resolution of the span the pane actually opens on.
///
/// It is a CEILING on the count, not the count itself: [`live_slab`] picks the
/// finest rung of its ladder that fits a window inside this many slabs, so the
/// image holds between half of them and all of them.
pub(crate) const LIVE_SLAB_CAP: f32 = 1024.0;
/// The same for the offline whole-song build, which spans an entire take rather
/// than a scrolling window and so wants more of them.
pub(crate) const WHOLE_SONG_SLAB_CAP: f32 = 4096.0;
/// Columns per slab, at the finest slab the display can ask for: the margin that
/// keeps every slab occupied when the two grids are independent (the analyzer
/// counts samples, the slabs divide a window). At 1.0 they would beat against
/// each other and leave slabs empty.
pub(crate) const COLUMNS_PER_SLAB: f64 = 1.6;
/// Never subdivide finer than the data arrives. A shorter bucket leaves empty
/// buckets between columns, and the texture's linear time axis assumes
/// evenly-spaced slabs — gaps there stretch the edge columns into flat streaks.
/// Derived from the FFT rate rather than restated, because the two must move
/// together and a stale copy of this number is exactly the bug that shows up as
/// duplicated columns scrolling past.
///
/// The WHOLE-SONG build's floor. The live grid gets the same guarantee from
/// [`live_slab`]'s ladder, whose lowest rung is two analysis intervals, so this
/// no longer floors it.
pub(crate) const MIN_BUCKET: f64 = crate::AudioSpectrum::FFT_INTERVAL * COLUMNS_PER_SLAB;

/// Device pixels the pane's size is rounded UP to before anything is derived
/// from it.
///
/// The same argument as [`live_slab`]'s ladder, for the other axis. A pane's
/// height decides how many rows the image has, and its width decides how many
/// slabs and how wide the ring is — so taken to the pixel, every pixel of a
/// resize drag is a different image AND a different texture, and the drag pays
/// a full re-fold and a full repaint on every frame of itself. Measured on the
/// overlay's fallback row, a resize sat at the frame rate: one of each, per
/// frame, for as long as the drag lasted.
///
/// Rounded UP, so the image is never coarser than the pane it is stretched
/// over — the quantum costs a little oversampling and no detail, which is the
/// same trade [`PaneView`] makes by sizing in pixels rather than points. Sixty
/// four turns a 600-pixel drag into ten re-layouts instead of six hundred, and
/// leaves the image at most 9% taller than a 700-pixel pane needs.
const PANE_QUANTUM: f32 = 64.0;

/// A pane measurement in device pixels, rounded up to [`PANE_QUANTUM`].
fn quantized(pixels: f32) -> f32 {
    (pixels / PANE_QUANTUM).ceil() * PANE_QUANTUM
}

/// The live grid's finest rung, in analysis intervals — see [`live_slab`].
///
/// TWO, not one: the column grid and the slab grid share a period on the ladder
/// but not a phase (columns land on a sample counter, slabs on absolute time),
/// so at one column per slab a boundary falling mid-interval leaves some slabs
/// empty, and the texture's uniform time axis then stretches the columns either
/// side of an empty slab into a flat streak. At two, a phase offset costs a slab
/// one of its columns and never both. It is [`COLUMNS_PER_SLAB`]'s job, done by
/// the ladder instead of by a margin.
const LADDER_FLOOR_COLUMNS: f64 = 2.0;

/// The slab width a LIVE window is cut into: the analysis interval, doubled
/// until the window fits in `target_cols` slabs.
///
/// A ladder rather than `window / target_cols`, and specifically the ladder
/// [`SpectrumHistory`](harmonigraph_core::SpectrumHistory) merges its columns on —
/// every rung is a power of two analysis intervals. Two things follow.
///
/// **The grid holds still.** A slab width taken straight from the window moves
/// on every frame of a Span drag, and a moved slab width re-lays the
/// aggregator's grid and re-blanks the ring's texture — the entire per-frame
/// rebuild, for as long as the drag lasts. On the ladder it moves only when the
/// Span crosses a doubling.
///
/// **The store is structurally fine enough to fill it.** A column of age `a`
/// sits in a tier spaced at most `a / LIVE_SLAB_CAP`, and a window that reaches
/// that column is at least `a` long, so its slabs are at least that wide. The
/// two were already the same relation — that is why
/// [`COARSE_COLUMNS`](harmonigraph_core::SpectrumHistory::COARSE_COLUMNS) has to be
/// at least [`LIVE_SLAB_CAP`] — but as an inequality between constants chosen
/// apart, kept true by a test. On one shared ladder they round to the same rung.
///
/// The cost is resolution that steps rather than tracks: within a rung a wider
/// pane buys nothing, so the image can hold half the slabs the depth axis has
/// pixels. That is well under what the analysis resolves — at the default Span a
/// slab is 16 ms against a 171 ms analysis window — so what it gives up is
/// detail the FFT never had.
pub(crate) fn live_slab(window: f64, target_cols: usize) -> f64 {
    let mut bucket = crate::AudioSpectrum::FFT_INTERVAL * LADDER_FLOOR_COLUMNS;
    // The Span reaches ten minutes and the pane can be a sliver, so walk the
    // ladder under a bound rather than trusting the ratio to be sane.
    for _ in 0..64 {
        if window / bucket <= target_cols as f64 {
            break;
        }
        bucket *= 2.0;
    }
    bucket
}

/// A run of empty slabs this short is a seam in the sample stream rather than a
/// stall in the analyzer, and holds the previous column instead of reading as
/// silence.
///
/// It used to absorb frame jitter, which was the common case: the FFT fired on
/// frame boundaries, so one long frame left a slab with nothing in it. Columns
/// now land on a sample grid [`COLUMNS_PER_SLAB`] finer than the narrowest slab
/// (see `AudioSpectrum::push_samples`), so an ordinary stream cannot skip one at
/// all. What is left for this to cover is a real gap in the samples — a host
/// dropout, the pane being switched on, a transport jump re-anchoring the grid.
/// A stall, by contrast — switching the FFT window empties the ring for a
/// window's worth of samples, 341 ms at 48 kHz on Precise — is many slabs wide
/// and genuinely was silence as far as the analyzer is concerned.
const JITTER_SLABS: i64 = 1;

/// Draw the spectrogram across the roll's depth region (`split..1`), sharing
/// the roll's `depth_of` time mapping so its columns register with the notes.
///
/// Builds the heatmap into a `[time slab x pitch bin]` image, (re)uploads it
/// to the surface's texture, then stretches it over the region as a
/// single bilinear-filtered quad — smooth in both axes, and opaque (silence is
/// the ramp's dark end, not transparent) so the plane is a filled image rather
/// than bright patches floating on the background.
/// One row of the heatmap image: how it reads the source buckets, its center
/// MIDI pitch, and that pitch's fraction `t` up the pitch axis.
///
/// A row is a PIXEL of the pitch axis, not a bucket. Zoomed out, a row takes the
/// max over the buckets that fall in it: the axis holds thousands of buckets and
/// the pane a few hundred pixels, so one row per bucket would build an image far
/// taller than the screen (and, at 32 buckets per semitone, taller than the GPU
/// will allocate) only for the sampler to throw the detail away. Zoomed in,
/// several rows share one bucket — that is the resolution the analyzer has — and
/// they read it INTERPOLATED rather than repeated. See [`RowRead`].
struct Bin {
    read: RowRead,
    midi: f32,
    t: f32,
}

/// How one row reads the buckets under it.
///
/// The same choice, for the same reason, that
/// [`pitch_spectrum`](harmonigraph_core::spectrum::SpectrumAnalyzer::pitch_spectrum)
/// makes one level down between FFT bins: MAX where the row is WIDER than what
/// it reads (a peak must not be averaged away by its quiet neighbours), and a
/// lerp where it is narrower (the grid is being asked for more than it holds, so
/// read between its points instead of repeating them).
///
/// Repeating was the old behaviour, and it is visible: at a three-semitone zoom
/// a bucket is seven rows tall, and bilinear filtering cannot smooth a run of
/// identical texels, so the pitch axis came out as plateaus with a step between
/// them rather than as a ridge.
#[derive(Clone, Copy, PartialEq)]
enum RowRead {
    /// The loudest of `from..to` (always at least one bucket wide).
    Max { from: usize, to: usize },
    /// Between `lo` and the bucket above it, `f` of the way up.
    Lerp { lo: usize, f: f32 },
}

impl RowRead {
    /// This row's value from one stored column, in the dB the column holds —
    /// which is also the domain the ramp reads, so interpolating in it is
    /// interpolating exactly what will be drawn.
    fn of(self, db: &harmonigraph_core::spectrogram::ColumnDb) -> BucketDb {
        match self {
            RowRead::Max { from, to } => db[from..to].iter().copied().max().unwrap_or(0),
            RowRead::Lerp { lo, f } => {
                let (a, b) = (db[lo], db[(lo + 1).min(SPECTRUM_BINS - 1)]);
                (f32::from(a) + (f32::from(b) - f32::from(a)) * f).round() as BucketDb
            }
        }
    }
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
    /// Seconds one slab spans — one texel of the time axis. Carried here
    /// because every mapping below reads it alongside the rest: a texel's width
    /// is part of describing where the slabs sit, not a separate fact about
    /// them.
    pub(crate) bucket: f64,
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

/// Why a ring could not be carried forward.
///
/// Worth telling apart, and reported separately by the performance overlay,
/// because each says something different about what to go and look at: a style
/// is the image changing, a capacity is the PANE changing, and a gap is the
/// window having moved somewhere the texture cannot reach from. Twice now the
/// question "which of these is it?" has been the whole diagnosis, answered by
/// instrumenting rather than by reasoning about it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Restart {
    /// The image's shape: its rows (the pane's pitch side), or the width of a
    /// slab (its time side, through the Span).
    Rows,
    Slab,
    /// The pitch range the rows read, or the dB window, tilt and ramp that
    /// colour them.
    Pitch,
    Colour,
    /// The ring's own size, which is the pane's time side.
    Pane,
    /// The run does not connect to what is painted — the window jumped, or
    /// history was cleared.
    Gap,
}

impl Restart {
    /// Where each is counted, and what the overlay calls it. Named to the FIELD
    /// rather than to the layer, because "the style changed" only narrows it to
    /// six things and the whole value of the readout is not having to narrow it
    /// by argument.
    pub(crate) const LABELS: [&'static str; Self::COUNT] =
        ["rows", "slab", "pitch", "colour", "pane", "gap"];
    pub(crate) const COUNT: usize = 6;

    fn slot(self) -> usize {
        match self {
            Restart::Rows => 0,
            Restart::Slab => 1,
            Restart::Pitch => 2,
            Restart::Colour => 3,
            Restart::Pane => 4,
            Restart::Gap => 5,
        }
    }
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
    style: ColumnStyle,
    /// Newest slab key written, and the oldest still valid.
    written_through: i64,
    oldest_valid: i64,
}

/// Everything that decides a column's pixels: how tall the image is, how wide a
/// slab is, which buckets a row reads, and how a bucket is coloured. A change to
/// any of it invalidates every column at once.
///
/// It is the part of [`crate::SpectrogramKey`] that outlives a SCROLL —
/// the key is this plus which columns are in the window — and the ring compares
/// exactly this to decide whether it can carry its texture forward. One
/// definition for both, because they are one question asked at two
/// granularities, and as two hand-kept lists in two files they were one list to
/// forget: a pixel-affecting input added to the key alone leaves the ring
/// carrying forward columns painted under the old setting, which is a WRONG
/// picture rather than a slow one. `the_key_is_sensitive_to_every_input` covers
/// both because there is only one to cover.
///
/// The converse costs frames rather than correctness, and so is the one that
/// went unnoticed: an input that decides NOTHING about a column must stay out,
/// or every change to it throws away a texture that was still good. See
/// [`ColumnColor`] for what that cost looked like.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ColumnStyle {
    rows: usize,
    bucket_bits: u64,
    scale_min_bits: u32,
    scale_span_bits: u32,
    color: ColumnColor,
}

/// How a bucket becomes a colour: the dB window it is read against and the ramp
/// the result lands on. Exactly the fields [`fill_column`] reaches —
/// `cell_color`'s ramp, and everything [`loudness_db`] reads — and nothing else.
///
/// It is spelled out field by field rather than holding a whole
/// [`SpectrumConfig`] because the config is also where the pane keeps what it
/// is LOOKING at, and that moves continuously: `roll_seconds` on every frame of
/// a Span drag, `roll_fraction` on every frame of a divider drag — neither of
/// which changes a texel. Keying the ring on the whole config made every one of
/// those a full re-blank and repaint, which is precisely the per-frame rebuild
/// [`live_slab`]'s ladder was built to end; the ladder held `bucket` still and
/// the config moved anyway. `dragging_the_span_carries_the_ring_forward` is
/// that drag.
///
/// The cost of listing fields is that a new colour input has to be added here
/// too, and forgetting leaves a WRONG picture rather than a slow one — so
/// `the_key_is_sensitive_to_every_input` walks every field in both directions.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ColumnColor {
    ramp: SpectrogramColor,
    // Bit patterns, for the same exactness the outer struct's floats get: the
    // Level window the heatmap shares with the curve, and the tilt applied
    // inside it.
    floor_bits: u32,
    ceiling_bits: u32,
    tilt_bits: u32,
}

impl ColumnColor {
    fn new(cfg: &SpectrumConfig) -> ColumnColor {
        ColumnColor {
            ramp: cfg.spectrogram_color,
            floor_bits: cfg.floor_db.to_bits(),
            ceiling_bits: cfg.ceiling_db.to_bits(),
            tilt_bits: cfg.tilt.to_bits(),
        }
    }
}

impl ColumnStyle {
    /// Which field of this differs from `other`, or `None` if none does — the
    /// first found, since one is enough to say where to look.
    fn differs(&self, other: &ColumnStyle) -> Option<Restart> {
        if self.rows != other.rows {
            Some(Restart::Rows)
        } else if self.bucket_bits != other.bucket_bits {
            Some(Restart::Slab)
        } else if self.scale_min_bits != other.scale_min_bits
            || self.scale_span_bits != other.scale_span_bits
        {
            Some(Restart::Pitch)
        } else if self.color != other.color {
            Some(Restart::Colour)
        } else {
            None
        }
    }

    pub(crate) fn new(
        rows: usize,
        bucket: f64,
        scale_min: f32,
        scale_span: f32,
        cfg: &SpectrumConfig,
    ) -> ColumnStyle {
        // Floats as bit patterns, so equality is exact and free of NaN quirks.
        ColumnStyle {
            rows,
            bucket_bits: bucket.to_bits(),
            scale_min_bits: scale_min.to_bits(),
            scale_span_bits: scale_span.to_bits(),
            color: ColumnColor::new(cfg),
        }
    }
}

impl SpectrogramRing {
    /// Texel x of a slab key. The `+ capacity` twin is written by the caller.
    fn x_of(&self, key: i64) -> usize {
        key.rem_euclid(self.capacity as i64) as usize
    }

    /// Whether this ring can be carried forward for the run
    /// `first_key..=last_key`: it has to describe THIS texture, in this style,
    /// and the run has to connect to what is already painted.
    ///
    /// The window grows at the near end as columns arrive and at the FAR end
    /// when the Span is zoomed out, and both are served by painting what is
    /// missing rather than starting over — the slabs a widening window reveals
    /// are in the view already, since the aggregator keeps what this ring can
    /// hold. Without that, a restart resets the painted range to the window's
    /// own, the next frame of the gesture reaches past it again, and the drag
    /// restarts the texture on every frame of itself.
    ///
    /// Answering NO is always safe and never cheap — it re-blanks the texture
    /// and repaints every column — so this is the predicate the ring's whole
    /// value rests on, and `no_cache_layer_falls_back_as_the_window_scrolls`
    /// holds it to yes for every Span the pane offers.
    fn carries(
        &self,
        capacity: usize,
        style: &ColumnStyle,
        first_key: i64,
        last_key: i64,
    ) -> Option<Restart> {
        if let Some(field) = self.style.differs(style) {
            // The image itself changed, so every column is wrong at once.
            return Some(field);
        }
        if self.capacity != capacity {
            // A different capacity is a different texture: the slab a texel
            // stands for is `key mod capacity`, so the whole mapping moves.
            return Some(Restart::Pane);
        }
        // The run has to CONNECT to what is painted at both ends. A run starting
        // past `written_through + 1`, or ending before `oldest_valid - 1`,
        // leaves never-written texels inside itself, and painting at the edges
        // cannot reach them. It also has to fit in a lap with its far guard, or
        // painting the far end would overwrite the near one.
        let connects = first_key <= self.written_through + 1
            && last_key >= self.oldest_valid - 1
            && last_key - first_key + 2 <= capacity as i64;
        (!connects).then_some(Restart::Gap)
    }

    /// A ring with nothing written yet, for a run starting at `first_key` — so
    /// its caller paints every visible slab rather than trusting a column that
    /// was never uploaded.
    fn restarted(capacity: usize, style: ColumnStyle, first_key: i64) -> SpectrogramRing {
        SpectrogramRing { capacity, style, written_through: first_key - 1, oldest_valid: first_key }
    }

    /// Record the run `first_key..=last_key` as painted, widening what this ring
    /// holds at whichever end it reached past.
    ///
    /// Anything older than a full lap has been overwritten by it; the far guard
    /// sits one before the run, so it is the oldest texel in use.
    ///
    /// The floor is `first_key` and NOT the oldest run ever painted, because
    /// the guard destroys exactly that slack. [`write_ring`] duplicates the
    /// run's oldest column into the texel of `first_key - 1`, so that key stops
    /// being its own, and a window that scrolls a slab at a time walks the
    /// guard forward one key per frame — remembering a band below `first_key`
    /// would be remembering the keys it has just walked over.
    ///
    /// A widen still costs only the slabs it reveals: `back` runs from the new
    /// `first_key` up to this floor, one past the key the previous frame's
    /// guard overwrote.
    ///
    /// It is a floor and not the exact truth. When `first_key` jumps several
    /// keys at once — a dropped frame, a fast narrowing — only the last of
    /// them was guarded, and the ones before it are forgotten though they were
    /// still their own. That costs a repaint of a few columns and never a wrong
    /// pixel, and two endpoints cannot say otherwise: one guard punches a hole
    /// in the middle of the painted range, so the valid set stops being an
    /// interval and this is the tightest interval inside it.
    fn wrote(&mut self, first_key: i64, last_key: i64) {
        self.written_through = self.written_through.max(last_key);
        self.oldest_valid = first_key.max(last_key - self.capacity as i64 + 2);
    }
}

/// The pane's geometry and settings for one frame — half of a [`Plan`]'s
/// inputs, the half that has nothing to do with the store.
///
/// Sized in PIXELS, not points: [`Axes`] is laid out in egui points and this
/// image is stretched over that rect by the GPU, so sizing it in points builds
/// it at the display's density divided by the scale factor and then upsamples —
/// half the resolution in each axis on a 2x screen, for a heatmap softer than
/// the pane it sits in. The label glyphs oversample by the same factor for the
/// same reason (see `text::draw_glyphs`).
struct PaneView {
    /// Physical pixels per egui point.
    ppp: f32,
    /// The tallest image the GPU will take.
    max_rows: usize,
    /// Points across the pitch axis, and across the heatmap's SHARE of the
    /// depth axis (the roll owns the rest).
    pitch_len: f32,
    depth_len: f32,
    /// Seconds the depth axis spans.
    window: f64,
    scale: PitchScale,
    cfg: SpectrumConfig,
    /// The whole-song (offline playhead) layout rather than the live window.
    whole: bool,
}

/// Which stored columns a frame draws — the other half of a [`Plan`]'s inputs.
struct Columns {
    /// The oldest in-window column; it advances as the window scrolls one off
    /// the far end. Whole-song draws its entire fixed set, so 0.
    first: usize,
    len: usize,
    /// The newest column's time, which moves whenever a fresh column arrives —
    /// catching one even in a saturated store, where the count holds steady.
    newest: f64,
}

/// What this frame's heatmap needs built: the image's shape, and the key that
/// says whether the uploaded one already IS it.
///
/// Pure, and deliberately so. Every cliff this pipeline has fallen off has been
/// in this arithmetic — a slab against the analyzer's lag, a window against the
/// store's finest tier — and deciding it apart from the build means it can be
/// checked without a GPU, a texture or a frame. See
/// `no_cache_layer_falls_back_as_the_window_scrolls`.
struct Plan {
    /// One row per pixel of the pitch axis, never taller than the GPU takes.
    rows: usize,
    /// A time slab's width, in seconds.
    bucket: f64,
    /// Slabs the ring holds, and so the most the aggregator keeps folded.
    ///
    /// Sized off the PANE rather than off the window, which is what makes it
    /// hold still: the widest run a window can have at this slab width is
    /// `target_cols` of them, whatever the Span is doing inside that rung, so a
    /// drag never resizes the texture. Unused by the whole-song build, which
    /// owns its texture outright.
    capacity: usize,
    first: usize,
    key: crate::SpectrogramKey,
}

impl Plan {
    fn new(view: &PaneView, columns: &Columns) -> Plan {
        // The offline build's pane cannot resize mid-render, so it has nothing
        // to hold still and takes its size to the pixel; the live one rounds up
        // to a quantum so a resize drag re-lays the grid a handful of times
        // instead of once a frame. See [`PANE_QUANTUM`].
        let pitch_px = view.pitch_len * view.ppp;
        let depth_px = view.depth_len * view.ppp;
        let (pitch_px, depth_px) = if view.whole {
            (pitch_px.round(), depth_px.round())
        } else {
            (quantized(pitch_px), quantized(depth_px))
        };
        let rows = (pitch_px as usize).clamp(2, view.max_rows);
        // One image column per output depth pixel; whole-song spans an entire
        // take, so it needs a higher cap than the live window.
        let col_cap = if view.whole { WHOLE_SONG_SLAB_CAP } else { LIVE_SLAB_CAP };
        let target_cols = depth_px.clamp(2.0, col_cap) as usize;
        let bucket = if view.whole {
            // The offline build draws its own fixed column set rather than the
            // live store's, so it shares no ladder with it and has nothing to
            // hold still — its grid is laid out once and cached for the render.
            (view.window / target_cols as f64).max(MIN_BUCKET)
        } else {
            live_slab(view.window, target_cols)
        };
        // The heatmap's pixels are a pure function of these; if none has moved
        // since the uploaded texture was built, building it again is dead work.
        let style =
            ColumnStyle::new(rows, bucket, view.scale.min_midi, view.scale.span, &view.cfg);
        let key = crate::SpectrogramKey::new(
            style,
            columns.first,
            columns.len,
            columns.newest,
            view.whole,
        );
        Plan { rows, bucket, capacity: target_cols + RING_HEADROOM, first: columns.first, key }
    }
}

/// The image's rows for a plan of `rows` over `scale`.
///
/// A row's pitch span maps back to the source buckets under it, read by MAX or
/// by interpolation depending on which of the two is finer — see [`RowRead`]. A
/// bucket of slack on each side lets the filtering carry the visible range
/// cleanly to its edges.
fn bins_for(rows: usize, scale: &PitchScale) -> Vec<Bin> {
    let bin_semis = 1.0 / BINS_PER_SEMITONE as f32;
    let margin = (bin_semis / scale.span).min(0.5);
    let bucket_of = |t: f32| {
        let midi = scale.min_midi + t * scale.span;
        (((midi - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32).floor() as isize)
            .clamp(0, SPECTRUM_BINS as isize - 1) as usize
    };
    (0..rows)
        .map(|r| {
            // The row's own slice of the visible pitch range, widened by the
            // margin so the edge rows reach past the range like the buckets did.
            let span = 1.0 + 2.0 * margin;
            let t0 = -margin + span * r as f32 / rows as f32;
            let t1 = -margin + span * (r + 1) as f32 / rows as f32;
            let (idx, last) = (bucket_of(t0), bucket_of(t1));
            let t = 0.5 * (t0 + t1);
            let midi = scale.min_midi + t * scale.span;
            let read = if last > idx {
                RowRead::Max { from: idx, to: (last + 1).min(SPECTRUM_BINS) }
            } else {
                // Narrower than a bucket: read between the two whose centers
                // straddle this row's center. A bucket's center sits half a
                // bucket above where `bucket_of` divides them, which is the 0.5.
                let x = (midi - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32 - 0.5;
                let lo = (x.floor() as isize).clamp(0, SPECTRUM_BINS as isize - 2) as usize;
                RowRead::Lerp { lo, f: (x - lo as f32).clamp(0.0, 1.0) }
            };
            Bin { read, midi, t }
        })
        .collect()
}

/// Build the image a [`Plan`] describes into the surface's texture, and record
/// the build in its cache. Returns where the visible slabs sit in the texture,
/// or `None` if the plan came out too small to draw.
///
/// The only step here that touches the GPU — everything it needs decided was
/// decided in [`Plan::new`].
fn build(
    ctx: &egui::Context,
    spectrum: &mut crate::AudioSpectrum,
    whole: Option<&crate::WholeSong>,
    surface: usize,
    plan: &Plan,
    view: &PaneView,
    bins: &[Bin],
) -> Option<TexLayout> {
    let (bucket, cfg) = (plan.bucket, view.cfg);
    // Aggregate the in-window columns into one image column per depth pixel by
    // a FIXED time grid, MAX within each slab (which keeps a short note's peak
    // and pins it against the scroll — see `aggregate_rows`).
    let reads: Vec<RowRead> = bins.iter().map(|b| b.read).collect();
    let (centers, power) = match whole {
        // Offline whole-song: a fixed column set, already cached after the first
        // frame — a plain batch aggregate.
        Some(ws) => aggregate_rows(ws.columns.iter(), &reads, bucket),
        // Live: fold only the new column(s) into the kept slab grid instead of
        // rescanning the whole window every rebuild. `history` and the
        // aggregator are disjoint fields of `spectrum`.
        None => {
            let hist = &spectrum.history;
            let agg = spectrum.spectrogram[surface].agg.get_or_insert_with(SpectrogramAgg::new);
            agg.window(hist, plan.first, bucket, &reads, plan.capacity)
        }
    };
    let (w, h) = (centers.len(), bins.len());
    if w < 2 {
        return None;
    }
    // The image covers absolute time `[t_origin, t_origin + w*bucket]` — the
    // oldest slab's start to the newest slab's end. Its texel centers sit at the
    // slab centers, so `u = (t - t_origin) / span` places time exactly.
    let t_origin = centers[0] - 0.5 * bucket;
    let tex_span = w as f64 * bucket;
    let (t0, tn) = (bins[0].t, bins[h - 1].t);
    if tex_span < 1e-9 || (tn - t0).abs() < 1e-6 {
        return None;
    }
    let first_key = (centers[0] / bucket).floor() as i64;

    // The offline whole-song build keeps the full-width path: its column set is
    // fixed and already cached after the first frame, so there is nothing for a
    // ring to save.
    let layout = if whole.is_none() {
        // The key's own style, not a second copy of it: the ring is asking the
        // same question about the same columns.
        let style = plan.key.style().clone();
        let capacity = ring_capacity(plan.capacity, w);
        write_ring(ctx, spectrum, surface, style, capacity, &cfg, bins, &power, first_key, w);
        let ring = spectrum.spectrogram[surface].ring.as_ref();
        let x0 = ring.map_or(0.0, |r| r.x_of(first_key) as f32);
        let tex_w = ring.map_or(w as f32, |r| (r.capacity * 2) as f32);
        TexLayout { bucket, t_origin, tex_span, t0, tn, x0, tex_w }
    } else {
        // The full-width build owns the whole texture, so any ring bookkeeping
        // describing it is now a lie about which slabs its columns hold.
        spectrum.spectrogram[surface].ring = None;
        // Build and upload the image (pixel (x = slab, y = bin), y = 0 low pitch).
        let pixels = fill_pixels(&cfg, w, bins, &power);
        let image = egui::ColorImage::new([w, h], pixels);
        let opts = egui::TextureOptions::LINEAR; // bilinear + ClampToEdge
        match &mut spectrum.spectrogram[surface].tex {
            Some(handle) => handle.set(image, opts),
            slot => *slot = Some(ctx.load_texture("spectrogram", image, opts)),
        }
        TexLayout { bucket, t_origin, tex_span, t0, tn, x0: 0.0, tex_w: w as f32 }
    };
    spectrum.spectrogram[surface].cache =
        Some(crate::SpectrogramCache::new(plan.key.clone(), layout));
    Some(layout)
}

/// The scrolling quads over the uploaded texture, from `d_near` to `d_far`.
///
/// Pure geometry, which is what lets the UV rule below be checked at all: these
/// are VERTEX UVs, so the scale every fragment samples at is interpolated
/// between a quad's corners.
///
/// Map a screen depth to the texture's time axis CONTINUOUSLY, through the
/// roll's own `now`-anchored depth<->time relation (unclamped, the inverse of
/// `depth_of`), so `u` slides with `now` frame to frame exactly as a note ribbon
/// at the same depth does. Pinning it to the slab endpoints instead jumps a
/// whole slab at a time, which is both the image losing the notes it is meant to
/// register with and a per-slab stutter. `v` maps pitch to the bin rows.
///
/// Slabs occupy texels `x0 .. x0 + visible` of a `tex_w`-wide texture; for a
/// full-width build that is the whole thing and this collapses to the plain
/// `(t - t_origin) / tex_span`.
fn heatmap_mesh(
    tex: egui::TextureId,
    axes: &Axes,
    time: &TimeAxis,
    layout: &TexLayout,
    d_near: f32,
    d_far: f32,
) -> egui::Mesh {
    // Straight in time out to the newest column the texture holds, then HELD
    // there — see [`u_drawn`], and the mesh is split at that corner so no
    // fragment ever interpolates across it.
    let u_at = |d: f32| u_drawn(layout, time.time_at(d));
    let v_at = |p: f32| (p - layout.t0) / (layout.tn - layout.t0);
    // Untinted: the texels carry the whole of the colour.
    //
    // A tint here is how an Opacity setting fades the heatmap so it can sit
    // under the notes, and what that costs is the SHARED SCHEME. The spectrum
    // curve is drawn from the same [`cell_color`] ramp against the same
    // `loudness_db`, and it takes no tint — so a faded heatmap means equal
    // levels stop looking equal across the two halves of one pane, which is the
    // whole reason they share a mapping. A heatmap worth less than solid is one
    // to turn off.
    //
    // What a tint does NOT cost is the ramp's dark end: `Color32` is
    // premultiplied, so a black texel over the black bed composites to black at
    // any alpha (`spectral_pane` lays that bed and leans on the same fact).
    let tint = Color32::WHITE;
    let vert = |p: f32, d: f32| egui::epaint::Vertex {
        pos: axes.at(p, d),
        uv: egui::pos2(u_at(d), v_at(p)),
        color: tint,
    };

    // Quads over pitch [0,1] x a depth span; the GPU bilinear-samples them.
    let mut mesh = egui::Mesh::with_texture(tex);
    let quad = |mesh: &mut egui::Mesh, near: f32, far: f32| {
        let i = mesh.vertices.len() as u32;
        mesh.vertices.push(vert(0.0, far)); // far, low
        mesh.vertices.push(vert(1.0, far)); // far, high
        mesh.vertices.push(vert(1.0, near)); // near, high
        mesh.vertices.push(vert(0.0, near)); // near, low
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    };

    // Split at the corner in `u_drawn`: one quad whose UVs are straight in time
    // (the data) and one whose UVs are CONSTANT (the sliver past the newest
    // column — leading for the live window, trailing for the whole-song build,
    // which is the only reason `d_hold` is clamped into the pair rather than
    // assumed to sit inside it).
    //
    // Letting one quad span the corner instead is what the vertex-UV rule
    // forbids: a vertex sitting mid-bend rescales the whole image, and the bend
    // crosses each slab, so it would rescale it once per slab — the jitter.
    // Split here, the data quad's two ends are both straight in time, so it
    // holds exactly one texel per slab forever; all that changes over a slab is
    // how long the flat sliver is, and its colour matches the data at the seam
    // (both sample the newest texel's centre), so the join is invisible.
    let d_hold = time.depth_of(hold_time(layout)).max(d_near).min(d_far);
    if d_hold > d_near {
        quad(&mut mesh, d_near, d_hold);
    }
    quad(&mut mesh, d_hold, d_far);
    mesh
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
    // A small copy, so `state.spectrum` is then free to take mutably (its
    // texture handle) without fighting the config reads.
    let cfg = state.spectrum_config;
    // Shared time<->depth mapping: a `now`-anchored scrolling window live, or
    // the whole take laid out statically (offline playhead mode).
    let time = TimeAxis::new(state, split, now);
    let whole = state.whole_song.as_ref();
    let spectrum = &mut state.spectrum;
    // Columns come from the precomputed whole-take set (playhead mode) or the
    // live store.
    let enough = match whole {
        Some(ws) => ws.columns.len() >= 2,
        None => spectrum.history().len() >= 2,
    };
    if !enough {
        return;
    }

    let view = PaneView {
        ppp: painter.ctx().pixels_per_point().max(1.0),
        max_rows: painter.ctx().input(|i| i.max_texture_side).max(64),
        pitch_len: axes.pitch_len(),
        depth_len: (1.0 - split) * axes.depth_len(),
        window: time.window(),
        scale: *scale,
        cfg,
        whole: whole.is_some(),
    };
    let columns = match whole {
        Some(ws) => Columns {
            first: 0,
            len: ws.columns.len(),
            newest: ws.columns.last().map_or(now, |c| c.time),
        },
        None => {
            let hist = spectrum.history();
            Columns {
                first: hist.partition_point(|c| c.time < time.oldest()).saturating_sub(1),
                len: hist.len(),
                newest: hist.back().map_or(now, |c| c.time),
            }
        }
    };
    let plan = Plan::new(&view, &columns);

    // Fast path: the built image is still valid — reuse the uploaded texture and
    // its geometry; only the scrolling quad below is recomputed (with `now`).
    let surf = &spectrum.spectrogram[surface];
    let reused = match &surf.cache {
        Some(c) if c.matches(&plan.key) && surf.tex.is_some() => Some(c.geometry()),
        _ => None,
    };
    let layout = match reused {
        Some(layout) => layout,
        None => {
            let bins = bins_for(plan.rows, scale);
            match build(painter.ctx(), spectrum, whole, surface, &plan, &view, &bins) {
                Some(layout) => layout,
                None => return,
            }
        }
    };

    let Some(tex) = &spectrum.spectrogram[surface].tex else { return };

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
        let near =
            if now - columns.newest <= stale_after { split } else { time.depth_of(columns.newest) };
        (near, time.depth_of(layout.t_origin))
    };

    let mesh = heatmap_mesh(tex.id(), axes, &time, &layout, d_near, d_far);
    painter.add(egui::Shape::mesh(mesh));
}

/// Group `columns` (oldest first) into time-slabs of `bucket` seconds, taking
/// the MAX over each slab AND over each output row's run of source buckets
/// (`reads` gives how a row draws from them). Returns each slab's
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
    reads: &[RowRead],
    bucket: f64,
) -> (Vec<f64>, Vec<BucketDb>) {
    let mut grid = SlabGrid::default();
    for col in columns {
        grid.fold(col, reads, bucket);
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
    /// `held[i]` marks slab `i` as a COPY of slab `i - 1` — an empty slab the
    /// jitter hold filled — rather than a slab of its own columns. It is what
    /// lets [`SpectrogramAgg::view`] carry its pruning of the window's first
    /// slab into the copies behind it; without it a held slab keeps energy from
    /// columns that have since left the window. A held slab is never MAXed
    /// afterwards (columns arrive in time order, so it is already behind the
    /// front when it is created), so the mark stays true for its whole life.
    held: Vec<bool>,
    cur_key: Option<i64>,
}

impl SlabGrid {
    /// Fold one column (columns arrive oldest-first) into the grid, appending
    /// slabs and MAXing the column into the current one. Returns `false` iff
    /// the column ran BACKWARDS in time relative to the current slab — batch
    /// ignores the result (it just starts a fresh row, as before), while the
    /// incremental aggregator treats it as a broken invariant and rebuilds.
    fn fold(&mut self, col: &crate::SpectrogramColumn, reads: &[RowRead], bucket: f64) -> bool {
        let nb = reads.len();
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
                    self.held.push(empty <= JITTER_SLABS);
                }
                self.centers.push((key as f64 + 0.5) * bucket);
                self.power.resize(self.power.len() + nb, 0);
                self.held.push(false);
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
                self.held.push(false);
                other.is_none()
            }
        };
        let base = self.power.len() - nb;
        for (k, read) in reads.iter().enumerate() {
            let v = read.of(&col.db);
            if v > self.power[base + k] {
                self.power[base + k] = v;
            }
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
/// It reproduces `aggregate_rows` over the columns AS THEY ARRIVED: the shared
/// [`SlabGrid::fold`] gives identical slab values, and the window is served from
/// the same first slab batch would start at. A layout change (bucket/bins), a
/// backward transport jump, or a window that jumped outside the kept grid falls
/// back to a full rebuild — each of which is just `aggregate_rows` again, so
/// correctness never rides on the fast path alone. The offline whole-song path
/// does NOT use this (its column set is fixed and already cached after the first
/// frame).
///
/// **A folded slab is never recomputed, even when the store re-writes the
/// columns behind it.** Columns arrive in time order, so no future column can
/// land in a slab the newest one has already passed: the slab is FINAL the
/// moment it is behind the front. What is not final is the STORE — past
/// `SpectrumHistory`'s finest tier, columns are MAX-merged in pairs and re-timed
/// to their midpoint as they age. Re-deriving an old slab from the merged store
/// therefore answers a slightly different question than folding the raw columns
/// did (a merged pair lands in one slab rather than straddling two, and a
/// [`RowRead::Lerp`] row interpolating across a maxed-together pair is not the
/// max of the two interpolations) — so the two disagree, and this keeps what it
/// folded from the finer data rather than re-reading the coarser.
///
/// That is also what the offline renderer sees: [`crate::WholeSong`] analyses a
/// take into raw columns and never merges them, so a grid built from raw columns
/// is the picture the live heatmap is meant to match.
///
/// Treating the merge as a reason to fall back instead is what this replaced,
/// and it was not a small cost: it fired on every frame at any Span past the
/// finest tier's ~16 s reach, once playback had run long enough for the merging
/// to start — turning a 0.04 ms fold into a multi-millisecond rescan of the
/// whole window that stayed until the plugin was reloaded.
pub(crate) struct SpectrogramAgg {
    grid: SlabGrid,
    bucket_bits: u64,
    reads: Vec<RowRead>,
    /// Time of the newest column already folded; the next update folds only
    /// columns past it.
    last_time: f64,
    /// How many full rebuilds have been taken. The fast path is the whole point
    /// of this type, and every condition guarding it can only ever ADD a reason
    /// to fall back — so without a count, a guard that quietly always fires
    /// would still pass every correctness test here and simply hand back the
    /// `aggregate_rows` cost this exists to avoid.
    ///
    /// Compiled in rather than test-only, and read out by the performance
    /// overlay: a fallback is invisible from inside the DAW, where the picture
    /// is correct and only the frame rate says otherwise — and a frame rate has
    /// other suspects. Twice this pane has cost a round trip of measurement to
    /// learn what this number would have said at a glance.
    rebuilds: u32,
}

impl SpectrogramAgg {
    /// Full rebuilds taken since this aggregator was made — see the field.
    pub(crate) fn rebuilds(&self) -> u32 {
        self.rebuilds
    }

    fn new() -> Self {
        SpectrogramAgg {
            grid: SlabGrid::default(),
            bucket_bits: 0,
            reads: Vec::new(),
            last_time: f64::NEG_INFINITY,
            rebuilds: 0,
        }
    }

    /// Re-fold from scratch, filling the grid to the `keep` slabs it retains
    /// rather than only to the window's own.
    ///
    /// The slack is the point. A rebuild that started at the window's first
    /// column would leave the grid flush with the window, and a Span being
    /// ZOOMED OUT asks for something older on the very next frame — which
    /// rebuilds, flush again, and asks again. That cascade is self-sustaining:
    /// once a widening gesture trips it, every frame of it rebuilds, however
    /// long the aggregator had been running before. It reads on the overlay as a
    /// refold rate pinned at the frame rate for the length of the drag.
    fn rebuild(
        &mut self,
        history: &crate::SpectrumHistory,
        first: usize,
        bucket: f64,
        reads: &[RowRead],
        keep: usize,
    ) {
        self.rebuilds += 1;
        self.grid = SlabGrid::default();
        // Far enough back to fill the retention, but never past the window's
        // own first column — that one must be folded whatever the slack says.
        let newest = history.back().map_or(0.0, |c| c.time);
        let cutoff = ((newest / bucket).floor() - keep as f64 + 1.0) * bucket;
        let start = history.partition_point(|c| c.time < cutoff).min(first);
        for col in history.iter_from(start) {
            self.grid.fold(col, reads, bucket);
        }
        self.bucket_bits = bucket.to_bits();
        self.reads.clear();
        self.reads.extend_from_slice(reads);
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
        reads: &[RowRead],
        keep: usize,
    ) -> (Vec<f64>, Vec<BucketDb>) {
        let target = history.get(first).map(|c| (c.time / bucket).floor() as i64);
        let newest = history.back().map_or(f64::NEG_INFINITY, |c| c.time);
        let layout_same = self.bucket_bits == bucket.to_bits() && self.reads == reads;
        // The fast path is valid only when: the layout is unchanged, we have a
        // prior grid, time hasn't gone backwards, and the window's first slab
        // still sits inside the grid we kept (front..=back). Anything else is a
        // full rebuild, always correct.
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
            self.rebuild(history, first, bucket, reads, keep);
        } else {
            // Fold only columns newer than the last we folded.
            let start = history.partition_point(|c| c.time <= self.last_time);
            let mut forward = true;
            for col in history.iter_from(start) {
                if !self.grid.fold(col, reads, bucket) {
                    forward = false;
                    break;
                }
                self.last_time = col.time;
            }
            if !forward {
                // A mid-stream backward jump broke the grid; rebuild clean.
                self.rebuild(history, first, bucket, reads, keep);
            }
        }
        self.view(history, first, bucket, reads, target, keep)
    }

    /// The window as the display reads it, taken from the kept grid: every slab
    /// from the window's first on, with that one — the only PARTIAL slab —
    /// recomputed from the in-window columns alone, and any HELD copies of it
    /// carrying that recompute forward.
    ///
    /// The recompute is what keeps this equal to batch at the far edge. Batch
    /// folds only columns from `first` onward, so an earlier column sharing that
    /// slab must not count, and the grid MAXed one in while it was still in
    /// window. It is a handful of columns, so still O(1) per frame.
    ///
    /// The grid keeps what the RING keeps — `keep` slabs, sized off the pane —
    /// rather than only what the window currently shows, and everything before
    /// the window's first slab is sliced off here rather than dropped. Two
    /// things need that slack.
    ///
    /// A Span GROWING reaches back to slabs it did not want a frame ago. Trimmed
    /// flush to the window, every frame of a widening drag would ask for a slab
    /// just discarded and rebuild; holding what the texture can hold means the
    /// whole rung is already folded.
    ///
    /// And the window's first column is not fixed in time: once it ages past the
    /// finest tier it is replaced by a merged column standing at its pair's
    /// MIDPOINT, which is EARLIER, so the window's first slab can step BACK. A
    /// merge moves a time by at most half a slab at any Span the pane offers,
    /// which the same slack covers.
    ///
    /// Slicing rather than dropping also keeps the pruning below off a slab that
    /// is about to become an interior one — an interior slab must hold every
    /// column that landed in it, in-window or not.
    fn view(
        &mut self,
        history: &crate::SpectrumHistory,
        first: usize,
        bucket: f64,
        reads: &[RowRead],
        target: Option<i64>,
        keep: usize,
    ) -> (Vec<f64>, Vec<BucketDb>) {
        let nb = reads.len();
        // One mark per slab, which the hold loop at the bottom relies on to
        // index `held` by the same offset it indexes `centers` by. The three
        // arrays are grown together by `fold` and trimmed together below, and
        // nothing downstream compares a mark against anything, so the two
        // going out of step is silent: the marks simply start answering for
        // slabs `drop` positions older than the ones being read.
        debug_assert_eq!(
            self.grid.held.len(),
            self.grid.centers.len(),
            "a held mark per kept slab",
        );
        let (Some(t), Some(&front_center)) = (target, self.grid.centers.first()) else {
            return (self.grid.centers.clone(), self.grid.power.clone());
        };
        let front = (front_center / bucket).floor() as i64;

        // Drop what has fallen out of the ring's reach. Centers run one per slab
        // with no gaps (`fold` gives an empty slab its row too), so a slab key
        // indexes the grid directly and the count IS the reach.
        let last = self.grid.centers.len().saturating_sub(1);
        let drop = self.grid.centers.len().saturating_sub(keep.max(1)).min(last);
        if drop > 0 {
            self.grid.centers.drain(0..drop);
            self.grid.power.drain(0..drop * nb);
            self.grid.held.drain(0..drop);
        }

        let kept = self.grid.centers.len().saturating_sub(1) as i64;
        let start = (t - (front + drop as i64)).clamp(0, kept) as usize;
        let centers = self.grid.centers[start..].to_vec();
        let mut power = self.grid.power[start * nb..].to_vec();
        for v in &mut power[0..nb] {
            *v = 0;
        }
        for c in history.iter_from(first) {
            if (c.time / bucket).floor() as i64 != t {
                break;
            }
            for (k, read) in reads.iter().enumerate() {
                let v = read.of(&c.db);
                if v > power[k] {
                    power[k] = v;
                }
            }
        }
        // A HELD slab is a copy of the one before it, so pruning the first slab
        // has to reach the run of copies standing behind it — they were filled
        // with what the grid held, columns now out of window included, and only
        // this copy is pruned, not the grid. Batch, folding in-window columns
        // alone, holds forward the pruned value; without this the empty slab
        // right after the window's edge reads brighter than the audio was.
        for j in 1..centers.len() {
            if !self.grid.held[start + j] {
                break;
            }
            power.copy_within((j - 1) * nb..j * nb, j * nb);
        }
        (centers, power)
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
fn u_of(layout: &TexLayout, t: f64) -> f32 {
    let slabs = ((t - layout.t_origin) / layout.bucket) as f32;
    (layout.x0 + slabs) / layout.tex_w
}

/// The newest time the texture has data for: the CENTRE of its last slab's
/// texel, which is the last point `u` may reach.
///
/// The strip is drawn out to the now-line, but the newest column is always
/// older than that — it is stamped at the middle of the window it measured, so
/// a healthy stream still lags by half an analysis window (171 ms on Precise),
/// and it lands in a slab that then has to finish before the next begins. What
/// fills that sliver has to come from the newest column, because it is the only
/// thing the analyzer has said about the stretch.
fn hold_time(layout: &TexLayout) -> f64 {
    layout.t_origin + layout.tex_span - 0.5 * layout.bucket
}

/// Texture `u` for a time on the DRAWN strip: [`u_of`] out to
/// [`hold_time`], and pinned there past it.
///
/// A full-width build gets ClampToEdge for this — its last texel IS the texture
/// edge — but a ring's last texel has a neighbour, holding whatever that texel
/// carried a lap ago, which is a column from a whole window back. The strip's
/// leading sliver would then be a stale copy of the far end of the window: dark
/// while something is playing, bright after it stopped. A guard column just
/// outside the run covers a texel of that and no more, and the overrun here is
/// the analyzer's lag, which is several texels once the window is short enough
/// for slabs to sit on [`live_slab`]'s lowest rung — so the sliver is filled by
/// pinning `u`, and the guard is left to the far end, which really does overrun
/// by only half a texel.
///
/// Pinning is safe HERE, at a corner the mesh is split on, and nowhere else:
/// see the split in [`draw_spectrogram`] for why a bend inside a quad is not.
fn u_drawn(layout: &TexLayout, t: f64) -> f32 {
    u_of(layout, t.min(hold_time(layout)))
}

/// Slabs the ring holds: what the [`Plan`] sized off the pane, floored by the
/// run it is actually being asked to show.
///
/// Sized off the PANE, not off the window and not off how much history has
/// arrived: a capacity that tracked either would change as they moved, and
/// every change reallocates the texture and repaints every column — rebuilding
/// the very thing the ring caches. At a fixed slab width the pane's own column
/// count is the widest run any Span can produce, so this holds still across a
/// whole Span drag and only moves when the pane does.
///
/// The run does not sit still at `window / bucket` slabs. It reaches from the
/// last column BEFORE the window's far edge (up to a column's spacing further
/// back) to the newest column (which lags the now-line by half an analysis
/// window), and both ends are floored onto the slab grid, so it breathes by a
/// slab or two as the window scrolls. The headroom covers that breathing: when
/// this was `(window / bucket).ceil() + 2`, a long Span — where a slab is WIDER
/// than the analyzer's lag, so the run reaches the far end instead of stopping
/// short of it — flipped between two capacities seventeen times a second, and
/// each flip cost a full-texture reallocation and a repaint of every column.
fn ring_capacity(planned: usize, visible: usize) -> usize {
    // The max is the correctness floor — the run must fit, with the far end's
    // guard column outside it. It never binds: a window at this slab width is
    // at most `target_cols` slabs and the run overruns it by two, against the
    // headroom's eight. That it never binds is the point, and what
    // `no_cache_layer_falls_back_as_the_window_scrolls` holds it to.
    planned.max(visible + 2)
}

/// Slabs of headroom [`ring_capacity`] holds past the PANE's own column count
/// — `target_cols`, which is what the capacity is now sized off, so it holds
/// still across a Span drag. Four covers the breathing described there (a
/// column's spacing at the far edge is at most one slab, plus a floor at each
/// end); the rest is margin, and `ring_capacity`'s body leans on the whole
/// eight. Each costs TWO texel columns, since every column is written twice
/// (see [`SpectrogramRing`]) — so a full-width pane's texture is
/// `2 * (1024 + 8)` = 2064 texels across.
const RING_HEADROOM: usize = 8;

/// Compose a whole ring texture: every column of the run at its own texel and
/// at its twin, the guard column outside the oldest end, and black everywhere
/// the run does not reach.
///
/// Split out from [`write_ring`] to be testable — the texture it goes into is
/// opaque once uploaded, so the placement is only checkable here, and getting
/// it wrong scrambles the picture rather than failing anything.
fn restart_pixels(
    ring: &SpectrogramRing,
    tex_w: usize,
    h: usize,
    first_key: i64,
    last_key: i64,
    mut column: impl FnMut(usize) -> Vec<Color32>,
) -> Vec<Color32> {
    let mut pixels = vec![Color32::BLACK; tex_w * h];
    // From one before the run: that key is the guard, which duplicates the
    // oldest slab, so it reads the same column as `first_key`.
    for key in first_key - 1..=last_key {
        let column = column((key.max(first_key) - first_key) as usize);
        let x = ring.x_of(key);
        for (row, texel) in column.iter().enumerate().take(h) {
            pixels[row * tex_w + x] = *texel;
            pixels[row * tex_w + x + ring.capacity] = *texel;
        }
    }
    pixels
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
    style: ColumnStyle,
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

    // A ring with no texture under it describes nothing, whatever its
    // bookkeeping says — and there is no more specific reason to report than
    // that it has to be built at all.
    let restart = match (&spectrum.spectrogram[surface].ring, &spectrum.spectrogram[surface].tex) {
        (Some(ring), Some(_)) => ring.carries(capacity, &style, first_key, last_key),
        _ => Some(Restart::Rows),
    };

    if let Some(why) = restart {
        // Counted for the same reason the aggregator counts its rebuilds: a
        // restart re-blanks the texture and repaints every column, and nothing
        // about the picture says it happened.
        spectrum.spectrogram[surface].restarts[why.slot()] += 1;
        // Every column at once, as ONE upload.
        //
        // A restart repaints the whole run, and a column at a time is two
        // uploads per column — each its own texture delta, each carrying a
        // texel of payload and a call's worth of overhead. At a full-width
        // pane that is some two thousand of them in a single frame, which
        // measured 7-10ms: the whole of what makes a window resize stutter,
        // since every step of a drag changes the pane and restarts the ring.
        // Composed here instead, it is one upload of the same pixels.
        //
        // Black behind them for the same reason a fresh texture was blanked:
        // a column never written has to read as silence rather than as
        // whatever the allocation held.
        let fresh = SpectrogramRing::restarted(capacity, style, first_key);
        let pixels = restart_pixels(&fresh, tex_w, h, first_key, last_key, |i| {
            fill_column(cfg, bins, &power[i * h..(i + 1) * h])
        });
        let image = egui::ColorImage::new([tex_w, h], pixels);
        match &mut spectrum.spectrogram[surface].tex {
            Some(handle) => handle.set(image, opts),
            slot => *slot = Some(ctx.load_texture("spectrogram", image, opts)),
        }
        let mut ring = fresh;
        ring.wrote(first_key, last_key);
        spectrum.spectrogram[surface].ring = Some(ring);
        return;
    }

    let (Some(ring), Some(tex)) =
        (&mut spectrum.spectrogram[surface].ring, &mut spectrum.spectrogram[surface].tex)
    else {
        return;
    };

    // Paint what the run has and the texture does not, at either end. Forward
    // starts AT the last column written rather than past it: that slab was
    // uploaded mid-accumulation and may have gained energy since. Backward is
    // the slabs a zoomed-out window has just revealed — none on an ordinary
    // frame, a handful on the frames of a widening gesture.
    //
    // A handful is the whole point of the column-at-a-time writes that follow:
    // this is the steady state, where one or two columns are stale and
    // uploading the other thousand would be the waste. The restart above is
    // the other case, and it is the one that has to go wide.
    let back = first_key..ring.oldest_valid.min(last_key + 1);
    let forward = ring.written_through.max(first_key)..=last_key;
    for key in back.chain(forward) {
        let i = (key - first_key) as usize;
        let column = fill_column(cfg, bins, &power[i * h..(i + 1) * h]);
        let image = egui::ColorImage::new([1, h], column);
        let x = ring.x_of(key);
        tex.set_partial([x, 0], image.clone(), opts);
        // The twin, `capacity` texels along, is what keeps any run of at most
        // `capacity` slabs contiguous — see [`SpectrogramRing`].
        tex.set_partial([x + capacity, 0], image, opts);
    }

    // Duplicate the oldest column just outside the run. The quad reaches half a
    // texel past that end — it stops at the oldest slab's leading EDGE, half a
    // slab before its centre — and a sampler set to ClampToEdge only clamps at
    // the TEXTURE edge, which inside a ring is somewhere else entirely, so
    // without this the far sliver would blend in a column from a whole window
    // ago. Half a texel is all it overruns, so one column covers it.
    //
    // The newest end needs no such column: it overruns by the analyzer's lag,
    // far more than a guard or two would cover, and is filled by pinning `u`
    // instead (see [`u_drawn`]) — which leaves nothing past the last slab's
    // centre ever sampled.
    {
        let column = fill_column(cfg, bins, &power[..h]);
        let image = egui::ColorImage::new([1, h], column);
        let x = ring.x_of(first_key - 1);
        tex.set_partial([x, 0], image.clone(), opts);
        tex.set_partial([x + capacity, 0], image, opts);
    }

    ring.wrote(first_key, last_key);
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
    loudness_db(cfg, db_of(bucket), midi)
}

/// [`bin_level`] for the crate's own tests — the bridge
/// `the_heatmap_reads_the_curve_s_own_level_scale` holds the curve against.
///
/// Exposed rather than let that test reach for `loudness_db` itself, which is
/// what the curve reads too: an assertion whose two sides are both the curve's
/// mapping is one expression compared with itself, and cannot fail however the
/// heatmap's pixels are derived.
#[cfg(test)]
pub(crate) fn bin_level_for_test(cfg: &SpectrumConfig, bucket: BucketDb, midi: f32) -> f32 {
    bin_level(cfg, bucket, midi)
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
/// down in `spectral_pane`), so silence recedes while energy stands out — and
/// the quad is drawn untinted, so what a texel says is what lands. Shared with
/// the spectrum curve so the two read in the same scheme.
pub(super) fn cell_color(kind: SpectrogramColor, level: f32) -> Color32 {
    let t = level.clamp(0.0, 1.0);
    let rgb = match kind {
        SpectrogramColor::Mono => ramp(t, &[[0, 0, 0], [255, 255, 255]]),
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
    // ROUND, not truncate. `as u8` floors, which drops up to a whole level of
    // every interpolated colour — a systematic darkening that also flattens the
    // first step out of each stop into a plateau, on a ramp whose whole job is to
    // read as smooth.
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f).round() as u8;
    [
        lerp(stops[i][0], stops[i + 1][0]),
        lerp(stops[i][1], stops[i + 1][1]),
        lerp(stops[i][2], stops[i + 1][2]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_core::spectrum::SPECTRUM_BINS;

    /// Slabs an aggregator is told to keep, where the test is about the values
    /// it produces rather than about what it retains: larger than any of these
    /// windows holds, so the trim never enters into it. The sweep and the drag
    /// test below pass the real, pane-sized retention instead.
    const KEEP: usize = 1 << 20;

    /// The pitch range the ring sweeps hold fixed while they move time.
    const SWEEP_SCALE: PitchScale = PitchScale { min_midi: 40.0, max_midi: 88.0, span: 48.0 };

    /// The style the pane would build for a window of `span` seconds.
    ///
    /// `cfg.roll_seconds` moves WITH the span, because in the pane they are the
    /// same number — `TimeAxis::new` reads the window straight out of the
    /// config. A fixture that leaves it at its default makes them independent,
    /// and then a style keyed on the config looks stable across a Span drag
    /// when in the pane it is being rewritten every frame. That is exactly how
    /// the drag bug survived a test named after the drag.
    fn style_for(rows: usize, bucket: f64, span: f64, scale: &PitchScale) -> ColumnStyle {
        let cfg = SpectrumConfig { roll_seconds: span as f32, ..SpectrumConfig::default() };
        ColumnStyle::new(rows, bucket, scale.min_midi, scale.span, &cfg)
    }

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
        harmonigraph_core::spectrogram::quantize(power)
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
        let (centers, power) = aggregate_rows(cols.iter(), &[RowRead::Max { from: 5, to: 6 }], 1.0);
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
        let (centers, power) = aggregate_rows(cols.iter(), &[RowRead::Max { from: 5, to: 6 }], 0.25);
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
        let (centers, power) = aggregate_rows(cols.iter(), &[RowRead::Max { from: 5, to: 6 }], 0.25);
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
        let (centers, power) = aggregate_rows(cols.iter(), &[RowRead::Max { from: 5, to: 6 }], 0.25);
        assert_eq!(centers.len(), 2);
        assert_eq!(power[1], q(0.5), "the rewound column landed in its own row");
    }

    #[test]
    fn a_slab_is_anchored_to_absolute_time_not_ring_position() {
        // The same note must land in the same slab whether or not older columns
        // are present — otherwise scrolling would shift it and it would shimmer.
        // A note at t=2.6 sits in slab floor(2.6)=2.
        let with_old = [col(0.1, &[(0, 0.1)]), col(2.6, &[(0, 0.5)])];
        let (c_full, _) = aggregate_rows(with_old.iter(), &[RowRead::Max { from: 0, to: 1 }], 1.0);
        let just_note = [col(2.6, &[(0, 0.5)])];
        let (c_scrolled, _) = aggregate_rows(just_note.iter(), &[RowRead::Max { from: 0, to: 1 }], 1.0);
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
            Bin { read: RowRead::Max { from: 10, to: 11 }, midi: 40.0, t: 0.1 },
            Bin { read: RowRead::Max { from: 11, to: 12 }, midi: 41.0, t: 0.2 },
            Bin { read: RowRead::Max { from: 12, to: 13 }, midi: 42.0, t: 0.3 },
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
        use super::super::axes::{loudness_db, power_db};
        let mut cfg = SpectrumConfig::default();
        let tolerance =
            0.5 * harmonigraph_core::spectrogram::DB_STEP / (cfg.ceiling_db - cfg.floor_db) + 1e-6;
        for tilt in [0.0, 3.0, -3.0] {
            cfg.tilt = tilt;
            for midi in [20.0f32, 60.0, 100.0, 130.0] {
                for power in [1e-8f32, 1e-6, 1e-4, 1e-2, 0.1, 0.5, 1.0, 4.0] {
                    let exact = loudness_db(&cfg, power_db(power), midi);
                    let stored = bin_level(&cfg, q(power), midi);
                    assert!(
                        (stored - exact).abs() <= tolerance,
                        "power {power} at MIDI {midi} (tilt {tilt}): \
                         {exact} exact vs {stored} stored",
                    );
                }
            }
        }
        // And silence stays exactly silent rather than creeping up off the
        // quantizer's floor, whatever the dB window is set to.
        cfg.floor_db = -120.0;
        assert_eq!(bin_level(&cfg, 0, 60.0), 0.0, "an empty bucket must read as silence");
    }

    /// The quiet end of the ramp must FADE to black, not fall off a cliff into
    /// it. A shortcut answering everything under -90 dB as silence is
    /// invisible while the dB window bottoms out above that, and becomes a
    /// hard edge — faintest colour straight to black — as soon as the window
    /// can be dragged below it. Nothing between two adjacent stored bytes may
    /// move the level by more than the step between them.
    ///
    /// The sweep runs past [`LEVEL_MIN_DB`](crate::LEVEL_MIN_DB) on purpose: the
    /// Level bar stops at -100, and a hand-edited blob does not.
    #[test]
    fn the_quiet_end_of_the_ramp_fades_instead_of_cutting_off() {
        let mut cfg = SpectrumConfig { ceiling_db: 0.0, ..SpectrumConfig::default() };
        for floor in [-60.0f32, -90.0, -100.0, -120.0] {
            cfg.floor_db = floor;
            // One stored step, as a fraction of the window it is drawn in; the
            // levels either side of any stored byte may differ by that and no
            // more.
            let step = harmonigraph_core::spectrogram::DB_STEP / (cfg.ceiling_db - floor);
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
        let quiet = cell_color(SpectrogramColor::Magma, 0.0);
        let loud = cell_color(SpectrogramColor::Magma, 1.0);
        // Opaque throughout: a cell's level is its COLOUR, never its alpha, so
        // silence recedes by being dark rather than by being see-through.
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
    /// Both row kinds are in play, since the two paths share one `RowRead::of`
    /// but reach it from different loops.
    #[test]
    fn incremental_aggregation_matches_batch_step_for_step() {
        let reads = [
            RowRead::Max { from: 4, to: 6 },
            RowRead::Max { from: 6, to: 10 },
            RowRead::Lerp { lo: 10, f: 0.25 },
        ];
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
            let inc = agg.window(&history, first, bucket, &reads, KEEP);
            let bat = aggregate_rows(history.iter_from(first), &reads, bucket);
            assert_eq!(inc, bat, "incremental != batch at step {i} (t={t})");
        }

        // A layout change (new bucket) must fall back to a rebuild — still exact.
        let now = *times.last().unwrap();
        let first = history.partition_point(|c| c.time < now - window_span).saturating_sub(1);
        let inc = agg.window(&history, first, 0.4, &reads, KEEP);
        let bat = aggregate_rows(history.iter_from(first), &reads, 0.4);
        assert_eq!(inc, bat, "incremental != batch after a bucket change");
    }

    /// A HELD empty slab must scroll out of the window with the slab it copied,
    /// not keep a peak the window no longer reaches.
    ///
    /// `fold` fills a one-slab gap by copying the previous slab as the GRID
    /// holds it — every column that landed there, in-window or not — while
    /// `view` prunes the window's first slab to its in-window columns. Prune
    /// only the first and the copy behind it still reads the louder value: one
    /// heatmap column brighter than the audio ever was.
    ///
    /// The step-for-step test above cannot reach this. Its one-slab-gap case
    /// (`0.80 → 1.60`) is really a TWO-slab gap, which takes the zero branch,
    /// and no step in its fixture has both two pre-window columns in slab `t`
    /// and slab `t + 1` empty.
    #[test]
    fn a_held_empty_slab_leaves_the_window_with_the_slab_it_copied() {
        let reads = [RowRead::Max { from: 4, to: 6 }];
        let bucket = 0.25;
        // Slab 0 holds three columns, slab 1 is EMPTY (a one-slab gap, so `fold`
        // holds the previous), slab 2 holds one.
        let times = [0.00, 0.05, 0.10, 0.60];
        let energy = [1.0f32, 0.9, 0.2, 0.5];
        let mut history = crate::SpectrumHistory::default();
        let mut agg = SpectrogramAgg::new();
        // Fold with the whole run IN window, so the held slab is filled while
        // nothing has scrolled out of it yet.
        for (&t, &e) in times.iter().zip(&energy) {
            history.push(col(t, &[(4, e)]));
            let _ = agg.window(&history, 0, bucket, &reads, KEEP);
        }
        let rebuilds = agg.rebuilds();

        // Now the window starts at the column at 0.10 — index 2 — so the two
        // louder columns sharing its slab have fallen out of it.
        let inc = agg.window(&history, 2, bucket, &reads, KEEP);
        let bat = aggregate_rows(history.iter_from(2), &reads, bucket);
        assert_eq!(agg.rebuilds(), rebuilds, "a rebuild would explain it away");
        assert_eq!(inc, bat, "slab 1 holds the out-of-window column at 0.00");

        // The same numbers through the REBUILD path: a fresh aggregator whose
        // first call already starts at index 2. Running only this one would let
        // you conclude the fast path was to blame; it is the pair that settles
        // that the bug is in the hold itself.
        let mut fresh = SpectrogramAgg::new();
        assert_eq!(fresh.window(&history, 2, bucket, &reads, KEEP), bat, "and after a rebuild");
    }

    /// A held slab's MARK has to scroll out of the grid with the slab it
    /// describes.
    ///
    /// `view` trims three arrays in step — `centers`, `power`, and the `held`
    /// marks beside them — and only the first two carry a value anything
    /// compares. Leave `held` untrimmed and it grows while the other two are
    /// cut, so `held[start + j]` answers for a slab `drop` positions older
    /// than the one being read. Both directions are wrong and neither shows
    /// up as a crash: a held slab whose mark now reads `false` keeps the value
    /// the GRID folded instead of the pruned one — the empty column past the
    /// window's edge reading brighter than the audio ever was — and an
    /// interior slab whose mark now reads `true` is overwritten with its
    /// neighbour's column.
    ///
    /// The test above cannot reach it. It passes [`KEEP`], so `drop` is always
    /// zero and the trim never runs; the tests that DO pass a pane-sized
    /// retention push columns with no gaps, so every mark is `false` and a
    /// misaligned read still reads `false`. This one needs both at once: a
    /// one-slab gap, and a retention the fixture outgrows.
    #[test]
    fn the_held_marks_are_trimmed_with_the_slabs_they_describe() {
        let reads = [RowRead::Max { from: 4, to: 6 }];
        let bucket = 0.25;
        // Three slabs — a retention the run below outgrows on its last column,
        // unlike [`KEEP`].
        const KEPT: usize = 3;
        // Slab 2 holds three columns, of which the window keeps only the last;
        // slab 3 is EMPTY, so `fold` marks it held and copies slab 2 as the
        // grid holds it, which is the loud one rather than the pruned one.
        let times = [0.00, 0.30, 0.55, 0.60, 0.65, 1.10];
        let energy = [1.0f32, 0.8, 0.7, 0.6, 0.1, 0.5];
        let mut history = crate::SpectrumHistory::default();
        let mut agg = SpectrogramAgg::new();
        for (&t, &e) in times.iter().zip(&energy) {
            history.push(col(t, &[(4, e)]));
            let _ = agg.window(&history, 0, bucket, &reads, KEPT);
        }
        // The last column opened slab 4 with slab 3 empty behind it, taking the
        // grid to five slabs and forcing its first trim: two off the front, and
        // with them two marks.
        assert_eq!(agg.grid.centers.len(), KEPT, "the grid should have been trimmed");
        assert_eq!(agg.grid.held.len(), KEPT, "and the marks trimmed with it");
        // Not vacuous only if a held slab SURVIVED the trim — the whole point
        // is a mark that is still being read after the array moved under it.
        assert!(agg.grid.held.iter().any(|&h| h), "a held slab has to be left to read");
        let rebuilds = agg.rebuilds();

        // Read from the column at 0.65 — index 4 — so the two louder columns
        // sharing its slab have fallen out of the window and are pruned.
        let inc = agg.window(&history, 4, bucket, &reads, KEPT);
        let bat = aggregate_rows(history.iter_from(4), &reads, bucket);
        assert_eq!(agg.rebuilds(), rebuilds, "a rebuild would explain it away");
        assert_eq!(inc, bat, "the held slab carries the PRUNED value forward");
    }

    /// Reaching back past a TIER MERGE — which the step-for-step test above
    /// never does, because it pushes 14 columns and tier 0 holds
    /// [`crate::SpectrumHistory::FINE_COLUMNS`] of them.
    ///
    /// Once tier 0 overflows, its two oldest columns are MAX-merged into one at
    /// their MIDPOINT time (`SpectrogramColumn::absorb`), which rewrites history
    /// the grid has ALREADY folded. Batch over the store AS IT NOW STANDS can
    /// therefore see something else entirely — the merged column falls in a slab
    /// neither original was folded into, and a [`RowRead::Lerp`] row reads ACROSS
    /// two buckets that the merge has already maxed together
    /// (`max(lerp(a), lerp(b)) != lerp(max(a, b))`) — so THAT is not the
    /// comparison to hold this to.
    ///
    /// What it must equal is batch over the columns AS THEY ARRIVED, which is
    /// both the finer answer and the one [`crate::WholeSong`] gives the offline
    /// renderer from its raw, never-merged columns. The window's first slab is
    /// the exception: it is pruned to the in-window columns, which by then are
    /// the merged ones the store holds, so the comparison starts past it.
    #[test]
    fn incremental_aggregation_matches_the_raw_columns_across_a_tier_merge() {
        let reads = [RowRead::Max { from: 4, to: 6 }, RowRead::Lerp { lo: 10, f: 0.5 }];
        let bucket = 0.25;
        let interval = 0.008; // SpectrumState::FFT_INTERVAL, the live column rate.
        // Long enough that nothing scrolls out: the whole run stays in window,
        // so any mismatch is the merge and not the front trim.
        let window_span = 60.0;
        let columns = crate::SpectrumHistory::FINE_COLUMNS + 64;

        let mut agg = SpectrogramAgg::new();
        let mut history = crate::SpectrumHistory::default();
        // The same columns the store was given, kept unmerged as the reference.
        let mut raw: Vec<crate::SpectrogramColumn> = Vec::new();
        for i in 0..columns {
            let t = i as f64 * interval;
            // Adjacent buckets alternate, so the Lerp row's two inputs are never
            // loud in the same column — exactly the case a per-bin MAX across a
            // merge collapses into one loud pair.
            let (a, b) = if i % 2 == 0 { (1.0, 0.0) } else { (0.0, 1.0) };
            let energy = [(4, 0.5), (10, a), (11, b)];
            history.push(col(t, &energy));
            raw.push(col(t, &energy));

            let first = history.partition_point(|c| c.time < t - window_span).saturating_sub(1);
            let inc = agg.window(&history, first, bucket, &reads, KEEP);
            // Comparing every step would be O(columns^2); check often enough to
            // catch the crossing itself, and either side of it.
            let near_merge = i + 8 >= crate::SpectrumHistory::FINE_COLUMNS;
            if near_merge || i % 256 == 0 || i + 1 == columns {
                let t0 = history.get(first).map_or(0, |c| (c.time / bucket).floor() as i64);
                let in_window = raw.iter().filter(|c| (c.time / bucket).floor() as i64 >= t0);
                let want = aggregate_rows(in_window, &reads, bucket);
                assert_eq!(inc.0, want.0, "slab centers diverged at column {i} (t={t})");
                let nb = reads.len();
                assert_eq!(
                    inc.1[nb..],
                    want.1[nb..],
                    "incremental != the raw columns at column {i} (t={t})",
                );
            }
        }
        // And it did it WITHOUT falling back: the merging behind the window is
        // exactly what used to force a rescan on every frame at long Spans.
        assert_eq!(agg.rebuilds, 1, "a merge behind the window forced a rebuild");
    }

    /// The bug this guards: a Span LONGER than the finest tier's ~16 s reach
    /// used to drop to a from-scratch rescan on every frame, for as long as the
    /// plugin stayed open — the fold is O(new columns), but the rescan is
    /// O(columns in window), which at a 30 s Span is thousands of columns times
    /// hundreds of rows, per frame. It survived every value assertion (a rescan
    /// is still correct) and showed up only as the analyzer dropping to 60 fps
    /// after half a minute of playback and never coming back.
    #[test]
    fn a_long_window_keeps_the_fast_path_once_the_store_starts_merging() {
        let reads = [RowRead::Max { from: 4, to: 6 }, RowRead::Lerp { lo: 10, f: 0.5 }];
        let interval = 0.008;
        // Past the finest tier's reach (FINE_COLUMNS * interval, ~16.4 s), which
        // is the line the old guard broke at, and out to a Span the pane really
        // offers (up to ROLL_SECONDS_MAX, 600 s).
        let window_span = 30.0;
        let bucket = window_span / LIVE_SLAB_CAP as f64;
        // Long enough that the window is full AND the merging is continuous.
        let columns = ((window_span + 20.0) / interval) as usize;

        let mut agg = SpectrogramAgg::new();
        let mut history = crate::SpectrumHistory::default();
        for i in 0..columns {
            let t = i as f64 * interval;
            let (a, b) = if i % 2 == 0 { (1.0, 0.0) } else { (0.0, 1.0) };
            history.push(col(t, &[(4, 0.5), (10, a), (11, b)]));
            let first = history.partition_point(|c| c.time < t - window_span).saturating_sub(1);
            let (centers, power) = agg.window(&history, first, bucket, &reads, KEEP);
            // The window it serves is still the right shape and length.
            assert_eq!(power.len(), centers.len() * reads.len(), "grid shape at column {i}");
            assert!(
                centers.len() <= LIVE_SLAB_CAP as usize + 2,
                "the served window grew past the Span at column {i}: {} slabs",
                centers.len(),
            );
        }
        assert_eq!(agg.rebuilds, 1, "a long Span fell back to a rescan");
    }

    /// The merge guard must not cost the fast path on the windows that use it.
    /// A Span SHORTER than the finest tier's reach sits entirely inside columns
    /// no merge has touched, so the merging going on behind it is none of its
    /// business and the incremental fold has to keep running. Only the rebuild
    /// count can tell: falling back is still CORRECT, so every assertion about
    /// values would pass just the same with the optimization switched off.
    #[test]
    fn a_short_window_keeps_the_fast_path_across_merges() {
        let reads = [RowRead::Max { from: 4, to: 6 }, RowRead::Lerp { lo: 10, f: 0.5 }];
        let bucket = 0.25;
        let interval = 0.008;
        // Two seconds against the finest tier's ~16, so the window stays well
        // inside it even once merging is continuous.
        let window_span = 2.0;
        let columns = crate::SpectrumHistory::FINE_COLUMNS + 256;

        let mut agg = SpectrogramAgg::new();
        let mut history = crate::SpectrumHistory::default();
        for i in 0..columns {
            let t = i as f64 * interval;
            let (a, b) = if i % 2 == 0 { (1.0, 0.0) } else { (0.0, 1.0) };
            history.push(col(t, &[(4, 0.5), (10, a), (11, b)]));

            let first = history.partition_point(|c| c.time < t - window_span).saturating_sub(1);
            let inc = agg.window(&history, first, bucket, &reads, KEEP);
            if i % 256 == 0 || i + 1 == columns {
                let bat = aggregate_rows(history.iter_from(first), &reads, bucket);
                assert_eq!(inc, bat, "incremental != batch at column {i} (t={t})");
            }
        }
        // One to build the grid; the rest of the run rides the incremental path,
        // merges and all. Anything more means the guard is firing on windows it
        // has no reason to touch.
        assert_eq!(agg.rebuilds, 1, "the fast path stopped carrying a short window");
    }

    /// The layout arithmetic every rebuild rides on, decided without a frame.
    ///
    /// This is where the pipeline's cliffs live — a slab against the analyzer's
    /// lag, a slab against the column rate, an image against what the GPU will
    /// allocate — and until [`Plan`] was split out of the draw it could only be
    /// exercised through an `egui::Context` with a real texture, which is to say
    /// not at all. Each assertion below is a bug this pane has actually had.
    #[test]
    fn the_plan_decides_the_layout_without_a_frame() {
        let scale = PitchScale { min_midi: 40.0, max_midi: 88.0, span: 48.0 };
        let view = |ppp: f32, pitch_len: f32, depth_len: f32, window: f64, whole: bool| PaneView {
            ppp,
            max_rows: 8192,
            pitch_len,
            depth_len,
            window,
            scale,
            cfg: SpectrumConfig::default(),
            whole,
        };
        let columns = Columns { first: 3, len: 400, newest: 12.0 };
        let plan = |v: &PaneView| Plan::new(v, &columns);

        // Rows are PIXELS, not points: on a 2x screen the image is built at the
        // density it will be drawn at, rather than upsampled from half of it.
        // Rounded up to a quantum, so never coarser than the pane and never
        // more than a quantum finer — see [`PANE_QUANTUM`].
        let rows_at = |ppp: f32, pitch: f32| plan(&view(ppp, pitch, 800.0, 12.0, false)).rows as f32;
        for (ppp, pitch) in [(1.0, 300.0), (2.0, 300.0), (1.0, 517.0), (2.0, 517.0)] {
            let rows = rows_at(ppp, pitch);
            let want = pitch * ppp;
            assert!(rows >= want, "{rows} rows is coarser than {want} pixels");
            assert!(rows < want + PANE_QUANTUM, "{rows} rows for {want} pixels wastes an image");
        }
        // Twice the density really is twice the image, quantum aside.
        assert!((rows_at(2.0, 517.0) / rows_at(1.0, 517.0) - 2.0).abs() < 0.2);
        // And never a taller image than the GPU will take.
        let mut small = view(2.0, 4000.0, 800.0, 12.0, false);
        small.max_rows = 2048;
        assert_eq!(plan(&small).rows, 2048, "an image taller than the GPU allocates");

        // A slab is never finer than the columns arrive, whatever the pane's
        // width asks for: a shorter one leaves empty slabs between columns, and
        // the uniform time axis then stretches the edge columns into flat
        // streaks. Two columns to a slab, since the grids share the ladder's
        // period but not its phase.
        let floor = crate::AudioSpectrum::FFT_INTERVAL * LADDER_FLOOR_COLUMNS;
        let dense = plan(&view(2.0, 300.0, 4000.0, 1.0, false));
        assert!(dense.bucket >= floor, "slab {} is finer than the data", dense.bucket);
        // And it is always ON the ladder, so it lands on the store's own grid.
        for window in [1.0f64, 12.0, 30.0, 600.0] {
            let steps = (plan(&view(2.0, 300.0, 4000.0, window, false)).bucket / floor).log2();
            assert!((steps - steps.round()).abs() < 1e-9, "a {window} s Span fell off the ladder");
        }

        // And the live image is never cut into more slabs than the store can
        // keep up with — the cap `SpectrumHistory::COARSE_COLUMNS` is sized
        // against.
        for window in [1.0f64, 12.0, 60.0, 600.0] {
            let p = plan(&view(2.0, 300.0, 4000.0, window, false));
            let slabs = window / p.bucket;
            assert!(
                slabs <= LIVE_SLAB_CAP as f64 + 1.0,
                "a {window} s Span asks for {slabs} slabs, past the {LIVE_SLAB_CAP} cap",
            );
        }
        // The whole-song build spans an entire take rather than a window, so it
        // is allowed the higher cap.
        let take = plan(&view(2.0, 300.0, 4000.0, 600.0, true));
        assert!(
            600.0 / take.bucket > LIVE_SLAB_CAP as f64,
            "the offline build should out-resolve the live cap",
        );

        // A wider pane buys finer slabs, up to the cap — the pane's size is an
        // INPUT to the grid, which is why resizing rebuilds it.
        let narrow = plan(&view(1.0, 300.0, 200.0, 30.0, false));
        let wide = plan(&view(1.0, 300.0, 900.0, 30.0, false));
        assert!(wide.bucket < narrow.bucket, "a wider pane should resolve time more finely");

        // The key travels with the plan, so a cache hit means THIS layout.
        let at_1x = plan(&view(1.0, 300.0, 800.0, 12.0, false)).key;
        let at_2x = plan(&view(2.0, 300.0, 800.0, 12.0, false)).key;
        assert_eq!(at_1x, plan(&view(1.0, 300.0, 800.0, 12.0, false)).key);
        assert_ne!(at_1x, at_2x, "a density change must not hit the cache");
    }

    /// **Every cache layer must stay on its fast path across the whole REGIME
    /// GRID**, not just at the settings the pane opens on.
    ///
    /// Falling back is always CORRECT — a rescan and a reallocation both draw
    /// the right picture — so no value assertion anywhere in this file can see
    /// a layer that has quietly stopped working. What is left is to count the
    /// fallbacks, and to count them at settings the defaults never reach: both
    /// of the regressions this guards fired only past a Span the whole suite
    /// otherwise stayed below, and both then held until the plugin was reloaded.
    ///
    /// The grid is chosen where the cliffs are, and the cliffs are RATIOS
    /// between constants picked independently of each other:
    ///
    /// - the window against the store's finest tier
    ///   (`FINE_COLUMNS * FFT_INTERVAL`, ~16.4 s), which decides whether the
    ///   aggregator's window holds merged columns; and
    /// - a SLAB against the analyzer's LAG (half an analysis window), which
    ///   decides whether the visible run reaches the far end of the window and
    ///   so whether the ring's capacity tracks it.
    ///
    /// The second moves with the FFT window AND with the pane's width, since a
    /// slab is `Span / depth pixels` — a narrow pane crosses it at a much
    /// shorter Span than a wide one. So the sweep is taken per (window, pane)
    /// pair, either side of that pair's own crossing.
    ///
    /// The texture cache above these two is deliberately not counted: its key
    /// holds the newest column's time, so it is MEANT to miss once per column.
    /// It is the two layers under it that must turn a miss into O(one column).
    #[test]
    fn no_cache_layer_falls_back_as_the_window_scrolls() {
        let reads = [RowRead::Max { from: 4, to: 6 }, RowRead::Lerp { lo: 10, f: 0.5 }];
        let interval = crate::AudioSpectrum::FFT_INTERVAL;

        // Every FFT window the pane offers, by the lag it gives a column: a
        // column is stamped at the middle of the window it measured.
        let windows = [
            ("Fast", 0.5 * 4096.0 / 48000.0),
            ("Balanced", 0.5 * 8192.0 / 48000.0),
            ("Precise", 0.5 * 16384.0 / 48000.0),
        ];
        // Depth pixels the image is cut into: a full-width pane (which the cap
        // holds at LIVE_SLAB_CAP) and a narrow one.
        let panes = [("wide", LIVE_SLAB_CAP as f64), ("narrow", 384.0)];

        for (algo, lag) in windows {
            for (pane, cols) in panes {
                // Where a slab is exactly the lag — the crossing this pair's
                // ring behaviour turns on — plus the Span the pane opens on.
                let crossing = lag * cols;
                for span in [12.0f64, crossing * 0.6, crossing * 1.4] {
                    let planned = cols as usize + RING_HEADROOM;
                    let bucket = live_slab(span, cols as usize);
                    let at = format!("{algo} window, {pane} pane, {span:.1} s Span");

                    let mut agg = SpectrogramAgg::new();
                    let mut history = crate::SpectrumHistory::default();
                    let mut ring: Option<SpectrogramRing> = None;
                    let (mut restarts, mut caps) = (0u32, std::collections::BTreeSet::new());

                    // Long enough to fill the window and then scroll a while
                    // inside it, which is where the run starts breathing.
                    let columns = ((span + 15.0) / interval) as usize;
                    for i in 0..columns {
                        let t = i as f64 * interval;
                        history.push(col(t, &[(4, 0.5), (10, 1.0)]));
                        // The shell clock: the newest column always lags it.
                        let now = t + lag;

                        // Exactly what `draw_spectrogram` asks for each frame.
                        let first =
                            history.partition_point(|c| c.time < now - span).saturating_sub(1);
                        let (centers, _) =
                            agg.window(&history, first, bucket, &reads, planned);
                        let visible = centers.len();
                        let first_key = (centers[0] / bucket).floor() as i64;
                        let last_key = first_key + visible as i64 - 1;
                        let capacity = ring_capacity(planned, visible);
                        let style = style_for(reads.len(), bucket, span, &SWEEP_SCALE);

                        // And exactly what `write_ring` then decides.
                        if ring.as_ref().is_none_or(|r| {
                            r.carries(capacity, &style, first_key, last_key).is_some()
                        }) {
                            restarts += 1;
                            ring = Some(SpectrogramRing::restarted(
                                capacity,
                                style.clone(),
                                first_key,
                            ));
                            caps.insert(capacity);
                        }
                        ring.as_mut().expect("just restarted").wrote(first_key, last_key);
                    }

                    // One of each to get started, and none after: from then on a
                    // frame folds one column and writes one texel column.
                    assert_eq!(agg.rebuilds, 1, "the aggregator rescans the window: {at}");
                    assert_eq!(restarts, 1, "the ring is reallocated ({caps:?} slabs): {at}");
                }
            }
        }
    }

    /// A gesture must cost a re-layout at its BOUNDARIES, not one per frame.
    ///
    /// Both caches used to reset their own slack whenever they rebuilt: the
    /// aggregator re-folded flush with the window, and the ring's painted range
    /// restarted flush with it too. A Span being ZOOMED OUT then asks for
    /// something older on the very next frame — which rebuilds, sits flush, and
    /// is asked again. Self-sustaining: once a widening gesture trips it, every
    /// frame of the drag rebuilds, however long the caches had been running
    /// before, and the overlay reads both counters pinned at the frame rate.
    ///
    /// Now a rebuild refills the retention behind the window, and the ring
    /// paints the revealed slabs rather than starting over — so a zoom costs a
    /// re-layout only where the slab width actually changes, at a rung.
    ///
    /// The exception is a PITCH zoom, which moves `reads` on every frame and so
    /// re-folds on every frame. That one wants time and pitch aggregated
    /// separately, which is a larger change and not made here; it is left
    /// unasserted rather than pinned, so fixing it does not fail this.
    #[test]
    fn a_gesture_costs_a_layout_at_its_boundaries_not_a_frame() {
        let interval = crate::AudioSpectrum::FFT_INTERVAL;
        let lag = 0.5 * 8192.0 / 48000.0;
        let cols = LIVE_SLAB_CAP as usize;
        let planned = cols + RING_HEADROOM;
        let scale = PitchScale { min_midi: 40.0, max_midi: 88.0, span: 48.0 };
        let rows = 64;
        let reads: Vec<RowRead> = bins_for(rows, &scale).iter().map(|b| b.read).collect();

        // A gesture: what Span it asks for on its `n`th frame, and the
        // re-layouts it is allowed over the whole run.
        type Gesture<'a> = (&'a str, u32, &'a dyn Fn(u32) -> f64);
        // Held still; zoomed out across four rungs; zoomed in inside one. The
        // gesture starts as soon as the window is full, so neither cache has
        // scrolled itself any slack — which is the case that cascaded.
        let gestures: [Gesture; 3] = [
            ("held still", 1, &|_| 30.0),
            ("zoomed out across rungs", 5, &|i| 20.0 * 1.004f64.powi(i as i32)),
            ("zoomed in inside a rung", 1, &|i| 30.0 - 5.0 * (i as f64 / 200.0)),
        ];

        for (name, allowed, gesture) in gestures {
            let mut agg = SpectrogramAgg::new();
            let mut history = crate::SpectrumHistory::default();
            let mut ring: Option<SpectrogramRing> = None;
            let (mut restarts, mut frames) = (0u32, 0u32);
            let settle = 70.0;
            for i in 0..(((settle + 2.0) / interval) as usize) {
                let t = i as f64 * interval;
                history.push(col(t, &[(4, 0.5), (10, 1.0)]));
                let now = t + lag;
                if now < settle {
                    continue;
                }
                let span = gesture(frames);
                frames += 1;

                let bucket = live_slab(span, cols);
                let style = style_for(rows, bucket, span, &scale);
                let first = history.partition_point(|c| c.time < now - span).saturating_sub(1);
                let (centers, _) = agg.window(&history, first, bucket, &reads, planned);
                let first_key = (centers[0] / bucket).floor() as i64;
                let last_key = first_key + centers.len() as i64 - 1;
                let capacity = ring_capacity(planned, centers.len());
                if ring
                    .as_ref()
                    .is_none_or(|r| r.carries(capacity, &style, first_key, last_key).is_some())
                {
                    restarts += 1;
                    ring = Some(SpectrogramRing::restarted(capacity, style.clone(), first_key));
                }
                ring.as_mut().expect("just restarted").wrote(first_key, last_key);
            }
            assert!(frames > 200, "{name}: the gesture never ran ({frames} frames)");
            assert!(
                agg.rebuilds() <= allowed,
                "{name}: {} re-folds over {frames} frames, not {allowed}",
                agg.rebuilds(),
            );
            assert!(
                restarts <= allowed,
                "{name}: {restarts} ring restarts over {frames} frames, not {allowed}",
            );
        }
    }

    /// Resizing the pane must not re-lay the grid on every PIXEL of the drag.
    ///
    /// The pane's height decides the image's rows and its width decides the
    /// slabs and the ring's width, so taken to the pixel every frame of a resize
    /// is a different image and a different texture — a full re-fold and a full
    /// repaint each, for as long as the drag lasts. On the overlay's fallback
    /// row that showed up as both counters sitting at the frame rate: the drag
    /// was not merely dropping frames, it was spending each one rebuilding.
    ///
    /// Vertical moves both (rows change the image AND which buckets a row
    /// reads); horizontal moves the ring alone, since [`live_slab`] holds the
    /// slab width across a rung. Both are counted here.
    #[test]
    fn resizing_the_pane_holds_the_grid_between_quanta() {
        let scale = PitchScale { min_midi: 40.0, max_midi: 88.0, span: 48.0 };
        let view = |pitch_len: f32, depth_len: f32| PaneView {
            ppp: 2.0,
            max_rows: 8192,
            pitch_len,
            depth_len,
            window: 30.0,
            scale,
            cfg: SpectrumConfig::default(),
            whole: false,
        };
        let columns = Columns { first: 3, len: 4000, newest: 12.0 };
        // What a re-layout is, from each cache's point of view: the aggregator
        // re-folds when the style changes (rows or slab width), and the ring
        // restarts when the style OR its capacity does.
        let layouts = |sizes: &dyn Fn(f32) -> PaneView, from: i32, to: i32| {
            let (mut styles, mut rings) = (
                std::collections::BTreeSet::new(),
                std::collections::BTreeSet::new(),
            );
            for px in from..=to {
                let plan = Plan::new(&sizes(px as f32), &columns);
                styles.insert(format!("{:?}", plan.key.style()));
                rings.insert((format!("{:?}", plan.key.style()), plan.capacity));
            }
            (styles.len(), rings.len())
        };

        // A 600-point drag at 2x is 1200 device pixels: without a quantum every
        // one of them is its own layout.
        let dragged = 600;
        let quanta = (dragged as f32 * 2.0 / PANE_QUANTUM).ceil() as usize + 1;

        let (folds, rings) = layouts(&|h| view(h, 800.0), 300, 300 + dragged);
        assert!(folds <= quanta, "a vertical drag re-folds {folds} times, not {quanta}");
        assert!(rings <= quanta, "a vertical drag restarts the ring {rings} times");

        let (folds, rings) = layouts(&|w| view(400.0, w), 300, 300 + dragged);
        assert!(rings <= quanta, "a horizontal drag restarts the ring {rings} times");
        // And the ladder means width barely touches the aggregator at all: only
        // a rung crossing re-folds, which a whole drag does a handful of times.
        assert!(folds <= 8, "a horizontal drag re-folds {folds} times, not a handful");
    }

    /// Dragging the Span must not re-lay the grid on every frame of the drag.
    ///
    /// This is what [`live_slab`]'s ladder is for. A slab width taken straight
    /// from the window moves whenever the window does, and a moved slab width is
    /// a different grid AND a different texture — so a drag used to pay the full
    /// rebuild, every frame, for as long as it lasted. On the ladder the width
    /// holds across a whole rung, the ring is sized off the pane so it holds too,
    /// and the aggregator keeps what the ring keeps so a WIDENING Span finds its
    /// slabs already folded rather than asking for ones just trimmed.
    ///
    /// Swept in both directions: widening is the harder one, since it reaches
    /// back to slabs the window did not want a frame ago.
    #[test]
    fn dragging_the_span_holds_the_grid_between_ladder_steps() {
        let reads = [RowRead::Max { from: 4, to: 6 }, RowRead::Lerp { lo: 10, f: 0.5 }];
        let interval = crate::AudioSpectrum::FFT_INTERVAL;
        let lag = 0.5 * 8192.0 / 48000.0;
        let cols = LIVE_SLAB_CAP as usize;
        let planned = cols + RING_HEADROOM;
        // One rung, end to end: at the 1024-slab cap a width holds while the
        // Span runs from 512 of them to 1024 of them, which here is 16.4 s to
        // 32.8 s. The sweep stops just inside both ends, since crossing a rung
        // is a real re-layout and not what this is about.
        let rung = live_slab(30.0, cols);
        let (lo, hi) = (rung * 520.0, rung * 1010.0);

        let mut agg = SpectrogramAgg::new();
        let mut history = crate::SpectrumHistory::default();
        let mut ring: Option<SpectrogramRing> = None;
        let (mut restarts, mut widths) = (0u32, std::collections::BTreeSet::new());
        // Which reason, if it does — a Span drag must not restyle at all now
        // that the style holds only what reaches a texel, so anything but the
        // opening build is a real bug and this says which.
        let mut reasons: std::collections::BTreeSet<&'static str> =
            std::collections::BTreeSet::new();

        // Fill past the widest Span in the sweep, so every frame of it is asking
        // for a window the store can actually cover.
        let settled = hi + 10.0;
        let frames = ((settled + 40.0) / interval) as usize;
        let mut dragged = 0u32;
        for i in 0..frames {
            let t = i as f64 * interval;
            history.push(col(t, &[(4, 0.5), (10, 1.0)]));
            let now = t + lag;
            // Once the store is deep enough, sweep the Span down and back up,
            // a frame at a time as a drag delivers it.
            let span = if now < settled {
                hi
            } else {
                dragged += 1;
                let phase = (dragged as f64 / 600.0).min(2.0);
                if phase <= 1.0 { hi - (hi - lo) * phase } else { lo + (hi - lo) * (phase - 1.0) }
            };

            let bucket = live_slab(span, cols);
            let style = style_for(reads.len(), bucket, span, &SWEEP_SCALE);
            widths.insert((bucket * 1e6).round() as i64); // microseconds, to compare exactly
            let first = history.partition_point(|c| c.time < now - span).saturating_sub(1);
            let (centers, _) = agg.window(&history, first, bucket, &reads, planned);
            let first_key = (centers[0] / bucket).floor() as i64;
            let last_key = first_key + centers.len() as i64 - 1;
            let capacity = ring_capacity(planned, centers.len());
            let why = ring.as_ref().map(|r| r.carries(capacity, &style, first_key, last_key));
            if why.is_none_or(|w| w.is_some()) {
                restarts += 1;
                reasons.extend(why.flatten().map(|w| Restart::LABELS[w.slot()]));
                ring = Some(SpectrogramRing::restarted(capacity, style.clone(), first_key));
            }
            ring.as_mut().expect("just restarted").wrote(first_key, last_key);
        }

        assert!(dragged > 1000, "the sweep never ran: {dragged} frames");
        assert_eq!(
            widths.len(),
            1,
            "the slab width took {} values inside one rung ({:?} us) — every one of them \
             re-lays the grid and re-blanks the texture",
            widths.len(),
            (widths.first(), widths.last()),
        );
        assert_eq!(agg.rebuilds, 1, "the drag rescanned the window");
        // The opening build and nothing else. A reason here names the field
        // that moved, which since the style holds only what reaches a texel is
        // a real bug rather than a cost.
        assert!(reasons.is_empty(), "the drag restarted the ring: {reasons:?}");
        assert_eq!(restarts, 1, "the drag reallocated the ring");
    }

    /// The same drag as above, with the Span read from where the pane reads it.
    ///
    /// [`dragging_the_span_holds_the_grid_between_ladder_steps`] sweeps a span
    /// past a style built once from a default config, so in that fixture the
    /// two are independent. In the pane they are one number: the window IS
    /// `cfg.roll_seconds` (`TimeAxis::new`), and the drag writes
    /// `cfg.roll_seconds` on every frame it delivers. So anything the style
    /// keeps from `cfg` moves on every frame of the drag, and the ladder's
    /// whole promise — the grid holds still, so the texture is carried forward
    /// — is void however still `bucket` holds.
    ///
    /// Two frames is the whole test: the drag's cost is per frame, so a single
    /// step inside one rung either carries or does not.
    #[test]
    fn dragging_the_span_carries_the_ring_forward() {
        let scale = PitchScale { min_midi: 40.0, max_midi: 88.0, span: 48.0 };
        let columns = Columns { first: 3, len: 400, newest: 12.0 };
        let plan_at = |span: f32| {
            // The Span as the pane holds it: the window IS this field.
            let cfg = SpectrumConfig { roll_seconds: span, ..Default::default() };
            Plan::new(
                &PaneView {
                    ppp: 2.0,
                    max_rows: 8192,
                    pitch_len: 300.0,
                    depth_len: 800.0,
                    window: span as f64,
                    scale,
                    cfg,
                    whole: false,
                },
                &columns,
            )
        };
        // One drag-delta apart, well inside a rung — the ladder is what makes
        // that a claim about the style alone rather than about the slab width.
        let (a, b) = (plan_at(12.0), plan_at(12.043));
        assert_eq!(a.bucket, b.bucket, "the sweep crossed a rung; pick a smaller step");
        assert_eq!(a.capacity, b.capacity, "the pane did not move, so neither may the ring");
        assert_eq!(
            a.key.style(),
            b.key.style(),
            "one frame of a Span drag re-blanks the whole texture and repaints every column",
        );
    }

    /// The cache reuses the uploaded texture only while its key matches, and
    /// the RING carries its columns forward only while the key's style matches,
    /// so both must move for every input the pixels depend on — otherwise a
    /// change leaves a stale image on screen, or worse, a texture whose newest
    /// column was painted under the new setting and whose other thousand were
    /// not.
    ///
    /// Those used to be two hand-kept lists in two files with a test on one of
    /// them. They are one list now, so this covers both.
    #[test]
    fn the_key_is_sensitive_to_every_input() {
        let cfg = SpectrumConfig::default();
        let style =
            |rows, bucket, min, span, cfg: &SpectrumConfig| {
                ColumnStyle::new(rows, bucket, min, span, cfg)
            };
        let base_style = || style(100, 0.1, 40.0, 48.0, &cfg);
        let base = || crate::SpectrogramKey::new(base_style(), 3, 200, 5.0, false);
        // Same inputs -> equal: this is the hit that skips the rebuild.
        assert_eq!(base(), base());
        assert_eq!(*base().style(), base_style());

        // Every field of the STYLE recolours or re-reads every column, so the
        // ring cannot carry a texture across a change to any of them.
        let styled = |s: ColumnStyle| {
            assert_ne!(s, base_style(), "the ring would carry a stale texture forward");
            assert_ne!(crate::SpectrogramKey::new(s, 3, 200, 5.0, false), base());
        };
        styled(style(101, 0.1, 40.0, 48.0, &cfg)); // rows
        styled(style(100, 0.2, 40.0, 48.0, &cfg)); // slab width
        styled(style(100, 0.1, 41.0, 48.0, &cfg)); // pitch range, low end
        styled(style(100, 0.1, 40.0, 49.0, &cfg)); // pitch range, span
        // Every colour input, one at a time: the palette, either end of the
        // Level window, or the tilt recolours every pixel without moving a
        // column. Spelled out one by one because [`ColumnColor`] is a list kept
        // by hand, and a field left off it is a WRONG picture — which, unlike a
        // slow one, no frame counter reports.
        let recoloured = |edit: fn(&mut SpectrumConfig)| {
            let mut c = cfg;
            edit(&mut c);
            styled(style(100, 0.1, 40.0, 48.0, &c));
        };
        recoloured(|c| c.spectrogram_color = SpectrogramColor::Mono);
        recoloured(|c| c.floor_db -= 6.0);
        recoloured(|c| c.ceiling_db -= 6.0);
        recoloured(|c| c.tilt += 1.0);

        // The converse, which is what the ring is FOR. A config field that
        // reaches no texel has to leave the style ALONE, or every frame of the
        // drag that moves it re-blanks the texture and repaints every column.
        // Both below are continuous drags, and the first is the one the ladder
        // in [`live_slab`] was written to make free.
        let carried = |edit: fn(&mut SpectrumConfig)| {
            let mut c = cfg;
            edit(&mut c);
            assert_eq!(
                style(100, 0.1, 40.0, 48.0, &c),
                base_style(),
                "a drag on this would re-blank the whole texture on every frame",
            );
        };
        carried(|c| c.roll_seconds *= 1.01); // Span: the drag along time
        carried(|c| c.roll_fraction += 0.01); // the roll/heatmap divider

        // And every field that says WHICH columns were drawn. These move as the
        // window scrolls, which the ring is built to survive — so they must move
        // the key without moving the style.
        let windowed = |k: crate::SpectrogramKey| {
            assert_ne!(k, base(), "a scrolled window would reuse the built image");
            assert_eq!(*k.style(), base_style(), "a scroll must not restyle the ring");
        };
        windowed(crate::SpectrogramKey::new(base_style(), 4, 200, 5.0, false)); // scrolled
        windowed(crate::SpectrogramKey::new(base_style(), 3, 201, 5.0, false)); // count
        windowed(crate::SpectrogramKey::new(base_style(), 3, 200, 6.0, false)); // fresh column
        windowed(crate::SpectrogramKey::new(base_style(), 3, 200, 5.0, true)); // whole-song
    }

    /// A column written on its own must be pixel-for-pixel what the
    /// whole-image build would have put in that column. If these two ever
    /// disagree, the live heatmap and an offline render of the same audio stop
    /// looking alike, since which one runs is exactly what tells them apart.
    #[test]
    fn one_column_matches_the_whole_image_build() {
        let cfg = SpectrumConfig::default();
        let bins = [
            Bin { read: RowRead::Max { from: 0, to: 1 }, midi: 40.0, t: 0.0 },
            Bin { read: RowRead::Max { from: 1, to: 2 }, midi: 52.0, t: 0.5 },
            Bin { read: RowRead::Max { from: 2, to: 3 }, midi: 64.0, t: 1.0 },
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

    /// Every texel the window reads holds ITS OWN slab's column — asserted
    /// against the uploaded texture, not against the ring's account of it.
    ///
    /// [`write_ring`] duplicates the run's oldest column into the texel of
    /// `first_key - 1`, to fill the half texel the quad overruns at the far
    /// edge, so that one texel deliberately holds a slab that is not its own.
    /// A window scrolling a slab at a time walks that guard forward a key per
    /// frame, and the whole band below `first_key` ends up holding its
    /// neighbour's column. What keeps the band off the screen is
    /// [`SpectrogramRing::wrote`] floating its floor up to `first_key`, so a
    /// widen reaching back into it lands inside `back` and is repainted rather
    /// than trusted.
    ///
    /// Bookkeeping cannot see any of that. An assertion on `oldest_valid`
    /// restates the arithmetic the floor is written in, and passes just as
    /// happily when the repaint it authorises is deleted — which is how both a
    /// gutted `back` range and a deleted guard once left the whole suite
    /// green. This drives the real [`write_ring`], drains egui's texture
    /// deltas into a model of the texture, and compares each texel against the
    /// column its slab should have, so it fails on the WRONG PICTURE rather
    /// than on a changed expression, and survives a refactor of how `back` is
    /// computed.
    ///
    /// Neither of the branches that met here owns the bug this pins. While the
    /// ring's style still held the whole `SpectrumConfig` it held
    /// `roll_seconds`, so every frame of a Span drag restarted the ring and
    /// wiped the guarded band before anything could sample it — the
    /// carry-forward was unreachable for the one gesture that reaches
    /// backwards. Narrowing the style is what made a widen carry, and so made
    /// the band reachable.
    ///
    /// The run starts below zero and crosses it, so `x_of`'s `rem_euclid` is
    /// in play on negative keys.
    #[test]
    fn every_texel_the_window_reads_holds_its_own_slab() {
        let ctx = egui::Context::default();
        let mut spectrum = crate::AudioSpectrum::default();
        let scale = SWEEP_SCALE;
        let bins = bins_for(4, &scale);
        let h = bins.len();
        let cfg = SpectrumConfig::default();
        let bucket = 0.05;
        let style = style_for(h, bucket, 12.0, &scale);
        let capacity = 32usize;
        let tex_w = capacity * 2;

        // A distinct, reproducible column per slab key, so a texel holding its
        // NEIGHBOUR is a value mismatch rather than a shape one.
        let power_of = |key: i64| -> Vec<BucketDb> {
            (0..h).map(|b| ((key * 13 + b as i64 * 29).rem_euclid(200) + 30) as BucketDb).collect()
        };
        let expect = |key: i64| fill_column(&cfg, &bins, &power_of(key));

        // A model of the uploaded texture, kept in step by draining egui's
        // texture deltas after every call — `set_partial` is the only thing
        // that says where a column actually landed.
        let mut model = vec![Color32::TRANSPARENT; tex_w * h];
        let drain = |model: &mut Vec<Color32>| {
            let delta = ctx.tex_manager().write().take_delta();
            for (_, d) in delta.set {
                let egui::epaint::image::ImageData::Color(img) = d.image;
                let w = img.size[0];
                let [px, py] = d.pos.unwrap_or([0, 0]);
                for (i, c) in img.pixels.iter().enumerate() {
                    model[(py + i / w) * tex_w + px + i % w] = *c;
                }
            }
        };

        let run = |spectrum: &mut crate::AudioSpectrum, first: i64, last: i64| {
            let n = (last - first + 1) as usize;
            let mut power = Vec::with_capacity(n * h);
            for key in first..=last {
                power.extend(power_of(key));
            }
            write_ring(&ctx, spectrum, 0, style.clone(), capacity, &cfg, &bins, &power, first, n);
        };

        // Start below zero and cross it, so the wrap is in play on negatives.
        let (k0, visible) = (-3i64, 12i64);
        run(&mut spectrum, k0, k0 + visible - 1);
        drain(&mut model);

        // Scroll a slab at a time, as `now` advancing does. Each frame the
        // guard overwrites the texel of the key just left behind.
        for n in 1..=6 {
            run(&mut spectrum, k0 + n, k0 + n + visible - 1);
            drain(&mut model);
        }

        // Widen inside the same rung: the far edge reaches back into the band
        // the guard has walked over, and `back` is what has to repaint it.
        let (first, last) = (k0 + 1, k0 + 6 + visible - 1);
        run(&mut spectrum, first, last);
        drain(&mut model);

        let ring = spectrum.spectrogram[0].ring.as_ref().expect("a ring");
        for key in first..=last {
            let x = ring.x_of(key);
            let got: Vec<Color32> = (0..h).map(|y| model[y * tex_w + x]).collect();
            assert_eq!(
                got,
                expect(key),
                "texel {x} should hold slab {key}; it holds {}",
                (first - 4..=last + 4)
                    .find(|&k| (0..h).map(|y| model[y * tex_w + x]).eq(expect(k)))
                    .map_or("no slab in range".to_string(), |k| format!("slab {k}")),
            );
        }

        // The far guard is the ONE texel meant to hold another slab's column:
        // `first - 1` carries a duplicate of `first`, which is what fills the
        // half texel the quad overruns past the oldest slab's leading edge.
        let x = ring.x_of(first - 1);
        let got: Vec<Color32> = (0..h).map(|y| model[y * tex_w + x]).collect();
        assert_eq!(got, expect(first), "the far guard at texel {x} does not duplicate slab {first}");
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
            style: ColumnStyle {
                rows: 3,
                bucket_bits: 0.1f64.to_bits(),
                scale_min_bits: 40.0f32.to_bits(),
                scale_span_bits: 48.0f32.to_bits(),
                color: ColumnColor::new(&SpectrumConfig::default()),
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
            style: ColumnStyle {
                rows: 3,
                bucket_bits: 0.1f64.to_bits(),
                scale_min_bits: 40.0f32.to_bits(),
                scale_span_bits: 48.0f32.to_bits(),
                color: ColumnColor::new(&SpectrumConfig::default()),
            },
            written_through: 0,
            oldest_valid: 0,
        };
        for key in -20i64..0 {
            assert!(ring.x_of(key) < 8, "key {key} fell outside the ring");
        }
    }

    /// The strip is drawn out to the now-line, but the newest column is always
    /// older than that — half an analysis window, by construction. Inside a
    /// ring the texels past the newest one are not empty: they hold what they
    /// carried a lap ago, which is a column from a whole window back, so a `u`
    /// allowed to run on paints the leading sliver with the far end of the
    /// window (dark while a note sounds, bright once it stops). Nothing past
    /// the newest slab's CENTRE may be sampled.
    #[test]
    fn the_leading_sliver_holds_the_newest_column_instead_of_reading_round_the_ring() {
        // A live window at the settings that make the sliver widest: a short
        // span, so slabs sit on the ladder's lowest rung and the analyzer's lag
        // spans several of them.
        let bucket = crate::AudioSpectrum::FFT_INTERVAL * LADDER_FLOOR_COLUMNS;
        let window = 2.0;
        let visible = (window / bucket) as usize; // 62 slabs
        let capacity = visible + 2;
        let layout = TexLayout {
            bucket,
            t_origin: 400.0,
            tex_span: visible as f64 * bucket,
            t0: 0.0,
            tn: 1.0,
            // Parked mid-ring, as it is for all but one lap in `capacity`.
            x0: 17.0,
            tex_w: (capacity * 2) as f32,
        };
        // The now-line: the newest column lags by half a Precise window, and
        // its slab has to finish before the next one starts.
        let now = layout.t_origin + layout.tex_span + 0.171;
        let texel = |u: f32| u * layout.tex_w - layout.x0; // texels into the run

        // What the run holds: `visible` columns, a guard column before them,
        // and past their newest a whole window of other laps' columns.
        let newest = texel(u_drawn(&layout, now));
        assert!(
            newest <= visible as f32 - 0.5 + 1e-4,
            "the sliver sampled {newest} texels in, past the newest column at {}",
            visible as f32 - 0.5,
        );
        // And it is the newest column it holds, not something short of it.
        assert!(newest >= visible as f32 - 0.5 - 1e-4, "held short of the newest column");
        // Worth pinning for, because an unheld mapping runs well past anything
        // a guard column or two could cover.
        let unheld = texel(u_of(&layout, now));
        assert!(unheld > visible as f32 + 4.0, "expected a multi-texel overrun, got {unheld}");
    }

    /// Everything BEFORE the hold is untouched by it: the drawn mapping is the
    /// plain one over the data, so the image still tracks the notes texel for
    /// texel, and the hold is a corner rather than a bend that creeps inward.
    #[test]
    fn holding_the_sliver_leaves_the_data_mapping_alone() {
        let bucket = 0.05;
        let layout = TexLayout {
            bucket,
            t_origin: 10.0,
            tex_span: 20.0 * bucket,
            t0: 0.0,
            tn: 1.0,
            x0: 3.0,
            tex_w: 44.0,
        };
        let hold = hold_time(&layout);
        assert!((hold - (layout.t_origin + layout.tex_span - 0.5 * bucket)).abs() < 1e-9);
        let mut t = layout.t_origin - 2.0 * bucket;
        while t <= hold {
            assert_eq!(u_drawn(&layout, t), u_of(&layout, t), "bent at {t}");
            t += bucket / 8.0;
        }
        // Past it, pinned — however far past, and however long the analyzer
        // stalls for.
        let pinned = u_of(&layout, hold);
        for t in [hold + 1e-6, hold + bucket, hold + 10.0] {
            assert_eq!(u_drawn(&layout, t), pinned, "ran on at {t}");
        }
    }

    /// The time -> texture mapping has to be a straight line, including across
    /// the slab boundary the newest column sits on. Clamping it to the edge
    /// texel pins it for part of every slab and lets it slide for the rest, and
    /// since these are VERTEX UVs that rescales the whole image once per slab —
    /// visible as the heatmap jittering. Which is why the one place `u` does
    /// stop ([`u_drawn`]) is a corner the mesh is SPLIT on, leaving no quad to
    /// interpolate across it and the data quad straight from end to end.
    #[test]
    fn the_time_to_texture_mapping_is_a_straight_line() {
        let bucket = 0.08;
        let layout = TexLayout {
            bucket,
            t_origin: 100.0,
            tex_span: 8.0 * bucket,
            t0: 0.0,
            tn: 1.0,
            // A run parked mid-ring, as it is for all but one lap in eight.
            x0: 5.0,
            tex_w: 16.0,
        };
        let step = bucket / 4.0;
        let at = |i: i32| u_of(&layout, layout.t_origin + i as f64 * step);

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

    /// A restart paints every column of the run at its own texel AND at its
    /// twin, duplicates the oldest as the guard outside the run, and leaves
    /// black wherever the run does not reach.
    ///
    /// The picture is only checkable here: once the pixels are in a texture
    /// they are opaque, so a misplaced column shows up as a scrambled
    /// spectrogram after every window resize rather than as a failure.
    #[test]
    fn a_restart_paints_every_column_and_its_twin() {
        const CAPACITY: usize = 6;
        const H: usize = 2;
        let tex_w = CAPACITY * 2;
        let style = style_for(H, 0.05, 1.0, &SWEEP_SCALE);
        let ring = SpectrogramRing::restarted(CAPACITY, style, 10);
        // One flat colour per column, so where each lands is readable.
        let shade = |i: usize| Color32::from_gray(10 * (i as u8 + 1));
        let pixels =
            restart_pixels(&ring, tex_w, H, 10, 12, |i| vec![shade(i); H]);

        for (i, key) in (10..=12).enumerate() {
            let x = ring.x_of(key);
            for row in 0..H {
                assert_eq!(pixels[row * tex_w + x], shade(i), "column {key} at {x}");
                assert_eq!(
                    pixels[row * tex_w + x + CAPACITY],
                    shade(i),
                    "column {key}'s twin",
                );
            }
        }
        // The guard duplicates the oldest slab, one texel before the run.
        let guard = ring.x_of(9);
        assert_eq!(pixels[guard], shade(0), "the guard column");
        assert_eq!(pixels[guard + CAPACITY], shade(0), "and its twin");
        // Everything the run does not reach stays silent rather than showing
        // whatever the allocation held.
        let painted: Vec<usize> = (9..=12)
            .flat_map(|key| [ring.x_of(key), ring.x_of(key) + CAPACITY])
            .collect();
        for (x, texel) in pixels.iter().take(tex_w).enumerate() {
            if !painted.contains(&x) {
                assert_eq!(*texel, Color32::BLACK, "unwritten column {x}");
            }
        }
    }

}
