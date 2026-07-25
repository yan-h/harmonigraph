//! Haloed label text: collected as glyphs here, drawn by
//! [`lattice_render::text`].
//!
//! Text on either picture pane needs to be lifted off what it lands on —
//! note names over lit nodes, pitch labels over the spectrogram — and the
//! rim that does it used to be the whole label stamped around two rings,
//! twenty more copies of every glyph. That was most of the geometry in a
//! busy frame, so labels were a budget: every new one cost twenty-one draws
//! of its own text.
//!
//! Now a piece of text becomes one quad per glyph and the rim is computed
//! per pixel from the same offsets (see `lattice_render::text` for why the
//! two are the same arithmetic). What a label costs stopped depending on
//! its rim, which is what makes labels something to place where they help
//! rather than something to ration.
//!
//! egui still lays the text out. A [`TextBatch`] collects the glyphs of
//! however many pieces of text a pane draws, and hands them over in one
//! callback when the pane flushes it — flushing where the pane would
//! otherwise draw something on top, so the paint order the panes had is the
//! paint order they keep.

use lattice_render::{GlyphInstance, TextRing};

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

/// Which batch a flush belongs to. Unique per batch drawn in one frame,
/// since each keeps its own instance buffer: the two picture panes, their
/// Render-preview copies, and the analyzer's readout, which flushes
/// separately because the divider is drawn between it and the axis labels.
pub(crate) const LATTICE_LABELS: u64 = 0;
pub(crate) const LATTICE_PREVIEW_LABELS: u64 = 1;
/// The analyzer's, one pair per surface (docked, then the preview).
pub(crate) fn spectral_labels(surface: usize) -> u64 {
    2 + surface as u64 * 2
}
pub(crate) fn spectral_readout(surface: usize) -> u64 {
    3 + surface as u64 * 2
}

/// The glyphs a pane has drawn so far, waiting to be handed to the GPU.
#[derive(Default)]
pub(crate) struct TextBatch {
    glyphs: Vec<GlyphInstance>,
    /// Every (font size, character) this batch drew — the probe that decides
    /// whether egui has rasterized something our mirror of its atlas has not
    /// seen. See [`AtlasMirror`].
    drawn: Vec<(u32, char)>,
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
        let ppp = painter.ctx().pixels_per_point();
        let round = |v: f32| (v * ppp).round() / ppp;
        // egui rounds a galley onto a whole physical pixel before drawing it
        // (`round_text_to_pixels`), and its glyphs are already snapped
        // relative to that. Both have to happen here too, or the text lands
        // a fraction of a pixel off what it used to and every glyph softens.
        let pos = align.anchor_size(anchor, galley.size()).min;
        let pos = egui::pos2(round(pos.x), round(pos.y));

        for row in &galley.rows {
            for glyph in &row.glyphs {
                if glyph.uv_rect.is_nothing() {
                    continue;
                }
                let left_top = glyph.pos + glyph.uv_rect.offset;
                let min = pos
                    + row.pos.to_vec2()
                    + egui::vec2(round(left_top.x), round(left_top.y));
                self.drawn.push((font_size_bits, glyph.chr));
                self.glyphs.push(GlyphInstance {
                    rect: [min.x, min.y, glyph.uv_rect.size.x, glyph.uv_rect.size.y],
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
                let galley = egui::Rect::from_min_size(pos, galley.size());
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
        let atlas = atlas_if_changed(painter.ctx(), state, std::mem::take(&mut self.drawn));
        let rings = RINGS.map(|(radius, alpha, samples)| {
            let ppp = painter.ctx().pixels_per_point();
            TextRing { radius: (radius * ppp).round().max(1.0) / ppp, alpha, samples }
        });
        painter.add(lattice_render::text_paint_callback(
            rect,
            std::mem::take(&mut self.glyphs),
            rings,
            atlas,
            state.target_format,
            pane_id,
        ));
    }
}

/// What the callback's copy of egui's font atlas currently holds.
///
/// The callback cannot bind egui's own font texture — `CallbackResources`
/// holds what WE put there — so the atlas is mirrored, and something has to
/// say when the mirror is stale. egui's delta channel is not ours to take
/// (draining it would starve egui's own renderer and the rest of the UI's
/// text would stop updating), and the atlas exposes no version, so staleness
/// is inferred from the one thing that is knowable: a glyph egui has never
/// been asked for cannot be in the atlas yet, so the first time a
/// (size, character) pair is drawn, the mirror is refreshed.
///
/// That set stops growing almost immediately — an editor draws the same
/// digits and note names over and over — so in a running session the atlas
/// is copied a handful of times and then never again.
#[derive(Default)]
pub(crate) struct AtlasMirror {
    seen: std::collections::HashSet<(u32, char)>,
    /// The atlas size the mirror was taken at; a resize moves every glyph.
    size: [usize; 2],
    /// The scale factor it was taken at, as bits — the third way every glyph
    /// moves, and the one neither of the others can see.
    ///
    /// A pair is a POINT size and a character, but egui rasterizes at PHYSICAL
    /// pixels, so the same pair is a different image at a different scale.
    /// Changing it re-rasterizes the whole atlas and hands out new UVs while the
    /// dimensions stay put — measured: at 1x and 2x the atlas is `[2048, 32]`
    /// both times, and 'A' sits at texels `[90, 0]..[98, 8]` against
    /// `[111, 0]..[127, 17]`. So `size` does not move and `seen` reports nothing
    /// new, and without this the GPU would keep the old pixels while every
    /// glyph indexes the new layout — labels sampling whatever now lies at the
    /// coordinates they remember. Dragging the window between a Retina display
    /// and an external monitor is the ordinary way to do it.
    ppp: u32,
    /// Bumped on every refresh; the callback compares it against what it
    /// last uploaded.
    key: u64,
}

/// egui's font atlas, on the frames the mirror needs it, and `None` on the
/// rest — which is nearly all of them.
fn atlas_if_changed(
    ctx: &egui::Context,
    state: &crate::SharedState,
    drawn: Vec<(u32, char)>,
) -> Option<lattice_render::FontAtlas> {
    let mut mirror = state.font_atlas.lock().expect("the label mirror is never held across a panic");
    // A resize (or the overflow that clears the atlas and starts over)
    // rearranges everything, so it counts as a change on its own.
    let size = ctx.fonts(|fonts| fonts.font_image_size());
    // So does a scale change, which repacks the atlas without resizing it — see
    // [`AtlasMirror::ppp`]. The pairs already seen were rasterized at the old
    // scale and say nothing about the new one, so they go with it: every glyph
    // the panes draw counts as unseen again, which is also what refills `seen`.
    let ppp = ctx.pixels_per_point().to_bits();
    let rescaled = ppp != mirror.ppp;
    if rescaled {
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
    Some(lattice_render::FontAtlas {
        image: std::sync::Arc::new(ctx.fonts(|fonts| fonts.image())),
        key: mirror.key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lay a string out at `ppp` and return the atlas dimensions, the first
    /// glyph's texels, and the (size, char) pairs a pane would report drawing
    /// it — everything [`atlas_if_changed`] has to decide on.
    fn draw_at(
        ctx: &egui::Context,
        font: &egui::FontId,
        ppp: f32,
    ) -> ([usize; 2], [u16; 2], Vec<(u32, char)>) {
        ctx.set_pixels_per_point(ppp);
        let mut probe = None;
        // Twice: the first pass asks for glyphs the atlas may not hold yet, and
        // the rasterization it triggers lands for the next one.
        for _ in 0..2 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let galley =
                    ui.painter().layout_no_wrap("Ag1".to_owned(), font.clone(), egui::Color32::WHITE);
                let glyphs = &galley.rows[0].glyphs;
                probe = Some((
                    glyphs[0].uv_rect.min,
                    glyphs.iter().map(|g| (font.size.to_bits(), g.chr)).collect::<Vec<_>>(),
                ));
            });
        }
        let (uv, drawn) = probe.expect("the closure runs");
        (ctx.fonts(|fonts| fonts.font_image_size()), uv, drawn)
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
        let state = crate::SharedState::new(crate::TextureFormat::Bgra8Unorm);
        let ctx = egui::Context::default();
        let font = egui::FontId::proportional(12.0);

        let (size_1x, uv_1x, drawn_1x) = draw_at(&ctx, &font, 1.0);
        assert!(atlas_if_changed(&ctx, &state, drawn_1x.clone()).is_some(), "the first mirror");
        assert!(
            atlas_if_changed(&ctx, &state, drawn_1x.clone()).is_none(),
            "an unchanged frame must not re-upload the atlas — that is the point of the mirror"
        );

        let (size_2x, uv_2x, drawn_2x) = draw_at(&ctx, &font, 2.0);
        assert_eq!(size_2x, size_1x, "the atlas is repacked at the SAME dimensions");
        assert_ne!(uv_2x, uv_1x, "...but the glyph has moved inside it");
        assert_eq!(drawn_2x, drawn_1x, "...and the pane reports the same pairs it always did");

        assert!(
            atlas_if_changed(&ctx, &state, drawn_2x).is_some(),
            "a scale change must refresh the mirror: the UVs moved, so the pixels must follow"
        );
    }
}
