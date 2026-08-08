//! Dragging the Spectral pane: the divider between the spectrum and the
//! roll, and the pans and zooms that move the two axes.
//!
//! Apart from the drawing because a gesture reads the plane and writes the
//! CONFIG — it is the only part of the pane that runs backwards, from a
//! screen position to the setting that would put something there.

use crate::SharedState;
use super::axes::{spectrum_share, widest_span, Axes};
use egui::Sense;

/// Half-width of the divider's grab band, in points. Wider than the hairline
/// it drags: the band is invisible until the pointer is inside it, so it has
/// to forgive an aim that is a few points off the line.
pub(super) const SPLIT_GRAB_HALF: f32 = 6.0;

/// The spectrum/far-region divider, draggable where it is drawn.
///
/// Deliberately NOT a dock separator: the spectrum, spectrogram and roll are
/// one pane on purpose — the spectrum's baseline sits ON this line and the
/// roll's notes arrive at it, which is how you see they are sounding — so
/// making them separate
/// panes would put that shared calibration across a pane boundary (and dock
/// panes detach, which this divider must not). Instead the handle drags
/// [`roll_fraction`](crate::SpectrumConfig::roll_fraction) directly, so the
/// drag persists with the rest of the UI state. It is the only way to set the
/// split: a "Roll share" bar on the Analyzer tab would be a second control for
/// the same field, and dragging the boundary you can see beats aiming a number
/// at it.
///
/// Returns the handle's response so the caller can paint its highlight last,
/// over the plots. `surface` keeps the docked pane and the Video preview from
/// sharing one interaction id.
pub(super) fn drag_split(
    ui: &mut egui::Ui,
    axes: &Axes,
    state: &mut SharedState,
    surface: usize,
) -> egui::Response {
    let band = axes.depth_band(spectrum_share(&state.spectrum_config), SPLIT_GRAB_HALF);
    let response = ui
        .interact(band, egui::Id::new(("spectral-split", surface)), Sense::drag())
        .on_hover_cursor(if axes.time_vertical {
            egui::CursorIcon::ResizeVertical
        } else {
            egui::CursorIcon::ResizeHorizontal
        });
    if let Some(pointer) = response.dragged().then(|| response.interact_pointer_pos()).flatten() {
        // Track the pointer absolutely rather than accumulating deltas: pushed
        // against either limit, an accumulated split drifts out of sync and
        // stops following the cursor on the way back. The band is a few pixels
        // wide, so grabbing it off-center snaps imperceptibly. Depth runs away
        // from the spectrum, so the roll gets what is left.
        state.spectrum_config.roll_fraction = (1.0 - axes.depth_at(pointer)).clamp(0.0, 1.0);
    }
    response
}

/// The `surface` that is the real, docked Analyzer pane. Slot 1 is the Video
/// tab's preview and the offline renderer never has a pointer, so this is the
/// one copy of the pane a person can actually navigate.
const DOCKED_SURFACE: usize = 0;

/// How much one point of scroll zooms, as an exponent — a full notch of a
/// mouse wheel (~50 points) closes the range by about a third, and a trackpad's
/// finer stream lands proportionally.
const ZOOM_PER_SCROLL_POINT: f32 = 0.008;

/// How much one point of drag along the depth axis zooms, as an exponent.
///
/// ONE rate for both of the values a depth drag can zoom, because it is one
/// gesture: a drag of a given length scales the picture under the hand by the
/// same factor whether that picture is the roll's time window or the spectrum's
/// dB window, and a hand that had to learn two gains for one movement would be
/// learning the code's seam rather than the pane's.
///
/// Sized on the time axis, which is the wider range of the two: from the 12 s
/// default, ~410 points of drag toward the past reaches 1 s and ~650 the other
/// way reaches 600 s. The level range is shorter at the same gain — its whole
/// 12..60 dB sweep against the default -60 dB floor is ~270 points — because a
/// dB window has less room to travel, not because it is dialled differently.
const DEPTH_ZOOM_PER_DRAG_POINT: f32 = 0.006;

/// How far a drag must lean along the depth axis before it is a zoom rather
/// than a pitch pan, in points. Panning is the default — the axes do NOT both
/// move at once, because what a depth drag zooms is a value being dialled in
/// (it shows on the Analyzer tab's bars), and a pitch pan with a few points of
/// depthwise slop in it would leave the Span or the Level quietly breathing.
/// The margin is small enough that the pitch the picture slides by before a
/// zoom takes over is a fraction of a semitone.
const DEPTH_ZOOM_LEAN: f32 = 4.0;

/// Drag or scroll to navigate the picture instead of aiming the Analyzer tab's
/// bars at it: across the pitch axis to pan the range, the wheel to zoom it,
/// and along the depth axis to zoom what that part of the depth axis measures —
/// the Span over the roll and spectrogram, the Level over the spectrum. All
/// four write [`SpectrumConfig`](crate::SpectrumConfig) —
/// `low_midi`/`high_midi`, `roll_seconds` and `ceiling_db` — so a navigated
/// view persists and reads back out on those bars.
///
/// The two axes carry DIFFERENT gestures because they are not the same kind of
/// axis. The pitch range is a window that can sit anywhere on the analyzer's
/// axis, so it pans and zooms. The depth axis is anchored at both of the things
/// it measures: time's near edge is always `now`, and the level's bottom is
/// always the baseline the curve stands on — which is the SAME line where the
/// two share a pane. So there is nothing to pan to, which leaves a drag along
/// depth free to mean zoom, and means the zoom is about that line rather than
/// about the pointer the way the pitch wheel is. Dragging away from it spreads
/// the picture out, so what the picture spans shrinks: toward the past for
/// fewer seconds, outward along the curve for fewer dB.
///
/// WHICH of the two a drag zooms is decided by where it STARTED, because that
/// is what the depth under the hand measures there: seconds in the far region,
/// dB over the spectrum's own share. The one gesture moving two values is not a
/// mode — a drag moves whatever it is on top of, and the two regions are drawn
/// far enough apart to aim at.
///
/// Docked pane only. The Video tab draws this same pane as its preview, and
/// that tab's body is a vertical `ScrollArea` — a wheel spent zooming there is
/// a wheel the tab cannot be scrolled with, which is the only thing
/// that keeps its controls reachable when the preview squeezes it. (The
/// divider is a different case and stays live in the preview: it is a handle
/// you aim at, not a gesture over the whole surface.)
pub(super) fn drag_zoom(
    ui: &egui::Ui,
    axes: &Axes,
    response: &egui::Response,
    state: &mut SharedState,
    surface: usize,
) {
    use harmonigraph_core::spectrum::{SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI};

    if surface != DOCKED_SURFACE {
        return;
    }
    // Where the far region begins, and so which drags have a time axis under
    // them and which a dB one. At 1.0 there is no far region at all (both
    // layers off, or the divider dragged shut) and the Span has nothing on
    // screen to zoom; at 0.0 the spectrum is the one with nothing.
    let split = spectrum_share(&state.spectrum_config);
    let cfg = &mut state.spectrum_config;
    let mut low = cfg.low_midi;
    let mut span = (cfg.high_midi - cfg.low_midi).max(crate::PITCH_RANGE_MIN_SPAN);
    // Nothing is written unless a gesture actually moved something. The range
    // is also the Analyzer tab's range bar, and rewriting it every frame would
    // put this function's rounding between that bar and the value it shows.
    let mut moved = false;

    // Zoom about the pitch under the pointer, so the note being looked at
    // stays put while the range closes in on it. `contains_pointer` rather
    // than `hovered()`: the divider sits on top of the pane, and the wheel
    // should keep zooming while the pointer crosses it.
    if response.contains_pointer() {
        // Wheel and trackpad pinch both zoom. egui routes ctrl+wheel and pinch
        // into `zoom_delta` and zeroes the scroll for them, so the two can't
        // double up here.
        let (scroll, pinch) = ui.ctx().input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = (scroll * ZOOM_PER_SCROLL_POINT).exp() * pinch;
        if (factor - 1.0).abs() > 1e-4 {
            let anchor = ui
                .ctx()
                .pointer_hover_pos()
                .map_or(0.5, |p| axes.pitch_at(p).clamp(0.0, 1.0));
            let held = low + anchor * span;
            span = (span / factor).clamp(crate::PITCH_RANGE_MIN_SPAN, widest_span());
            low = held - anchor * span;
            moved = true;
        }
    }

    // Grab the picture. Per-frame deltas rather than the absolute tracking the
    // divider uses — pushed against an end of an axis, an absolute anchor would
    // keep accumulating off-screen and the view would sit still on the way back.
    if response.dragged() {
        let delta = response.drag_delta();
        // Which axis this drag is on, decided from the TOTAL travel since the
        // press: a per-frame decision would flip between the two on the jitter
        // of a slow drag, and this way an L-shaped drag simply hands over once
        // its second leg is the longer one. What comes back is the depth the
        // press landed at, which then says which value the zoom writes.
        let leaning = ui
            .ctx()
            .input(|i| i.pointer.press_origin())
            .zip(response.interact_pointer_pos())
            .filter(|&(from, at)| {
                let total = at - from;
                total.dot(axes.dir_depth()).abs()
                    > total.dot(axes.dir_pitch()).abs() + DEPTH_ZOOM_LEAN
            })
            .map(|(from, _)| axes.depth_at(from));
        // A region with no depth of its own has nothing to zoom, so a drag over
        // where it would be is left to pan.
        let zoom_time = split < 1.0 && leaning.is_some_and(|d| d >= split);
        let zoom_level = split > 0.0 && leaning.is_some_and(|d| d < split);
        if zoom_time || zoom_level {
            let along = delta.dot(axes.dir_depth());
            if zoom_time {
                // Dragging toward the past pulls the picture away from the
                // now-line it is anchored on, spreading it — so the seconds it
                // spans shrink.
                cfg.roll_seconds = (cfg.roll_seconds * (-along * DEPTH_ZOOM_PER_DRAG_POINT).exp())
                    .clamp(crate::ROLL_SECONDS_MIN, crate::ROLL_SECONDS_MAX);
            } else {
                // Same rule, along the direction the curve actually grows in:
                // away from the far region it joins, or — with that region off
                // — up from the outer edge instead, which is `spectral_pane`'s
                // `joined` seen from here.
                let outward = if split < 1.0 { -along } else { along };
                // Only the ceiling moves. The floor IS the baseline the curve
                // stands on, so it is the fixed end of this zoom the way `now`
                // is the fixed end of the Span's; sliding the window bodily is
                // the Level bar's gesture, and there is one depth axis to drag
                // along, not two.
                let window = (cfg.ceiling_db - cfg.floor_db).max(crate::LEVEL_RANGE_MIN_SPAN);
                let window = (window * (-outward * DEPTH_ZOOM_PER_DRAG_POINT).exp())
                    .max(crate::LEVEL_RANGE_MIN_SPAN);
                // `min` then `max` rather than a clamp: a hand-edited floor can
                // sit within the minimum span of the domain's top, and clamp
                // panics on bounds that cross. The minimum span wins there,
                // which is the answer `loudness_raw` gives the same pair.
                cfg.ceiling_db = (cfg.floor_db + window)
                    .min(crate::LEVEL_MAX_DB)
                    .max(cfg.floor_db + crate::LEVEL_RANGE_MIN_SPAN);
            }
            ui.ctx().set_cursor_icon(if axes.time_vertical {
                egui::CursorIcon::ResizeVertical
            } else {
                egui::CursorIcon::ResizeHorizontal
            });
        } else {
            // The pitch under the pointer travels with it, so the range moves
            // the opposite way.
            let along = delta.dot(axes.dir_pitch());
            low -= along / axes.pitch_len().max(1.0) * span;
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            moved = true;
        }
    }

    if !moved {
        return;
    }
    // Land inside the axis the analyzer actually covers. Same invariant
    // `SpectrumConfig::sanitize` enforces on load: a range past the axis
    // draws a band with no buckets behind it. The upper bound takes a `max`
    // because at the full span the two ends meet, and float rounding is enough
    // to cross them — which `clamp` answers with a panic.
    let span = span.clamp(crate::PITCH_RANGE_MIN_SPAN, widest_span());
    cfg.low_midi = low.clamp(SPECTRUM_MIN_MIDI, (SPECTRUM_MAX_MIDI - span).max(SPECTRUM_MIN_MIDI));
    cfg.high_midi = cfg.low_midi + span;
}
