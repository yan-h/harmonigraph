//! Haloed label text: collected as glyphs here, drawn by
//! `harmonigraph_render`'s own text module.
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
//!
//! A label's DRAWN marks — the accidentals, the syntonic `+`/`−`, the
//! septimal chevron — are glyphs here too. They are rasterized by `marks`
//! rather than by egui and packed into a sheet of their own
//! ([`MarkAtlas`]), and from there everything is shared: one quad apiece, the
//! rim from `fs_rim`'s arithmetic rather than a second bitmap, and a place in
//! whichever run of letters they were collected with. That last is what the
//! move is for — a mark on the painter is drawn over the finished picture,
//! where the node in front that just covered the name beside it cannot reach
//! it.

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
const RINGS: [(f32, f32, u32); 2] = [(2.0, 0.21, 8), (1.2, 1.0, 12)];

/// One ring's radius on this display, rounded to a whole physical pixel.
///
/// A SIZE, and the one thing here that is still rounded now that positions
/// are not: a sub-pixel or mixed-fraction radius reads as a lumpy outline,
/// and unlike a position it is a constant of the frame, so rounding it
/// cannot make anything step as it moves.
///
/// One rim for everything the pass draws, marks included: they are instances
/// of the same shader, so the `+` and the letter beside it take their halo
/// from one radius by construction rather than from two constants that have
/// to be kept equal.
fn ring_radius(radius: f32, ppp: f32) -> f32 {
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
///     (`marks::mark_key`), which is what makes a name and the `+`
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
/// so a camera zoomed fully in with the Name size bar at 3 would ask for type twice
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

/// How much empty box a piece of text leaves between its INK and the edge of
/// its layout box on the side `toward` points at, in points.
///
/// A label is placed by its box, and for a line of digits that box is a good
/// deal taller than the digits: the font's ascent stands above the figures and
/// its descent hangs below them, and both scale with the point size. So a
/// label set one point off a line is one point plus a descent off it, and the
/// second term grows with the type — the number drifts off the thing it names
/// exactly at the sizes where the drift is easiest to see. Subtracting this
/// from the anchor makes the gap the caller asks for the gap a reader gets, at
/// every size.
///
/// Measured off the galley rather than taken as a fraction of the em, which is
/// what makes it worth a function: a ratio is right for one typeface and
/// silently wrong for the next, and it would be wrong in the direction that
/// looks fine on the machine it was tuned on. egui caches galleys, so laying
/// the text out here and again where it is drawn costs one layout and a
/// lookup.
///
/// `toward` is a screen direction, and only its dominant axis is read — the
/// callers hand it an axis off [`Axes`](crate::panes::spectral::axes::Axes),
/// which is always one of the four cardinals. Text with no ink at all (an
/// empty label, a string of spaces) has no edge to measure, and reports 0.
pub(crate) fn ink_inset(
    painter: &egui::Painter,
    text: &str,
    font: &egui::FontId,
    toward: egui::Vec2,
) -> f32 {
    let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER);
    // The same glyph rects `TextBatch::text` draws, in the galley's own space
    // — read here rather than out of the batch because this has to answer
    // BEFORE the anchor it corrects is handed over.
    let ink = galley
        .rows
        .iter()
        .flat_map(|row| {
            row.glyphs.iter().filter(|g| !g.uv_rect.is_nothing()).map(|glyph| {
                let left_top = row.pos + (glyph.pos + glyph.uv_rect.offset).to_vec2();
                egui::Rect::from_min_size(left_top, glyph.uv_rect.size)
            })
        })
        .reduce(|a, b| a.union(b));
    let Some(ink) = ink else { return 0.0 };
    let size = galley.size();
    if toward.x.abs() > toward.y.abs() {
        if toward.x > 0.0 {
            size.x - ink.max.x
        } else {
            ink.min.x
        }
    } else if toward.y > 0.0 {
        size.y - ink.max.y
    } else {
        ink.min.y
    }
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
/// size a person would ask for. Held under the NARROWEST atlas epaint builds —
/// 2048, what it takes when a context is told no limit — rather than under the
/// 8192 both shells report off their wgpu device, so the bound holds for a
/// context that is told nothing as well as for the ones that are.
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
/// The spiral's rim names. ONE, where the analyzer has one per surface,
/// because the spiral is drawn at most once in a frame: the dock holds one tab
/// per pane, and the Video panel's preview composes `Layout::split`, which
/// places no spiral at all. A hand-written offline layout with two spirals in
/// it would want a second — the same assumption `draw_pane` already makes
/// about the analyzer's texture slot.
pub(crate) const SPIRAL_NAMES: u64 = 1;
/// The analyzer's, one per surface (docked, then the preview).
///
/// LAST, and every constant above it: this is the one id here that is a
/// function, so it is the only one whose range grows on its own. A constant
/// placed inside that range collides with nothing until a surface is added and
/// then collides in silence, which is why a new id goes above this line rather
/// than after the number it happens to hand out today.
pub(crate) fn spectral_labels(surface: usize) -> u64 {
    2 + surface as u64
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
    /// Test-only: the quad of every drawn mark, in the order they were added.
    /// A mark carries no text to be found by, and it is a `GlyphInstance` like
    /// any other once it is in `glyphs`, so this is the only handle a
    /// typography test has on which mark went where.
    #[cfg(test)]
    marks: Vec<egui::Rect>,
}

/// One `text()` call, for tests: what it said, at what size, the ink it covers
/// on screen and the colour that ink was laid down in.
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct TextPiece {
    pub text: String,
    pub font_size: f32,
    /// What the glyphs were filled with — the same premultiplied bytes every
    /// [`GlyphInstance`] of this piece carries. Per PIECE, which is what makes
    /// it worth keeping: a lattice label is two calls, its name and its cents
    /// line, and whether they are drawn in one colour is a claim only a test
    /// that can see both can hold.
    pub fill: egui::Color32,
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
    /// Both colors' ALPHA is the label's strength, and each is worth it in
    /// full: a rim at half covers half, exactly as the fill does, so fading
    /// the pair together fades one thing (`fs_rim` in `harmonigraph_render`'s
    /// text shader). Fading only the fill leaves the halo behind, and the
    /// halo is the letter's own shape in the skin's darkest color.
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
        //
        // Which is a claim on the shader, and the reason the margin in
        // `text.wgsl`'s `coverage` is load-bearing rather than a detail of
        // sampling: the offset is only a softening while the coverage it
        // resamples is continuous across the glyph's own patch edge. Cut
        // there and the softness stops being constant — a letter's edge
        // column snaps on and off once per pixel it travels, which is this
        // paragraph's own bug arriving by the other road.
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
                    atlas: GlyphInstance::TYPE,
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
                self.pieces.push(TextPiece { text: said, font_size, fill: color, ink, galley });
            }
        }
    }

    /// Add one drawn mark, centred on `center` and haloed like the glyphs it
    /// sits among — the same instance stream, the same rim, the same run.
    ///
    /// The quad is the mark's BITMAP rather than its ink, margin included, and
    /// the uv is that bitmap's whole patch. The two are the same box in two
    /// spaces, exactly as a glyph's screen rect and atlas patch are, which is
    /// what keeps the mark's own ink interior to the quad that carries it
    /// (see `marks::mark_geometry`) once the shader grows both by the
    /// rim's reach.
    ///
    /// Returns the bitmap's size in texels, which is what the caller needs to
    /// know how far the mark reaches — and it is the caller that knows how
    /// much of that is margin.
    pub(crate) fn mark(
        &mut self,
        ctx: &egui::Context,
        key: crate::marks::MarkKey,
        center: egui::Pos2,
        ppp: f32,
        color: egui::Color32,
        outline: egui::Color32,
    ) -> [u32; 2] {
        let sheet = mark_sheet(ctx);
        let patch = sheet
            .lock()
            .expect("the mark sheet is never held across a panic")
            .patch(key, ctx.cumulative_pass_nr());
        // Rasterized in device pixels and drawn in points, which is the one
        // boundary a mark crosses that a glyph does not have to be told about:
        // egui's own text path crosses it inside the galley.
        let size = egui::vec2(patch.size[0] as f32, patch.size[1] as f32) / ppp;
        // The same split the type takes, by the same route: laid out at the
        // rasterized size, then magnified with everything else in the label.
        let k = self.magnify.map_or(1.0, |m| m.factor);
        let min = center - size / 2.0;
        let min = self.magnify.map_or(min, |m| m.point(min));
        let [x, y] = patch.at.map(|n| n as f32);
        self.glyphs.push(GlyphInstance {
            rect: [min.x, min.y, size.x * k, size.y * k],
            uv: [x, y, x + patch.size[0] as f32, y + patch.size[1] as f32],
            fill: color.to_array(),
            rim: outline.to_array(),
            atlas: GlyphInstance::MARK,
        });
        #[cfg(test)]
        self.marks.push(egui::Rect::from_min_size(min, size * k));
        patch.size
    }

    /// Every piece of text collected so far (tests only).
    #[cfg(test)]
    pub(crate) fn pieces(&self) -> &[TextPiece] {
        &self.pieces
    }

    /// Which nodes this batch drew a name over, in the order it drew them
    /// (tests only). One entry per label that put at least one glyph down, so
    /// this is exactly the set of nodes that ARE named — which is what the
    /// resting markers have to be the complement of.
    #[cfg(test)]
    pub(crate) fn labels(&self) -> &[harmonigraph_render::Label] {
        &self.labels
    }

    /// Every drawn mark's quad, in the order they were added (tests only).
    #[cfg(test)]
    pub(crate) fn marks(&self) -> &[egui::Rect] {
        &self.marks
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
    ///
    /// `slide` is which way this batch's text travels, which the glyph
    /// shader's reconstruction filter follows. It is asked of the caller
    /// rather than defaulted because the caller is the only one that knows:
    /// the analyzer's names ride its time axis and so scroll whichever way
    /// its orientation points that, and nothing here can see it.
    pub(crate) fn flush(
        &mut self,
        painter: &egui::Painter,
        rect: egui::Rect,
        state: &crate::SharedState,
        pane_id: u64,
        slide: harmonigraph_render::SlideAxis,
    ) {
        if self.glyphs.is_empty() {
            return;
        }
        #[cfg(test)]
        {
            self.pieces.clear();
            self.marks.clear();
        }
        let atlas = atlas_if_changed(
            painter.ctx(),
            &state.instruments.font_atlas,
            std::mem::take(&mut self.drawn),
        );
        let marks = marks_if_changed(painter.ctx(), &state.instruments.font_atlas);
        painter.add(harmonigraph_render::text_paint_callback(
            rect,
            std::mem::take(&mut self.glyphs),
            rings(painter.ctx()),
            atlas,
            marks,
            slide,
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
        rect: egui::Rect,
        scene: &harmonigraph_scene::Scene,
        state: &crate::SharedState,
    ) -> harmonigraph_render::LatticeLabels {
        #[cfg(test)]
        {
            self.pieces.clear();
            self.marks.clear();
        }
        // A batch that drew nothing publishes nothing. [`flush`](Self::flush)
        // says this by returning outright; this has a value to hand back, so
        // it skips the publication alone — and it has to skip it explicitly,
        // because `atlas_if_changed` cannot tell "no glyphs to check" from "a
        // mirror holding nothing". Until the first lattice name is drawn its
        // `seen` is empty, which is the arm that reports a resize, so an
        // empty `drawn` would hand back the whole atlas and a new key on
        // every frame — and the lattice pane draws no names at all whenever
        // `show_labels` is off, or before the first note or hover.
        let (atlas, marks) = if self.glyphs.is_empty() {
            (None, None)
        } else {
            (
                atlas_if_changed(
                    painter.ctx(),
                    &state.instruments.lattice_atlas,
                    std::mem::take(&mut self.drawn),
                ),
                marks_if_changed(painter.ctx(), &state.instruments.lattice_atlas),
            )
        };
        let mut glyphs = std::mem::take(&mut self.glyphs);
        for glyph in &mut glyphs {
            glyph.rect[0] -= rect.min.x;
            glyph.rect[1] -= rect.min.y;
        }
        harmonigraph_render::LatticeLabels {
            glyphs,
            labels: std::mem::take(&mut self.labels),
            // The one thing the glyph pass cannot work out for itself: a node
            // is world-space geometry and a name is typeset in points, so the
            // standoff a name casts — dialled in node radii, like every other
            // standoff in the picture — has no unit until the pane says how
            // large a node draws on it.
            node_points: scene.node_radius * scene.camera.points_per_world(rect.height()),
            atlas,
            marks,
            // The default, and here that is a want of an answer rather than
            // one: an orbiting camera carries a node name across the screen
            // and up it at once, so there is no axis its motion is along.
            // See `FILTER_TAP` in the glyph shader.
            slide: harmonigraph_render::SlideAxis::default(),
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
    /// The version of the MARK sheet this renderer was last handed. Nothing
    /// like the four guards above is needed for it: that sheet is ours, so it
    /// carries a version of its own and this is simply which one has been
    /// published here (see [`marks_if_changed`]). It rides along in this
    /// struct because a mirror answers for one RENDERER's copies, and each
    /// renderer holds both sheets.
    marks_key: u64,
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
    ///
    /// `marks_key` goes, and for the opposite reason: the mark sheet lives in
    /// the context's own data store, so a new context builds a new one, and
    /// "already published" here would name a sheet that no longer exists.
    /// Belt and braces — [`next_sheet_key`] is process-wide, so the new
    /// sheet's versions are past this one's anyway.
    pub(crate) fn forget_context(&mut self) {
        self.seen.clear();
        self.size = [0, 0];
        self.ppp = 0;
        self.fill = 0.0;
        self.marks_key = 0;
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

/// The drawn marks' sheet, on the frames one renderer's mirror of it is
/// stale, and `None` on the rest.
///
/// Simpler than [`atlas_if_changed`], and for one reason: this atlas is OURS.
/// egui's has to be probed for staleness a glyph at a time because nothing
/// says when it moved; here the only thing that moves it is a mark being
/// packed, which happens right here, so the sheet carries a version and the
/// comparison is that version against what this renderer was last handed.
fn marks_if_changed(
    ctx: &egui::Context,
    mirror: &std::sync::Mutex<AtlasMirror>,
) -> Option<harmonigraph_render::FontAtlas> {
    let sheet = mark_sheet(ctx);
    let sheet = sheet.lock().expect("the mark sheet is never held across a panic");
    // Nothing has ever been packed — a shell that draws no note names, or a
    // session before its first one. There is no sheet to hand over.
    if sheet.key == 0 {
        return None;
    }
    let mut mirror = mirror.lock().expect("the label mirror is never held across a panic");
    if mirror.marks_key == sheet.key {
        return None;
    }
    mirror.marks_key = sheet.key;
    Some(harmonigraph_render::FontAtlas { image: sheet.image.clone(), key: sheet.key })
}

/// Where one mark sits in the sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MarkPatch {
    /// Top-left texel of the patch.
    pub(crate) at: [u32; 2],
    /// The bitmap's own size in texels, which is also the mark's quad in
    /// device pixels — the clear margin
    /// ([`MARK_BITMAP_PAD`](crate::marks::MARK_BITMAP_PAD)) is part
    /// of both.
    pub(crate) size: [u32; 2],
}

/// The drawn marks, packed into one sheet the glyph pass binds beside egui's
/// font atlas.
///
/// A mark is a bitmap of coverage keyed on (design, size, weight) — the same
/// thing a font rasterizer hands an atlas for a glyph — so what it wants is
/// what egui's atlas gives type: rasterize once, pack, hand out a patch, and
/// re-upload only when the pixels move. A `egui::TextureHandle` per bitmap is
/// the alternative, and it forces marks onto the painter's image path and out
/// of the pass their letters are drawn in.
///
/// Packed rather than shared with egui's own image, though that would need no
/// second binding: a patch in a shared sheet has to keep the whole sheet's
/// texel:pixel identity, and `text.wgsl` steps the rim in texels at
/// `pixels_per_point` on that basis. A sheet of our own keeps that identity
/// per texture, so a mark could be rasterized finer than the display without
/// dragging every letter with it (issue #304).
///
/// Two rules make the sheet safe to hand out mid-frame, and they are the same
/// two egui's atlas keeps (see `harmonigraph_render::text`'s `mirror_atlas`):
/// a patch is only ever APPENDED within a pass, and the sheet only ever grows
/// downward, so a uv issued earlier in the pass still points at its own mark.
/// A repack waits for the next pass, where it runs before any uv is issued.
#[derive(Default)]
pub(crate) struct MarkAtlas {
    /// Behind an `Arc` because publishing it is handing this very image to a
    /// renderer: a mutation while one is still held clones (`Arc::make_mut`),
    /// and the ordinary case — nobody holding it — mutates in place.
    image: std::sync::Arc<egui::ColorImage>,
    at: std::collections::HashMap<crate::marks::MarkKey, MarkPatch>,
    /// The shelf being filled: the row it starts on, how tall it is, and how
    /// far along it the next patch goes.
    shelf: (u32, u32, u32),
    /// Which version of the pixels this is. Zero is "nothing packed yet";
    /// every other value comes from [`next_sheet_key`], so no two states of
    /// any sheet in the process ever share one.
    key: u64,
    /// The pass the marks below were asked for on, and which of them were —
    /// the set a repack keeps. Read at the pass boundary, which is the only
    /// place the packing may move.
    pass: u64,
    used: std::collections::HashSet<crate::marks::MarkKey>,
}

/// How wide the sheet is, in texels.
///
/// Wide enough that no mark can fail to fit on a shelf, which is what makes
/// the packer a packer rather than a packer plus a fallback: a label's type is
/// bounded at [`MAX_GLYPH_PX`] and every mark sets at a fixed fraction of it,
/// so the widest bitmap the app can ask for is a constant —
/// `a_mark_is_never_wider_than_the_sheet_it_is_packed_into` in
/// `marks` measures it. Four of those to a shelf at the ceiling, and
/// dozens at the sizes a label is really drawn at.
pub(crate) const MARK_SHEET_WIDTH: u32 = 512;

/// How tall the sheet may get before a pass boundary repacks it down to what
/// is actually being drawn.
///
/// A ceiling on the CHURN, not on the live set: zooming walks through sizes
/// and each is its own bitmap, so an hour's use would otherwise pack every
/// size the camera ever passed through. A frame whose own marks need more than
/// this gets more than this — the repack is skipped when there is nothing dead
/// to drop, since repacking a live set to the same size every pass is a
/// full-sheet re-upload per frame for nothing.
const MARK_SHEET_SOFT_HEIGHT: u32 = 512;

/// Where the [`MarkAtlas`] lives: in egui's own per-frame data store, keyed on
/// the context, so it dies with the window that built it — exactly as the
/// textures a renderer holds do.
fn mark_sheet(ctx: &egui::Context) -> std::sync::Arc<std::sync::Mutex<MarkAtlas>> {
    let id = egui::Id::new("harmonigraph-mark-sheet");
    type Shared = std::sync::Arc<std::sync::Mutex<MarkAtlas>>;
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Shared>(id).clone())
}

/// The next version any sheet in this process may take.
///
/// Process-wide and monotone, so a mirror's "I already hold key K" can never
/// be true of a DIFFERENT sheet's K: egui evicts an untouched entry from its
/// data store, and a context that closes takes its sheet with it, so a fresh
/// [`MarkAtlas`] counting from its own zero would eventually mint a key some
/// renderer still names — and answer a stale mirror with "nothing has moved"
/// while holding another sheet's pixels.
fn next_sheet_key() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl MarkAtlas {
    /// Where `key`'s bitmap sits, rasterizing and packing it if this is the
    /// first time this pass has been asked.
    fn patch(&mut self, key: crate::marks::MarkKey, pass: u64) -> MarkPatch {
        if pass != self.pass {
            self.retire();
            self.pass = pass;
            self.used.clear();
        }
        self.used.insert(key);
        if let Some(&patch) = self.at.get(&key) {
            return patch;
        }
        let image = crate::marks::rasterize_mark(key);
        let size = [image.size[0] as u32, image.size[1] as u32];
        let at = self.claim(size);
        self.blit(&image, at);
        let patch = MarkPatch { at, size };
        self.at.insert(key, patch);
        self.key = next_sheet_key();
        patch
    }

    /// Drop what the last pass did not draw, and pack what it did afresh.
    ///
    /// Only worth doing when there is something to drop AND the sheet has
    /// grown past what it should be carrying — see [`MARK_SHEET_SOFT_HEIGHT`].
    fn retire(&mut self) {
        if self.image.height() as u32 <= MARK_SHEET_SOFT_HEIGHT || self.used.len() >= self.at.len()
        {
            return;
        }
        // Tallest first, so a shelf is set by its tallest member and the rest
        // fill in beside it rather than each mark starting one of its own.
        //
        // Broken by the old POSITION, which is unique per patch, so the order
        // is total and the packing is a function of the frames that came
        // before rather than of a `HashMap`'s iteration. The picture would be
        // the same either way — a patch holds its own mark wherever it lands —
        // but the offline renderer promises byte-identical frames between
        // runs, and "identical unless you look at the atlas" is not a promise
        // worth having to re-derive.
        let mut keep: Vec<_> = self
            .at
            .iter()
            .filter(|(key, _)| self.used.contains(*key))
            .map(|(&key, &patch)| (key, patch))
            .collect();
        keep.sort_by_key(|(_, p)| std::cmp::Reverse((p.size[1], p.size[0], p.at[1], p.at[0])));

        let old = std::mem::take(&mut self.image);
        self.at.clear();
        self.shelf = (0, 0, 0);
        for (key, patch) in keep {
            let at = self.claim(patch.size);
            self.copy_patch(&old, patch, at);
            self.at.insert(key, MarkPatch { at, size: patch.size });
        }
        self.key = next_sheet_key();
    }

    /// Reserve `size` texels on the current shelf (or the next), growing the
    /// sheet downward if the shelf runs off the bottom.
    fn claim(&mut self, [w, h]: [u32; 2]) -> [u32; 2] {
        // A mark too wide for a shelf has nowhere to go, and what it would do
        // instead is write across the row into whatever is packed under it.
        // Unreachable rather than handled: the widest mark the app can ask for
        // is a constant of the size ladder's ceiling, and
        // `a_mark_is_never_wider_than_the_sheet_it_is_packed_into` is where
        // that is worked out.
        debug_assert!(w <= MARK_SHEET_WIDTH, "a mark {w} texels wide cannot be packed");
        let (mut top, mut height, mut x) = self.shelf;
        if x + w > MARK_SHEET_WIDTH {
            top += height;
            height = 0;
            x = 0;
        }
        let at = [x, top];
        self.shelf = (top, height.max(h), x + w);
        self.grow(top + h);
        at
    }

    /// Make sure the sheet is at least `rows` tall, appending transparent rows
    /// rather than repacking: everything already in it keeps its texels, which
    /// is what lets a uv handed out earlier in this pass stay correct.
    fn grow(&mut self, rows: u32) {
        if self.image.height() as u32 >= rows {
            return;
        }
        // A power of two, so a shelf at a time does not mean an upload at a
        // time: a sheet filling up doubles a handful of times and then stops.
        let rows = rows.next_power_of_two() as usize;
        let image = std::sync::Arc::make_mut(&mut self.image);
        image.pixels.resize(MARK_SHEET_WIDTH as usize * rows, egui::Color32::TRANSPARENT);
        image.size = [MARK_SHEET_WIDTH as usize, rows];
        image.source_size = egui::vec2(MARK_SHEET_WIDTH as f32, rows as f32);
    }

    /// Copy a freshly rasterized bitmap into the sheet at `at`.
    fn blit(&mut self, mark: &egui::ColorImage, [x, y]: [u32; 2]) {
        let stride = MARK_SHEET_WIDTH as usize;
        let (w, h) = (mark.size[0], mark.size[1]);
        let image = std::sync::Arc::make_mut(&mut self.image);
        for row in 0..h {
            let from = row * w;
            let to = (y as usize + row) * stride + x as usize;
            image.pixels[to..to + w].copy_from_slice(&mark.pixels[from..from + w]);
        }
    }

    /// Copy a patch out of a previous sheet, for a repack.
    fn copy_patch(&mut self, old: &egui::ColorImage, from: MarkPatch, [x, y]: [u32; 2]) {
        let stride = MARK_SHEET_WIDTH as usize;
        let (w, h) = (from.size[0] as usize, from.size[1] as usize);
        let image = std::sync::Arc::make_mut(&mut self.image);
        for row in 0..h {
            let src = (from.at[1] as usize + row) * old.size[0] + from.at[0] as usize;
            let dst = (y as usize + row) * stride + x as usize;
            image.pixels[dst..dst + w].copy_from_slice(&old.pixels[src..src + w]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::marks::{mark_key, MarkKind, MARK_WEIGHT};

    /// Backing an anchor off by [`ink_inset`] lands the label's INK where the
    /// caller asked for its box — on both axes, and at any size.
    ///
    /// The size-independence is the whole claim. What sits between a layout
    /// box and the digits inside it is the font's own ascent, descent and side
    /// bearings, all of which grow with the point size, so a caller that
    /// spaces a label off a line by a constant gets the constant plus a term
    /// that quadruples when the type does. The analyzer's frequency labels are
    /// that caller, and a number drifting off the ruling it names as the pane
    /// grows is what this exists to stop.
    ///
    /// Both axes because the analyzer turns: its pitch axis runs up the screen
    /// in the wide orientations and across it in the tall ones, so the edge a
    /// label presents to its ruling is a descent in one pair and a side
    /// bearing in the other.
    #[test]
    fn an_ink_correction_lands_a_label_the_same_way_at_any_size() {
        let anchor = egui::pos2(100.0, 50.0);
        // The label's box grows up and to the right of the anchor, so the
        // edges facing back at it are its bottom and its left.
        let placed = |size: f32, facing: egui::Vec2| {
            let mut batch = TextBatch::default();
            let font = egui::FontId::monospace(size);
            let mut inset = 0.0;
            let _ = crate::tests::probe::painted_full(egui::vec2(400.0, 200.0), |ui| {
                inset = ink_inset(ui.painter(), "500", &font, facing);
                batch.text(
                    ui.painter(),
                    anchor + facing * inset,
                    egui::Align2::LEFT_BOTTOM,
                    "500".to_owned(),
                    font.clone(),
                    egui::Color32::WHITE,
                    egui::Color32::BLACK,
                );
            });
            (inset, batch.pieces()[0].ink)
        };

        let (down, left) = (egui::vec2(0.0, 1.0), egui::vec2(-1.0, 0.0));
        for size in [10.0, 40.0] {
            let (inset, ink) = placed(size, down);
            assert!(
                (ink.max.y - anchor.y).abs() < 0.5,
                "{size}pt type left the ink {} off the anchor it was corrected onto",
                ink.max.y - anchor.y,
            );
            assert!(inset > 0.0, "{size}pt type reported no descent to correct for");

            let (inset, ink) = placed(size, left);
            assert!(
                (ink.min.x - anchor.x).abs() < 0.5,
                "{size}pt type left the ink {} off the anchor across the other axis",
                ink.min.x - anchor.x,
            );
            // No assertion that this one is nonzero: a monospace digit's side
            // bearing is a twentieth of the em, which rounds to no whole
            // pixel at all until the type is large. The correction is
            // therefore 0 at small sizes, and 0 is the right answer there.
            let _ = inset;
        }

        // And it is the SCALING term it removes, not a fixed one: quadruple
        // the type and the air quadruples with it. A constant would pass every
        // assertion above at one size and none of them at the other.
        let small = placed(10.0, down).0;
        let big = placed(40.0, down).0;
        assert!(big > small * 3.0, "a 4x size grew the descent only from {small} to {big}");

        // Nothing to measure reports nothing rather than a box's worth of
        // air: a label with no ink has no edge for a ruling to be near.
        let _ = crate::tests::probe::painted_full(egui::vec2(400.0, 200.0), |ui| {
            assert_eq!(ink_inset(ui.painter(), "", &egui::FontId::monospace(20.0), down), 0.0);
        });
    }

    /// A mark at a size in physical pixels, which is the axis a zoom walks:
    /// every rung of the size ladder is one more of these.
    fn mark_at(kind: MarkKind, size_px: f32) -> crate::marks::MarkKey {
        mark_key(kind, size_px, MARK_WEIGHT, 1.0)
    }

    /// A walk of `count` distinct sizes, tall enough that a few dozen of them
    /// fill the sheet past [`MARK_SHEET_SOFT_HEIGHT`] — which is what a zoom
    /// drag does, a rung at a time.
    fn a_zooms_worth(count: usize) -> Vec<crate::marks::MarkKey> {
        (0..count).map(|step| mark_at(MarkKind::Sharp, 60.0 + step as f32)).collect()
    }

    /// What the sheet holds at `patch`, read back out as an image — so a test
    /// can ask whether a mark is still where it was told it would be.
    fn patched(sheet: &MarkAtlas, patch: MarkPatch) -> egui::ColorImage {
        let (w, h) = (patch.size[0] as usize, patch.size[1] as usize);
        let stride = sheet.image.size[0];
        let pixels = (0..h)
            .flat_map(|row| {
                let at = (patch.at[1] as usize + row) * stride + patch.at[0] as usize;
                sheet.image.pixels[at..at + w].to_vec()
            })
            .collect();
        egui::ColorImage { size: [w, h], pixels, source_size: egui::vec2(w as f32, h as f32) }
    }

    /// Every mark a pass asked for is somewhere of its own in the sheet, and
    /// what is there is that mark's own bitmap.
    ///
    /// Two failures in one reading, and neither one shows up as an error. A
    /// packer that lets two patches meet draws each mark with a bite of its
    /// neighbour in it; one whose blit disagrees with the coordinates it hands
    /// out draws a mark that is simply somewhere else on the sheet, which is
    /// another mark or nothing at all.
    ///
    /// Sizes that grow, because a shelf packer is at its most interesting when
    /// the things it packs do not fit each other: even sizes fill a shelf
    /// tidily and hide an off-by-one that a ragged set exposes.
    #[test]
    fn every_mark_gets_a_patch_of_its_own() {
        let mut sheet = MarkAtlas::default();
        let kinds = [
            MarkKind::Minus,
            MarkKind::Plus,
            MarkKind::Sharp,
            MarkKind::Flat,
            MarkKind::Septimal(true),
        ];
        let mut packed = Vec::new();
        for step in 0..14 {
            for kind in kinds {
                let key = mark_at(kind, 7.0 + step as f32 * 3.0);
                packed.push((key, sheet.patch(key, 0)));
            }
        }

        // Touching is allowed and overlapping is not, so this is a strict
        // comparison rather than `Rect::intersects`, which counts a shared
        // edge as an intersection. Touching is in fact the ordinary case: a
        // shelf packs its marks side by side, and what keeps their INK apart
        // there is the clear texel each bitmap carries of its own.
        let apart = |a: &MarkPatch, b: &MarkPatch| {
            a.at[0] + a.size[0] <= b.at[0]
                || b.at[0] + b.size[0] <= a.at[0]
                || a.at[1] + a.size[1] <= b.at[1]
                || b.at[1] + b.size[1] <= a.at[1]
        };
        for (i, (_, a)) in packed.iter().enumerate() {
            assert!(
                a.at[0] + a.size[0] <= MARK_SHEET_WIDTH
                    && a.at[1] + a.size[1] <= sheet.image.height() as u32,
                "patch {i} runs off a sheet of {:?}: {a:?}",
                sheet.image.size,
            );
            for (j, (_, b)) in packed.iter().enumerate().skip(i + 1) {
                assert!(apart(a, b), "patches {i} and {j} share texels: {a:?} and {b:?}");
            }
        }
        for (i, (key, patch)) in packed.iter().enumerate() {
            assert_eq!(
                patched(&sheet, *patch).pixels,
                crate::marks::rasterize_mark(*key).pixels,
                "patch {i} does not hold the mark it was handed out for",
            );
        }
    }

    /// A patch handed out earlier in a pass still points at its own mark after
    /// the sheet has grown under it.
    ///
    /// This is the rule the whole publication scheme rests on, and it is the
    /// same one egui's atlas keeps: `harmonigraph_render`'s `mirror_atlas`
    /// carries a pane that has already prepared onto a texture some LATER pane
    /// grew, and what makes that safe is that the earlier pane's uvs still
    /// name their own texels. Repack inside a pass instead and every mark
    /// already drawn this frame is reading whatever moved into its place.
    ///
    /// Driven past the soft height on purpose: that is where a repack becomes
    /// tempting, and the test is that it does not happen HERE. The pass after
    /// it is the one allowed to move things.
    #[test]
    fn the_sheet_only_grows_downward_within_a_pass() {
        let mut sheet = MarkAtlas::default();
        let first = mark_at(MarkKind::Plus, 11.0);
        let early = sheet.patch(first, 0);
        for key in a_zooms_worth(80) {
            sheet.patch(key, 0);
        }
        assert!(
            sheet.image.height() as u32 > MARK_SHEET_SOFT_HEIGHT,
            "the walk has to fill the sheet past the point a repack would be due, got {:?}",
            sheet.image.size,
        );
        assert_eq!(sheet.patch(first, 0), early, "a patch moved under the pass that was handed it");
        assert_eq!(
            patched(&sheet, early).pixels,
            crate::marks::rasterize_mark(first).pixels,
            "the first mark's texels are not its own any more",
        );
    }

    /// ...and once a pass has gone by without them, the sheet packs back down
    /// to what is being drawn.
    ///
    /// Without that, an hour of zooming is an hour of sizes: each rung of the
    /// ladder is its own bitmap, so the sheet would carry every size the
    /// camera ever passed through and be re-uploaded at that size whenever a
    /// new one arrived.
    ///
    /// It takes a whole pass to notice, and it has to: what a mark being dead
    /// MEANS is that a pass drew without it, so the pass that stops asking is
    /// the evidence and the one after it is the earliest that can act. The
    /// walk here is one drag's worth of sizes, then a camera that has stopped.
    ///
    /// Skipping the repack when there is nothing dead to drop is the other
    /// half of the trade, and it belongs to
    /// [`a_sheet_still_drawing_all_its_marks_is_not_repacked`] rather than to
    /// this: what the camera settles on here is three marks, which puts the
    /// sheet back under the soft height, where height alone is already reason
    /// enough to leave it alone.
    #[test]
    fn a_pass_boundary_packs_the_sheet_back_down_to_what_is_drawn() {
        let mut sheet = MarkAtlas::default();
        for key in a_zooms_worth(80) {
            sheet.patch(key, 0);
        }
        let filled = sheet.image.height();
        assert!(filled as u32 > MARK_SHEET_SOFT_HEIGHT, "the walk has to fill the sheet: {filled}");

        // The camera stops: from here on the pass draws three marks at one
        // size, and asks for them again on every frame.
        let live: Vec<_> = [MarkKind::Sharp, MarkKind::Plus, MarkKind::Septimal(false)]
            .map(|kind| mark_at(kind, 13.0))
            .to_vec();
        for &key in &live {
            sheet.patch(key, 1);
        }
        assert_eq!(
            sheet.image.height(),
            filled,
            "the pass that stopped asking is the evidence, not the repack",
        );

        let patches: Vec<_> = live.iter().map(|&key| sheet.patch(key, 2)).collect();
        assert!(
            sheet.image.height() * 4 < filled,
            "the sheet stayed at {:?} a pass after the zoom stopped",
            sheet.image.size,
        );
        for (key, patch) in live.iter().zip(&patches) {
            assert_eq!(
                patched(&sheet, *patch).pixels,
                crate::marks::rasterize_mark(*key).pixels,
                "a repacked mark does not hold its own bitmap",
            );
        }

        // And every pass after that leaves it alone, rather than repacking a
        // live set to the same size on every frame.
        let settled = sheet.key;
        for pass in 3..6 {
            for &key in &live {
                sheet.patch(key, pass);
            }
        }
        assert_eq!(sheet.key, settled, "a pass that asked for nothing new must not move the sheet");
    }

    /// A sheet is left alone while everything in it is still being drawn,
    /// however tall it has grown.
    ///
    /// Height is what makes a repack worth doing and is not on its own what
    /// makes it worth anything: a repack that drops nothing packs the same
    /// marks to the same size, and mints a fresh key doing it. A fresh key is
    /// a full re-upload of the sheet into every renderer mirroring it, per
    /// pane, per frame, for a picture that has not moved — the one cost the
    /// sheet exists to avoid, and the one nothing downstream would look wrong
    /// about.
    ///
    /// So the live set here is itself past the soft height, which is the state
    /// the test above cannot reach: it settles on three marks, and under the
    /// soft height height alone answers whatever the second half of the guard
    /// does. A camera that stops with the sheet full is an ordinary way to
    /// arrive here — the drag ended on the frame that filled it.
    #[test]
    fn a_sheet_still_drawing_all_its_marks_is_not_repacked() {
        let mut sheet = MarkAtlas::default();
        let live = a_zooms_worth(80);
        for &key in &live {
            sheet.patch(key, 0);
        }
        let filled = sheet.image.height();
        assert!(
            filled as u32 > MARK_SHEET_SOFT_HEIGHT,
            "the LIVE set has to clear the soft height, or height alone answers: {filled}",
        );

        // The camera has stopped with the sheet full: every pass from here
        // asks for exactly what is already packed.
        let settled = sheet.key;
        for pass in 1..4 {
            for &key in &live {
                sheet.patch(key, pass);
            }
            assert_eq!(sheet.key, settled, "pass {pass} repacked a sheet with nothing dead in it");
            assert_eq!(sheet.image.height(), filled, "pass {pass} moved a sheet it must not touch");
        }
    }

    /// The sheet reaches a renderer when it moves, and not otherwise.
    ///
    /// One publication per change is the whole cost model: the sheet is a
    /// `ColorImage` handed over to be written into a texture whole, so an
    /// unconditional hand-over is a full re-upload per pane per frame. Nothing
    /// downstream would look wrong, which is why this is a test rather than
    /// something a picture catches.
    #[test]
    fn the_mark_sheet_is_published_when_it_moves_and_not_otherwise() {
        let ctx = egui::Context::default();
        let state = crate::tests::probe::fresh();
        let mirror = &state.instruments.font_atlas;

        // Nothing packed: there is no sheet to publish, on any frame.
        assert!(marks_if_changed(&ctx, mirror).is_none(), "an empty sheet publishes nothing");

        let sheet = mark_sheet(&ctx);
        sheet.lock().expect("fresh").patch(mark_at(MarkKind::Plus, 11.0), 0);
        assert!(marks_if_changed(&ctx, mirror).is_some(), "a first mark has to reach the renderer");
        assert!(marks_if_changed(&ctx, mirror).is_none(), "nothing has moved");

        // A mark already packed is not a change...
        sheet.lock().expect("held").patch(mark_at(MarkKind::Plus, 11.0), 0);
        assert!(marks_if_changed(&ctx, mirror).is_none(), "the same mark again is the same sheet");
        // ...and a new one is.
        sheet.lock().expect("held").patch(mark_at(MarkKind::Sharp, 11.0), 0);
        assert!(marks_if_changed(&ctx, mirror).is_some(), "a new mark has to reach the renderer");

        // The lattice keeps a mirror of its own, and it holds none of this:
        // each mirror answers for ONE renderer's texture, and a sheet
        // published to the text callback is not in the lattice's copy.
        assert!(
            marks_if_changed(&ctx, &state.instruments.lattice_atlas).is_some(),
            "a second renderer must be shown the sheet too, not told it already has it",
        );
    }

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
                let galley = ui.painter().layout_no_wrap(
                    text.to_owned(),
                    font.clone(),
                    egui::Color32::WHITE,
                );
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
    ///
    /// Swept well past the two surfaces that exist, because the analyzer's is
    /// the one id here that is a FUNCTION and a hand-written list of the
    /// surfaces alive today cannot fail on the thing that makes it one: a
    /// constant standing inside the run it hands out collides with nothing
    /// until a surface is added, and then collides silently. Sweeping is what
    /// holds the constants clear of that run rather than clear of `{0, 1}`.
    const SURFACES: usize = 8;
    #[test]
    fn every_batch_drawn_in_a_frame_has_an_id_of_its_own() {
        let ids: Vec<u64> = [LATTICE_LEARN, SPIRAL_NAMES]
            .into_iter()
            .chain((0..SURFACES).map(spectral_labels))
            .collect();
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
    /// A lattice that drew no names hands over no atlas.
    ///
    /// [`TextBatch::flush`] returns early on an empty batch, and
    /// [`TextBatch::lattice_labels`] has to do the same on its own —
    /// `atlas_if_changed` cannot cover for it. `seen` is empty until the
    /// first lattice glyph is handed over, so `resized` is true, an empty
    /// `drawn` finds nothing fresh but takes that arm anyway, and every call
    /// publishes: a whole `ColorImage` cloned out of egui and a new key,
    /// which the renderer answers with a full-atlas `write_texture` and a
    /// rebuilt glyph bind group. Once per frame, per lattice pane, for as
    /// long as it lasts.
    ///
    /// It lasts. `show_labels` gates `draw_node_labels` and not this call, so
    /// switching "Note names" off runs it for as long as the editor is open;
    /// with names on it is the resting state from the editor opening until
    /// the first note or hover, and it comes back whenever the mirror is
    /// cleared with the lattice idle — a drag between the Retina display and
    /// an external monitor, or the window closing and reopening.
    #[test]
    fn a_lattice_that_drew_no_names_hands_over_no_atlas() {
        let ctx = egui::Context::default();
        let state = crate::tests::probe::fresh();
        let scene = harmonigraph_scene::derive_scene(
            &state.tracker,
            &state.tuning,
            &state.view,
            &state.view.reach(),
            &state.frame_params,
            state.camera,
            None,
            0.0,
        );
        let mut published = 0usize;
        // Several frames: the first publication is the one that would seed
        // `seen` and quiet the rest, and with nothing drawn there is none.
        for _ in 0..3 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let mut batch = TextBatch::default();
                let labels = batch.lattice_labels(
                    ui.painter(),
                    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0)),
                    &scene,
                    &state,
                );
                published += usize::from(labels.atlas.is_some());
            });
        }
        assert_eq!(published, 0, "an empty batch publishes no atlas, on any frame");
    }

    #[test]
    fn a_glyph_at_a_new_texel_refreshes_the_mirror() {
        let ctx = egui::Context::default();
        let font = egui::FontId::monospace(13.0);
        for _ in 0..2 {
            batch_keys(&ctx, &font, "Ai");
            batch_keys(&ctx, &font, "iA");
        }
        let settled = ctx.fonts(|fonts| fonts.font_image_size());

        let state = crate::tests::probe::fresh();
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
        let mut state = crate::tests::probe::fresh();

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
        let (base, ppp) = (15.0, 2.0);
        let ctx = crate::tests::probe::themed_at(ppp);

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
    /// camera zoomed fully in with the Name size bar at its top asks for type half
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
        let state = crate::tests::probe::fresh();
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
