//! The spectrogram heatmap's data path: the slab grid an incoming column folds
//! into, and the GPU's copy of that grid the fragment shader reads.
//!
//! The grid travels as DATA rather than as a picture — `capacity` slots of one
//! slab's stored dB bytes each — so a pitch zoom, a resize, a Level drag or a
//! palette change moves uniforms and never comes here. What a frame owes the
//! GPU is the run of slabs it draws and the few whose bytes have moved since
//! the last one; the read that turns those bytes into pixels lives in
//! [`harmonigraph_render`]'s shader.
//! [`panes::spectral::spectrogram`](crate::panes::spectral::spectrogram) is the
//! pane above it: the geometry the run is drawn over, the draw call, and the
//! cell colour the spectrum curve shares with it.
//!
//! This is the data-vs-drawing cut, not a time/pitch split — the pane side is
//! geometry and a draw call regardless of how the grid was folded, and
//! everything here is reachable with no `egui::Context` in sight (see the test
//! module).

use std::sync::Arc;

use harmonigraph_core::spectrogram::{db_of, BucketDb, DB_STEP};
use harmonigraph_core::spectrum::{BINS_PER_SEMITONE, SPECTRUM_BINS, SPECTRUM_MIN_MIDI};
use harmonigraph_render::{SpectrogramGrid, SpectrogramRead, SpectrogramShades};

use crate::panes::spectral::axes::{spectrogram_level_raw, PitchScale};
use crate::SpectrumConfig;
use harmonigraph_scene::Gradient;

/// Most time slabs a live window is ever cut into, whatever the pane's size —
/// and so, with the window, the FINEST slab any given moment can be drawn into.
/// That is what [`SpectrumHistory`](harmonigraph_core::SpectrumHistory) sizes its
/// tiers against: a column of age `a` is only on screen when the window is at
/// least `a` long, so it never needs storing finer than `a / LIVE_SLAB_CAP`.
///
/// Raising this is not free: the store's tiers have to keep up with it (see
/// [`SpectrumHistory::COARSE_COLUMNS`](harmonigraph_core::SpectrumHistory::COARSE_COLUMNS),
/// which must be at least as large), so this is what puts the store at 30 MB
/// where half the cap would cost 17. What the larger one buys is the SHORT
/// spans, which is where a halving still lands somewhere the data can tell
/// apart: a 12 s close-up is cut into 16 ms slabs here and 32 ms ones at half
/// this, against an 8 ms column rate. At the three-minute Span a fresh view
/// opens on it is 256 ms against 512, and both are far coarser than the data —
/// the cap still doubles the resolution there, but neither is drawing grain.
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
/// buckets between columns, and the grid's linear time axis assumes
/// evenly-spaced slabs — gaps there stretch the edge columns into flat streaks.
/// Derived from the FFT rate rather than restated, because the two must move
/// together and a stale copy of this number is exactly the bug that shows up as
/// duplicated columns scrolling past.
///
/// The WHOLE-SONG build's floor. The live grid gets the same guarantee from
/// [`live_slab`]'s ladder, whose lowest rung is two analysis intervals, so this
/// no longer floors it.
pub(crate) const MIN_BUCKET: f64 = crate::AudioSpectrum::FFT_INTERVAL * COLUMNS_PER_SLAB;

/// The live grid's finest rung, in analysis intervals — see [`live_slab`].
///
/// TWO, not one: the column grid and the slab grid share a period on the ladder
/// but not a phase (columns land on a sample counter, slabs on absolute time),
/// so at one column per slab a boundary falling mid-interval leaves some slabs
/// empty, and the uniform time axis then stretches the columns either side of an
/// empty slab into a flat streak. At two, a phase offset costs a slab one of its
/// columns and never both. It is [`COLUMNS_PER_SLAB`]'s job, done by the ladder
/// instead of by a margin.
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
/// aggregator's grid and re-uploads every slab — the entire per-frame rebuild,
/// for as long as the drag lasts. On the ladder it moves only when the Span
/// crosses a doubling.
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
/// pane buys nothing, so the picture can hold half the slabs the depth axis has
/// pixels. What that gives up depends on the Span, and the two ends differ in
/// kind. At a close-up — a 12 s window cuts into 16 ms slabs — the slab is well
/// under the 171 ms analysis window, so the stepping loses detail the FFT never
/// had. At the three-minute Span a fresh view opens on, the slab is 256 ms and
/// the CAP is what sets the resolution: those slabs merge detail the FFT did
/// resolve. That is what a long Span is for rather than a flaw in it — the
/// shape of a piece instead of the grain of a phrase — and zooming in is what
/// asks for the grain back.
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

/// Stored steps the power mean falls per halving of the summed weight — the
/// byte-form of `(10 / ROW_MEAN_ORDER) * log10(..)`, with `log10` reached
/// through `log2` and the dB then divided by the store's own step.
///
/// Uploaded as [`SpectrogramRead::mean_steps`], which is the only thing that
/// reads it: the mean itself is a fragment's work.
const ROW_MEAN_STEPS: f32 = 10.0
    / (crate::panes::spectral::spectrogram::ROW_MEAN_ORDER as f32
        * harmonigraph_core::spectrogram::DB_STEP
        * std::f32::consts::LOG2_10);

/// The weight a bucket carries in the power mean, indexed by how many stored
/// steps BELOW the loudest bucket of its row it sits.
///
/// Relative to the row's own loudest rather than absolute, which is what keeps
/// the arithmetic in range: at order 4 an absolute weight spans `10^-48` across
/// the stored dB range and flushes to zero in an `f32`, where relative weights
/// run from exactly 1 downward and their sum can never fall below 1.
///
/// Behind an `Arc` because it is a uniform now — every frame hands the same
/// allocation to [`SpectrogramRead::weight`], so the table is built once for the
/// process and a frame's share of it is a refcount.
static ROW_WEIGHT: std::sync::LazyLock<Arc<[f32; 256]>> = std::sync::LazyLock::new(|| {
    Arc::new(std::array::from_fn(|j| {
        let db = j as f32 * harmonigraph_core::spectrogram::DB_STEP;
        10f32.powf(-0.1 * crate::panes::spectral::spectrogram::ROW_MEAN_ORDER as f32 * db)
    }))
});

/// Where the visible slabs sit in absolute time — everything the mesh's slab
/// coordinate needs.
///
/// One shape for both builds: the live window's run and the whole-song fold's
/// differ in how they were folded and in nothing the geometry can see.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TexLayout {
    /// Seconds one slab spans. Carried here because every mapping below reads it
    /// alongside the rest: a slab's width is part of describing where the slabs
    /// sit, not a separate fact about them.
    pub(crate) bucket: f64,
    /// Absolute time at the left edge of the first visible slab.
    pub(crate) t_origin: f64,
    /// Seconds the visible slabs span.
    pub(crate) tex_span: f64,
}

/// A gradient reduced to the knobs that actually decide a texel — the cases
/// where two different gradients name one picture, and so have to be one key.
///
/// There are two, one per axis, and each is an axis collapsing to a point:
///
/// - **No chroma anywhere.** At `chroma` and `chroma_ramp` both 0, `chroma_at`
///   is 0 at every level, so the absolute chroma is 0 whatever the gamut holds,
///   and Oklab's `a` and `b` are `c * cos(h)` and `c * sin(h)` — identically 0.
///   Every texel is the same grey at every angle, and the HUE pair decides
///   nothing. That is the Mono preset.
/// - **A brightness pair closed on either end of the `L*` axis.** `HUE_FLOOR` is
///   0 at both 0 and 100, so `chroma_of` answers 0 for every fraction and every
///   hue, and `oklab_srgb` is black at 0 and white at 100 whichever way the arc
///   runs. Here the CHROMA pair decides nothing either, so both fold.
///
/// Both are drags a reader is dialling their way OUT of, which is what makes
/// them worth folding rather than curiosities: the bars that reach them are the
/// bars that fix them. Every preset opens with silence at `L*` 0 and
/// `Spread::snapped` rounds to whole units, so the second is landed on exactly
/// and in one gesture rather than by luck.
/// `the_lut_key_folds_two_gradients_that_draw_one_picture` holds both
/// directions of both.
///
/// **Here and not in [`Gradient::sanitized`]**, which is the tempting place and
/// the wrong one. Sanitize's answer is what the BARS read back and write, so
/// folding there would snap the hue home under a pointer dragging it — and a hue
/// dialled at no chroma is a real setting, the one a picture opens on when the
/// chroma bar is next raised off 0. `SpectrogramPreset::Mono` writes one
/// deliberately. The key's question is not "is this legal" but "does this decide
/// a texel", and only the key may answer it.
fn what_decides_a_texel(g: Gradient) -> Gradient {
    let toneless = g.chroma == 0.0 && g.chroma_ramp == 0.0;
    // The ends of the axis, and only the ends: a pair closed anywhere BETWEEN
    // them still draws a colour, so the hues decide a texel there. `sanitized`
    // is what makes the equality safe to write — it holds `lightness` inside
    // 0..=100 and keeps a -0.0 out of the ramp, so a pair at the wall lands on
    // exactly these two numbers.
    let unlit = g.lightness_ramp == 0.0 && (g.lightness == 0.0 || g.lightness == 100.0);
    match (toneless, unlit) {
        (_, true) => Gradient { hue_start: 0.0, hue_span: 0.0, chroma: 0.0, chroma_ramp: 0.0, ..g },
        (true, false) => Gradient { hue_start: 0.0, hue_span: 0.0, ..g },
        (false, false) => g,
    }
}

/// Levels the ramp is sampled at, and so entries in the table the shader reads.
///
/// A table indexed by level cannot be exact for every row at once — a row's
/// offset is continuous, so its levels fall between samples wherever they like,
/// and no sample count stops one landing on the wrong side of a channel's
/// rounding boundary. What a count buys is how FAR wrong: the ramp's segments
/// span at most 255 levels of an 8-bit channel, so 1024 samples per segment put
/// every texel within an eighth of a level of the mapping's own value, and the
/// only texels that then differ are those whose true value sat within that
/// eighth of a boundary. Swept across every ramp, dB window and tilt, that is
/// 1.25% of texels differing, always by exactly one level of one channel.
///
/// That is the same order as the store's OWN quantization, which moves a colour
/// by about a level at the default window (half a dB step of a 60 dB range,
/// across a 255-level ramp) and was settled by eye against a sixteen-bit store
/// — see `quantizing_a_bucket_does_not_move_its_colour`, which is the same
/// judgement made one layer down. Exactness would need a table per ROW, which is
/// a megabyte of texture per pane and slower than the arithmetic it replaces.
pub(crate) const SHADES: usize = 4096;

/// The gradient's table as the shader reads it, and the gradient it stands for.
struct ShadeLut {
    /// The FOLDED gradient (see [`what_decides_a_texel`]) the entries were built
    /// for. Two gradients that fold together draw one picture, so one table
    /// serves both; the entries themselves come off the gradient as dialled.
    gradient: Gradient,
    /// Bumped on every rebuild, which is what the GPU copy is keyed on.
    generation: u64,
    lut: Arc<Vec<[u8; 4]>>,
}

/// The GPU's copy of the slab grid, as the CPU last described it, plus the
/// gradient table beside it.
///
/// One per drawing surface. It holds no GPU object of its own — the buffers
/// live in the render crate, keyed by pane — only the statement of what those
/// buffers contain, which is what lets a frame send a delta instead of a grid.
#[derive(Default)]
pub(crate) struct GpuGrid {
    /// A new value makes the next frame's copy a full upload of the whole run.
    /// Bumped whenever nothing can be said about what the GPU holds: a first
    /// frame, a fresh context ([`GpuGrid::release`]), a capacity change.
    generation: u64,
    /// The run last handed to a callback, and which of its keys the GPU is being
    /// asked to write.
    ///
    /// The invariant the delta rests on: **every key in this run has its bytes
    /// in its slot on the GPU.** A full upload establishes it — the render crate
    /// writes every slab of the run whenever [`generation`](Self::generation)
    /// moves — and the dirty writes maintain it, so a refold, a rung crossing, a
    /// gap and a backward jump all fall out of the byte comparison in
    /// [`SentRun::moved`] rather than each needing a reason of its own.
    /// `the_gpu_grid_equals_a_full_upload_after_any_sequence` is what holds it.
    sent: Option<SentRun>,
    lut: Option<ShadeLut>,
    /// Times this surface has handed the grid over to be rebuilt rather than
    /// patched — a rate the performance overlay reads beside the aggregator's
    /// refolds, for the reason the refolds are counted: a full upload is CORRECT
    /// and costs megabytes, so a delta that has quietly stopped working draws
    /// the right picture and says nothing.
    full_uploads: u32,
}

/// One run of slabs as it was handed to the GPU.
struct SentRun {
    /// Which columns it was folded from — a match means this run is still the
    /// picture, and nothing is folded at all.
    key: RunKey,
    /// The oldest visible slab's key; the run is contiguous from there.
    first_key: i64,
    /// Slots the GPU buffer holds. The run must fit inside it, so a key's slot
    /// (`key mod capacity`) names one slab at a time.
    capacity: usize,
    /// Slab-major stored dB, [`SPECTRUM_BINS`] to a slab — the aggregator's own
    /// bytes, shared with the callback rather than copied into it.
    run: Arc<Vec<u8>>,
    /// Keys whose slot the GPU is being asked to write while this run stands.
    ///
    /// Re-sent every frame rather than cleared once it has been handed over,
    /// because a frame's callback is not certain to run: egui drops a callback
    /// whose clip rect is empty, and a write dropped with it would leave a slot
    /// holding a slab this run says it does not. Writing the same bytes again
    /// costs a slab a frame; a slot that silently disagrees with the run is a
    /// wrong column that no later frame repairs.
    dirty: Vec<i64>,
    /// Where these slabs sit in absolute time, so a frame that folds nothing
    /// still knows where to draw them.
    layout: TexLayout,
}

impl SentRun {
    /// The keys of `run` whose bytes are not already in their slot: the ones
    /// outside this run's key range, and the ones whose bytes have moved.
    ///
    /// A key ENTERING the run is outside the range and so named by construction
    /// — its slot may still hold `key - capacity`, a whole lap back — which is
    /// what makes a byte comparison a complete answer rather than a fast path
    /// needing a list of exceptions beside it.
    fn moved(&self, first_key: i64, run: &[u8]) -> Vec<i64> {
        let held = self.run.len() / SPECTRUM_BINS;
        fn slab(bytes: &[u8], j: usize) -> &[u8] {
            &bytes[j * SPECTRUM_BINS..(j + 1) * SPECTRUM_BINS]
        }
        (0..run.len() / SPECTRUM_BINS)
            .filter(|&j| {
                let at = first_key + j as i64 - self.first_key;
                !(0..held as i64).contains(&at) || slab(&self.run, at as usize) != slab(run, j)
            })
            .map(|j| first_key + j as i64)
            .collect()
    }
}

impl GpuGrid {
    /// The layout of the run already on the GPU, if it was folded from these
    /// columns — the frame then draws it without touching the store.
    fn hit(&self, key: &RunKey) -> Option<TexLayout> {
        self.sent.as_ref().filter(|sent| sent.key == *key).map(|sent| sent.layout)
    }

    /// Take a freshly folded run as the one to draw, working out what the GPU
    /// has to be told about it.
    fn accept(
        &mut self,
        key: RunKey,
        first_key: i64,
        capacity: usize,
        run: Vec<u8>,
        layout: TexLayout,
    ) {
        debug_assert!(
            run.len() / SPECTRUM_BINS <= capacity,
            "a run of {} slabs puts two keys in one of {capacity} slots",
            run.len() / SPECTRUM_BINS,
        );
        let dirty = match &self.sent {
            // The buffer is the same one, so what it holds is known slab by
            // slab and only the slabs that moved need writing.
            Some(sent) if sent.capacity == capacity => sent.moved(first_key, &run),
            // A different capacity is a different buffer — the slot a key lands
            // in is `key mod capacity`, so the whole mapping moves — and no
            // previous run at all is a context that has just been rebuilt.
            // Either way the copy is made from the run rather than patched.
            _ => {
                self.generation += 1;
                self.full_uploads += 1;
                Vec::new()
            }
        };
        self.sent = Some(SentRun { key, first_key, capacity, run: Arc::new(run), dirty, layout });
    }

    /// The grid a frame hands to the callback, or `None` before anything has
    /// been folded into it.
    fn grid(&self) -> Option<SpectrogramGrid> {
        let sent = self.sent.as_ref()?;
        Some(SpectrogramGrid {
            generation: self.generation,
            capacity: sent.capacity as u32,
            bins: SPECTRUM_BINS as u32,
            first_key: sent.first_key,
            run: sent.run.clone(),
            dirty: sent.dirty.clone(),
        })
    }

    /// The gradient table a frame hands to the callback, built here on the
    /// first sight of a gradient that draws a different picture.
    ///
    /// Only the gradient decides an entry: the dB window and the tilt reach a
    /// texel through the level it is looked up at, which is a uniform.
    fn shades(&mut self, cfg: &SpectrumConfig) -> SpectrogramShades {
        let gradient = what_decides_a_texel(cfg.spectrogram_gradient.sanitized());
        if self.lut.as_ref().is_none_or(|held| held.gradient != gradient) {
            let generation = self.lut.as_ref().map_or(0, |held| held.generation) + 1;
            let lut = (0..SHADES)
                .map(|i| {
                    crate::panes::spectral::spectrogram::cell_color(
                        cfg.spectrogram_gradient,
                        (i as f32 + 0.5) / SHADES as f32,
                    )
                    .to_array()
                })
                .collect();
            self.lut = Some(ShadeLut { gradient, generation, lut: Arc::new(lut) });
        }
        let held = self.lut.as_ref().expect("built above when the fold moved");
        SpectrogramShades { generation: held.generation, lut: held.lut.clone() }
    }

    /// Full uploads taken since this surface was opened — see the field.
    pub(crate) fn full_uploads(&self) -> u32 {
        self.full_uploads
    }

    /// Slabs in the run the GPU holds, or 0 before anything has been folded.
    #[cfg(test)]
    pub(crate) fn run_slabs(&self) -> usize {
        self.sent.as_ref().map_or(0, |sent| sent.run.len() / SPECTRUM_BINS)
    }

    /// Forget what the GPU holds, so the next frame uploads the whole run.
    ///
    /// The generation is what carries that to the callback: it keys the copy, so
    /// a bump is the rebuild. The run goes with it because the delta is computed
    /// against it, and a run kept across a context change would have the next
    /// frame patch two slabs of a buffer that was never written.
    pub(crate) fn release(&mut self) {
        self.generation += 1;
        self.sent = None;
    }
}

/// The pane's geometry and settings for one frame — half of a [`Plan`]'s
/// inputs, the half that has nothing to do with the store.
///
/// Sized in PIXELS, not points:
/// [`Axes`](crate::panes::spectral::axes::Axes) is laid out in egui points and the
/// picture is stretched over that rect by the GPU, so sizing it in points reads
/// it at the display's density divided by the scale factor — half the resolution
/// in each axis on a 2x screen, for a heatmap softer than the pane it sits in.
/// The label glyphs oversample by the same factor for the same reason (see
/// `text::draw_glyphs`).
pub(crate) struct PaneView {
    /// Physical pixels per egui point.
    pub(crate) ppp: f32,
    /// Points across the pitch axis, and across the FAR region of the depth
    /// axis — which the heatmap does not own: the roll's ribbons draw over the
    /// same region on the same time axis, and it is the spectrum CURVE that has
    /// the near share to itself.
    pub(crate) pitch_len: f32,
    pub(crate) depth_len: f32,
    /// Seconds the far region spans.
    pub(crate) window: f64,
    pub(crate) scale: PitchScale,
    pub(crate) cfg: SpectrumConfig,
    /// The whole-song (offline playhead) layout rather than the live window.
    pub(crate) whole: bool,
}

/// Which stored columns a frame draws — the other half of a [`Plan`]'s inputs.
///
/// The whole-song arm counts the UNTRIMMED set, while the fold reads
/// [`WholeSong::drawn_columns`](crate::WholeSong::drawn_columns) — so `start`
/// reaches the key through nothing at all, and `span` only through `bucket`,
/// which is many-to-one wherever [`MIN_BUCKET`] binds. Two windows on one
/// column set can therefore mint the same key for different runs.
///
/// What makes that safe rather than the stale-key bug it looks like is that a
/// [`WholeSong`](crate::WholeSong) is built once per render, before the frame
/// loop, and only its `roll` is written afterwards — so `start` and `span` are
/// constants for the life of every key minted from them. It is safe by that
/// fact and not by the key, which is why the fact is written down: a
/// `WholeSong` that changed window mid-render would draw the previous
/// window's grid at the previous window's geometry, and no assertion here
/// would see it.
pub(crate) struct Columns {
    /// The oldest in-window column; it advances as the window scrolls one off
    /// the far end. Whole-song draws its entire fixed set, so 0.
    pub(crate) first: usize,
    pub(crate) len: usize,
    /// The newest column's time, which moves whenever a fresh column arrives —
    /// catching one even in a saturated store, where the count holds steady.
    pub(crate) newest: f64,
}

/// Which columns a run was folded from, and how. Equal keys mean the run on the
/// GPU is still the one to draw, so the frame folds nothing and re-sends what it
/// holds.
///
/// Staleness-safe by construction — every way the RUN can change moves a field:
/// a fresh column moves `newest_bits` (even in a saturated store, where the
/// count holds), the oldest column scrolling out of the window moves `first`,
/// and the Span crossing a ladder rung moves `bucket_bits`. Floats compare by
/// bit pattern so equality is exact and free of NaN quirks.
///
/// What is deliberately NOT here is everything that decides how the run is READ
/// — the rows, the pitch range, the dB window, the gradient. Those are uniforms
/// now, so a zoom or a palette drag draws the same bytes a different way, and a
/// key that watched them would re-fold the store on every frame of a gesture
/// that cannot move a slab. The buffer's `capacity` is out for the same reason:
/// it sizes the GPU's copy, not the fold, and a change to it is answered where
/// the copy is made ([`GpuGrid::accept`]).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RunKey {
    first: usize,
    cols_len: usize,
    newest_bits: u64,
    bucket_bits: u64,
    whole: bool,
}

/// What this frame's heatmap needs folded, and the key that says whether the
/// grid on the GPU already IS it.
///
/// Pure, and deliberately so. Every cliff this pipeline has fallen off has been
/// in this arithmetic — a slab against the analyzer's lag, a window against the
/// store's finest tier — and deciding it apart from the fold means it can be
/// checked without a GPU, a texture or a frame. See
/// `no_cache_layer_falls_back_as_the_window_scrolls`.
pub(crate) struct Plan {
    /// The picture's pitch-axis resolution in device pixels, which is what the
    /// shader reads its rows at. It bounds nothing on the GPU — the grid is
    /// bucket-space data — so the pane's own pixels are the whole of it.
    pub(crate) rows: usize,
    /// A time slab's width, in seconds.
    bucket: f64,
    /// Slabs the GPU's copy holds, and so the most the aggregator keeps folded.
    ///
    /// Sized off the PANE rather than off the window, which is what makes it
    /// hold still: the widest run a window can have at this slab width is
    /// `target_cols` of them, whatever the Span is doing inside that rung, so a
    /// drag never resizes the buffer. Unused by the whole-song build, whose run
    /// is its own capacity.
    capacity: usize,
    first: usize,
    pub(crate) key: RunKey,
}

impl Plan {
    pub(crate) fn new(view: &PaneView, columns: &Columns) -> Plan {
        // One row per device pixel of the pitch axis, and one slab per pixel of
        // the depth axis, under the cap the slab count takes.
        let rows = ((view.pitch_len * view.ppp).round() as usize).max(2);
        let depth_px = (view.depth_len * view.ppp).round();
        // Whole-song spans an entire take, so it needs a higher cap than the
        // live window.
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
        let key = RunKey {
            first: columns.first,
            cols_len: columns.len,
            newest_bits: columns.newest.to_bits(),
            bucket_bits: bucket.to_bits(),
            whole: view.whole,
        };
        Plan { rows, bucket, capacity: target_cols + RING_HEADROOM, first: columns.first, key }
    }
}

/// How far past the visible range the edge rows reach, as a fraction of it: one
/// bucket, so the filtering carries the range cleanly to the picture's own
/// edges, and never more than half the range on top of it.
///
/// The row geometry it belongs to is the shader's; this is the one term of it
/// that needs the analyzer's constants and the pane's zoom in the same place.
fn pitch_margin(span: f32) -> f32 {
    (1.0 / BINS_PER_SEMITONE as f32 / span).min(0.5)
}

/// The level mapping in the form the shader carries it:
/// `(level0, per stored step, per MIDI)`, before its 0..1 clamp.
///
/// Read off [`spectrogram_level_raw`] at three points rather than written out
/// again from its algebra — it is affine in the dB and in the pitch, so this is
/// the mapping itself whatever it is written as, and the dB window, the tilt and
/// their pivot stay the pane's business.
fn level_affine(cfg: &SpectrumConfig) -> (f32, f32, f32) {
    let db0 = db_of(0);
    let level0 = spectrogram_level_raw(cfg, db0, 0.0);
    (
        level0,
        spectrogram_level_raw(cfg, db0 + DB_STEP, 0.0) - level0,
        spectrogram_level_raw(cfg, db0, 1.0) - level0,
    )
}

/// The scalars a fragment reads the grid through — the row geometry, the level
/// mapping and the mean's weights, for a picture `rows` pixels up the pitch
/// axis.
pub(crate) fn read_of(view: &PaneView, rows: usize) -> SpectrogramRead {
    let (level0, level_per_step, level_per_midi) = level_affine(&view.cfg);
    SpectrogramRead {
        min_midi: view.scale.min_midi,
        span: view.scale.span,
        margin: pitch_margin(view.scale.span),
        rows: rows as u32,
        spectrum_min_midi: SPECTRUM_MIN_MIDI,
        bins_per_semitone: BINS_PER_SEMITONE as f32,
        level0,
        level_per_step,
        level_per_midi,
        mean_steps: ROW_MEAN_STEPS,
        weight: ROW_WEIGHT.clone(),
    }
}

/// Fold the run a [`Plan`] describes and hand it to the surface's GPU mirror.
/// Returns where the visible slabs sit in time, or `None` if the fold came out
/// too short to draw.
pub(crate) fn build(
    spectrum: &mut crate::AudioSpectrum,
    whole: Option<&crate::WholeSong>,
    surface: usize,
    plan: &Plan,
    view: &PaneView,
) -> Option<TexLayout> {
    let bucket = plan.bucket;
    // Aggregate the in-window columns into one slab per depth pixel by a FIXED
    // time grid, MAX within each slab (which keeps a short note's peak and pins
    // it against the scroll — see `aggregate_slabs`). Slabs stay in BUCKET
    // space: the rows read them in the fragment shader, so the fold is blind to
    // the pitch axis and a zoom or pan of it re-reads this grid instead of
    // re-walking the store.
    let (centers, power) = match whole {
        // Offline whole-song: a fixed column set, folded once per render — a
        // plain batch aggregate, over the columns the depth axis can draw rather
        // than every column the take holds. That trim is what bounds the RUN
        // (see [`WholeSong::drawn_columns`](crate::WholeSong::drawn_columns)):
        // `bucket` is cut for the window, so folding a longer take's whole
        // column set would spend slabs on time no pixel shows.
        Some(ws) => aggregate_slabs(ws.drawn_columns(view.window), bucket),
        // Live: fold only the new column(s) into the kept slab grid instead of
        // rescanning the whole window every rebuild. `history` and the
        // aggregator are disjoint fields of `spectrum`.
        None => {
            let hist = &spectrum.history;
            let agg = spectrum.spectrogram[surface].agg.get_or_insert_with(SpectrogramAgg::new);
            agg.window(hist, plan.first, bucket, plan.capacity)
        }
    };
    let w = centers.len();
    if w < 2 {
        return None;
    }
    // The run covers absolute time `[t_origin, t_origin + w*bucket]` — the
    // oldest slab's start to the newest slab's end. Slab centres sit half a slab
    // inside each, so a time places itself at `(t - t_origin) / bucket` slabs.
    let t_origin = centers[0] - 0.5 * bucket;
    let tex_span = w as f64 * bucket;
    if tex_span < 1e-9 {
        return None;
    }
    let first_key = (centers[0] / bucket).floor() as i64;
    // The whole-song run is its own capacity: it is folded once for the render
    // and never scrolls, so there is no lap for a key to come round on.
    let capacity = if view.whole { w } else { ring_capacity(plan.capacity, w) };
    let layout = TexLayout { bucket, t_origin, tex_span };
    spectrum.spectrogram[surface].gpu.accept(plan.key.clone(), first_key, capacity, power, layout);
    Some(layout)
}

/// The run this frame draws, and where it sits in time: the one already on the
/// GPU when the plan's key still names it, and a fresh fold otherwise.
pub(crate) fn run_for(
    spectrum: &mut crate::AudioSpectrum,
    whole: Option<&crate::WholeSong>,
    surface: usize,
    plan: &Plan,
    view: &PaneView,
) -> Option<TexLayout> {
    match spectrum.spectrogram[surface].gpu.hit(&plan.key) {
        Some(layout) => Some(layout),
        None => build(spectrum, whole, surface, plan, view),
    }
}

/// The grid and the gradient table this frame's callback carries.
///
/// `None` before anything has been folded into the surface, which is the same
/// frame [`run_for`] answers `None` on.
pub(crate) fn frame_data(
    spectrum: &mut crate::AudioSpectrum,
    surface: usize,
    cfg: &SpectrumConfig,
) -> Option<(SpectrogramGrid, SpectrogramShades)> {
    let gpu = &mut spectrum.spectrogram[surface].gpu;
    let grid = gpu.grid()?;
    Some((grid, gpu.shades(cfg)))
}

/// Group `columns` (oldest first) into time-slabs of `bucket` seconds, taking
/// each bucket's MAX over the columns that landed in the slab. Returns each
/// slab's center time and a flat slab-major grid of whole spectra
/// (`slabs * SPECTRUM_BINS`).
///
/// The slabs stay in BUCKET space — the display rows read them in the fragment
/// shader, not here. That order is what makes the grid a thing worth keeping: it
/// depends on nothing but the slab width, so the pitch axis can zoom, pan or
/// change its row count and the fold is untouched. The two axes still aggregate
/// DIFFERENTLY, and the asymmetry is still the point.
/// Time takes a plain max, because a spectrogram cell answers "was there
/// anything here" and averaging a brief loud column with the silence either
/// side answers "not much" — a slab spans a few columns of a heavily
/// OVERLAPPED stream (95% at the live rate, and 84% even where
/// [`WholeSong`](crate::WholeSong) stretches the hop for a three-minute take),
/// so the max is over near-copies of one measurement rather than over a
/// distribution. Pitch takes a power mean instead, because a row zoomed out
/// spans a dozen INDEPENDENT buckets, and the max of a dozen samples of a noise
/// floor is a function of how many were drawn.
///
/// Maxing the buckets FIRST and reading the rows from the result is not the
/// same picture as reading each column and maxing the answers — a mean over a
/// slab's maxed buckets can only come out at or above the max of the per-column
/// means, and a lerp across a maxed pair is not the max of the lerps. The
/// difference lives entirely inside one slab, between columns that are
/// near-copies of one measurement (the same overlap argument as the max
/// itself), where it is a fraction of a dB; what it buys is the fold's
/// independence from the rows, which is the whole cost of a pitch gesture.
///
/// The slab a column lands in is `floor(time / bucket)` — a function of
/// absolute time alone, so it doesn't move as columns scroll off the far end
/// of the run. That, plus MAX (rather than dropping samples), is what stops a
/// short, bright note from flickering: its peak is kept and stays in one
/// slowly-scrolling slab instead of blinking in and out with the sampling.
fn aggregate_slabs<'a>(
    columns: impl Iterator<Item = &'a crate::SpectrogramColumn>,
    bucket: f64,
) -> (Vec<f64>, Vec<BucketDb>) {
    let mut grid = SlabGrid::default();
    for col in columns {
        grid.fold(col, bucket);
    }
    (grid.centers, grid.power)
}

/// The growing slab grid the spectrogram is read out of: `centers[i]` is
/// slab `i`'s center time and `power` is the flat slab-major
/// `[slab][source bucket]` MAX grid (`slab * SPECTRUM_BINS + bucket`).
/// [`fold`](SlabGrid::fold) is the single per-column step both
/// [`aggregate_slabs`] (batch, from scratch) and [`SpectrogramAgg`]
/// (incremental, live) drive — so the two can never disagree.
///
/// A slab is a whole SPECTRUM, not a column of display rows. The rows read the
/// grid in the fragment shader, so nothing here knows the pitch scale or the
/// row count — which is what lets a pitch drag re-read a grid that is already
/// folded instead of re-walking the store on every frame of itself.
///
/// Held in the same dB bytes the columns are stored in: MAX is order-preserving
/// under the encoding, so aggregating in it is exact, and folding a column is
/// an elementwise byte max the compiler vectorizes.
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
    fn fold(&mut self, col: &crate::SpectrogramColumn, bucket: f64) -> bool {
        let nb = SPECTRUM_BINS;
        let key = (col.time / bucket).floor() as i64;
        let forward = match self.cur_key {
            Some(k) if k == key => true,
            // A slab with no columns in it STILL gets a row, so the grid stays
            // one row per slab of elapsed time. The time axis is uniform —
            // `slab_at` maps time linearly across `w * bucket` — so skipping an
            // empty slab makes the rows either side of it neighbouring slabs,
            // and the quad then stretches that pair over
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
        // Branchless, so the loop vectorizes: with the compare written as a
        // branch this is the whole cost of a refold, byte by byte.
        for (kept, &fresh) in self.power[base..].iter_mut().zip(col.db.iter()) {
            *kept = (*kept).max(fresh);
        }
        forward
    }
}

/// Live-only incremental spectrogram aggregation. `aggregate_slabs` re-scans
/// EVERY in-window column on each ~20 Hz rebuild — O(columns-in-window), which
/// grows with the roll Span and is the residual creep the run key didn't
/// remove. This keeps the slab grid across frames instead: a rebuild folds only
/// the newly-arrived column(s) and drops the scrolled-out front, so its cost is
/// O(new columns), independent of how much history has accumulated.
///
/// The grid's one layout input is the slab width. The pitch axis — its range,
/// its row count, which buckets a row reads — is applied downstream, in the
/// fragment shader, so a pitch zoom or pan re-reads a grid that is already folded
/// and never comes here. That is not incidental: a pitch drag moves its scale
/// on every frame of itself, and a fold that applied the rows would re-walk
/// the whole retention on each of those frames — 46 ms at a 12 s Span on a
/// full-height 2x pane, and past 100 ms at long ones, measured.
///
/// It reproduces `aggregate_slabs` over the columns AS THEY ARRIVED: the shared
/// [`SlabGrid::fold`] gives identical slab values, and the window is served from
/// the same first slab batch would start at. A slab-width change (the Span
/// crossing a ladder rung), a backward transport jump, or a window that jumped
/// outside the kept grid falls back to a full rebuild — each of which is just
/// `aggregate_slabs` again, so correctness never rides on the fast path alone;
/// and the refold is an elementwise byte max per column, a few milliseconds
/// even over the whole store. The offline whole-song path does NOT use this
/// (its column set is fixed and already cached after the first frame).
///
/// **A folded slab is never recomputed, even when the store re-writes the
/// columns behind it.** Columns arrive in time order, so no future column can
/// land in a slab the newest one has already passed: the slab is FINAL the
/// moment it is behind the front. What is not final is the STORE — past
/// `SpectrumHistory`'s finest tier, columns are MAX-merged in pairs and re-timed
/// to their midpoint as they age. Re-deriving an old slab from the merged store
/// therefore answers a slightly different question than folding the raw columns
/// did — a merged pair lands in one slab rather than straddling two, smearing
/// its energy across a boundary the raw columns respected — so the two
/// disagree, and this keeps what it folded from the finer data rather than
/// re-reading the coarser.
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
    /// Time of the newest column already folded; the next update folds only
    /// columns past it.
    last_time: f64,
    /// How many full rebuilds have been taken. The fast path is the whole point
    /// of this type, and every condition guarding it can only ever ADD a reason
    /// to fall back — so without a count, a guard that quietly always fires
    /// would still pass every correctness test here and simply hand back the
    /// `aggregate_slabs` cost this exists to avoid.
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
    /// once a widening drag trips it, every frame of it rebuilds, however
    /// long the aggregator had been running before. It reads on the overlay as a
    /// refold rate pinned at the frame rate for the length of the drag.
    fn rebuild(
        &mut self,
        history: &crate::SpectrumHistory,
        first: usize,
        bucket: f64,
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
            self.grid.fold(col, bucket);
        }
        self.bucket_bits = bucket.to_bits();
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
        keep: usize,
    ) -> (Vec<f64>, Vec<BucketDb>) {
        let target = history.get(first).map(|c| (c.time / bucket).floor() as i64);
        let newest = history.back().map_or(f64::NEG_INFINITY, |c| c.time);
        let layout_same = self.bucket_bits == bucket.to_bits();
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
            self.rebuild(history, first, bucket, keep);
        } else {
            // Fold only columns newer than the last we folded.
            let start = history.partition_point(|c| c.time <= self.last_time);
            let mut forward = true;
            for col in history.iter_from(start) {
                if !self.grid.fold(col, bucket) {
                    forward = false;
                    break;
                }
                self.last_time = col.time;
            }
            if !forward {
                // A mid-stream backward jump broke the grid; rebuild clean.
                self.rebuild(history, first, bucket, keep);
            }
        }
        self.view(history, first, bucket, target, keep)
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
    /// The grid keeps what the GPU's COPY keeps — `keep` slabs, sized off the pane —
    /// rather than only what the window currently shows, and everything before
    /// the window's first slab is sliced off here rather than dropped. Two
    /// things need that slack.
    ///
    /// A Span GROWING reaches back to slabs it did not want a frame ago. Trimmed
    /// flush to the window, every frame of a widening drag would ask for a slab
    /// just discarded and rebuild; holding what the GPU's copy can hold means
    /// the whole rung is already folded.
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
        target: Option<i64>,
        keep: usize,
    ) -> (Vec<f64>, Vec<BucketDb>) {
        let nb = SPECTRUM_BINS;
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

        // Drop what has fallen out of the copy's reach. Centers run one per slab
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
            for (kept, &fresh) in power[..nb].iter_mut().zip(c.db.iter()) {
                *kept = (*kept).max(fresh);
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

/// The slab coordinate of an absolute time: slabs from the first visible slab's
/// LEFT EDGE, which is what a vertex carries to the shader.
///
/// A straight line in `t`, with no clamping, and it must stay that way: this is
/// a vertex attribute, so the scale every fragment reads at is interpolated
/// between the quad's corners. Bending or pinning either end changes the whole
/// picture's scale, and doing it for only part of each slab — which is what
/// clamping to the newest slab does, since `now` crosses its centre mid-slab —
/// makes the heatmap twitch once per slab.
fn slab_at(layout: &TexLayout, t: f64) -> f32 {
    ((t - layout.t_origin) / layout.bucket) as f32
}

/// The newest time the run has data for: the CENTRE of its last slab, which is
/// the last point the slab coordinate may reach.
///
/// The strip is drawn out to the now-line, but the newest column is always
/// older than that — it is stamped at the middle of the window it measured, so
/// a healthy stream still lags by half an analysis window (171 ms on Precise),
/// and it lands in a slab that then has to finish before the next begins. What
/// fills that sliver has to come from the newest column, because it is the only
/// thing the analyzer has said about the stretch.
pub(crate) fn hold_time(layout: &TexLayout) -> f64 {
    layout.t_origin + layout.tex_span - 0.5 * layout.bucket
}

/// The slab coordinate for a time on the DRAWN strip: [`slab_at`] out to
/// [`hold_time`], and pinned there past it.
///
/// The shader clamps its taps into the run, which covers half a slab at the far
/// end — the strip stops at the oldest slab's leading edge, half a slab before
/// its centre. The NEWEST end overruns by the analyzer's lag instead, which is
/// several slabs once the window is short enough to sit on [`live_slab`]'s
/// lowest rung, and a clamp there would hold only part of each slab. So the
/// sliver is filled by pinning the coordinate.
///
/// Pinning is safe HERE, at a corner the mesh is split on, and nowhere else:
/// see the split in
/// [`draw_spectrogram`](crate::panes::spectral::spectrogram::draw_spectrogram) for
/// why a bend inside a quad is not.
pub(crate) fn slab_drawn(layout: &TexLayout, t: f64) -> f32 {
    slab_at(layout, t.min(hold_time(layout)))
}

/// Slabs the GPU's copy holds: what the [`Plan`] sized off the pane, floored by
/// the run it is actually being asked to show.
///
/// Sized off the PANE, not off the window and not off how much history has
/// arrived: a capacity that tracked either would change as they moved, and
/// every change reallocates the buffer and re-uploads every slab — rebuilding
/// the very thing the delta saves. At a fixed slab width the pane's own column
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
/// each flip cost a full reallocation and a re-upload of every slab.
fn ring_capacity(planned: usize, visible: usize) -> usize {
    // The max is the correctness floor: the run must fit, or two of its keys
    // share a slot and one of them draws the other's slab. It never binds — a
    // window at this slab width is at most `target_cols` slabs and the run
    // overruns it by two, against the headroom's eight — and that it never binds
    // is what `no_cache_layer_falls_back_as_the_window_scrolls` holds it to.
    planned.max(visible + 2)
}

/// Slabs of headroom [`ring_capacity`] holds past the PANE's own column count —
/// `target_cols`, which is what the capacity is sized off, so it holds still
/// across a Span drag. Four covers the breathing described there (a column's
/// spacing at the far edge is at most one slab, plus a floor at each end); the
/// rest is margin, and `ring_capacity`'s body leans on the whole eight.
const RING_HEADROOM: usize = 8;

/// The level the heatmap's pixels actually go through, for the crate's own
/// tests — the bridge `the_heatmap_reads_the_curve_s_own_level_scale` holds the
/// curve against. It is the color mapping, not the analyzer's height mapping.
///
/// Through the affine the SHADER is handed rather than through
/// [`spectrogram_level_db`](crate::panes::spectral::axes::spectrogram_level_db)
/// directly, which is the whole point of the function: the bridge has to be what
/// the shipping path computes, or it stops being able to fail.
#[cfg(test)]
pub(crate) fn bin_level_for_test(cfg: &SpectrumConfig, bucket: BucketDb, midi: f32) -> f32 {
    let (level0, per_step, per_midi) = level_affine(cfg);
    (level0 + per_step * f32::from(bucket) + per_midi * midi).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panes::spectral::DEPTH_ZOOM_PER_DRAG_POINT;
    use egui::Color32;
    use harmonigraph_core::spectrum::SPECTRUM_BINS;

    /// Slabs an aggregator is told to keep, where the test is about the values
    /// it produces rather than about what it retains: larger than any of these
    /// windows holds, so the trim never enters into it. The sweep and the drag
    /// test below pass the real, pane-sized retention instead.
    const KEEP: usize = 1 << 20;

    /// The pitch range the scroll sweeps hold fixed while they move time.
    const SWEEP_SCALE: PitchScale = PitchScale { min_midi: 40.0, max_midi: 88.0, span: 48.0 };

    /// The key a live frame mints for a window starting at column `first` of a
    /// store holding `len`, whose newest column is stamped `newest`.
    fn run_key(first: usize, len: usize, newest: f64, bucket: f64) -> RunKey {
        RunKey {
            first,
            cols_len: len,
            newest_bits: newest.to_bits(),
            bucket_bits: bucket.to_bits(),
            whole: false,
        }
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
        let (centers, power) = aggregate_slabs(cols.iter(), 1.0);
        assert_eq!(centers.len(), 1, "one slab of width 1.0 s holds all three");
        assert_eq!(power[5], q(1.0), "the short note's peak survives");
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
        let (centers, power) = aggregate_slabs(cols.iter(), 0.25);
        assert_eq!(centers.len(), 5, "one row per slab, silent ones included");
        let at = |slab: usize| power[slab * SPECTRUM_BINS + 5];
        assert_eq!(at(0), q(1.0), "the column before the gap");
        assert_eq!([at(1), at(2), at(3)], [0, 0, 0], "the gap reads as silence, not as a smear");
        assert_eq!(at(4), q(0.5), "the column after it");
        // Evenly spaced centers are exactly what the texture mapping assumes.
        for pair in centers.windows(2) {
            assert!(
                (pair[1] - pair[0] - 0.25).abs() < 1e-9,
                "slabs must stay uniform: {centers:?}"
            );
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
        let (centers, power) = aggregate_slabs(cols.iter(), 0.25);
        assert_eq!(centers.len(), 3, "the empty slab still gets its row");
        assert_eq!(power[SPECTRUM_BINS + 5], q(1.0), "and holds the column before it");
        assert_eq!(power[2 * SPECTRUM_BINS + 5], q(0.5));
    }

    /// Time running backwards (a transport jump) starts a fresh row rather
    /// than trying to fill a negative gap — which would be an enormous loop,
    /// or a silent no-row.
    #[test]
    fn columns_going_back_in_time_still_get_a_row() {
        let cols = [col(10.0, &[(5, 1.0)]), col(1.0, &[(5, 0.5)])];
        let (centers, power) = aggregate_slabs(cols.iter(), 0.25);
        assert_eq!(centers.len(), 2);
        assert_eq!(power[SPECTRUM_BINS + 5], q(0.5), "the rewound column landed in its own row");
    }

    #[test]
    fn a_slab_is_anchored_to_absolute_time_not_ring_position() {
        // The same note must land in the same slab whether or not older columns
        // are present — otherwise scrolling would shift it and it would shimmer.
        // A note at t=2.6 sits in slab floor(2.6)=2.
        let with_old = [col(0.1, &[(0, 0.1)]), col(2.6, &[(0, 0.5)])];
        let (c_full, _) = aggregate_slabs(with_old.iter(), 1.0);
        let just_note = [col(2.6, &[(0, 0.5)])];
        let (c_scrolled, _) = aggregate_slabs(just_note.iter(), 1.0);
        assert!(c_full.contains(&2.5), "slab center is 2.5 with old columns");
        assert!(c_scrolled.contains(&2.5), "and still 2.5 after they scroll off");
    }

    /// Storing a bucket as a byte of dB is a memory decision, and it is only
    /// allowed to be one: the colour a cell ends up must be the colour the
    /// power itself would have produced, to within half the grid step it was
    /// put on. Anything wider would be a look change wearing an optimization's
    /// clothes. (The step itself was judged by eye against a sixteen-bit store
    /// and found invisible; this is what keeps it from drifting after.)
    #[test]
    fn quantizing_a_bucket_does_not_move_its_colour() {
        use crate::panes::spectral::axes::{loudness_db, power_db};
        let mut cfg = SpectrumConfig::default();
        let tolerance =
            0.5 * harmonigraph_core::spectrogram::DB_STEP / (cfg.ceiling_db - cfg.floor_db) + 1e-6;
        for tilt in [0.0, 3.0, -3.0] {
            cfg.tilt = tilt;
            for midi in [20.0f32, 60.0, 100.0, 130.0] {
                for power in [1e-8f32, 1e-6, 1e-4, 1e-2, 0.1, 0.5, 1.0, 4.0] {
                    let exact = loudness_db(&cfg, power_db(power), midi);
                    let stored = bin_level_for_test(&cfg, q(power), midi);
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
        assert_eq!(bin_level_for_test(&cfg, 0, 60.0), 0.0, "an empty bucket must read as silence");
    }

    /// The quiet end of the ramp must FADE to black, not fall off a cliff into
    /// it. A shortcut answering everything under -90 dB as silence is
    /// invisible while the dB window bottoms out above that, and becomes a
    /// hard edge — faintest colour straight to black — as soon as the window
    /// can be dragged below it. Nothing between two adjacent stored bytes may
    /// move the level by more than the step between them.
    ///
    /// The sweep runs past [`LEVEL_MIN_DB`](crate::LEVEL_MIN_DB) on purpose: the
    /// Level range bar stops at -100, and a hand-edited blob does not.
    #[test]
    fn the_quiet_end_of_the_ramp_fades_instead_of_cutting_off() {
        let mut cfg = SpectrumConfig { volume_ceiling_db: 0.0, ..SpectrumConfig::default() };
        for floor in [-60.0f32, -90.0, -100.0, -120.0] {
            cfg.volume_floor_db = floor;
            // One stored step, as a fraction of the window it is drawn in; the
            // levels either side of any stored byte may differ by that and no
            // more.
            let step = harmonigraph_core::spectrogram::DB_STEP / (cfg.volume_ceiling_db - floor);
            for bucket in 0..BucketDb::MAX {
                let here = bin_level_for_test(&cfg, bucket, 60.0);
                let next = bin_level_for_test(&cfg, bucket + 1, 60.0);
                assert!(
                    next - here <= step * 1.001 && next >= here,
                    "floor {floor}: byte {bucket} ({here}) -> {next} jumps by {}, \
                     one step is {step}",
                    next - here,
                );
            }
            // And the bottom byte is black at every window, so silence still
            // recedes into the region's bed rather than glowing.
            assert_eq!(
                bin_level_for_test(&cfg, 0, 60.0),
                0.0,
                "floor {floor}: silence must be black"
            );
        }
    }

    /// Every preset, which is what a fresh install and a Palette press can put
    /// on screen — where the field itself accepts any gradient, that being the
    /// point of the bars.
    #[test]
    fn cells_are_opaque_and_run_dark_to_bright() {
        for preset in crate::SpectrogramPreset::ALL {
            let g = preset.gradient();
            let cell_color = crate::panes::spectral::spectrogram::cell_color;
            let (quiet, loud) = (cell_color(g, 0.0), cell_color(g, 1.0));
            // Opaque throughout: a cell's level is its COLOUR, never its alpha,
            // so silence recedes by being dark rather than by being
            // see-through.
            assert_eq!(quiet.a(), 255, "{preset:?}");
            assert_eq!(loud.a(), 255, "{preset:?}");
            // Silence is the dark end; loud is brighter. In `L*`, which is the
            // units the gradient is authored in — a channel sum weights blue
            // like green and would call a violet and a yellow of one sum
            // equally bright.
            let lightness = |c: Color32| {
                let v = |b: u8| f64::from(b) / 255.0;
                harmonigraph_scene::color::lightness_of_encoded(v(c.r()), v(c.g()), v(c.b()))
            };
            assert!(
                lightness(quiet) < 0.5,
                "{preset:?}: silence must sit on the ramp's black end, got {quiet:?}"
            );
            assert!(lightness(loud) > lightness(quiet), "{preset:?}");
        }
    }

    /// A preset's six numbers ARE the two ends each of its bars reads out.
    ///
    /// [`SpectrogramPreset::gradient`] states each stretch as the pair a reader
    /// can name — the `L*` and the chroma share that silence and a full bucket
    /// are drawn at — and composes the middle-and-signed-ramp the gradient
    /// stores. That composition is arithmetic with no picture of its own, so
    /// nothing downstream can catch it being wrong: a `chroma` written as
    /// `c_lo` rather than as the midpoint draws every colored preset visibly
    /// duller and leaves every other assertion in the crate standing.
    ///
    /// Asserted against the pair the ARM names rather than against a constant
    /// restated here — a second copy of the numbers would agree with itself
    /// and the arm at once, and stop discriminating the moment either moved.
    #[test]
    fn a_preset_composes_the_pair_its_arm_names() {
        // The pairs, spelled once, beside the arm that has to produce them.
        let ends = |p: crate::SpectrogramPreset| -> ((f32, f32), (f32, f32)) {
            use crate::SpectrogramPreset::*;
            match p {
                Mono => ((0.0, 100.0), (0.0, 0.0)),
                Ice => ((0.0, 92.0), (0.635, 0.985)),
                Aurora => ((0.0, 88.0), (0.518, 0.968)),
                Magma => ((0.0, 90.0), (0.819, 0.969)),
            }
        };
        // A middle and a signed ramp do not recompose to their own ends bit for
        // bit — `Spread::legal` says why, a hundredth being no binary fraction —
        // so the ends are compared to within an ulp or two rather than exactly.
        // Six orders under the smallest difference any real slip makes: writing
        // `c_lo` for the midpoint moves Aurora's low end by 0.11.
        let close = |got: f32, want: f32| (got - want).abs() < 1e-5;
        for preset in crate::SpectrogramPreset::ALL {
            let g = preset.gradient();
            let ((l_lo, l_hi), (c_lo, c_hi)) = ends(preset);
            let half = g.lightness_ramp * 0.5;
            assert!(
                close(g.lightness - half, l_lo) && close(g.lightness + half, l_hi),
                "{preset:?}: the brightness bar would read out \
                 {} -> {}, not {l_lo} -> {l_hi}",
                g.lightness - half,
                g.lightness + half,
            );
            // Off the curve rather than off the fields, so this measures what a
            // cell is actually drawn with: `chroma_at` is what resolves the pair
            // at a level, and the bars read out its ends.
            let (got_lo, got_hi) = (g.chroma_at(0.0) as f32, g.chroma_at(1.0) as f32);
            assert!(
                close(got_lo, c_lo) && close(got_hi, c_hi),
                "{preset:?}: the chroma bar would read out \
                 {got_lo} -> {got_hi}, not {c_lo} -> {c_hi}",
            );
            // And the gradient accepts all six untouched. Sanitize bounds a ramp
            // by what its middle leaves on the axis, so a preset that overran
            // would be silently pulled in and the pane would draw a picture the
            // preset did not ask for — passing every test above, which read the
            // clamped numbers back.
            assert_eq!(g.sanitized(), g, "{preset:?}: sanitize moved a preset");
        }
    }

    /// The heatmap's ramp reaches the two `L*` ends its brightness bar reads
    /// out, which is the whole of what the pane promises about a gradient it did
    /// not author: a cell at level 0 or 1 has to be drawn at them.
    ///
    /// Through [`cell_color`](crate::panes::spectral::spectrogram::cell_color) rather
    /// than through the curve, so what is measured
    /// is the table the texels come off — including its quantization to a byte,
    /// which is what the tolerance is for. The numbers it is held to are
    /// `sanitized`'s, which is honest only because
    /// `a_preset_composes_the_pair_its_arm_names` proves sanitize moves none of
    /// them; without that, this would be measuring the clamped picture against
    /// the clamped numbers and agreeing with itself.
    #[test]
    fn a_gradient_draws_its_own_ends() {
        for preset in crate::SpectrogramPreset::ALL {
            let g = preset.gradient().sanitized();
            let half = g.lightness_ramp * 0.5;
            let (lo, hi) = (g.lightness - half, g.lightness + half);
            for (level, want) in [(0.0, lo), (1.0, hi)] {
                let c = crate::panes::spectral::spectrogram::cell_color(g, level);
                let v = |b: u8| f64::from(b) / 255.0;
                let got =
                    harmonigraph_scene::color::lightness_of_encoded(v(c.r()), v(c.g()), v(c.b()));
                assert!(
                    (got - f64::from(want)).abs() < 0.6,
                    "{preset:?} at level {level}: asked for L* {want}, drew {got:.2} ({c:?})"
                );
            }
        }
    }

    /// The incremental aggregator must produce EXACTLY what a from-scratch
    /// `aggregate_slabs` over the window would, at every step — otherwise the
    /// live spectrogram would drift from what the batch/offline path draws. This
    /// walks a column stream with same-slab clusters, a one-slab jitter gap
    /// (hold-previous), a multi-slab gap (zeros), steady scroll (so `first`
    /// advances and the front trims), a ring trim (so indices shift), and a
    /// bucket change (a forced rebuild), comparing byte-for-byte each step.
    #[test]
    fn incremental_aggregation_matches_batch_step_for_step() {
        let bucket = 0.25;
        let window_span = 1.0;
        // Exercises: cluster (0.30, 0.31), 1-slab gap (0.55->0.80 is 1 apart;
        // 0.80->1.60 is a multi-slab gap), then steady scroll.
        let times: [f64; 14] =
            [0.05, 0.10, 0.30, 0.31, 0.55, 0.80, 1.60, 1.62, 1.90, 2.15, 2.40, 2.65, 2.90, 3.15];

        let mut agg = SpectrogramAgg::new();
        let mut history = crate::SpectrumHistory::default();
        for (i, &t) in times.iter().enumerate() {
            // Per-column, per-bin energy, so a wrong slab or a stale hold surfaces
            // as a value mismatch, not just a shape one.
            let e =
                [(4, 0.1 * (i as f32 + 1.0)), (7, 0.05 * i as f32), (10, 1.0 - 0.03 * i as f32)];
            history.push(col(t, &e));
            // Trim the store, so `first` indices shift under the aggregator.
            history.trim_older_than(t - (window_span + 0.5));
            let oldest = t - window_span;
            let first = history.partition_point(|c| c.time < oldest).saturating_sub(1);
            let inc = agg.window(&history, first, bucket, KEEP);
            let bat = aggregate_slabs(history.iter_from(first), bucket);
            assert_eq!(inc, bat, "incremental != batch at step {i} (t={t})");
        }

        // A layout change (new bucket) must fall back to a rebuild — still exact.
        let now = *times.last().unwrap();
        let first = history.partition_point(|c| c.time < now - window_span).saturating_sub(1);
        let inc = agg.window(&history, first, 0.4, KEEP);
        let bat = aggregate_slabs(history.iter_from(first), 0.4);
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
            let _ = agg.window(&history, 0, bucket, KEEP);
        }
        let rebuilds = agg.rebuilds();

        // Now the window starts at the column at 0.10 — index 2 — so the two
        // louder columns sharing its slab have fallen out of it.
        let inc = agg.window(&history, 2, bucket, KEEP);
        let bat = aggregate_slabs(history.iter_from(2), bucket);
        assert_eq!(agg.rebuilds(), rebuilds, "a rebuild would explain it away");
        assert_eq!(inc, bat, "slab 1 holds the out-of-window column at 0.00");

        // The same numbers through the REBUILD path: a fresh aggregator whose
        // first call already starts at index 2. Running only this one would let
        // you conclude the fast path was to blame; it is the pair that settles
        // that the bug is in the hold itself.
        let mut fresh = SpectrogramAgg::new();
        assert_eq!(fresh.window(&history, 2, bucket, KEEP), bat, "and after a rebuild");
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
            let _ = agg.window(&history, 0, bucket, KEPT);
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
        let inc = agg.window(&history, 4, bucket, KEPT);
        let bat = aggregate_slabs(history.iter_from(4), bucket);
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
    /// neither original was folded into, carrying both originals' energy across
    /// a slab boundary the raw columns respected — so THAT is not the
    /// comparison to hold this to.
    ///
    /// What it must equal is batch over the columns AS THEY ARRIVED, which is
    /// both the finer answer and the one [`crate::WholeSong`] gives the offline
    /// renderer from its raw, never-merged columns. The window's first slab is
    /// the exception: it is pruned to the in-window columns, which by then are
    /// the merged ones the store holds, so the comparison starts past it.
    #[test]
    fn incremental_aggregation_matches_the_raw_columns_across_a_tier_merge() {
        // Buckets 10 and 11 alternate between adjacent columns, so a merged
        // pair holds both loud where each original held one — a slab refolded
        // from the merged store reads differently from the slab the raw
        // columns built. A constant bucket — 4, which never moves — would
        // demonstrate nothing.
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
            let inc = agg.window(&history, first, bucket, KEEP);
            // Comparing every step would be O(columns^2); check often enough to
            // catch the crossing itself, and either side of it.
            let near_merge = i + 8 >= crate::SpectrumHistory::FINE_COLUMNS;
            if near_merge || i % 256 == 0 || i + 1 == columns {
                let t0 = history.get(first).map_or(0, |c| (c.time / bucket).floor() as i64);
                let in_window = raw.iter().filter(|c| (c.time / bucket).floor() as i64 >= t0);
                let want = aggregate_slabs(in_window, bucket);
                assert_eq!(inc.0, want.0, "slab centers diverged at column {i} (t={t})");
                assert_eq!(
                    inc.1[SPECTRUM_BINS..],
                    want.1[SPECTRUM_BINS..],
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
            let (centers, power) = agg.window(&history, first, bucket, KEEP);
            // The window it serves is still the right shape and length.
            assert_eq!(power.len(), centers.len() * SPECTRUM_BINS, "grid shape at column {i}");
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
            let inc = agg.window(&history, first, bucket, KEEP);
            if i % 256 == 0 || i + 1 == columns {
                let bat = aggregate_slabs(history.iter_from(first), bucket);
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
    /// lag, a slab against the column rate, a slab count against the cap the
    /// store is sized for — and with [`Plan`] split out of the draw it is
    /// exercised without an `egui::Context` at all. Each assertion below is a
    /// bug this pane has actually had.
    #[test]
    fn the_plan_decides_the_layout_without_a_frame() {
        let scale = PitchScale { min_midi: 40.0, max_midi: 88.0, span: 48.0 };
        let view = |ppp: f32, pitch_len: f32, depth_len: f32, window: f64, whole: bool| PaneView {
            ppp,
            pitch_len,
            depth_len,
            window,
            scale,
            cfg: SpectrumConfig::default(),
            whole,
        };
        let columns = Columns { first: 3, len: 400, newest: 12.0 };
        let plan = |v: &PaneView| Plan::new(v, &columns);

        // Rows are PIXELS, not points: on a 2x screen the picture is read at
        // the density it will be drawn at, rather than upsampled from half of
        // it — and to the pixel, since a row costs a fragment's arithmetic and
        // no memory at all.
        let rows_at =
            |ppp: f32, pitch: f32| plan(&view(ppp, pitch, 800.0, 12.0, false)).rows as f32;
        for (ppp, pitch) in [(1.0, 300.0), (2.0, 300.0), (1.0, 517.0), (2.0, 517.0)] {
            assert_eq!(rows_at(ppp, pitch), (pitch * ppp).round(), "rows are the pane's pixels");
        }
        // Twice the density really is twice the picture.
        assert!((rows_at(2.0, 517.0) / rows_at(1.0, 517.0) - 2.0).abs() < 0.01);

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

        // The key travels with the plan, and names the FOLD: a density change
        // that moves no slab boundary re-reads the run it already has, while one
        // that crosses a ladder rung does not.
        let at_1x = plan(&view(1.0, 300.0, 800.0, 12.0, false)).key;
        assert_eq!(at_1x, plan(&view(1.0, 300.0, 800.0, 12.0, false)).key);
        assert_eq!(
            at_1x,
            plan(&view(1.0, 900.0, 800.0, 12.0, false)).key,
            "a taller pane reads the same slabs and must not refold",
        );
        assert_ne!(
            at_1x,
            plan(&view(1.0, 300.0, 200.0, 12.0, false)).key,
            "a quarter of the depth pixels crosses a rung, which is a different fold",
        );
    }

    /// **A take longer than the render's window must not fold past the slab
    /// cap.**
    ///
    /// The sweep above cannot see this: it derives the whole-song slab count
    /// from `window / bucket`, and the two numbers part company exactly here.
    /// `bucket` is cut for the WINDOW ([`Plan::new`]) while the run's length
    /// comes from the columns the fold walks, and
    /// [`WholeSong::precompute`](crate::WholeSong::precompute) analyses the
    /// whole `samples` buffer whatever `--start`/`--end` asked for — so a short
    /// window on a long bounce arrives with columns spanning the file. Ten
    /// seconds of a three-minute take folds to some 14 000 slabs, which is a
    /// grid several times the size of the picture that shows it, on time no
    /// pixel reaches (issue #367).
    ///
    /// FOLDED rather than counted. The length is a property of
    /// [`SlabGrid::fold`]'s absolute keying — an empty slab still takes a row,
    /// so the count follows the columns' EXTENT and not their number — and an
    /// arithmetic restatement of that is a restatement of the thing under test.
    /// It is also what lets the fixture be sparse: columns half a second apart
    /// reach the same 14 000 slabs as the analyzer's own rate for a sixtieth of
    /// the memory.
    ///
    /// The window starts are deliberately off the slab grid: a window whose ends
    /// both fall mid-slab is the case that spends a slab at each end.
    #[test]
    fn a_take_longer_than_the_render_window_folds_inside_the_slab_cap() {
        const TAKE: f64 = 180.0;
        let columns: Vec<_> = (0..=360).map(|i| col(i as f64 * 0.5, &[(1000, 1.0)])).collect();
        let mut ws = crate::WholeSong {
            start: 0.0,
            span: TAKE,
            columns,
            roll: harmonigraph_core::NoteRoll::default(),
        };
        let plan_for = |ppp: f32, depth: f32, span: f64| {
            Plan::new(
                &PaneView {
                    ppp,
                    pitch_len: 1000.0,
                    depth_len: depth,
                    window: span,
                    scale: SWEEP_SCALE,
                    cfg: SpectrumConfig { roll_seconds: span as f32, ..SpectrumConfig::default() },
                    whole: true,
                },
                &Columns { first: 0, len: 361, newest: TAKE },
            )
        };

        // The cap, plus the slab each end of the window can spend by falling
        // mid-slab.
        let ceiling = WHOLE_SONG_SLAB_CAP as usize + 2;
        for ppp in [1.0f32, 2.0, 3.0] {
            for depth in [800.0f32, 2000.0, 8000.0] {
                for span in [2.5f64, 10.0, 47.0, TAKE] {
                    for offset in [0.0f64, 0.331, 60.017] {
                        ws.start = offset.min(TAKE - span);
                        ws.span = span;
                        let p = plan_for(ppp, depth, span);
                        let (centers, _) = aggregate_slabs(ws.drawn_columns(span), p.bucket);
                        assert!(
                            centers.len() <= ceiling,
                            "a {span} s window at {} of a {TAKE} s take folds to {} slabs, \
                             past the {ceiling} the cap allows",
                            ws.start,
                            centers.len(),
                        );
                    }
                }
            }
        }

        // And the untrimmed fold really did overrun, so a sweep that passes is
        // reading the trim rather than a take that happened to fit.
        ws.start = 60.0;
        ws.span = 10.0;
        let p = plan_for(2.0, 800.0, 10.0);
        let (all, all_power) = aggregate_slabs(ws.columns.iter(), p.bucket);
        assert!(
            all.len() > ceiling,
            "the fixture no longer reproduces #367: the whole take folds to {} slabs, \
             inside the {ceiling} the cap allows",
            all.len(),
        );

        // Both ENDS are the window's own, inclusive. A column stamped exactly
        // at an edge is inside the region the axis draws — `frac` gives it 0
        // or 1 — so dropping it would shorten the image by a slab at a
        // boundary that is otherwise the commonest one there is: `--start 0`
        // puts a column on the near edge whenever the hop divides it.
        ws.start = 60.0;
        ws.span = 10.0;
        let edges: Vec<f64> = ws.drawn_columns(ws.span).map(|c| c.time).collect();
        assert_eq!(
            (edges.first().copied(), edges.last().copied()),
            (Some(60.0), Some(70.0)),
            "the window's own endpoints were trimmed off it",
        );

        // And the trim moved nothing that was drawn. The slab keys are
        // ABSOLUTE, so the window's slabs are the same slabs at the same times
        // carrying the same bytes — the trimmed grid is a contiguous run of the
        // untrimmed one, which is the whole claim that this costs no picture.
        let (kept, kept_power) = aggregate_slabs(ws.drawn_columns(10.0), p.bucket);
        let at = all
            .iter()
            .position(|c| (c - kept[0]).abs() < 1e-9)
            .expect("the window's first slab is one of the take's");
        assert_eq!(&all[at..at + kept.len()], &kept[..], "the window's slabs moved in time");
        assert_eq!(
            &all_power[at * SPECTRUM_BINS..(at + kept.len()) * SPECTRUM_BINS],
            &kept_power[..],
            "the window's slabs changed value",
        );
    }

    /// **The fold is trimmed to the window the AXIS draws, not to the span the
    /// take was asked for** — the two are the same number only above the axis'
    /// own floor.
    ///
    /// `TimeAxis::new` puts a floor of 0.05 s under the window it maps time
    /// across, so a render shorter than that draws a depth region reaching past
    /// `start + span`, and the columns out there have a real depth on screen.
    /// Trimming to `span` dropped them: a 20 ms render folded out to 60.032 of
    /// a region drawn to 60.05, leaving 36% of the heatmap as bare bed with the
    /// columns for it sitting unused in `WholeSong::columns`.
    ///
    /// Through [`build`] rather than
    /// [`drawn_columns`](crate::WholeSong::drawn_columns) directly, and that is
    /// the whole point of the test: what broke was not the trim but WHICH
    /// window `build` hands it, so a test that passes the window in itself
    /// asserts its own arithmetic and would have passed throughout. The
    /// returned [`TexLayout`] is where the answer shows.
    ///
    /// A sub-50 ms export is degenerate (one frame at 30 fps), which is why the
    /// fix is to hand the trim the plan's own `window` rather than to lower the
    /// axis' floor: two expressions of one window are what drift, and the
    /// degenerate case is only where the drift becomes visible.
    #[test]
    fn the_fold_covers_the_whole_depth_region_a_short_render_draws() {
        // The axis' own floor, from `TimeAxis::new`.
        const FLOOR: f64 = 0.05;
        let (start, span) = (60.0, 0.02);
        // Columns at the analyzer's rate across the window and past it —
        // `precompute` stamps them over the whole take whatever was asked for.
        let columns: Vec<_> = (0..40)
            .map(|i| col(59.9 + i as f64 * crate::AudioSpectrum::FFT_INTERVAL, &[(1000, 1.0)]))
            .collect();
        let ws =
            crate::WholeSong { start, span, columns, roll: harmonigraph_core::NoteRoll::default() };
        // What the pane hands the plan: `time.window()`, the FLOORED one.
        let window = ws.span.max(FLOOR);
        let view = PaneView {
            ppp: 2.0,
            pitch_len: 500.0,
            depth_len: 300.0,
            window,
            scale: SWEEP_SCALE,
            cfg: SpectrumConfig { roll_seconds: window as f32, ..SpectrumConfig::default() },
            whole: true,
        };
        let columns_in = Columns { first: 0, len: ws.columns.len(), newest: 60.2 };
        let plan = Plan::new(&view, &columns_in);
        let mut spectrum = crate::AudioSpectrum::default();
        let layout = build(&mut spectrum, Some(&ws), 0, &plan, &view)
            .expect("a whole-song fold over columns this dense");

        // The last column the axis puts inside the region. The run has to reach
        // it: `depth_of` maps both through the same `frac`, so a run ending
        // short of it leaves the rest of the region bare.
        let last_on_screen = ws
            .columns
            .iter()
            .rev()
            .find(|c| c.time <= start + window)
            .expect("the fixture spans the window")
            .time;
        let reach = layout.t_origin + layout.tex_span;
        assert!(
            reach >= last_on_screen,
            "the run reaches {reach}, short of the column at {last_on_screen} that the \
             region still draws — {:.0}% of the depth region is bare",
            100.0 * (start + window - reach) / window,
        );
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
    ///   so whether the buffer's capacity tracks it.
    ///
    /// The second moves with the FFT window AND with the pane's width, since a
    /// slab is `Span / depth pixels` — a narrow pane crosses it at a much
    /// shorter Span than a wide one. So the sweep is taken per (window, pane)
    /// pair, either side of that pair's own crossing.
    ///
    /// The run key above these two is deliberately not counted: it holds the
    /// newest column's time, so it is MEANT to miss once per column. It is the
    /// two layers under it that must turn a miss into O(one slab).
    #[test]
    fn no_cache_layer_falls_back_as_the_window_scrolls() {
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
                // ring behaviour turns on — plus a close-up to anchor it.
                //
                // The close-up is 12 s rather than the three-minute Span a
                // fresh view opens on, and the two crossing-relative spans are
                // why that costs no coverage: they land either side of the
                // crossing for each window and pane, which is a longer Span
                // than the default for some of those pairs and a shorter one
                // for others. Driving the default outright would add 24,000
                // columns per pair (the loop below runs `(span + 15) /
                // interval` of them) to make a case these already bracket.
                let crossing = lag * cols;
                for span in [12.0f64, crossing * 0.6, crossing * 1.4] {
                    let planned = cols as usize + RING_HEADROOM;
                    let bucket = live_slab(span, cols as usize);
                    let at = format!("{algo} window, {pane} pane, {span:.1} s Span");

                    let mut agg = SpectrogramAgg::new();
                    let mut history = crate::SpectrumHistory::default();
                    let mut gpu = GpuGrid::default();
                    let (mut caps, mut widest) = (std::collections::BTreeSet::new(), 0usize);

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
                        let (centers, power) = agg.window(&history, first, bucket, planned);
                        let visible = centers.len();
                        let first_key = (centers[0] / bucket).floor() as i64;
                        let capacity = ring_capacity(planned, visible);
                        caps.insert(capacity);
                        let layout = TexLayout {
                            bucket,
                            t_origin: centers[0] - 0.5 * bucket,
                            tex_span: visible as f64 * bucket,
                        };
                        gpu.accept(
                            run_key(first, history.len(), t, bucket),
                            first_key,
                            capacity,
                            power,
                            layout,
                        );
                        // Past the frames that fill the window, where the run is
                        // still growing at both ends.
                        if t > span + 1.0 {
                            let dirty = gpu.sent.as_ref().expect("just accepted").dirty.len();
                            widest = widest.max(dirty);
                        }
                    }

                    // One of each to get started, and none after: from then on a
                    // frame folds one column and writes a slab or two.
                    assert_eq!(agg.rebuilds, 1, "the aggregator rescans the window: {at}");
                    assert_eq!(
                        gpu.full_uploads(),
                        1,
                        "the grid is uploaded whole ({caps:?} slabs): {at}",
                    );
                    // The window's first slab (repruned as columns leave it),
                    // the newest (still accumulating), and whichever one has
                    // just appeared.
                    assert!(widest <= 3, "a scrolling frame wrote {widest} slabs: {at}");
                }
            }
        }
    }

    /// Dragging the Span must not re-lay the grid on every frame of the drag.
    ///
    /// This is what [`live_slab`]'s ladder is for. A slab width taken straight
    /// from the window moves whenever the window does, and a moved slab width
    /// re-lays the whole grid — so a drag would pay the full refold and a full
    /// upload, every frame, for as long as it lasted. On the ladder the width
    /// holds across a whole rung, the capacity is sized off the pane so it holds
    /// too, and the aggregator keeps what the GPU's copy keeps so a WIDENING
    /// Span finds its slabs already folded rather than asking for ones just
    /// trimmed.
    ///
    /// Swept in both directions: widening is the harder one, since it reaches
    /// back to slabs the window did not want a frame ago.
    #[test]
    fn dragging_the_span_holds_the_grid_between_ladder_steps() {
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
        let mut gpu = GpuGrid::default();
        let mut widths = std::collections::BTreeSet::new();
        let mut caps = std::collections::BTreeSet::new();

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
                if phase <= 1.0 {
                    hi - (hi - lo) * phase
                } else {
                    lo + (hi - lo) * (phase - 1.0)
                }
            };

            let bucket = live_slab(span, cols);
            widths.insert((bucket * 1e6).round() as i64); // microseconds, to compare exactly
            let first = history.partition_point(|c| c.time < now - span).saturating_sub(1);
            let (centers, power) = agg.window(&history, first, bucket, planned);
            let first_key = (centers[0] / bucket).floor() as i64;
            let capacity = ring_capacity(planned, centers.len());
            caps.insert(capacity);
            let layout = TexLayout {
                bucket,
                t_origin: centers[0] - 0.5 * bucket,
                tex_span: centers.len() as f64 * bucket,
            };
            gpu.accept(
                run_key(first, history.len(), t, bucket),
                first_key,
                capacity,
                power,
                layout,
            );
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
        // The opening upload and nothing else: the capacity is the pane's, so a
        // Span that moves inside its rung moves no slab into a different slot.
        assert_eq!(caps.len(), 1, "the drag resized the buffer: {caps:?} slabs");
        assert_eq!(gpu.full_uploads(), 1, "the drag re-uploaded the whole grid");
    }

    /// A Span drag CROSSING ladder rungs refolds once per rung, not once per
    /// frame.
    ///
    /// [`dragging_the_span_holds_the_grid_between_ladder_steps`] stops just
    /// inside both ends of one rung, so the crossing itself is the regime it
    /// leaves out — and a crossing is the whole of what a horizontal zoom asks
    /// of the aggregator. A crossed rung moves `bucket`, and a moved `bucket`
    /// discards the grid and refolds the retention out of history
    /// ([`SpectrogramAgg::rebuild`]).
    ///
    /// The COUNT is the claim here, because the cost of one is small and known:
    /// a refold is an elementwise byte max per column, half a millisecond to
    /// three across the whole Span range, which fits inside a 144 Hz frame —
    /// once. What would not fit is the cascade the slack in
    /// [`SpectrogramAgg::rebuild`] exists to prevent, where a widening drag
    /// rebuilds flush to its window and is asked for something older on the
    /// very next frame. That costs the same 0.5-3 ms EVERY frame for the
    /// length of the drag, draws the identical picture, and is invisible to
    /// everything except this counter.
    ///
    /// Driven as the pane drives it. The Span is exponential in drag distance
    /// (`roll_seconds * (-along * DEPTH_ZOOM_PER_DRAG_POINT).exp()`) and the
    /// rungs are powers of two, so the crossings are EVENLY spaced along the
    /// drag — one every `ln 2 / DEPTH_ZOOM_PER_DRAG_POINT`, 116 points of
    /// travel.
    ///
    /// The bound is on a drag that TRAVELS, and only on that. Distance is the
    /// whole of what limits the count here, so a Span that stops moving stops
    /// being bounded by anything this test says:
    /// [`a_span_dithering_across_a_rung_refolds_every_frame`] is the same
    /// aggregator, parked, refolding on every frame instead.
    #[test]
    fn a_span_drag_refolds_once_per_rung_crossed() {
        let interval = crate::AudioSpectrum::FFT_INTERVAL;
        let cols = LIVE_SLAB_CAP as usize;
        let planned = cols + RING_HEADROOM;
        // Two rungs of travel each way: 50 s sits in the 64 ms rung and 10 s on
        // the ladder's floor, so a drag between them crosses at 32.768 s and
        // 16.384 s. Below the floor the width cannot move at all, which is why
        // the low end is not pushed further.
        let (hi, lo) = (50.0f64, 10.0f64);

        // Deep enough that the widest rung's retention is history rather than a
        // store that simply ends: a rebuild reaches `planned` slabs back, which
        // at this sweep's widest bucket is 66 s.
        let mut history = crate::SpectrumHistory::default();
        let settle = planned as f64 * live_slab(hi, cols) + hi;
        for i in 0..(settle / interval) as usize {
            history.push(col(i as f64 * interval, &[(4, 0.5), (10, 1.0)]));
        }
        let mut now = history.back().expect("filled").time;

        let mut agg = SpectrogramAgg::new();
        let mut widths = std::collections::BTreeSet::new();
        // A steady drag on a 144 Hz display: 1.5 points of travel per frame, so
        // the whole 50 s -> 10 s sweep is 268 points and takes 1.2 s. A
        // deliberate zoom rather than a flick — and a slower one only adds
        // frames, never crossings, which is the whole shape of the claim.
        const FRAME: f64 = 1.0 / 144.0;
        let step = (-1.5f64 * DEPTH_ZOOM_PER_DRAG_POINT as f64).exp();

        let mut span = hi;
        let mut frames = 0u32;
        // Down the range and back up. Widening is the harder direction: it
        // reaches back to slabs the window did not want a frame ago.
        for narrowing in [true, false] {
            loop {
                span = if narrowing { span * step } else { span / step };
                if narrowing && span < lo || !narrowing && span > hi {
                    break;
                }
                frames += 1;
                // Columns arrive on their own clock (125/s), not the frame's.
                now += FRAME;
                while history.back().is_none_or(|c| c.time + interval <= now) {
                    let t = history.back().map_or(0.0, |c| c.time) + interval;
                    history.push(col(t, &[(4, 0.5), (10, 1.0)]));
                }
                let bucket = live_slab(span, cols);
                widths.insert((bucket * 1e6).round() as i64);
                let first = history.partition_point(|c| c.time < now - span).saturating_sub(1);
                let _ = agg.window(&history, first, bucket, planned);
            }
        }

        assert_eq!(widths.len(), 3, "the sweep did not cross two rungs: {widths:?} us");
        assert!(frames > 300, "the sweep never ran: {frames} frames");
        // The opening build, then one per crossing in each direction. Anything
        // approaching `frames` is the cascade above.
        assert_eq!(
            agg.rebuilds, 5,
            "a {frames}-frame Span drag across 2 rungs each way refolded {} times",
            agg.rebuilds,
        );
    }

    /// A Span sitting ON a rung boundary refolds every frame, because the
    /// ladder has no hysteresis.
    ///
    /// [`a_span_drag_refolds_once_per_rung_crossed`] bounds a drag that TRAVELS.
    /// This is the other regime, and it is not a corner: [`live_slab`] re-decides
    /// the rung from the window alone on every frame, so a Span parked within one
    /// drag-point of a boundary alternates its slab width for as long as the hand
    /// holds still. One point is 0.6% of the Span, which a resting finger clears
    /// on tremor alone.
    ///
    /// Both caches then miss together, which is what makes it expensive rather
    /// than merely frequent: `bucket` is the aggregator's one layout input AND a
    /// [`RunKey`] field, so every frame pays a full refold and rewrites every
    /// slab it folded — the cascade the test above shows a travelling drag
    /// avoids.
    ///
    /// Asserted as the cost it is rather than as a bug, since nothing here fixes
    /// it. What would is hysteresis in [`live_slab`] — holding the current rung
    /// until the window is some way past the boundary — and that is shipping-code
    /// work with its own picture question, since the rung then depends on which
    /// side the Span arrived from.
    #[test]
    fn a_span_dithering_across_a_rung_refolds_every_frame() {
        let interval = crate::AudioSpectrum::FFT_INTERVAL;
        let cols = LIVE_SLAB_CAP as usize;
        let planned = cols + RING_HEADROOM;
        // The boundary between the 32 ms and 64 ms rungs: `live_slab` steps up
        // exactly where the window stops fitting in `cols` slabs.
        let boundary = 0.032 * cols as f64;
        let step = (-(DEPTH_ZOOM_PER_DRAG_POINT as f64)).exp();

        let mut history = crate::SpectrumHistory::default();
        let settle = planned as f64 * 0.064 + boundary;
        for i in 0..(settle / interval) as usize {
            history.push(col(i as f64 * interval, &[(4, 0.5), (10, 1.0)]));
        }
        let mut now = history.back().expect("filled").time;

        let mut agg = SpectrogramAgg::new();
        let mut widths = std::collections::BTreeSet::new();
        const FRAME: f64 = 1.0 / 144.0;
        // One point of tremor either way, alternating: the hand is still and the
        // Span is not.
        let mut frames = 0u32;
        for i in 0..300 {
            let span = if i % 2 == 0 { boundary * step } else { boundary / step };
            frames += 1;
            now += FRAME;
            while history.back().is_none_or(|c| c.time + interval <= now) {
                let t = history.back().map_or(0.0, |c| c.time) + interval;
                history.push(col(t, &[(4, 0.5), (10, 1.0)]));
            }
            let bucket = live_slab(span, cols);
            widths.insert((bucket * 1e6).round() as i64);
            let first = history.partition_point(|c| c.time < now - span).saturating_sub(1);
            let _ = agg.window(&history, first, bucket, planned);
        }

        assert_eq!(widths.len(), 2, "the dither did not straddle a rung: {widths:?} us");
        // Every frame but the first, which is the opening build.
        assert_eq!(
            agg.rebuilds, frames,
            "a Span parked on a rung boundary refolded {} times in {frames} frames",
            agg.rebuilds,
        );
    }

    /// A frame that folds nothing re-sends the run it already holds, and a
    /// frame that folds writes only the slabs whose bytes moved.
    ///
    /// This is the whole of what replaced the texture: the picture is a
    /// statement about which slabs are in which slots, so the cheap frame is
    /// the one that repeats it and the expensive one is the full upload. Both
    /// halves are invisible from the picture — a frame that re-uploaded
    /// everything, and a frame that patched nothing when it should have, draw
    /// the same heatmap — so they are counted rather than looked at.
    #[test]
    fn a_second_frame_on_the_same_columns_reuses_the_run_it_sent() {
        let mut spectrum = crate::AudioSpectrum::default();
        let mut bins = [0.0f32; SPECTRUM_BINS];
        bins[1000] = 0.8;
        for i in 0..200 {
            spectrum.push_history(90.0 + f64::from(i) * 0.01, &bins);
        }
        let view = PaneView {
            ppp: 2.0,
            pitch_len: 300.0,
            depth_len: 600.0,
            window: 1.5,
            scale: SWEEP_SCALE,
            cfg: SpectrumConfig { roll_seconds: 1.5, ..SpectrumConfig::default() },
            whole: false,
        };
        let columns = |len: usize, newest: f64| Columns { first: 0, len, newest };
        let fold = |spectrum: &mut crate::AudioSpectrum, cols: &Columns| {
            let plan = Plan::new(&view, cols);
            let layout = run_for(spectrum, None, 0, &plan, &view).expect("a run to draw");
            let sent = spectrum.spectrogram[0].gpu.sent.as_ref().expect("a run was accepted");
            (layout, sent.run.clone(), sent.dirty.clone())
        };

        let cols = columns(200, 91.99);
        let (cold, run, dirty) = fold(&mut spectrum, &cols);
        assert!(dirty.is_empty(), "the first fold patched a buffer that was never written");
        assert_eq!(spectrum.spectrogram[0].gpu.full_uploads(), 1);

        // Same columns: the key hits, so nothing is folded and the same
        // allocation goes back to the GPU.
        let (hit, again, _) = fold(&mut spectrum, &cols);
        assert_eq!(cold, hit, "the reused run drew at different geometry");
        assert!(Arc::ptr_eq(&run, &again), "a hit refolded the store");

        // A fresh column: the key misses, and what the GPU is told is the one
        // slab that moved — the newest, still accumulating its max.
        spectrum.push_history(92.0, &bins);
        let (_, _, dirty) = fold(&mut spectrum, &columns(201, 92.0));
        assert_eq!(dirty.len(), 1, "a column's arrival wrote {} slabs", dirty.len());
        assert_eq!(spectrum.spectrogram[0].gpu.full_uploads(), 1, "a column forced a full upload");

        // And a released context is a full upload rather than a patch of a
        // buffer nothing wrote.
        spectrum.spectrogram[0].gpu.release();
        let (_, _, dirty) = fold(&mut spectrum, &columns(201, 92.0));
        assert!(dirty.is_empty(), "a fresh context was sent a delta");
        assert_eq!(spectrum.spectrogram[0].gpu.full_uploads(), 2);
    }

    /// The gradient table is keyed on what decides a texel of it, so two
    /// gradients that draw one picture share a table and every other change
    /// builds a new one.
    ///
    /// Both directions matter and they fail differently. A knob the key misses
    /// leaves the OLD table on the GPU under a gradient that no longer draws it
    /// — a wrong picture, which no frame counter reports. A knob the key watches
    /// that decides nothing rebuilds and re-uploads 4096 entries on every frame
    /// of the drag that moves it, which is the cost
    /// [`what_decides_a_texel`] exists to remove.
    ///
    /// The folded pairs are checked as PICTURES and not just as keys: the fold
    /// claims two gradients draw the same table, so the tables themselves are
    /// compared. A fold that were merely a key equality would serve one
    /// gradient's colours under another's settings.
    #[test]
    fn the_lut_key_folds_two_gradients_that_draw_one_picture() {
        let cfg = SpectrumConfig::default();
        let mut gpu = GpuGrid::default();
        // A table built for `c` alone, so a folded pair can be compared as
        // pixels rather than as keys.
        let table = |c: &SpectrumConfig| {
            let mut fresh = GpuGrid::default();
            fresh.shades(c).lut
        };
        // Whether the table `c` is served is a NEW one: the generation moves
        // only where the fold does, whatever order these are asked in.
        let mut rebuilds = |c: &SpectrumConfig| {
            let before = gpu.shades(&cfg).generation;
            gpu.shades(c).generation != before
        };

        // Every knob of the gradient recolours every texel without moving a
        // slab, so each has to move the key on its own.
        let edited = |edit: fn(&mut Gradient)| {
            let mut c = cfg;
            edit(&mut c.spectrogram_gradient);
            c
        };
        for edit in [
            (|g: &mut Gradient| g.hue_start += 10.0) as fn(&mut Gradient),
            |g: &mut Gradient| g.hue_span -= 10.0,
            |g: &mut Gradient| g.lightness -= 5.0,
            |g: &mut Gradient| g.lightness_ramp -= 5.0,
            |g: &mut Gradient| g.chroma -= 0.1,
            |g: &mut Gradient| g.chroma_ramp += 0.1,
        ] {
            let moved = edited(edit);
            assert!(rebuilds(&moved), "a gradient knob left the old table on the GPU");
            assert_ne!(table(&moved), table(&cfg), "the knob was expected to move a texel");
        }

        // And the level window does NOT: the dB floor, the ceiling and the tilt
        // reach a texel through the LEVEL it is looked up at, which is a
        // uniform, so a Level drag moves no entry of the table.
        for edit in [
            (|c: &mut SpectrumConfig| c.volume_floor_db -= 6.0) as fn(&mut SpectrumConfig),
            |c: &mut SpectrumConfig| c.volume_ceiling_db -= 6.0,
            |c: &mut SpectrumConfig| c.tilt += 1.0,
            |c: &mut SpectrumConfig| c.roll_seconds *= 1.01,
            |c: &mut SpectrumConfig| c.roll_fraction += 0.01,
        ] {
            let mut moved = cfg;
            edit(&mut moved);
            assert!(!rebuilds(&moved), "a drag on this rebuilds the table on every frame");
        }

        // The hue pair at NO CHROMA, which is the Mono preset and the one place
        // a gradient knob decides nothing. `chroma_at` is 0 at every level, so
        // the absolute chroma is 0 whatever the hue, and Oklab's `a` and `b` are
        // `c * cos(h)` and `c * sin(h)` — identically 0. Every texel is the same
        // grey at every angle, and the spectrum bar's track is a DRAG.
        let mono = crate::SpectrogramPreset::Mono.gradient();
        let toneless = SpectrumConfig { spectrogram_gradient: mono, ..cfg };
        for turned in [30.0f32, 180.0, 359.0] {
            let mut c = toneless;
            c.spectrogram_gradient.hue_start = (mono.hue_start + turned).rem_euclid(360.0);
            c.spectrogram_gradient.hue_span = turned;
            assert_eq!(
                what_decides_a_texel(c.spectrogram_gradient.sanitized()),
                what_decides_a_texel(toneless.spectrogram_gradient.sanitized()),
                "a hue turned {turned} degrees at no chroma rebuilds a table that was \
                 still good",
            );
            assert_eq!(table(&c), table(&toneless), "...and it really does draw the same table");
        }

        // The same fact along the OTHER axis: a Brightness pair closed on either
        // end of the `L*` axis. `HUE_FLOOR` is 0 at both 0 and 100, so
        // `chroma_of` answers 0 for every fraction and every hue, and
        // `oklab_srgb` is black (white at 100) whichever way the arc runs — the
        // whole picture is one colour, and neither the hue pair nor the chroma
        // pair decides a texel of it.
        //
        // Reachable in one gesture and landed on exactly, not by luck:
        // `Spread::snapped` rounds both handles to whole `L*`, and every preset
        // already opens with silence at 0, so closing the pair down to the wall
        // is where a reader ends up. Reaching for the hue track or the Chroma
        // bar to get back OUT is then a drag rebuilding the table every frame
        // while changing not one entry.
        for l in [0.0f32, 100.0] {
            let flat = SpectrumConfig {
                spectrogram_gradient: Gradient {
                    lightness: l,
                    lightness_ramp: 0.0,
                    chroma: 0.5,
                    chroma_ramp: 0.0,
                    hue_start: 40.0,
                    hue_span: 120.0,
                },
                ..cfg
            };
            for edit in [
                (|g: &mut Gradient| g.hue_start = (g.hue_start + 90.0).rem_euclid(360.0))
                    as fn(&mut Gradient),
                |g: &mut Gradient| g.hue_span = -300.0,
                |g: &mut Gradient| g.chroma = 0.9,
                |g: &mut Gradient| g.chroma_ramp = 0.2,
            ] {
                let mut moved = flat;
                edit(&mut moved.spectrogram_gradient);
                assert_eq!(
                    what_decides_a_texel(moved.spectrogram_gradient.sanitized()),
                    what_decides_a_texel(flat.spectrogram_gradient.sanitized()),
                    "at a Brightness pair closed on L* {l} the picture is one colour, and a \
                     drag on the hue or the chroma rebuilds the table every frame",
                );
                assert_eq!(table(&moved), table(&flat), "...and the table really is the same");
            }
        }

        // The converses, so neither fold can be a blanket "ignore the hues": a
        // picture WITH chroma watches them, and so does a brightness pair closed
        // anywhere BETWEEN the ends, which still draws a colour.
        let mut coloured = toneless;
        coloured.spectrogram_gradient.chroma = 0.5;
        let mut turned = coloured;
        turned.spectrogram_gradient.hue_start += 40.0;
        assert_ne!(table(&turned), table(&coloured), "a coloured ramp must watch its hues");
        let mid = SpectrumConfig {
            spectrogram_gradient: Gradient {
                lightness: 50.0,
                lightness_ramp: 0.0,
                chroma: 0.5,
                chroma_ramp: 0.0,
                hue_start: 40.0,
                hue_span: 120.0,
            },
            ..cfg
        };
        let mut mid_turned = mid;
        mid_turned.spectrogram_gradient.hue_start += 90.0;
        assert_ne!(
            what_decides_a_texel(mid_turned.spectrogram_gradient.sanitized()),
            what_decides_a_texel(mid.spectrogram_gradient.sanitized()),
            "a flat ramp between the ends still draws a colour",
        );
    }

    /// The run key names WHICH COLUMNS were folded, and nothing about how they
    /// are read.
    ///
    /// Both directions again. A field that decides the run and is missing here
    /// draws a stale run — the picture stops at a column that has already
    /// scrolled past. A field that decides nothing about the run re-folds the
    /// store on every frame of the drag that moves it, and every one of those
    /// is now a drag that moves a uniform: the zoom, the row count and the
    /// palette all read the same bytes a different way.
    #[test]
    fn the_run_key_names_the_fold_and_not_the_read() {
        let scale = PitchScale { min_midi: 40.0, max_midi: 88.0, span: 48.0 };
        let base_view = PaneView {
            ppp: 2.0,
            pitch_len: 300.0,
            depth_len: 800.0,
            window: 12.0,
            scale,
            cfg: SpectrumConfig::default(),
            whole: false,
        };
        let columns = Columns { first: 3, len: 200, newest: 5.0 };
        let key = |v: &PaneView, c: &Columns| Plan::new(v, c).key;
        let base = key(&base_view, &columns);
        assert_eq!(base, key(&base_view, &columns), "the same fold must hit");

        // Which columns are in the window, and where the newest one sits: each
        // moves as the window scrolls, and the run moves with it.
        for moved in [
            Columns { first: 4, ..columns },
            Columns { len: 201, ..columns },
            Columns { newest: 6.0, ..columns },
        ] {
            assert_ne!(base, key(&base_view, &moved), "a scrolled window reuses the old run");
        }
        // The slab width, through the ladder, and the build path.
        assert_ne!(base, key(&PaneView { depth_len: 200.0, ..base_view }, &columns));
        assert_ne!(base, key(&PaneView { whole: true, ..base_view }, &columns));

        // And everything that is a UNIFORM: the rows, the pitch range and every
        // colour input read the run rather than deciding it, so none of them may
        // refold a thing.
        let read_only = |v: PaneView| {
            assert_eq!(base, key(&v, &columns), "a uniform change refolded the store");
        };
        read_only(PaneView { pitch_len: 900.0, ..base_view });
        read_only(PaneView { ppp: 1.0, pitch_len: 600.0, ..base_view });
        read_only(PaneView {
            scale: PitchScale { min_midi: 60.0, max_midi: 84.0, span: 24.0 },
            ..base_view
        });
        for edit in [
            (|c: &mut SpectrumConfig| c.volume_floor_db -= 6.0) as fn(&mut SpectrumConfig),
            |c: &mut SpectrumConfig| c.volume_ceiling_db -= 6.0,
            |c: &mut SpectrumConfig| c.tilt += 1.0,
            |c: &mut SpectrumConfig| c.spectrogram_gradient.hue_start += 40.0,
            |c: &mut SpectrumConfig| c.roll_fraction += 0.01,
        ] {
            let mut cfg = base_view.cfg;
            edit(&mut cfg);
            read_only(PaneView { cfg, ..base_view });
        }
    }

    /// The strip is drawn out to the now-line, but the newest column is always
    /// older than that — half an analysis window, by construction. The shader
    /// clamps a tap into the run, so a coordinate allowed to run on holds the
    /// newest slab's colour across the whole sliver rather than reading a slab
    /// that does not exist — but only the coordinate says where the newest slab
    /// IS, and it must stop at that slab's CENTRE, which is where the run's own
    /// filtering stops having two taps to blend.
    #[test]
    fn the_leading_sliver_holds_the_newest_slabs_centre() {
        // A live window at the settings that make the sliver widest: a short
        // span, so slabs sit on the ladder's lowest rung and the analyzer's lag
        // spans several of them.
        let bucket = crate::AudioSpectrum::FFT_INTERVAL * LADDER_FLOOR_COLUMNS;
        let window = 2.0;
        let visible = (window / bucket) as usize; // 62 slabs
        let layout = TexLayout { bucket, t_origin: 400.0, tex_span: visible as f64 * bucket };
        // The now-line: the newest column lags by half a Precise window, and
        // its slab has to finish before the next one starts.
        let now = layout.t_origin + layout.tex_span + 0.171;

        let newest = slab_drawn(&layout, now);
        assert!(
            newest <= visible as f32 - 0.5 + 1e-4,
            "the sliver reached {newest} slabs in, past the newest slab's centre at {}",
            visible as f32 - 0.5,
        );
        // And it is the newest slab it holds, not something short of it.
        assert!(newest >= visible as f32 - 0.5 - 1e-4, "held short of the newest slab");
        // Worth pinning for, because an unheld mapping runs several slabs past
        // the run rather than a fraction of one.
        let unheld = slab_at(&layout, now);
        assert!(unheld > visible as f32 + 4.0, "expected a multi-slab overrun, got {unheld}");
    }

    /// Everything BEFORE the hold is untouched by it: the drawn mapping is the
    /// plain one over the data, so the picture still tracks the notes slab for
    /// slab, and the hold is a corner rather than a bend that creeps inward.
    #[test]
    fn holding_the_sliver_leaves_the_data_mapping_alone() {
        let bucket = 0.05;
        let layout = TexLayout { bucket, t_origin: 10.0, tex_span: 20.0 * bucket };
        let hold = hold_time(&layout);
        assert!((hold - (layout.t_origin + layout.tex_span - 0.5 * bucket)).abs() < 1e-9);
        let mut t = layout.t_origin - 2.0 * bucket;
        while t <= hold {
            assert_eq!(slab_drawn(&layout, t), slab_at(&layout, t), "bent at {t}");
            t += bucket / 8.0;
        }
        // Past it, pinned — however far past, and however long the analyzer
        // stalls for.
        let pinned = slab_at(&layout, hold);
        for t in [hold + 1e-6, hold + bucket, hold + 10.0] {
            assert_eq!(slab_drawn(&layout, t), pinned, "ran on at {t}");
        }
    }

    /// The time -> slab mapping has to be a straight line, including across the
    /// slab boundary the newest column sits on. Clamping it to the run's end
    /// pins it for part of every slab and lets it slide for the rest, and since
    /// this is a VERTEX attribute that rescales the whole picture once per slab
    /// — visible as the heatmap jittering. Which is why the one place the
    /// coordinate does stop ([`slab_drawn`]) is a corner the mesh is SPLIT on,
    /// leaving no quad to interpolate across it and the data quad straight from
    /// end to end.
    #[test]
    fn the_time_to_slab_mapping_is_a_straight_line() {
        let bucket = 0.08;
        let layout = TexLayout { bucket, t_origin: 100.0, tex_span: 8.0 * bucket };
        let step = bucket / 4.0;
        let at = |i: i32| slab_at(&layout, layout.t_origin + i as f64 * step);

        // Equal steps in time, equal steps in the coordinate — everywhere,
        // including past BOTH ends of the run where the quad reaches for its
        // slivers.
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
