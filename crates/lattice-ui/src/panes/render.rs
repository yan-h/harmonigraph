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
    // scaled down so they read at render size in the shrunken preview.
    if state.view.show_labels {
        super::lattice::draw_node_labels(ui, rect, &scene, &state.view, label_scale);
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
        "Record everything the visualization is a function of — notes, bends, \
         parameter automation, and the current look — to a .take file. Render \
         it to video afterwards with lattice-offline, at any resolution and \
         frame rate. Events are stamped with transport position, so nothing is \
         captured until the transport rolls and the take lines up with a bounce \
         of the same song.",
    );
    if !state.take_status.is_empty() {
        ui.weak(&state.take_status);
    }

    let render = &mut state.render_config;
    ui.checkbox(&mut render.record_audio, "Record audio too").on_hover_text(
        "Write the plugin's audio input beside the take, so the render gets a \
         spectrum and a soundtrack with no separate bounce. Needs the device to \
         be somewhere audio actually reaches it — after the instrument, or on a \
         bus.",
    );
    ui.checkbox(&mut render.auto_render, "Render video when done").on_hover_text(
        "Run lattice-offline as soon as a take finishes, writing the video next \
         to the take. The render happens in the background — it does not hold up \
         the DAW.",
    );
    ui.checkbox(&mut render.playhead, "Whole-song playhead").on_hover_text(
        "Lay the whole take's spectrogram out at once and sweep a playhead \
         through it, instead of the live scrolling window. Needs audio. Applies \
         to every render of this take; the --playhead flag turns it on too.",
    );
    labeled_path(ui, "Bounced audio", &mut render.audio_path).on_hover_text(
        "A clean WAV bounce of this take to render with — muxed into the video \
         and analyzed for the spectrum. Paste its path here. Leave empty to use \
         the take's own recording, or to render silent.",
    );
    labeled_path(ui, "Audio offset (s)", &mut render.audio_offset).on_hover_text(
        "Take-time seconds where the bounce starts. Leave empty to auto-align to \
         the MIDI onsets; set a number if the auto-align drifts.",
    );
    if render.auto_render {
        crate::widgets::choice_row(
            ui,
            "When",
            &mut render.trigger,
            &[
                (
                    crate::RenderTrigger::OnDisarm,
                    "Switched off",
                    "Render when you turn Record take off. The only choice that \
                     works with a looping transport.",
                ),
                (
                    crate::RenderTrigger::OnTransportStop,
                    "Transport stops",
                    "Render the moment the transport stops after recording \
                     something — a play-through or an audio export then needs no \
                     further clicks. Recording switches itself off too.",
                ),
            ],
        );
        // Free-text paths rather than a file dialog: a plugin GUI has no
        // portable one, and these are set once and then left.
        labeled_path(ui, "Renderer", &mut render.renderer_path).on_hover_text(
            "Path to the lattice-offline binary. Leave empty to use the copy \
             update-plugin.sh installs.",
        );
        labeled_path(ui, "Options", &mut render.extra_args).on_hover_text(
            "Extra lattice-offline flags, split on spaces: \
             --size 3840x2160 --layout side-by-side",
        );
    }

    // Render the take you just recorded with the settings on screen NOW — the
    // take file itself only carries a record-time snapshot, so this is what
    // makes a post-record frame/bounce/offset actually reach the video.
    if state.last_take_ready {
        ui.add_space(2.0);
        if ui
            .button("Render now")
            .on_hover_text(
                "Render the take you last recorded, now, with these current \
                 settings — frame, bounced audio, and offset. Runs in the \
                 background; the video lands next to the take.",
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
