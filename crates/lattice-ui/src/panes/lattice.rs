//! The 3D lattice view pane: orbit camera on drag, zoom on scroll, node
//! labels, and the tuning-learn overlay.

use super::visibility_floor;
use crate::{theme, SharedState};
use super::learn_pulse;
use egui::Sense;
use lattice_render::lattice_paint_callback;
use lattice_scene::{derive_scene, Camera, Projection};

/// The 3D lattice view: orbit camera on drag, zoom on scroll, pick on hover.
pub(super) fn lattice_pane(ui: &mut egui::Ui, state: &mut SharedState, now: f64) {
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

    let scene = derive_scene(
        &state.tracker,
        &state.tuning,
        &state.view,
        &state.frame_params,
        state.camera,
        state.hovered,
        now,
    );

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
        .add(lattice_paint_callback(rect, &scene, state.target_format, 0));

    if state.learn_active {
        draw_learn_overlay(ui, rect, now);
    }
    if state.view.show_labels {
        draw_node_labels(ui, rect, &scene, &state.view);
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
fn draw_node_labels(
    ui: &egui::Ui,
    rect: egui::Rect,
    scene: &lattice_scene::Scene,
    view: &lattice_scene::ViewConfig,
) {
    let projector = scene.projector(glam::Vec2::new(rect.width(), rect.height()));
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
        // Fade with the activation envelope. visibility_floor keeps the
        // label readable through most of the release, but on its own it
        // bottoms out at 35% and then pops to nothing when the voice is
        // pruned at activation 0. Ease that last stretch out with a
        // smoothstep tail so the label fades gradually to zero instead.
        // Hovered nodes get a full, steady label.
        const LABEL_FADE_TAIL: f32 = 0.25;
        let strength = if node.hovered {
            1.0
        } else {
            let t = (node.activation / LABEL_FADE_TAIL).clamp(0.0, 1.0);
            let sounding = visibility_floor(node.activation) * t * t * (3.0 - 2.0 * t);
            // A note's label hands over to its memory's as it dies, so the
            // text never blinks at the moment the fade completes. The trail
            // level rides along, so a forgetting note dims out with it.
            sounding.max(if trailed { TRAIL_LABEL_STRENGTH * node.trail } else { 0.0 })
        };
        let center = egui::pos2(rect.min.x + p.x, rect.min.y + p.y);
        let outline = theme::well().gamma_multiply(strength);
        // Meantone tempers out the syntonic comma, so drop the comma marks
        // (E- and E name the same pitch).
        let name = node.lattice_pos.note_name();
        let name = if view.meantone { name.without_syntonic_commas() } else { name };
        // Monospace for in-lattice text: labels align across nodes and
        // match the technical feel of the readouts.
        outlined_text(
            ui.painter(),
            center,
            egui::Align2::CENTER_CENTER,
            name.to_string(),
            egui::FontId::monospace(12.0),
            theme::text().gamma_multiply(strength),
            outline,
        );
        if view.show_cents {
            outlined_text(
                ui.painter(),
                center + egui::vec2(0.0, 9.0),
                egui::Align2::CENTER_TOP,
                format!("{:.2}", node.cents),
                egui::FontId::monospace(10.0),
                theme::text_dim().gamma_multiply(strength),
                outline,
            );
        }
    }
}

/// Text drawn over the 3D view, haloed so it stays readable whatever
/// ends up behind it (bright nodes, edges, glow). The outline color
/// should be the skin's recessed surface (`theme::well`), which
/// contrasts with its text color by construction.
///
/// The halo is the galley stamped around two rings: a tight opaque ring
/// for contrast and a wider faint one that fades the edge out. Every
/// sample sits at the same radius, snapped to whole physical pixels —
/// mixed cardinal/diagonal offsets and sub-pixel radii both read as a
/// lumpy outline on high-DPI displays.
fn outlined_text(
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
    // Soft ring first so the crisp ring and fill paint over it. Stamp
    // alpha is well below the intended rim alpha because neighboring
    // stamps overlap and accumulate.
    for (radius, alpha) in [(snap(2.0), 0.15), (snap(1.2), 1.0)] {
        let ring = outline.gamma_multiply(alpha);
        for i in 0..16 {
            let angle = std::f32::consts::TAU * i as f32 / 16.0;
            let off = egui::vec2(angle.cos(), angle.sin()) * radius;
            painter.galley(pos + off, galley.clone(), ring);
        }
    }
    painter.galley(pos, galley, color);
}
