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
use crate::widgets::{button_row, choice_row, option_label, record_button};
use crate::{theme, LatticeSide, Layout, Pane, PictureState};

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
pub(crate) fn render_pane(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    interaction: &mut crate::Interaction,
    now: f64,
) {
    record_controls(ui, state, interaction);
    frame_controls(ui, state);
    render_controls(ui, state, interaction);

    section(ui, "Preview");
    ui.weak(
        "Drag pictures to an edge · Shift-drag pictures to navigate · Scroll or pinch to zoom · Drag dividers to resize",
    );
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
    let placeholder = state.appearance.render.playhead;
    for (pane, rect) in &placements {
        let rect = rect.translate(box_rect.min.to_vec2());
        match pane {
            Pane::Spectral if placeholder => playhead_preview(ui, rect, state),
            Pane::Spectral => {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                // Its text sizes itself off the rect it is given, so drawing
                // the pane small draws its type small, as the render will.
                // Shadows instead use screen points. Scale their widths for
                // this drawing only, including the reach used by roll culling.
                let shadow = state.appearance.view.shadow;
                state.appearance.view.shadow =
                    preview_shadows(shadow, box_rect.width(), &state.appearance.render);
                super::spectral::spectral_pane(
                    &mut child,
                    state,
                    now,
                    PREVIEW_SURFACE,
                    preview_scale(box_rect.width(), &state.appearance.render),
                    super::spectral::Navigation::Preview,
                );
                state.appearance.view.shadow = shadow;
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
    let appearance = &mut state.appearance;
    preview_layout_controls(
        ui,
        box_rect,
        &mut appearance.render.frame,
        &mut appearance.camera,
        &mut appearance.view,
    );
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
    let navigation_held = ui.input(|i| i.modifiers.shift || i.pointer.middle_down());
    let moving_lattice = ui
        .interact(move_rect, ui.id().with("preview_lattice_move"), Sense::drag())
        .on_hover_cursor(egui::CursorIcon::Grab)
        .on_hover_text(if navigation_held {
            "Pan the lattice"
        } else {
            "Drag the lattice to the left, right, top or bottom edge · Hold Shift and drag to pan"
        });
    let resizing = ui
        .interact(handle, ui.id().with("preview_lattice_resize"), Sense::drag())
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
            frame.split = (fraction_at(pointer) + offset).clamp(0.05, 0.95);
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
        ui.painter().line_segment(ends, egui::Stroke::new(2.0, color));
    }
    // Freeze the choice made at press time. Letting Shift change during a drag
    // switch modes would first pan the camera and then redock the picture on
    // release (or the reverse), which turns one gesture into two edits.
    let navigation_id = moving_lattice.id.with("preview_navigation");
    if moving_lattice.drag_started() {
        ui.data_mut(|data| data.insert_temp(navigation_id, navigation_held));
    }
    let navigating =
        ui.data(|data| data.get_temp::<bool>(navigation_id)).unwrap_or(navigation_held);
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
        ui.data_mut(|data| data.remove::<bool>(navigation_id));
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

fn preview_shadows(
    mut shadow: harmonigraph_scene::ShadowSettings,
    width: f32,
    config: &crate::RenderConfig,
) -> harmonigraph_scene::ShadowSettings {
    let scale = preview_scale(width, config);
    shadow.spectral_geometry.width *= scale;
    shadow.spectral_text.width *= scale;
    shadow
}

/// The preview's logical frame size relative to the one the export uses.
/// Point-sized effects use this so a smaller live preview remains the same
/// composition rather than making fixed-width ink look heavier.
fn preview_scale(width: f32, config: &crate::RenderConfig) -> f32 {
    let pixels = config.frame.pixels(config.short_edge);
    let export_width = pixels[0] as f32 / crate::layout::export_pixels_per_point(pixels);
    // The live preview normally shrinks the shot. At larger-than-export sizes
    // keep the dial's maximum rather than manufacture an out-of-range style.
    (width / export_width).clamp(0.0, 1.0)
}

/// Aspect ratio and resolution — editing the persisted
/// `RenderFrame` and the resolution beside it.
fn frame_controls(ui: &mut egui::Ui, state: &mut PictureState) {
    section(ui, "Frame");
    // A `button_row` rather than a `choice_row`: the selection is a PAIR of
    // numbers, not one enum value, so there is nothing for choice_row's
    // `selectable_value` to compare against.
    button_row(ui, |ui| {
        ui.label("Aspect ratio")
            .on_hover_text("Shape of the exported video. Short edge sets its size in pixels.");
        let f = &mut state.appearance.render.frame;
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
    let frame = state.appearance.render.frame;
    let sizes: Vec<(u32, String, String)> = [720u32, 1080, 1440, 2160]
        .iter()
        .map(|&short| {
            let [w, h] = frame.pixels(short);
            (short, short.to_string(), format!("{w}x{h}"))
        })
        .collect();
    let options: Vec<(u32, &str, &str)> =
        sizes.iter().map(|(v, label, hint)| (*v, label.as_str(), hint.as_str())).collect();
    choice_row(ui, "Short edge (px)", &mut state.appearance.render.short_edge, &options);
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
fn clear_everything(ui: &mut egui::Ui, state: &mut PictureState) {
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
fn render_controls(
    ui: &mut egui::Ui,
    state: &mut PictureState,
    interaction: &mut crate::Interaction,
) {
    section(ui, "Render");
    choice_row(
        ui,
        "Spectrogram",
        &mut state.appearance.render.playhead,
        &[
            (false, "Scrolling", "Bake the live scrolling spectrogram, exactly as previewed here"),
            (
                true,
                "Playhead",
                "Show the entire recorded spectrogram with a moving playhead. Requires recorded audio; this region stays blank in the live preview.",
            ),
        ],
    );
    if !interaction.take.supported {
        return;
    }

    // When a take finishes and turns into a video.
    choice_row(
        ui,
        "Render when",
        &mut state.appearance.render.trigger,
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
            (
                crate::RenderTrigger::AtBar,
                "Bar",
                "Finish recording and render when the transport plays through the bar below. The one that works during an audio export, which never loops and never reports itself playing.",
            ),
        ],
    );
    // Only under the trigger that reads it — a bar shown beside three triggers
    // that ignore it is a dial that appears to do nothing three times out of
    // four. The value is kept either way, so switching away and back does not
    // lose it.
    if state.appearance.render.trigger == crate::RenderTrigger::AtBar {
        button_row(ui, |ui| {
            ui.label("Stop at bar").on_hover_text(
                "Counted as the host's arranger counts: bar 1 is the song's start. \
                 Recording must reach this bar from before it — arming with the playhead \
                 already past it records until you turn Record take off.",
            );
            ui.add(
                egui::DragValue::new(&mut state.appearance.render.stop_bar)
                    .range(crate::STOP_BAR_RANGE.0..=crate::STOP_BAR_RANGE.1)
                    .speed(0.25),
            );
        });
    }

    // Re-render the last take with the frame you've dialed in since recording.
    // The take carries only a record-time snapshot, so this is how a reframed
    // preview reaches the video without recording again.
    if interaction.take.last_ready {
        ui.add_space(2.0);
        if ui
            .button("Re-render take")
            .on_hover_text(
                "Render the last take using the current frame settings. Saves the video beside the take. If a render is running, it is replaced by this one.",
            )
            .clicked()
        {
            interaction.take.render_now = true;
        }
    }
    render_progress(ui, interaction);
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

fn playhead_preview(ui: &mut egui::Ui, rect: egui::Rect, state: &mut PictureState) {
    playhead_placeholder(ui, rect);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    super::spectral::preview_gestures(&mut child, state, PREVIEW_SURFACE);
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
fn preview_lattice(ui: &mut egui::Ui, rect: egui::Rect, state: &mut PictureState, now: f64) {
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }
    // The preview is a picture of the RENDER, so it stands on the render
    // layout's background rather than on the panel this preview happens to sit
    // on — the same colour `harmonigraph-offline` will clear to.
    let background = harmonigraph_scene::skin::ground_color(
        Layout::split(state.appearance.render.frame.lattice, state.appearance.render.frame.split)
            .background,
    );
    let response = ui.interact(rect, ui.id().with("preview_lattice_zoom"), Sense::hover());
    if let Some((scroll, zoom)) = super::zoom_gesture(ui, &response) {
        if scroll != 0.0 {
            state.appearance.camera.zoom(scroll);
            // This pane lives inside the Video tab's vertical ScrollArea. A
            // wheel over the picture belongs to its zoom; taking it here keeps
            // the parent from scrolling the controls at the same time.
            ui.input_mut(|input| input.smooth_scroll_delta.y = 0.0);
        }
        if zoom != 1.0 {
            state.appearance.camera.zoom_by(zoom);
        }
    }
    super::lattice::draw_lattice(ui, rect, state, now, PREVIEW_SURFACE, background, None, None);
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
    section(ui, "Record");
    let rolling = interaction.take.rolling;
    record_button(ui, &mut interaction.take.recording, rolling, "Record take").on_hover_text(
        "Record notes, automation, the current look and the selected audio input for video export. Press again to finish, or choose an automatic ending under Render when.",
    );
    if !interaction.take.status.is_empty() {
        ui.weak(&interaction.take.status);
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
fn render_progress(ui: &mut egui::Ui, interaction: &mut crate::Interaction) {
    let Some(progress) = interaction.take.render_progress else { return };
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
            interaction.take.cancel_render = true;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SpectralOrientation;

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
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![],
            vec![egui::Event::PointerMoved(start)],
            vec![button(start, true)],
            vec![egui::Event::PointerMoved(start.lerp(end, 0.02))],
            vec![egui::Event::PointerMoved(end)],
            vec![button(end, false)],
        ] {
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 1000.0),
                    )),
                    events,
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
        placeholder: bool,
    ) {
        let spectral = preview_pane_rect(
            frame_rect,
            state.appearance.render.frame.lattice,
            state.appearance.render.frame.split,
            Pane::Spectral,
        );
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                modifiers,
                events,
                ..Default::default()
            },
            |ui| {
                if placeholder {
                    playhead_preview(ui, spectral, state);
                } else {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(spectral));
                    super::super::spectral::spectral_pane(
                        &mut child,
                        state,
                        100.0,
                        PREVIEW_SURFACE,
                        1.0,
                        super::super::spectral::Navigation::Preview,
                    );
                }
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
    fn shift_drag_pans_the_preview_lattice_without_redocking_it() {
        let ctx = crate::tests::probe::themed();
        let mut state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
        state.appearance.render.frame.lattice = LatticeSide::Left;
        state.appearance.render.frame.split = 0.4;
        let before_frame = state.appearance.render.frame;
        let before_target = state.appearance.camera.target;
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 50.0), egui::vec2(600.0, 400.0));
        let start = preview_lattice_rect(rect, before_frame.lattice, before_frame.split).center();
        let end = start + egui::vec2(60.0, 30.0);
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
            vec![egui::Event::PointerMoved(end)],
            vec![button(end, false)],
        ] {
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    modifiers: egui::Modifiers::SHIFT,
                    events,
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
        assert_ne!(state.appearance.camera.target, before_target, "Shift-drag did not pan");
        assert_eq!(
            state.appearance.render.frame.lattice, before_frame.lattice,
            "Shift-drag also redocked the lattice"
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
                    modifiers: egui::Modifiers::NONE,
                };
                for events in [
                    vec![egui::Event::PointerMoved(start)],
                    vec![egui::Event::PointerMoved(start), button(start, true)],
                    vec![egui::Event::PointerMoved(end)],
                    vec![button(end, false)],
                ] {
                    spectral_preview_frame(
                        &ctx,
                        &mut state,
                        rect,
                        egui::Modifiers::NONE,
                        events,
                        false,
                    );
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

                let before = state.appearance.spectrum.orientation;
                let outside = rect.max + egui::vec2(20.0, 20.0);
                for events in [
                    vec![egui::Event::PointerMoved(start)],
                    vec![egui::Event::PointerMoved(start), button(start, true)],
                    vec![egui::Event::PointerMoved(outside)],
                    vec![button(outside, false)],
                ] {
                    spectral_preview_frame(
                        &ctx,
                        &mut state,
                        rect,
                        egui::Modifiers::NONE,
                        events,
                        false,
                    );
                }
                assert_eq!(
                    state.appearance.spectrum.orientation, before,
                    "a drop outside changed the orientation"
                );
            }
        }
    }

    #[test]
    fn playhead_placeholder_keeps_analyzer_orientation_and_pitch_zoom_live() {
        let ctx = crate::tests::probe::themed();
        let mut state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
        state.appearance.render.playhead = true;
        state.appearance.render.frame.lattice = LatticeSide::Left;
        state.appearance.render.frame.split = 0.3;
        state.appearance.spectrum.low_midi = 36.0;
        state.appearance.spectrum.high_midi = 96.0;
        let rect = egui::Rect::from_min_size(egui::pos2(40.0, 50.0), egui::vec2(600.0, 400.0));
        let spectral = preview_pane_rect(
            rect,
            state.appearance.render.frame.lattice,
            state.appearance.render.frame.split,
            Pane::Spectral,
        );
        let start = spectral.center();
        let end = rect.center_top() + egui::vec2(0.0, 2.0);
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
            spectral_preview_frame(&ctx, &mut state, rect, egui::Modifiers::NONE, events, true);
        }
        assert_eq!(state.appearance.spectrum.orientation, SpectralOrientation::Top);

        let before = state.appearance.spectrum.high_midi - state.appearance.spectrum.low_midi;
        for events in [
            vec![egui::Event::PointerMoved(start)],
            vec![egui::Event::PointerMoved(start)],
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 40.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        ] {
            spectral_preview_frame(&ctx, &mut state, rect, egui::Modifiers::NONE, events, true);
        }
        let after = state.appearance.spectrum.high_midi - state.appearance.spectrum.low_midi;
        assert!(after < before - 1.0, "the placeholder swallowed pitch zoom");
    }

    #[test]
    fn shift_drag_reaches_analyzer_zoom_beneath_the_orientation_overlay() {
        let ctx = crate::tests::probe::themed();
        let mut state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
        state.appearance.render.frame.lattice = LatticeSide::Left;
        state.appearance.render.frame.split = 0.3;
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
            modifiers: egui::Modifiers::SHIFT,
        };
        for events in [
            vec![egui::Event::PointerMoved(start)],
            vec![egui::Event::PointerMoved(start), button(start, true)],
            vec![egui::Event::PointerMoved(end)],
            vec![button(end, false)],
        ] {
            spectral_preview_frame(&ctx, &mut state, rect, egui::Modifiers::SHIFT, events, false);
        }
        assert!(
            state.appearance.spectrum.roll_seconds < before.roll_seconds * 0.75,
            "Shift-drag never reached the preview's Span zoom",
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
            modifiers: egui::Modifiers::NONE,
        };
        for events in [
            vec![egui::Event::PointerMoved(start)],
            vec![egui::Event::PointerMoved(start), button(start, true)],
            vec![egui::Event::PointerMoved(end)],
            vec![button(end, false)],
        ] {
            spectral_preview_frame(&ctx, &mut state, rect, egui::Modifiers::NONE, events, false);
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
    fn preview_lattice_accepts_plain_wheel_and_pinch_zoom() {
        let gestures = [
            (
                "plain wheel",
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, 1.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                },
            ),
            ("pinch", egui::Event::Zoom(1.5)),
        ];
        for (gesture, event) in gestures {
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
                let _ = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800.0, 600.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        preview_lattice(ui, rect, &mut state, 0.0);
                        remaining_scroll.set(ui.input(|input| input.smooth_scroll_delta));
                    },
                );
            }
            assert!(
                state.appearance.camera.distance < before,
                "{gesture} did not zoom the preview lattice",
            );
            assert_eq!(
                remaining_scroll.get().y,
                0.0,
                "plain wheel would also scroll the Video controls",
            );
        }
    }

    #[test]
    fn preview_shadows_keep_the_exports_relative_reach_without_changing_the_dials() {
        use harmonigraph_render::spectral_shadow_reach;
        use harmonigraph_scene::ShadowKernel;

        let mut state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
        for aspect in [(16, 9), (9, 16)] {
            state.appearance.render.frame.aspect_w = aspect.0;
            state.appearance.render.frame.aspect_h = aspect.1;
            for short_edge in [1080, 2160] {
                state.appearance.render.short_edge = short_edge;
                let pixels = state.appearance.render.frame.pixels(short_edge);
                let export_width =
                    pixels[0] as f32 / crate::layout::export_pixels_per_point(pixels);
                for kernel in [ShadowKernel::Distance, ShadowKernel::Gaussian] {
                    state.appearance.view.shadow.spectral_geometry.kernel = kernel;
                    state.appearance.view.shadow.spectral_text.kernel = kernel;
                    let saved = state.appearance.view.shadow;
                    for width in [240.0, 480.0] {
                        let scaled = preview_shadows(saved, width, &state.appearance.render);
                        for (preview, export) in [
                            (scaled.spectral_geometry, saved.spectral_geometry),
                            (scaled.spectral_text, saved.spectral_text),
                        ] {
                            let reach = spectral_shadow_reach(export);
                            assert!(reach > 0.0, "both shadow groups must cast");
                            assert!(
                                (spectral_shadow_reach(preview) / width - reach / export_width)
                                    .abs()
                                    < 1e-6
                            );
                            assert_eq!(preview.depth, export.depth);
                            assert_eq!(preview.falloff, export.falloff);
                            assert_eq!(preview.kernel, export.kernel);
                        }
                    }
                    // Exercise the actual scoped draw too: it must restore the
                    // settings that the dock and a subsequent export will read.
                    assert!(!state.appearance.render.playhead, "the spectral preview must draw");
                    let ctx = egui::Context::default();
                    crate::theme::apply_theme(&ctx);
                    let output = ctx.run_ui(
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
    }

    #[test]
    fn preview_scale_tracks_the_logical_frame_and_caps_at_export_size() {
        let state = PictureState::new(harmonigraph_render::wgpu::TextureFormat::Rgba8Unorm);
        let pixels = state.appearance.render.frame.pixels(state.appearance.render.short_edge);
        let export_width = pixels[0] as f32 / crate::layout::export_pixels_per_point(pixels);

        assert!((preview_scale(export_width, &state.appearance.render) - 1.0).abs() < 1e-6);
        assert!((preview_scale(export_width * 0.25, &state.appearance.render) - 0.25).abs() < 1e-6);
        assert_eq!(preview_scale(export_width * 2.0, &state.appearance.render), 1.0);
    }
}
