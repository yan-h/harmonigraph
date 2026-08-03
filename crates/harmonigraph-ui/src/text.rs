//! Haloed label text: collected as glyphs here, drawn by
//! [`harmonigraph_render::text`].
//!
//! Text on either picture pane needs to be lifted off what it lands on —
//! note names over lit nodes, pitch labels over the spectrogram. Stamping
//! that rim as geometry, the whole label repeated around two rings, is
//! twenty more copies of every glyph: most of the geometry in a busy frame,
//! and it makes labels a budget where every new one costs twenty-one draws
//! of its own text.
//!
//! So a piece of text becomes one quad per glyph and the rim is computed
//! per pixel from the same offsets (see `harmonigraph_render::text` for why the
//! two are the same arithmetic). What a label costs does not depend on its
//! rim, which is what makes labels something to place where they help
//! rather than something to ration.
//!
//! egui still lays the text out. A [`TextBatch`] collects the glyphs of
//! however many pieces of text a pane draws, and hands them over in one
//! callback when the pane flushes it — flushing where the pane would
//! otherwise draw something on top, so the paint order the panes had is the
//! paint order they keep.
//!
//! The lattice's node names take the other exit. They are collected here the
//! same way, but handed to the LATTICE's callback
//! ([`TextBatch::lattice_labels`]) rather than flushed as a pass of their
//! own, because a name belongs at its node's place in the scene's own
//! back-to-front order: a node in front covers the name of the node behind
//! it, which nothing drawn over the finished picture can reproduce.

use harmonigraph_render::{GlyphInstance, TextRing};

/// The halo's two rings, as (radius in points, stamp alpha, samples).
///
/// The sample counts are a cost, not a look: each is one more evaluation of
/// the glyph's coverage per pixel. They were 16 and 16 —
///
///   - the crisp ring is opaque, so its samples only have to close the gap:
///     at a 1.2pt radius, 12 land half a point apart and the union reads as
///     one line (8 starts to scallop, 4 visibly thins on the diagonals);
///   - the soft ring is a fade, and a fade is made of overlap. Halving its
///     samples to 8 thins it, so its stamp alpha rises to compensate: 0.21
///     against 0.15, tuned by rendering the pair and matching pixels rather
///     than by the compositing arithmetic, which assumes an overlap count
///     that in fact varies across the rim.
///
/// Radii are snapped to whole physical pixels before use: mixed
/// cardinal/diagonal offsets and sub-pixel radii both read as a lumpy
/// outline on a high-DPI display.
pub(crate) const RINGS: [(f32, f32, u32); 2] = [(2.0, 0.21, 8), (1.2, 1.0, 12)];

/// One ring's radius on this display, rounded to a whole physical pixel.
///
/// A SIZE, and the one thing here that is still rounded now that positions
/// are not: a sub-pixel or mixed-fraction radius reads as a lumpy outline,
/// and unlike a position it is a constant of the frame, so rounding it
/// cannot make anything step as it moves.
///
/// Shared because the drawn label marks build the same two rings out of
/// geometry (`panes::lattice::paint_mark`), and a rim that is 1.2pt around
/// a glyph and 1.0pt around the `+` beside it is exactly the mismatch this
/// pairing exists to avoid.
pub(crate) fn ring_radius(radius: f32, ppp: f32) -> f32 {
    (radius * ppp).round().max(1.0) / ppp
}

/// A text scale snapped so that text of `base` points lands on a whole
/// PHYSICAL pixel.
///
/// Every distinct size a pane asks for is one more entry in egui's font atlas:
/// epaint rasterizes and caches a glyph per exact size, and rebuilds the whole
/// font store once the atlas passes 80% full — throwing away the UVs this
/// module's mirror is holding. That cost nothing while label sizes were
/// constants. It is not nothing now that they follow a zoom: unsnapped, the
/// scale takes a new value on every frame of a drag, so one gesture asks for
/// dozens of sizes, each rasterizing its own copy of every glyph on screen and
/// re-uploading the mirror behind it (see [`AtlasMirror`]).
///
/// Two grains bound that set, and the coarser of them is the one that bites:
///
///   - a LADDER of fixed relative steps, anchored at scale 1. What the eye
///     judges of a size change is its proportion, so this is the grain a step
///     is invisible at — and, being relative, it makes the number of sizes a
///     zoom can ask for a function of how many times it doubles rather than of
///     how many pixels it crosses. A pixel grid alone is far too fine once a
///     name is 60 pixels tall and rising: measured over one 120-frame zoom
///     drag it let through 299 distinct sizes, and every one of them is a set
///     of glyphs rasterized and an atlas re-uploaded behind it.
///   - the PIXEL, which the ladder's rungs are rounded onto. Below a few tens
///     of pixels the rungs fall closer together than that, and two sizes
///     inside one pixel are the same picture of the same letter twice. It is
///     also the grain the DRAWN marks' bitmaps are built on
///     (`panes::lattice::mark_key`), which is what makes a name and the `+`
///     beside it step together as the camera moves rather than one at a time.
///
/// A `base` that is not itself a whole number of pixels moves by up to half of
/// one — the roll's 12.35pt name is 24.7 pixels at 2x and draws at 25 — which
/// is the grid asserting itself, not a size being got wrong.
///
/// Bounded at both ends, in PIXELS, since that is where the consequences are.
/// A pixel is the floor because a glyph smaller than one is nothing at all.
/// [`MAX_GLYPH_PX`] is the ceiling: every factor feeding this is bounded, but
/// they multiply, and their product times a 30pt name is not — and a size past
/// the atlas's own width is not merely large. epaint takes the overflow path
/// there, recycling texels that live glyphs still point at, so the failure is
/// not a big label but every label on the pane going to garbage.
///
/// `base` is the size the scale is quoted against — the note name's, since it
/// is the biggest thing in a label and the one whose stepping would show. The
/// rest of a label is sized off the same scale, so it lands where the
/// proportions put it rather than on a pixel of its own.
pub(crate) fn snap_scale(scale: f32, base: f32, ppp: f32) -> f32 {
    // A scale that is not a number cannot be drawn at, and passing it on is
    // the quietest of the failures available: egui rasterizes such a size to
    // an empty glyph, so every label vanishes and nothing says why. The size
    // the base was chosen at is the one value certain to be legible.
    if !scale.is_finite() {
        return 1.0;
    }
    // Physical pixels per unit of scale. A nonsense one (a zero base, a
    // hand-edited ppp) leaves the scale alone rather than dividing by it.
    let per_scale = base * ppp;
    if !per_scale.is_finite() || per_scale <= 0.0 || scale <= 0.0 {
        return scale;
    }
    // The rung of the ladder this scale sits nearest, then the pixel that rung
    // rounds onto. Anchored at scale 1, so the size a label was dialled at is
    // reproduced exactly rather than to within a step.
    let rung = SIZE_STEP.powf((scale.ln() / SIZE_STEP.ln()).round());
    (rung * per_scale).round().clamp(1.0, MAX_GLYPH_PX) / per_scale
}

/// The scale to RASTERIZE at, and how much bigger than that to DRAW — the two
/// halves of following a zoom without asking egui for a size per frame.
///
/// The pair belongs together, and this exists so that no caller holds one
/// without the other. A caller that snapped and forgot to magnify would step
/// exactly as it did before any of this; one that magnified without snapping
/// would draw the right size off an atlas entry per frame; and the ceiling
/// below only holds if whoever divides also clamps.
///
/// That ceiling is the whole reason this is not two lines at each call site.
/// [`snap_scale`] clamps what is RASTERIZED to [`MAX_GLYPH_PX`], and a
/// magnification computed against the raw request absorbs everything past it —
/// so a camera zoomed fully in with the Size bar at 3 would ask for type twice
/// the size of the cell behind it, which is not a big label but a blurred one,
/// and a hand-edited camera makes it far worse. Clamping the request the same
/// way keeps the ceiling meaning what `harmonigraph_scene::Camera` says it
/// means: what a label may finally be sized at is bounded here.
pub(crate) fn ladder(want: f32, base: f32, ppp: f32) -> (f32, f32) {
    let raster = snap_scale(want, base, ppp);
    let per_scale = base * ppp;
    // The guard paths of `snap_scale`, which return something usable without
    // choosing a rung. There is no residual to take against those.
    if !(want.is_finite() && want > 0.0 && per_scale.is_finite() && per_scale > 0.0) {
        return (raster, 1.0);
    }
    if !(raster.is_finite() && raster > 0.0) {
        return (raster, 1.0);
    }
    // Bounded in PIXELS, exactly as the rasterized size is and for the same
    // reasons — a floor because type under a pixel is nothing, a ceiling
    // because past it the size is no longer one anybody asked for.
    let want = want.clamp(1.0 / per_scale, MAX_GLYPH_PX / per_scale);
    (raster, want / raster)
}

/// One rung of the size ladder, as a ratio. 4% — under what reads as a change
/// of size while a picture is moving, and coarse enough that a sixfold zoom
/// asks for some 45 sizes where a pixel grid asked for 300.
const SIZE_STEP: f32 = 1.04;

/// The largest a label's type is ever rasterized, in physical pixels.
///
/// Far past anything readable — 512 pixels is a quarter of a tall pane on a
/// Retina display — so it bounds the accidents (a hand-edited blob, a camera
/// and a bar and a pane all at their limits at once) without reaching any
/// size a person would ask for. Chosen under the smallest atlas any shell
/// here builds: the offline renderer's egui context takes egui's 2048 default
/// rather than the 8192 the plugin gets from wgpu, and a video is exactly
/// where a corrupted glyph is least recoverable.
const MAX_GLYPH_PX: f32 = 512.0;

/// Which batch a flush belongs to. Unique per FLUSH drawn in one frame, since
/// each keeps its own instance buffer — the analyzer and its Render-preview
/// copy, plus every pane that flushes more than once to put something between
/// two groups of text. A pane is not the unit here, which is what makes adding
/// one a renumbering rather than an append; the ids are checked for collisions
/// in this module's tests.
///
/// The lattice's node names are not among them: they are drawn inside the
/// lattice's own pass, so that a node in front covers the name of the node
/// behind it, and they reach it through
/// [`lattice_labels`](TextBatch::lattice_labels) rather than through a flush.
/// The learn badge still flushes, being chrome ABOUT the pane rather than
/// something in the picture.
pub(crate) const LATTICE_LEARN: u64 = 0;
/// The analyzer's, one per surface (docked, then the preview).
pub(crate) fn spectral_labels(surface: usize) -> u64 {
    1 + surface as u64
}

/// One glyph as the mirror identifies it: its size, its character, and the
/// TEXEL it was found at.
///
/// The texel is what makes this exact rather than nearly right. What the
/// mirror has to guarantee is that every glyph a batch points at exists, at
/// those coordinates, in the copy of the atlas the GPU holds — so the thing to
/// notice is a glyph turning up somewhere we have not uploaded, whatever the
/// reason. Keyed on (size, character) alone it misses the commonest reason of
/// all: epaint bins a glyph's subpixel position into four cells and caches
/// each separately (`SubpixelBin`, and its place in `GlyphCacheKey`), so one
/// character at one size is up to FOUR images at four texels, chosen by what
/// precedes it in the string. A digit shifting column between `-13.69` and
/// `-13.71` lands in a cell the atlas has never held, the pair says "seen",
/// and the glyph samples blank space until something else forces a refresh —
/// which is a digit dropping out of the lattice's cents readout and coming
/// back a frame later.
type GlyphKey = (u32, char, [u16; 2]);

/// A uniform magnification applied to every quad a batch emits: a factor, and
/// the point it is taken about.
///
/// This is what lets type be RASTERIZED on the ladder and DRAWN at any size in
/// between. A glyph's screen rect and its atlas rect reach the shader as
/// independent inputs, so a quad drawn larger than the cell behind it simply
/// resamples that cell — the sampler is already `Linear`, because a glyph
/// already lands at a fractional position (see [`TextBatch::text`]).
///
/// Which is what keeps type from stepping as a camera zooms. The ladder exists
/// so that a continuous zoom cannot ask egui for a continuous set of font sizes
/// (see [`snap_scale`]); a size quantized WITH its raster steps from rung to
/// rung while the nodes under it scale continuously, and the nodes are shader
/// geometry at continuous positions. Splitting the two settles it: the atlas
/// sees one size per rung, and the picture gets a size that moves as smoothly
/// as the geometry it is written over.
///
/// How far off its raster a glyph can be drawn is bounded by the COARSER of the
/// ladder's two grains, not by the rung alone: half a rung is about 2%, but the
/// rungs are also rounded onto whole physical pixels, and down where a pixel is
/// worth more than a rung that rounding is what the residual answers to. The
/// analyzer's names run to about 3% at their dialled size for exactly that
/// reason. [`ladder`] bounds it in the one direction that is not a grain at
/// all — the ceiling, where the raster stops rising and a magnification
/// computed against the raw request would not.
///
/// All of which sits well inside what the same bilinear tap already does for a
/// glyph's fractional position, which is not bounded by anything.
#[derive(Clone, Copy)]
struct Magnify {
    origin: egui::Pos2,
    factor: f32,
}

impl Magnify {
    /// Where `p` lands, and how much longer a length at it becomes.
    fn point(self, p: egui::Pos2) -> egui::Pos2 {
        self.origin + (p - self.origin) * self.factor
    }
}

/// The glyphs a pane has drawn so far, waiting to be handed to the GPU.
#[derive(Default)]
pub(crate) struct TextBatch {
    glyphs: Vec<GlyphInstance>,
    /// Which node each of those glyphs names, for a batch whose text is going
    /// to the LATTICE's own pass rather than over the finished picture. Filled
    /// by [`TextBatch::attached_to`] and empty for every other pane, which has
    /// nothing for a glyph to belong to.
    labels: Vec<harmonigraph_render::Label>,
    /// Every glyph this batch drew — the probe that decides whether egui has
    /// rasterized something our mirror of its atlas has not seen. See
    /// [`GlyphKey`] and [`AtlasMirror`].
    drawn: Vec<GlyphKey>,
    /// In force for the piece being drawn, if any. See [`Magnify`].
    magnify: Option<Magnify>,
    /// Test-only: which glyphs came from which piece of text. The glyphs
    /// alone carry no text — they are rects and atlas coordinates — so without
    /// this a test could see WHERE a label was drawn but not WHAT, and the
    /// typography tests are all about which piece sits where.
    #[cfg(test)]
    pieces: Vec<TextPiece>,
}

/// One `text()` call, for tests: what it said, at what size, and the ink it
/// covers on screen.
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct TextPiece {
    pub text: String,
    pub font_size: f32,
    /// Tight to the glyphs — what the eye reads, and what the label's own
    /// stacking is measured against.
    pub ink: egui::Rect,
    /// The line box egui laid the text out in, which carries the font's
    /// leading above and below the ink.
    pub galley: egui::Rect,
}

impl TextBatch {
    /// Draw everything `f` emits magnified by `factor` about `origin` — the
    /// label's own anchor, so the whole piece grows about the thing it names
    /// rather than sliding off it.
    ///
    /// Scoped rather than a pair of set/clear calls, since a magnification left
    /// switched on is every later label on the pane drawn at the wrong size
    /// about the wrong point, and nothing would say so.
    ///
    /// `f` lays out at the RASTERIZED size throughout — it measures galleys,
    /// stacks rows and places marks exactly as it would have — and this scales
    /// the finished result. That is what keeps the split from reaching the
    /// layout: a caller cannot measure at one size and draw at another, because
    /// it never learns that the two differ.
    pub(crate) fn magnified<R>(
        &mut self,
        origin: egui::Pos2,
        factor: f32,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        // A nonsense factor draws nothing anyone can read, and the size the
        // ladder already chose is right to within a rung.
        let factor = if factor.is_finite() && factor > 0.0 { factor } else { 1.0 };
        let previous = self.magnify.replace(Magnify { origin, factor });
        let out = f(self);
        self.magnify = previous;
        out
    }

    /// Everything `f` draws is the name of node `node` — an index into the
    /// scene's own `nodes`.
    ///
    /// Only the lattice has anything to say here, and what it buys is where
    /// the name is DRAWN: the lattice's callback puts a label at its node's
    /// place in the back-to-front order, so a nearer node covers the name of
    /// the node behind it exactly as it covers the node itself. Everything
    /// else this batch collects is drawn over a finished picture, where there
    /// is no order to belong to.
    ///
    /// Scoped, so a label is one uninterrupted run of glyphs: the runs are
    /// what say which glyphs are whose, and they carry lengths rather than
    /// indices.
    pub(crate) fn attached_to<R>(&mut self, node: u32, f: impl FnOnce(&mut Self) -> R) -> R {
        let first = self.glyphs.len();
        let out = f(self);
        let glyphs = (self.glyphs.len() - first) as u32;
        if glyphs > 0 {
            self.labels.push(harmonigraph_render::Label { node, glyphs });
        }
        out
    }

    /// Add one piece of text, haloed. `outline` should be the skin's
    /// recessed surface (`theme::well`), which contrasts with any text color
    /// by construction; a transparent one draws the glyphs bare.
    ///
    /// egui does the work that decides what the pixels are — shaping,
    /// rasterizing, and placing every glyph — and this reads the placement
    /// back out of the galley. The rect and rounding below are exactly what
    /// `epaint::Tessellator` would have used for the same galley, which is
    /// what keeps a label on the same pixels it has always been on.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn text(
        &mut self,
        painter: &egui::Painter,
        anchor: egui::Pos2,
        align: egui::Align2,
        text: String,
        font: egui::FontId,
        color: egui::Color32,
        outline: egui::Color32,
    ) {
        #[cfg(test)]
        let (said, font_size) = (text.clone(), font.size);
        #[cfg(test)]
        let first = self.glyphs.len();
        let font_size_bits = font.size.to_bits();
        let galley = painter.layout_no_wrap(text, font, egui::Color32::PLACEHOLDER);
        // Placed at whatever position it is handed, NOT rounded onto a whole
        // physical pixel — for EVERY pane that draws through this batch, not
        // only the ones whose text moves.
        //
        // egui rounds a galley before drawing it (`round_text_to_pixels`),
        // and rounding with it does keep each glyph on the pixel grid it was
        // rasterized for. The cost is that text becomes the only thing on a
        // picture pane that moves in whole pixels: the lattice's nodes and
        // the roll's ribbons are shader geometry at continuous positions, so
        // a label steps while the thing it names glides, and the mismatch
        // reads as judder against its own subject.
        //
        // Global rather than per-pane, deliberately. Nearly every label this
        // batch carries sits on something that moves — lattice names ride a
        // camera, roll names ride a scrolling picture, and the render pane
        // draws the lattice again. The few that hold still, chiefly the
        // spectral pane's axis labels, pay a constant softening so that there
        // is ONE text path rather than two: a mode here would mean the same
        // label rendered differently depending on which pane drew it, and
        // this module's worth is that it has no modes.
        //
        // The softening is real — a glyph at a fractional offset resamples
        // its atlas cell. Constant softness reads as a typeface;
        // intermittent stepping reads as a bug.
        let pos = align.anchor_size(anchor, galley.size()).min;

        // The size the quad is drawn at, against the size the atlas holds. One
        // everywhere the caller wants the rasterized size; see [`Magnify`] for
        // why a label that follows a zoom does not.
        let k = self.magnify.map_or(1.0, |m| m.factor);
        for row in &galley.rows {
            for glyph in &row.glyphs {
                if glyph.uv_rect.is_nothing() {
                    continue;
                }
                let left_top = glyph.pos + glyph.uv_rect.offset;
                let min = pos + row.pos.to_vec2() + left_top.to_vec2();
                let min = self.magnify.map_or(min, |m| m.point(min));
                self.drawn.push((font_size_bits, glyph.chr, glyph.uv_rect.min));
                self.glyphs.push(GlyphInstance {
                    rect: [min.x, min.y, glyph.uv_rect.size.x * k, glyph.uv_rect.size.y * k],
                    uv: [
                        f32::from(glyph.uv_rect.min[0]),
                        f32::from(glyph.uv_rect.min[1]),
                        f32::from(glyph.uv_rect.max[0]),
                        f32::from(glyph.uv_rect.max[1]),
                    ],
                    fill: color.to_array(),
                    rim: outline.to_array(),
                });
            }
        }
        #[cfg(test)]
        {
            let ink = self.glyphs[first..]
                .iter()
                .map(|g| {
                    egui::Rect::from_min_size(
                        egui::pos2(g.rect[0], g.rect[1]),
                        egui::vec2(g.rect[2], g.rect[3]),
                    )
                })
                .reduce(|a, b| a.union(b));
            if let Some(ink) = ink {
                // Magnified alongside the ink, or a test comparing the two
                // would be comparing two different spaces.
                let min = self.magnify.map_or(pos, |m| m.point(pos));
                let galley = egui::Rect::from_min_size(min, galley.size() * k);
                self.pieces.push(TextPiece { text: said, font_size, ink, galley });
            }
        }
    }

    /// Every piece of text collected so far (tests only).
    #[cfg(test)]
    pub(crate) fn pieces(&self) -> &[TextPiece] {
        &self.pieces
    }

    /// How many glyphs are waiting — what a batch actually costs.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.glyphs.len()
    }

    /// Hand everything collected so far to the painter, and start again.
    ///
    /// `pane_id` must be unique per batch drawn in one frame — a pane that
    /// flushes twice (to put something between two groups of labels) needs
    /// an id for each, since each keeps its own buffer.
    pub(crate) fn flush(
        &mut self,
        painter: &egui::Painter,
        rect: egui::Rect,
        state: &crate::SharedState,
        pane_id: u64,
    ) {
        if self.glyphs.is_empty() {
            return;
        }
        #[cfg(test)]
        self.pieces.clear();
        let atlas = atlas_if_changed(
            painter.ctx(),
            &state.instruments.font_atlas,
            std::mem::take(&mut self.drawn),
        );
        painter.add(harmonigraph_render::text_paint_callback(
            rect,
            std::mem::take(&mut self.glyphs),
            rings(painter.ctx()),
            atlas,
            state.target_format,
            pane_id,
        ));
    }

    /// Hand the node names collected so far to the LATTICE's own callback,
    /// and start again.
    ///
    /// `origin` is the pane's top-left corner: the glyphs were placed in
    /// screen points, as everything laid out against an `egui::Painter` is,
    /// and the pass that draws them is the pane's own — where the pane's
    /// corner is the origin and the render scale decides the pixels.
    ///
    /// The atlas comes from a mirror of its own rather than the one
    /// [`flush`](Self::flush) draws from. Each mirror answers for one
    /// TEXTURE — "is every glyph this batch points at in the copy that
    /// renderer holds" — and the lattice has its own, so a single mirror
    /// would hand each renderer half the publications and leave both holding
    /// half an atlas.
    pub(crate) fn lattice_labels(
        &mut self,
        painter: &egui::Painter,
        origin: egui::Pos2,
        state: &crate::SharedState,
    ) -> harmonigraph_render::LatticeLabels {
        #[cfg(test)]
        self.pieces.clear();
        let atlas = atlas_if_changed(
            painter.ctx(),
            &state.instruments.lattice_atlas,
            std::mem::take(&mut self.drawn),
        );
        let mut glyphs = std::mem::take(&mut self.glyphs);
        for glyph in &mut glyphs {
            glyph.rect[0] -= origin.x;
            glyph.rect[1] -= origin.y;
        }
        harmonigraph_render::LatticeLabels {
            glyphs,
            labels: std::mem::take(&mut self.labels),
            rings: rings(painter.ctx()),
            atlas,
        }
    }
}

/// The halo's rings at this display's scale, as the renderer takes them.
fn rings(ctx: &egui::Context) -> [TextRing; 2] {
    let ppp = ctx.pixels_per_point();
    RINGS.map(|(radius, alpha, samples)| TextRing {
        radius: ring_radius(radius, ppp),
        alpha,
        samples,
    })
}

/// What one renderer's copy of egui's font atlas currently holds.
///
/// One of these per copy, and there are two: the text callback's, and the
/// lattice's, which draws its node names inside its own scene pass off a
/// texture of its own. Each answers for the texture it belongs to, so they
/// are separate mirrors rather than one shared — a single mirror publishes an
/// atlas once, and whichever renderer asked second would be told nothing had
/// changed while holding none of it.
///
/// The callback cannot bind egui's own font texture — `CallbackResources`
/// holds what WE put there — so the atlas is mirrored, and something has to
/// say when the mirror is stale. egui's delta channel is not ours to take
/// (draining it would starve egui's own renderer and the rest of the UI's
/// text would stop updating), and the atlas exposes no version, so staleness
/// is inferred from the one thing that is knowable: a glyph found at a texel
/// we have never uploaded cannot be in the copy the GPU holds, so the first
/// time a [`GlyphKey`] is drawn, the mirror is refreshed.
///
/// The three fields below are guards on top of that, each for a way the whole
/// atlas can move at once — cheaper to notice wholesale than one glyph at a
/// time, and `size` and `ppp` also cover the case where a rearrangement lands
/// a DIFFERENT glyph on a texel some key already claims.
///
/// How often this copies is a function of how many distinct sizes the panes
/// ask for, which is no longer a handful: a label's size follows the camera
/// and the pitch zoom, so a zoom gesture walks through sizes and each is a
/// fresh set of glyphs. See [`snap_scale`], which bounds that set, and the
/// note on what a refresh costs in `harmonigraph_render::text`.
#[derive(Default)]
pub(crate) struct AtlasMirror {
    seen: std::collections::HashSet<GlyphKey>,
    /// The atlas size the mirror was taken at; a resize moves every glyph.
    size: [usize; 2],
    /// The scale factor it was taken at, as bits — the third way every glyph
    /// moves, and the one the others can miss.
    ///
    /// A key carries a POINT size, but egui rasterizes at PHYSICAL pixels, so
    /// the same size is a different image at a different scale. Changing it
    /// appends every glyph afresh — epaint's own cache is keyed on the scale
    /// too — while the atlas dimensions stay put: measured at 1x and 2x, the
    /// atlas is `[2048, 32]` both times and 'A' sits at texels `[90, 0]`
    /// against `[111, 0]`. The keys move with the texels, so `seen` does now
    /// catch this on its own; this stays because it is the cheap wholesale
    /// version of the same fact, and because a glyph landing where an old key
    /// already points is the one arrangement a per-glyph probe cannot see.
    /// Dragging the window between a Retina display and an external monitor is
    /// the ordinary way to do it.
    ppp: u32,
    /// How full egui said its atlas was, last time we looked — the fourth way
    /// every glyph moves, and the only signal that egui has thrown the atlas
    /// away and started over.
    ///
    /// `Fonts::begin_pass` rebuilds the entire font store once the atlas passes
    /// 80% full, re-rasterizing every glyph at fresh UVs. Zoom-scaled labels are
    /// what put that within reach: a label's size follows the camera and the
    /// pitch range, so a session walks through many more sizes than the handful
    /// of constants the panes used to draw at, and each one fills more of the
    /// atlas. The other triggers can both miss it — the rebuilt atlas regrows
    /// through whatever size the mirror recorded, and the panes report the same
    /// pairs they always did — so the DROP in fill ratio is what says it
    /// happened. (It only ever drops there; new glyphs raise it.) Everything in
    /// `seen` is stale with it, so it goes too.
    fill: f32,
    /// Bumped on every refresh; the callback compares it against what it
    /// last uploaded.
    key: u64,
}

impl AtlasMirror {
    /// Forget the context this describes, so the next flush publishes.
    ///
    /// All four guards above read one egui `Context`, and a shell that builds
    /// a second one leaves them answering for an atlas nobody is drawing from.
    /// See [`release_context_resources`](crate::SharedState::release_context_resources),
    /// which is the one caller.
    ///
    /// `key` is deliberately kept: it counts publications rather than
    /// describing an atlas, and a renderer that survived the context still
    /// compares against the last one it uploaded, so it has to keep rising.
    pub(crate) fn forget_context(&mut self) {
        self.seen.clear();
        self.size = [0, 0];
        self.ppp = 0;
        self.fill = 0.0;
    }
}

/// egui's font atlas, on the frames the mirror needs it, and `None` on the
/// rest — which is nearly all of them.
fn atlas_if_changed(
    ctx: &egui::Context,
    mirror: &std::sync::Mutex<AtlasMirror>,
    drawn: Vec<GlyphKey>,
) -> Option<harmonigraph_render::FontAtlas> {
    let mut mirror = mirror.lock().expect("the label mirror is never held across a panic");
    // A resize (or the overflow that clears the atlas and starts over)
    // rearranges everything, so it counts as a change on its own.
    let size = ctx.fonts(|fonts| fonts.font_image_size());
    // So does a scale change, which repacks the atlas without resizing it — see
    // [`AtlasMirror::ppp`]. The pairs already seen were rasterized at the old
    // scale and say nothing about the new one, so they go with it: every glyph
    // the panes draw counts as unseen again, which is also what refills `seen`.
    let ppp = ctx.pixels_per_point().to_bits();
    // ...and so does the rebuild egui does when its atlas fills up, which
    // repacks every glyph without either of the two above having to change.
    // See [`AtlasMirror::fill`].
    let fill = ctx.fonts(|fonts| fonts.font_atlas_fill_ratio());
    let rebuilt = fill < mirror.fill;
    mirror.fill = fill;
    if ppp != mirror.ppp || rebuilt {
        mirror.seen.clear();
    }
    let resized = mirror.seen.is_empty() || size != mirror.size;
    let fresh = drawn.into_iter().fold(false, |fresh, pair| mirror.seen.insert(pair) || fresh);
    if !fresh && !resized {
        return None;
    }
    mirror.size = size;
    mirror.ppp = ppp;
    mirror.key = mirror.key.wrapping_add(1);
    Some(harmonigraph_render::FontAtlas {
        image: std::sync::Arc::new(ctx.fonts(|fonts| fonts.image())),
        key: mirror.key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lay `text` out at `ppp` and report exactly what a pane drawing it would
    /// hand [`atlas_if_changed`]: the keys of its glyphs, with the atlas
    /// dimensions alongside.
    fn draw_at(
        ctx: &egui::Context,
        font: &egui::FontId,
        text: &str,
        ppp: f32,
    ) -> ([usize; 2], Vec<GlyphKey>) {
        ctx.set_pixels_per_point(ppp);
        let mut drawn = None;
        // Twice: the first pass asks for glyphs the atlas may not hold yet, and
        // the rasterization it triggers lands for the next one.
        for _ in 0..2 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let galley =
                    ui.painter().layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE);
                drawn = Some(
                    galley.rows[0]
                        .glyphs
                        .iter()
                        .map(|g| (font.size.to_bits(), g.chr, g.uv_rect.min))
                        .collect::<Vec<_>>(),
                );
            });
        }
        (ctx.fonts(|fonts| fonts.font_image_size()), drawn.expect("the closure runs"))
    }

    /// Collect the keys a batch reports for `text`, exactly as a pane would.
    fn batch_keys(ctx: &egui::Context, font: &egui::FontId, text: &str) -> Vec<GlyphKey> {
        let mut batch = TextBatch::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            batch.text(
                ui.painter(),
                egui::pos2(20.0, 20.0),
                egui::Align2::LEFT_TOP,
                text.to_owned(),
                font.clone(),
                egui::Color32::WHITE,
                egui::Color32::BLACK,
            );
        });
        batch.drawn
    }

    /// Every batch drawn in one frame keeps its own instance buffer, keyed on
    /// its id, so two batches sharing one id draw each other's glyphs — the
    /// second flush overwrites the first's buffer and the first pane's text
    /// lands wherever the second's was. The ids are hand-numbered and the
    /// analyzer's run from a base, which is what makes adding one a renumber
    /// rather than an append.
    #[test]
    fn every_batch_drawn_in_a_frame_has_an_id_of_its_own() {
        let ids = [
            LATTICE_LEARN,
            // The docked analyzer, then the Video pane's preview copy.
            spectral_labels(0),
            spectral_labels(1),
        ];
        let distinct: std::collections::HashSet<u64> = ids.iter().copied().collect();
        assert_eq!(distinct.len(), ids.len(), "two batches share an id: {ids:?}");
    }

    /// A character the atlas has never rasterized THERE is a glyph the mirror
    /// has never uploaded, whatever its (size, character) says.
    ///
    /// epaint bins a glyph's subpixel position into four cells and caches each
    /// separately, so one character at one size is up to four images at four
    /// texels — chosen by what precedes it in the string. This is the ordinary
    /// case, not a corner: the lattice's cents readout reformats as a note
    /// bends, a digit shifts column, and it lands in a cell nothing has
    /// uploaded. Keyed on the pair alone the mirror reports "seen" and that
    /// digit samples blank space — it drops out of the readout and comes back
    /// a frame later, when some genuinely new pair happens to force a refresh.
    ///
    /// Both strings are laid out BEFORE the mirror is made, so every glyph is
    /// already in the atlas and its dimensions have settled. Without that the
    /// second draw grows the atlas and the size guard refreshes for it — which
    /// is what happens in a fresh editor for a few seconds, and is exactly why
    /// this went unnoticed: the bug needs an atlas that has stopped growing,
    /// which is every session after the first moments.
    #[test]
    fn a_glyph_at_a_new_texel_refreshes_the_mirror() {
        let ctx = egui::Context::default();
        let font = egui::FontId::monospace(13.0);
        for _ in 0..2 {
            batch_keys(&ctx, &font, "Ai");
            batch_keys(&ctx, &font, "iA");
        }
        let settled = ctx.fonts(|fonts| fonts.font_image_size());

        let state = crate::SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        let first = batch_keys(&ctx, &font, "Ai");
        let mirror = &state.instruments.font_atlas;
        assert!(atlas_if_changed(&ctx, mirror, first.clone()).is_some(), "the first mirror");
        assert!(atlas_if_changed(&ctx, mirror, first.clone()).is_none(), "nothing has moved");

        // The same character at the same size, pushed along by what is now in
        // front of it. Every PAIR here is one the mirror has already seen...
        let swapped = batch_keys(&ctx, &font, "iA");
        let (a, moved) = (first[0], swapped[1]);
        assert_eq!((a.0, a.1), (moved.0, moved.1), "the same size and character");
        assert_ne!(a.2, moved.2, "...rasterized at a texel of its own");
        assert!(
            swapped.iter().all(|k| first.iter().any(|seen| (seen.0, seen.1) == (k.0, k.1))),
            "every pair here is one the mirror has already been shown",
        );
        assert_eq!(ctx.fonts(|fonts| fonts.font_image_size()), settled, "the atlas has not grown");

        // ...so the texel is the whole of what stands between that glyph and a
        // blank rectangle where a digit should be.
        assert!(
            atlas_if_changed(&ctx, mirror, swapped).is_some(),
            "a glyph at an unuploaded texel must refresh the mirror",
        );
    }

    /// Every field the mirror compares describes ONE egui context, so the
    /// mirror belongs to that context and dies with it.
    ///
    /// [`SharedState`](crate::SharedState) outlives the context — the plugin's
    /// editor builds a fresh one per window — while the atlas texture and the
    /// per-pane bind groups the mirror is a mirror OF live in the renderer,
    /// which the new window builds fresh alongside it. A mirror carried across
    /// therefore answers for texels in a texture nobody allocated: it reports
    /// "already uploaded", the callback finds no atlas and paints nothing, and
    /// the pane's labels are simply absent — the lattice's note names, the
    /// analyzer's, the learn badge — with no frame that recovers them, because
    /// the mirror is itself the only thing that would ask.
    ///
    /// [`release_context_resources`](crate::SharedState::release_context_resources)
    /// is where a shell says the context is gone, and clearing the mirror
    /// there is what bounds this to the window that opened it.
    ///
    /// The two contexts here draw the same text at the same scale, which is
    /// what a reopened window does before anything moves: same atlas
    /// dimensions, same fill ratio, same glyphs at the same texels. That is
    /// the case none of the mirror's four guards can see. A window whose
    /// first frame asks for LESS than the last one rasterized drops the fill
    /// ratio and refreshes on that, which is why this survives casual use.
    #[test]
    fn a_fresh_context_is_shown_the_atlas_again() {
        let font = egui::FontId::monospace(13.0);
        let mut state =
            crate::SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);

        // A window's worth of labels, settled: the second pass draws what the
        // first pass rasterized, which is where the keys stop moving.
        let ctx = egui::Context::default();
        batch_keys(&ctx, &font, "C4 Eb5 G7");
        let opened = batch_keys(&ctx, &font, "C4 Eb5 G7");
        {
            let mirror = &state.instruments.font_atlas;
            assert!(atlas_if_changed(&ctx, mirror, opened.clone()).is_some(), "the first mirror");
            assert!(atlas_if_changed(&ctx, mirror, opened.clone()).is_none(), "nothing has moved");
        }

        // The window closes and another opens: a new context, and a new
        // renderer holding no atlas at all.
        state.release_context_resources();
        let reopened = egui::Context::default();
        batch_keys(&reopened, &font, "C4 Eb5 G7");
        let drawn = batch_keys(&reopened, &font, "C4 Eb5 G7");
        assert_eq!(drawn, opened, "the same labels rasterize to the same texels");

        assert!(
            atlas_if_changed(&reopened, &state.instruments.font_atlas, drawn).is_some(),
            "a context that has never been handed the atlas must be handed it",
        );
    }

    /// A scale that follows a zoom takes a new value on every frame of a drag,
    /// and every distinct SIZE is its own set of rasterized glyphs in egui's
    /// atlas. Snapping is what keeps a continuous gesture from asking for a
    /// continuum of them: everything inside one physical pixel is one size.
    ///
    /// The identity end matters as much as the bucketing — a label drawn at
    /// the framing its sizes were dialled for must come out at exactly those
    /// sizes, not a rounding away from them.
    #[test]
    fn snapping_bounds_the_sizes_a_zoom_can_ask_for() {
        // 15pt at 2x is 30 physical pixels, so scale 1 is already whole.
        assert_eq!(snap_scale(1.0, 15.0, 2.0), 1.0, "the dialled size is left alone");
        assert_eq!(snap_scale(1.0, 9.5, 2.0), 1.0, "19 physical pixels, already whole");
        // A base that isn't whole on the display it is drawn on lands on the
        // grid rather than sitting off it: the roll's name is 24.7 pixels at
        // 2x, and draws at 25.
        assert!((snap_scale(1.0, 12.35, 2.0) * 12.35 * 2.0 - 25.0).abs() < 1e-4);

        // A rung is 4%, so everything inside one lands on the same size...
        let bucket = |scale: f32| snap_scale(scale, 15.0, 2.0);
        assert_eq!(bucket(2.0), bucket(2.01));
        assert_eq!(bucket(2.0), bucket(2.03));
        let px = bucket(2.0) * 15.0 * 2.0;
        assert_eq!(px, px.round(), "...and it lands on a whole pixel: {px}");
        // ...while a step past one is a step, which is what makes the label
        // track the zoom at all.
        assert_ne!(bucket(2.0), bucket(2.2));
        // Over a sixfold zoom that is some 45 sizes, where a pixel grid at
        // these sizes let through 300 — the whole point of the ladder.
        let rungs: std::collections::HashSet<u32> =
            (0..600).map(|i| snap_scale(1.0 + i as f32 / 100.0, 30.0, 2.0).to_bits()).collect();
        assert!(rungs.len() < 60, "{} distinct sizes across a sixfold zoom", rungs.len());

        // A nonsense denominator leaves the scale alone rather than dividing
        // by it: no size this could produce is better than the one asked for.
        assert_eq!(snap_scale(1.5, 0.0, 2.0), 1.5);
        assert_eq!(snap_scale(1.5, 15.0, 0.0), 1.5);
        assert_eq!(snap_scale(1.5, f32::NAN, 2.0), 1.5);
        // A scale that is not a number is a different matter: there is no
        // size to snap, and passing it on draws nothing at all.
        assert_eq!(snap_scale(f32::NAN, 15.0, 2.0), 1.0);
        assert_eq!(snap_scale(f32::INFINITY, 15.0, 2.0), 1.0);
        // And nothing snaps to zero: a floored pixel is still a pixel...
        assert!(snap_scale(0.001, 15.0, 2.0) * 15.0 * 2.0 >= 1.0);
        // ...nor past what a rasterizer will take. Every factor feeding a
        // label's size is bounded on its own, but they multiply, and a glyph
        // wider than the atlas is not a big label: it is the overflow path
        // recycling texels that live glyphs point at.
        for absurd in [50.0, 1e6, f32::INFINITY] {
            let px = snap_scale(absurd, 30.0, 2.0) * 30.0 * 2.0;
            assert!(px <= MAX_GLYPH_PX, "a scale of {absurd} asked for {px} pixels of type");
        }
    }

    /// Type follows a zoom CONTINUOUSLY, while the atlas still sees only the
    /// ladder's rungs. Those are the two halves of the split, and each is
    /// worthless without the other: quantized drawing is the stepping this
    /// removes, and unquantized rasterizing is the atlas churn the ladder
    /// exists to prevent.
    ///
    /// Walked as a zoom walks — a scale creeping up by a fraction of a rung at
    /// a time — and measured on the drawn INK, which is what the eye reads.
    #[test]
    fn a_zoom_moves_the_drawn_size_smoothly_and_the_atlas_by_rungs() {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let (base, ppp) = (15.0, 2.0);
        ctx.set_pixels_per_point(ppp);

        // One frame of a zoom, with the split either applied or not: lay a piece
        // out at the rung `want` falls on, draw it magnified by the rest (or
        // not), and report its drawn ink and the size it was rasterized at.
        let frame = |want: f32, split: bool| -> (f32, u32) {
            let (raster, magnify) = ladder(want, base, ppp);
            let magnify = if split { magnify } else { 1.0 };
            let font = egui::FontId::monospace(base * raster);
            let mut batch = TextBatch::default();
            let anchor = egui::pos2(100.0, 100.0);
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                batch.magnified(anchor, magnify, |batch| {
                    batch.text(
                        ui.painter(),
                        anchor,
                        egui::Align2::CENTER_CENTER,
                        "Ag".to_owned(),
                        font.clone(),
                        egui::Color32::WHITE,
                        egui::Color32::BLACK,
                    );
                });
            });
            (batch.pieces()[0].ink.width(), font.size.to_bits())
        };
        let walk = |split: bool| -> Vec<(f32, u32)> {
            (0..=120).map(|i| frame(1.0 + i as f32 * 0.02, split)).collect()
        };
        // The worst single frame of a walk, and how far it grew overall.
        let worst = |steps: &[(f32, u32)]| -> f32 {
            steps.windows(2).map(|w| w[1].0 - w[0].0).fold(f32::MIN, f32::max)
        };

        let split = walk(true);
        let quantized = walk(false);

        // The atlas sees rungs either way: a 3.4x zoom asks for a couple of
        // dozen sizes, not one per frame. That is the half the ladder is for,
        // and the half the split must not cost.
        let sizes: std::collections::HashSet<u32> = split.iter().map(|(_, f)| *f).collect();
        assert_eq!(
            sizes,
            quantized.iter().map(|(_, f)| *f).collect::<std::collections::HashSet<u32>>(),
            "the split must not change WHAT is rasterized, only what is drawn",
        );
        assert!(sizes.len() * 2 < split.len(), "{} sizes over {} frames", sizes.len(), split.len());
        assert!(sizes.len() > 5, "the rasterized size must still FOLLOW the zoom");

        // The picture does not step with them. Measured against the same walk
        // drawn at the rung — a self-calibrating comparison, since what is left
        // over is epaint's own per-size metric rounding, which no amount of
        // magnification undoes and which a fixed threshold here would either
        // forgive entirely or trip over on a font change.
        let (a, b) = (worst(&split), worst(&quantized));
        assert!(
            a * 3.0 < b,
            "worst frame {a} against {b} at the rung — the drawn size is still stepping",
        );
        assert!(
            split[split.len() - 1].0 > split[0].0 * 3.0,
            "the label has to actually grow with the zoom",
        );
    }

    /// The pair reconstructs the size that was asked for — and stops doing so
    /// exactly where the rasterized size stops rising.
    ///
    /// The ceiling is the point. [`snap_scale`] clamps what is rasterized at
    /// [`MAX_GLYPH_PX`], so a magnification taken against the raw request
    /// absorbs everything past it and the DRAWN size is bounded by nothing: a
    /// camera zoomed fully in with the Size bar at its top asks for type half
    /// again the size of the cell behind it, and a hand-edited camera for many
    /// times that — a blurred label rather than the bounded one the ceiling is
    /// there to promise. `harmonigraph_scene::Camera` says in as many words
    /// that what a label may finally be sized at is bounded downstream of it,
    /// and this is the downstream.
    #[test]
    fn the_ladder_reconstructs_a_size_and_bounds_it_at_the_ceiling() {
        let (base, ppp) = (30.0, 2.0);
        let drawn = |want: f32| {
            let (raster, magnify) = ladder(want, base, ppp);
            (raster * magnify * base * ppp, magnify)
        };

        // In the ordinary range the two halves put back exactly what was asked
        // for, which is the whole claim: the ladder is invisible in the
        // picture and visible only in the atlas.
        for want in [0.5, 1.0, 1.37, 2.0, 5.5, 8.0] {
            let (px, _) = drawn(want);
            assert!(
                (px - want * base * ppp).abs() < 1e-2,
                "a scale of {want} drew {px} pixels, not {}",
                want * base * ppp,
            );
        }

        // Past the ceiling it stops, rather than growing softer without bound.
        for want in [8.6, 12.0, 18.0, 60.0, 1e6] {
            let (px, magnify) = drawn(want);
            assert!(px <= MAX_GLYPH_PX + 1e-2, "a scale of {want} drew {px} pixels of type");
            assert!(magnify <= 1.05, "a scale of {want} magnified by {magnify}");
        }

        // The analyzer's names are quoted against their own dialled size, so
        // scale 1 is a RUNG and the residual at the default view is the pixel
        // grain alone — 12.35pt is 24.7 pixels at 2x, which no raster can be,
        // so half a pixel is the whole of what is left. Anchored against the
        // lattice's 30pt instead, 12.35 falls between two rungs and a static
        // pane pays several times that for a continuity it is not using. See
        // `panes::spectral::names::draw`.
        let (raster, magnify) = ladder(1.0, 12.35, 2.0);
        assert!((raster * 12.35 * 2.0 - 25.0).abs() < 1e-3, "the rung lands on the pixel above");
        assert!(
            (magnify * 25.0 - 24.7).abs() < 1e-3,
            "the dialled size must be DRAWN at 24.7 pixels, off a 25-pixel raster",
        );
        assert!((magnify - 1.0).abs() < 0.5 / 24.7, "within half a pixel: {magnify}");

        // Nonsense in, something drawable out — and never a magnification that
        // would make a label vanish or fill the pane.
        for want in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let (_, magnify) = ladder(want, base, ppp);
            assert!(magnify.is_finite() && magnify > 0.0, "{want} magnified by {magnify}");
        }
        assert!(ladder(1.0, 0.0, ppp).1.is_finite(), "a zero base");
        assert!(ladder(1.0, base, 0.0).1.is_finite(), "a zero scale factor");
    }

    /// A SCALE change re-rasterizes every glyph and hands out new UVs, and the
    /// mirror's other two triggers are both blind to it: the atlas keeps its
    /// dimensions, and a pane drawing the same characters at the same POINT size
    /// reports no pair it has not seen. Without a trigger of its own the GPU
    /// would hold the old pixels while every glyph indexes the new layout, so
    /// each label would sample whatever now sits where its glyph used to.
    ///
    /// The two assertions before the point are what makes it one: they pin the
    /// egui behaviour the bug is made of, so if a future version repacks to a
    /// different SIZE this stops quietly passing for the wrong reason.
    #[test]
    fn a_scale_change_refreshes_the_mirror_though_nothing_else_moves() {
        let state = crate::SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        let ctx = egui::Context::default();
        let font = egui::FontId::proportional(12.0);

        let (size_1x, drawn_1x) = draw_at(&ctx, &font, "Ag1", 1.0);
        let mirror = &state.instruments.font_atlas;
        assert!(atlas_if_changed(&ctx, mirror, drawn_1x.clone()).is_some(), "the first mirror");
        assert!(
            atlas_if_changed(&ctx, mirror, drawn_1x.clone()).is_none(),
            "an unchanged frame must not re-upload the atlas — that is the point of the mirror"
        );

        let (size_2x, drawn_2x) = draw_at(&ctx, &font, "Ag1", 2.0);
        assert_eq!(size_2x, size_1x, "the atlas takes the new glyphs at the SAME dimensions");
        let point_size = |k: &GlyphKey| (k.0, k.1);
        assert_eq!(
            drawn_2x.iter().map(point_size).collect::<Vec<_>>(),
            drawn_1x.iter().map(point_size).collect::<Vec<_>>(),
            "the pane asks for the same characters at the same POINT size",
        );
        assert_ne!(drawn_2x, drawn_1x, "...and they are rasterized somewhere else entirely");

        assert!(
            atlas_if_changed(&ctx, mirror, drawn_2x).is_some(),
            "a scale change must refresh the mirror: the UVs moved, so the pixels must follow"
        );
    }
}
