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
use crate::widgets::{button_row, toggle_switch, ValueBar};
use crate::{theme, Layout, Pane, SharedState};

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
    render_settings(ui, state);
    frame_controls(ui, state);

    section(ui, "Preview");
    let frame = state.render_config.frame;
    let avail = ui.available_size();
    if avail.x < 20.0 || avail.y < 20.0 {
        return;
    }
    let (outer, _) = ui.allocate_exact_size(avail, Sense::hover());
    let aspect = frame.aspect_w.max(1) as f32 / frame.aspect_h.max(1) as f32;
    let box_rect = letterbox(outer, aspect);
    // How far the preview shrinks the render frame — labels scale by this so
    // they read at the size they will in the render, not at full point size.
    let label_scale = box_rect.width() / RENDER_POINTS_ACROSS;

    // Compose with the SAME Layout the offline renderer resolves.
    let layout = Layout::split(frame.stacked, frame.split);

    // Make the render frame obvious against the pane. The letterbox padding
    // takes the panel color, so it reads as inert chrome rather than part of
    // the shot; the aspect box takes the render's OWN frame background — the
    // color the offline renderer shows in its margins and inter-pane gaps — so
    // the box is exactly the pixels the video will contain. Before this the
    // padding and the pane fills were both `well()`, and you couldn't tell
    // where the frame ended. A hairline edge keeps the boundary crisp even
    // when a pane fills the box edge to edge.
    let bg = layout.background;
    ui.painter().rect_filled(outer, 0.0, theme::panel());
    ui.painter().rect_filled(box_rect, 0.0, egui::Color32::from_rgb(bg.0, bg.1, bg.2));
    ui.painter().rect_stroke(
        box_rect,
        0,
        egui::Stroke::new(1.0, theme::accent_edge()),
        egui::StrokeKind::Inside,
    );

    for (pane, rect) in layout.resolve(box_rect.size()) {
        let rect = rect.translate(box_rect.min.to_vec2());
        match pane {
            Pane::Spectral => {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                // Directly, so the preview's shrink scales its text too — the
                // one fixed-size thing draw_pane can't carry. Texture slot 1, so
                // its spectrogram doesn't clobber the docked pane's (slot 0).
                super::spectral::spectral_pane(&mut child, state, now, label_scale, 1);
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
    // scaled down so they read at render size in the shrunken preview. Clipped
    // to the lattice rect: a node near the frame edge would otherwise paint its
    // label out past the preview box, since draw_node_labels uses an unclipped
    // painter (harmless in the docked pane, which owns its whole rect and is
    // clipped by the dock; here the rect is only a sub-region of the pane).
    if state.view.show_labels {
        let mut clipped = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        clipped.set_clip_rect(rect);
        super::lattice::draw_node_labels(&clipped, rect, &scene, &state.view, label_scale);
    }
}

/// Recording and render-output settings. Lives here, in the Render tab, so that
/// tab is the one home for everything about turning a take into a video.
fn render_settings(ui: &mut egui::Ui, state: &mut SharedState) {
    // Take recording: the input half of offline video rendering. A mode with
    // ongoing side effects (it keeps writing a file), so a switch rather than a
    // checkbox — the house rule in widgets.rs.
    if !state.take_supported {
        return;
    }
    section(ui, "Record");
    toggle_switch(ui, &mut state.take_recording, "Record take").on_hover_text(
        "Record the performance — notes, bends, parameter automation, the \
         current look, and the plugin's audio input — to a .take file. Switch \
         it off and the take renders to video automatically: the recorded audio \
         becomes the spectrogram and a playhead sweeps the whole piece. Events \
         are stamped with transport position, so nothing is captured until the \
         transport rolls.",
    );
    if !state.take_status.is_empty() {
        ui.weak(&state.take_status);
    }

    // Resolution and any other lattice-offline flags, split on spaces. The
    // frame's aspect already picks a default resolution, so this is only for
    // going bigger (e.g. 4K) or the occasional override.
    let render = &mut state.render_config;
    labeled_path(ui, "Options", &mut render.extra_args).on_hover_text(
        "Extra lattice-offline flags, split on spaces — resolution and the \
         like: --size 3840x2160",
    );

    // Re-render the last take with the frame you've dialed in since recording.
    // The take carries only a record-time snapshot, so this is how a reframed
    // preview reaches the video without recording again.
    if state.last_take_ready {
        ui.add_space(2.0);
        if ui
            .button("Render now")
            .on_hover_text(
                "Render the take you last recorded again, now, with the current \
                 frame. Runs in the background; the video lands next to the take.",
            )
            .clicked()
        {
            state.render_now = true;
        }
    }
}

/// A labeled single-line text field that fills the pane width — the shape every
/// path setting in the render settings uses.
fn labeled_path(ui: &mut egui::Ui, label: &str, value: &mut String) -> egui::Response {
    button_row(ui, |ui| {
        ui.label(label);
        ui.add(egui::TextEdit::singleline(value).desired_width(ui.available_width()))
    })
}
