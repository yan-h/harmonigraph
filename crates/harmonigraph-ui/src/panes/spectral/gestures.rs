//! Dragging the Spectral pane: the divider between the spectrum and the
//! roll, the pans and zooms that move the two axes, and where the divider
//! stands when the pane it is drawn on is resized.
//!
//! Apart from the drawing because all of it runs BACKWARDS — from a screen
//! position, or a screen SIZE, to where the pane's boundary goes.
//!
//! Which is why the resize sits beside the drag rather than in the pane: the
//! two are the only things that decide that boundary, and they answer in
//! different currencies. [`drag_split`] writes the DIAL
//! ([`roll_fraction`](crate::SpectrumConfig::roll_fraction), which persists and
//! is what a take exports with); [`hold_spectrum`] leaves that alone and works
//! out what the docked pane draws instead, so that a resized pane keeps the
//! spectrum's own size. [`spectrum_split`] is where the two meet, and it is the
//! one answer every layer of the pane takes its boundary from.

use super::axes::{spectrum_share, widest_span, Axes};
use crate::panes::{zoom_gesture, DOCKED_SURFACE};
use crate::SharedState;
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
/// split: a "Roll share" bar on the Analyzer section would be a second control for
/// the same field, and dragging the boundary you can see beats aiming a number
/// at it.
///
/// Returns the handle's response so the caller can paint its highlight last,
/// over the plots. `surface` keeps the docked pane and the Video preview from
/// sharing one interaction id.
///
/// `split` is where the divider is DRAWN on this pane — the caller's one
/// reading of [`spectrum_split`], so the band cannot end up straddling a line
/// somewhere else. Writing the dial rather than that is what makes the drag a
/// re-dial: it lands at the pointer, and the hold takes it from there.
/// Which axis a depth drag committed to, once it has.
///
/// `Default` is egui's, asked of anything `remove_temp` can remove; the value
/// is always written by the frame that commits before anything reads it.
#[derive(Clone, Copy, Default)]
struct Leg {
    depth: bool,
}

fn lean_id(surface: usize) -> egui::Id {
    egui::Id::new(("spectral-lean", surface))
}

/// Whether this drag is a DEPTH gesture (a zoom) rather than a pitch one (a
/// pan).
///
/// A drag picks its axis on the first frame it has travelled
/// [`DEPTH_ZOOM_LEAN`] further along one than the other, and KEEPS it until the
/// hand lets go — the same rule every bar here follows, where `Grab::at` is
/// asked once and held for the gesture.
///
/// Re-deciding each frame is what it replaces, and the reason is that the
/// question has no stable answer late in a drag. Travel since the press has the
/// perpendicular drift of a curving hand accumulating for as long as the drag
/// lasts, while the along-axis travel returns toward zero if the hand comes
/// back — so a drag out and back crosses over somewhere on the way home,
/// at a distance that grows with both how far it went and how much the hand
/// wandered. What that looks like is a zoom stopping dead with the mouse still
/// moving and the cursor turning to a hand, at a value different every time.
/// Measuring each LEG from its own start does not fix it either: the leg only
/// restarts when the axis changes, which is the moment already too late.
///
/// The cost is that an L-shaped drag no longer hands over mid-gesture — turning
/// the corner keeps the axis the first leg took, and switching means letting go
/// and pressing again. That is what every other gesture in the app asks for,
/// and it buys a drag that cannot change what it controls under the hand.
///
/// Held in egui's temp store, because the committed axis is the one thing here
/// a frame cannot see for itself. `drag_stopped` forgets it, or the next press
/// inherits this one's axis.
fn lean_is_depth(
    ui: &egui::Ui,
    surface: usize,
    from: egui::Pos2,
    at: egui::Pos2,
    axes: &Axes,
) -> bool {
    let id = lean_id(surface);
    if let Some(leg) = ui.data(|d| d.get_temp::<Leg>(id)) {
        return leg.depth;
    }
    let travel = at - from;
    let (depth, pitch) = (travel.dot(axes.dir_depth()).abs(), travel.dot(axes.dir_pitch()).abs());
    // Inside the margin the press has not said which gesture it is yet, and
    // nothing is committed: the pan answers, as it does for a drag that never
    // leans at all.
    let committed = if depth > pitch + DEPTH_ZOOM_LEAN {
        Some(true)
    } else if pitch > depth + DEPTH_ZOOM_LEAN {
        Some(false)
    } else {
        None
    };
    if let Some(depth) = committed {
        ui.data_mut(|d| d.insert_temp(id, Leg { depth }));
    }
    committed.unwrap_or(false)
}

pub(super) fn drag_split(
    ui: &mut egui::Ui,
    axes: &Axes,
    state: &mut SharedState,
    surface: usize,
    split: f32,
) -> egui::Response {
    let band = axes.depth_band(split, SPLIT_GRAB_HALF);
    let response = ui
        .interact(band, egui::Id::new(("spectral-split", surface)), Sense::drag())
        .on_hover_cursor(axes.resize_cursor());
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

/// What the divider is holding for the pane it was dialled on: the picture the
/// dial made, in points, so that resizing that pane resizes the SPECTROGRAM and
/// leaves the spectrum the size it was.
///
/// The two regions are not the same kind of picture, which is what makes one of
/// them the one to stretch. The spectrum is a curve whose height IS its dB
/// window: give it more depth and every peak grows, so the shape being read —
/// how steeply a partial stands out of the noise — is drawn differently on
/// every pane size. The far region is a WINDOW onto time: give it more depth
/// and the same [`roll_seconds`](crate::SpectrumConfig::roll_seconds) is laid
/// out over more of it, which spreads the picture without restating it. So a
/// resize spent there costs a reading nothing, and one spent on the curve costs
/// the reading the curve exists for.
///
/// **The dial is not touched.** `roll_fraction` stays exactly what was dragged,
/// and this carries the answer the docked pane draws from
/// ([`spectrum_split`]) instead. Which matters because that field is what a take
/// EXPORTS with (`save_persist` rides the config into every take, and the
/// offline renderer composes the frame from it): written through, the editor
/// window's size would decide a video's composition, and dragging the plugin
/// wider would restyle a render nobody touched. The Video tab is where a
/// video's look is dialled in and its preview reads the same field, so it has
/// to sit still while the editor around it moves — the same call
/// [`spectrum_settings_pane`](super::spectrum_settings_pane) makes in refusing
/// an automatic orientation.
///
/// Held for the docked pane alone, and by its tab dispatch (see
/// [`crate::panes`]), which is the one surface a window can resize: the Video
/// preview and the offline renderer draw from a size their own layout fixes,
/// and a hold means nothing there.
#[derive(Default)]
pub(crate) struct SpectrumHold(Option<Held>);

#[derive(Clone, Copy)]
struct Held {
    /// The `roll_fraction` this was dialled from.
    ///
    /// A fraction that no longer matches it says the divider has been dragged,
    /// or a blob loaded, or a take's config arrived — so what is held describes
    /// a picture nobody is asking for any more, and the dial answers until the
    /// next pass re-takes it. An exact comparison, and it can be: nothing
    /// derives this field from itself, so there is no rounding to forgive.
    dial: f32,
    /// The depth axis that dial was made against, in points. With the dialled
    /// share it gives the spectrum's length, which is the thing being kept.
    depth: f32,
    /// Which screen axis that was, so a change of
    /// [`orientation`](crate::SpectrumConfig::orientation) re-dials rather than
    /// spending a length measured down the pane on the way across it. The two
    /// differ by more than three to one on the analyzer's own column.
    vertical: bool,
    /// The spectrum's share of the depth axis on the pane this was measured
    /// for — what [`spectrum_split`] hands the docked pane in place of the
    /// dialled share.
    held: f32,
}

/// The depth the far region keeps for itself once holding the spectrum starts
/// crowding it, in points.
///
/// A spectrogram thinner than this is a band rather than a picture — the
/// heatmap's structure is in how it CHANGES along time, and there is no room to
/// see that in a strip a few notes wide; a roll alone is the same argument with
/// ribbons for cells. So the hold is a promise about the spectrum's size that
/// the far region's own legibility ends: past this the spectrum yields, which
/// is the failure that costs the pane less.
///
/// A judgement rather than a derivation, sized against the thing it is
/// protecting: about a third of the depth the analyzer column gets in the
/// plugin's own default window, so it bites only on a pane squeezed well under
/// what it was dialled at.
pub(super) const FAR_REGION_FLOOR_PT: f32 = 64.0;

/// How the depth axis is shared out on the pane being DRAWN: the dialled share
/// ([`spectrum_share`]), or the docked pane's hold where one stands.
///
/// Every layer of the pane takes its split from here, so the divider's grab
/// band, the curve, the heatmap and the ribbons cannot disagree about where the
/// boundary is.
pub(super) fn spectrum_split(state: &SharedState, surface: usize) -> f32 {
    let cfg = &state.spectrum_config;
    let dialled = spectrum_share(cfg);
    match state.spectrum_hold.0 {
        // A dial of the WHOLE axis is a state rather than a size: the far
        // region turned off, or the divider dragged shut — which the pane
        // keeps grabbable precisely so that shutting it is not one-way (see
        // `spectral_pane`). Holding points against it would re-open a region
        // that was closed on purpose, so the dial wins.
        Some(held)
            if surface == DOCKED_SURFACE
                && dialled < 1.0
                && held.dial == cfg.roll_fraction
                && held.vertical == cfg.orientation.is_time_vertical() =>
        {
            held.held
        }
        _ => dialled,
    }
}

/// Take the size the spectrum is drawn at on the docked pane, so that resizing
/// that pane spends the difference on the spectrogram.
///
/// Called with the pane's own size, BEFORE it draws, so what this settles is
/// what the frame is composed from and the picture never shows an intermediate.
///
/// Two things bound it, and both are the far region's:
///
/// - Growing, nothing does: the spectrum holds its points and every point the
///   pane gains goes to the spectrogram, however tall the pane gets.
/// - Shrinking, the far region floors at [`FAR_REGION_FLOOR_PT`] and the
///   spectrum yields from there — and under a pane too small to hold even that
///   beside the dialled share, the whole thing gives way to the plain
///   proportional split, which is what a pane with no hold on it draws at every
///   size. So it degrades toward sharing the squeeze rather than toward a
///   spectrum filling the pane edge to edge.
pub(crate) fn hold_spectrum(state: &mut SharedState, pane: egui::Vec2) {
    let cfg = state.spectrum_config;
    // No far region is no divider — there is nothing to take a resize. The hold
    // is left standing rather than dropped, so turning the spectrogram back on
    // returns the picture it was turned off at.
    if !(cfg.show_roll || cfg.show_spectrogram) {
        return;
    }
    let share = spectrum_share(&cfg);
    // The divider dragged shut, which is a state to sit in rather than a size to
    // keep (see [`spectrum_split`]). Dropped rather than left standing: what
    // re-opens it is a drag, which re-dials anyway.
    if share >= 1.0 {
        state.spectrum_hold.0 = None;
        return;
    }
    let vertical = cfg.orientation.is_time_vertical();
    let depth = if vertical { pane.y } else { pane.x };
    // A pane with no depth to share out: the first frame of a dock that has not
    // laid out yet, or a leaf folded to its tab bar. Dividing by it would hand
    // the picture an infinity, and holding a size against it would price the
    // dial at zero points. Non-finite is the same case at its limit —
    // egui_dock's viewport is `Rect::NOTHING` until it has laid out, whose sides
    // subtract to an infinity rather than to a number worth dividing by.
    if !depth.is_finite() || depth <= 0.0 {
        return;
    }
    let held = match state.spectrum_hold.0 {
        Some(held) if held.dial == cfg.roll_fraction && held.vertical == vertical => held,
        // Dialled since this last looked — so THIS is the picture to keep, at
        // the size and along the axis it was set on. A dial is already the
        // answer for the pane it was made on, so nothing is derived from it
        // until that pane changes.
        _ => {
            state.spectrum_hold.0 =
                Some(Held { dial: cfg.roll_fraction, depth, vertical, held: share });
            return;
        }
    };
    // The spectrum's dialled length, and the two floors under it. `max` before
    // `min` so the order reads as what it is: what the far region will spare
    // decides the ceiling, and the dialled share is the floor under THAT — at a
    // depth where the two cross, a bare `min` would hand the spectrum less than
    // its share of a pane it is supposed to be filling more of.
    let kept = share * held.depth;
    let keep = kept.min((depth - FAR_REGION_FLOOR_PT).max(share * depth)) / depth;
    state.spectrum_hold.0 = Some(Held { held: keep.clamp(0.0, 1.0), ..held });
}

/// How much one point of scroll zooms, as an exponent — a full notch of a
/// mouse wheel (~50 points) closes the range by about a third, and a trackpad's
/// finer stream lands proportionally.
///
/// The [`spiral`](crate::panes::spiral) pane's wheel reads this too. The two
/// panes zoom different things — the range here, the disc's own magnification
/// there — and share the rate anyway, because what a rate says is how far a hand
/// has to spin for a third of a picture, and that answer should not depend on
/// which drawing of the analyzer is under it.
pub(crate) const ZOOM_PER_SCROLL_POINT: f32 = 0.008;

/// How much one point of drag along the depth axis zooms, as an exponent.
///
/// ONE rate for both of the values a depth drag can zoom, because it is one
/// gesture: a drag of a given length scales the picture under the hand by the
/// same factor whether that picture is the roll's time window or the spectrum's
/// dB window, and a hand that had to learn two gains for one movement would be
/// learning the code's seam rather than the pane's.
///
/// Sized on the time axis, which is the wider range of the two: the whole
/// 1 s..600 s sweep is ~1070 points of drag, and the three-minute default sits
/// near its long end — ~870 points toward the past reach 1 s, ~200 the other
/// way reach 600 s. The travel is lopsided because the default is, not because
/// the gain is: most of what a drag has to reach from there is the close-up,
/// which is the direction it is reached for. The level range is shorter at the
/// same gain — its whole
/// 12..60 dB sweep against the default -60 dB floor is ~270 points — because a
/// dB window has less room to travel, not because it is dialled differently.
pub(crate) const DEPTH_ZOOM_PER_DRAG_POINT: f32 = 0.006;

/// How far a drag must lean along the depth axis before it is a zoom rather
/// than a pitch pan, in points. Panning is the default — the axes do NOT both
/// move at once, because what a depth drag zooms is a value being dialled in
/// (it shows on the Analyzer section's bars), and a pitch pan with a few points of
/// depthwise slop in it would leave the Span or the Level quietly breathing.
/// The margin is small enough that the pitch the picture slides by before a
/// zoom takes over is a fraction of a semitone.
pub(super) const DEPTH_ZOOM_LEAN: f32 = 4.0;

/// Which of the two things the depth axis measures a drag has under it — the
/// whole of what [`drag_zoom`] decides from where a press landed, held as one
/// value so the choice is made once. Two booleans and an `else` say the same
/// thing three times, and the `else` reads as a fallthrough rather than as the
/// other half of a pair.
enum DepthZoom {
    /// The far region's time window: the roll's Span.
    Span,
    /// The spectrum's dB window: the Level.
    Level,
}

/// Drag or scroll to navigate the picture instead of aiming the Analyzer section's
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
    // Where the far region begins on the pane being drawn, and so which drags
    // have a time axis under them and which a dB one. The caller's reading, for
    // the reason [`drag_split`] takes one: a gesture aimed at a region has to
    // read the boundary the picture shows. At 1.0 there is no far region at all
    // (both layers off, or the divider dragged shut) and the Span has nothing on
    // screen to zoom; at 0.0 the spectrum is the one with nothing.
    split: f32,
) {
    use harmonigraph_core::spectrum::{SPECTRUM_MAX_MIDI, SPECTRUM_MIN_MIDI};

    if surface != DOCKED_SURFACE {
        return;
    }
    let cfg = &mut state.spectrum_config;
    let mut low = cfg.low_midi;
    let mut span = (cfg.high_midi - cfg.low_midi).max(crate::PITCH_RANGE_MIN_SPAN);
    // Nothing is written unless a gesture actually moved something. The range
    // is also the Analyzer section's range bar, and rewriting it every frame would
    // put this function's rounding between that bar and the value it shows.
    let mut moved = false;

    // Zoom about the pitch under the pointer, so the note being looked at
    // stays put while the range closes in on it.
    if let Some((scroll, pinch)) = zoom_gesture(ui, response) {
        let factor = (scroll * ZOOM_PER_SCROLL_POINT).exp() * pinch;
        if (factor - 1.0).abs() > 1e-4 {
            let anchor =
                ui.ctx().pointer_hover_pos().map_or(0.5, |p| axes.pitch_at(p).clamp(0.0, 1.0));
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
        // Which axis this drag is on — see [`lean_is_depth`]. What comes back is
        // the depth the PRESS landed at, which then says which value the zoom
        // writes: the region a gesture is aimed at is where it started, not
        // wherever it has since travelled to.
        let leaning = ui
            .ctx()
            .input(|i| i.pointer.press_origin())
            .zip(response.interact_pointer_pos())
            .filter(|&(from, at)| lean_is_depth(ui, surface, from, at, axes))
            .map(|(from, _)| axes.depth_at(from));
        // A region with no depth of its own has nothing to zoom, so the other
        // one answers for the whole pane rather than leaving a strip of it
        // dead: at a split of 1.0 the far EDGE is still the spectrum's, and at
        // 0.0 the near edge is still the roll's. Only a pane with neither
        // region falls through to the pan.
        let zoom = leaning.and_then(|d| {
            if split < 1.0 && d >= split {
                Some(DepthZoom::Span)
            } else if split > 0.0 {
                Some(DepthZoom::Level)
            } else {
                None
            }
        });
        if let Some(zoom) = zoom {
            let along = delta.dot(axes.dir_depth());
            match zoom {
                DepthZoom::Span => {
                    // Dragging toward the past pulls the picture away from the
                    // now-line it is anchored on, spreading it — so the seconds it
                    // spans shrink.
                    let zoomed = cfg.roll_seconds * (-along * DEPTH_ZOOM_PER_DRAG_POINT).exp();
                    cfg.roll_seconds =
                        zoomed.clamp(crate::ROLL_SECONDS_MIN, crate::ROLL_SECONDS_MAX);
                }
                DepthZoom::Level => {
                    // Same rule, along the direction the curve actually grows in:
                    // away from the far region it joins, or — with that region off
                    // — up from the outer edge instead. That is `spectral_pane`'s
                    // `joined`, and it reads the same from here because the one
                    // layout where the two part company — whole-song, where the
                    // pane forces the split to 0 and draws no spectrum at all —
                    // belongs to the offline renderer, which has no pointer.
                    let outward = if split < 1.0 { -along } else { along };
                    // Only the ceiling moves. The floor IS the baseline the curve
                    // stands on, so it is the fixed end of this zoom the way `now`
                    // is the fixed end of the Span's; sliding the window bodily is
                    // the Level range bar's gesture, and there is one depth axis to drag
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
                    // Every frame of this drag restarts the spectrogram's ring:
                    // the dB window is the heatmap's too, so `ColumnColor` sees
                    // the change and every column is repainted. That is the
                    // picture genuinely changing, not the waste `ColumnColor` was
                    // spelled out field by field to end — and it is the cost the
                    // pitch drag already pays each frame through `scale_min_bits`.
                }
            }
            ui.ctx().set_cursor_icon(axes.resize_cursor());
        } else {
            // The pitch under the pointer travels with it, so the range moves
            // the opposite way.
            let along = delta.dot(axes.dir_pitch());
            low -= along / axes.pitch_len().max(1.0) * span;
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            moved = true;
        }
    }

    if response.drag_stopped() {
        ui.data_mut(|d| d.remove_temp::<Leg>(lean_id(surface)));
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
