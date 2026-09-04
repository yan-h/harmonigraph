//! The Video pane (the `render` module): a live, aspect-locked preview of the
//! offline video frame.
//!
//! It composes the Lattice and Spectral panes through the same
//! [`Layout`](crate::Layout) the offline renderer uses, at a chosen aspect
//! ratio and split — so what you frame here is what the render produces. The
//! frame settings live in [`RenderFrame`](crate::RenderFrame), persisted with
//! the take, so `harmonigraph-offline` reproduces exactly this composition.
//!
//! The preview's lattice is a *second* live lattice view, drawn on a surface of
//! its own so that nothing it holds between frames is the docked Lattice tab's.
//! You frame the camera in the Lattice tab and watch it land here at the
//! render's aspect — which is exactly what decides "how much of the lattice is
//! exposed" (a wider frame shows more horizontally).

use egui::Sense;

use super::section;
use crate::widgets::{button_row, choice_row, option_label, record_button, ValueBar};
use crate::{theme, LatticeSide, Layout, Pane, SharedState};

/// The surface this preview's panes draw on. Every copy of a pane holds
/// something between frames keyed on its surface — a GPU buffer, a bloom chain,
/// a folded slab grid — so the preview takes one the dock does not
/// ([`DOCKED_SURFACE`](crate::panes::DOCKED_SURFACE)) and both stay whole.
const PREVIEW_SURFACE: usize = 1;

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
    record_controls(ui, state);
    frame_controls(ui, state);
    render_controls(ui, state);

    section(ui, "Preview");
    let frame = state.take.render_config.frame;
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
    let scale = crate::theme::ui_scale(ui.ctx());
    let size = egui::vec2(avail.x, avail.y.max(PREVIEW_MIN_HEIGHT * scale));
    let (outer, _) = ui.allocate_exact_size(size, Sense::hover());
    let aspect = frame.aspect_w.max(1) as f32 / frame.aspect_h.max(1) as f32;
    // Inset before letterboxing: `letterbox` fits the box exactly on one axis,
    // so without this the frame's boundary chrome would have nowhere to go on
    // two sides. Shrinks on a small preview rather than eating it.
    let pad = (FRAME_CHROME_PAD * scale).min(size.min_elem() * 0.15);
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
    let placeholder = state.take.render_config.playhead;
    for (pane, rect) in &placements {
        let rect = rect.translate(box_rect.min.to_vec2());
        match pane {
            Pane::Spectral if placeholder => playhead_placeholder(ui, rect),
            Pane::Spectral => {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                // Its text sizes itself off the rect it is given, so drawing
                // the pane small draws its type small, as the render will.
                super::spectral::spectral_pane(&mut child, state, now, PREVIEW_SURFACE);
            }
            // Unreachable, and here for the match rather than for the picture:
            // this preview composes `Layout::split`, which places the lattice
            // and the Analyzer and nothing else, so the Video panel cannot
            // preview a spiral at all. The `spiral` layout preset is
            // render-only, and a frame that wants one beside something else is
            // a hand-written `.ron`.
            //
            // Drawn rather than left as a `todo!()` so that whatever reaches
            // here if `Layout::split` ever grows a spiral gets the pane instead
            // of a panic inside the host.
            Pane::Spiral => {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                super::spiral::spiral_pane(&mut child, state, now, PREVIEW_SURFACE);
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
        ui.label("Aspect ratio")
            .on_hover_text("Shape of the exported video. Short edge sets its size in pixels.");
        let f = &mut state.take.render_config.frame;
        for (w, h) in [(16u32, 9u32), (9, 16), (1, 1), (4, 5), (21, 9)] {
            let on = f.aspect_w == w && f.aspect_h == h;
            if ui.selectable_label(on, option_label(&format!("{w}:{h}"))).clicked() {
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
    // 720 is on the list rather than only the three sizes worth delivering
    // because it is a real draft setting for a render measured in minutes.
    let frame = state.take.render_config.frame;
    let sizes: Vec<(u32, String, String)> = [720u32, 1080, 1440, 2160]
        .iter()
        .map(|&short| {
            let [w, h] = frame.pixels(short);
            (short, short.to_string(), format!("{w}x{h}"))
        })
        .collect();
    let options: Vec<(u32, &str, &str)> =
        sizes.iter().map(|(v, label, hint)| (*v, label.as_str(), hint.as_str())).collect();
    choice_row(ui, "Short edge (px)", &mut state.take.render_config.short_edge, &options);
    let f = &mut state.take.render_config.frame;
    // Named for where the LATTICE goes, so the row reads as the placement it
    // is — "Lattice: Top" rather than an axis plus a convention about which
    // pane leads. Off `ALL` with an exhaustive match, like the Spectral pane's
    // own four-sided row: a fifth side cannot reach the pane without a name
    // and a hint of its own.
    let sides = LatticeSide::ALL.map(|side| {
        let (label, hint) = match side {
            LatticeSide::Left => ("Left", "Lattice left, Analyzer right"),
            LatticeSide::Right => ("Right", "Lattice right, Analyzer left"),
            LatticeSide::Top => ("Top", "Lattice above, Analyzer below"),
            LatticeSide::Bottom => ("Bottom", "Lattice below, Analyzer above"),
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
    ValueBar::new(&mut f.split, 0.05..=0.95, label).percent().show(ui).on_hover_text(
        "How much of the frame the lattice takes; the Analyzer gets the \
         rest.",
    );
}

/// Empty the four things that accumulate, in one press, next to the button
/// that starts a take.
///
/// Each pane that owns an accumulation already clears its own — Labels' "Clear
/// note names", the Analyzer's "Clear roll and spectrogram" — and those stay, since
/// clearing what one pane draws is a real thing to want while dialing that pane
/// in. This is for the other moment, when all four are wanted together and
/// there is only one reason: a take about to be recorded should start on an
/// empty picture, because whatever is left over is baked into the video's
/// opening seconds. Two panes to visit for one intention is what makes it a
/// button here.
///
/// It clears display state only — nothing about the take, the render, or the
/// tuning — so there is nothing to undo and no confirmation to sit through.
fn clear_everything(ui: &mut egui::Ui, state: &mut SharedState) {
    button_row(ui, |ui| {
        if ui
            .button("Clear display history")
            .on_hover_text(
                "Clear lattice label history, MIDI ribbons, spectrogram history and lingering glow before recording. Held MIDI notes stay on the lattice but leave the roll until played again.",
            )
            .clicked()
        {
            state.clear_accumulated();
        }
    });
}

/// Turning a recorded take into a video: which spectrogram gets baked, when the
/// render fires, how it opens, and how far a running one has got.
///
/// Its own section rather than rows under Record, which is about CAPTURE: only
/// the record switch, its status and the clear are about getting a take, and
/// everything here happens after there is one. The Spectrogram row belongs here
/// rather than under a heading of its own, which would be one row calling
/// itself "Spectrogram" beside the Analyzer settings' heading of that name; what it
/// decides is what this render bakes.
///
/// The Spectrogram row draws whatever the shell is, since a standalone with no
/// transport still renders; the rows that need a take to exist follow the same
/// `supported` gate Record does.
///
/// `RenderConfig.playhead` is the ONLY thing deciding live-vs-playhead. The
/// renderer turns the playhead on for `--playhead` or this setting, whichever
/// says yes, so a plugin that also passed the flag would be answering a
/// question the row is supposed to own — and passing it unconditionally would
/// make "Scrolling" unreachable. `RenderRequest::playhead` is what keeps the row
/// deciding. The live preview can't lay a whole take out, so a Playhead choice
/// leaves the preview's spectral region blank — see `playhead_placeholder`.
fn render_controls(ui: &mut egui::Ui, state: &mut SharedState) {
    section(ui, "Render");
    choice_row(
        ui,
        "Spectrogram",
        &mut state.take.render_config.playhead,
        &[
            (false, "Scrolling", "Bake the live scrolling spectrogram, exactly as previewed here"),
            (
                true,
                "Playhead",
                "Show the entire recorded spectrogram with a moving playhead. Requires recorded audio; this region stays blank in the live preview.",
            ),
        ],
    );
    if !state.take.supported {
        return;
    }

    // When a take finishes and turns into a video.
    choice_row(
        ui,
        "Render when",
        &mut state.take.render_config.trigger,
        &[
            (
                crate::RenderTrigger::OnDisarm,
                "Record off",
                "Finish recording and start rendering when you turn Record take off.",
            ),
            (
                crate::RenderTrigger::OnTransportStop,
                "Transport stop",
                "Finish recording and render when the host transport stops or jumps backward after recording has begun.",
            ),
            (
                crate::RenderTrigger::AtLoopEnd,
                "Loop end",
                "Record one loop, then render when playback wraps to its start. Enable looping in the host; without a wrap, recording continues until you turn Record take off.",
            ),
        ],
    );

    // Re-render the last take with the frame you've dialed in since recording.
    // The take carries only a record-time snapshot, so this is how a reframed
    // preview reaches the video without recording again.
    if state.take.last_ready {
        ui.add_space(2.0);
        if ui
            .button("Re-render take")
            .on_hover_text(
                "Render the last take using the current frame settings. Saves the video beside the take. If a render is running, it is replaced by this one.",
            )
            .clicked()
        {
            state.take.render_now = true;
        }
    }
    render_progress(ui, state);
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
///
/// Runs the same draw sequence the docked Lattice tab does (see
/// [`super::lattice::lattice_pane`]) with its own GPU pane id, so a second
/// live copy never overwrites the docked pane's buffers within a frame — and
/// with no GPU-time slot, since the Video pane's preview is a second lattice
/// on screen, and reporting its cost as THE lattice cost would be wrong.
fn preview_lattice(ui: &mut egui::Ui, rect: egui::Rect, state: &mut SharedState, now: f64) {
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    // The preview is a picture of the RENDER, so it stands on the render
    // layout's background rather than on the panel this preview happens to sit
    // on — the same colour `harmonigraph-offline` will clear to.
    let background = harmonigraph_scene::skin::ground_color(
        Layout::split(state.take.render_config.frame.lattice, state.take.render_config.frame.split)
            .background,
    );
    super::lattice::draw_lattice(ui, rect, state, now, PREVIEW_SURFACE, background, None, None);
}

/// Capturing a take: the switch, what it is doing, and the clear that gives it
/// an empty picture to open on. What becomes of the take once it exists is
/// [`render_controls`].
///
/// Absent entirely in a shell with no transport to record against, which is
/// what leaves the standalone opening on Frame.
fn record_controls(ui: &mut egui::Ui, state: &mut SharedState) {
    // Take recording: the input half of offline video rendering. A record
    // button that doubles as its own indicator — press to arm; the dot breathes
    // while it waits for the transport, then goes solid while capturing. See
    // record_button in widgets.rs.
    if !state.take.supported {
        return;
    }
    section(ui, "Record");
    let rolling = state.take.rolling;
    record_button(ui, &mut state.take.recording, rolling, "Record take").on_hover_text(
        "Record notes, automation, the current look and the selected audio input for video export. Press again to finish, or choose an automatic ending under Render when.",
    );
    if !state.take.status.is_empty() {
        ui.weak(&state.take.status);
    }
    clear_everything(ui, state);
}

/// How far the background render has got, while one is running — and the way
/// to call it off.
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
///
/// The cancel shares that lifetime, because a running render is the only thing
/// it can act on. What it stops is the RENDER: the recording on disk is
/// untouched, so "Re-render take" above starts a fresh one from the same take,
/// and a video some earlier render finished stays where it landed — only the
/// run in flight has anything half-written to throw away.
fn render_progress(ui: &mut egui::Ui, state: &mut SharedState) {
    let Some(progress) = state.take.render_progress else { return };
    let value = match progress.total {
        // Pad `done` to the width of `total` so the readout keeps one width as
        // it counts up: monospace, so that holds the name still beside it —
        // `progress_bar` has no range to reserve from, unlike `ValueBar`.
        0 => "starting".to_owned(),
        total => format!("{:>width$}/{total}", progress.done, width = total.to_string().len()),
    };
    ui.add_space(2.0);
    crate::widgets::progress_bar(ui, progress.fraction(), "Rendering", &value).on_hover_text(
        "Completed frames out of the total. Rendering runs in the background while the DAW and editor remain available.",
    );
    button_row(ui, |ui| {
        if ui
            .button("Cancel render")
            .on_hover_text(
                "Stop this render and delete the part of the video it has \
                 written. The take is kept — \"Re-render take\" starts over from \
                 it.",
            )
            .clicked()
        {
            state.take.cancel_render = true;
        }
    });
}
