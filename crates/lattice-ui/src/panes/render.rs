//! The Video pane (the `render` module): a live, aspect-locked preview of the
//! offline video frame.
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
use crate::widgets::{button_row, record_button, ValueBar};
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

/// Points of breathing room between the render frame and the pane edge, so
/// the frame's boundary chrome always has somewhere to sit outside the
/// picture. See [`frame_chrome`].
const FRAME_CHROME_PAD: f32 = 8.0;

/// The preview keeps at least this much height, even when the controls above
/// it have already used the pane up. See [`render_pane`] — the floor is what
/// lets the pane overflow, and overflow is what the wheel scrolls.
const PREVIEW_MIN_HEIGHT: f32 = 160.0;

/// Frame controls, then a live preview of exactly what the offline render will
/// compose.
pub(crate) fn render_pane(ui: &mut egui::Ui, state: &mut SharedState, now: f64) {
    render_settings(ui, state);
    frame_controls(ui, state);
    spectrogram_controls(ui, state);

    section(ui, "Preview");
    let frame = state.render_config.frame;
    let avail = ui.available_size();
    if avail.x < 20.0 {
        return;
    }
    // The preview takes whatever the controls left it — but never less than
    // PREVIEW_MIN_HEIGHT. Without that floor it absorbed exactly the slack, so
    // the pane's content measured the same height as the pane no matter how
    // short the pane got: the dock's `ScrollArea` saw nothing sticking out and
    // the wheel had nothing to grab, which made Video the one settings pane
    // that would not scroll. Now a squeezed pane overflows instead, and the
    // controls stay reachable by scrolling rather than the preview shrinking
    // to a sliver.
    let size = egui::vec2(avail.x, avail.y.max(PREVIEW_MIN_HEIGHT));
    let (outer, _) = ui.allocate_exact_size(size, Sense::hover());
    let aspect = frame.aspect_w.max(1) as f32 / frame.aspect_h.max(1) as f32;
    // Inset before letterboxing: `letterbox` fits the box exactly on one axis,
    // so without this the frame's boundary chrome would have nowhere to go on
    // two sides. Shrinks on a small preview rather than eating it.
    let pad = FRAME_CHROME_PAD.min(size.min_elem() * 0.15);
    let box_rect = letterbox(outer.shrink(pad), aspect);
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
    // where the frame ended.
    let bg = layout.background;
    ui.painter().rect_filled(outer, 0.0, theme::panel());
    ui.painter().rect_filled(box_rect, 0.0, egui::Color32::from_rgb(bg.0, bg.1, bg.2));
    frame_chrome(ui, box_rect, pad);

    // The "Playhead" render variant lays the whole take's spectrogram out with
    // a sweeping playhead, from audio the live preview doesn't have. Rather
    // than show the live scrolling spectrogram and quietly mislead, leave the
    // spectral region empty and say so.
    let placements = layout.resolve(box_rect.size());
    let placeholder = state.render_config.playhead;
    for (pane, rect) in &placements {
        let rect = rect.translate(box_rect.min.to_vec2());
        match pane {
            Pane::Spectral if placeholder => playhead_placeholder(ui, rect),
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

    // The seam between panes, exactly as the render bakes it.
    let translated: Vec<_> =
        placements.iter().map(|(p, r)| (*p, r.translate(box_rect.min.to_vec2()))).collect();
    layout.paint_dividers(ui.painter(), &translated);
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

/// Which spectrogram the render bakes: the live scrolling window (exactly what
/// the preview shows), or the whole take laid out at once with a sweeping
/// playhead. Sets `RenderConfig.playhead`, which lattice-offline reads. The
/// live preview can't lay the whole take out, so a Playhead choice leaves the
/// preview's spectral region blank — see `playhead_placeholder`.
fn spectrogram_controls(ui: &mut egui::Ui, state: &mut SharedState) {
    section(ui, "Spectrogram");
    let playhead = &mut state.render_config.playhead;
    ui.horizontal(|ui| {
        ui.label("Render");
        ui.selectable_value(playhead, false, "Live")
            .on_hover_text("Bake the live scrolling spectrogram, exactly as previewed here");
        ui.selectable_value(playhead, true, "Playhead").on_hover_text(
            "Lay the whole take's spectrogram out at once with a sweeping playhead. \
             Needs recorded audio. The live preview has neither, so it leaves that \
             region of the frame blank.",
        );
    });
}

/// The marks that say "this rectangle is the video frame", drawn entirely
/// OUTSIDE the box so not one pixel of them lands in the picture: a hairline
/// tracing the boundary, plus crop ticks stepping out from the corners the way
/// every camera and layout tool marks a frame.
///
/// This replaced a 1px accent stroke drawn INSIDE the box. It was the color
/// the UI uses for selection and it sat on the outermost row of render pixels,
/// so it read as a blue border in the shot rather than as the edge of it.
fn frame_chrome(ui: &egui::Ui, box_rect: egui::Rect, pad: f32) {
    let p = ui.painter();
    p.rect_stroke(
        box_rect,
        0,
        egui::Stroke::new(1.0, theme::hairline()),
        egui::StrokeKind::Outside,
    );
    // One step further out than the hairline, and only when the inset left
    // room for them.
    let out = pad * 0.5;
    if out < 3.0 {
        return;
    }
    let len = (box_rect.size().min_elem() * 0.05).clamp(4.0, 14.0);
    let stroke = egui::Stroke::new(1.0, theme::text_dim());
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let corner = egui::pos2(
            if sx < 0.0 { box_rect.left() } else { box_rect.right() },
            if sy < 0.0 { box_rect.top() } else { box_rect.bottom() },
        );
        let o = corner + egui::vec2(sx * out, sy * out);
        p.line_segment([o, o - egui::vec2(sx * len, 0.0)], stroke);
        p.line_segment([o, o - egui::vec2(0.0, sy * len)], stroke);
    }
}

/// The preview's spectral region when the whole-song playhead variant is
/// selected: deliberately blank, with a label saying why.
///
/// That render lays the take's whole spectrogram out at once from recorded
/// audio and sweeps a playhead across it. The live preview has neither the
/// audio nor the layout, so anything it drew here would be a different picture
/// from the render — better to show nothing and name it. (It used to draw the
/// live scrolling spectrogram under a small "Playhead" pill in the corner,
/// which read as an odd label stuck on an otherwise trustworthy preview.)
fn playhead_placeholder(ui: &egui::Ui, rect: egui::Rect) {
    let p = ui.painter_at(rect);
    // The pane's own background, so the region still reads as the spectral
    // pane sitting there empty rather than as a hole in the frame.
    p.rect_filled(rect, 0.0, theme::well());
    if rect.width() < 90.0 || rect.height() < 30.0 {
        return;
    }
    let text = |s: &str, size: f32, color| {
        p.layout(s.to_owned(), egui::FontId::proportional(size), color, rect.width() - 16.0)
    };
    let title = text("Playhead render", 14.0, theme::accent());
    let sub = (rect.height() > 56.0)
        .then(|| text("the whole take, laid out at render time", 11.0, theme::text_dim()));
    let gap = if sub.is_some() { 4.0 } else { 0.0 };
    let total = title.size().y + gap + sub.as_ref().map_or(0.0, |g| g.size().y);
    let mut y = rect.center().y - total * 0.5;
    for galley in [Some(title), sub].into_iter().flatten() {
        let x = rect.center().x - galley.size().x * 0.5;
        let height = galley.size().y;
        p.galley(egui::pos2(x, y), galley, theme::text_dim());
        y += height + gap;
    }
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
    // No GPU-time slot: the Video pane's preview is a second lattice on
    // screen, and reporting its cost as THE lattice cost would be wrong.
    ui.painter().add(lattice_paint_callback(
        rect,
        &scene,
        state.target_format,
        PREVIEW_PANE_ID,
        None,
    ));
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

/// Recording and render-output settings. Lives here, in the Video tab, so that
/// tab is the one home for everything about turning a take into a video.
fn render_settings(ui: &mut egui::Ui, state: &mut SharedState) {
    // Take recording: the input half of offline video rendering. A record
    // button that doubles as its own indicator — press to arm; the dot breathes
    // while it waits for the transport, then goes solid while capturing. See
    // record_button in widgets.rs.
    if !state.take_supported {
        return;
    }
    section(ui, "Record");
    let rolling = state.take_rolling;
    record_button(ui, &mut state.take_recording, rolling, "Record take").on_hover_text(
        "Record the performance — notes, bends, parameter automation, the \
         current look, and the plugin's audio input — to a .take file. Press \
         again to stop; the take then renders to video (its audio becomes the \
         spectrogram and a playhead sweeps the piece). Events are stamped with \
         transport position, so nothing is captured until the transport rolls.",
    );
    if !state.take_status.is_empty() {
        ui.weak(&state.take_status);
    }

    // When a take finishes and turns into a video. No other home, so it sits
    // right under the switch that starts one.
    ui.horizontal(|ui| {
        ui.label("Finish");
        let trigger = &mut state.render_config.trigger;
        ui.selectable_value(trigger, crate::RenderTrigger::OnDisarm, "On disarm").on_hover_text(
            "Render when you switch Record take off — predictable, and works however the \
             transport behaves.",
        );
        ui.selectable_value(trigger, crate::RenderTrigger::OnTransportStop, "On stop")
            .on_hover_text(
                "Render as soon as the transport stops after recording something, disarming at \
                 the same moment — a play-through renders itself.",
            );
        ui.selectable_value(trigger, crate::RenderTrigger::AtLoopEnd, "At loop end").on_hover_text(
            "Record one arranger-loop pass, then end the moment the loop repeats and render it — \
             no manual stop to mistime. Turn LOOPING ON: it ends when the transport wraps back. \
             With looping off it just waits for you to disarm.",
        );
    });

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
