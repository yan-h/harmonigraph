//! The Video pane (the `render` module): a live, aspect-locked preview of the
//! offline video frame.
//!
//! It composes the Lattice and Spectral panes through the same
//! [`Layout`](crate::Layout) the offline renderer uses, at a chosen aspect
//! ratio and split — so what you frame here is what the render produces. The
//! frame settings live in [`RenderFrame`](crate::RenderFrame), persisted with
//! the take, so `harmonigraph-offline` reproduces exactly this composition.
//!
//! The preview's lattice is a *second* live lattice view. It must not go
//! through [`draw_pane`](crate::draw_pane) (which hardcodes GPU pane id 0 and
//! would fight the docked Lattice tab); it renders directly with its own id.
//! You frame the camera in the Lattice tab and watch it land here at the
//! render's aspect — which is exactly what decides "how much of the lattice is
//! exposed" (a wider frame shows more horizontally).

use egui::Sense;
use harmonigraph_render::lattice_paint_callback;
use harmonigraph_scene::derive_scene;

use super::section;
use crate::widgets::{button_row, choice_row, record_button, ValueBar};
use crate::{theme, LatticeSide, Layout, Pane, SharedState};

/// The preview's lattice is a second live view, so it needs its own GPU id —
/// the docked Lattice tab owns 0, and two views sharing an id overwrite each
/// other's buffers within a frame.
const PREVIEW_PANE_ID: u64 = 1;

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
    // Compose with the SAME Layout the offline renderer resolves.
    let layout = Layout::split(frame.lattice, frame.split);

    // Make the render frame obvious against the pane. The letterbox padding
    // takes the panel color, so it reads as inert chrome rather than part of
    // the shot; the aspect box takes the render's OWN frame background — the
    // color the offline renderer shows in its margins and inter-pane gaps — so
    // the box is exactly the pixels the video will contain. Painting the
    // padding and the pane fills both `well()` leaves no way to tell where
    // the frame ends.
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
                // Directly rather than through `draw_pane`, for the texture
                // slot: 1 here, so this second live copy doesn't clobber the
                // docked pane's spectrogram (slot 0). Its text sizes itself
                // off the rect it is given, so drawing the pane small draws
                // its type small, as the render will.
                super::spectral::spectral_pane(&mut child, state, now, 1);
            }
            Pane::Lattice => preview_lattice(ui, rect, state, now),
        }
    }

    // The seam between panes, exactly as the render bakes it.
    let translated: Vec<_> =
        placements.iter().map(|(p, r)| (*p, r.translate(box_rect.min.to_vec2()))).collect();
    layout.paint_dividers(ui.painter(), &translated);
}

/// Aspect ratio, resolution, arrangement, and split — editing the persisted
/// `RenderFrame` and the resolution beside it.
fn frame_controls(ui: &mut egui::Ui, state: &mut SharedState) {
    section(ui, "Frame");
    // A `button_row` rather than a `choice_row`: the selection is a PAIR of
    // numbers, not one enum value, so there is nothing for choice_row's
    // `selectable_value` to compare against.
    button_row(ui, |ui| {
        ui.label("Aspect");
        let f = &mut state.render_config.frame;
        for (w, h) in [(16u32, 9u32), (9, 16), (1, 1), (4, 5), (21, 9)] {
            let on = f.aspect_w == w && f.aspect_h == h;
            if ui.selectable_label(on, format!("{w}:{h}")).clicked() {
                f.aspect_w = w;
                f.aspect_h = h;
            }
        }
    });
    // The SHORT edge, not a named format: "1080p" means nothing to a 9:16
    // frame, where 1080 is the width. Aspect decides the shape and this
    // decides only how big, so each option shows the pixels it lands on and
    // the pair is what the plugin passes as `--size`.
    //
    // A size lifted out of a pre-Resolution blob (see
    // `RenderConfig::migrate_legacy`) can land between these, and then no
    // button reads as selected until one is clicked. That is why 720 is on the
    // list rather than only the three sizes worth delivering: it is both a
    // real draft setting for a render measured in minutes and where a
    // `--size 1280x720` migrates to.
    let frame = state.render_config.frame;
    let sizes: Vec<(u32, String, String)> = [720u32, 1080, 1440, 2160]
        .iter()
        .map(|&short| {
            let [w, h] = frame.pixels(short);
            (short, short.to_string(), format!("{w}x{h}"))
        })
        .collect();
    let options: Vec<(u32, &str, &str)> =
        sizes.iter().map(|(v, label, hint)| (*v, label.as_str(), hint.as_str())).collect();
    choice_row(ui, "Resolution", &mut state.render_config.short_edge, &options);
    let f = &mut state.render_config.frame;
    // Named for where the LATTICE goes, so the row reads as the placement it
    // is — "Lattice: Top" rather than an axis plus a convention about which
    // pane leads. Off `ALL` with an exhaustive match, like the Spectral pane's
    // own four-sided row: a fifth side cannot reach the pane without a name
    // and a hint of its own.
    let sides = LatticeSide::ALL.map(|side| {
        let (label, hint) = match side {
            LatticeSide::Left => ("Left", "Lattice left, spectral pane right"),
            LatticeSide::Right => ("Right", "Lattice right, spectral pane left"),
            LatticeSide::Top => ("Top", "Lattice above, spectral pane below"),
            LatticeSide::Bottom => ("Bottom", "Lattice below, spectral pane above"),
        };
        (side, label, hint)
    });
    choice_row(ui, "Lattice", &mut f.lattice, &sides);
    let label = if f.lattice.sizes_by_height() { "Lattice height" } else { "Lattice width" };
    // The range `Layout::split` itself honours, rather than a tighter one on
    // top of it: the layout clamps to 0.05..=0.95, so a bar that went wider
    // would move under the pointer and render the same picture either side of
    // the clamp. Both panes stay on screen at the ends — a frame that is all
    // lattice or all spectrum is what the `lattice` and `spectral` layout
    // presets are for, and they say so in the render rather than by a slider
    // pushed to its stop.
    ValueBar::new(&mut f.split, 0.05..=0.95, label).show(ui);
}

/// Empty the three things that accumulate, in one press, next to the button
/// that starts a take.
///
/// Each already has a button in the pane that owns it — Scene's "Clear trail",
/// the Analyzer's "Clear roll" and "Clear spectrogram" — and those stay, since
/// clearing one is a real thing to want while dialing that one in. This is for
/// the other moment, when the three are wanted together and there is only one
/// reason: a take about to be recorded should start on an empty picture,
/// because whatever is left over is baked into the video's opening seconds.
/// Three panes to visit for one intention is what makes it a button here.
///
/// It clears display state only — nothing about the take, the render, or the
/// tuning — so there is nothing to undo and no confirmation to sit through.
fn clear_everything(ui: &mut egui::Ui, state: &mut SharedState) {
    button_row(ui, |ui| {
        if ui
            .button("Clear everything")
            .on_hover_text(
                "Forget the lattice trail, the piano roll, and the spectrogram, so \
                 the next take opens on an empty picture. A note held across this \
                 is gone from the roll until it is played again; the lattice keeps \
                 what is still sounding.",
            )
            .clicked()
        {
            state.clear_accumulated();
        }
    });
}

/// Which spectrogram the render bakes: the live scrolling window (exactly what
/// the preview shows), or the whole take laid out at once with a sweeping
/// playhead. The live preview can't lay the whole take out, so a Playhead
/// choice leaves the preview's spectral region blank — see
/// `playhead_placeholder`.
///
/// Sets `RenderConfig.playhead`, which rides in the take's `ui_state` and is
/// the ONLY thing that decides this. The renderer turns the playhead on for
/// `--playhead` or this setting, whichever says yes, so a plugin that also
/// passed the flag would be answering a question the row is supposed to own —
/// and passing it unconditionally, which the plugin did, made "Live"
/// unreachable. `RenderRequest::playhead` is why it no longer does.
fn spectrogram_controls(ui: &mut egui::Ui, state: &mut SharedState) {
    section(ui, "Spectrogram");
    choice_row(
        ui,
        "Render",
        &mut state.render_config.playhead,
        &[
            (false, "Live", "Bake the live scrolling spectrogram, exactly as previewed here"),
            (
                true,
                "Playhead",
                "Lay the whole take's spectrogram out at once with a sweeping playhead. \
                 Needs recorded audio. The live preview has neither, so it leaves that \
                 region of the frame blank.",
            ),
        ],
    );
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
/// from the render — better to show nothing and name it. (Drawing the live
/// scrolling spectrogram under a small "Playhead" pill in the corner reads as
/// an odd label stuck on an otherwise trustworthy preview.)
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
fn preview_lattice(ui: &mut egui::Ui, rect: egui::Rect, state: &SharedState, now: f64) {
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    let mut scene = derive_scene(
        &state.tracker,
        &state.tuning,
        &state.view,
        &state.frame_params,
        state.camera,
        None,
        now,
    );
    // The preview is a picture of the RENDER, so its knockouts clear to the
    // render layout's background rather than to the panel this preview
    // happens to sit on — the same color `harmonigraph-offline` will clear to.
    scene.background = harmonigraph_scene::skin::ground_color(
        Layout::split(state.render_config.frame.lattice, state.render_config.frame.split)
            .background,
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
    // Node names/cents, exactly as the Lattice pane and the render draw them:
    // sized off this rect, so a preview a third of the render's size draws
    // them a third of the size, which is what makes it a preview. Clipped
    // to the lattice rect: a node near the frame edge would otherwise paint its
    // label out past the preview box, since draw_node_labels uses an unclipped
    // painter (harmless in the docked pane, which owns its whole rect and is
    // clipped by the dock; here the rect is only a sub-region of the pane).
    if state.view.show_labels {
        let mut clipped = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        clipped.set_clip_rect(rect);
        let mut batch = crate::text::TextBatch::default();
        super::lattice::draw_node_labels(&clipped, rect, &scene, &state.view, &mut batch);
        batch.flush(clipped.painter(), rect, state, crate::text::LATTICE_PREVIEW_LABELS);
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
    clear_everything(ui, state);
    render_progress(ui, state);

    // When a take finishes and turns into a video. No other home, so it sits
    // right under the switch that starts one.
    choice_row(
        ui,
        "Finish",
        &mut state.render_config.trigger,
        &[
            (
                crate::RenderTrigger::OnDisarm,
                "On disarm",
                "Render when you switch Record take off — predictable, and works however the \
                 transport behaves.",
            ),
            (
                crate::RenderTrigger::OnTransportStop,
                "On stop",
                "Render as soon as the transport stops after recording something, disarming at \
                 the same moment — a play-through renders itself.",
            ),
            (
                crate::RenderTrigger::AtLoopEnd,
                "At loop end",
                "Record one arranger-loop pass, then end the moment the loop repeats and render \
                 it — no manual stop to mistime. Turn LOOPING ON: it ends when the transport \
                 wraps back. With looping off it just waits for you to disarm.",
            ),
        ],
    );

    // Re-render the last take with the frame you've dialed in since recording.
    // The take carries only a record-time snapshot, so this is how a reframed
    // preview reaches the video without recording again.
    if state.last_take_ready {
        ui.add_space(2.0);
        if ui
            .button("Re-render take")
            .on_hover_text(
                "Render the take you last recorded again, now, with the current \
                 frame. Runs in the background; the video lands next to the take. \
                 Pressing it during a render cancels that one and starts over.",
            )
            .clicked()
        {
            state.render_now = true;
        }
    }
}

/// How far the background render has got, while one is running.
///
/// A render is minutes of work started by a button that then looks like
/// nothing happened: the status line names the file and never changes again
/// until it is finished, so a long render and a hung one read identically. The
/// bar is the difference between them, and the frame counts are what say how
/// much longer — the renderer counts frames, and a rate you have watched for
/// ten seconds turns "3400/5400" into a time.
///
/// Absent, not greyed, when nothing is rendering: the take controls are the
/// pane's steady state and a permanent empty bar under them would read as a
/// render stuck at zero.
fn render_progress(ui: &mut egui::Ui, state: &SharedState) {
    let Some(progress) = state.render_progress else { return };
    let value = match progress.total {
        // Pad `done` to the width of `total` so the readout keeps one width as
        // it counts up: monospace, so that holds the name still beside it —
        // `progress_bar` has no range to reserve from, unlike `ValueBar`.
        0 => "starting".to_owned(),
        total => format!("{:>width$}/{total}", progress.done, width = total.to_string().len()),
    };
    ui.add_space(2.0);
    crate::widgets::progress_bar(ui, progress.fraction(), "Rendering", &value).on_hover_text(
        "Frames written of the frames this render is composing. It runs in \
         the background — the DAW, and this window, keep going while it does.",
    );
}
