//! The 3D lattice view pane: orbit camera on drag, zoom on scroll, node
//! labels, and the tuning-learn overlay.

use super::{display_note_name, learn_pulse};
use crate::{theme, SharedState};
use egui::Sense;
use harmonigraph_render::lattice_paint_callback;
use harmonigraph_scene::{derive_scene, Camera, Projection, SevensLabel, TrailMark};

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

    // The lattice's own place in the shape list, claimed before the labels are
    // laid out and filled in after. The names go INTO that callback — they are
    // drawn inside its scene pass, so that a node in front covers the name of
    // the node behind it — and they are not known until they have been drawn.
    // What still has to land on top of the callback is the drawn comma and
    // septimal marks, which `draw_node_labels` puts straight on this painter
    // (see #207), so the order the shapes are added in is not free to change.
    let lattice = ui.painter().add(egui::Shape::Noop);

    // One batch for the node labels: a batch draws in the order it collected,
    // so anything meant to sit ON TOP of the names has to be a second batch
    // rather than a later call into this one.
    let mut batch = crate::text::TextBatch::default();
    if state.view.show_labels {
        draw_node_labels(ui, rect, &scene, &state.view, &mut batch);
    }
    // The badge is laid out here, before the names are handed over, though it
    // is DRAWN after them. Laying text out is what rasterizes glyphs into
    // egui's font atlas, and an atlas that changes size between the two
    // recreates the texture behind whichever renderer went first. Growing it
    // before the names go is what keeps that to one recreate.
    //
    // A saving now, not a correctness requirement: each renderer holds a
    // mirror of its own (see `crate::text::AtlasMirror`), and each is handed
    // the atlas on the frame its own glyphs move.
    let badge = state.learn_active.then(|| learn_badge(ui, rect, now));
    ui.painter().set(
        lattice,
        lattice_paint_callback(
            rect,
            &scene,
            batch.lattice_labels(ui.painter(), rect.min, state),
            state.target_format,
            0,
            Some(state.instruments.lattice_stats.clone()),
        ),
    );
    if let Some(mut badge) = badge {
        draw_learn_overlay(ui, rect, state, now, &mut badge);
    }
}

/// Learn mode is armed: show it ON the lattice too, so the mode is obvious
/// even when the Tuning tab (and its Learn toggle) is hidden.
///
/// The one piece of text on this pane that does NOT follow the camera. It
/// names a mode rather than a node — pinned to the corner, sized like the rest
/// of the UI's chrome — and a badge that grew as you zoomed in would be saying
/// something about the lattice, which is exactly what it is not about.
///
/// Which is also why it draws OVER the node labels, in a batch and a border
/// stroke of its own: it is chrome about the pane, not a thing in the picture,
/// and a name that happens to land in the corner crossing the word or the
/// border reads as the badge being part of the lattice.
///
/// The word itself is laid out by [`learn_badge`] one step earlier — see the
/// call site for why the layout and the drawing are on opposite sides of the
/// names' flush.
fn draw_learn_overlay(
    ui: &egui::Ui,
    rect: egui::Rect,
    state: &SharedState,
    now: f64,
    badge: &mut crate::text::TextBatch,
) {
    let painter = ui.painter_at(rect);
    painter.rect_stroke(
        rect.shrink(1.5),
        0,
        egui::Stroke::new(2.0, theme::armed().gamma_multiply(learn_pulse(now))),
        egui::StrokeKind::Inside,
    );
    badge.flush(&painter, rect, state, crate::text::LATTICE_LEARN);
}

/// The badge's word, laid out into a batch of its own and drawn by
/// [`draw_learn_overlay`].
fn learn_badge(ui: &egui::Ui, rect: egui::Rect, now: f64) -> crate::text::TextBatch {
    let mut badge = crate::text::TextBatch::default();
    badge.text(
        &ui.painter_at(rect),
        rect.left_top() + egui::vec2(10.0, 8.0),
        egui::Align2::LEFT_TOP,
        "LEARN".to_string(),
        egui::FontId::monospace(12.0),
        theme::armed().gamma_multiply(learn_pulse(now)),
        theme::well().gamma_multiply(learn_pulse(now)),
    );
    badge
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
/// Reserved on the way OUT only, which is what `node.departing` is read for.
/// The argument above is entirely about a record that is coming, and nothing
/// is coming for a note easing IN — a low activation there is a note that has
/// barely started rather than one nearly gone. Reserving on both ends puts
/// the name at half brightness the instant a key goes down, over a node still
/// at a fraction of it, which is the same "steady, then switched" the reserve
/// exists to remove, mirrored.
///
/// Reserved only where a trail can actually land, which is the home sheet:
/// off-sheet nodes are deliberately never marked (a lone memory floating out
/// in the sevens dimension reads as noise — see `harmonigraph_scene::trail`), so
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
fn label_strength(node: &harmonigraph_scene::NodeInstance, trailed: bool, keeps_names: bool) -> f32 {
    if node.hovered {
        return 1.0;
    }
    let recorded = if trailed { TRAIL_LABEL_STRENGTH * node.trail } else { 0.0 };
    let reserved = if keeps_names && node.on_home && node.departing && node.activation > 0.0 {
        TRAIL_LABEL_STRENGTH
    } else {
        0.0
    };
    node.activation.max(recorded).max(reserved)
}

/// Labels on hovered, sounding, and -- with the trail's "Keep note names"
/// on -- already-visited nodes, projected with the same camera as the nodes:
/// the note name centered on the node, optionally its pitch class in cents
/// just below.
///
/// Collected here and drawn by the lattice's own callback, inside its scene
/// pass, each name at its node's place in the back-to-front order — so a node
/// in front covers the name of the node behind it. The batch carries which
/// node each glyph belongs to (`TextBatch::attached_to`); the caller hands the
/// finished batch to `lattice_paint_callback`.
///
/// The DRAWN marks are the exception, and the one thing here that still goes
/// straight onto the painter: the syntonic `+`/`-` and the septimal chevron
/// are egui image quads, so they are drawn over the whole picture and float
/// on a node that has covered the name beside them. See #207, which is about
/// putting them in the same instance stream as the glyphs.
///
/// Every factor a label's size answers to is gathered here rather than by the
/// callers -- the pane's own size, the camera's zoom and the user's Size bar --
/// so the docked view, the Render preview and the offline render cannot drift
/// apart over which of the three each remembered to apply.
pub(crate) fn draw_node_labels(
    ui: &egui::Ui,
    rect: egui::Rect,
    scene: &harmonigraph_scene::Scene,
    view: &harmonigraph_scene::ViewConfig,
    batch: &mut crate::text::TextBatch,
) {
    // The nodes are world-space geometry and their labels are typeset in
    // points, so a label stays ON its node only by following the two things
    // that decide how big a node draws:
    //
    //   - the PANE, whose height is the whole of what maps world units to
    //     pixels (the projection's window is `distance * tan(fov/2)` high
    //     and lands on the viewport's height, in x as much as in y);
    //   - the CAMERA, via `screen_scale`.
    //
    // The pane is why this needs no argument from its callers. A preview
    // drawing the lattice small, an offline render drawing it large, and a
    // window being dragged narrower are one question with one answer, where
    // they were three: a factor threaded from the Video pane, a scale factor,
    // and nothing at all.
    //
    // The size the labels are actually DRAWN at, and it is continuous: the
    // nodes are shader geometry that scales smoothly with the camera, so type
    // that moved from rung to rung of the size ladder stepped against its own
    // subject every few frames of a zoom. What each node is rasterized at is
    // snapped off this below -- see `crate::text::TextBatch::magnified`, which
    // is where the two part company and why.
    let want = rect.height() / REFERENCE_HEIGHT * view.label_scale * scene.camera.screen_scale();
    let ppp = ui.painter().ctx().pixels_per_point();
    let projector = scene.projector(glam::Vec2::new(rect.width(), rect.height()));
    // "Keep note names" retains a name only while the trail marks that
    // populate `node.trail` are on; with the marks Off the field never fills,
    // so a fading name has nothing to settle onto and should ease all the way
    // out (the pre-existing behavior).
    let keeps_names = view.trail_labels && view.trail_mark != TrailMark::Off;
    for (index, node) in scene.nodes.iter().enumerate() {
        let trailed = view.trail_labels && node.trail > 0.0;
        // `is_visible` re-checks what `Scene::pick` already enforces, and
        // `hovered` is picking's alone, so this is a second lock on one door.
        // It stays because the field is public shared state rather than
        // picking's private output, and what it costs to be wrong is a name
        // floating in the sevens dimension on a node that draws nothing.
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
        if !rect.expand(LABEL_REACH * want).contains(center) {
            continue;
        }
        let outline = theme::well().gamma_multiply(strength);
        // Everything below is this node's name, and is collected as such: the
        // lattice draws a label at its own node's place in the back-to-front
        // order, so a nearer node covers it the way it covers the node behind
        // it. See `crate::text::TextBatch::attached_to`.
        batch.attached_to(index as u32, |batch| {
            // Off-sheet nodes draw at their own size (ViewConfig::sevens_size),
            // and their text goes with them — a full-size label on a half-size
            // node reads as a label with a node attached. Floored so the
            // smallest sheet is still legible rather than merely present.
            // Per NODE, because an off-sheet node draws at its own size and so
            // does its label: the rasterized size has to be snapped for the size
            // it will really be set at, not for the pane's. `magnify` is the
            // little that is left over, and it is what the ladder would otherwise
            // have thrown away.
            //
            // Through `ladder` rather than a `snap_scale` and a division, so the
            // ceiling on the raster is a ceiling on the DRAWN size too. Dividing
            // the raw request by the clamped raster absorbs everything past
            // `MAX_GLYPH_PX` into the magnification, which is a bitmap stretched
            // rather than type set larger — and on a 2x display the ceiling is
            // crossed at half the zoom, so the same camera reads sharp on an
            // external monitor and soft on the laptop panel.
            let want = want * node.scale.max(0.6);
            let (scale, magnify) = crate::text::ladder(want, NAME_SIZE, ppp);
            // What an off-sheet node says, and whether it says anything: its
            // name shares a LETTER and an accidental with the node two fifths
            // down, but not the whole string — the septimal mark is the column
            // that tells the two apart, so the name is what distinguishes an
            // off-sheet node rather than the one thing every sheet repeats.
            // See SevensLabel.
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
                    magnify,
                ),
                SevensLabel::Name => {
                    let name = display_note_name(node.lattice_pos, view.tempered());
                    draw_stacked_name(
                        batch,
                        ui.painter(),
                        center,
                        name,
                        theme::text().gamma_multiply(strength),
                        outline,
                        scale,
                        magnify,
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
                // Magnified about the node with the name above it -- `name_bottom`
                // and the gap are both measured at the rasterized size, so the
                // readout hangs off the name by the same proportion at any zoom.
                batch.magnified(center, magnify, |batch| {
                    batch.text(
                        ui.painter(),
                        center + egui::vec2(0.0, name_bottom + CENTS_GAP * scale - top),
                        egui::Align2::CENTER_TOP,
                        text,
                        font,
                        theme::text_dim().gamma_multiply(strength),
                        outline,
                    );
                });
            }
        });
    }
}

/// The note name's letter, the size the label reads at.
///
/// At the framing a fresh view opens at, that is. Every size here is a size at
/// scale 1, and the scale a label is actually drawn at follows the camera —
/// see [`draw_node_labels`].
///
/// Doubled from the 15pt these labels were drawn at for as long as they had no
/// setting, along with everything below that is measured in points. The Size
/// bar landed on 2 immediately and stayed there, which says the old number was
/// wrong rather than that the bar wanted using — so the number moved and the
/// bar went back to reading 1 at the size the lattice is actually looked at.
pub(crate) const NAME_SIZE: f32 = 30.0;
/// The pane the sizes here are quoted against: 860 points tall, which is what
/// the lattice gets in the 1512x886 window they were dialled in. A pane half
/// as tall draws them half the size, because a pane half as tall draws the
/// LATTICE half the size — see [`draw_node_labels`].
///
/// Height, not width: the projection's window is a height, and the aspect
/// ratio spreads it sideways, so a node's size on screen is a function of the
/// pane's height whichever way the pane is stretched.
const REFERENCE_HEIGHT: f32 = 860.0;
/// The cents readout under it: subordinate to the name, so smaller, and
/// tucked right beneath it rather than floating free.
///
/// Under half the letter, which is what makes the pair read as a name with a
/// number under it rather than as two lines. It is a six-character number
/// against a one-character name, so at any size close to the letter's the
/// readout is the wider of the two and wins the eye.
pub(crate) const CENTS_SIZE: f32 = 14.0;
/// How far a label can reach from the node it belongs to, in points at
/// scale 1 — the name, its marks, the gap and the cents line under it, with
/// room to spare. Scaled with the label like everything else here, so the
/// cull leaves a zoomed-in label the room it now takes. Only used to decide
/// that a label is too far off the pane to be worth laying out, so it errs
/// generous: too small silently clips a label at the edge, too large only
/// costs the work this saves.
pub(crate) const LABEL_REACH: f32 = 96.0;

/// Air between the bottom of the name's glyphs and the top of the cents
/// readout's. Real pixels of gap, since both ends are measured as ink: the
/// two are one label, sitting together without crowding.
///
/// Small against the letter it hangs under — a fifth of the name's size. Ink
/// to ink is a tighter measure than it sounds: the same gap struck between
/// galley boxes would carry the leading of both fonts on top of it, which is
/// the spacing that had the two floating apart.
pub(crate) const CENTS_GAP: f32 = 3.0;
/// Every mark in the column, relative to the letter -- accidental and comma
/// alike, which is what keeps the two rows reading as one annotation rather
/// than as one row set smaller than the other. Small enough that the two
/// stacked still fit inside the letter's own height: a label that grows
/// taller than its letter reads as two lines rather than one name.
const MARK_SCALE: f32 = 0.55;
/// The size the marks are actually laid out at.
pub(crate) const MARK_SIZE: f32 = NAME_SIZE * MARK_SCALE;

/// How far the accidental rises and the comma sign drops, as a fraction of
/// the offset that would set each flush with the letter's own line box.
///
/// This is the ONLY lever on the air between the two rows, and it has to be
/// the only one: every mark in the column sets at [`MARK_SIZE`], so anything
/// that opens the gap by shrinking one row makes that row visibly the
/// smaller of the two. `♯`'s ink is the tallest in the column, but the pair
/// that gives the mismatch away is the two count digits — one directly over
/// the other, in the same column, where 10% off one of them reads as a
/// typo rather than as air.
///
/// Flush (1.0) is the loosest the rows can sit and still be one name: it
/// spends every point the letter's leading offers on the space between them,
/// about 6pt of clear air under `♯` at scale 1 and nearer nine under `♭`,
/// which is more than the marks' own heights and stops the column reading as
/// one stacked annotation. Pulling in tightens that without touching what
/// says which mark is which, since the cue is the ORDER of the two.
///
/// Measured as ink at scale 1, the two clearances that bind — a count over
/// the count below it, and `♯` over the whole row under it:
///
/// | rise | count over count | `♯` over the row |
/// |------|------------------|------------------|
/// | 0.8  | 0.6pt            | −1.4pt (they interleave) |
/// | 0.9  | 2.3pt            | 0.3pt            |
/// | 1.0  | 4.0pt            | 2.0pt            |
///
/// 0.9 is the tightest that keeps the rows from interleaving at all.
const MARK_RISE: f32 = 0.9;

/// Iosevka Fixed's advance, as a fraction of the em: every cell is half an
/// em wide. A drawn mark claims exactly this, so it sits in the same column
/// grid as the typeset accidental above it.
pub(crate) const MARK_ADVANCE: f32 = 0.5;
/// How far a mark's COUNT is pulled back toward its sign, as a fraction of
/// the mark size — `♯2` and `+2` set tighter than one monospace cell each.
///
/// A count is not a second character of a word, it is a multiplier on the
/// sign beside it, and monospace advance sets the two as far apart as it
/// sets `♯` from the letter's own column. Iosevka leaves 0.11em of ink-to-ink
/// air there (0.13 after `♭`, which has the deeper side bearing), against
/// glyphs about 0.4em wide.
///
/// One constant covers both halves of the column because the drawn signs are
/// already cut to the typeface's own ink width (see [`MARK_INK_W`]): `+2`
/// and `♯2` open within 0.001em of each other, so tracking them by the same
/// amount keeps them matched rather than merely both tighter.
pub(crate) const MARK_TRACK: f32 = 0.06;
/// Iosevka's own stroke weight, measured off its outlines: 70/1000 em, as a
/// fraction of the mark's font size.
///
/// A constant rather than a setting, because the face gives no other answer
/// to weigh it against. It uses ONE weight for everything — `♯`'s verticals
/// are 69 and its bars 70, the hyphen is 70, `+` is about 70 — and it does
/// that across a glyph 878 units tall (`♯`) and one 70 units tall (`-`)
/// alike. So the typeface's own answer to "should a smaller mark be drawn
/// heavier?" is no, and an optical-sizing argument for 0.10 or 0.12 is an
/// argument against the face these marks sit in.
///
/// Heavier weights were also compensating for something since fixed: while
/// the marks were composited shapes their feathered joins read heavier than
/// the geometry measured, and while they were typeset a bar this thin really
/// did smear. Rasterized with a whole-pixel floor (see [`mark_key`]), 0.07 is
/// a clean line — and it is the line the rest of the label is drawn with.
const MARK_WEIGHT: f32 = 0.07;

/// The ink width Iosevka gives `+` and `-` within that cell (372/1000 em).
/// Matching it is what keeps a drawn sign from reading as a different size
/// of mark than the `♯` stacked over it.
pub(crate) const MARK_INK_W: f32 = 0.372;
/// And the height of `+`'s upright (386/1000 em).
const PLUS_INK_H: f32 = 0.386;
/// Air between the accidental/comma column and the septimal mark, as a
/// fraction of the mark's font size. Small: enough that the mark is not
/// read as another row of the stack it sits beside, not so much that it
/// floats free of the name it belongs to.
pub(super) const SEPTIMAL_GAP: f32 = 0.22;
/// The septimal mark's box, relative to the `+` box beside it. One, which
/// is to say: the same box.
///
/// It was 1.25 to keep a FILLED TRIANGLE from reading lighter than the `+`,
/// a triangle covering half its bounding box. The chevron that replaced it
/// has the opposite problem -- two arms spanning the full diagonal are
/// already more ink than the `+` at equal stroke -- so the compensation
/// went from justified to doubled.
const SEPTIMAL_BULK: f32 = 1.0;
/// One piece of a mark, in the mark bitmap's own pixel space.
///
/// These are never drawn to the screen. They describe a shape that gets
/// rasterized ONCE into a coverage bitmap, so pieces may abut or overlap
/// freely -- coverage is a max over pieces, not a composite of them, and
/// none of the artifacts of drawing them separately can arise.
enum MarkPiece {
    Bar(egui::Rect),
    /// A stroked segment with FLAT terminals, as four corners.
    ///
    /// Not a distance-to-segment test, which is the easy way to stroke a
    /// line and gives round caps and a round join. Iosevka cuts its
    /// terminals flat, and a chevron with rounded ends and a blunt apex
    /// reads as a different hand than the `♯` above it.
    Quad([egui::Pos2; 4]),
}

impl MarkPiece {
    /// Whether this piece covers a point, in bitmap pixel space.
    fn covers(&self, p: egui::Pos2) -> bool {
        match self {
            MarkPiece::Bar(rect) => rect.contains(p),
            MarkPiece::Quad(corners) => {
                // Convex and consistently wound: inside is the same side of
                // every edge.
                let (mut neg, mut pos) = (false, false);
                for i in 0..4 {
                    let (a, b) = (corners[i], corners[(i + 1) % 4]);
                    let cross = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
                    neg |= cross < 0.0;
                    pos |= cross > 0.0;
                }
                !(neg && pos)
            }
        }
    }
}

/// One arm of a stroked mark: the segment `a`-`b` at `width`, with flat
/// terminals, extended past `b` by half a width so two arms meeting there
/// overlap into a clean point instead of leaving a notch in the outer
/// corner. Overlap is free -- coverage is a union.
fn arm(a: egui::Pos2, b: egui::Pos2, width: f32) -> MarkPiece {
    let along = (b - a).normalized();
    let across = egui::vec2(-along.y, along.x) * (width / 2.0);
    let tip = b + along * (width / 2.0);
    MarkPiece::Quad([a + across, tip + across, tip - across, a - across])
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
    /// Each [`crate::text::RINGS`] radius in whole physical pixels, which is
    /// what [`crate::text::ring_radius`] rounds them to.
    ///
    /// In the key because the RIM is rasterized into its own bitmap now, so
    /// the bitmap is a function of the ring geometry as well as the mark's.
    /// It is the only place `ppp` reaches the rim, the two `size` fields
    /// having already folded it in.
    rings_px: [u32; crate::text::RINGS.len()],
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
            vec![arm(base_l, tip, thick), arm(base_r, tip, thick)]
        }
    };
    (pieces, [bw as usize, bh as usize])
}

/// Supersampling grid used to turn a mark's outline into coverage. 4x4 is
/// finer than the antialiasing a shape would have got from the tessellator,
/// and on a mark a dozen pixels square it is the difference between an edge
/// and a staircase.
///
/// It used to be free — "a mark bitmap is a dozen pixels square and is built
/// once per size" — and it is not now that a size follows the camera. The rim
/// reads this grid twenty times over, so the same 4x4 on a mark two hundred
/// pixels across was sixteen million coverage tests, several frames' worth,
/// spent on one `+` the first time a zoom asks for that size. The samples
/// stayed and the reading of them got cheaper: see [`mark_coverage`].
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

/// The mark's own coverage, supersampled, as a SUMMED-AREA table over a
/// `(w*n) x (h*n)` grid — every entry the number of covered cells above and
/// left of it, so the count inside any rectangle is four lookups.
///
/// Split out of [`rasterize_mark`] because the rim reads the SAME coverage
/// twenty times over at twenty offsets, and testing the geometry again for
/// each would be twenty times the work for an answer that cannot differ.
/// Summed rather than plain because each of those reads is an `n x n` block:
/// counting it cell by cell made the rim quadratic in the supersampling on
/// top of everything else, which was survivable while marks were a dozen
/// pixels square and is what made a zoomed one cost frames. The answer is
/// identical — this counts the same cells by subtraction.
fn mark_coverage(pieces: &[MarkPiece], [w, h]: [usize; 2], n: usize) -> Coverage {
    let (sw, sh) = (w * n, h * n);
    let step = 1.0 / n as f32;
    let mut sums = vec![0u32; (sw + 1) * (sh + 1)];
    for sy in 0..sh {
        for sx in 0..sw {
            let p = egui::pos2((sx as f32 + 0.5) * step, (sy as f32 + 0.5) * step);
            let covered = pieces.iter().any(|piece| piece.covers(p)) as u32;
            sums[(sy + 1) * (sw + 1) + sx + 1] = covered + sums[sy * (sw + 1) + sx + 1]
                + sums[(sy + 1) * (sw + 1) + sx]
                - sums[sy * (sw + 1) + sx];
        }
    }
    Coverage { sums, size: [sw, sh] }
}

/// A mark's supersampled coverage, summed. See [`mark_coverage`].
struct Coverage {
    sums: Vec<u32>,
    /// The supersampled grid's own dimensions, which is `sums` less its zero
    /// row and column.
    size: [usize; 2],
}

impl Coverage {
    /// How many covered cells lie in the `n x n` block whose top-left cell is
    /// `(x, y)`, counting cells off the grid as uncovered.
    fn block(&self, x: isize, y: isize, n: usize) -> u32 {
        let [sw, sh] = self.size;
        let (x0, y0) = (x.clamp(0, sw as isize) as usize, y.clamp(0, sh as isize) as usize);
        let x1 = (x + n as isize).clamp(0, sw as isize) as usize;
        let y1 = (y + n as isize).clamp(0, sh as isize) as usize;
        let at = |x: usize, y: usize| self.sums[y * (sw + 1) + x];
        at(x1, y1) + at(x0, y0) - at(x1, y0) - at(x0, y1)
    }
}

/// The mark's RIM, rasterized to alpha as one bitmap: the same rings stamped
/// the same way, composited here instead of once per stamp in the shape list.
///
/// This is what `crate::text`'s shader does for glyphs, and for the reason it
/// gives -- twenty copies of every mark was most of the geometry in a frame
/// that draws a name on every ribbon. Doing it at rasterization time rather
/// than in a shader keeps the mark on egui's ordinary image path, and costs
/// nothing per frame: a bitmap is built once per key and cached.
///
/// Exact, not an approximation of the stamped version. Every stamp carries
/// the same colour, and source-over compositing of one colour at successive
/// alphas is associative -- `1 - prod(1 - a_i)` is the accumulated alpha the
/// twenty draws arrived at -- so the tinted result is identical. Only the
/// resampling differs, and in the safe direction: the composite is resolved
/// once at draw time instead of twenty separately-filtered copies landing on
/// each other.
fn rasterize_mark_rim(key: MarkKey) -> egui::ColorImage {
    let (pieces, [w, h]) = mark_geometry(key);
    let n = MARK_SUPERSAMPLE;
    let coverage = mark_coverage(&pieces, [w, h], n);
    // Room for the widest ring on every side, so no stamp is clipped.
    let pad = key.rings_px.iter().copied().max().unwrap_or(1) as usize;
    let (rw, rh) = (w + 2 * pad, h + 2 * pad);

    // Stamp offsets in SUPERSAMPLE cells, in the order the rings are drawn:
    // compositing is order-dependent, and this is the order the shape list
    // had.
    let mut stamps: Vec<(isize, isize, f32)> = Vec::new();
    for (ring, (_, alpha, samples)) in crate::text::RINGS.iter().enumerate() {
        if *samples == 0 {
            continue;
        }
        let radius = key.rings_px[ring] as f32;
        for i in 0..*samples {
            let angle = std::f32::consts::TAU * i as f32 / *samples as f32;
            stamps.push((
                (angle.cos() * radius * n as f32).round() as isize,
                (angle.sin() * radius * n as f32).round() as isize,
                *alpha,
            ));
        }
    }

    let mut pixels = Vec::with_capacity(rw * rh);
    for y in 0..rh {
        for x in 0..rw {
            // This pixel's supersample block in the mark's own grid.
            let (bx, by) = (
                (x as isize - pad as isize) * n as isize,
                (y as isize - pad as isize) * n as isize,
            );
            let mut a = 0.0f32;
            for &(ox, oy, alpha) in &stamps {
                let hits = coverage.block(bx - ox, by - oy, n);
                if hits > 0 {
                    let frac = hits as f32 / (n * n) as f32;
                    a += alpha * frac * (1.0 - a);
                }
            }
            pixels.push(egui::Color32::from_white_alpha((a * 255.0).round() as u8));
        }
    }
    egui::ColorImage {
        size: [rw, rh],
        pixels,
        source_size: egui::vec2(rw as f32, rh as f32),
    }
}

/// How many mark bitmaps to keep before starting over. Zooming walks through
/// sizes, and each one is its own bitmap; this is a ceiling on the churn,
/// not a working-set estimate.
pub(crate) const MARK_CACHE_LIMIT: usize = 96;

type MarkTextures = (egui::TextureHandle, egui::TextureHandle);

/// The mark bitmaps, plus the ones evicted too recently to destroy yet.
#[derive(Clone, Default)]
struct MarkCache {
    live: std::collections::HashMap<MarkKey, MarkTextures>,
    /// Evicted handles, each with the pass it was evicted on, held until a
    /// LATER pass has begun.
    ///
    /// Dropping the last handle to a texture makes egui queue its id into
    /// `textures_delta.free`, and the wgpu renderer applies those frees
    /// BEFORE it submits the encoder (see `free_texture` then `queue.submit`
    /// in egui-baseview's wgpu renderer). So a texture evicted midway
    /// through a pass is destroyed while draw commands recorded EARLIER in
    /// that same pass still name it, and the submit fails validation with
    /// "Texture ... has been destroyed" — which is fatal, not recoverable.
    /// Eviction picks an arbitrary victim, so the victim is sometimes a mark
    /// the pass has already painted.
    ///
    /// Holding the handle until the next pass is what makes the eviction
    /// safe: by then the pass that drew it has been submitted, and the key
    /// is already out of `live`, so nothing new can reference the old id.
    retired: Vec<(u64, MarkTextures)>,
}

/// The texture for one mark, rasterized on first use and kept in egui's own
/// per-frame data store.
fn mark_texture(ctx: &egui::Context, key: MarkKey) -> MarkTextures {
    let pass = ctx.cumulative_pass_nr();
    let cached = ctx.data_mut(|d| d.get_temp::<std::sync::Arc<MarkCache>>(egui::Id::NULL));
    if let Some(hit) = cached.as_ref().and_then(|c| c.live.get(&key)) {
        return hit.clone();
    }
    // LINEAR, because a mark is placed at a subpixel position and resampled
    // exactly as a glyph is. NEAREST would put the pixel grid back.
    let handle = (
        ctx.load_texture(
            format!("{:?}", key),
            rasterize_mark(key),
            egui::TextureOptions::LINEAR,
        ),
        ctx.load_texture(
            format!("{:?}-rim", key),
            rasterize_mark_rim(key),
            egui::TextureOptions::LINEAR,
        ),
    );
    let mut next = cached.map(|c| (*c).clone()).unwrap_or_default();
    // Anything retired on an earlier pass has had its pass submitted, so the
    // handle can go now and the texture is destroyed on this pass's frees.
    next.retired.retain(|(evicted_on, _)| *evicted_on >= pass);
    // Evict ONE rather than emptying the map. Zooming walks through sizes a
    // pixel at a time, so the cache fills during an ordinary drag; clearing
    // it there drops every texture at once and re-rasterizes the whole
    // visible set on the next frame, which is a stall exactly while the
    // camera is moving. Which one goes is arbitrary — there is no recency
    // here to consult — but one at a time keeps the cost flat.
    if next.live.len() >= MARK_CACHE_LIMIT {
        if let Some(&victim) = next.live.keys().next() {
            if let Some(handles) = next.live.remove(&victim) {
                next.retired.push((pass, handles));
            }
        }
    }
    next.live.insert(key, handle.clone());
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
/// `harmonigraph_render`'s text shader states the identity the rim rests on: a
/// label's rim IS the shape stamped around two rings, and the shader moved
/// that loop out of the geometry because 20 copies of every glyph was most
/// of the geometry in a busy frame. The rim here is stamped for the same
/// reason and by the same arithmetic -- same radii, same sample counts, same
/// per-stamp alpha, same `angle = 2*PI*i/samples` -- but it is composited
/// into [`rasterize_mark_rim`]'s bitmap rather than into the shape list, so
/// a mark costs TWO quads however many stamps its rim is made of.
///
/// It has to. A mark was one quad plus twenty stamps when the only marks
/// were on the handful of hovered and sounding lattice nodes; note names put
/// one on every roll ribbon and on every lit node of a collapsed 12-EDO
/// lattice, which is hundreds, and twenty-one quads apiece is most of a
/// frame's geometry again -- the exact cost the text shader exists to have
/// removed.
///
/// Rim first, then the fill, which is the order stamping had and the order
/// the shader kept.
/// Returns how far the mark reaches from its own center, which the caller
/// needs to know what the cents readout has to clear. Read off the texture
/// rather than rebuilt: the bitmap's size IS the mark's size, and asking
/// `mark_geometry` again allocated a fresh piece list per mark per frame on
/// the label path.
fn paint_mark(
    painter: &egui::Painter,
    ppp: f32,
    key: MarkKey,
    center: egui::Pos2,
    color: egui::Color32,
    outline: egui::Color32,
    // The magnification the name beside it is drawn under: a factor, and the
    // node center it is taken about. See `crate::text::TextBatch::magnified`.
    magnify: (egui::Pos2, f32),
) -> f32 {
    let (texture, rim) = mark_texture(painter.ctx(), key);
    let [w, h] = texture.size();
    // The bitmap is rasterized on the same grid the type is (see `mark_key`),
    // and DRAWN at whatever size the label is actually at -- the two are the
    // same split, for the same reason, and they have to be the same split or a
    // name would glide while the `+` beside it stepped. The textures are
    // LINEAR, so a quad off its bitmap's size resamples exactly as a glyph off
    // its atlas cell does.
    let (origin, k) = magnify;
    let at = |p: egui::Pos2| origin + (p - origin) * k;
    let rect = egui::Rect::from_center_size(
        at(center),
        egui::vec2(w as f32 / ppp, h as f32 / ppp) * k,
    );
    let uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
    // The rim's bitmap is the mark's, padded by the widest ring on every
    // side, and centred on the same point -- so it is placed by its own size
    // rather than by an offset from the fill.
    let [rw, rh] = rim.size();
    let rim_rect = egui::Rect::from_center_size(
        at(center),
        egui::vec2(rw as f32 / ppp, rh as f32 / ppp) * k,
    );
    painter.image(rim.id(), rim_rect, uv, outline);
    painter.image(texture.id(), rect, uv, color);
    // The FILL's half height, not the rim's: this is what the cents readout
    // has to clear, and the rim is a halo the text already overlaps.
    //
    // UNmagnified, because the caller is still laying out at the rasterized
    // size and this is one of its measurements — the magnification is applied
    // once, to the finished label, and a measurement that had it applied
    // already would carry it twice.
    h as f32 / ppp / 2.0
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
    MarkKey {
        kind,
        size_px: size_px as u32,
        weight_16: (thick * 16.0).round() as u32,
        rings_px: crate::text::RINGS.map(|(r, _, _)| (r * ppp).round().max(1.0) as u32),
    }
}

/// A note name centered on `anchor`: the letter, then a column carrying its
/// accidental above its syntonic-comma sign, then a column for the septimal
/// mark (`♯` riding high like a superscript, `+` low like a subscript).
/// Every mark is counted rather than repeated (see [`harmonigraph_core::NoteName`]),
/// so a name deep in the lattice -- or five modulations out along the sevens
/// axis -- stays a couple of characters wide instead of sprawling off its node.
///
/// The two comma signs are DRAWN and the accidental is typeset, which is not
/// an inconsistency but the point: `♯` and `♭` are real musical symbols with
/// 878 and 818 units of ink, and they survive the size. The comma signs are
/// bars, and Iosevka has no bar thicker than 70 units. See [`mark_geometry`].
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
///
/// `scale` is the size the label is LAID OUT and rasterized at; `magnify` is
/// how much bigger it is finally drawn, which is what lets the size follow a
/// zoom continuously while the atlas still sees one size per rung. See
/// [`crate::text::ladder`], which hands the pair out together, and
/// [`crate::text::TextBatch::magnified`]; 1.0 is a label drawn at exactly the
/// size it was rasterized at.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_stacked_name(
    batch: &mut crate::text::TextBatch,
    painter: &egui::Painter,
    anchor: egui::Pos2,
    name: harmonigraph_core::NoteName,
    color: egui::Color32,
    outline: egui::Color32,
    scale: f32,
    magnify: f32,
) -> f32 {
    batch.magnified(anchor, magnify, |batch| {
        stacked_name(batch, painter, anchor, name, color, outline, scale, magnify)
    })
}

/// [`draw_stacked_name`]'s layout, all of it at the rasterized size. The one
/// thing that has to know about `magnify` down here is the drawn marks, which
/// are painted straight onto the painter rather than collected in the batch.
#[allow(clippy::too_many_arguments)]
fn stacked_name(
    batch: &mut crate::text::TextBatch,
    painter: &egui::Painter,
    anchor: egui::Pos2,
    name: harmonigraph_core::NoteName,
    color: egui::Color32,
    outline: egui::Color32,
    scale: f32,
    magnify: f32,
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
    // Every mark sits on one line of the mark font, so both rows rise by the
    // same amount -- including the drawn ones, which have no galley to ask.
    let line = measure("0", &mark_font);
    // A drawn mark is centered on its line box, with NO correction toward
    // the font's own bar axis, and that is a measured decision rather than
    // an omission.
    //
    // Iosevka puts the ink of `-`, `+` and `♯` alike at 340/1000 em above
    // the baseline: one axis, by design. Its typo line box is centered on
    // exactly that and its hhea box 35 units above it, which looked like a
    // 0.035 em correction worth applying. But at MARK_SIZE egui rasterizes
    // into whole-point atlas cells, and those same three glyphs come back
    // centered at -0.5pt, +0.0pt and +0.5pt from the line box: a whole
    // point of spread across glyphs the font draws on one axis. The offset
    // is below the size at which the text beside it can be positioned, and
    // reading it off any single glyph measures that glyph's rounding.
    //
    // Centered is the mean of what the font actually renders, and it is
    // exactly where `+` lands.
    let rise = MARK_RISE * (letter.y - line.y) / 2.0;
    let cell = MARK_ADVANCE * mark_size;

    let accidental = name.accidental_mark();
    // Core hands the accidental over as one string, sign then count (see
    // NoteName::accidental_mark). Split rather than respelled, so the choice
    // of `♯` or `♭` stays core's: the count is laid out as its own piece
    // only so it can be tracked in toward the sign.
    let mut accidental_chars = accidental.chars();
    let accidental_sign: String = accidental_chars.by_ref().take(1).collect();
    let accidental_count: String = accidental_chars.collect();
    let syntonic = count_text(name.syntonic_commas);
    let septimal = count_text(name.septimal_commas);
    let track = MARK_TRACK * mark_size;
    // A sign and its count read as ONE mark, so the count follows its sign's
    // cell tracked in rather than a clear cell away -- see MARK_TRACK.
    let tracked_width = |sign: f32, count: &str| {
        if count.is_empty() { sign } else { sign + measure(count, &mark_font).x - track }
    };
    // A drawn sign claims one cell; its count follows in the same column.
    let signed_width =
        |count: &str, present: bool| if present { tracked_width(cell, count) } else { 0.0 };
    // The accidental claims the same cell as the drawn sign under it -- one
    // column grid for both rows.
    let accidental_width =
        if accidental_sign.is_empty() { 0.0 } else { tracked_width(cell, &accidental_count) };
    let column = accidental_width.max(signed_width(&syntonic, name.syntonic_commas != 0));
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

    // The accidental rides high, just inside the top of the letter.
    if !accidental_sign.is_empty() {
        let sign_x = left + letter.x;
        batch.text(
            painter,
            egui::pos2(sign_x, anchor.y - rise),
            egui::Align2::LEFT_CENTER,
            accidental_sign.clone(),
            mark_font.clone(),
            color,
            outline,
        );
        if !accidental_count.is_empty() {
            // Off the CELL, so this count and the comma's below share a
            // left edge.
            batch.text(
                painter,
                egui::pos2(sign_x + cell - track, anchor.y - rise),
                egui::Align2::LEFT_CENTER,
                accidental_count.clone(),
                mark_font.clone(),
                color,
                outline,
            );
        }
        // Whole-string ink: tracking moves the count sideways, and how far
        // the pair reaches DOWN is the same either way.
        bottom = bottom.max(-rise + ink_below(&accidental, &mark_font, line));
    }

    // Drawn sign, then its count: same column, same line, so the pair reads
    // as one mark rather than as a glyph with a number after it.
    let mut draw_signed = |x: f32,
                           direction: f32,
                           count: &str,
                           kind: MarkKind|
     -> f32 {
        let key = mark_key(kind, mark_size, MARK_WEIGHT, ppp);
        let center = egui::pos2(x + cell / 2.0, anchor.y + direction * rise);
        let half_height =
            paint_mark(painter, ppp, key, center, color, outline, (anchor, magnify));
        if !count.is_empty() {
            batch.text(
                painter,
                egui::pos2(x + cell - track, anchor.y + direction * rise),
                egui::Align2::LEFT_CENTER,
                count.to_owned(),
                mark_font.clone(),
                color,
                outline,
            );
        }
        // Whichever reaches lower: the mark's own bitmap from its center, or
        // the count's digits from theirs.
        let ink = half_height
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
/// [`SevensLabel`]).
#[allow(clippy::too_many_arguments)]
fn draw_plain_name(
    batch: &mut crate::text::TextBatch,
    painter: &egui::Painter,
    anchor: egui::Pos2,
    text: &str,
    color: egui::Color32,
    outline: egui::Color32,
    scale: f32,
    magnify: f32,
) -> f32 {
    let font = egui::FontId::monospace(NAME_SIZE * scale);
    let size = painter
        .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
        .size();
    batch.magnified(anchor, magnify, |batch| {
        batch.text(
            painter,
            anchor,
            egui::Align2::CENTER_CENTER,
            text.to_owned(),
            font.clone(),
            color,
            outline,
        );
    });
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
    use harmonigraph_core::{NoteEvent, NoteEventKind};

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

    /// Every drawn mark is one stroke weight, and it is the one the font
    /// draws with.
    ///
    /// Iosevka uses a single weight across its whole set -- `♯`'s verticals
    /// are 69 units, its bars 70, the hyphen 70 -- and it does that over a
    /// glyph 878 units tall and one 70 units tall alike. These marks sit in
    /// that set, so they are drawn at 70/1000 em too, and the `-` and the
    /// `+`'s bar have to come out identical.
    #[test]
    fn every_drawn_mark_is_the_fonts_one_stroke_weight() {
        const WEIGHT: f32 = 0.07;
        let size = 24.0;
        let ppp = 2.0;
        let minus = rasterize_mark(mark_key(MarkKind::Minus, size, WEIGHT, ppp));
        let plus = rasterize_mark(mark_key(MarkKind::Plus, size, WEIGHT, ppp));
        // The bar's thickness: inked rows down a column, read a quarter of
        // the way across so the `+`'s upright is nowhere near it.
        let bar = |img: &egui::ColorImage| {
            let x = img.size[0] / 4;
            (0..img.size[1]).filter(|&y| coverage(img, x, y) > 127).count()
        };
        assert_eq!(bar(&minus), bar(&plus), "the - and the +'s bar disagree");
        // And that thickness is the font's own 0.07 em, to the pixel.
        let want = (WEIGHT * size * ppp).round() as usize;
        assert_eq!(bar(&minus), want, "{} px drawn, {want} px is 0.07 em", bar(&minus));
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
        labelled(rect, distance).0
    }

    /// The pieces of text a chord's labels lay out, with the scene they came
    /// from — so a test can ask what SHOULD have been labelled without
    /// borrowing the answer from the code that decides it.
    fn labelled(
        rect: egui::Rect,
        distance: f32,
    ) -> (Vec<crate::text::TextPiece>, harmonigraph_scene::Scene) {
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.camera.distance = distance;
        // No arrival ramp: the scene below is derived 50ms in, a fraction of
        // any real Fade, and a label's alpha rides its node's activation.
        // This suite is about where a label is DRAWN, not how lit it is.
        state.frame_params.fade_time = 0.0;
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
                draw_node_labels(&child, rect, &scene, &state.view, &mut batch);
            },
        );
        (batch.pieces().to_vec(), scene)
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
        // The other half, and the one an upper bound cannot state: the cull
        // may only drop what is OFF the pane. A bound written as the cull's
        // own reach is met by any cull, however tight — including one that
        // clips a label whose node you are looking straight at, which is the
        // failure `LABEL_REACH`'s doc warns about. So this counts what should
        // have been labelled, through the scene's own projector rather than
        // through the cull's arithmetic: every node the pane can show carries
        // a name, wherever the reach is set.
        for distance in [14.0f32, 2.0] {
            let (pieces, scene) = labelled(rect, distance);
            let projector =
                scene.projector(glam::Vec2::new(rect.width(), rect.height()));
            let on_pane = scene
                .nodes
                .iter()
                // The home sheet's, which is what the size filter below
                // picks out: an off-sheet node draws its name smaller, so
                // counting those on one side and not the other would compare
                // two different things.
                .filter(|node| node.on_home && node.activation > 0.0 && node.is_visible())
                .filter_map(|node| projector.project(node.world_pos))
                .filter(|p| rect.contains(egui::pos2(rect.min.x + p.x, rect.min.y + p.y)))
                .count();
            assert!(on_pane > 0, "no lit node projects onto the pane at distance {distance}");
            // One letter per label, so the letters count the labels. Off-sheet
            // nodes draw theirs smaller, so this is the LARGEST size drawn.
            let biggest = pieces.iter().map(|piece| piece.font_size).fold(0.0f32, f32::max);
            let names = pieces.iter().filter(|piece| piece.font_size == biggest).count();
            assert!(
                names >= on_pane,
                "{on_pane} lit nodes are on the pane at distance {distance} but only \
                 {names} names were laid out: the cull is dropping what you can see",
            );
        }
    }

    /// Labels follow the camera: a name is the same size ON its node at every
    /// zoom, which is the whole of what makes it a label on a node rather than
    /// text over a picture of one.
    ///
    /// Halving the distance doubles the lattice on screen — the ortho window's
    /// half-height is `distance * tan(fov/2)` — so it has to double the type
    /// too. Read off the largest piece each frame laid out, which is the note
    /// name: the marks and the cents line are sized off it.
    #[test]
    fn a_label_grows_with_the_camera() {
        // A pane the height the sizes are quoted against, so this is a test
        // about the camera and not about the pane.
        let rect =
            egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(500.0, REFERENCE_HEIGHT));
        let biggest = |distance: f32| {
            label_pieces(rect, distance)
                .iter()
                .map(|piece| piece.font_size)
                .fold(0.0f32, f32::max)
        };
        assert_eq!(
            biggest(Camera::DEFAULT_DISTANCE),
            NAME_SIZE,
            "the default framing is where the sizes are dialled",
        );
        // Within a rung of the ladder either way. The size follows the camera
        // continuously and is RASTERIZED at the nearest size on offer, which
        // is what keeps a zoom from asking egui for a new one every frame —
        // see `text::snap_scale`.
        let tracks = |distance: f32, want: f32| {
            let got = biggest(distance);
            // Off by at most a rung of the ladder, or half a pixel where that
            // is coarser — the two grains `snap_scale` quantizes on. A quarter
            // of a 30pt name is 7.5 pixels on this 1x context, where half a
            // pixel is a fifteenth of the size and the rung is a thirtieth.
            let slack = (0.04 * want).max(0.5);
            assert!(
                (got - want).abs() <= slack,
                "at distance {distance} a name drew at {got}, not within {slack} of {want}",
            );
        };
        tracks(Camera::DEFAULT_DISTANCE * 0.5, NAME_SIZE * 2.0);
        tracks(Camera::DEFAULT_DISTANCE * 2.0, NAME_SIZE * 0.5);
        tracks(Camera::DEFAULT_DISTANCE * 4.0, NAME_SIZE * 0.25);
        // And the ladder is really there: a nudge of the camera too small to
        // see is not a new size to rasterize.
        assert_eq!(
            biggest(Camera::DEFAULT_DISTANCE),
            biggest(Camera::DEFAULT_DISTANCE * 1.01),
            "a 1% camera move asked for a size of its own",
        );
    }

    /// A node with `activation`, on the home sheet or off it, and nothing
    /// else going on — the two inputs the reserve turns on.
    ///
    /// Departing, as the name says: every level below is a note on its way
    /// out, which is the only end the reserve is for. The arrival is
    /// `a_name_arriving_is_no_brighter_than_the_note_it_names`.
    fn fading(activation: f32, on_home: bool) -> harmonigraph_scene::NodeInstance {
        harmonigraph_scene::NodeInstance {
            lattice_pos: harmonigraph_core::LatticePos::new(0, 0, if on_home { 0 } else { 1 }),
            world_pos: glam::Vec3::ZERO,
            color: glam::Vec4::ONE,
            activation,
            departing: true,
            octaves: [0.0; harmonigraph_scene::OCTAVE_SLOTS],
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

    /// A name ARRIVES on its own note's envelope, like every other layer.
    ///
    /// The reserve is a departure device and its argument only holds there:
    /// it stands in for a record that `node.trail` does not carry until the
    /// frame the release finishes. A note on its way IN has no record coming
    /// and nothing to settle onto, so reserving there pins the name at half
    /// brightness over a node still at a fraction of it — the same "holding
    /// steady and then switching" the reserve exists to remove, at the other
    /// end of the note.
    ///
    /// The band `0 < activation < TRAIL_LABEL_STRENGTH` was reachable only on
    /// the way out when the reserve was written, because a note's core simply
    /// appeared at full. It is now climbed on every note-on, and at the fresh
    /// view — trail marks and their names both on — that is every lit node.
    #[test]
    fn a_name_arriving_is_no_brighter_than_the_note_it_names() {
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        // A long arrival, so the climb through the reserve's band is a stretch
        // to sample rather than a frame of it.
        state.frame_params.fade_time = 1.0;
        state.tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        let scene = derive_scene(
            &state.tracker,
            &state.tuning,
            &state.view,
            &state.frame_params,
            state.camera,
            None,
            0.05,
        );
        let keeps_names = state.view.trail_labels && state.view.trail_mark != TrailMark::Off;
        assert!(keeps_names, "the fresh view keeps names; without that this proves nothing");
        let node = scene.nodes.iter().find(|n| n.activation > 0.0).expect("the note lit a node");
        assert!(node.on_home, "the lit node is off the home sheet, where nothing is reserved");
        assert!(
            node.activation < TRAIL_LABEL_STRENGTH,
            "sampled past the reserve's band at {}, so this cannot see the plateau",
            node.activation,
        );
        assert_eq!(
            label_strength(node, false, keeps_names),
            node.activation,
            "an arriving name was drawn at the trail reserve, not at its note's own level",
        );

        // The other end, through the same derive rather than a hand-built
        // node: the key comes up once the arrival has landed, and at the same
        // depth into the departure the reserve DOES hold the name up. Without
        // this half, a fix that simply never reserved would pass the half
        // above.
        state.tracker.handle_event(NoteEvent {
            time: 1.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Off,
        });
        let scene = derive_scene(
            &state.tracker,
            &state.tuning,
            &state.view,
            &state.frame_params,
            state.camera,
            None,
            1.9,
        );
        let node = scene.nodes.iter().find(|n| n.activation > 0.0).expect("the note still lights");
        assert!(node.departing, "the key is up and the arrival landed, so this is a departure");
        assert!(
            node.activation < TRAIL_LABEL_STRENGTH,
            "sampled at {}, above the reserve, so this cannot see it hold",
            node.activation,
        );
        assert_eq!(
            label_strength(node, false, keeps_names),
            TRAIL_LABEL_STRENGTH,
            "a departing name stopped reserving the level its trail record takes over at",
        );
    }

    /// The rim bitmap IS the stamped halo: the mark's own shape grown by the
    /// widest ring on every side, opaque where the stamps pile up over the
    /// mark and clear at the corners no stamp reaches.
    ///
    /// Pins the geometry the fragment-stage move has to preserve. A rim that
    /// came out unpadded would clip the halo to the mark's own box; one
    /// padded asymmetrically would put the mark off-centre inside its own
    /// glow, which reads as a lopsided mark rather than as a bug.
    #[test]
    fn the_rim_bitmap_grows_the_mark_by_its_widest_ring() {
        let key = mark_key(MarkKind::Minus, 8.0, 0.07, 2.0);
        let [fw, fh] = rasterize_mark(key).size;
        let rim = rasterize_mark_rim(key);
        let [rw, rh] = rim.size;

        let pad = key.rings_px.iter().copied().max().unwrap() as usize;
        assert_eq!([rw, rh], [fw + 2 * pad, fh + 2 * pad], "padded by the widest ring");

        // Symmetric, so the mark sits centred in its own halo — which is what
        // lets `paint_mark` place the two quads on one centre.
        for y in 0..rh {
            for x in 0..rw {
                assert_eq!(
                    rim.pixels[y * rw + x].a(),
                    rim.pixels[y * rw + (rw - 1 - x)].a(),
                    "the halo of a symmetric mark is symmetric, at {x},{y}"
                );
            }
        }

        // Solid over the mark, and untouched in the corners: a `-` is wide and
        // flat, so the corners of a box grown by the ring on every side are
        // past every stamp.
        assert_eq!(rim.pixels[(rh / 2) * rw + rw / 2].a(), 255, "opaque behind the mark");
        assert_eq!(rim.pixels[0].a(), 0, "clear where no stamp reaches");
    }
}
