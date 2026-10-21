//! The spectrogram heatmap's data path: the slab grid an incoming column folds
//! into, and the GPU's copy of that grid the fragment shader reads.
//!
//! The grid travels as DATA rather than as a picture — `capacity` slots of one
//! slab's stored dB bytes each — so a pitch zoom, a resize, a Level drag or a
//! palette change normally moves only uniforms. A larger retention can also
//! recover a clipped gap (see [`FoldedGrid::hit`](crate::spectrogram::FoldedGrid::hit)). What a frame owes the
//! GPU is an immutable snapshot of the slabs it draws. The renderer owns
//! upload history; the read that turns those bytes into pixels lives in
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
use harmonigraph_render::{SpectrogramGrid, SpectrogramRead, SpectrogramShades, SpectrogramSlab};

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
/// which must be at least as large), so this puts bfloat16 linear-power history at
/// about 54 MiB, twice the former byte store. What the larger one buys is the SHORT
/// spans, which is where a halving still lands somewhere the data can tell
/// apart: a 12 s close-up is cut into 16 ms slabs here and 32 ms ones at half
/// this, against an 8 ms column rate. At a three-minute Span it is 256 ms
/// against 512, and both are far coarser than the data —
/// the cap still doubles the resolution there, but neither is drawing grain.
///
/// It is a CEILING on the count, not the count itself: [`live_slab`] picks the
/// finest rung of its ladder that fits a window inside this many slabs, so the
/// image holds between half of them and all of them.
pub(crate) const LIVE_SLAB_CAP: f32 = 1024.0;
/// The live grid's finest rung, in analysis intervals — see [`live_slab`].
///
/// TWO, not one: the column grid and the slab grid share a period on the ladder
/// but not a phase (columns land on a sample counter, slabs on absolute time),
/// so at one column per slab a boundary falling mid-interval leaves some slabs
/// empty, and the uniform time axis then stretches the columns either side of an
/// empty slab into a flat streak. At two, a phase offset costs a slab one of its
/// columns and never both.
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
/// under the analysis window (171 ms at the fresh Balanced setting and 48 kHz), so
/// the stepping loses detail the FFT never had. At a three-minute Span the
/// slab is 256 ms and the CAP is what sets the resolution: those slabs merge detail the FFT did
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

/// How far past a rung boundary the window must fall before the slab width
/// steps back DOWN, as a fraction of the boundary — see [`Plan::new`].
///
/// Eight drag-points of the Span gesture (`DEPTH_ZOOM_PER_DRAG_POINT` is 0.6%
/// of the window each). One point is what a resting finger clears on tremor
/// alone, which is the whole reason this exists; eight is past that and still
/// inside the 2x the rung itself is worth, so what the margin can cost the
/// picture is bounded by what the ladder already costs it.
const RUNG_HOLD: f64 = 0.05;

/// [`live_slab`]'s ladder with hysteresis: the rung `held` names, while the
/// window is still comfortably on it, and the fresh rung otherwise.
///
/// The ladder alone re-decides the rung from the window on every frame, so a
/// Span parked within a drag-point of a boundary alternates its slab width for
/// as long as the hand holds still. That is not merely frequent but expensive:
/// `bucket` is the aggregator's one layout input AND a [`RunKey`] field, so
/// both caches miss together and every frame pays a full refold and rewrites
/// every slab it folded. One drag-point is 0.6% of the Span, which a resting
/// finger clears on tremor alone.
///
/// The hold is on the way DOWN only, and the asymmetry is what keeps it safe
/// rather than being a preference. A held rung is always the COARSER one, so
/// the run it cuts is shorter than the one the pane sized its ring for; hold a
/// FINER rung past its boundary and the run outgrows `capacity`, which is two
/// keys in one slot. Stepping UP happens at the boundary, and stepping back
/// down waits for the window to fall [`RUNG_HOLD`] below it.
///
/// The rung below `b` takes over at `target_cols * b / 2`, which is what the
/// margin is measured against. A window far below that — a zoom that crossed
/// several rungs in one gesture — fails the test and takes the fresh rung, so
/// a hold cannot strand the grid at a width the window has left behind.
fn held_slab(window: f64, target_cols: usize, held: Option<f64>) -> f64 {
    let fresh = live_slab(window, target_cols);
    match held {
        Some(b) if b > fresh && window > target_cols as f64 * b * 0.5 * (1.0 - RUNG_HOLD) => b,
        _ => fresh,
    }
}

/// A run of empty slabs this short is a seam in the sample stream rather than a
/// stall in the analyzer, and holds the previous column instead of reading as
/// silence.
///
/// It used to absorb frame jitter, which was the common case: the FFT fired on
/// frame boundaries, so one long frame left a slab with nothing in it. Columns
/// now land on a sample grid finer than the narrowest live slab
/// (see `AudioSpectrum::push_samples`), so an ordinary stream cannot skip one at
/// all. What is left for this to cover is a real gap in the samples — a host
/// dropout, the pane being switched on, a transport jump re-anchoring the grid.
/// A stall, by contrast — switching the FFT window empties the ring for a
/// window's worth of samples, 341 ms at 48 kHz on Precise — is many slabs wide
/// and genuinely was silence as far as the analyzer is concerned.
const JITTER_SLABS: i64 = 1;

/// Where the visible slabs sit in absolute time — everything the mesh's slab
/// coordinate needs.
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
///   nothing. That is the Saturation bar dragged to 0 at both ends.
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
/// chroma bar is next raised off 0. The key's question is not "is this legal" but "does this decide
/// a texel", and only the key may answer it.
fn what_decides_a_texel(g: Gradient) -> Gradient {
    let toneless = g.chroma == 0.0 && g.chroma_ramp == 0.0;
    // The ends of the axis, and only the ends: a pair closed anywhere BETWEEN
    // them still draws a colour, so the hues decide a texel there. `sanitized`
    // is what makes the equality safe to write — it holds `lightness` inside
    // 0..=100 and keeps a -0.0 out of the ramp, so a pair at the wall lands on
    // exactly these two numbers.
    let unlit = g.lightness_ramp == 0.0 && (g.lightness == 0.0 || g.lightness == 100.0);
    // A straight curve applies to nothing, so which channels it is switched
    // onto decides nothing either; and a channel's switch decides nothing
    // where that channel does not.
    let fresh = harmonigraph_scene::Bend::default();
    let g = if g.bend.is_straight() { Gradient { bend: fresh, ..g } } else { g };
    match (toneless, unlit) {
        (_, true) => Gradient {
            hue_start: 0.0,
            hue_span: 0.0,
            chroma: 0.0,
            chroma_ramp: 0.0,
            bend: fresh,
            ..g
        },
        // Without tone, the hue and chroma switches no longer decide a color,
        // but whether EITHER is on still decides how the table is spaced.
        (true, false) => {
            let tone = g.bend.hue || g.bend.chroma;
            Gradient {
                hue_start: 0.0,
                hue_span: 0.0,
                bend: harmonigraph_scene::Bend { hue: tone, chroma: tone, ..g.bend },
                ..g
            }
        }
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
/// by under two levels at the default window (half a dB step of a 36 dB range,
/// across a 255-level ramp) and was settled by eye against a sixteen-bit store
/// — see `quantizing_a_bucket_does_not_move_its_colour`, which is the same
/// judgement made one layer down. Exactness would need a table per ROW, which is
/// several megabytes of texture per pane and slower than the read it serves.
pub(crate) const SHADES: usize = 4096;

/// Entry `i` of the gradient's table, which samples the gradient at each
/// entry's centre.
fn shade(gradient: Gradient, i: usize) -> egui::Color32 {
    crate::panes::spectral::spectrogram::cell_color(gradient, (i as f32 + 0.5) / SHADES as f32)
}

/// The colour the heatmap draws silence in: the table's first entry, which the
/// shader holds flat for every level under that entry's centre.
pub(crate) fn silence_color(gradient: Gradient) -> egui::Color32 {
    shade(gradient, 0)
}

/// The gradient's table as the shader reads it, and the gradient it stands for.
struct ShadeLut {
    /// The FOLDED gradient (see [`what_decides_a_texel`]) the entries were built
    /// for. Two gradients that fold together draw one picture, so one table
    /// serves both; the entries themselves come off the gradient as dialled.
    gradient: Gradient,
    lut: Arc<Vec<[u8; 4]>>,
}

/// CPU memo of the folded data and palette. Upload history belongs to the renderer.
#[derive(Default)]
pub(crate) struct FoldedGrid {
    run: Option<FoldedRun>,
    lut: Option<ShadeLut>,
    full_uploads: Arc<std::sync::atomic::AtomicU32>,
}

struct FoldedRun {
    key: RunKey,
    first_key: i64,
    capacity: usize,
    run: Arc<[SpectrogramSlab]>,
    layout: TexLayout,
}

impl FoldedGrid {
    /// Reuse the folded bytes only when their source and retained extent still fit.
    fn hit(&self, plan: &Plan, history: &crate::SpectrumHistory) -> Option<TexLayout> {
        let held = self.run.as_ref().filter(|held| held.key == plan.key)?;
        if history.get(plan.first).zip(history.back()).is_some_and(|(first, newest)| {
            let target = (first.time / plan.bucket).floor() as i64;
            let min_key = ((newest.time / plan.bucket).floor() as i64)
                .saturating_sub(plan.capacity.max(1) as i64 - 1);
            // The same admitted first slab as SpectrogramAgg::window.
            // Sampling clamps to this edge, so reusing a longer run would
            // change interpolation even if it contains every requested slab.
            target.max(min_key) != held.first_key
        }) {
            return None;
        }
        Some(held.layout)
    }

    fn accept(
        &mut self,
        key: RunKey,
        first_key: i64,
        capacity: usize,
        run: Arc<[SpectrogramSlab]>,
        layout: TexLayout,
    ) {
        debug_assert!(run.len() <= capacity);
        self.run = Some(FoldedRun { key, first_key, capacity, run, layout });
    }

    fn grid(&self) -> Option<SpectrogramGrid> {
        let held = self.run.as_ref()?;
        Some(SpectrogramGrid {
            capacity: held.capacity as u32,
            bins: SPECTRUM_BINS as u32,
            first_key: held.first_key,
            run: held.run.clone(),
            full_uploads: self.full_uploads.clone(),
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
            let lut = (0..SHADES).map(|i| shade(cfg.spectrogram_gradient, i).to_array()).collect();
            self.lut = Some(ShadeLut { gradient, lut: Arc::new(lut) });
        }
        let held = self.lut.as_ref().expect("built above when the fold moved");
        SpectrogramShades { lut: held.lut.clone() }
    }

    /// Full uploads taken since this surface was opened — see the field.
    pub(crate) fn full_uploads(&self) -> u32 {
        self.full_uploads.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Number of slabs in the currently memoized run, for test diagnostics.
    #[cfg(test)]
    pub(crate) fn run_slabs(&self) -> usize {
        self.run.as_ref().map_or(0, |run| run.run.len())
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
/// [`crate::text::ladder`]).
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
}

/// Which stored columns a frame draws — the other half of a [`Plan`]'s inputs.
///
pub(crate) struct Columns {
    /// The oldest in-window column; it advances as the window scrolls one off
    /// the far end.
    pub(crate) first: usize,
    pub(crate) len: usize,
    /// The newest column's time, which moves whenever a fresh column arrives —
    /// catching one even in a saturated store, where the count holds steady.
    pub(crate) newest: f64,
}

/// Source columns and slab width used to fold a run. The admitted oldest slab
/// is checked separately in [`FoldedGrid::hit`]: scrolling within one completed
/// slab cannot change its bytes, but crossing its edge changes shader sampling.
/// A fresh column changes `newest_bits`, even when the store's count is stable.
/// Pitch, rows, level and palette only change how the same bytes are read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RunKey {
    cols_len: usize,
    newest_bits: u64,
    bucket_bits: u64,
}

/// What this frame's heatmap needs folded, and the key that says whether the
/// memoized snapshot already IS it.
///
/// Pure, and deliberately so. Every cliff this pipeline has fallen off has been
/// in this arithmetic — a slab against the analyzer's lag, a window against the
/// store's finest tier — and deciding it apart from the fold means it can be
/// checked without a GPU, a store or a frame. See
/// `no_cache_layer_falls_back_as_the_window_scrolls`.
pub(crate) struct Plan {
    /// The picture's pitch-axis resolution in device pixels, which is what
    /// sets the footprint a fragment resamples the grid over. It bounds
    /// nothing on the GPU — the grid is bucket-space data — so the pane's own
    /// pixels are the whole of it.
    pub(crate) rows: usize,
    /// A time slab's width, in seconds.
    bucket: f64,
    /// Slabs the GPU's copy holds, and so the most the aggregator keeps folded.
    ///
    /// Sized off the PANE rather than off the window, which is what holds it
    /// still under a SPAN drag: the widest run a window can have at this slab
    /// width is `target_cols` of them, whatever the Span is doing inside that
    /// rung. What holds it still under a PANE drag is [`ring_slots`] — the
    /// pane's own pixel count moves every frame of one.
    capacity: usize,
    first: usize,
    pub(crate) key: RunKey,
}

impl Plan {
    /// The plan for a frame drawn after `held`, which is the slab width the
    /// previous frame drew at — `None` on the first frame of a surface.
    ///
    /// It reaches the slab width through [`held_slab`], which is where the
    /// ladder's hysteresis and the reason for it are. What that costs the
    /// picture is that the width near a rung boundary depends on which side the
    /// Span arrived from: zooming in holds the blockier grid for a further
    /// [`RUNG_HOLD`] of Span. That is the same factor of two the rung itself is,
    /// moved by a twentieth of a Span, against a picture that otherwise stops
    /// dead under a resting finger.
    pub(crate) fn new(view: &PaneView, columns: &Columns, held: Option<f64>) -> Plan {
        // One row per device pixel of the pitch axis, and one slab per pixel of
        // the depth axis, under the cap the slab count takes.
        let rows = pitch_pixels(view.pitch_len, view.ppp);
        let depth_px = (view.depth_len * view.ppp).round();
        let target_cols = depth_px.clamp(2.0, LIVE_SLAB_CAP) as usize;
        let bucket = held_slab(view.window, target_cols, held);
        let key = RunKey {
            cols_len: columns.len,
            newest_bits: columns.newest.to_bits(),
            bucket_bits: bucket.to_bits(),
        };
        Plan { rows, bucket, capacity: ring_slots(target_cols), first: columns.first, key }
    }
}

/// Pixels the pitch axis spends: the pane's points at the display's density,
/// floored at two so a footprint has a width to be.
///
/// One definition because TWO panes sample this axis — the heatmap's rows and
/// the spectrum curve's columns — and sharing the resample
/// ([`panes::spectral::spectrogram::footprint_mean_db`](crate::panes::spectral::spectrogram::footprint_mean_db))
/// is only half of sharing the read. The other half is the footprint handed to
/// it, which is one of these pixels wide on each side: counted in POINTS
/// instead, the curve would integrate `ppp` times as many buckets per column as
/// the heatmap does, and a ridge and the curve over it would sit at different
/// heights on any display that is not 1x.
/// Physical pixels per egui point for a spectral pane's two pictures, and the
/// single read of it: the heatmap declares a fragment's footprint from the row
/// count derived here (the shader's `0.5 / rows`) and the curve derives a
/// column width from the same number, so a floor applied to one read and not
/// the other puts a ridge and the curve drawn over it on different runs of
/// buckets.
///
/// Raw, because the counts downstream carry their own floors ([`pitch_pixels`]
/// takes `.max(2)`, [`Plan::new`] clamps its slab count). A floor HERE would
/// claim more pixels than the device has, and each fragment would then
/// integrate a fraction of the buckets it actually covers — the pane-height
/// dependence of #491, arrived at through the footprint instead of the pane.
pub(crate) fn pane_ppp(ctx: &egui::Context) -> f32 {
    ctx.pixels_per_point()
}

pub(crate) fn pitch_pixels(pitch_len: f32, ppp: f32) -> usize {
    ((pitch_len * ppp).round() as usize).max(2)
}

/// Columns the spectrum curve is drawn in: [`pitch_pixels`], capped so a very
/// wide pane cannot turn the fill into thousands of shapes a frame.
///
/// The cap is the one place the two counts part, and it parts them the safe
/// way — past it the curve UNDERSAMPLES a shared footprint rather than reading
/// a different run of buckets under every pixel.
pub(crate) fn curve_cols(pitch_len: f32, ppp: f32) -> usize {
    pitch_pixels(pitch_len, ppp).min(4096)
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

/// The scalars a fragment reads the grid through — the footprint's width and
/// the level mapping — for a pane spending `rows` pixels on the pitch axis.
pub(crate) fn read_of(view: &PaneView, rows: usize) -> SpectrogramRead {
    let (level0, level_per_step, level_per_midi) = level_affine(&view.cfg);
    SpectrogramRead {
        min_midi: view.scale.min_midi,
        span: view.scale.span,
        rows: rows as u32,
        spectrum_min_midi: SPECTRUM_MIN_MIDI,
        bins_per_semitone: BINS_PER_SEMITONE as f32,
        level0,
        level_per_step,
        level_per_midi,
    }
}

/// Fold the run a [`Plan`] describes into the surface's immutable snapshot.
/// Returns where the visible slabs sit in time, or `None` if the fold came out
/// too short to draw.
pub(crate) fn build(
    history: &crate::SpectrumHistory,
    surfaces: &mut crate::spectrum::SpectrogramSurfaces,
    surface: usize,
    plan: &Plan,
) -> Option<TexLayout> {
    let bucket = plan.bucket;
    // Aggregate the in-window columns into one slab per depth pixel by a FIXED
    // time grid, mean linear power within each slab. Slabs stay in BUCKET
    // space: the rows read them in the fragment shader, so the fold is blind to
    // the pitch axis and a zoom or pan of it re-reads this grid instead of
    // re-walking the store.
    let agg = surfaces.at(surface).agg.get_or_insert_with(SpectrogramAgg::new);
    let (centers, power) = agg.window(history, plan.first, bucket, plan.capacity);
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
    let capacity = ring_capacity(plan.capacity, w);
    let layout = TexLayout { bucket, t_origin, tex_span };
    surfaces.at(surface).folded.accept(plan.key.clone(), first_key, capacity, power, layout);
    Some(layout)
}

/// The run this frame draws, and where it sits in time: the one already on the
/// surface when the source key and oldest slab still fit, and a fresh fold
/// otherwise.
pub(crate) fn run_for(
    history: &crate::SpectrumHistory,
    surfaces: &mut crate::spectrum::SpectrogramSurfaces,
    surface: usize,
    plan: &Plan,
) -> Option<TexLayout> {
    // The next frame retains this slab width until it crosses a ladder rung.
    surfaces.at(surface).held_bucket = Some(plan.bucket);
    match surfaces.at(surface).folded.hit(plan, history) {
        Some(layout) => Some(layout),
        None => build(history, surfaces, surface, plan),
    }
}

/// The grid and the gradient table this frame's callback carries.
///
/// `None` before anything has been folded into the surface, which is the same
/// frame [`run_for`] answers `None` on.
pub(crate) fn frame_data(
    surfaces: &mut crate::spectrum::SpectrogramSurfaces,
    surface: usize,
    cfg: &SpectrumConfig,
) -> Option<(SpectrogramGrid, SpectrogramShades)> {
    let gpu = &mut surfaces.at(surface).folded;
    let grid = gpu.grid()?;
    Some((grid, gpu.shades(cfg)))
}

/// The same accumulator serves incremental and batch aggregation.
/// f64 summation avoids accumulating rounding error during a long live slab.
/// History sums are bfloat16 (at most 64 samples per group), so regrouping can
/// still differ by a final byte right at a quantization boundary.
#[derive(Clone)]
struct PowerMean {
    sum: Box<[f64; SPECTRUM_BINS]>,
    count: u64,
}

impl Default for PowerMean {
    fn default() -> Self {
        Self { sum: Box::new([0.0; SPECTRUM_BINS]), count: 0 }
    }
}

impl PowerMean {
    fn add(&mut self, col: &crate::SpectrogramColumn) {
        for (sum, fresh) in self.sum.iter_mut().zip(col.power_sum()) {
            *sum += fresh as f64;
        }
        self.count += col.count as u64;
    }

    fn write(&self, out: &mut [BucketDb]) {
        for (out, &sum) in out.iter_mut().zip(self.sum.iter()) {
            *out = harmonigraph_core::spectrogram::quantize(if self.count == 0 {
                0.0
            } else {
                (sum / self.count as f64) as f32
            });
        }
    }

    fn clear(&mut self) {
        self.sum.fill(0.0);
        self.count = 0;
    }
}

/// Completed slabs stay byte-sized. Only the active slab retains sums/counts;
/// continuing it after a display read never averages its quantized output.
///
/// One shared [`SpectrogramSlab`] per slab rather than one flat byte run, so a
/// frame's snapshot is a list of pointers and the renderer can tell the slabs
/// that moved from the ones that did not without reading either. The contract
/// that makes that exact is the one [`SlabGrid::finish`] keeps: a slab is
/// written only through `Arc::make_mut`, so one a snapshot still holds is
/// copied rather than changed under it.
#[derive(Default, Clone)]
struct SlabGrid {
    mean: PowerMean,
    dirty: bool,
    centers: Vec<f64>,
    power: Vec<SpectrogramSlab>,
    cur_key: Option<i64>,
}

/// A fresh slab of silence, which is what every slab starts as.
fn silent_slab() -> SpectrogramSlab {
    SpectrogramSlab::from([0; SPECTRUM_BINS])
}

impl SlabGrid {
    /// Fold one column (columns arrive oldest-first) into the grid, appending
    /// slabs and adding the column to the current power mean. Returns `false` iff
    /// the column ran BACKWARDS in time relative to the current slab — batch
    /// ignores the result (it just starts a fresh row, as before), while the
    /// incremental aggregator treats it as a broken invariant and rebuilds.
    /// `min_key` is an explicit live retention bound; the batch driver passes
    /// `None`. One older seed may survive until an admitted column arrives.
    fn fold(&mut self, col: &crate::SpectrogramColumn, bucket: f64, min_key: Option<i64>) -> bool {
        let key = (col.time / bucket).floor() as i64;
        if self.cur_key != Some(key) {
            self.finish();
            self.mean.clear();
        }
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
                self.fill_gap(k, key, bucket, min_key);
                self.centers.push((key as f64 + 0.5) * bucket);
                self.power.push(silent_slab());
                self.cur_key = Some(key);
                true
            }
            // First column (None), or time running backwards (Some, key < k, a
            // transport jump): start a fresh row rather than fill a negative
            // gap. Only the backward case breaks the incremental invariant.
            other => {
                self.cur_key = Some(key);
                self.centers.push((key as f64 + 0.5) * bucket);
                self.power.push(silent_slab());
                other.is_none()
            }
        };
        self.mean.add(col);
        self.dirty = true;
        if let Some(min_key) = min_key {
            self.retain_from(min_key, bucket);
        }
        forward
    }

    /// The rows for the empty slabs strictly between finished slab `from` and
    /// slab `to`.
    fn fill_gap(&mut self, from: i64, to: i64, bucket: f64, min_key: Option<i64>) {
        let empty = to - from - 1;
        // The live owner admits only its retained interval. Keep the
        // original gap length for the hold decision: clipping a long
        // gap down to one empty slab must not turn silence into a hold.
        for slot in (from + 1).max(min_key.unwrap_or(i64::MIN))..to {
            self.centers.push((slot as f64 + 0.5) * bucket);
            if empty <= JITTER_SLABS {
                // Hold the previous column: a gap this short is a seam
                // in the sample stream (see `JITTER_SLABS`), and
                // painting it silent would leave a stripe of false
                // silence scrolling across the display for the rest
                // of the window. The held slab is already final, so
                // the copy shares it.
                let held = self.power.last().expect("a current slab").clone();
                self.power.push(held);
            } else {
                self.power.push(silent_slab());
            }
        }
    }

    /// Continue with `next`, a grid folded from the columns that follow this
    /// one's, starting at a LATER slab: what folding those columns here would
    /// have built. Folding a column touches only its own slab, so the
    /// one thing that crosses the seam is the gap between the two — its rows,
    /// and the hold that copies this grid's last slab into them.
    fn append(&mut self, mut next: SlabGrid, bucket: f64, min_key: Option<i64>) {
        self.finish();
        let first = next.centers.first().map(|&c| (c / bucket).floor() as i64);
        if let (Some(k), Some(first)) = (self.cur_key, first) {
            self.fill_gap(k, first, bucket, min_key);
        }
        self.centers.append(&mut next.centers);
        self.power.append(&mut next.power);
        self.cur_key = next.cur_key;
        self.mean = next.mean;
        self.dirty = next.dirty;
        if let Some(min_key) = min_key {
            self.retain_from(min_key, bucket);
        }
    }

    /// Fold `columns` in order into a fresh grid — exactly the grid
    /// [`fold`](Self::fold) builds from them one by one — cut into up to
    /// `runs` runs of whole slabs that are folded in parallel and then
    /// [appended](Self::append). The bytes are identical because a slab's sums
    /// start from zero and take its own columns in the same order either way.
    fn fold_runs(
        columns: &[&crate::SpectrogramColumn],
        bucket: f64,
        min_key: Option<i64>,
        runs: usize,
    ) -> SlabGrid {
        let fold = |run: &[&crate::SpectrogramColumn]| {
            let mut grid = SlabGrid::default();
            for col in run {
                grid.fold(col, bucket, min_key);
            }
            // Quantizing is the other half of the cost; do it on the worker.
            grid.finish();
            grid
        };
        let key = |i: usize| (columns[i].time / bucket).floor() as i64;
        // Cut near each equal share, moved forward onto a step to a later slab
        // (a backward step, which the store never makes, is never a seam).
        let mut cuts = vec![0];
        for r in 1..runs {
            let mut cut = (columns.len() * r / runs).max(cuts[cuts.len() - 1] + 1);
            while cut < columns.len() && key(cut) <= key(cut - 1) {
                cut += 1;
            }
            if cut < columns.len() {
                cuts.push(cut);
            }
        }
        cuts.push(columns.len());
        if cuts.len() <= 2 {
            return fold(columns);
        }
        let fold = &fold;
        let grids: Vec<SlabGrid> = std::thread::scope(|s| {
            // This runs on the host's GUI thread, where a panic takes the plugin
            // down: a run whose thread the OS refuses is folded here instead.
            let workers: Vec<_> = cuts[1..]
                .windows(2)
                .map(|w| {
                    let run = &columns[w[0]..w[1]];
                    (run, std::thread::Builder::new().spawn_scoped(s, move || fold(run)).ok())
                })
                .collect();
            let first = fold(&columns[..cuts[1]]);
            std::iter::once(first)
                .chain(workers.into_iter().map(|(run, worker)| match worker {
                    Some(w) => w.join().unwrap_or_else(|e| std::panic::resume_unwind(e)),
                    None => fold(run),
                }))
                .collect()
        });
        let mut grids = grids.into_iter();
        let mut grid = grids.next().expect("at least two runs");
        for next in grids {
            grid.append(next, bucket, min_key);
        }
        grid
    }

    fn finish(&mut self) {
        if self.dirty {
            // Through `make_mut`: the last frame's snapshot may hold this slab,
            // and it must go on reading what it was handed.
            let open = self.power.last_mut().expect("a dirty grid has a current slab");
            self.mean.write(Arc::make_mut(open));
            self.dirty = false;
        }
    }

    /// Keep one predecessor when the whole grid is older than the interval.
    /// It seeds a clipped jitter hold and establishes a real black gap; the
    /// next admitted column removes it after any copies have been made.
    fn retain_from(&mut self, min_key: i64, bucket: f64) {
        let drop = self
            .centers
            .partition_point(|&c| ((c / bucket).floor() as i64) < min_key)
            .min(self.centers.len().saturating_sub(1));
        self.centers.drain(..drop);
        self.power.drain(..drop);
    }

    /// Normal geometric growth is at most twice the admitted slabs plus a
    /// predecessor. Release capacity from a larger previous pane/budget at
    /// admission, before folding anything, rather than retaining it forever.
    fn shrink_excess(&mut self, keep: usize) {
        let bound = 2 * (keep.max(1) + 1);
        if self.centers.capacity() > bound.max(4) {
            self.centers.shrink_to_fit();
        }
        if self.power.capacity() > bound {
            self.power.shrink_to_fit();
        }
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
/// and the refold adds linear powers, then quantizes once per slab.
///
/// **A folded slab is never recomputed, even when the store re-writes the
/// columns behind it.** Columns arrive in time order, so no future column can
/// land in a slab the newest one has already passed: the slab is FINAL the
/// moment it is behind the front. What is not final is the STORE — past
/// `SpectrumHistory`'s finest tier, columns are power-summed in pairs and re-timed
/// to their midpoint as they age. Re-deriving an old slab from the merged store
/// therefore answers a slightly different question than folding the raw columns
/// did — a merged pair lands in one slab rather than straddling two, smearing
/// its energy across a boundary the raw columns respected — so the two
/// disagree, and this keeps what it folded from the finer data rather than
/// re-reading the coarser.
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

/// The columns a full refold folds: the retention and one complete
/// predecessor slab. The seed supplies a clipped hold, or proves the silent
/// leading interval when the source jumped. No skipped prefix is ever expanded
/// into rows.
fn refold_columns(
    history: &crate::SpectrumHistory,
    bucket: f64,
    min_key: Option<i64>,
) -> Vec<&crate::SpectrogramColumn> {
    let mut start = min_key
        .map_or(0, |min| history.partition_point(|c| ((c.time / bucket).floor() as i64) < min));
    if let Some(previous) = start.checked_sub(1).and_then(|i| history.get(i)) {
        let seed = (previous.time / bucket).floor() as i64;
        start = history.partition_point(|c| ((c.time / bucket).floor() as i64) < seed);
    }
    history.iter_from(start).collect()
}

/// Most threads a full refold splits across. The add is memory-bound: on an
/// M1 Pro one thread streams a full-cap history (7,168 columns, 110 MB) in
/// about 7 ms, four in about 3, and more buy little.
const REFOLD_THREADS: usize = 4;

/// Fewest columns worth a refold thread of their own: a thread costs tens of
/// microseconds to start, a column a microsecond or so to fold.
const REFOLD_RUN_COLUMNS: usize = 256;

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
    fn rebuild(&mut self, history: &crate::SpectrumHistory, bucket: f64, min_key: Option<i64>) {
        self.rebuilds += 1;
        let columns = refold_columns(history, bucket, min_key);
        let runs = (columns.len() / REFOLD_RUN_COLUMNS).clamp(
            1,
            std::thread::available_parallelism().map_or(1, |n| n.get()).min(REFOLD_THREADS),
        );
        self.grid = SlabGrid::fold_runs(&columns, bucket, min_key, runs);
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
    ) -> (Vec<f64>, Arc<[SpectrogramSlab]>) {
        let target = history.get(first).map(|c| (c.time / bucket).floor() as i64);
        let newest = history.back().map_or(f64::NEG_INFINITY, |c| c.time);
        let min_key = history
            .back()
            .map(|c| ((c.time / bucket).floor() as i64).saturating_sub(keep.max(1) as i64 - 1));
        let layout_same = self.bucket_bits == bucket.to_bits();
        // The fast path is valid only when: the layout is unchanged, we have a
        // prior grid, time hasn't gone backwards, and the admitted first slab
        // doesn't reach behind the kept grid. New columns can extend its newest
        // end, including across a gap. An excluded old target must not
        // force another rebuild on every resumed frame.
        let can_increment = layout_same
            && self.grid.cur_key.is_some()
            && newest >= self.last_time
            && target.zip(self.grid.centers.first()).is_some_and(|(t, &front_center)| {
                let front = (front_center / bucket).floor() as i64;
                t.max(min_key.unwrap_or(t)) >= front
            });

        if !can_increment {
            self.rebuild(history, bucket, min_key);
        } else {
            if let Some(min_key) = min_key {
                self.grid.retain_from(min_key, bucket);
                self.grid.shrink_excess(keep);
            }
            // Fold only columns newer than the last we folded.
            let start = history.partition_point(|c| c.time <= self.last_time);
            let mut forward = true;
            for col in history.iter_from(start) {
                if !self.grid.fold(col, bucket, min_key) {
                    forward = false;
                    break;
                }
                self.last_time = col.time;
            }
            if !forward {
                // A mid-stream backward jump broke the grid; rebuild clean.
                self.rebuild(history, bucket, min_key);
            }
        }
        self.grid.finish();
        self.view(bucket, target)
    }

    /// Slice the complete slabs at the admitted oldest boundary. Completed
    /// averages and jitter copies stay unchanged as individual source columns
    /// scroll out. Keeping the preceding slabs internally lets a widening Span
    /// or a history-tier merge reach back without refolding the retained grid.
    ///
    /// The slabs come out shared, not copied — see [`SlabGrid`].
    fn view(&self, bucket: f64, target: Option<i64>) -> (Vec<f64>, Arc<[SpectrogramSlab]>) {
        let (Some(t), Some(&front_center)) = (target, self.grid.centers.first()) else {
            return (self.grid.centers.clone(), self.grid.power.as_slice().into());
        };
        let front = (front_center / bucket).floor() as i64;
        let kept = self.grid.centers.len().saturating_sub(1) as i64;
        let start = (t - front).clamp(0, kept) as usize;
        (self.grid.centers[start..].to_vec(), self.grid.power[start..].into())
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
/// The live strip extends to the now-line while fresh, beyond the newest
/// measurement by the analysis delay. That sliver holds this centre's value.
pub(crate) fn hold_time(layout: &TexLayout) -> f64 {
    layout.t_origin + layout.tex_span - 0.5 * layout.bucket
}

/// The slab coordinate for a time on the DRAWN strip: [`slab_at`] out to
/// [`hold_time`], and pinned there past it.
///
/// The shader clamps its taps into the run, which covers half a slab at the far
/// end — the strip stops at the oldest slab's leading edge, half a slab before
/// its centre. The newest end can extend several slabs past the run while
/// covering the analysis delay, so pin its coordinate to hold the whole sliver.
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

/// Slabs of headroom [`ring_capacity`] holds past the rung [`ring_slots`] rounds
/// the pane's own column count up to, which is at least `target_cols` — so it
/// holds still across a Span drag and across most of a pane one. Four covers
/// the breathing described there (a column's spacing at the far edge is at most
/// one slab, plus a floor at each end); the rest is margin, and
/// `ring_capacity`'s body leans on the whole eight.
const RING_HEADROOM: usize = 8;

/// Slots to give the ring for a pane asking for `target_cols` slabs: the count
/// rounded UP to a power of two, plus the headroom.
///
/// A ladder for the same reason [`live_slab`] is one, against the other drag.
/// The slot a key lands in is `key mod capacity`, so every change of capacity
/// moves the whole mapping — the buffer is reallocated and every slab in it
/// re-sent ([`FoldedGrid::accept`], and `SpectrogramCallback::prepare` behind it).
/// Taken straight off the pane, that is a fresh megabytes-sized buffer and a
/// whole-ring upload on EVERY FRAME of a pane resize, since a horizontal drag
/// moves the depth axis by a pixel or more per frame. On the ladder a drag
/// crosses ten of them end to end.
///
/// Rounding up rather than down is what keeps the run inside the ring: it is at
/// most `target_cols + 2` slabs (see [`ring_capacity`]), and the headroom sits
/// past a power of two that is already at least `target_cols`.
///
/// The slack costs nothing the ring was not already sized for. `target_cols` is
/// capped at [`LIVE_SLAB_CAP`], itself a power of two, so the widest ring this
/// can ask for is the one the widest pane already asks for.
fn ring_slots(target_cols: usize) -> usize {
    target_cols.next_power_of_two() + RING_HEADROOM
}

/// The level the heatmap's pixels actually go through, for the crate's own
/// tests — the bridge `quantizing_a_bucket_does_not_move_its_colour` holds the
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
mod tests;
