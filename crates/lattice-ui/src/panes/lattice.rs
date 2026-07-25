//! The 3D lattice view pane: orbit camera on drag, zoom on scroll, node
//! labels, and the tuning-learn overlay.

use super::{display_note_name, learn_pulse};
use crate::{theme, SharedState};
use egui::Sense;
use lattice_render::lattice_paint_callback;
use lattice_scene::{derive_scene, Camera, Projection, SevensLabel, TrailMark};

/// The 3D lattice view: orbit camera on drag, zoom on scroll, pick on hover.
pub(crate) fn lattice_pane(ui: &mut egui::Ui, state: &mut SharedState, now: f64) {
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }

    // Camera input: plain drag orbits; shift-drag or middle-drag pans
    // (speeds and clamps live on Camera itself). Cabinet is fixed-viewpoint
    // — orbiting is meaningless there, so plain drags pan too.
    let shift = ui.input(|i| i.modifiers.shift);
    let panning = response.dragged_by(egui::PointerButton::Middle)
        || (response.dragged_by(egui::PointerButton::Primary)
            && (shift || state.camera.projection == Projection::Cabinet));
    if panning {
        let delta = response.drag_delta();
        state.camera.pan(glam::Vec2::new(delta.x, delta.y));
    } else if response.dragged_by(egui::PointerButton::Primary) {
        let delta = response.drag_delta();
        state.camera.orbit(glam::Vec2::new(delta.x, delta.y));
    }
    // Zoom when the pointer is over the view. Gate on contains_pointer (pure
    // geometry) rather than hovered(): the lattice sits under a wgpu paint
    // callback, and hovered() can be suppressed by the callback layer or a
    // transient focus/interaction elsewhere, silently killing the scroll. Honor
    // BOTH the scroll delta (mouse wheel) and egui's zoom_delta — trackpad
    // pinches and modifier+scroll arrive as a zoom factor, not a scroll delta,
    // and those would otherwise do nothing over the lattice.
    if response.contains_pointer() {
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        if scroll != 0.0 {
            state.camera.zoom(scroll);
        }
        if zoom != 1.0 {
            state.camera.zoom_by(zoom);
        }
    }
    if response.double_clicked() {
        // Reset orbit/zoom, but keep the chosen projection: that's a view
        // preference, not a navigation state.
        state.camera = Camera {
            projection: state.camera.projection,
            ..Default::default()
        };
    }

    let mut scene = derive_scene(
        &state.tracker,
        &state.tuning,
        &state.view,
        &state.frame_params,
        state.camera,
        state.hovered,
        now,
    );
    // The ground the sevens knockout clears to. Only the shell knows what
    // this pass is composited over -- the dock's tab body here, the render
    // layout's own background offline -- so it is carried on SharedState
    // rather than assumed by the scene.
    scene.background = state.background;

    // Picking updates the *shared* hover state, one frame behind the scene
    // it was derived from (imperceptible, standard for immediate mode).
    if let Some(pointer) = response.hover_pos() {
        state.hovered = scene.pick(
            glam::Vec2::new(rect.width(), rect.height()),
            glam::Vec2::new(pointer.x - rect.min.x, pointer.y - rect.min.y),
            24.0,
        );
    } else if !response.dragged() {
        state.hovered = None;
    }

    ui.painter()
        .add(lattice_paint_callback(rect, &scene, state.target_format, 0, Some(state.lattice_stats.clone())));

    // One batch for everything this pane writes, flushed at the end: nothing
    // is drawn over the text, so collecting it keeps the order it had.
    let mut batch = crate::text::TextBatch::default();
    if state.learn_active {
        draw_learn_overlay(&mut batch, ui, rect, now);
    }
    if state.view.show_labels {
        draw_node_labels(ui, rect, &scene, &state.view, 1.0, &mut batch);
    }
    batch.flush(ui.painter(), rect, state, crate::text::LATTICE_LABELS);
}

/// Learn mode is armed: show it ON the lattice too, so the mode is obvious
/// even when the Tuning tab (and its Learn toggle) is hidden.
fn draw_learn_overlay(batch: &mut crate::text::TextBatch, ui: &egui::Ui, rect: egui::Rect, now: f64) {
    let color = theme::armed().gamma_multiply(learn_pulse(now));
    let painter = ui.painter_at(rect);
    painter.rect_stroke(
        rect.shrink(1.5),
        0,
        egui::Stroke::new(2.0, color),
        egui::StrokeKind::Inside,
    );
    batch.text(
        &painter,
        rect.left_top() + egui::vec2(10.0, 8.0),
        egui::Align2::LEFT_TOP,
        "LEARN".to_string(),
        egui::FontId::monospace(12.0),
        color,
        theme::well().gamma_multiply(learn_pulse(now)),
    );
}

/// How readable a label on a visited node is next to a sounding one. Well
/// below full so the notes actually playing still read first -- but flat,
/// not scaled by the trail level, because the whole point of keeping the
/// text is that it can be read: a name at 5% alpha says nothing.
const TRAIL_LABEL_STRENGTH: f32 = 0.5;

/// How readable one node's label is, 0..1. `trailed` is whether this node
/// carries a kept name already, `keeps_names` whether the view keeps them at
/// all (see [`draw_node_labels`]).
///
/// A sounding label rides its note's envelope straight down to nothing. The
/// one exception is a name this view is about to KEEP, which settles on
/// `TRAIL_LABEL_STRENGTH` instead of fading out: `node.trail` is the recorded
/// memory, and it is only written the frame the release finishes, so during
/// the fade there is nothing to settle onto and the level has to be reserved
/// ahead of the record. Without that reserve the name eases to zero and the
/// trail pops it back a frame later — the "flash back in" that made one label
/// read as two.
///
/// Reserved only where a trail can actually land, which is the home sheet:
/// off-sheet nodes are deliberately never marked (a lone memory floating out
/// in the sevens dimension reads as noise — see `lattice_scene::trail`), so
/// reserving there held the label at half brightness through the whole
/// release and then dropped it to nothing at prune. Fading to a level and
/// vanishing from it is exactly what a visibility floor looks like, and this
/// was the last one left.
///
/// One corner is left, and it needs a number this layer doesn't have: a trail
/// Memory *shorter than the Fade* forgets the pitch before the release ends,
/// so the reserve holds a home-sheet name that then has nothing to hand it to.
/// Memory is 0 — never forget — by default, and the fix needs the fade time
/// down here to predict the level the record will land on.
fn label_strength(node: &lattice_scene::NodeInstance, trailed: bool, keeps_names: bool) -> f32 {
    if node.hovered {
        return 1.0;
    }
    let recorded = if trailed { TRAIL_LABEL_STRENGTH * node.trail } else { 0.0 };
    let reserved = if keeps_names && node.on_home && node.activation > 0.0 {
        TRAIL_LABEL_STRENGTH
    } else {
        0.0
    };
    node.activation.max(recorded).max(reserved)
}

/// Labels on hovered, sounding, and -- with the trail's "Keep note names"
/// on -- already-visited nodes, drawn as egui text over the 3D view
/// (projected with the same camera as the nodes): the note name centered on
/// the node, optionally its pitch class in cents just below.
pub(crate) fn draw_node_labels(
    ui: &egui::Ui,
    rect: egui::Rect,
    scene: &lattice_scene::Scene,
    view: &lattice_scene::ViewConfig,
    scale: f32,
    batch: &mut crate::text::TextBatch,
) {
    let projector = scene.projector(glam::Vec2::new(rect.width(), rect.height()));
    // "Keep note names" retains a name only while the trail marks that
    // populate `node.trail` are on; with the marks Off the field never fills,
    // so a fading name has nothing to settle onto and should ease all the way
    // out (the pre-existing behavior).
    let keeps_names = view.trail_labels && view.trail_mark != TrailMark::Off;
    for node in &scene.nodes {
        let trailed = view.trail_labels && node.trail > 0.0;
        // `is_visible` re-checks what `Scene::pick` already enforces,
        // because hover also arrives from the Spectral pane
        // (`nearest_visible_node`), which can land on an off-sheet node.
        // Either way a label only belongs on a node you can actually see.
        if !(node.hovered || node.activation > 0.0 || trailed) || !node.is_visible() {
            continue;
        }
        let Some(p) = projector.project(node.world_pos) else {
            continue;
        };
        let strength = label_strength(node, trailed, keeps_names);
        let center = egui::pos2(rect.min.x + p.x, rect.min.y + p.y);
        // Off the pane: nothing to draw. `project` only rejects what is
        // behind the camera, so a node off to the side still lands at a
        // screen position — outside the pane, where the pane's own clip
        // throws it away. It was being thrown away AFTER laying the text
        // out and stamping it 33 times per piece, which is most of the
        // label work in the frame the further in the camera is: zoomed
        // right in, almost every node is off the pane.
        if !rect.expand(LABEL_REACH * scale).contains(center) {
            continue;
        }
        let outline = theme::well().gamma_multiply(strength);
        // Off-sheet nodes draw at their own size (ViewConfig::sevens_size),
        // and their text goes with them — a full-size label on a half-size
        // node reads as a label with a node attached. Floored so the
        // smallest sheet is still legible rather than merely present.
        let scale = scale * node.scale.max(0.6);
        // What an off-sheet node says, and whether it says anything: its
        // NAME is shared with the node two fifths down and is the same
        // string on every sheet, so it is the one thing not worth the
        // biggest glyph on the node. See SevensLabel.
        let sevens = if node.on_home { SevensLabel::Name } else { view.sevens_label };
        let name_bottom = match sevens {
            SevensLabel::None => 0.0,
            SevensLabel::Cents => draw_plain_name(
                batch,
                ui.painter(),
                center,
                &format!("{:.0}", node.cents),
                theme::text().gamma_multiply(strength),
                outline,
                scale,
            ),
            SevensLabel::Name => {
                let name = display_note_name(node.lattice_pos, view.meantone);
                draw_stacked_name(
                    batch,
                    ui.painter(),
                    center,
                    name,
                    theme::text().gamma_multiply(strength),
                    outline,
                    scale,
                    view.mark_weight,
                )
            }
        };
        // The cents line is the home sheet's business: off the home sheet
        // the scheme above has already chosen what number (if any) belongs
        // under the name, and stacking a second one would bury the node.
        if view.show_cents && (node.on_home || sevens == SevensLabel::Name) {
            let text = format!("{:.2}", node.cents);
            let font = egui::FontId::monospace(CENTS_SIZE * scale);
            // Hang the readout off the name's INK, not its galley box: a
            // monospace box carries enough leading above and below the glyphs
            // that box-to-box spacing left the two floating far apart.
            let top = painter_ink(ui.painter(), &text, &font).min.y;
            batch.text(
                ui.painter(),
                center + egui::vec2(0.0, name_bottom + CENTS_GAP * scale - top),
                egui::Align2::CENTER_TOP,
                text,
                font,
                theme::text_dim().gamma_multiply(strength),
                outline,
            );
        }
    }
}

/// The note name's letter, the size the label reads at.
pub(crate) const NAME_SIZE: f32 = 15.0;
/// The cents readout under it: subordinate to the name, so smaller, and
/// tucked right beneath it rather than floating free.
pub(crate) const CENTS_SIZE: f32 = 8.0;
/// How far a label can reach from the node it belongs to, in points at
/// scale 1 — the name, its marks, the gap and the cents line under it, with
/// room to spare. Only used to decide that a label is too far off the pane
/// to be worth laying out, so it errs generous: too small silently clips a
/// label at the edge, too large only costs the work this saves.
pub(crate) const LABEL_REACH: f32 = 48.0;

/// Air between the bottom of the name's glyphs and the top of the cents
/// readout's. Real pixels of gap, since both ends are measured as ink: the
/// two are one label, sitting together without crowding.
pub(crate) const CENTS_GAP: f32 = 3.0;
/// Accidental and comma marks, relative to the letter. Small enough that the
/// two of them stacked still fit inside the letter's own height -- the pair
/// is an annotation on the name, and a label that grows taller than its
/// letter reads as two lines rather than one name.
const MARK_SCALE: f32 = 0.55;
/// The size the marks are actually laid out at.
pub(crate) const MARK_SIZE: f32 = NAME_SIZE * MARK_SCALE;

/// Iosevka Fixed's advance, as a fraction of the em: every cell is half an
/// em wide. A drawn mark claims exactly this, so it sits in the same column
/// grid as the typeset accidental above it.
const MARK_ADVANCE: f32 = 0.5;
/// The ink width Iosevka gives `+` and `-` within that cell (372/1000 em).
/// Matching it is what keeps a drawn sign from reading as a different size
/// of mark than the `♯` stacked over it.
const MARK_INK_W: f32 = 0.372;
/// And the height of `+`'s upright (386/1000 em).
const PLUS_INK_H: f32 = 0.386;
/// Air between the accidental/comma column and the septimal mark, as a
/// fraction of the mark's font size. Small: enough that the mark is not
/// read as another row of the stack it sits beside, not so much that it
/// floats free of the name it belongs to.
const SEPTIMAL_GAP: f32 = 0.22;
/// How much larger the septimal shape draws than that `+` box. A triangle
/// covers half its bounding box, so drawn to the same box it reads as the
/// lighter mark of the two; this is the size at which the pair looks like
/// one system rather than a mark and a smaller mark.
const SEPTIMAL_BULK: f32 = 1.25;
/// One piece of a mark, in the mark bitmap's own pixel space.
///
/// These are never drawn to the screen. They describe a shape that gets
/// rasterized ONCE into a coverage bitmap, so pieces may abut or overlap
/// freely -- coverage is a max over pieces, not a composite of them, and
/// none of the artifacts of drawing them separately can arise.
enum MarkPiece {
    Bar(egui::Rect),
    Line(Vec<egui::Pos2>, f32),
}

impl MarkPiece {
    /// Whether this piece covers a point, in bitmap pixel space.
    fn covers(&self, p: egui::Pos2) -> bool {
        match self {
            MarkPiece::Bar(rect) => rect.contains(p),
            MarkPiece::Line(points, width) => points
                .windows(2)
                .any(|seg| point_to_segment(p, seg[0], seg[1]) <= width / 2.0),
        }
    }
}

/// Distance from `p` to the segment `a`-`b`.
fn point_to_segment(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    let t = if len2 <= f32::EPSILON { 0.0 } else { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) };
    ((p - a) - ab * t).length()
}

/// Which mark, at what size in physical pixels -- the identity of one
/// rasterized bitmap, and its cache key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct MarkKey {
    kind: MarkKind,
    /// The mark's font size in whole physical pixels. Whole, because a
    /// bitmap has to be rasterized at SOME integer size, exactly as a glyph
    /// is; the on-screen size then steps by a pixel as the camera zooms,
    /// which is what a glyph atlas does too.
    size_px: u32,
    /// Stroke weight in physical pixels x16, so the cache key stays integral
    /// without quantizing the weight to something visible.
    weight_16: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum MarkKind {
    Minus,
    Plus,
    /// The septimal mark -- a chevron -- and whether it points up.
    ///
    /// A chevron out of four designs built to be compared: it is the
    /// lightest of them, and the only one whose weight reads as belonging
    /// with the `+`/`-` rather than as a second, heavier kind of mark. The
    /// others (filled and hollow triangles, a stemmed arrow) are deleted
    /// rather than kept switchable -- a comparison that has been made is
    /// not a setting.
    Septimal(bool),
}

/// A mark's geometry in its bitmap's pixel space, with the bitmap size.
///
/// Built once per (design, size, weight) on a canonical grid at the origin,
/// which is what makes a mark's proportions IDENTICAL every time it is
/// drawn. Building it in screen space instead let the two arms of a `+`
/// land on different subpixel offsets and rasterize to different lengths --
/// the lopsidedness that survived every attempt to fix it by snapping,
/// because snapping the pieces is what made them disagree.
fn mark_geometry(key: MarkKey) -> (Vec<MarkPiece>, [usize; 2]) {
    let size = key.size_px as f32;
    let thick = key.weight_16 as f32 / 16.0;
    let (w, h) = match key.kind {
        MarkKind::Septimal(_) => (
            MARK_INK_W * size * SEPTIMAL_BULK,
            PLUS_INK_H * size * SEPTIMAL_BULK,
        ),
        _ => (MARK_INK_W * size, PLUS_INK_H * size),
    };
    // The bitmap is a whole number of pixels and the shape is centered in
    // it, so a design and its mirror rasterize to mirror images.
    let (bw, bh) = (w.ceil().max(1.0), h.ceil().max(1.0));
    let c = egui::pos2(bw / 2.0, bh / 2.0);
    let (hw, hh) = (w / 2.0, h / 2.0);
    let pieces = match key.kind {
        MarkKind::Minus => vec![MarkPiece::Bar(egui::Rect::from_center_size(
            c,
            egui::vec2(w, thick),
        ))],
        MarkKind::Plus => vec![
            MarkPiece::Bar(egui::Rect::from_center_size(c, egui::vec2(w, thick))),
            MarkPiece::Bar(egui::Rect::from_center_size(c, egui::vec2(thick, h))),
        ],
        MarkKind::Septimal(up) => {
            // Point-toward-the-tip: -1 draws upward, +1 downward, so the
            // shape is written once and mirrored by arithmetic.
            let dir = if up { -1.0 } else { 1.0 };
            let tip = egui::pos2(c.x, c.y + dir * hh);
            let base_l = egui::pos2(c.x - hw, c.y - dir * hh);
            let base_r = egui::pos2(c.x + hw, c.y - dir * hh);
            vec![MarkPiece::Line(vec![base_l, tip, base_r], thick)]
        }
    };
    (pieces, [bw as usize, bh as usize])
}

/// Supersampling grid used to turn a mark's outline into coverage. 4x4 is
/// finer than the antialiasing a shape would have got from the tessellator
/// and costs nothing: a mark bitmap is a dozen pixels square and is built
/// once per size, not once per frame.
const MARK_SUPERSAMPLE: usize = 4;

/// Rasterize a mark to an alpha coverage image -- the same thing a font
/// rasterizer hands the atlas for a glyph.
fn rasterize_mark(key: MarkKey) -> egui::ColorImage {
    let (pieces, [w, h]) = mark_geometry(key);
    let n = MARK_SUPERSAMPLE;
    let step = 1.0 / n as f32;
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let mut hits = 0;
            for sy in 0..n {
                for sx in 0..n {
                    let p = egui::pos2(
                        x as f32 + (sx as f32 + 0.5) * step,
                        y as f32 + (sy as f32 + 0.5) * step,
                    );
                    // Coverage is a UNION over pieces, never a sum: two
                    // pieces meeting cannot darken or brighten their join.
                    if pieces.iter().any(|piece| piece.covers(p)) {
                        hits += 1;
                    }
                }
            }
            let a = (255 * hits / (n * n)) as u8;
            pixels.push(egui::Color32::from_white_alpha(a));
        }
    }
    egui::ColorImage { size: [w, h], pixels, source_size: egui::vec2(w as f32, h as f32) }
}

/// How many mark bitmaps to keep before starting over. Zooming walks through
/// sizes, and each one is its own bitmap; this is a ceiling on the churn,
/// not a working-set estimate.
const MARK_CACHE_LIMIT: usize = 96;

/// The texture for one mark, rasterized on first use and kept in egui's own
/// per-frame data store.
fn mark_texture(ctx: &egui::Context, key: MarkKey) -> egui::TextureHandle {
    type Cache = std::collections::HashMap<MarkKey, egui::TextureHandle>;
    let cached = ctx.data_mut(|d| d.get_temp::<std::sync::Arc<Cache>>(egui::Id::NULL));
    if let Some(hit) = cached.as_ref().and_then(|c| c.get(&key)) {
        return hit.clone();
    }
    // LINEAR, because a mark is placed at a subpixel position and resampled
    // exactly as a glyph is. NEAREST would put the pixel grid back.
    let handle = ctx.load_texture(
        format!("mark{key:?}", key = key.size_px),
        rasterize_mark(key),
        egui::TextureOptions::LINEAR,
    );
    let mut next = cached.map(|c| (*c).clone()).unwrap_or_default();
    if next.len() >= MARK_CACHE_LIMIT {
        next.clear();
    }
    next.insert(key, handle.clone());
    ctx.data_mut(|d| d.insert_temp(egui::Id::NULL, std::sync::Arc::new(next)));
    handle
}

/// Paint a mark with the rim the glyphs beside it carry -- the same rim, by
/// the same arithmetic, over the same kind of thing.
///
/// The mark is ONE textured quad of coverage, which is what a glyph is, so
/// every difference that came of drawing it as separate shapes is gone by
/// construction: no seam between pieces to feather twice, no join to
/// composite twice, no arm to rasterize at its own subpixel offset. The
/// quad lands wherever the label lands, and bilinear sampling resolves it
/// the way it resolves a glyph.
///
/// `lattice_render`'s text shader states the identity the rim rests on: a
/// label's rim IS the shape stamped around two rings, and the shader only
/// moved that loop into the fragment stage because 20 copies of every glyph
/// was most of the geometry in a busy frame. A mark is one quad, so the
/// loop is affordable here -- same radii, same sample counts, same
/// per-stamp alpha, same `angle = 2*PI*i/samples`.
///
/// Rim first, then the fill, which is the order stamping had and the order
/// the shader kept.
fn paint_mark(
    painter: &egui::Painter,
    ppp: f32,
    key: MarkKey,
    center: egui::Pos2,
    color: egui::Color32,
    outline: egui::Color32,
) {
    let texture = mark_texture(painter.ctx(), key);
    let [w, h] = texture.size();
    // One texel per physical pixel: the bitmap was rasterized at this size,
    // so it is placed at it and never scaled.
    let rect = egui::Rect::from_center_size(
        center,
        egui::vec2(w as f32 / ppp, h as f32 / ppp),
    );
    let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
    for (radius, alpha, samples) in crate::text::RINGS {
        if samples == 0 {
            continue;
        }
        let radius = crate::text::ring_radius(radius, ppp);
        let paint = outline.gamma_multiply(alpha);
        for i in 0..samples {
            let angle = std::f32::consts::TAU * i as f32 / samples as f32;
            let offset = egui::vec2(angle.cos(), angle.sin()) * radius;
            painter.image(texture.id(), rect.translate(offset), uv, paint);
        }
    }
    painter.image(texture.id(), rect, uv, color);
}

/// The key for one mark at the size a label is drawing at.
///
/// `size` is the mark font size in points; the bitmap is rasterized in
/// physical pixels, so the size crosses into pixels here and is rounded --
/// a bitmap has an integer size or it has none.
fn mark_key(kind: MarkKind, size: f32, weight: f32, ppp: f32) -> MarkKey {
    let size_px = (size * ppp).round().max(2.0);
    // Floored at a whole physical pixel: the whole reason these marks are
    // not type is that Iosevka's own bars are 70/1000 em, which is 0.58px
    // at MARK_SIZE, and a stroke thinner than a pixel spends all of its
    // contrast on partial coverage.
    let thick = (weight * size * ppp).max(1.0);
    MarkKey { kind, size_px: size_px as u32, weight_16: (thick * 16.0).round() as u32 }
}

/// A note name centered on `anchor`: the letter, then a column carrying its
/// accidental above its syntonic-comma sign, then a column for the septimal
/// mark (`♯` riding high like a superscript, `+` low like a subscript).
/// Every mark is counted rather than repeated (see [`lattice_core::NoteName`]),
/// so a name deep in the lattice -- or five modulations out along the sevens
/// axis -- stays a couple of characters wide instead of sprawling off its node.
///
/// The two comma signs are DRAWN and the accidental is typeset, which is not
/// an inconsistency but the point: `♯` and `♭` are real musical symbols with
/// 878 and 818 units of ink, and they survive the size. The comma signs are
/// bars, and Iosevka has no bar thicker than 70 units. See [`comma_sign`].
///
/// The septimal mark takes its direction twice over -- from the shape it is
/// drawn as, and from which end of the column it sits at. The second cue is
/// free (the column already offsets its marks) and it is the one that
/// survives when the node is small enough that the shape is four pixels.
///
/// Returns how far the lowest thing drawn reaches below `anchor.y` -- ink,
/// not boxes -- which is what the cents readout hangs off.
///
/// Monospace for in-lattice text: labels align across nodes and match the
/// technical feel of the readouts.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_stacked_name(
    batch: &mut crate::text::TextBatch,
    painter: &egui::Painter,
    anchor: egui::Pos2,
    name: lattice_core::NoteName,
    color: egui::Color32,
    outline: egui::Color32,
    scale: f32,
    mark_weight: f32,
) -> f32 {
    let name_font = egui::FontId::monospace(NAME_SIZE * scale);
    let mark_font = egui::FontId::monospace(MARK_SIZE * scale);
    let mark_size = MARK_SIZE * scale;
    let ppp = painter.ctx().pixels_per_point();
    let measure = |text: &str, font: &egui::FontId| {
        painter.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER).size()
    };
    // Each piece is drawn centered on its own line, so its ink reaches this
    // far below that line.
    let ink_below = |text: &str, font: &egui::FontId, size: egui::Vec2| {
        painter_ink(painter, text, font).max.y - size.y / 2.0
    };
    // Past one, a mark carries its count as a digit beside the sign. One is
    // common enough that the digit would be noise, so it stays bare.
    let count_text = |n: i32| if n.abs() > 1 { n.abs().to_string() } else { String::new() };

    let letter_text = name.letter.to_string();
    let letter = measure(&letter_text, &name_font);
    // Every mark sits on one line of the mark font, so they all rise by the
    // same amount -- including the drawn ones, which have no galley to ask.
    let line = measure("0", &mark_font);
    let rise = (letter.y - line.y) / 2.0;
    let cell = MARK_ADVANCE * mark_size;

    let accidental = name.accidental_mark();
    let syntonic = count_text(name.syntonic_commas);
    let septimal = count_text(name.septimal_commas);
    // A drawn sign claims one cell; its count follows in the same column.
    let signed_width =
        |count: &str, present: bool| if present { cell + measure(count, &mark_font).x } else { 0.0 };
    let column = measure(&accidental, &mark_font)
        .x
        .max(signed_width(&syntonic, name.syntonic_commas != 0));
    let septimal_column = signed_width(&septimal, name.septimal_commas != 0);
    // Air between the accidental column and the septimal mark, so the mark
    // reads as its own thing rather than as a third row of the stack.
    let gap = if name.septimal_commas != 0 { SEPTIMAL_GAP * mark_size } else { 0.0 };
    let left = anchor.x - (letter.x + column + gap + septimal_column) / 2.0;

    batch.text(
        painter,
        egui::pos2(left, anchor.y),
        egui::Align2::LEFT_CENTER,
        letter_text.clone(),
        name_font.clone(),
        color,
        outline,
    );
    let mut bottom = ink_below(&letter_text, &name_font, letter);

    // The accidental rides high, flush with the top of the letter.
    if !accidental.is_empty() {
        batch.text(
            painter,
            egui::pos2(left + letter.x, anchor.y - rise),
            egui::Align2::LEFT_CENTER,
            accidental.clone(),
            mark_font.clone(),
            color,
            outline,
        );
        bottom = bottom.max(-rise + ink_below(&accidental, &mark_font, line));
    }

    // Drawn sign, then its count: same column, same line, so the pair reads
    // as one mark rather than as a glyph with a number after it.
    let mut draw_signed = |x: f32,
                           direction: f32,
                           count: &str,
                           kind: MarkKind|
     -> f32 {
        let key = mark_key(kind, mark_size, mark_weight, ppp);
        let center = egui::pos2(x + cell / 2.0, anchor.y + direction * rise);
        paint_mark(painter, ppp, key, center, color, outline);
        if !count.is_empty() {
            batch.text(
                painter,
                egui::pos2(x + cell, anchor.y + direction * rise),
                egui::Align2::LEFT_CENTER,
                count.to_owned(),
                mark_font.clone(),
                color,
                outline,
            );
        }
        // Whichever reaches lower: the mark's own bitmap from its center, or
        // the count's digits from theirs.
        let ink = (mark_geometry(key).1[1] as f32 / ppp / 2.0)
            .max(if count.is_empty() { 0.0 } else { ink_below(count, &mark_font, line) });
        direction * rise + ink
    };

    if name.syntonic_commas != 0 {
        let kind =
            if name.syntonic_commas > 0 { MarkKind::Plus } else { MarkKind::Minus };
        bottom = bottom.max(draw_signed(left + letter.x, 1.0, &syntonic, kind));
    }
    if name.septimal_commas != 0 {
        // Centered on the letter's own line -- the seam between the
        // accidental riding above it and the comma sitting below -- rather
        // than in one slot or the other. It is not a third member of that
        // stack: it belongs to a different prime, and sitting across the
        // divide with a gap before it is what says so. The chevron carries
        // its own direction, so the slot is free to mean this instead.
        bottom = bottom.max(draw_signed(
            left + letter.x + column + gap,
            0.0,
            &septimal,
            MarkKind::Septimal(name.septimal_commas > 0),
        ));
    }
    bottom
}

/// One line of centered label text, measured like [`draw_stacked_name`] so
/// the two can be stacked against each other: returns how far its ink reaches
/// below `anchor.y`. Used for the label lines that are numbers rather than
/// note names — an off-sheet node's cents, and its comma (see
/// [`SevensLabel`](lattice_scene::SevensLabel)).
fn draw_plain_name(
    batch: &mut crate::text::TextBatch,
    painter: &egui::Painter,
    anchor: egui::Pos2,
    text: &str,
    color: egui::Color32,
    outline: egui::Color32,
    scale: f32,
) -> f32 {
    let font = egui::FontId::monospace(NAME_SIZE * scale);
    let size = painter
        .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
        .size();
    batch.text(
        painter,
        anchor,
        egui::Align2::CENTER_CENTER,
        text.to_owned(),
        font.clone(),
        color,
        outline,
    );
    painter_ink(painter, text, &font).max.y - size.y / 2.0
}

/// The box the glyphs of `text` actually cover, relative to the galley's own
/// top-left. Distinct from the galley's size, which pads the glyphs out to a
/// full line box; laying two pieces of text out edge to edge by their boxes
/// leaves a visible gap that neither piece's ink accounts for.
fn painter_ink(painter: &egui::Painter, text: &str, font: &egui::FontId) -> egui::Rect {
    painter
        .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
        .mesh_bounds
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_core::{NoteEvent, NoteEventKind};

    /// The alpha at one pixel of a rasterized mark.
    fn coverage(img: &egui::ColorImage, x: usize, y: usize) -> u8 {
        img.pixels[y * img.size[0] + x].a()
    }

    /// A `+` rasterizes to its own mirror, both ways.
    ///
    /// The lopsidedness -- one arm visibly longer than the other -- came of
    /// building the arms in SCREEN space, where each landed on its own
    /// subpixel offset and rasterized to its own length. Built on the
    /// bitmap's canonical grid, symmetry is structural.
    #[test]
    fn a_plus_rasterizes_to_its_own_mirror() {
        for size in [6.0_f32, 8.25, 13.0, 21.0] {
            let img = rasterize_mark(mark_key(MarkKind::Plus, size, 0.12, 2.0));
            let [w, h] = img.size;
            for y in 0..h {
                for x in 0..w {
                    let a = coverage(&img, x, y);
                    assert_eq!(a, coverage(&img, x, h - 1 - y), "top/bottom at {x},{y} @{size}");
                    assert_eq!(a, coverage(&img, w - 1 - x, y), "left/right at {x},{y} @{size}");
                }
            }
        }
    }

    /// The square where a `+`'s bar and upright cross is no brighter than
    /// the bar itself.
    ///
    /// A mark is rasterized as COVERAGE, and coverage is a union over the
    /// pieces -- so a join cannot be composited twice. Drawn as separate
    /// translucent shapes it could, and did: the middle of a fading `+` lit
    /// up against its own arms however carefully the pieces were made not to
    /// overlap, because the tessellator feathers every edge and two abutting
    /// edges feather over each other.
    #[test]
    fn a_pluss_join_is_no_brighter_than_its_arms() {
        let img = rasterize_mark(mark_key(MarkKind::Plus, 16.0, 0.12, 2.0));
        let [w, h] = img.size;
        let peak = img.pixels.iter().map(|p| p.a()).max().expect("a + has ink");
        assert_eq!(coverage(&img, w / 2, h / 2), peak, "the join is not the brightest point");
        // And it is not the ONLY point at that level: the arms reach it too,
        // so the join does not read as a spot.
        let at_peak = img.pixels.iter().filter(|p| p.a() == peak).count();
        assert!(at_peak > w.min(h), "only {at_peak} pixels reach peak coverage of {w}x{h}");
    }

    /// A mark's bitmap is the same whatever subpixel position its node
    /// projects to: the key rounds into whole physical pixels, and the
    /// bitmap is placed rather than rebuilt.
    #[test]
    fn a_mark_is_one_bitmap_wherever_it_lands() {
        // All within one rounding bucket: 8.25..8.37 points is 17 physical
        // pixels at 2x. Sizes that straddle a bucket edge SHOULD differ --
        // that is the bitmap stepping by a pixel as the camera zooms, the
        // same thing a glyph atlas does.
        let a = mark_key(MarkKind::Minus, 8.25, 0.12, 2.0);
        for size in [8.25_f32, 8.26, 8.30, 8.36] {
            assert_eq!(mark_key(MarkKind::Minus, size, 0.12, 2.0), a, "{size}");
        }
        // A minus is a single bar, so it too is its own mirror.
        let img = rasterize_mark(a);
        let [w, h] = img.size;
        for y in 0..h {
            for x in 0..w {
                assert_eq!(coverage(&img, x, y), coverage(&img, x, h - 1 - y));
            }
        }
    }

    /// The septimal mark's direction lives in the shape: up and down are
    /// each other's mirror, which is the whole of what tells them apart now
    /// that both sit on the same line.
    #[test]
    fn the_septimal_chevron_mirrors_with_its_direction() {
        let up = rasterize_mark(mark_key(MarkKind::Septimal(true), 12.0, 0.10, 2.0));
        let down = rasterize_mark(mark_key(MarkKind::Septimal(false), 12.0, 0.10, 2.0));
        assert_eq!(up.size, down.size);
        let [w, h] = up.size;
        for y in 0..h {
            for x in 0..w {
                assert_eq!(
                    coverage(&up, x, y),
                    coverage(&down, x, h - 1 - y),
                    "up and down disagree at {x},{y}"
                );
            }
        }
        // And it is NOT its own vertical mirror, or the two would be
        // indistinguishable and the mark would say nothing.
        assert!(
            (0..h).any(|y| (0..w).any(|x| coverage(&up, x, y) != coverage(&up, x, h - 1 - y))),
            "a chevron that mirrors onto itself carries no direction"
        );
    }

    /// The stroke floor is one PHYSICAL pixel, not one point -- a point is
    /// two pixels here, so flooring in points drew every small mark at twice
    /// the minimum and made the whole set look heavy.
    #[test]
    fn the_weight_floor_is_a_physical_pixel() {
        assert_eq!(mark_key(MarkKind::Minus, 4.0, 0.0001, 2.0).weight_16, 16);
        assert_eq!(mark_key(MarkKind::Minus, 4.0, 0.0001, 1.0).weight_16, 16);
        // Above the floor the weight is what decides it: 0.1 * 20 * 2 = 4px.
        assert_eq!(mark_key(MarkKind::Minus, 20.0, 0.1, 2.0).weight_16, 64);
    }

    /// Draw the labels for a chord, with the camera at `distance`, and
    /// report the pieces of text that were laid out.
    fn label_pieces(rect: egui::Rect, distance: f32) -> Vec<crate::text::TextPiece> {
        let mut state = SharedState::new(lattice_render::wgpu::TextureFormat::Bgra8Unorm);
        state.camera.distance = distance;
        // A chord spread across the lattice, so nodes land all over the pane
        // and (zoomed in) well outside it.
        for note in [55u8, 60, 62, 64, 67, 69, 71] {
            state.tracker.handle_event(NoteEvent {
                time: 0.0,
                channel: 0,
                note,
                kind: NoteEventKind::On { velocity: 1.0 },
            });
        }
        let scene = derive_scene(
            &state.tracker,
            &state.tuning,
            &state.view,
            &state.frame_params,
            state.camera,
            None,
            0.05,
        );
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 900.0));
        let mut batch = crate::text::TextBatch::default();
        let _ = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                let child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                draw_node_labels(&child, rect, &scene, &state.view, 1.0, &mut batch);
            },
        );
        batch.pieces().to_vec()
    }

    /// One quad per glyph, whatever the rim is doing.
    ///
    /// This is the whole point of drawing labels ourselves: stamping the rim
    /// as geometry multiplies a label by twenty-one, making every new label a
    /// cost decision. Here the rim is arithmetic in the fragment shader and
    /// a piece of text costs its own glyphs and nothing else.
    #[test]
    fn a_label_costs_one_quad_per_glyph() {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 100.0));
        let mut batch = crate::text::TextBatch::default();
        let _ = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                batch.text(
                    ui.painter(),
                    egui::pos2(100.0, 50.0),
                    egui::Align2::CENTER_CENTER,
                    "C440".to_owned(),
                    egui::FontId::monospace(15.0),
                    egui::Color32::WHITE,
                    egui::Color32::BLACK,
                );
            },
        );
        assert_eq!(batch.len(), 4, "four glyphs, four quads");
    }

    /// A label is laid out only if it can land on the pane.
    ///
    /// `Camera::project` only rejects what is behind the camera, so a node
    /// off to the side still comes back with a screen position — one the
    /// pane's clip then throws away, but only after the text has been laid
    /// out and stamped once per halo sample. Zoomed in, that is almost every
    /// node in the scene, and it was most of the pane's per-frame CPU.
    #[test]
    fn a_label_off_the_pane_is_not_laid_out() {
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(500.0, 400.0));
        let wide = label_pieces(rect, 14.0);
        let zoomed = label_pieces(rect, 2.0);
        assert!(!wide.is_empty(), "the pane drew no labels at all; the test is vacuous");
        assert!(
            zoomed.len() < wide.len(),
            "zooming in laid out as many labels ({}) as zoomed out ({}), so the \
             off-pane ones are still being built",
            zoomed.len(),
            wide.len(),
        );
        // And nothing is laid out far outside the pane, at either zoom: a
        // label's own reach is the only slack the cull allows.
        let slack = rect.expand(LABEL_REACH * 2.0);
        for piece in wide.iter().chain(&zoomed) {
            assert!(
                slack.contains(piece.ink.min) && slack.contains(piece.ink.max),
                "a label was laid out at {:?}, outside the pane {rect:?}",
                piece.ink,
            );
        }
    }

    /// A node with `activation`, on the home sheet or off it, and nothing
    /// else going on — the two inputs the reserve turns on.
    fn fading(activation: f32, on_home: bool) -> lattice_scene::NodeInstance {
        lattice_scene::NodeInstance {
            lattice_pos: lattice_core::LatticePos::new(0, 0, if on_home { 0 } else { 1 }),
            world_pos: glam::Vec3::ZERO,
            color: glam::Vec4::ONE,
            activation,
            octaves: [0.0; lattice_scene::OCTAVE_SLOTS],
            seed: 0.0,
            outlined: false,
            hovered: false,
            on_home,
            scale: 1.0,
            gutter: 0.0,
            comma: 0.0,
            cents: 0.0,
            melody_slots: 0,
            bass_slots: 0,
            melody_level: 0.0,
            bass_level: 0.0,
            melody_color: glam::Vec4::ONE,
            bass_color: glam::Vec4::ONE,
            trail: 0.0,
        }
    }

    /// The last visibility floor: a label that stops part-way down and then
    /// vanishes reads as holding steady and being switched off, which is the
    /// thing the 0.35 floor was removed everywhere for.
    ///
    /// The reserve that produces it is right on the home sheet — the recorded
    /// name takes over at exactly that level — and wrong anywhere a trail can
    /// never land, which is every other sheet.
    #[test]
    fn only_a_name_that_will_be_kept_stops_short_of_zero() {
        // Off the home sheet: nothing will ever be recorded there, so the
        // label rides the envelope all the way out.
        for keeps_names in [false, true] {
            assert_eq!(label_strength(&fading(0.2, false), false, keeps_names), 0.2);
            assert_eq!(label_strength(&fading(0.02, false), false, keeps_names), 0.02);
            assert_eq!(label_strength(&fading(0.0, false), false, keeps_names), 0.0);
        }
        // On the home sheet with the names kept, it settles on the level the
        // record will hold it at rather than easing out and popping back.
        assert_eq!(label_strength(&fading(0.8, true), false, true), 0.8);
        assert_eq!(label_strength(&fading(0.2, true), false, true), TRAIL_LABEL_STRENGTH);
        // ...and with them off, the home sheet fades out like anything else.
        assert_eq!(label_strength(&fading(0.2, true), false, false), 0.2);
        // A silent node reserves nothing at all, wherever it sits: the
        // reserve is for a name on its way to being kept, not for every node
        // the view holds.
        assert_eq!(label_strength(&fading(0.0, true), false, true), 0.0);
        // A hover is always fully readable, mid-fade or not.
        let mut hovered = fading(0.05, false);
        hovered.hovered = true;
        assert_eq!(label_strength(&hovered, false, false), 1.0);
        // Once the name IS recorded, it reads at the kept level, scaled by
        // how much memory is left.
        let mut kept = fading(0.0, true);
        kept.trail = 1.0;
        assert_eq!(label_strength(&kept, true, true), TRAIL_LABEL_STRENGTH);
        kept.trail = 0.5;
        assert_eq!(label_strength(&kept, true, true), TRAIL_LABEL_STRENGTH * 0.5);
    }
}
