//! The spectrogram heatmap's data path: the slab pyramid every incoming column
//! folds into ([`SlabPyramid`](crate::spectrogram::SlabPyramid)), and the GPU's copy of the run a surface reads
//! out of it, which the fragment shader reads.
//!
//! The grid travels as DATA rather than as a picture — [`RING_SLOTS`](crate::spectrogram::RING_SLOTS) slots of
//! one slab's stored dB bytes each — so a pitch zoom, a resize, a Level drag or a
//! palette change normally moves only uniforms. What a frame owes the
//! GPU is an immutable snapshot of the slabs it draws. The renderer owns
//! upload history; the read that turns those bytes into pixels lives in
//! [`harmonigraph_render`]'s shader.
//! [`panes::spectral::spectrogram`] is the
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
/// It also sizes the slab pyramid: each of the finest levels keeps a ring this
/// wide (and its headroom, [`RING_SLOTS`]), 30 MB across the pyramid.
///
/// Raising this is not free: each of those rings grows with it. What the larger one buys is the SHORT
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

/// Slabs the GPU's copy of a surface's run holds, whatever the pane's size:
/// the widest window [`LIVE_SLAB_CAP`] cuts, and eight more.
///
/// One size for every surface, so a pane resize or a rung change never
/// reallocates the buffer or moves the slot a key lands in (`key mod
/// RING_SLOTS`). It costs 4.2 MB of GPU buffer per drawn surface. Eight is
/// headroom over the run's breathing: it reaches from one slab before the
/// slab the window's far edge is in to the newest column's slab, and both
/// ends are floored onto the slab grid. No pyramid level keeps more than
/// this many slabs, so a run read out of one always fits.
pub(crate) const RING_SLOTS: usize = LIVE_SLAB_CAP as usize + 8;
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
/// the slab pyramid keeps a level per rung of — every rung is a power of two
/// analysis intervals. Two things follow.
///
/// **The grid holds still.** A slab width taken straight from the window moves
/// on every frame of a Span drag, and a moved slab width re-lays the
/// surface's run and re-uploads every slab. On the ladder it moves only when
/// the Span crosses a doubling.
///
/// **Every rung is already folded.** A surface that steps to another rung
/// reads that level, which has folded every column since it arrived, so a
/// Span change draws the history exactly as it was drawn at that rung before.
///
/// The cost is resolution that steps rather than tracks: within a rung a wider
/// pane buys nothing, so the picture can hold half the slabs the depth axis has
/// pixels. What that gives up depends on the Span, and the two ends differ in
/// kind. At a close-up — a 12 s window cuts into 16 ms slabs — the slab is well
/// under the analysis window (85 ms at the fresh Fast setting and 48 kHz), so
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
/// every step reads another level, so every frame re-uploads every slab. One
/// drag-point is 0.6% of the Span, which a resting finger clears on tremor
/// alone.
///
/// The hold is on the way DOWN only, and the asymmetry is what keeps it safe
/// rather than being a preference. A held rung is always the COARSER one, so
/// the run it cuts is shorter than the pane has pixels for; hold a FINER rung
/// past its boundary and the window needs more slabs than the pane has
/// pixels, or than [`LIVE_SLAB_CAP`] allows. Stepping UP happens at the boundary, and stepping back
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

/// Every drawing surface's heatmap state, indexed by the surface id its copy of
/// the pane was drawn with: 0 the docked Spectral pane, 1 the Video pane's
/// preview, and offline the placement's index in the layout (see
/// [`draw_panes`](crate::draw_panes)).
///
/// Each surface reads a run of its own: the slab width comes off the pane's
/// own depth in points, so two placements at different sizes sharing one
/// surface would each replace the other's run every frame.
///
/// What stays SHARED is the analyzer and the slab pyramid it fills
/// ([`AudioSpectrum::pyramid`](crate::AudioSpectrum::pyramid)): there is one selected input and one stream of
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
    /// Full uploads of the slab grid across every surface since the plugin
    /// was opened.
    ///
    /// CORRECT and expensive, which is the whole problem — it draws the right
    /// picture at many times the cost, so nothing on screen distinguishes a
    /// working delta upload from one that has quietly stopped. The overlay
    /// turns it into a rate, where "climbing" is the entire diagnosis. The
    /// Tape heatmaps' count with the live ones'.
    pub(crate) fn full_uploads(&self) -> u32 {
        self.all()
            .flat_map(|s| std::iter::once(&s.folded).chain(&s.tape))
            .map(FoldedGrid::full_uploads)
            .sum()
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
    /// [`FoldedGrid`].
    pub(crate) folded: FoldedGrid,
    /// The pyramid level the previous frame drew at, which is what gives
    /// [`live_slab`]'s ladder its hysteresis —
    /// see [`Plan::new`]. `None` before the first frame.
    pub(crate) held_level: Option<usize>,
    /// The Tape heatmaps' runs, by [`TapeStrip`]: the far region's and the
    /// overview's, a memo each, since the two read different runs every
    /// frame they draw.
    pub(crate) tape: [FoldedGrid; 2],
}

/// Which of Tape mode's two heatmaps a run is drawn into.
#[derive(Clone, Copy, Debug)]
pub(crate) enum TapeStrip {
    /// The far region's.
    Region = 0,
    /// The overview's, the whole recording under the pane.
    Overview = 1,
}

/// Which slabs of the tape's spectrogram a Tape heatmap draws, and the copy
/// they were read from: the whole of what decides its run (#1869).
///
/// Slabs `first..=last` of `level`, a silent slab where the tape holds none,
/// read from the tape at [`revision`](crate::tape::spectrogram::TapeSpectrogram::revision).
/// The live heatmap's test of its newest slab cannot stand in for the
/// revision: the tape replaces slabs anywhere in its history, and a run
/// whose newest slab held still would draw the replaced ones stale with
/// nothing saying so. The revision is the whole tape's rather than the
/// run's, so the memo is rebuilt whenever the view's copy is written,
/// stretches off the window included: while recording, at each sync. A
/// rebuild costs a pointer per slab of the run; the renderer still uploads
/// only the slabs that changed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TapeRun {
    pub(crate) level: usize,
    pub(crate) first: i64,
    pub(crate) last: i64,
    pub(crate) revision: u64,
}

/// CPU memo of the run a surface drew and its palette. Upload history belongs
/// to the renderer.
#[derive(Default)]
pub(crate) struct FoldedGrid {
    run: Option<FoldedRun>,
    lut: Option<ShadeLut>,
    full_uploads: Arc<std::sync::atomic::AtomicU32>,
}

struct FoldedRun {
    first_key: i64,
    run: Arc<[SpectrogramSlab]>,
    from: RunSource,
}

/// What a held run was read out of, which is what decides whether the next
/// frame's run is the same one.
#[derive(Clone, Copy)]
enum RunSource {
    /// The live pyramid, where it sits in time: see [`FoldedGrid::accept`].
    Live(TexLayout),
    /// The tape's spectrogram: see [`TapeRun`].
    Tape(TapeRun),
}

impl FoldedGrid {
    /// Hold `run` and where it sits in time — or keep what is already held,
    /// when it is the same run: of the same level and first key, as long,
    /// and ending in the same slab allocation. That is the whole test of
    /// whether the snapshot is current, made before any run is built: a slab
    /// already handed out is never written in place (see the pyramid's
    /// `finish`), and the only slab that changes is the newest, so a still
    /// frame allocates no run and hands the renderer the pointer it already
    /// uploaded.
    ///
    /// A Clear does not need to be in the test, though the newest slab can be
    /// the shared silent slab before it and after. Columns after a Clear date
    /// after those before it, so the first slab after it is at the newest
    /// key before it or later; a held run spans at least two slabs, so it
    /// starts at least one key before that newest one, and the two first
    /// keys never match.
    fn accept(&mut self, run: pyramid::Run<'_>, layout: TexLayout) -> TexLayout {
        let first_key = run.first_key;
        let len = run.len();
        let last = run.slabs.clone().next_back();
        if let Some(held) = self.run.as_ref() {
            if let RunSource::Live(held_layout) = held.from {
                if held_layout.bucket.to_bits() == layout.bucket.to_bits()
                    && held.first_key == first_key
                    && held.run.len() == len
                    && held.run.last().zip(last).is_some_and(|(a, b)| Arc::ptr_eq(a, b))
                {
                    return held_layout;
                }
            }
        }
        let run: Arc<[SpectrogramSlab]> = run.slabs.cloned().collect();
        self.run = Some(FoldedRun { first_key, run, from: RunSource::Live(layout) });
        layout
    }

    /// The grid a Tape heatmap draws: the run `key` names, read through
    /// `slab` only when the run held is not that one, so a still frame hands
    /// the renderer the run it already uploaded.
    pub(crate) fn tape(
        &mut self,
        key: TapeRun,
        slab: impl Fn(i64) -> SpectrogramSlab,
    ) -> SpectrogramGrid {
        let held = self
            .run
            .as_ref()
            .is_some_and(|held| matches!(held.from, RunSource::Tape(from) if from == key));
        if !held {
            let run = (key.first..=key.last).map(slab).collect();
            self.run = Some(FoldedRun { first_key: key.first, run, from: RunSource::Tape(key) });
        }
        self.grid().expect("a run is held")
    }

    /// The Tape run held and the slabs it holds, for tests.
    #[cfg(test)]
    pub(crate) fn tape_run(&self) -> Option<(TapeRun, Arc<[SpectrogramSlab]>)> {
        let held = self.run.as_ref()?;
        match held.from {
            RunSource::Tape(key) => Some((key, held.run.clone())),
            RunSource::Live(_) => None,
        }
    }

    fn grid(&self) -> Option<SpectrogramGrid> {
        let held = self.run.as_ref()?;
        Some(SpectrogramGrid {
            // A live run always fits the ring. A Tape run can be longer only
            // at the coarsest level, a song past about 2.3 hours.
            capacity: RING_SLOTS.max(held.run.len()) as u32,
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

/// What this frame's heatmap reads: a level of the pyramid, from a far edge.
/// Whether the surface's snapshot already IS it is [`FoldedGrid::accept`]'s
/// call; the pitch, rows, level and palette only change how the same bytes
/// are read, and reach neither.
///
/// Pure, and deliberately so. Every cliff this pipeline has fallen off has been
/// in this arithmetic — a slab against the analyzer's lag, a window against the
/// ring a level keeps — and deciding it apart from the fold means it can be
/// checked without a GPU, a store or a frame.
pub(crate) struct Plan {
    /// The picture's pitch-axis resolution in device pixels, which is what
    /// sets the footprint a fragment resamples the grid over. It bounds
    /// nothing on the GPU — the grid is bucket-space data — so the pane's own
    /// pixels are the whole of it.
    pub(crate) rows: usize,
    /// The pyramid level read, the rung of [`live_slab`]'s ladder.
    pub(crate) level: usize,
    /// The window's far edge, in absolute time.
    from: f64,
}

impl Plan {
    /// The plan for a frame whose window's far edge is at `from`, drawn after
    /// `held`, which is the level the previous frame drew at — `None` on the
    /// first frame of a surface.
    ///
    /// It reaches the slab width through [`held_slab`], which is where the
    /// ladder's hysteresis and the reason for it are. What that costs the
    /// picture is that the width near a rung boundary depends on which side the
    /// Span arrived from: zooming in holds the blockier grid for a further
    /// [`RUNG_HOLD`] of Span. That is the same factor of two the rung itself is,
    /// moved by a twentieth of a Span, against a picture that otherwise stops
    /// dead under a resting finger.
    pub(crate) fn new(view: &PaneView, from: f64, held: Option<usize>) -> Plan {
        // One row per device pixel of the pitch axis, and one slab per pixel of
        // the depth axis, under the cap the slab count takes.
        let rows = pitch_pixels(view.pitch_len, view.ppp);
        let depth_px = (view.depth_len * view.ppp).round();
        let target_cols = depth_px.clamp(2.0, LIVE_SLAB_CAP) as usize;
        let bucket = held_slab(view.window, target_cols, held.map(pyramid::width));
        Plan { rows, level: pyramid::level_of(bucket), from }
    }

    /// The slab width read.
    pub(crate) fn bucket(&self) -> f64 {
        pyramid::width(self.level)
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

/// The run this frame draws, out of the pyramid into the surface's immutable
/// snapshot, and where it sits in time — `None` while the level holds under
/// two slabs of it.
///
/// Every frame reads the pyramid, which folded each column into every level
/// as it arrived, and hands back the run it served last when no slab moved:
/// a still frame keeps the same snapshot.
pub(crate) fn run_for(
    pyramid: &mut SlabPyramid,
    surfaces: &mut SpectrogramSurfaces,
    surface: usize,
    plan: &Plan,
) -> Option<TexLayout> {
    // The next frame retains this level until the window crosses a rung.
    surfaces.at(surface).held_level = Some(plan.level);
    let bucket = plan.bucket();
    // Slabs stay in BUCKET space: the rows read them in the fragment shader,
    // so a zoom or pan of the pitch axis re-reads this run rather than
    // re-reading the pyramid.
    let run = pyramid.run(plan.level, plan.from)?;
    let w = run.len();
    // The run covers absolute time `[t_origin, t_origin + w*bucket]` — the
    // oldest slab's start to the newest slab's end. Slab centres sit half a
    // slab inside each, so a time places itself at `(t - t_origin) / bucket`
    // slabs.
    let t_origin = (run.first_key as f64 + 0.5) * bucket - 0.5 * bucket;
    let tex_span = w as f64 * bucket;
    let layout = TexLayout { bucket, t_origin, tex_span };
    Some(surfaces.at(surface).folded.accept(run, layout))
}

/// The shade table `cfg` draws a heatmap through, shared with the surface's
/// live heatmap.
pub(crate) fn shades(
    surfaces: &mut SpectrogramSurfaces,
    surface: usize,
    cfg: &SpectrumConfig,
) -> SpectrogramShades {
    surfaces.at(surface).folded.shades(cfg)
}

/// The grid and the gradient table this frame's callback carries.
///
/// `None` before anything has been folded into the surface, which is the same
/// frame [`run_for`] answers `None` on.
pub(crate) fn frame_data(
    surfaces: &mut SpectrogramSurfaces,
    surface: usize,
    cfg: &SpectrumConfig,
) -> Option<(SpectrogramGrid, SpectrogramShades)> {
    let gpu = &mut surfaces.at(surface).folded;
    let grid = gpu.grid()?;
    Some((grid, gpu.shades(cfg)))
}

/// A slab's mean linear power while its columns arrive, summed in f64 so a
/// long slab accumulates no rounding error, and quantized once per read.
#[derive(Clone)]
pub(crate) struct PowerMean {
    sum: Box<[f64; SPECTRUM_BINS]>,
    pub(crate) count: u64,
}

impl Default for PowerMean {
    fn default() -> Self {
        Self { sum: Box::new([0.0; SPECTRUM_BINS]), count: 0 }
    }
}

impl PowerMean {
    /// Add one column's power.
    pub(crate) fn add(&mut self, power: &[f32]) {
        for (sum, &fresh) in self.sum.iter_mut().zip(power) {
            *sum += f64::from(fresh);
        }
        self.count += 1;
    }

    pub(crate) fn write(&self, out: &mut [BucketDb]) {
        for (out, &sum) in out.iter_mut().zip(self.sum.iter()) {
            *out = harmonigraph_core::spectrogram::quantize(if self.count == 0 {
                0.0
            } else {
                (sum / self.count as f64) as f32
            });
        }
    }

    pub(crate) fn clear(&mut self) {
        self.sum.fill(0.0);
        self.count = 0;
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

mod pyramid;
pub(crate) use pyramid::SlabPyramid;
#[cfg(test)]
pub(crate) use pyramid::{width, LEVELS};

#[cfg(test)]
mod oracle;

#[cfg(test)]
mod tests;
