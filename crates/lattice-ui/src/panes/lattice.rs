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

    if state.learn_active {
        draw_learn_overlay(ui, rect, now);
    }
    if state.view.show_labels {
        draw_node_labels(ui, rect, &scene, &state.view, 1.0);
    }
}

/// Learn mode is armed: show it ON the lattice too, so the mode is obvious
/// even when the Tuning tab (and its Learn toggle) is hidden.
fn draw_learn_overlay(ui: &egui::Ui, rect: egui::Rect, now: f64) {
    let color = theme::armed().gamma_multiply(learn_pulse(now));
    let painter = ui.painter_at(rect);
    painter.rect_stroke(
        rect.shrink(1.5),
        0,
        egui::Stroke::new(2.0, color),
        egui::StrokeKind::Inside,
    );
    outlined_text(
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

/// Labels on hovered, sounding, and -- with the trail's "Keep note names"
/// on -- already-visited nodes, drawn as egui text over the 3D view
/// (projected with the same camera as the nodes): the note name centered on
/// the node, optionally its pitch class in cents just below.
pub(super) fn draw_node_labels(
    ui: &egui::Ui,
    rect: egui::Rect,
    scene: &lattice_scene::Scene,
    view: &lattice_scene::ViewConfig,
    scale: f32,
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
        // Fade with the activation envelope; hovered nodes get a full,
        // steady label regardless.
        let strength = if node.hovered {
            1.0
        } else {
            let sounding = node.activation;
            // The level a kept name settles on. `node.trail` is the recorded
            // memory, but it is only written the frame the release finishes —
            // during the fade it's still zero. So for a note still sounding
            // under "Keep note names", reserve its resting level NOW: the
            // fading `sounding` term then lands on the same value the recorded
            // trail takes over at, instead of easing to zero and the trail
            // popping back a frame later. That pop was the "flash back in"
            // that made one label read as two.
            let recorded = if trailed { TRAIL_LABEL_STRENGTH * node.trail } else { 0.0 };
            let reserved =
                if keeps_names && node.activation > 0.0 { TRAIL_LABEL_STRENGTH } else { 0.0 };
            sounding.max(recorded).max(reserved)
        };
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
            outlined_text(
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
    outlined_text(
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
        outlined_text(
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
    outlined_text(
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

/// The halo's two rings, as (radius in points, stamp alpha, samples).
///
/// The sample counts are a cost, not a look: every stamp is the whole label
/// again, and a lattice full of labels is most of the geometry in the frame.
/// Both rings were 16 and neither needed it —
///
///   - the crisp ring is opaque, so its samples only have to close the gap:
///     at a 1.2pt radius, 12 land half a point apart and the union reads as
///     one line (8 starts to scallop, 4 visibly thins on the diagonals);
///   - the soft ring is a fade, and a fade is made of overlap. Halving its
///     samples to 8 thins it, so its stamp alpha rises to compensate: 0.21
///     against the old 0.15, tuned by rendering the pair and matching pixels
///     rather than by the compositing arithmetic, which assumes an overlap
///     count that varies across the rim.
///
/// 20 stamps against 32, for a rim that measures within a couple of 8-bit
/// levels of the one it replaces.
const RINGS: [(f32, f32, usize); 2] = [(2.0, 0.21, 8), (1.2, 1.0, 12)];

/// Text drawn over a busy picture, haloed so it stays readable whatever
/// ends up behind it (bright nodes, edges, glow; the Spectral pane's axis
/// labels use it over the spectrogram). The outline color
/// should be the skin's recessed surface (`theme::well`), which
/// contrasts with its text color by construction.
///
/// The halo is the galley stamped around two rings: a tight opaque ring
/// for contrast and a wider faint one that fades the edge out. Every
/// sample sits at the same radius, snapped to whole physical pixels —
/// mixed cardinal/diagonal offsets and sub-pixel radii both read as a
/// lumpy outline on high-DPI displays.
// One more than clippy's taste, and they are all distinct: the painter, where
// the text goes, what it says, and how it is dressed. Bundling any of them
// would name a group that does not exist.
#[allow(clippy::too_many_arguments)]
pub(super) fn outlined_text(
    painter: &egui::Painter,
    anchor: egui::Pos2,
    align: egui::Align2,
    text: String,
    font: egui::FontId,
    color: egui::Color32,
    outline: egui::Color32,
) {
    let galley = painter.layout_no_wrap(text, font, egui::Color32::PLACEHOLDER);
    let pos = align.anchor_size(anchor, galley.size()).min;
    let ppp = painter.ctx().pixels_per_point();
    let snap = |pt: f32| (pt * ppp).round().max(1.0) / ppp;
    // Soft ring first so the crisp ring and the fill paint over it.
    //
    // The sample counts are a cost, not a look: every stamp is the whole
    // label again, and a lattice full of labels is most of the geometry in
    // the frame. Both rings were 16 and neither needed it —
    //
    //   - the crisp ring is opaque, so its samples only have to close the
    //     gap: at a 1.2pt radius, 12 land half a point apart and the union
    //     reads as one line (8 starts to scallop, 4 visibly thins on the
    //     diagonals);
    //   - the soft ring is a fade, and a fade is made of overlap. Halving
    //     its samples to 8 thins it, so its stamp alpha rises to compensate:
    //     0.21 against the old 0.15, tuned by rendering the pair and
    //     matching pixels rather than by the compositing arithmetic, which
    //     assumes an overlap count that varies across the rim.
    //

    // A rim that cannot paint still costs its full ring of stamps, and a
    // label's rim is the single biggest thing this pane hands the
    // tessellator. So one that would paint nothing is skipped rather than
    // drawn in nothing: that is every label whose fade has taken it past the
    // last visible alpha, on every frame of every release.
    if outline.a() > 0 {
        for (radius, alpha, samples) in RINGS {
            let radius = snap(radius);
            let ring = outline.gamma_multiply(alpha);
            for i in 0..samples {
                let angle = std::f32::consts::TAU * i as f32 / samples as f32;
                let off = egui::vec2(angle.cos(), angle.sin()) * radius;
                painter.galley(pos + off, galley.clone(), ring);
            }
        }
    }
    painter.galley(pos, galley, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_core::{NoteEvent, NoteEventKind};

    /// Paint the Lattice pane into `rect` with the camera at `distance`, and
    /// report every galley the labels emitted.
    fn label_galleys(rect: egui::Rect, distance: f32) -> Vec<egui::epaint::TextShape> {
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
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 900.0));
        let out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                lattice_pane(&mut child, &mut state, 0.05);
            },
        );
        out.shapes
            .into_iter()
            .filter_map(|s| match s.shape {
                egui::Shape::Text(t) => Some(t),
                _ => None,
            })
            .collect()
    }

    /// The rim is drawn once per sample and no more, and its budget is 20.
    ///
    /// Both halves matter and neither is obvious from reading the loop: a
    /// stamp that slipped in twice would double the cost of every label in
    /// the frame invisibly, and the sample counts are the one number that
    /// decides what labels cost — 32 of them was the frame's largest single
    /// expense before they were tuned down.
    #[test]
    fn the_rim_draws_one_stamp_per_sample_and_no_more() {
        let samples: usize = RINGS.iter().map(|&(_, _, n)| n).sum();
        assert!(samples <= 20, "the rim's sample budget grew to {samples}");

        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 100.0));
        let out = ctx.run_ui(
            egui::RawInput { screen_rect: Some(screen), ..Default::default() },
            |ui| {
                outlined_text(
                    ui.painter(),
                    egui::pos2(100.0, 50.0),
                    egui::Align2::CENTER_CENTER,
                    "C".to_owned(),
                    egui::FontId::monospace(15.0),
                    egui::Color32::WHITE,
                    egui::Color32::BLACK,
                );
            },
        );
        let galleys = out
            .shapes
            .iter()
            .filter(|s| matches!(s.shape, egui::Shape::Text(_)))
            .count();
        assert_eq!(galleys, samples + 1, "one stamp per sample, plus the text itself");
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
        let wide = label_galleys(rect, 14.0);
        let zoomed = label_galleys(rect, 2.0);
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
        for shape in wide.iter().chain(&zoomed) {
            assert!(
                slack.contains(shape.pos),
                "a label was laid out at {:?}, outside the pane {rect:?}",
                shape.pos,
            );
        }
    }
}
