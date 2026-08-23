//! The 3D lattice view pane: orbit camera on drag, zoom on scroll, node
//! labels, and the tuning-learn overlay.

use super::{display_note_name, learn_pulse, zoom_gesture};
use crate::{theme, SharedState};
use egui::Sense;
use harmonigraph_render::lattice_paint_callback;
use harmonigraph_scene::{derive_scene, Camera, NoteNames, Projection, SevensLabel};
use crate::marks::{
    draw_plain_name, draw_stacked_name, painter_ink, CENTS_GAP, CENTS_SIZE, LABEL_REACH,
    NAME_SIZE, REFERENCE_HEIGHT,
};

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
    // Zoom when the pointer is over the view.
    if let Some((scroll, zoom)) = zoom_gesture(ui, &response) {
        if scroll != 0.0 {
            state.camera.zoom(scroll);
        }
        if zoom != 1.0 {
            state.camera.zoom_by(zoom);
        }
    }
    if response.double_clicked() {
        // Reset orbit/zoom, but keep the chosen projection: that's a view
        // preference, not a navigation state. Home is the ORIGIN of the
        // lattice, so the window's center goes back with the camera —
        // otherwise a double-click on a scrolled view resets the camera into
        // the middle of wherever it had scrolled to, which is not a reset.
        state.camera = Camera {
            projection: state.camera.projection,
            ..Default::default()
        };
        state.view.center_threes = 0;
        state.view.center_fives = 0;
    }
    // The window's center follows the camera, so scrolling never leaves the
    // reach the note names are chosen out of behind. Here rather than inside
    // each gesture: it is idempotent, and one call cannot be the one a new
    // gesture forgets. The interactive copy only — it writes shared state.
    state.view.follow_camera(&mut state.camera);

    // The ground the picture stands on, painted here rather than left to
    // whatever is behind the pane. A picture pane is recessed below the chrome
    // around it — `spectral_pane`, `spiral_pane` and the render preview all
    // open by filling their rect with the well — and the lattice showing the
    // dock's own tab body through instead put it a rung up, reading as a
    // lighter card beside the analyzer.
    //
    // Painted from the SAME value handed to the knockout, which is the whole
    // reason to take it off `state` rather than off the theme: offline the
    // ground is the render layout's, not the skin's, and a fill that went to
    // the theme for it would stand the picture on one color while its cleared
    // discs cleared to another.
    let background = state.background;
    ui.painter().rect_filled(rect, 0.0, state.background_ink());
    let stats = Some(state.instruments.lattice_stats.clone());
    draw_lattice(ui, rect, state, now, 0, background, Some(&response), stats);
}

/// The lattice's shared draw sequence: derive the scene, pick when this is
/// the interactive copy, lay out and draw the node labels, and hand the
/// frame to its paint callback. Both [`lattice_pane`] and the Render
/// preview's second live copy of the lattice
/// (`panes::render::preview_lattice`) run this — they differ only in which
/// GPU pane id this copy claims (`surface`, see [`pane_id`]), what the
/// sevens knockout clears to (`background`), and where the frame's GPU stats
/// land (`stats`).
///
/// `response` is `None` for a non-interactive copy: showing a hover,
/// picking, and the learn badge all answer to a pointer, and the preview
/// has none of its own — its camera is framed in the Lattice tab, not here
/// — so all three are skipped together rather than by three flags that
/// could disagree.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_lattice(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    state: &mut SharedState,
    now: f64,
    surface: usize,
    background: glam::Vec4,
    response: Option<&egui::Response>,
    stats: Option<std::sync::Arc<harmonigraph_render::LatticeStats>>,
) {
    // The lattice this pane shows, which is a different window from the one
    // the pane beside it shows and from the one the names are chosen out of —
    // see `ViewConfig::scrolled`. Derived here, per copy and per frame, and
    // never written back: the docked pane and the Video tab's preview both
    // reach this function every frame at their own aspects.
    let window = state.view.scrolled(&state.camera, rect.width() / rect.height().max(1.0));
    // Published for the perf overlay's node count and for the panes that have
    // to say what the picture shows, from the one place it exists. The docked
    // copy alone writes it — see `SharedState::drawn`, and the `response`
    // argument's own doc for why that flag is what identifies it.
    if response.is_some() {
        state.drawn_this_frame = Some(window);
    }
    let mut scene = derive_scene(
        &state.tracker,
        &state.tuning,
        &state.view,
        &window,
        &state.frame_params,
        state.camera,
        // Only the interactive copy has a hover to show: the preview's
        // camera is framed in the Lattice tab, not here, and a hover picked
        // up while working there is that view's business, not a picture of
        // the render's.
        response.and(state.hovered),
        now,
    );
    // What the AUDIO says, over a scene derived exactly as above: a ring of
    // measured octaves inside the octave band, carrying whichever of two
    // readings the selector asks for. Whether there IS a ring is the WIDTH's
    // to say, not the selector's, and with no ring the pass does not so much
    // as look at the spectrum (`spectral_ring_draws`). A
    // post-pass and not a branch inside the derivation, because the picture
    // around the ring — the geometry, the wheel, the markers, the camera, and the
    // whole of what the keys light — is the same answer either way, and a
    // reading that reached into `derive_scene` would be a second path through
    // all of it.
    super::spectral_fold::apply(&mut scene, state, now);
    // And the node glow's own clock, last: the light is assembled out of every
    // layer's ink, the audio ring included, so what a node is lighting the
    // picture with is not known until the pass above has said what its ring is
    // doing. Per surface, because what this hands out is rows of that surface's
    // own ink strip.
    super::glow_fade::apply(&mut scene, state, surface, now);
    // And the markers' shadows against that light, which is why this is behind
    // both passes above rather than inside the derivation: a cross may not cut
    // the halo of the node it stands in the middle of, and what a node is
    // lighting the picture with is the line above's answer.
    scene.shade_markers();
    // The ground the sevens knockout clears to. Only the shell knows what
    // this pass is composited over -- the fill the docked pane just painted
    // here, the render layout's own background offline -- so it is carried in
    // by the caller rather than assumed by the scene.
    scene.background = background;

    // Picking updates the *shared* hover state, one frame behind the scene
    // it was derived from (imperceptible, standard for immediate mode). Only
    // the interactive copy has a pointer to read.
    if let Some(response) = response {
        if let Some(pointer) = response.hover_pos() {
            state.hovered = scene.pick(
                glam::Vec2::new(rect.width(), rect.height()),
                glam::Vec2::new(pointer.x - rect.min.x, pointer.y - rect.min.y),
                24.0,
            );
        } else if !response.dragged() {
            state.hovered = None;
        }
    }

    // The lattice's own place in the shape list, claimed before the labels are
    // laid out and filled in after. The names go INTO that callback — they are
    // drawn inside its scene pass, so that a node in front covers the name of
    // the node behind it — and they are not known until they have been drawn.
    // A label puts NOTHING on this painter, marks included, so the only thing
    // this slot orders is the learn badge that draws over the finished pane.
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
    //
    // Only the interactive copy shows it at all: the badge is chrome about
    // the working view being in learn mode, and the preview is a picture of
    // the render, not a place to work.
    let badge = (response.is_some() && state.learn_active).then(|| learn_badge(ui, rect, now));
    ui.painter().set(
        lattice,
        lattice_paint_callback(
            rect,
            &scene,
            batch.lattice_labels(ui.painter(), rect.min, state),
            state.target_format,
            pane_id(surface),
            stats,
        ),
    );
    if let Some(mut badge) = badge {
        draw_learn_overlay(ui, rect, state, now, &mut badge);
    }
}

/// The GPU pane id a live copy of a lattice paint callback claims, so two
/// live copies never overwrite each other's buffers within a frame — the
/// docked pane's own id and the Render preview's second copy.
///
/// One id space per callback type
/// (`lattice_paint_callback`, `harmonigraph_render::roll_paint_callback`),
/// not one shared across all of them: the lattice's 0/1 and the roll's do
/// not have to, and do not, mean the same live copy. `crate::text`'s
/// `spectral_labels` is a THIRD, unrelated space — a text batch's flush id,
/// not a GPU pane id — offset to leave room for the constants that space
/// hands out before it, `LATTICE_LEARN` and `SPIRAL_NAMES`.
pub(crate) fn pane_id(surface: usize) -> u64 {
    surface as u64
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
    // Pinned to the corner, so it slides along nothing; the filter's axis is
    // whichever, and the default is what says so.
    badge.flush(
        &painter,
        rect,
        state,
        crate::text::LATTICE_LEARN,
        harmonigraph_render::SlideAxis::default(),
    );
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

/// The ink every lattice label is drawn in — the note name, its marks and
/// the cents under it alike — and it is white rather than the skin's
/// [`text`](theme::text)/[`text_dim`](theme::text_dim) pair.
///
/// Type over the lattice is not chrome. The skin dresses the panels around
/// the picture; what stands ON the picture is the picture's own ink, and a
/// node's light is where its colour lives, not its name. Two ranks of grey
/// said something the label does not mean — the note the brighter, its cents
/// the fainter — where they are one label naming one node.
const LABEL_INK: egui::Color32 = egui::Color32::WHITE;

/// How readable a label on a node that is not sounding is: exactly as
/// readable as one that is.
///
/// A kept name and a sounding name carry the same ink, and what tells them
/// apart is the NODE — a sounding one is lit and a remembered one is not.
/// Dimming the type as well says it twice, and it costs the quieter half its
/// legibility rather than merely its rank, because alpha over the lattice's
/// dark ground is grey rather than a fainter white.
///
/// It is also the level the marker underneath already assumes.
/// [`name_level`](harmonigraph_scene::NodeInstance::name_level) is what the
/// resting marker cross-fades OUT on and it reaches 1.0 for a kept name;
/// short of that here, the marker left the position completely while the
/// name replacing it arrived at a fraction, so the handoff lost ink in the
/// middle. One level on both sides and the swap is even.
const RESTING_LABEL_STRENGTH: f32 = 1.0;

/// How readable one node's label is, 0..1. `names` is which nodes the view
/// names at all (see [`draw_node_labels`]), and this is where each of the
/// three answers turns into a level.
///
/// A sounding label rides its note's envelope straight down to nothing. The
/// one exception is a name this view is about to KEEP, which settles on
/// `RESTING_LABEL_STRENGTH` instead of fading out: `node.trail` is the recorded
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
/// the name at full brightness the instant a key goes down, over a node still
/// at a fraction of it, which is the same "steady, then switched" the reserve
/// exists to remove, mirrored.
///
/// Reserved only where a trail can actually land, which is the home sheet:
/// off-sheet nodes are deliberately never marked (a lone memory floating out
/// in the sevens dimension reads as noise — see `harmonigraph_scene::trail`), so
/// reserving there held the label at full brightness through the whole
/// release and then dropped it to nothing at prune. Fading to a level and
/// vanishing from it is exactly what a visibility floor looks like, and this
/// was the last one left.
///
/// Reserved under [`NoteNames::Past`] alone, which is the only mode a record
/// is ever coming under. [`All`](NoteNames::All) needs none — every node
/// already draws at the kept level, so there is nothing to reserve up to —
/// and under [`Played`](NoteNames::Played) nothing is kept, so a name held
/// short of zero would hold there for good.
fn label_strength(node: &harmonigraph_scene::NodeInstance, names: NoteNames) -> f32 {
    if node.hovered {
        return 1.0;
    }
    // What a name that is not sounding draws at: every node under All, a
    // visited one under Past, and nothing at all under Played.
    let resting = match names {
        NoteNames::All => RESTING_LABEL_STRENGTH,
        NoteNames::Past => RESTING_LABEL_STRENGTH * node.trail,
        NoteNames::Played => 0.0,
    };
    let keeps_past = names == NoteNames::Past;
    let reserved = if keeps_past && node.on_home && node.departing && node.activation > 0.0 {
        RESTING_LABEL_STRENGTH
    } else {
        0.0
    };
    node.activation.max(resting).max(reserved)
}

/// Labels on hovered and sounding nodes, plus whatever else the Show row
/// names -- every visited node under [`NoteNames::Past`], every node on
/// screen under [`All`](NoteNames::All) -- projected with the same camera as
/// the nodes: the note name centered on the node, optionally its pitch class
/// in cents just below.
///
/// Collected here and drawn by the lattice's own callback, inside its scene
/// pass, each name at its node's place in the back-to-front order — so a node
/// in front covers the name of the node behind it. The batch carries which
/// node each glyph belongs to (`TextBatch::attached_to`); the caller hands the
/// finished batch to `lattice_paint_callback`.
///
/// The DRAWN marks go the same way and are collected into the same runs: an
/// accidental or a comma sign is a glyph of a sheet of its own
/// (`crate::text::MarkAtlas`), so what covers a name covers its marks with it.
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
    // Past IS the trail: it is what populates `node.trail` (see
    // `TrailField::build`) as well as what draws off it. Under either other
    // mode the field never fills, so a fading name has nothing to settle onto
    // and eases all the way out.
    let names = view.note_names;
    for (index, node) in scene.nodes.iter().enumerate() {
        // Whether this node is named at all — asked of the node rather than
        // spelled out here, because the resting MARKER under it turns on the same
        // answer and the two have to be one rule (see
        // `NodeInstance::is_named`). What is left to this pass is where the
        // name lands and what it says.
        if !node.is_named(view) {
            continue;
        }
        let Some(p) = projector.project(node.world_pos) else {
            continue;
        };
        let strength = label_strength(node, names);
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
                    LABEL_INK.gamma_multiply(strength),
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
                        LABEL_INK.gamma_multiply(strength),
                        outline,
                        scale,
                        magnify,
                        // A node's label sits ON its node, so the node is the
                        // middle of it.
                        crate::marks::NameLead::Centred,
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
                        LABEL_INK.gamma_multiply(strength),
                        outline,
                    );
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::probe::{fresh, frame_full, painted_full, painted_into, themed};
    use harmonigraph_core::NoteEvent;

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
        let mut state = fresh();
        state.camera.distance = distance;
        // No arrival ramp: the scene below is derived 50ms in, a fraction of
        // any real Fade, and a label's alpha rides its node's activation.
        // This suite is about where a label is DRAWN, not how lit it is.
        state.frame_params.fade_time = 0.0;
        // A chord spread across the lattice, so nodes land all over the pane
        // and (zoomed in) well outside it.
        for note in [55u8, 60, 62, 64, 67, 69, 71] {
            state.tracker.handle_event(NoteEvent::on(0.0, 0, note, 1.0));
        }
        let scene = derive_scene(
            &state.tracker,
            &state.tuning,
            &state.view,
            &state.view.reach(),
            &state.frame_params,
            state.camera,
            None,
            0.05,
        );
        let mut batch = crate::text::TextBatch::default();
        let _ = painted_into(egui::vec2(1200.0, 900.0), rect, |ui| {
            draw_node_labels(ui, rect, &scene, &state.view, &mut batch);
        });
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
        let mut batch = crate::text::TextBatch::default();
        let _ = painted_full(egui::vec2(200.0, 100.0), |ui| {
            batch.text(
                ui.painter(),
                egui::pos2(100.0, 50.0),
                egui::Align2::CENTER_CENTER,
                "C440".to_owned(),
                egui::FontId::monospace(15.0),
                egui::Color32::WHITE,
                egui::Color32::BLACK,
            );
        });
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
    ///
    /// The size at the default framing is `NAME_SIZE` through the fresh view's
    /// own Name size, rather than the constant bare: the bar is a multiple of
    /// the built-in and a fresh view does not open at 1, so quoting the
    /// constant alone would pin the picture to a bar position nobody starts
    /// from. What the test is about is the RATIO the camera moves it by, and
    /// that is untouched by where the bar sits.
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
        // Within a rung of the ladder either way. The size follows the camera
        // continuously and is RASTERIZED at the nearest size on offer, which
        // is what keeps a zoom from asking egui for a new one every frame —
        // see `text::snap_scale`.
        let tracks = |distance: f32, want: f32| {
            let got = biggest(distance);
            // Off by at most a rung of the ladder, or half a pixel where that
            // is coarser — the two grains `snap_scale` quantizes on. The rung
            // is the coarser of the two at every size here but the smallest,
            // where a quarter of the dialled name is a few pixels on this 1x
            // context and half a pixel is the wider tolerance of the pair.
            let slack = (0.04 * want).max(0.5);
            assert!(
                (got - want).abs() <= slack,
                "at distance {distance} a name drew at {got}, not within {slack} of {want}",
            );
        };
        let dialled = NAME_SIZE * harmonigraph_scene::ViewConfig::default().label_scale;
        // The default framing is where the sizes are dialled: the camera
        // contributes a factor of 1 there, so the letter is the built-in
        // through the bar and nothing else.
        //
        // To within a rung, like every other distance here, and NOT exactly.
        // `snap_scale` reproduces a dialled size exactly only at scale 1, where
        // its ladder is anchored; anywhere else it rounds the nearest rung onto
        // a whole physical pixel, and whether that lands back on the dialled
        // number is a property of the number. This one does (a rung of 1.3159
        // times a 30pt base is 39.478, which rounds to 39, which is 1.3 of the
        // base again) and 1.25 would not (37.96 rounds to 38 against a dialled
        // 37.5). Asserting the exact value would pass here and fail on the next
        // retune of the bar, blaming the camera for the pixel grid.
        tracks(Camera::DEFAULT_DISTANCE, dialled);
        tracks(Camera::DEFAULT_DISTANCE * 0.5, dialled * 2.0);
        tracks(Camera::DEFAULT_DISTANCE * 2.0, dialled * 0.5);
        // The factor-of-four rung is taken zoomed IN rather than out: the far
        // end of the zoom is `MAX_DISTANCE`, twice the default framing, and a
        // distance past it is not a smaller label but the same one — the scale
        // is read through the range the camera is navigable in.
        tracks(Camera::DEFAULT_DISTANCE * 0.25, dialled * 4.0);
        // And the ladder is really there: a walk of nudges each too small to
        // see costs a handful of sizes to rasterize rather than one apiece.
        //
        // Counted over a walk rather than asserted on a single nudge, and the
        // difference is the rung BOUNDARIES: two sizes 1% apart are usually
        // one size and are two whenever the pair straddles a boundary, so a
        // single nudge is a test of where the fresh view's Name size happens
        // to sit on the ladder. What is actually promised is that the count of
        // sizes follows the rungs crossed and not the frames drawn — and 24
        // steps of 1% is a quarter more distance, which is 6 rungs of a 4%
        // ladder however the bar is set.
        //
        // The bound has one size of slack and it is worth knowing how little:
        // the walk asks for 7. Strip the rung out of `snap_scale` and leave the
        // pixel rounding alone and it asks for 9, which is what this catches;
        // strip both and it asks for 25. So the margin between the promise and
        // the degradation is a single size, and it is that tight because of the
        // base and the ppp this fixture uses, not because 8 was picked loosely.
        let mut sizes: Vec<f32> = (0..25)
            .map(|step| biggest(Camera::DEFAULT_DISTANCE * 1.01f32.powi(step)))
            .collect();
        sizes.sort_by(f32::total_cmp);
        sizes.dedup();
        assert!(
            sizes.len() <= 8,
            "25 camera nudges of 1% asked for {} sizes: {sizes:?}",
            sizes.len(),
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
            // The lattice pass draws the ring on every node it ships; the
            // gate is the fold's answer and there is no fold here.
            audio_ring: 1.0,
            ring_peak: 1.0,
            // Nothing here draws a glow, and the labels this fixture is for do
            // not read one: an unlit light on the first row.
            glow: harmonigraph_scene::GlowStep::default(),
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
        for names in [NoteNames::Played, NoteNames::Past] {
            assert_eq!(label_strength(&fading(0.2, false), names), 0.2);
            assert_eq!(label_strength(&fading(0.02, false), names), 0.02);
            assert_eq!(label_strength(&fading(0.0, false), names), 0.0);
        }
        // On the home sheet with the past kept, it settles on the level the
        // record will hold it at rather than easing out and popping back. That
        // level being full, a name on its way to being kept never dims at all:
        // the release is the NODE's, and the name it carries is one the view is
        // about to keep saying.
        assert_eq!(label_strength(&fading(0.8, true), NoteNames::Past), RESTING_LABEL_STRENGTH);
        assert_eq!(label_strength(&fading(0.2, true), NoteNames::Past), RESTING_LABEL_STRENGTH);
        // ...and with only the played notes named, the home sheet fades out
        // like anything else.
        assert_eq!(label_strength(&fading(0.2, true), NoteNames::Played), 0.2);
        // A silent node reserves nothing at all, wherever it sits: the
        // reserve is for a name on its way to being kept, not for every node
        // the view holds.
        assert_eq!(label_strength(&fading(0.0, true), NoteNames::Past), 0.0);
        // A hover is always fully readable, mid-fade or not.
        let mut hovered = fading(0.05, false);
        hovered.hovered = true;
        assert_eq!(label_strength(&hovered, NoteNames::Played), 1.0);
        // Once the name IS recorded, it reads at the kept level.
        let mut kept = fading(0.0, true);
        kept.trail = 1.0;
        assert_eq!(label_strength(&kept, NoteNames::Past), RESTING_LABEL_STRENGTH);
    }

    /// Naming every node is a floor under the whole lattice, not a memory:
    /// silence reads at the kept level on a node that has never sounded and
    /// on one off the home sheet alike.
    ///
    /// A sounding node does NOT outshine the field it sits in, and that is
    /// the point rather than a gap in the mode. Its LIGHT does that; the type
    /// over it says what the node is called, and a name dimmed to mean "not
    /// this one" was a second answer to a question the lattice had already
    /// answered — paid for in the legibility of every name that was not
    /// currently playing.
    #[test]
    fn naming_everything_puts_every_node_at_the_kept_level() {
        for on_home in [false, true] {
            for activation in [0.0, 0.8] {
                assert_eq!(
                    label_strength(&fading(activation, on_home), NoteNames::All),
                    RESTING_LABEL_STRENGTH,
                );
            }
        }
    }

    /// A name ARRIVES on its own note's envelope, like every other layer.
    ///
    /// The reserve is a departure device and its argument only holds there:
    /// it stands in for a record that `node.trail` does not carry until the
    /// frame the release finishes. A note on its way IN has no record coming
    /// and nothing to settle onto, so reserving there pins the name at full
    /// brightness over a node still at a fraction of it — the same "holding
    /// steady and then switching" the reserve exists to remove, at the other
    /// end of the note.
    ///
    /// The band `0 < activation < RESTING_LABEL_STRENGTH` is the whole of a
    /// note's climb, and it is climbed on every note-on: at the fresh view —
    /// the trail's kept names on — that is every lit node.
    #[test]
    fn a_name_arriving_is_no_brighter_than_the_note_it_names() {
        let mut state = fresh();
        // A long arrival, so the climb through the reserve's band is a stretch
        // to sample rather than a frame of it.
        state.frame_params.fade_time = 1.0;
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        let scene = derive_scene(
            &state.tracker,
            &state.tuning,
            &state.view,
            &state.view.reach(),
            &state.frame_params,
            state.camera,
            None,
            0.05,
        );
        let names = state.view.note_names;
        assert_eq!(
            names,
            NoteNames::Past,
            "the fresh view keeps the past; without that this proves nothing",
        );
        let node = scene.nodes.iter().find(|n| n.activation > 0.0).expect("the note lit a node");
        assert!(node.on_home, "the lit node is off the home sheet, where nothing is reserved");
        assert!(
            node.activation < RESTING_LABEL_STRENGTH,
            "sampled past the reserve's band at {}, so this cannot see the plateau",
            node.activation,
        );
        assert_eq!(
            label_strength(node, names),
            node.activation,
            "an arriving name was drawn at the trail reserve, not at its note's own level",
        );

        // The other end, through the same derive rather than a hand-built
        // node: the key comes up once the arrival has landed, and at the same
        // depth into the departure the reserve DOES hold the name up. Without
        // this half, a fix that simply never reserved would pass the half
        // above.
        state.tracker.handle_event(NoteEvent::off(1.0, 0, 60));
        let scene = derive_scene(
            &state.tracker,
            &state.tuning,
            &state.view,
            &state.view.reach(),
            &state.frame_params,
            state.camera,
            None,
            1.9,
        );
        let node = scene.nodes.iter().find(|n| n.activation > 0.0).expect("the note still lights");
        assert!(node.departing, "the key is up and the arrival landed, so this is a departure");
        assert!(
            node.activation < RESTING_LABEL_STRENGTH,
            "sampled at {}, above the reserve, so this cannot see it hold",
            node.activation,
        );
        assert_eq!(
            label_strength(node, names),
            RESTING_LABEL_STRENGTH,
            "a departing name stopped reserving the level its trail record takes over at",
        );
    }

    /// The level a name is DRAWN at and the level a marker cross-fades out on
    /// are one rule, and a departure is where they can come apart: the reserve
    /// that holds a departing name up to what its record takes over at lives in
    /// `label_strength`, while the marker's opacity is the complement of
    /// [`name_level`](harmonigraph_scene::NodeInstance::name_level), which is a
    /// second spelling of the same rule. A position carries one mark; two
    /// spellings is the one way it can carry two.
    ///
    /// Measured through the same derive as the reserve's own test rather than
    /// off a hand-built node, because the disagreement needs a state only a
    /// release reaches — on the home sheet, under `Past`, with the trail not
    /// written until the frame the release ends. Sampling before or after that
    /// window is what leaves the pair looking equal.
    #[test]
    fn a_departing_name_and_the_marker_under_it_are_one_rule() {
        let mut state = fresh();
        state.frame_params.fade_time = 1.0;
        state.tracker.handle_event(NoteEvent::on(0.0, 0, 60, 1.0));
        state.tracker.handle_event(NoteEvent::off(1.0, 0, 60));
        let scene = derive_scene(
            &state.tracker,
            &state.tuning,
            &state.view,
            &state.view.reach(),
            &state.frame_params,
            state.camera,
            None,
            1.9,
        );
        let names = state.view.note_names;
        assert_eq!(names, NoteNames::Past, "the reserve is Past's alone");
        let node = scene.nodes.iter().find(|n| n.activation > 0.0).expect("the note still lights");
        assert!(node.departing && node.on_home, "the reserve wants a departure on the home sheet");
        assert_eq!(node.trail, 0.0, "the record is not written until the release ends");
        assert_eq!(
            node.name_level(&state.view),
            label_strength(node, names),
            "the marker reads a departing name at {} while the pass draws it at {}",
            node.name_level(&state.view),
            label_strength(node, names),
        );
        // The half a level cannot say on its own: a name drawn whole leaves NO
        // marker under it, rather than one shipped just short of full.
        let standing = scene
            .pluses
            .iter()
            .find(|marker| marker.pos == node.world_pos)
            .map_or(0.0, |marker| marker.strength);
        assert_eq!(
            standing, 0.0,
            "a marker stood at {standing} under a name drawn at {}",
            label_strength(node, names),
        );
    }

    /// The learn badge is chrome about the WORKING view, not the picture —
    /// see [`lattice_pane`]'s own doc comment — so only the interactive copy
    /// draws it. [`draw_lattice`] gates it on `response` alone: the Render
    /// preview's second live copy has none, and must show none of it.
    #[test]
    fn only_the_interactive_copy_draws_the_learn_badge() {
        let mut state = fresh();
        state.learn_active = true;
        state.view.show_labels = false;
        let ctx = themed();
        let screen = egui::vec2(400.0, 400.0);
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 300.0));

        let without = frame_full(&ctx, screen, |ui| {
            draw_lattice(ui, rect, &mut state, 0.0, 0, glam::Vec4::ZERO, None, None);
        })
        .shapes
        .len();
        let with = frame_full(&ctx, screen, |ui| {
            let (_, response) = ui.allocate_exact_size(rect.size(), egui::Sense::hover());
            draw_lattice(ui, rect, &mut state, 0.0, 0, glam::Vec4::ZERO, Some(&response), None);
        })
        .shapes
        .len();
        assert!(
            with > without,
            "learn mode should draw the badge for the interactive copy ({with} shapes) but not \
             the non-interactive one ({without} shapes)",
        );
    }

    /// Only the interactive copy publishes its window, for the same reason it
    /// alone reports its node count: the Video tab's preview is a second
    /// lattice at the RENDER's aspect, so letting it publish would answer
    /// "what is the picture showing" with a picture the reader is not looking
    /// at — the analyzer's off-lattice band jumping with a tab beside it.
    #[test]
    fn only_the_interactive_copy_publishes_its_window() {
        let mut state = fresh();
        let ctx = themed();
        let screen = egui::vec2(400.0, 400.0);
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 300.0));

        let _ = frame_full(&ctx, screen, |ui| {
            draw_lattice(ui, rect, &mut state, 0.0, 1, glam::Vec4::ZERO, None, None);
        });
        assert_eq!(state.drawn_this_frame, None, "the preview published a window");

        let _ = frame_full(&ctx, screen, |ui| {
            let (_, response) = ui.allocate_exact_size(rect.size(), egui::Sense::hover());
            draw_lattice(ui, rect, &mut state, 0.0, 0, glam::Vec4::ZERO, Some(&response), None);
        });
        assert_eq!(
            state.drawn_this_frame,
            Some(state.view.scrolled(&state.camera, 1.0)),
            "the docked copy published something other than the window it drew",
        );
    }

    /// Picking only touches `state.hovered` for the interactive copy: the
    /// preview has no pointer of its own (see [`lattice_pane`]'s own doc
    /// comment), so a non-interactive [`draw_lattice`] call must leave
    /// whatever the docked pane last picked alone rather than clearing it
    /// out from under it.
    #[test]
    fn only_the_interactive_copy_lets_picking_touch_the_hover() {
        let mut state = fresh();
        let home = harmonigraph_core::LatticePos::new(0, 0, 0);
        state.hovered = Some(home);
        let ctx = themed();
        let screen = egui::vec2(400.0, 400.0);
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 300.0));

        // No response: nothing picks, so the docked pane's last hover must
        // survive a preview frame drawn in between.
        let _ = frame_full(&ctx, screen, |ui| {
            draw_lattice(ui, rect, &mut state, 0.0, 0, glam::Vec4::ZERO, None, None);
        });
        assert_eq!(state.hovered, Some(home), "a non-interactive copy must not touch the hover");

        // A response with no simulated pointer over it: picking runs and
        // reads "not hovering, not dragging", which clears it.
        let _ = frame_full(&ctx, screen, |ui| {
            let (_, response) = ui.allocate_exact_size(rect.size(), egui::Sense::hover());
            draw_lattice(ui, rect, &mut state, 0.0, 0, glam::Vec4::ZERO, Some(&response), None);
        });
        assert_eq!(state.hovered, None, "the interactive copy should pick, and clear a stale hover");
    }
}
