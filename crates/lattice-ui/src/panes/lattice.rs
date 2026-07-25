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
            SevensLabel::Name | SevensLabel::Comma => {
                let name = display_note_name(node.lattice_pos, view.meantone);
                let bottom = draw_stacked_name(
                    batch,
                    ui.painter(),
                    center,
                    name,
                    theme::text().gamma_multiply(strength),
                    outline,
                    scale,
                );
                if sevens == SevensLabel::Comma {
                    // The signed distance to the home-sheet node wearing
                    // this very name — the septimal comma, and the only
                    // part of the label that differs between sheets.
                    draw_plain_name(
                        batch,
                        ui.painter(),
                        center + egui::vec2(0.0, bottom + CENTS_GAP * scale),
                        &format!("{:+.0}", node.comma),
                        theme::armed().gamma_multiply(strength),
                        outline,
                        scale * 0.72,
                    ) + bottom
                        + CENTS_GAP * scale
                } else {
                    bottom
                }
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

/// A note name centered on `anchor`, with its accidental stacked above its
/// syntonic-comma mark in a single column after the letter (`♯` riding high
/// like a superscript, `+` low like a subscript). Both marks are counted
/// rather than repeated (see [`lattice_core::NoteName`]), so even a name
/// deep in the lattice stays roughly two characters wide instead of
/// sprawling off its node.
///
/// Returns how far the lowest glyph drawn reaches below `anchor.y` -- the
/// name's ink, not its box -- which is what the cents readout hangs off.
///
/// Monospace for in-lattice text: labels align across nodes and match the
/// technical feel of the readouts.
pub(crate) fn draw_stacked_name(
    batch: &mut crate::text::TextBatch,
    painter: &egui::Painter,
    anchor: egui::Pos2,
    name: lattice_core::NoteName,
    color: egui::Color32,
    outline: egui::Color32,
    scale: f32,
) -> f32 {
    let name_font = egui::FontId::monospace(NAME_SIZE * scale);
    let mark_font = egui::FontId::monospace(MARK_SIZE * scale);
    let measure = |text: &str, font: &egui::FontId| {
        painter.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER).size()
    };
    // Each piece is drawn centered on its own line, so its ink reaches this
    // far below that line.
    let ink_below = |text: &str, font: &egui::FontId, size: egui::Vec2| {
        painter_ink(painter, text, font).max.y - size.y / 2.0
    };

    let accidental = (name.accidental_mark(), -1.0);
    let comma = (name.comma_mark(), 1.0);
    let letter = measure(&name.letter.to_string(), &name_font);
    let mark_size = |(text, _): &(String, f32)| measure(text, &mark_font);
    // The two marks share one column, so it is as wide as the wider of them
    // -- and zero wide for a plain name, which then centers as before.
    let column = mark_size(&accidental).x.max(mark_size(&comma).x);
    let left = anchor.x - (letter.x + column) / 2.0;

    let letter_text = name.letter.to_string();
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
    for mark in [&accidental, &comma] {
        let (text, direction) = mark;
        if text.is_empty() {
            continue;
        }
        // Push each mark out until its own outer edge is flush with the
        // letter's, which is as far as it can go without standing proud of
        // the name. That the pair then meets near the middle is what makes
        // it read as a super/subscript stack rather than two loose glyphs.
        let size = mark_size(mark);
        let rise = (letter.y - size.y) / 2.0;
        batch.text(
            painter,
            egui::pos2(left + letter.x, anchor.y + direction * rise),
            egui::Align2::LEFT_CENTER,
            text.clone(),
            mark_font.clone(),
            color,
            outline,
        );
        // The comma hangs below the letter's baseline, so it -- not the
        // letter -- is what the cents readout has to clear.
        bottom = bottom.max(direction * rise + ink_below(text, &mark_font, size));
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
    /// This is the whole point of drawing labels ourselves: the rim used to
    /// multiply a label's geometry by twenty-one, so every new label was a
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
