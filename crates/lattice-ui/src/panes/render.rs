//! The Render pane: a live, aspect-locked preview of the offline video frame.
//!
//! It composes the Lattice and Spectral panes through the same
//! [`Layout`](crate::Layout) the offline renderer uses, at a chosen aspect
//! ratio and split — so what you frame here is what the render produces. The
//! frame settings live in [`RenderFrame`](crate::RenderFrame), persisted with
//! the take, so `lattice-offline` reproduces exactly this composition.
//!
//! The preview's lattice is a *second* live lattice view. It must not go
//! through [`draw_pane`](crate::draw_pane) (which hardcodes GPU pane id 0 and
//! would fight the docked Lattice tab); it renders directly with its own id.
//! You frame the camera in the Lattice tab and watch it land here at the
//! render's aspect — which is exactly what decides "how much of the lattice is
//! exposed" (a wider frame shows more horizontally).

use egui::Sense;
use lattice_render::lattice_paint_callback;
use lattice_scene::derive_scene;

use super::section;
use crate::widgets::ValueBar;
use crate::{draw_pane, theme, Layout, Pane, SharedState};

/// The preview's lattice is a second live view, so it needs its own GPU id —
/// the docked Lattice tab owns 0, and two views sharing an id overwrite each
/// other's buffers within a frame.
const PREVIEW_PANE_ID: u64 = 1;

/// The offline renderer composes its frame at about this many points across
/// (its default pixels-per-point reference in `lattice-offline`). A preview
/// `box_rect.width()` points wide is therefore that frame scaled by
/// `width / this`, and fixed-point-size labels have to scale with it or they
/// swamp a small preview.
const RENDER_POINTS_ACROSS: f32 = 1280.0;

/// Frame controls, then a live preview of exactly what the offline render will
/// compose.
pub(crate) fn render_pane(ui: &mut egui::Ui, state: &mut SharedState, now: f64) {
    frame_controls(ui, state);

    section(ui, "Preview");
    let frame = state.render_config.frame;
    let avail = ui.available_size();
    if avail.x < 20.0 || avail.y < 20.0 {
        return;
    }
    let (outer, _) = ui.allocate_exact_size(avail, Sense::hover());
    // The frame background fills the pane; the aspect box sits centered in it,
    // so the letterboxing reads exactly as the render's own margins will.
    ui.painter().rect_filled(outer, 0.0, theme::well());
    let aspect = frame.aspect_w.max(1) as f32 / frame.aspect_h.max(1) as f32;
    let box_rect = letterbox(outer, aspect);
    // How far the preview shrinks the render frame — labels scale by this so
    // they read at the size they will in the render, not at full point size.
    let label_scale = box_rect.width() / RENDER_POINTS_ACROSS;

    // Compose with the SAME Layout the offline renderer resolves.
    let layout = Layout::split(frame.stacked, frame.split);
    for (pane, rect) in layout.resolve(box_rect.size()) {
        let rect = rect.translate(box_rect.min.to_vec2());
        match pane {
            Pane::Spectral => {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                draw_pane(&mut child, Pane::Spectral, state, now);
            }
            Pane::Lattice => preview_lattice(ui, rect, state, now, label_scale),
        }
    }
}

/// Aspect ratio, arrangement, and split — editing the persisted `RenderFrame`.
fn frame_controls(ui: &mut egui::Ui, state: &mut SharedState) {
    section(ui, "Frame");
    let f = &mut state.render_config.frame;
    ui.horizontal(|ui| {
        ui.label("Aspect");
        for (w, h) in [(16u32, 9u32), (9, 16), (1, 1), (4, 5), (21, 9)] {
            let on = f.aspect_w == w && f.aspect_h == h;
            if ui.selectable_label(on, format!("{w}:{h}")).clicked() {
                f.aspect_w = w;
                f.aspect_h = h;
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label("Arrange");
        ui.selectable_value(&mut f.stacked, false, "Side by side");
        ui.selectable_value(&mut f.stacked, true, "Stacked");
    });
    let label = if f.stacked { "Lattice height" } else { "Lattice width" };
    ValueBar::new(&mut f.split, 0.15..=0.85, label).show(ui);
}

/// The largest sub-rect of `outer` with the given width:height aspect, centered
/// — the render frame letterboxed inside the pane.
fn letterbox(outer: egui::Rect, aspect: f32) -> egui::Rect {
    let (ow, oh) = (outer.width(), outer.height());
    let (w, h) =
        if ow / oh.max(1.0) > aspect { (oh * aspect, oh) } else { (ow, ow / aspect.max(0.01)) };
    egui::Rect::from_center_size(outer.center(), egui::vec2(w, h))
}

/// A second live lattice view at the preview rect's aspect. Aspect is taken
/// from `rect` inside the render callback, so this frames exactly as the render
/// will. Non-interactive: `hovered` is left `None`, and the camera is framed in
/// the Lattice tab (shared state), not here.
fn preview_lattice(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    state: &SharedState,
    now: f64,
    label_scale: f32,
) {
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    let scene = derive_scene(
        &state.tracker,
        &state.tuning,
        &state.view,
        &state.frame_params,
        state.camera,
        None,
        now,
    );
    ui.painter().add(lattice_paint_callback(rect, &scene, state.target_format, PREVIEW_PANE_ID));
    // Node names/cents, exactly as the Lattice pane and the render draw them —
    // scaled down so they read at render size in the shrunken preview.
    if state.view.show_labels {
        super::lattice::draw_node_labels(ui, rect, &scene, &state.view, label_scale);
    }
}
