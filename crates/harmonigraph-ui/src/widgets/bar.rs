//! The lengths and the two readings every bar here is built out of.
//!
//! Parked in one place rather than inside whichever control needed each first.
//! A reach one bar declares and three others read is a shared contract wherever
//! it is written down, and one living inside a caller reads as that caller's
//! own until the day it is changed for it.

use egui::{Color32, CornerRadius, Response, Ui};

use crate::theme;

// The bars' own geometry, written at the design size — scale 1.0. Each is
// multiplied by `theme::ui_scale` where it is drawn, so a bar shrinks with the
// type inside it rather than keeping a 20-point row around 9-point text.

// A bar's own HEIGHT is not among them: it is `theme::row_height`, which every
// control in a settings row is drawn at. A number here instead would be free to
// drift from that one, and a bar standing a few points off the buttons beside
// it is the whole thing the shared height exists to prevent — a bar being the
// tallest thing a row has to hold, two text runs inside a track, makes it what
// the row height is FOR rather than something measured against it.

/// The accent a bar fills its track with, at the strength its pointer state
/// asks for: the drag mix while it is being dragged, the hover mix while it is
/// merely under the pointer, and the resting one otherwise.
///
/// The three colours are the theme's. What is here is only WHICH of them a
/// `Response` picks, which is the part `ValueBar`, `RangeBar`, `OctaveStrip` and
/// `SpreadBar` were each spelling out identically — and a bar that answered the
/// two states in a different order from its neighbours would be the one control
/// in a pane lighting differently under the same pointer.
pub(super) fn track_fill(response: &Response) -> Color32 {
    if response.dragged() {
        theme::accent_fill_drag()
    } else if response.hovered() {
        theme::accent_fill_hover()
    } else {
        theme::accent_fill()
    }
}

/// Corner rounding of the bar track — the shared control radius, so a bar and
/// a button beside it round the same.
pub(super) fn bar_radius(scale: f32) -> u8 {
    theme::control_radius(scale)
}
/// Inset of a bar's name and its value readout from the bar's own ends.
pub(super) const BAR_TEXT_PAD: f32 = 8.0;
/// Clear space kept between the two, so an elided name stops short of the
/// number rather than touching it.
pub(super) const BAR_LABEL_GAP: f32 = 6.0;

/// A bar's NAME, laid out into what its readout leaves it: one row, ellipsised,
/// against `width` less the inset at each end, the gap held clear of the number,
/// and the `reserve` the number itself is owed.
///
/// The job arrives already carrying its sections, because what a name is MADE OF
/// differs per bar — a badge at the front, a colour the ground behind it decides
/// — while the room it is allowed is the same arithmetic in all six. Handing the
/// job in is what puts that arithmetic in one place without this having to know
/// what a badge is.
///
/// `width` rather than the bar's own rect, because one bar's name is not elided
/// against its row: a [`SpectrumBar`] elides against its TRACK, the flip button
/// beside it having taken the rest.
///
/// [`SpectrumBar`]: super::gradient::SpectrumBar
pub(super) fn elided_name(
    painter: &egui::Painter,
    mut job: egui::text::LayoutJob,
    width: f32,
    scale: f32,
    reserve: f32,
) -> std::sync::Arc<egui::Galley> {
    job.wrap.max_width =
        (width - 2.0 * BAR_TEXT_PAD * scale - BAR_LABEL_GAP * scale - reserve).max(0.0);
    job.wrap.max_rows = 1;
    job.wrap.overflow_character = Some('\u{2026}');
    painter.layout_job(job)
}

/// How wide a bar draws: the width the layout offers, but never past the
/// visible edge of the pane. Shared by [`ValueBar`] and [`RangeBar`], so every
/// bar in a settings column comes out the same length and they all narrow
/// together as the column does.
///
/// `available_width` alone is not enough, and the reason is worth stating
/// because nothing about it is visible here: egui's
/// `Region::expand_to_include_rect` unions `max_rect` as well as `min_rect`, so
/// any control that overruns the column widens the region for everything AFTER
/// it, and a bar sizing itself from the layout inherits the overrun as a floor
/// it cannot shrink past. Each bar's minimum length is then the width of the
/// widest thing above it — several different minimums down one pane, the bars
/// under a wide control running their value readout off the pane edge while the
/// ones above compress properly.
///
/// [`button_row`] keeps rows and their button labels inside the column, which is
/// what removes the usual sources; this covers the ones with nowhere to wrap to,
/// like the record button and the Options field in a very narrow Video pane.
///
/// The limit comes from [`crate::panes::pane_content_right`], which the pane
/// records on the way in. Deliberately not the clip rect: that is the tab BODY,
/// a [`theme::pane_inner_margin`] wider than the content box on each side, so a
/// bar clamped to it comes out a margin longer than its neighbours and flush on
/// the pane border. Outside a pane — the widget's own tests — there is nothing
/// to hand a value over, and the clip is then the honest fallback.
///
/// [`ValueBar`]: super::value::ValueBar
/// [`RangeBar`]: super::range::RangeBar
/// [`button_row`]: super::rows::button_row
pub(super) fn bar_width(ui: &Ui) -> f32 {
    let right = ui
        .data(|d| d.get_temp::<f32>(crate::panes::pane_content_right()))
        .unwrap_or_else(|| ui.clip_rect().right());
    ui.available_width().min(right - ui.cursor().left()).max(0.0)
}

/// How near a handle the pointer has to start for the drag to take that
/// handle rather than the span between them. Generous on purpose: grabbing
/// the span when you meant an end is the easy mistake, and the expensive one
/// — it moves BOTH values instead of the one you were aiming at.
///
/// The one length here the chrome scale leaves alone, because it is a reach
/// rather than a drawn thing: a bar dialled smaller is a smaller target, which
/// is the case for keeping the reach where it was rather than shrinking it
/// too. `HANDLE_REACH_SHARE` already stops it swallowing a narrow bar.
pub(super) const GRAB_PX: f32 = 14.0;
/// Ceiling on that reach, as a share of the span from each side, so the two
/// handles can never claim the whole of a narrow range and leave nothing to
/// slide.
pub(super) const HANDLE_REACH_SHARE: f32 = 0.35;
/// Width of a [`RangeBar`] handle grip.
///
/// [`RangeBar`]: super::range::RangeBar
pub(super) const HANDLE_W: f32 = 6.0;
/// How far the value track is inset from the bar's ends, so a handle parked
/// at either limit still sits fully inside the bar with track visible past
/// it.
///
/// Without this the bar has no visible affordance in the state it starts
/// life in: the pitch range defaults to the FULL axis, which puts both
/// handles flush against the ends, under the corner rounding, where they
/// read as the bar's own border. The control then looks exactly like a
/// ValueBar filled to 100% — nothing about it says there is anything to take
/// hold of.
pub(super) const HANDLE_INSET: f32 = HANDLE_W * 0.5 + 1.0;
/// Breathing room between a handle and its readout, and between a readout and
/// the bar's edge.
pub(super) const TEXT_GAP: f32 = 5.0;

/// Draw one handle, and any text run standing under it a second time INSIDE
/// it — the same galley at the same origin, clipped to the grip and overridden
/// to the panel colour, so the letters cross the thumb in reverse instead of
/// vanishing into it.
///
/// A thumb and this tree's text are both near-white, so a crossing swallows
/// whichever of the two paints first, and "-60 dB" reading "-60 B" is the
/// concrete thing that costs. Where a run can be PLACED clear of the thumbs
/// that is the better answer and [`RangeBar`] does it for its two numbers; this
/// is for the runs that cannot move — a name pinned to the left of the bar, or
/// a [`SpreadBar`] readout parked at the right that a handle crosses at rest.
///
/// Sliding such a run out of the way was built and dropped: it has to snap back
/// the moment the handle passes, and text jumping its own width mid-drag reads
/// worse than a letter that merely inverts.
///
/// The knocked-out letters read about as strongly as the run they belong to,
/// which is the bar this has to clear: panel on the near-white thumb is 14.1:1,
/// against 8.9:1 for a dimmed name on the `well()` track and 15.2:1 for a run
/// at full `text()` on it. So it is a shade stronger than a resting name and a
/// shade weaker than a run being hovered or dragged — the [`SpreadBar`] readout
/// is always the latter — and both ends of that are far enough above any
/// legibility floor that the crossing costs nothing either way. The claim is
/// parity, not improvement.
///
/// **Overlapping thumbs come out right whatever order they are painted in**,
/// and the reason is worth stating because the ordering here looks load-bearing
/// and is not. A later fill can only cover an earlier knockout inside
/// `A ∩ B ∩ run`; that region lies in `run`, so `B` intersects `run` too and
/// emits its own knockout over the same ground. Painting every fill and then
/// every knockout is equally correct. A flat ramp puts a [`SpreadBar`]'s two
/// grips at exactly the same x — the state the Nodes section's chroma bar opens
/// in, since `chroma_ramp` defaults to 0 — so this is a case that ships, not a
/// corner.
///
/// **The clip is square where the grip is rounded**, so a glyph pixel in a
/// corner notch lands beside the thumb rather than on it, in the panel colour,
/// over whatever the bar has there. egui has no rounded clip to ask for, so
/// this is a bound rather than a fix, and how tight it is depends on the
/// caller's `radius`:
///
/// - At a grip's own 2pt the notch is a sliver at the extreme corners of a grip
///   drawn 3pt shy of a 20pt row, well outside a Body galley's ink.
/// - A [`RangeBar::fade_span`] bar hands in the BAR's radius instead, so its
///   thumb is a 6pt-wide pill (epaint holds a corner to half the width, so 5
///   becomes 3) and the notch is the whole of the top and bottom 3pt. A
///   descender reaches into that band.
///
/// What keeps the second case cheap is where it lands rather than whether it
/// happens: outside the span the notch is over `well()`, which is DARKER than
/// the knockout, so nothing shows; over the span fill it is 2.7:1, a faint
/// speck at the tip of a descender. Not nothing, and the thing to fix if a
/// thumb is ever rounded harder still — by shrinking the runs' boxes to the
/// grip's straight section rather than by tightening this clip, which would
/// only hand the cap band back to a swallowed letter.
///
/// Guarded on a rect intersection, and the guard is cheaper than it looks
/// rather than the reverse: epaint sets its clip per shape and culls text per
/// ROW against it, so an unguarded pass over a thumb that misses the run would
/// be dropped before a vertex was written. What the guard actually saves is an
/// `Arc` clone, a shape push and a clip-rect split — small, and kept because a
/// paint list free of invisible shapes is what lets a test assert a knockout
/// exists by counting. It buys no tessellation back, and the CROSSED case still
/// tessellates a whole row to show 6pt of it, which no guard here addresses.
///
/// [`RangeBar`]: super::range::RangeBar
/// [`RangeBar::fade_span`]: super::range::RangeBar::fade_span
/// [`SpreadBar`]: super::gradient::SpreadBar
pub(super) fn grip_over_text(
    painter: &egui::Painter,
    grip: egui::Rect,
    radius: CornerRadius,
    runs: &[(egui::Pos2, std::sync::Arc<egui::Galley>)],
) {
    painter.rect_filled(grip, radius, theme::text());
    for (pos, galley) in runs {
        if grip.intersects(egui::Rect::from_min_size(*pos, galley.size())) {
            painter
                .with_clip_rect(grip)
                .galley_with_override_text_color(*pos, galley.clone(), theme::panel());
        }
    }
}

/// What a drag has hold of: the grab this gesture already settled on, or
/// `decide` on the first frame it is asked, remembered under `id` for the rest
/// of the gesture.
///
/// Deciding ONCE is the whole point, and each of the four bars that ask needs it
/// for a reason of its own — a range whose ends can be dragged past each other
/// would hand the gesture to whichever handle is nearest now, a strip that
/// splits its two gestures on a hard line would change its mind as the pointer
/// crossed it, and an arc dragged down to nothing would fall into the rotate
/// branch the moment the handle reached the left edge.
///
/// Asked from inside the drag rather than under `drag_started`, so a gesture
/// whose start frame was missed still does something.
///
/// Read and write are separate statements on purpose: nesting a `data_mut`
/// inside a `data` closure takes the context lock twice, and nothing here is
/// worth risking that on a path only a real pointer reaches.
///
/// `decide` is handed the `Ui` rather than closing over it, since all four
/// answers are asked of [`aimed_at`] and a closure borrowing the same `Ui` this
/// is reading through has nothing to gain by saying so twice.
pub(super) fn grabbed<G>(ui: &Ui, id: egui::Id, decide: impl FnOnce(&Ui) -> G) -> G
where
    G: Clone + Send + Sync + 'static,
{
    let stored = ui.data(|d| d.get_temp::<G>(id));
    match stored {
        Some(grab) => grab,
        None => {
            let grab = decide(ui);
            ui.data_mut(|d| d.insert_temp(id, grab.clone()));
            grab
        }
    }
}

/// Forget it, which is what letting go does: egui's temp store has no expiry, so
/// a grab left behind is inherited by the next press
/// (`a_second_gesture_on_the_strip_chooses_for_itself`).
///
/// The `Default` bound is egui's, asked of anything `remove_temp` can remove,
/// and is the whole reason each of the four grabs derives one.
pub(super) fn release_grab<G>(ui: &Ui, id: egui::Id)
where
    G: Clone + Send + Sync + Default + 'static,
{
    ui.data_mut(|d| d.remove_temp::<G>(id));
}

/// Where a gesture was AIMED, as against where it has since got to: the point
/// the press landed on, falling back to the live position when there is no
/// press on record.
///
/// **Every bar here decides WHICH of its handles a drag has hold of from this,
/// and where that handle then goes from the live position.** The two questions
/// want different positions because egui calls a press a drag only once the
/// pointer has left a six-point click threshold: the first frame a bar sees is
/// already that far along, in the direction of travel, and a real frame of a
/// brisk drag carries a good deal more. So a press that landed squarely inside
/// a handle's reach arrives with the pointer clear of it, pointing at whatever
/// the hand was aiming PAST — and the bar hands over the span instead of the
/// end it was aimed at, or the turn instead of the handle.
///
/// A reach is also what the CURSOR promises, and it promises it under a pointer
/// at rest, which is where a press lands. Deciding from anywhere else is the
/// control disagreeing with the arrow the hand aimed with.
///
/// [`GRAB_PX`]'s fourteen points are no protection: that is barely twice the
/// threshold, so it is the outer half of every reach that misbehaves — which is
/// what makes the failure intermittent rather than total, and so what let it
/// stand.
pub(super) fn aimed_at(ui: &Ui, live: egui::Pos2) -> egui::Pos2 {
    ui.input(|i| i.pointer.press_origin()).unwrap_or(live)
}
