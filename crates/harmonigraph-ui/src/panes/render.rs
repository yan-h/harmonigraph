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

use super::{section, subsection};
use crate::params::ParamKey;
use crate::widgets::{button_row, choice_row, record_button, ValueBar};
use crate::{theme, LatticeSide, Layout, Pane, PictureState, RenderFrame};

/// The surface this preview's panes draw on. Every copy of a pane holds
/// something between frames keyed on its surface — a GPU buffer, a bloom chain,
/// a folded slab grid — so the preview takes one the dock does not
/// ([`DOCKED_SURFACE`](crate::panes::DOCKED_SURFACE)) and both stay whole.
const PREVIEW_SURFACE: usize = 1;

/// Points of breathing room between the render frame and the pane edge, so
/// the frame's boundary chrome always has somewhere to sit outside the
/// picture. See [`frame_chrome`].
const FRAME_CHROME_PAD: f32 = 7.0;

/// The inset the crop ticks need: a line's width clear of the outline, and
/// half their own past that. In fixed points, as the lines are.
const TICK_ROOM: f32 = 3.0 * theme::LINE_WIDTH;

/// The inset round the render frame at UI `scale` in a preview `size` across:
/// the design pad, never too little for the crop ticks, and shrunk on a small
/// preview rather than eating it.
fn chrome_pad(scale: f32, size: egui::Vec2) -> f32 {
    (FRAME_CHROME_PAD * scale).max(TICK_ROOM).min(size.min_elem() * 0.15)
}

/// The preview keeps at least this much height, even when the controls above
/// it have already used the pane up. See [`render_pane`] — the floor is what
/// lets the pane overflow, and overflow is what the wheel scrolls.
const PREVIEW_MIN_HEIGHT: f32 = 144.0;

/// Frame controls, then a live preview of exactly what the offline render will
/// compose.
pub(crate) fn render_pane(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    interaction: &mut crate::Interaction,
    now: f64,
) {
    record_controls(ui, state, interaction);
    frame_controls(ui, state);
    render_controls(ui, state, interaction);

    section(ui, "Preview", |ui| {
        let frame = state.appearance.render.frame;
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
        // two sides.
        let pad = chrome_pad(scale, size);
        let box_rect = letterbox(outer.shrink(pad), aspect);
        // Compose with the SAME Layout the offline renderer resolves.
        let layout = Layout::split(frame.lattice, frame.split);

        // Make the render frame obvious against the pane. The letterbox padding
        // takes the panel color, so it reads as inert chrome rather than part of
        // the shot; the aspect box takes the render's OWN frame background — the
        // picture's black, which no composed layout shows through (margin and gap
        // are 0) — so the box is exactly the pixels the video will contain. Painting the
        // padding and the pane fills both `picture()` leaves no way to tell where
        // the frame ends.
        let bg = layout.background;
        ui.painter().rect_filled(outer, 0.0, theme::panel());
        ui.painter().rect_filled(box_rect, 0.0, egui::Color32::from_rgb(bg.0, bg.1, bg.2));
        frame_chrome(ui, box_rect, pad);

        let placements: Vec<_> = layout
            .resolve(box_rect.size())
            .into_iter()
            .map(|(pane, rect)| (pane, rect.translate(box_rect.min.to_vec2())))
            .collect();
        let point_scale = preview_scale(box_rect.width(), &state.appearance.render);
        preview_panes(ui, &placements, state, now, point_scale);

        // The seam between panes, exactly as the render bakes it.
        layout.paint_dividers(ui.painter(), &placements);
        let appearance = &mut state.appearance;
        preview_layout_controls(
            ui,
            box_rect,
            &mut appearance.render.frame,
            &mut appearance.camera,
            &mut appearance.view,
        );
    });
}

/// Interaction chrome lives over the preview only; exports keep the plain seam.
fn preview_layout_controls(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    frame: &mut crate::RenderFrame,
    camera: &mut harmonigraph_scene::Camera,
    view: &mut harmonigraph_scene::ViewConfig,
) {
    let lattice = preview_lattice_rect(rect, frame.lattice, frame.split);
    let vertical = !frame.lattice.sizes_by_height();
    let seam = match frame.lattice {
        LatticeSide::Left => lattice.right_center(),
        LatticeSide::Right => lattice.left_center(),
        LatticeSide::Top => lattice.center_bottom(),
        LatticeSide::Bottom => lattice.center_top(),
    };
    let handle = egui::Rect::from_center_size(
        seam,
        if vertical {
            egui::vec2(12.0_f32.min(lattice.width()), rect.height())
        } else {
            egui::vec2(rect.width(), 12.0_f32.min(lattice.height()))
        },
    );
    // Keep the move target clear of the divider, including at the 5% limit.
    let mut move_rect = lattice;
    match frame.lattice {
        LatticeSide::Left => move_rect.max.x = handle.left(),
        LatticeSide::Right => move_rect.min.x = handle.right(),
        LatticeSide::Top => move_rect.max.y = handle.top(),
        LatticeSide::Bottom => move_rect.min.y = handle.bottom(),
    }
    let rearrange_held = ui.input(|i| i.modifiers.shift);
    let moving_lattice = ui
        .interact(move_rect, ui.scope_id().with("preview_lattice_move"), Sense::drag())
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text(if rearrange_held {
            "Drag the lattice to the left, right, top or bottom edge"
        } else {
            "Drag to pan the lattice · Hold Shift and drag to move it to the left, right, top or bottom edge · Scroll or pinch to zoom"
        });
    let resizing = ui
        .interact(handle, ui.scope_id().with("preview_lattice_resize"), Sense::drag())
        .on_hover_cursor(if vertical {
            egui::CursorIcon::ResizeHorizontal
        } else {
            egui::CursorIcon::ResizeVertical
        })
        .on_hover_text("Drag to resize the lattice");
    let grab_id = resizing.id.with("grab_offset");
    let fraction_at = |pointer: egui::Pos2| match frame.lattice {
        LatticeSide::Left => (pointer.x - rect.left()) / rect.width(),
        LatticeSide::Right => (rect.right() - pointer.x) / rect.width(),
        LatticeSide::Top => (pointer.y - rect.top()) / rect.height(),
        LatticeSide::Bottom => (rect.bottom() - pointer.y) / rect.height(),
    };
    if resizing.drag_started() {
        if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
            ui.data_mut(|d| d.insert_temp(grab_id, frame.split - fraction_at(origin)));
        }
    }
    if resizing.dragged() {
        if let Some(pointer) = resizing.interact_pointer_pos() {
            let offset = ui.data(|d| d.get_temp::<f32>(grab_id)).unwrap_or(0.0);
            frame.split = (fraction_at(pointer) + offset)
                .clamp(RenderFrame::SPLIT_MIN, RenderFrame::SPLIT_MAX);
            ui.ctx().request_repaint();
        }
    }
    if resizing.drag_stopped() {
        ui.data_mut(|d| d.remove::<f32>(grab_id));
    }
    // Match the Analyzer's divider: only the existing seam at rest, a full
    // two-point accent line on hover, and the stronger accent while dragging.
    let lit = if resizing.dragged() {
        Some(theme::accent())
    } else if resizing.hovered() {
        Some(theme::accent_edge())
    } else {
        None
    };
    if let Some(color) = lit {
        let ends = if vertical {
            [egui::pos2(seam.x, rect.top()), egui::pos2(seam.x, rect.bottom())]
        } else {
            [egui::pos2(rect.left(), seam.y), egui::pos2(rect.right(), seam.y)]
        };
        ui.painter().line_segment(ends, egui::Stroke::new(theme::LINE_WIDTH, color));
    }
    // Freeze the choice made at press time. Letting Shift change during a drag
    // switch modes would first pan the camera and then redock the picture on
    // release (or the reverse), which turns one gesture into two edits.
    let rearrange_id = moving_lattice.id.with("preview_rearrange");
    if moving_lattice.drag_started() {
        ui.data_mut(|data| data.insert_temp(rearrange_id, rearrange_held));
    }
    let navigating = !ui.data(|data| data.get_temp::<bool>(rearrange_id)).unwrap_or(rearrange_held);
    if navigating && moving_lattice.dragged() {
        let delta = moving_lattice.drag_delta();
        camera.pan(glam::Vec2::new(delta.x, delta.y));
        view.follow_camera(camera);
        ui.ctx().request_repaint();
    } else if !navigating && (moving_lattice.dragged() || moving_lattice.drag_stopped()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        if let Some(side) =
            moving_lattice.interact_pointer_pos().and_then(|p| preview_drop_side(rect, p))
        {
            let target = preview_lattice_rect(rect, side, frame.split);
            super::paint_preview_drop_target(ui.painter(), target);
            if moving_lattice.drag_stopped() {
                frame.lattice = side;
                ui.ctx().request_repaint();
            }
        }
    }
    if moving_lattice.drag_stopped() {
        ui.data_mut(|data| data.remove::<bool>(rearrange_id));
    }
}

/// Keep the interaction geometry even when the preview is too small to draw
/// a pane: `Layout::resolve` culls sub-point panes, but the controls still need them.
fn preview_lattice_rect(rect: egui::Rect, side: LatticeSide, split: f32) -> egui::Rect {
    preview_pane_rect(rect, side, split, Pane::Lattice)
}

fn preview_pane_rect(rect: egui::Rect, side: LatticeSide, split: f32, pane: Pane) -> egui::Rect {
    let layout = Layout::split(side, split);
    let (x0, y0, x1, y1) = layout.panes.iter().find(|p| p.pane == pane).unwrap().rect;
    egui::Rect::from_min_max(
        rect.min + rect.size() * egui::vec2(x0, y0),
        rect.min + rect.size() * egui::vec2(x1, y1),
    )
}

/// Normalized edge distances give every side an equal target on portrait and landscape frames.
/// The middle half of each axis is a cancel region, so a small drag inside a
/// wide lattice cannot silently dock it to the opposite side.
fn preview_drop_side(rect: egui::Rect, pointer: egui::Pos2) -> Option<LatticeSide> {
    if !rect.contains(pointer) {
        return None;
    }
    let x = (pointer.x - rect.left()) / rect.width();
    let y = (pointer.y - rect.top()) / rect.height();
    let (distance, side) = [
        (x, LatticeSide::Left),
        (1.0 - x, LatticeSide::Right),
        (y, LatticeSide::Top),
        (1.0 - y, LatticeSide::Bottom),
    ]
    .into_iter()
    .min_by(|a, b| a.0.total_cmp(&b.0))
    .unwrap();
    (distance <= 0.25).then_some(side)
}

/// The preview's logical frame size relative to the one the export uses.
/// Point-sized geometry uses this so a live preview remains the same
/// composition at either side of the export's logical size.
fn preview_scale(width: f32, config: &crate::RenderConfig) -> f32 {
    let pixels = config.frame.pixels(config.short_edge);
    let export_width = pixels[0] as f32 / crate::layout::export_pixels_per_point(pixels);
    (width / export_width).max(0.0)
}

/// Aspect ratio and resolution — editing the persisted
/// `RenderFrame` and the resolution beside it.
fn frame_controls(ui: &mut egui::Ui, state: &mut PictureState) {
    section(ui, "Frame", |ui| {
        let f = &mut state.appearance.render.frame;
        let mut aspect = (f.aspect_w, f.aspect_h);
        choice_row(
            ui,
            "Aspect ratio",
            &mut aspect,
            &[
                ((16, 9), "16:9", "Landscape"),
                ((9, 16), "9:16", "Portrait"),
                ((1, 1), "1:1", "Square"),
                ((21, 9), "21:9", "Ultrawide"),
            ],
        );
        (f.aspect_w, f.aspect_h) = aspect;
        // The SHORT edge, not a named format: "1080p" means nothing to a 9:16
        // frame, where 1080 is the width. Aspect decides the shape and this
        // decides only how big, so each option shows the pixels it lands on, the
        // pair the renderer derives from the exported look.
        //
        // 1440 is the fresh size.
        let frame = state.appearance.render.frame;
        let sizes: Vec<(u32, String, String)> = [720u32, 1080, 1440, 2160]
            .iter()
            .map(|&short| {
                let [w, h] = frame.pixels(short);
                (
                    short,
                    format!("{w} × {h}"),
                    format!("{w} × {h} pixels; {short} px on the short edge"),
                )
            })
            .collect();
        let options: Vec<(u32, &str, &str)> =
            sizes.iter().map(|(v, label, hint)| (*v, label.as_str(), hint.as_str())).collect();
        choice_row(ui, "Output size (px)", &mut state.appearance.render.short_edge, &options);
        frame_notes(ui, state);
    });
}

/// Framing the lattice on what has been drawn of a piece's notes.
///
/// What a click could not do is said under the button for as long as the
/// camera is still where that click left it, and no longer: a hand zoom or a
/// second framing makes it a statement about some other picture.
fn frame_notes(ui: &mut egui::Ui, state: &mut PictureState) {
    let id = ui.scope_id().with("frame_notes_outcome");
    let tracker = &state.runtime.tracker;
    let drawn = !tracker.history().is_empty() || tracker.voices().next().is_some();
    button_row(ui, |ui| {
        let clicked = ui
            .add_enabled(drawn, egui::Button::new("Frame notes"))
            .on_hover_text(
                "Pan and zoom the lattice onto every note drawn since Clear display history, and size the lattice's share of the frame to fit them. Play the piece through first.",
            )
            .on_disabled_hover_text("Play the piece through first: nothing has been drawn since Clear display history.")
            .clicked();
        if clicked {
            let positions = crate::framing::drawn_positions(
                &state.runtime.tracker,
                &state.runtime.tuning,
                &state.appearance,
            );
            let framed = state.appearance.frame_lattice(&positions);
            if framed.is_some() {
                // It sets the zoom and pan outright; the orbit it keeps.
                let set = [ParamKey::CameraDistance, ParamKey::CameraPanX, ParamKey::CameraPanY];
                crate::camera_movement::set_outright(ui.ctx(), &set);
            }
            let outcome = match framed {
                Some(true) => None,
                Some(false) => Some(
                    "The notes span more than the camera can show zoomed out; the outermost are cut off.",
                ),
                None => Some("None of the notes drawn lights a node near the camera, so there is nothing to frame."),
            };
            let camera = state.appearance.camera_movement();
            ui.data_mut(|data| data.insert_temp(id, outcome.map(|text| (text, camera))));
        }
    });
    let outcome = ui.data(|data| data.get_temp::<Option<(&'static str, [f32; 5])>>(id)).flatten();
    if let Some((text, camera)) = outcome {
        // Within a hair rather than exactly: the host hands the camera back
        // through its normalized parameters, which rarely round-trip an f32.
        let now = state.appearance.camera_movement();
        if camera.iter().zip(now).all(|(a, b)| (a - b).abs() <= 1e-3 * a.abs().max(1.0)) {
            crate::widgets::label(ui, text);
        }
    }
}

/// Empty the four things that accumulate, in one press, next to the button
/// that starts a take.
///
/// Each pane that owns an accumulation already clears its own — Labels' "Clear
/// label history", View's "Clear history", which clears the ribbons as well as
/// the heatmap — and those stay, since
/// clearing what one pane draws is a real thing to want while dialing that pane
/// in. This is for the other moment, when all four are wanted together and
/// there is only one reason: to watch a take being recorded on an empty
/// picture. Two panes to visit for one intention is what makes it a button here.
///
/// It clears what you see and nothing else — not the take, the render, or the
/// tuning — so there is nothing to undo and no confirmation to sit through. It
/// does not change a video: an export draws from its take alone, replaying
/// the take's notes from its start, whatever the live picture holds.
fn clear_everything(ui: &mut egui::Ui, state: &mut PictureState) {
    button_row(ui, |ui| {
        if ui
            .button("Clear display history")
            .on_hover_text(
                "Clear lattice label history, MIDI ribbons, spectrogram history and lingering glow from what you see while recording. Held MIDI notes stay on the lattice but leave the roll until played again. A video export draws from its recorded take, not from what you see, so this does not change one.",
            )
            .clicked()
        {
            state.clear_accumulated();
        }
    });
}

/// Turning a recorded take into a video: its history window, export queue
/// and progress. The finish trigger belongs beside Record take.
///
/// Its own section rather than rows under Record, which is about CAPTURE: only
/// the record switch, finish trigger, status and clear are about getting a take, and
/// everything here happens after there is one. The Video history row belongs here
/// rather than under a heading of its own, which would be one row calling
/// itself "Spectrogram" beside the Analyzer settings' heading of that name; what it
/// decides is what this render bakes.
///
/// The Video history row draws whatever the shell is, since a standalone with no
/// transport still renders; the rows that need a take to exist follow the same
/// `supported` gate Record does.
///
fn render_controls(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    interaction: &mut crate::Interaction,
) {
    use crate::SpectrogramRender;

    section(ui, "Render", |ui| {
        // Scrolling names the span it scrolls, so the choice beside it reads as the
        // alternative to that number without the Analyzer page open.
        let scrolling = format!(
            "Scrolling ({})",
            super::spectral::settings::span_readout(state.appearance.spectrum.roll_seconds)
        );
        choice_row(
            ui,
            "Video history",
            &mut state.appearance.render.spectrogram,
            &[
                (
                    SpectrogramRender::Scrolling,
                    &scrolling,
                    "Bake the live scrolling spectrogram, exactly as previewed here. It shows the History duration set on the Analyzer page.",
                ),
                (
                    SpectrogramRender::WholeVideo,
                    "Fit video (max 10 min)",
                    "Scroll slowly enough that the whole video fits: by its last frame, the MIDI ribbons and spectrogram reach back to its first. Up to 10 minutes. The preview here keeps showing the History duration.",
                ),
            ],
        );
        // Dialled names its range, as Scrolling names its span.
        let spectrum = &state.appearance.spectrum;
        let dialled = format!(
            "Dialled ({}–{})",
            super::spectral::settings::hz_readout(spectrum.low_midi),
            super::spectral::settings::hz_readout(spectrum.high_midi)
        );
        choice_row(
            ui,
            "Frequency range",
            &mut state.appearance.render.pitch_range,
            &[
                (
                    crate::PitchRangeRender::Dialled,
                    &dialled,
                    "Draw the Frequency range set on the Analyzer page, exactly as previewed here.",
                ),
                (
                    crate::PitchRangeRender::Fit,
                    "Fit piece",
                    "Frame what the piece fills, anywhere from 20 Hz to 20 kHz, whatever the Frequency range set on the Analyzer page: \
                     every pitch the spectrogram lights a fifth of the way up its colours for half a second in all, \
                     and every recorded note, with a semitone to spare. Bands it leaves dark at either end are dropped. \
                     Choose Dialled to crop by hand. The preview here keeps showing the range as set.",
                ),
            ],
        );
        // Here rather than beside Finish recording: a render choice like Output
        // size, which a queued export takes from the current look.
        //
        // Stored in f64 seconds, which the export's frame count reads; the bar
        // edits an f32 copy and writes back only on an edit, so an untouched
        // tail keeps its exact stored value. Eased because a release worth
        // waiting for is a few seconds, at the fine end of a minute's range.
        let mut tail = state.appearance.render.tail as f32;
        if ValueBar::new(&mut tail, crate::TAIL_RANGE.0 as f32..=crate::TAIL_RANGE.1 as f32, "Tail")
            .eased(true)
            .unit(1.0, " s")
            .default_value(crate::RenderConfig::default().tail as f32)
            .hint(
                "How long the video runs past the last recorded note or automation, so releases finish fading. \
                 The video always runs to the end of the recorded audio, so 0 ends an audio export at its \
                 range end.",
            )
            .show(ui)
            .changed()
        {
            state.appearance.render.tail = f64::from(tail);
        }
        // A render choice like the Tail: the picture is the same either way,
        // and a queued export takes the current one.
        choice_row(
            ui,
            "Encoder",
            &mut state.appearance.render.encoder,
            &[
                (
                    crate::VideoEncoder::X264,
                    "x264",
                    "Software H.264 at the quality YouTube recommends, tuned so the spectrogram's grain does not flicker. The slower of the two, and it takes most of the processor while it runs.",
                ),
                (
                    crate::VideoEncoder::Hardware,
                    "Hardware",
                    "This Mac's hardware H.264 encoder: about a third faster than x264 at 1440p and it leaves the processor free, for a file up to a quarter larger and no grain tuning. An export fails on a Mac without one or without Apple Silicon. A frame over 4096 px on a side (21:9 at 2160) goes through x264 instead.",
                ),
            ],
        );
        crate::widgets::checkbox(ui, &mut state.appearance.render.archive, "Archive copy").on_hover_text(
            "Also save a small AV1 copy beside the video as <name>-archive.mp4, to keep once the video is uploaded. \
             About a fourteenth of the size and close to the video's quality, but upload the video itself: \
             YouTube re-encodes what it gets, and the copy has already lost a little. \
             Adds about 0.6 s of encoding per second of video. \
             Needs ffmpeg with SVT-AV1, as Homebrew's has; without it the video still saves and its export row says why.",
        );
        if !interaction.take.supported {
            return;
        }

        export_queue(ui, &state.appearance, interaction);
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
        egui::Stroke::new(theme::LINE_WIDTH, theme::hairline()),
        egui::StrokeKind::Outside,
    );
    // One step further out than the outline, a line's width clear of it so the
    // two read as two marks rather than one thick one, and only when the inset
    // left room for them.
    if pad < TICK_ROOM {
        return;
    }
    let out = TICK_ROOM - 0.5 * theme::LINE_WIDTH;
    let len = (box_rect.size().min_elem() * 0.05).clamp(4.0, 14.0);
    let stroke = egui::Stroke::new(theme::LINE_WIDTH, theme::text_dim());
    for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let corner = egui::pos2(
            if sx < 0.0 { box_rect.left() } else { box_rect.right() },
            if sy < 0.0 { box_rect.top() } else { box_rect.bottom() },
        );
        let o = corner + egui::vec2(sx * out, sy * out);
        // One path through the corner, so its join is mitred: two butt-ended
        // segments leave the outer corner square of a wide stroke unpainted.
        theme::round_ended(
            p,
            vec![o - egui::vec2(sx * len, 0.0), o, o - egui::vec2(0.0, sy * len)],
            stroke,
        );
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
/// will. The wheel and pinch zoom the shared camera here; picking, note hover
/// and the learn badge stay off, so `draw_lattice` still receives no response.
///
/// Runs the same draw sequence the docked Lattice tab does (see
/// [`super::lattice::lattice_pane`]) with its own GPU pane id, so a second
/// live copy never overwrites the docked pane's buffers within a frame — and
/// with no GPU-time slot, since the Video pane's preview is a second lattice
/// on screen, and reporting its cost as THE lattice cost would be wrong.
fn preview_lattice(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    state: &mut PictureState,
    now: f64,
    point_scale: f32,
) -> Option<harmonigraph_scene::DrawnWindow> {
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return None;
    }
    // The preview is a picture of the RENDER, so it stands on the render
    // layout's background rather than on the panel this preview happens to sit
    // on — the same colour `harmonigraph-offline` will clear to.
    let background = harmonigraph_scene::skin::ground_color(
        Layout::split(state.appearance.render.frame.lattice, state.appearance.render.frame.split)
            .background,
    );
    let response = ui.interact(rect, ui.scope_id().with("preview_lattice_zoom"), Sense::hover());
    if let Some((scroll, zoom)) = super::zoom_gesture(ui, &response) {
        if scroll != 0.0 {
            state.appearance.camera.zoom(scroll);
            // This pane lives inside the Video tab's vertical ScrollArea. A
            // wheel over the picture belongs to its zoom; taking it here keeps
            // the parent from scrolling the controls at the same time.
            ui.input_mut(|input| input.smooth_scroll_delta = egui::Vec2::ZERO);
        }
        if zoom != 1.0 {
            state.appearance.camera.zoom_by(zoom);
        }
    }
    Some(super::lattice::draw_lattice(
        ui,
        rect,
        state,
        now,
        PREVIEW_SURFACE,
        background,
        None,
        None,
        point_scale,
        super::lattice::NO_SOLO,
    ))
}

/// The preview's panes at their placements, as [`crate::draw_panes`] draws an
/// export: the lattice first, after its wheel and pinch have moved the camera,
/// and the spectral pane describing the window it returns. That window is
/// published nowhere, so the docked panes go on describing the docked lattice
/// (see `PictureState::shown`).
fn preview_panes(
    ui: &mut egui::Ui,
    placements: &[(Pane, egui::Rect)],
    state: &mut PictureState,
    now: f64,
    point_scale: f32,
) {
    debug_assert!(crate::layout::lattice_first(placements), "{placements:?}");
    let mut shown = None;
    for &(pane, rect) in placements {
        match pane {
            Pane::Lattice => {
                let window = preview_lattice(ui, rect, state, now, point_scale);
                shown = shown.or(window);
            }
            Pane::Spectral => {
                let shown = shown.unwrap_or_else(|| state.appearance.view.reach());
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                super::spectral::spectral_pane(
                    &mut child,
                    state,
                    now,
                    PREVIEW_SURFACE,
                    point_scale,
                    super::spectral::Navigation::Preview,
                    shown,
                    super::lattice::NO_SOLO,
                );
            }
        }
    }
}

/// Capturing a take: the switch, what it is doing, and the clear that gives it
/// an empty picture to open on. What becomes of the take once it exists is
/// [`render_controls`].
///
/// Absent entirely in a shell with no transport to record against, which is
/// what leaves the standalone opening on Frame.
fn record_controls(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    interaction: &mut crate::Interaction,
) {
    // Take recording: the input half of offline video rendering. A record
    // button that doubles as its own indicator — press to arm; the dot breathes
    // while it waits for the transport, then goes solid while capturing. See
    // record_button in widgets.rs.
    if !interaction.take.supported {
        return;
    }
    section(ui, "Record", |ui| {
        let rolling = interaction.take.rolling;
        record_button(ui, &mut interaction.take.recording, rolling, "Record take").on_hover_text(
            "Record notes, automation, the current look and the selected audio input for video export. Press again to finish, or choose an automatic ending below. Finishing starts the video render.",
        );
        if !interaction.take.status.is_empty() {
            crate::widgets::label(ui, &interaction.take.status);
        }
        // When a take finishes and turns into a video.
        choice_row(
            ui,
            "Finish recording",
            &mut state.appearance.render.trigger,
            &[
                (
                    crate::RenderTrigger::OnDisarm,
                    "Manually",
                    "Finish recording and start rendering when you turn Record take off.",
                ),
                (
                    crate::RenderTrigger::OnTransportStop,
                    "Transport stop",
                    "Finish recording and render once the host transport has stayed stopped after recording has begun. A jump records on.",
                ),
                (
                    crate::RenderTrigger::AtLoopEnd,
                    "Loop end",
                    "Record one loop, then render when playback wraps to its start. A forward seek records on. Without a wrap, recording continues until you turn Record take off.",
                ),
                (
                    crate::RenderTrigger::AtBar,
                    "At bar",
                    "Finish recording and render when the transport plays through the bar below.",
                ),
            ],
        );
        // Only under the trigger that reads it — a bar shown beside three triggers
        // that ignore it is a dial that appears to do nothing three times out of
        // four. The value is kept either way, so switching away and back does not
        // lose it.
        if state.appearance.render.trigger == crate::RenderTrigger::AtBar {
            ui.horizontal_wrapped(|ui| {
                crate::widgets::label(ui, "Stop at bar").on_hover_text(
                    "Counted as the host's arranger counts: bar 1 is the song's start. \
                     Recording must reach this bar from before it — arming with the playhead \
                     already past it records until you turn Record take off.",
                );
                // A drag value rather than a ValueBar: a bar's travel across
                // 100,000 bars would move hundreds of bars a point (#1677).
                #[allow(clippy::disallowed_methods)]
                ui.add(
                    egui::DragValue::new(&mut state.appearance.render.stop_bar)
                        .range(crate::STOP_BAR_RANGE.0..=crate::STOP_BAR_RANGE.1)
                        .speed(0.25),
                );
            });
        }
        clear_everything(ui, state);
    });
}

/// The Appearance row's Recorded option.
const RECORDED_HINT: &str = "Each take's own appearance, saved when it was recorded, with the Output size, Tail, Encoder and Archive copy set here.";

/// What both Queue buttons answer a hover with: what a queued job carries,
/// and what the queue survives.
const QUEUE_HINT: &str = "Every job captures the selected look and the current Video settings. \
     Under Recorded, only Output size, Tail, Encoder and Archive copy are the current ones: the take keeps the rest of the look it was recorded with. \
     Recorded camera movement wins during replay. \
     The queue survives closing this editor, but removing the plugin or closing the project cancels unfinished jobs, and there is no resume after a crash.";

/// Captured export requests and instance-owned job controls. The builder is
/// folded independently of the jobs so progress and failures remain visible.
fn export_queue(
    ui: &mut egui::Ui,
    appearance: &crate::AppearanceDocument,
    interaction: &mut crate::Interaction,
) {
    use crate::{ExportAction, ExportLook};
    use harmonigraph_take::render::ExportStatus;
    if ui
        .button("Open video folder")
        .on_hover_text(
            "Open the folder takes are recorded to in Finder. Each take's video is saved beside it.",
        )
        .clicked()
    {
        interaction.take.export_actions.push(ExportAction::OpenTakeFolder);
    }
    subsection(ui, "Exports", |ui| {
        ui.add(egui::TextEdit::multiline(&mut interaction.take.export_paths).desired_width(ui.available_width()).desired_rows(2).hint_text("/path/to/music.take"))
            .on_hover_text("One take path per line. Each video is saved beside its take; an existing video gets a numbered variant.");
        let options = [
            (ExportLook::Recorded, "Recorded", RECORDED_HINT),
            (ExportLook::Current, "Current", "The editor's appearance when you queue."),
        ];
        crate::widgets::choice_row(ui, "Appearance", &mut interaction.take.export_look, &options);
        let mut paths = Vec::new();
        button_row(ui, |ui| {
            if ui
                .add_enabled(
                    interaction.take.last_take.is_some(),
                    egui::Button::new("Queue last take"),
                )
                .on_hover_text(QUEUE_HINT)
                .clicked()
            {
                paths.extend(interaction.take.last_take.clone());
            }
            if ui.button("Queue paths").on_hover_text(QUEUE_HINT).clicked() {
                paths.extend(
                    interaction
                        .take
                        .export_paths
                        .lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                        .map(std::path::PathBuf::from),
                );
            }
        });
        if !paths.is_empty() {
            interaction.take.export_said.clear();
        }
        for path in paths {
            let said = match capture_export(appearance, interaction.take.export_look, &path) {
                Ok((action, notes)) => {
                    interaction.take.export_actions.push(action);
                    notes
                }
                Err(error) => vec![error],
            };
            for line in said {
                if !interaction.take.export_said.is_empty() {
                    interaction.take.export_said.push('\n');
                }
                interaction.take.export_said.push_str(&format!("{}: {line}", path.display()));
            }
        }
        if !interaction.take.export_said.is_empty() {
            crate::widgets::label(ui, &interaction.take.export_said);
        }
    });
    for job in &interaction.take.exports {
        ui.push_id(job.id, |ui| {
            // The video's name stands for both: it shares the take's stem and
            // folder, so the name tells jobs apart and the full path is a hover
            // away. A recorded take's own export has no size to show until the
            // renderer reads its header, so it shows none.
            let name = job.output.file_name().unwrap_or_default().to_string_lossy();
            let line = match job.size {
                Some([w, h]) => format!("{:?} · {w}×{h} · {name}", job.state),
                None => format!("{:?} · {name}", job.state),
            };
            crate::widgets::label(ui, line).on_hover_text(job.output.display().to_string());
            if matches!(job.state, ExportStatus::Running | ExportStatus::Cancelling) {
                let progress = job.progress;
                let value = progress.fraction().map_or_else(
                    || "starting".into(),
                    |f| format!("{:>3}%", (f * 100.0).floor() as u32),
                );
                crate::widgets::progress_bar(ui, progress.fraction(), "Rendering", &value)
                    .on_hover_text(format!(
                        "{} of {} frames encoded, an archive copy's counted after the video's",
                        progress.done, progress.total
                    ));
            }
            if !job.detail.is_empty() {
                crate::widgets::label(ui, &job.detail);
            }
            button_row(ui, |ui| {
                if matches!(job.state, ExportStatus::Pending | ExportStatus::Running)
                    && ui.button("Cancel render").clicked()
                {
                    interaction.take.export_actions.push(ExportAction::Cancel(job.id));
                }
                if matches!(job.state, ExportStatus::Failed | ExportStatus::Cancelled)
                    && ui.button("Retry").clicked()
                {
                    interaction.take.export_actions.push(ExportAction::Retry(job.id));
                }
                if job.state == ExportStatus::Completed && ui.button("Show in Finder").clicked() {
                    interaction.take.export_actions.push(ExportAction::Reveal(job.output.clone()));
                }
            });
        });
    }
    if interaction.take.exports.iter().any(|job| job.state.is_finished())
        && ui.button("Clear finished jobs").clicked()
    {
        interaction.take.export_actions.push(ExportAction::ClearFinished);
    }
}

/// The export to queue, and a line for each key a recorded look carries that
/// this build does not read (#1927 B): the queued look is serialized afresh,
/// so the renderer never meets those keys to warn of them.
///
/// A recorded look is queued as written, unrepaired, so the renderer names
/// each value its load repairs on the export's row, and a retry names them
/// again (#1956 Q2).
fn capture_export(
    appearance: &crate::AppearanceDocument,
    look: crate::ExportLook,
    path: &std::path::Path,
) -> Result<(crate::ExportAction, Vec<String>), String> {
    let mut notes = Vec::new();
    let captured = match look {
        // A recorded look keeps its own composition, so queueing it does not
        // recompose the take to the pane's aspect and split; only the pane's
        // output settings apply (`RenderConfig::take_output_from`). Every take
        // writer records a look, so a take without one is refused rather than
        // drawn in some other look.
        crate::ExportLook::Recorded => {
            let header =
                harmonigraph_take::Header::read(path).map_err(|error| error.to_string())?;
            let look = header.appearance.as_deref().ok_or("the take recorded no appearance")?;
            let (mut recorded, unread) = crate::AppearanceDocument::read_noting(look)?;
            // What the renderer would refuse is refused here, at the click.
            recorded.clone().normalize()?;
            notes.extend(crate::LoadNotes { unread, repaired: Vec::new() }.lines());
            // The pane's own output settings are the live look's, already
            // repaired, so they add no note of their own.
            recorded.render.take_output_from(&appearance.render);
            recorded
        }
        crate::ExportLook::Current => appearance.clone(),
    };
    // The size the Exports list shows is read back out of the blob, as the
    // renderer reads it, so whatever parsing repairs, the two agree.
    let appearance = captured.serialize();
    let render = crate::AppearanceDocument::parse(&appearance)?.render;
    let size = render.frame.pixels(render.short_edge);
    Ok((crate::ExportAction::Queue { take: path.to_owned(), appearance, size }, notes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SpectralOrientation;

    /// The crop ticks are drawn at every UI scale a roomy preview can be
    /// shown at. The pad scales with the chrome and the lines do not, so
    /// without a floor the low end of the range left them no room.
    #[test]
    fn a_roomy_preview_has_room_for_the_crop_ticks_at_every_scale() {
        let roomy = egui::vec2(600.0, 400.0);
        for scale in [*theme::UI_SCALE_RANGE.start(), 1.0, *theme::UI_SCALE_RANGE.end()] {
            assert!(chrome_pad(scale, roomy) >= TICK_ROOM, "no ticks at {scale}");
        }
    }

    fn drag_preview(
        frame: &mut crate::RenderFrame,
        rect: egui::Rect,
        start: egui::Pos2,
        end: egui::Pos2,
    ) {
        let ctx = crate::tests::probe::themed();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::SHIFT,
        };
        for events in [
            vec![],
            vec![egui::Event::PointerMoved(start)],
            vec![button(start, true)],
            vec![egui::Event::PointerMoved(start.lerp(end, 0.02))],
            vec![egui::Event::PointerMoved(end)],
            vec![button(end, false)],
        ] {
            let _ = crate::tests::probe::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 1000.0),
                    )),
                    events: crate::tests::probe::holding(&ctx, egui::Modifiers::SHIFT, events),
                    ..Default::default()
                },
                |ui| {
                    let mut camera = harmonigraph_scene::Camera::default();
                    let mut view = harmonigraph_scene::ViewConfig::default();
                    preview_layout_controls(ui, rect, frame, &mut camera, &mut view);
                },
            );
        }
    }

    fn spectral_preview_frame(
        ctx: &egui::Context,
        state: &mut PictureState,
        frame_rect: egui::Rect,
        modifiers: egui::Modifiers,
        events: Vec<egui::Event>,
    ) {
        let spectral = preview_pane_rect(
            frame_rect,
            state.appearance.render.frame.lattice,
            state.appearance.render.frame.split,
            Pane::Spectral,
        );
        let _ = crate::tests::probe::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events: crate::tests::probe::holding(ctx, modifiers, events),
                ..Default::default()
            },
            |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(spectral));
                super::super::spectral::spectral_pane(
                    &mut child,
                    state,
                    100.0,
                    PREVIEW_SURFACE,
                    1.0,
                    super::super::spectral::Navigation::Preview,
                    state.shown(),
                    crate::panes::lattice::NO_SOLO,
                );
            },
        );
    }

    #[test]
    fn preview_divider_drag_resizes_each_side_and_clamps() {
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 50.0), egui::vec2(400.0, 300.0));
        for side in LatticeSide::ALL {
            for target in [0.01_f32, 0.7, 0.99] {
                let mut frame =
                    crate::RenderFrame { lattice: side, split: 0.5, ..Default::default() };
                let end = match side {
                    LatticeSide::Left => {
                        egui::pos2(rect.left() + rect.width() * target, rect.center().y)
                    }
                    LatticeSide::Right => {
                        egui::pos2(rect.right() - rect.width() * target, rect.center().y)
                    }
                    LatticeSide::Top => {
                        egui::pos2(rect.center().x, rect.top() + rect.height() * target)
                    }
                    LatticeSide::Bottom => {
                        egui::pos2(rect.center().x, rect.bottom() - rect.height() * target)
                    }
                };
                drag_preview(&mut frame, rect, rect.center(), end);
                assert!(
                    (frame.split - target.clamp(0.05, 0.95)).abs() < 0.001,
                    "{side:?}: {}",
                    frame.split
                );
                assert_eq!(frame.lattice, side);
            }
        }
    }

    #[test]
    fn preview_lattice_drag_docks_to_every_edge_and_cancels_outside() {
        for size in [egui::vec2(600.0, 300.0), egui::vec2(300.0, 600.0), egui::vec2(16.0, 16.0)] {
            let rect = egui::Rect::from_min_size(egui::pos2(40.0, 50.0), size);
            for from in LatticeSide::ALL {
                for to in LatticeSide::ALL {
                    let mut frame =
                        crate::RenderFrame { lattice: from, split: 0.05, ..Default::default() };
                    let lattice = preview_lattice_rect(rect, from, frame.split);
                    // The outer quarter stays outside even a narrowed divider hit area.
                    let start = lattice.center().lerp(
                        match from {
                            LatticeSide::Left => lattice.left_center(),
                            LatticeSide::Right => lattice.right_center(),
                            LatticeSide::Top => lattice.center_top(),
                            LatticeSide::Bottom => lattice.center_bottom(),
                        },
                        0.5,
                    );
                    let end = match to {
                        LatticeSide::Left => rect.left_center() + egui::vec2(2.0, 0.0),
                        LatticeSide::Right => rect.right_center() - egui::vec2(2.0, 0.0),
                        LatticeSide::Top => rect.center_top() + egui::vec2(0.0, 2.0),
                        LatticeSide::Bottom => rect.center_bottom() - egui::vec2(0.0, 2.0),
                    };
                    drag_preview(&mut frame, rect, start, end);
                    assert_eq!(frame.lattice, to, "from {from:?}");
                    assert_eq!(frame.split, 0.05);
                    frame.lattice = from;
                    drag_preview(&mut frame, rect, start, rect.max + egui::vec2(20.0, 20.0));
                    assert_eq!(frame.lattice, from);
                }
            }
        }
    }

    #[test]
    fn plain_drag_pans_the_preview_lattice_without_redocking_it() {
        let ctx = crate::tests::probe::themed();
        let mut state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
        state.appearance.render.frame.lattice = LatticeSide::Left;
        state.appearance.render.frame.split = 0.4;
        let before_frame = state.appearance.render.frame;
        let before_target = state.appearance.camera.target;
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 50.0), egui::vec2(600.0, 400.0));
        let start = preview_lattice_rect(rect, before_frame.lattice, before_frame.split).center();
        // Ends in the opposite edge's drop band, where a Shift-drag would redock.
        let end = rect.right_center() - egui::vec2(2.0, 0.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![],
            vec![egui::Event::PointerMoved(start)],
            vec![button(start, true)],
            vec![egui::Event::PointerMoved(end)],
            vec![button(end, false)],
        ] {
            let _ = crate::tests::probe::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    events: crate::tests::probe::holding(&ctx, egui::Modifiers::NONE, events),
                    ..Default::default()
                },
                |ui| {
                    let appearance = &mut state.appearance;
                    preview_layout_controls(
                        ui,
                        rect,
                        &mut appearance.render.frame,
                        &mut appearance.camera,
                        &mut appearance.view,
                    );
                },
            );
        }
        assert_ne!(state.appearance.camera.target, before_target, "Plain drag did not pan");
        assert_eq!(
            state.appearance.render.frame.lattice, before_frame.lattice,
            "Plain drag also redocked the lattice"
        );
        assert_eq!(state.appearance.render.frame.split, before_frame.split);
    }

    #[test]
    fn preview_analyzer_drag_turns_at_its_own_edges_and_cancels_outside() {
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 50.0), egui::vec2(600.0, 400.0));
        for lattice_side in LatticeSide::ALL {
            for target in LatticeSide::ALL {
                let ctx = crate::tests::probe::themed();
                let mut state =
                    PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
                state.appearance.render.frame.lattice = lattice_side;
                state.appearance.render.frame.split = 0.55;
                state.appearance.spectrum.orientation = match target {
                    LatticeSide::Left => SpectralOrientation::Right,
                    _ => SpectralOrientation::Left,
                };
                let spectral = preview_pane_rect(
                    rect,
                    state.appearance.render.frame.lattice,
                    state.appearance.render.frame.split,
                    Pane::Spectral,
                );
                let navigable = {
                    let c = &state.appearance.spectrum;
                    (c.low_midi, c.high_midi, c.roll_seconds, c.ceiling_db)
                };
                let start = spectral.center();
                let end = match target {
                    LatticeSide::Left => spectral.left_center() + egui::vec2(2.0, 0.0),
                    LatticeSide::Right => spectral.right_center() - egui::vec2(2.0, 0.0),
                    LatticeSide::Top => spectral.center_top() + egui::vec2(0.0, 2.0),
                    LatticeSide::Bottom => spectral.center_bottom() - egui::vec2(0.0, 2.0),
                };
                let button = |pos, pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::SHIFT,
                };
                for events in [
                    vec![egui::Event::PointerMoved(start)],
                    vec![egui::Event::PointerMoved(start), button(start, true)],
                    vec![egui::Event::PointerMoved(end)],
                    vec![button(end, false)],
                ] {
                    spectral_preview_frame(&ctx, &mut state, rect, egui::Modifiers::SHIFT, events);
                }
                let expected = match target {
                    LatticeSide::Left => SpectralOrientation::Left,
                    LatticeSide::Right => SpectralOrientation::Right,
                    LatticeSide::Top => SpectralOrientation::Top,
                    LatticeSide::Bottom => SpectralOrientation::Bottom,
                };
                assert_eq!(
                    state.appearance.spectrum.orientation, expected,
                    "lattice on {lattice_side:?}"
                );
                assert_eq!(
                    state.appearance.render.frame.lattice, lattice_side,
                    "turning the analyzer moved the lattice"
                );
                let c = &state.appearance.spectrum;
                assert_eq!(
                    (c.low_midi, c.high_midi, c.roll_seconds, c.ceiling_db),
                    navigable,
                    "a Shift-drag that turned the analyzer also navigated it"
                );

                let before = state.appearance.spectrum.orientation;
                let outside = rect.max + egui::vec2(20.0, 20.0);
                for events in [
                    vec![egui::Event::PointerMoved(start)],
                    vec![egui::Event::PointerMoved(start), button(start, true)],
                    vec![egui::Event::PointerMoved(outside)],
                    vec![button(outside, false)],
                ] {
                    spectral_preview_frame(&ctx, &mut state, rect, egui::Modifiers::SHIFT, events);
                }
                assert_eq!(
                    state.appearance.spectrum.orientation, before,
                    "a drop outside changed the orientation"
                );
            }
        }
    }

    #[test]
    fn clicking_near_an_analyzer_edge_does_not_reorient_it() {
        let ctx = crate::tests::probe::themed();
        let mut state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
        state.appearance.render.frame.lattice = LatticeSide::Right;
        state.appearance.render.frame.split = 0.4;
        state.appearance.spectrum.orientation = SpectralOrientation::Left;
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 50.0), egui::vec2(600.0, 400.0));
        let spectral = preview_pane_rect(
            rect,
            state.appearance.render.frame.lattice,
            state.appearance.render.frame.split,
            Pane::Spectral,
        );
        let near_right = egui::pos2(spectral.right() - spectral.width() * 0.1, spectral.center().y);
        let button = |pressed| egui::Event::PointerButton {
            pos: near_right,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::SHIFT,
        };
        for events in
            [vec![egui::Event::PointerMoved(near_right)], vec![button(true)], vec![button(false)]]
        {
            spectral_preview_frame(&ctx, &mut state, rect, egui::Modifiers::SHIFT, events);
        }
        assert_eq!(state.appearance.spectrum.orientation, SpectralOrientation::Left);
    }

    #[test]
    fn plain_drag_reaches_analyzer_zoom_beneath_the_orientation_overlay() {
        let ctx = crate::tests::probe::themed();
        let mut state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
        state.appearance.render.frame.lattice = LatticeSide::Left;
        state.appearance.render.frame.split = 0.3;
        // The analyzer's own layout, pinned like the frame's: which region a
        // press lands in is the roll share's, and a Span zoom needs room below.
        state.appearance.spectrum.orientation = SpectralOrientation::Left;
        state.appearance.spectrum.roll_fraction = 0.5;
        state.appearance.spectrum.roll_seconds = 30.0;
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 50.0), egui::vec2(600.0, 400.0));
        let spectral = preview_pane_rect(
            rect,
            state.appearance.render.frame.lattice,
            state.appearance.render.frame.split,
            Pane::Spectral,
        );
        // Left orientation runs time rightward. Start well inside the far
        // region, clear of both dividers, and pull toward the past.
        let start = egui::pos2(spectral.left() + spectral.width() * 0.75, spectral.center().y);
        let end = start + egui::vec2(80.0, 0.0);
        let before = state.appearance.spectrum;
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(start)],
            vec![egui::Event::PointerMoved(start), button(start, true)],
            vec![egui::Event::PointerMoved(end)],
            vec![button(end, false)],
        ] {
            spectral_preview_frame(&ctx, &mut state, rect, egui::Modifiers::NONE, events);
        }
        assert!(
            state.appearance.spectrum.roll_seconds < before.roll_seconds * 0.75,
            "Plain drag never reached the preview's Span zoom",
        );
        assert_eq!(
            state.appearance.spectrum.orientation, before.orientation,
            "navigating also changed orientation",
        );
    }

    #[test]
    fn analyzer_divider_stays_above_the_preview_orientation_drag() {
        let ctx = crate::tests::probe::themed();
        let mut state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
        state.appearance.render.frame.lattice = LatticeSide::Left;
        state.appearance.render.frame.split = 0.3;
        // The analyzer's own layout, pinned like the frame's: which region a
        // press lands in is the roll share's, and a Span zoom needs room below.
        state.appearance.spectrum.orientation = SpectralOrientation::Left;
        state.appearance.spectrum.roll_fraction = 0.5;
        state.appearance.spectrum.roll_seconds = 30.0;
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 50.0), egui::vec2(600.0, 400.0));
        let spectral = preview_pane_rect(
            rect,
            state.appearance.render.frame.lattice,
            state.appearance.render.frame.split,
            Pane::Spectral,
        );
        let before = state.appearance.spectrum;
        let split = 1.0 - before.roll_fraction;
        let start = egui::pos2(spectral.left() + spectral.width() * split, spectral.center().y);
        let end = start + egui::vec2(40.0, 0.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::SHIFT,
        };
        for events in [
            vec![egui::Event::PointerMoved(start)],
            vec![egui::Event::PointerMoved(start), button(start, true)],
            vec![egui::Event::PointerMoved(end)],
            vec![button(end, false)],
        ] {
            spectral_preview_frame(&ctx, &mut state, rect, egui::Modifiers::SHIFT, events);
        }
        assert!(
            state.appearance.spectrum.roll_fraction < before.roll_fraction - 0.05,
            "the analyzer divider did not move",
        );
        assert_eq!(
            state.appearance.spectrum.orientation, before.orientation,
            "dragging the analyzer divider also turned the pane",
        );
    }

    #[test]
    fn dragging_inside_a_wide_lattice_does_not_redock_it() {
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 50.0), egui::vec2(400.0, 300.0));
        for side in LatticeSide::ALL {
            let mut frame = crate::RenderFrame { lattice: side, split: 0.95, ..Default::default() };
            let direction = match side {
                LatticeSide::Left => egui::vec2(1.0, 0.0),
                LatticeSide::Right => egui::vec2(-1.0, 0.0),
                LatticeSide::Top => egui::vec2(0.0, 1.0),
                LatticeSide::Bottom => egui::vec2(0.0, -1.0),
            };
            // Both points are past the midline, toward the opposite side,
            // and the movement exceeds the drag threshold without reaching an edge band.
            let start = rect.center() + rect.size() * direction * 0.1;
            let end = rect.center() + rect.size() * direction * 0.2;
            drag_preview(&mut frame, rect, start, end);
            assert_eq!(frame.lattice, side);
            assert_eq!(frame.split, 0.95);
        }
    }

    #[test]
    fn preview_lattice_accepts_wheel_with_or_without_shift_and_pinch_zoom() {
        let gestures = [
            (
                "plain wheel",
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, 1.0),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Modifiers::NONE,
            ),
            (
                "Shift-wheel",
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, 1.0),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    modifiers: egui::Modifiers::SHIFT,
                },
                egui::Modifiers::SHIFT,
            ),
            ("pinch", egui::Event::Zoom(1.5), egui::Modifiers::NONE),
        ];
        for (gesture, event, modifiers) in gestures {
            let ctx = crate::tests::probe::themed();
            let mut state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
            let before = state.appearance.camera.distance;
            let rect = egui::Rect::from_min_size(egui::pos2(40.0, 50.0), egui::vec2(600.0, 400.0));
            let pointer = rect.center();
            let remaining_scroll = std::cell::Cell::new(egui::Vec2::ZERO);
            for events in [
                vec![egui::Event::PointerMoved(pointer)],
                vec![egui::Event::PointerMoved(pointer)],
                vec![egui::Event::PointerMoved(pointer), event],
            ] {
                let _ = crate::tests::probe::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800.0, 600.0),
                        )),
                        events: crate::tests::probe::holding(&ctx, modifiers, events),
                        ..Default::default()
                    },
                    |ui| {
                        preview_lattice(ui, rect, &mut state, 0.0, 1.0);
                        remaining_scroll.set(ui.input(|input| input.smooth_scroll_delta));
                    },
                );
            }
            assert!(
                state.appearance.camera.distance < before,
                "{gesture} did not zoom the preview lattice",
            );
            assert_eq!(
                remaining_scroll.get(),
                egui::Vec2::ZERO,
                "{gesture} would also scroll the Video controls",
            );
        }
    }

    /// A preview's state with every input the off-lattice band reads pinned
    /// rather than fresh: the camera whole, a perspective one at `distance`;
    /// just tuning, where distinct nodes are distinct pitches; and a spectral
    /// pane with the Analyzer, where the band is drawn, at half its depth over
    /// 55..73.
    fn preview_state(distance: f32) -> PictureState {
        let mut state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
        let camera = &mut state.appearance.camera;
        camera.projection = harmonigraph_scene::Projection::Perspective;
        (camera.yaw, camera.pitch, camera.distance) = (0.4, 0.3, distance);
        camera.target = Default::default();
        state.runtime.tuning = harmonigraph_core::Tuning::just();
        state.appearance.view.fade_time = 0.0;
        let spectrum = &mut state.appearance.spectrum;
        spectrum.orientation = SpectralOrientation::Left;
        (spectrum.low_midi, spectrum.high_midi) = (55.0, 73.0);
        (spectrum.show_roll, spectrum.roll_fraction) = (true, 0.5);
        state
    }

    /// The off-lattice bands in a frame's shapes.
    fn off_lattice_bands(output: &egui::FullOutput) -> usize {
        let band = theme::picture_off_scale().gamma_multiply(0.3);
        let shapes = output.shapes.iter();
        shapes.filter(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == band)).count()
    }

    /// A pinch over the preview's lattice moves the camera before the lattice
    /// draws, and the spectral pane describes the window drawn after it: a
    /// zoom out that brings a held note's node into view stops flagging it on
    /// that frame, not the next (#1833).
    #[test]
    fn a_preview_pinch_reaches_the_spectral_pane_in_its_own_frame() {
        let mut state = preview_state(4.0);
        let frame = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 450.0));
        let placements = Layout::split(LatticeSide::Left, 0.5).resolve(frame.size());
        let (_, lattice) = placements[0];
        let aspect = lattice.width() / lattice.height();
        const PINCH: f32 = 0.5;
        let before = state.appearance.view.scrolled(&state.appearance.camera, aspect);
        let mut zoomed = state.appearance.camera;
        zoomed.zoom_by(PINCH);
        let after = state.appearance.view.scrolled(&zoomed, aspect);
        let node = (after.min.threes..=after.max.threes)
            .flat_map(|threes| {
                (after.min.fives..=after.max.fives)
                    .map(move |fives| harmonigraph_core::LatticePos::new(threes, fives, 0))
            })
            .find(|&pos| after.contains(pos) && !before.contains(pos))
            .expect("the pinch brings no node of the home sheet into view");
        let semitones = state.runtime.tuning.pitch_class(node).to_cents() / 100.0;
        let tracker = &mut state.runtime.tracker;
        let on = |kind| harmonigraph_core::NoteEvent {
            source: harmonigraph_core::SourceId::DIRECT,
            time: 0.0,
            channel: 0,
            note: 60,
            kind,
        };
        tracker.handle_event(on(harmonigraph_core::NoteEventKind::On { velocity: 1.0 }));
        tracker.handle_event(on(harmonigraph_core::NoteEventKind::Tuning { semitones }));

        let ctx = crate::tests::probe::themed();
        let pointer = lattice.center();
        let mut bands = Vec::new();
        for events in [
            vec![egui::Event::PointerMoved(pointer)],
            vec![egui::Event::PointerMoved(pointer)],
            vec![egui::Event::PointerMoved(pointer), egui::Event::Zoom(PINCH)],
        ] {
            let output = crate::tests::probe::run(
                &ctx,
                egui::RawInput { screen_rect: Some(frame), events, ..Default::default() },
                |ui| preview_panes(ui, &placements, &mut state, 0.05, 1.0),
            );
            bands.push(off_lattice_bands(&output));
        }
        assert_eq!(state.appearance.camera.distance, zoomed.distance, "the pinch did not zoom");
        assert_eq!(bands, [1, 1, 0], "the pinch's frame flags the node it brought into view");
        assert_eq!(state.surfaces.drawn_this_frame, None, "the preview published its window");
    }

    /// The preview's spectral pane describes the preview's own lattice, as the
    /// export it previews does, not the docked Lattice tab's (#1833): a docked
    /// tab panned elsewhere, folded, or at another aspect must not put an
    /// off-lattice band on a note the preview's lattice lights. And the
    /// preview publishes nothing, so the docked panes go on describing the
    /// docked lattice.
    ///
    /// Just tuning, where distinct nodes are distinct pitches, a note at home,
    /// which any lattice framing the fresh camera draws, and a docked window
    /// far from it.
    #[test]
    fn the_preview_describes_its_own_lattice_rather_than_the_docked_one() {
        use harmonigraph_core::LatticePos;
        let mut state = preview_state(harmonigraph_scene::Camera::DEFAULT_DISTANCE);
        let docked = harmonigraph_scene::DrawnWindow {
            min: LatticePos::new(40, 40, 0),
            max: LatticePos::new(41, 41, 0),
        };
        state.surfaces.drawn = Some(docked);
        let frame = &mut state.appearance.render.frame;
        (frame.aspect_w, frame.aspect_h, frame.lattice, frame.split) =
            (16, 9, LatticeSide::Left, 0.5);
        state.runtime.tracker.handle_event(harmonigraph_core::NoteEvent::on(
            0.0,
            harmonigraph_core::SourceId::DIRECT,
            0,
            60,
            1.0,
        ));

        let ctx = crate::tests::probe::themed();
        let output = crate::tests::probe::run(
            &ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(480.0, 1200.0),
                )),
                ..Default::default()
            },
            |ui| render_pane(ui, &mut state, &mut crate::Interaction::default(), 0.05),
        );
        assert_eq!(
            off_lattice_bands(&output),
            0,
            "the preview flagged a note its own lattice lights"
        );
        assert_eq!(state.surfaces.drawn, Some(docked), "the preview published its window");
        assert_eq!(state.surfaces.drawn_this_frame, None, "the preview published its window");
    }

    #[test]
    fn preview_draws_both_panes_without_changing_the_shadow_dials() {
        let mut state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
        // The split of 2026-09-26, whose seam the spectral pane's child Ui
        // rounds INTO its own side. egui snaps that rect to its 1/32-point
        // grid and leaves the lattice's alone, so at a split whose seam rounds
        // outward the two overlap by a sliver of a point and the guard below
        // cannot tell the panes apart.
        state.appearance.render.frame.split = 0.277_261_67;
        for aspect in [(16, 9), (9, 16)] {
            state.appearance.render.frame.aspect_w = aspect.0;
            state.appearance.render.frame.aspect_h = aspect.1;
            for short_edge in [1080, 2160] {
                state.appearance.render.short_edge = short_edge;
                state.appearance.view.shadow.spectral_text.width =
                    harmonigraph_scene::SPECTRAL_SHADOW_MAX;
                state.appearance.view.shadow.spectral_text.spread = 0.2;
                let saved = state.appearance.view.shadow;
                // The draw must not change the settings that the dock and a
                // subsequent export will read.
                let ctx = egui::Context::default();
                crate::theme::apply_theme(&ctx);
                let output = crate::tests::probe::run(
                    &ctx,
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(480.0, 1200.0),
                        )),
                        ..Default::default()
                    },
                    |ui| render_pane(ui, &mut state, &mut crate::Interaction::default(), 0.0),
                );
                let callback_rects: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|s| {
                        if let egui::Shape::Callback(callback) = &s.shape {
                            callback.rect.is_positive().then_some(callback.rect)
                        } else {
                            None
                        }
                    })
                    .collect();
                // The lattice callback alone is not evidence that the
                // spectral arm ran. Both panes must emit callbacks in
                // their separate regions of the composed frame.
                assert!(
                    callback_rects
                        .iter()
                        .any(|a| callback_rects.iter().any(|b| !a.intersect(*b).is_positive())),
                    "fixture must reach both preview panes, got {callback_rects:?}"
                );
                assert_eq!(state.appearance.view.shadow, saved);
            }
        }
    }

    #[test]
    fn preview_scale_tracks_the_logical_frame_below_and_above_export_size() {
        let state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
        let pixels = state.appearance.render.frame.pixels(state.appearance.render.short_edge);
        let export_width = pixels[0] as f32 / crate::layout::export_pixels_per_point(pixels);

        assert!((preview_scale(export_width, &state.appearance.render) - 1.0).abs() < 1e-6);
        assert!((preview_scale(export_width * 0.25, &state.appearance.render) - 0.25).abs() < 1e-6);
        assert_eq!(preview_scale(export_width * 2.0, &state.appearance.render), 2.0);
    }

    #[test]
    fn video_preview_routes_its_frame_scale_into_hairline_roll_geometry() {
        let mut state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
        state.appearance.render.frame.aspect_w = 16;
        state.appearance.render.frame.aspect_h = 9;
        state.appearance.render.frame.lattice = LatticeSide::Top;
        state.appearance.render.frame.split = 0.05;
        state.appearance.spectrum.orientation = crate::SpectralOrientation::Top;
        state.appearance.spectrum.low_midi = -240.0;
        state.appearance.spectrum.high_midi = 360.0;
        state.appearance.spectrum.roll_thickness = 0.0;
        // No lead, so the held note is its history alone and draws no tongue.
        state.appearance.spectrum.roll_lead = 0.0;
        state.appearance.spectrum.show_roll = true;
        state.appearance.spectrum.show_spectrogram = false;
        state.runtime.tracker.handle_event(harmonigraph_core::NoteEvent::on(
            0.0,
            harmonigraph_core::SourceId::DIRECT,
            0,
            60,
            1.0,
        ));

        let mut samples = Vec::new();
        for width in [480.0, 1_800.0] {
            let _ = crate::panes::spectral::roll::take_roll_draw_probe();
            let ctx = crate::tests::probe::themed();
            let _ = crate::tests::probe::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 2_000.0),
                    )),
                    ..Default::default()
                },
                |ui| render_pane(ui, &mut state, &mut crate::Interaction::default(), 0.05),
            );
            let probe = crate::panes::spectral::roll::take_roll_draw_probe()
                .expect("the preview did not reach the roll draw boundary");
            assert_eq!(probe.surface, PREVIEW_SURFACE);
            let expected = preview_scale(probe.pitch_len, &state.appearance.render);
            assert!((probe.point_scale - expected).abs() < 1e-6);
            assert_eq!(probe.note_count, 1, "the fixture did not draw exactly one roll note");
            let half_pitch = probe.first_half_pitch.expect("the hairline fixture drew no note");
            samples.push((expected, half_pitch));
        }
        assert!(samples[0].0 < 1.0, "the small preview did not exercise downscaling");
        assert!(samples[1].0 > 1.0, "the large preview did not exercise enlargement");
        assert!(samples[0].1 < samples[1].1, "the enlarged preview kept the smaller floor");
        assert!((samples[0].1 / samples[1].1 - samples[0].0 / samples[1].0).abs() < 1e-3);
    }

    /// The Tail is a value bar like every other number: a double-click puts it
    /// back on a fresh render's tail.
    #[test]
    fn a_double_click_resets_the_tail() {
        use crate::tests::probe::{events_into, press, themed};
        let ctx = themed();
        let mut state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
        let mut interaction = crate::Interaction::default();
        state.appearance.render.tail = 7.5;
        let size = egui::vec2(400.0, 800.0);
        let mut frame = |events| {
            events_into(
                &ctx,
                size,
                egui::Rect::from_min_size(egui::Pos2::ZERO, size),
                events,
                |ui| render_controls(ui, &mut state, &mut interaction),
            )
        };
        let at = frame(vec![])
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Tail" => {
                    Some(text.pos + egui::vec2(4.0, 4.0))
                }
                _ => None,
            })
            .expect("the Tail is drawn");
        frame(vec![egui::Event::PointerMoved(at)]);
        for _ in 0..2 {
            frame(vec![press(at, true)]);
            frame(vec![press(at, false)]);
        }
        frame(vec![]);
        assert_eq!(state.appearance.render.tail, crate::RenderConfig::default().tail);
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;

    /// A take queued under its recorded appearance keeps the composition it
    /// was recorded with, and takes only the output settings from the pane.
    /// Recorded as 9:16 with the lattice on top, the scrolling history and the
    /// dialled range, against a pane at the fresh 16:9, Fit video and Fit piece.
    /// A key the recorded look holds that this build does not read is named
    /// to the pane, since the queued look no longer carries it (#1927 B). A
    /// value its load repairs is queued as held, for the renderer to name on
    /// the export's row (#1956 Q2), and the pane's output settings add none.
    #[test]
    fn a_recorded_look_keeps_its_frame_and_takes_the_panes_output_settings() {
        use crate::{LatticeSide, SpectrogramRender, VideoEncoder};
        let mut recorded = crate::AppearanceDocument::default();
        recorded.render.frame.aspect_w = 9;
        recorded.render.frame.aspect_h = 16;
        recorded.render.frame.lattice = LatticeSide::Top;
        recorded.render.frame.split = 0.3;
        recorded.render.spectrogram = SpectrogramRender::Scrolling;
        recorded.render.pitch_range = crate::PitchRangeRender::Dialled;
        recorded.render.short_edge = 720;
        recorded.render.tail = 5.0;
        recorded.render.encoder = VideoEncoder::X264;
        recorded.render.archive = false;
        recorded.view.sevens_size = -1.0;
        let path = std::env::temp_dir()
            .join(format!("harmonigraph-recorded-queue-{}.take", std::process::id()));
        let look = recorded.serialize().replacen("render:(", "render:(retired_key:1,", 1);
        assert_ne!(look, recorded.serialize());
        let header = harmonigraph_take::Header { appearance: Some(look), ..Default::default() };
        drop(harmonigraph_take::Writer::create(&path, &header).unwrap());

        let mut pane = crate::AppearanceDocument::default();
        pane.render.short_edge = 1080;
        pane.render.tail = 2.0;
        pane.render.encoder = VideoEncoder::Hardware;
        pane.render.archive = true;
        let (queued, notes) = capture_export(&pane, crate::ExportLook::Recorded, &path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("render.retired_key"), "{notes:?}");

        let crate::ExportAction::Queue { appearance, size, .. } = queued else {
            panic!("queue action")
        };
        let (captured, found) = crate::AppearanceDocument::parse_noting(&appearance).unwrap();
        let repaired: Vec<_> = found.repaired.iter().map(|r| (&*r.path, &*r.held)).collect();
        assert_eq!(repaired, [("view.sevens_size", "-1.0")], "the renderer's notes");
        let render = &captured.render;
        let frame = render.frame;
        assert_eq!((frame.aspect_w, frame.aspect_h, frame.split), (9, 16, 0.3));
        assert_eq!(frame.lattice, LatticeSide::Top);
        assert_eq!(render.spectrogram, SpectrogramRender::Scrolling);
        assert_eq!(render.pitch_range, crate::PitchRangeRender::Dialled);
        assert_eq!(render.short_edge, 1080);
        assert_eq!(render.tail, 2.0);
        assert_eq!(render.encoder, VideoEncoder::Hardware);
        assert!(render.archive);
        assert_eq!(size, [1080, 1920]);
    }

    /// A take whose header holds no appearance, which no take writer
    /// produces, is refused under Recorded rather than drawn in another look.
    #[test]
    fn a_take_with_no_recorded_look_is_refused_under_recorded() {
        let path = std::env::temp_dir()
            .join(format!("harmonigraph-unrecorded-queue-{}.take", std::process::id()));
        let header = harmonigraph_take::Header { appearance: None, ..Default::default() };
        drop(harmonigraph_take::Writer::create(&path, &header).unwrap());
        let queued = capture_export(
            &crate::AppearanceDocument::default(),
            crate::ExportLook::Recorded,
            &path,
        );
        std::fs::remove_file(&path).unwrap();
        let error = queued.err().expect("a take with no recorded look was queued");
        assert!(error.contains("recorded no appearance"), "refused for another reason: {error}");
    }

    #[test]
    fn expanded_builder_fits_and_queues_each_path_with_captured_settings() {
        let ctx = crate::tests::probe::themed();
        let mut interaction = crate::Interaction::default();
        interaction.take.export_paths = "/tmp/first.take\n/tmp/path with spaces/second.take".into();
        let mut appearance = crate::AppearanceDocument::default();
        appearance.render.short_edge = 1080;
        let mut time = 0.0;
        let mut frame = |events| {
            time += 0.1;
            crate::tests::probe::run(
                &ctx,
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(120.0, 2500.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| export_queue(ui, &appearance, &mut interaction),
            )
        };
        let text = |output: &egui::FullOutput, label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        Some(text.pos + egui::vec2(3.0, 3.0))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing {label}"))
        };
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        let first = frame(vec![]);
        let at = text(&first, "Exports");
        frame(vec![egui::Event::PointerMoved(at)]);
        frame(vec![press(at, true)]);
        frame(vec![press(at, false)]);
        for _ in 0..5 {
            frame(vec![]);
        }
        let expanded = frame(vec![]);
        let at = text(&expanded, "Queue paths");
        for shape in &expanded.shapes {
            let rect = shape.shape.visual_bounding_rect();
            if shape.clip_rect.right() >= 119.5 && rect.is_finite() && rect.width() < 1.0e4 {
                assert!(rect.right() <= 120.5, "expanded builder overruns: {:?}", shape.shape);
            }
        }
        frame(vec![egui::Event::PointerMoved(at)]);
        frame(vec![press(at, true)]);
        frame(vec![press(at, false)]);
        assert_eq!(interaction.take.export_actions.len(), 2);
        for (action, path) in interaction
            .take
            .export_actions
            .iter()
            .zip(["/tmp/first.take", "/tmp/path with spaces/second.take"])
        {
            let crate::ExportAction::Queue { take, appearance, size } = action else {
                panic!("queue action")
            };
            assert_eq!(take, std::path::Path::new(path));
            let render = crate::AppearanceDocument::parse(appearance).unwrap().render;
            assert_eq!(render.short_edge, 1080);
            assert_eq!(*size, render.frame.pixels(render.short_edge));
        }
    }
}
